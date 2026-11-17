//! [`HcBackend`]：herdr-core 會話後端的本體（unix；Windows 見 `stub.rs`）。
//!
//! - 一個 manager 一條分幀連線（[`Client`]），所有會話的 attach 串流與一條 events 串流都在上面。
//! - 每個會話一直 attach 著（跟 PTY 的讀取執行緒一樣）：第一畫面＋之後的位元組都進 [`TermSession`] 的 ring buffer，
//!   前端照舊從 replay 接、看不出後端換了。
//! - 串流被 core 關掉（帶原因）不直接當結束：先 `terminal.get` 問清楚——真的結束了才收尾（[`TermSession::finish`]＋
//!   `terminal.rm`＋刪中繼資料），還活著就重新 attach（core 換版重啟的情況）。
//! - 整條連線斷掉：有上限的退避重連（約 30 秒）；連上了就重新訂事件、重新 attach 每個活著的會話；
//!   連不上就把會話標成結束並寫明原因（中繼資料留著，下次開 App 還會試著接回）。
//! - 狀態：core 的 `status` 事件（與 attach 回覆、訂閱快照裡的 `status`）→ [`ProgramRecorder`]。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use serde_json::{json, Map, Value};

use super::client::{Client, RpcError, StreamHandler, StreamMsg};
use super::frame::{self, Kind};
use super::locate::{self, HcSetup};
use super::{
    ended_note, load_meta, meta_dir, remove_meta, report_of, save_meta, spine_tag, Existing,
    StartError, StoredMeta, LABEL_KIND, LABEL_SID, LABEL_SPINE,
};
use crate::session_status::ProgramRecorder;
use crate::term::{
    now_started, session_env, title_or_program, BackendKind, ExitHook, SessionIo, SpawnSpec,
    TermKind, TermMeta, TermSession,
};
use crate::{Error, Result};

/// App 重開後第一次接回時要的捲動歷史（行）。之後重連只要畫面（歷史已經在 ring buffer 裡）。
const RESTORE_HISTORY: u32 = 1000;
/// core 叫不起來時最多等多久。
const START_TIMEOUT: Duration = Duration::from_secs(20);
/// 斷線重連的等待（合計約 30 秒）。前兩次只連、不叫 core（holder 會自己把 core 拉起來）。
const BACKOFF_MS: &[u64] = &[100, 300, 700, 1500, 3000, 5000, 8000, 10000];
/// core 一直關掉某個會話的串流（但它還活著）：重新 attach 幾次就放棄。
const MAX_REATTACH: u32 = 5;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 一個會話在 core 裡的連結。
struct Link {
    sid: String,
    address: String,
    /// 現在 attach 的 channel（None＝沒接著：剛斷線、重連中、已結束）
    channel: Mutex<Option<u32>>,
    size: Mutex<(u16, u16)>,
    pid: Mutex<Option<u32>>,
    /// agent 會話才有（config `session.program_status` 關掉也沒有）
    recorder: Mutex<Option<ProgramRecorder>>,
    reattached: AtomicU32,
}

struct Entry {
    sess: Weak<TermSession>,
    link: Arc<Link>,
}

/// herdr-core 會話後端。
pub struct HcBackend {
    setup: HcSetup,
    spine_dir: PathBuf,
    tag: String,
    conn: Mutex<Option<Arc<Client>>>,
    /// core 位址 → 會話
    entries: Mutex<HashMap<String, Entry>>,
    closing: AtomicBool,
    reconnecting: AtomicBool,
    version: Mutex<String>,
    me: Weak<HcBackend>,
}

/// 會話的 I/O（[`SessionIo`]）：輸入是 attach channel 上的 DATA、改大小是 RESIZE、殺掉是 `terminal.kill`。
struct HcIo {
    backend: Arc<HcBackend>,
    link: Arc<Link>,
}

impl SessionIo for HcIo {
    fn backend(&self) -> BackendKind {
        BackendKind::HerdrCore
    }

    fn pid(&self) -> Option<u32> {
        *lock(&self.link.pid)
    }

    /// 輸入延遲路徑：拿連線（一把鎖）→ 拿 channel（一把鎖）→ 一個 frame 一次 write。重連中＝丟掉這次輸入（不擋 UI）。
    fn write(&self, data: &[u8]) -> bool {
        let ch = *lock(&self.link.channel);
        if let (Some(c), Some(ch)) = (self.backend.client(), ch) {
            c.send(ch, Kind::Data, data);
        }
        true
    }

    fn resize(&self, cols: u16, rows: u16) {
        *lock(&self.link.size) = (cols, rows);
        let ch = *lock(&self.link.channel);
        if let (Some(c), Some(ch)) = (self.backend.client(), ch) {
            c.send(ch, Kind::Resize, &frame::resize(cols, rows));
        }
    }

    fn kill(&self) {
        self.backend.kill(&self.link);
    }

    fn detach(&self) {
        let ch = lock(&self.link.channel).take();
        if let (Some(c), Some(ch)) = (self.backend.client(), ch) {
            c.close_stream(ch);
        }
    }
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn u32_of(v: &Value, k: &str) -> Option<u32> {
    v.get(k)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

impl HcBackend {
    /// 找 hc、連上（沒在跑就 `hc server start`）、訂事件。失敗＝原因（manager 退回 PTY）。
    pub fn start(
        spine_dir: &Path,
        hc_path: Option<&Path>,
        session: &str,
        env: Vec<(String, String)>,
    ) -> std::result::Result<Arc<HcBackend>, StartError> {
        let exe = locate::find_hc(hc_path).map_err(|reason| StartError { reason, exe: None })?;
        let setup = HcSetup {
            exe: exe.clone(),
            session: session.to_string(),
            env,
        };
        let b = Arc::new_cyclic(|me| HcBackend {
            setup,
            spine_dir: spine_dir.to_path_buf(),
            tag: spine_tag(spine_dir),
            conn: Mutex::new(None),
            entries: Mutex::new(HashMap::new()),
            closing: AtomicBool::new(false),
            reconnecting: AtomicBool::new(false),
            version: Mutex::new(String::new()),
            me: me.clone(),
        });
        b.connect(true).map_err(|e| StartError {
            reason: format!("連不上 herdr-core：{e}"),
            exe: Some(exe),
        })?;
        Ok(b)
    }

    pub fn exe(&self) -> &Path {
        &self.setup.exe
    }

    /// core 的版本（`hello.version`）。
    pub fn version(&self) -> String {
        lock(&self.version).clone()
    }

    fn client(&self) -> Option<Arc<Client>> {
        lock(&self.conn).clone().filter(|c| c.alive())
    }

    fn lost_callback(&self) -> impl FnOnce(String) + Send + 'static {
        let me = self.me.clone();
        move |why| {
            if let Some(b) = me.upgrade() {
                b.on_lost(why);
            }
        }
    }

    fn open_client(&self, allow_start: bool) -> std::result::Result<Arc<Client>, RpcError> {
        let try_all = |last: &mut Option<RpcError>| {
            for p in self.setup.socket_candidates() {
                match Client::connect(&p, self.lost_callback()) {
                    Ok(c) => return Some(c),
                    Err(e) => *last = Some(e),
                }
            }
            None
        };
        let mut last = None;
        if let Some(c) = try_all(&mut last) {
            return Ok(c);
        }
        if !allow_start {
            return Err(last.unwrap_or_else(|| RpcError::unavailable("沒有可試的 socket")));
        }
        self.setup.start_core(START_TIMEOUT)?;
        // core 剛起來：socket 可能晚一點才出現
        for _ in 0..50 {
            if let Some(c) = try_all(&mut last) {
                return Ok(c);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(last.unwrap_or_else(|| RpcError::unavailable("hc server start 之後還是找不到 socket")))
    }

    /// 連上並訂這個 spine 的事件（狀態用）。
    fn connect(&self, allow_start: bool) -> std::result::Result<(), RpcError> {
        let client = self.open_client(allow_start)?;
        let me = self.me.clone();
        let mut carry: Vec<u8> = Vec::new();
        let events: StreamHandler = Box::new(move |m| {
            let StreamMsg::Data(b) = m else { return };
            carry.extend_from_slice(b);
            let Some(b) = me.upgrade() else { return };
            while let Some(i) = carry.iter().position(|&c| c == b'\n') {
                let line: Vec<u8> = carry.drain(..=i).collect();
                b.on_event(&line);
            }
            // 沒有換行但已經是完整的一則：也收（不靠對方一定送換行）
            if !carry.is_empty() && serde_json::from_slice::<Value>(&carry).is_ok() {
                let line = std::mem::take(&mut carry);
                b.on_event(&line);
            }
            if carry.len() > frame::MAX_PAYLOAD {
                carry.clear();
            }
        });
        let snap = client.call_stream(
            "events.subscribe",
            json!({"labels": {LABEL_SPINE: self.tag}}),
            events,
        )?;
        *lock(&self.version) = client.version().to_string();
        *lock(&self.conn) = Some(client);
        for t in snap
            .get("terminals")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(a) = str_of(t, "address") {
                self.on_status(a, t.get("status").unwrap_or(&Value::Null));
            }
        }
        Ok(())
    }

    fn on_event(&self, line: &[u8]) {
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        let p = &v["params"];
        match str_of(&v, "method") {
            Some("event") if str_of(p, "type") == Some("status") => {
                if let Some(a) = str_of(p, "terminal") {
                    self.on_status(a, p.pointer("/data/status").unwrap_or(&Value::Null));
                }
            }
            // 落後太多：core 給一份新快照
            Some("missed") => {
                for t in p
                    .get("terminals")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(a) = str_of(t, "address") {
                        self.on_status(a, t.get("status").unwrap_or(&Value::Null));
                    }
                }
            }
            _ => {}
        }
    }

    /// core 說這個終端機的程式現在是什麼狀態 → 程式回報檔（跟 PTY 後端的 Scanner 寫同一個檔）。
    fn on_status(&self, address: &str, status: &Value) {
        let Some(link) = lock(&self.entries).get(address).map(|e| e.link.clone()) else {
            return;
        };
        let Some(r) = report_of(status) else { return };
        let mut rec = lock(&link.recorder);
        if let Some(rec) = rec.as_mut() {
            rec.record(r, None);
        }
    }

    fn register(
        self: &Arc<Self>,
        stored: &StoredMeta,
        link: Arc<Link>,
        hook: ExitHook,
    ) -> Arc<TermSession> {
        let io = HcIo {
            backend: self.clone(),
            link: link.clone(),
        };
        let sess = TermSession::from_io(
            stored.sid.clone(),
            stored.term_meta(),
            stored.started.clone(),
            stored.cwd.as_ref().map(PathBuf::from),
            Box::new(io),
            Some(hook),
        );
        lock(&self.entries).insert(
            link.address.clone(),
            Entry {
                sess: Arc::downgrade(&sess),
                link,
            },
        );
        sess
    }

    /// 開一個會話：`terminal.run`（背景、不隨誰結束、帶標籤）→ 存中繼資料 → attach。
    pub fn spawn(
        self: &Arc<Self>,
        sid: &str,
        spec: &SpawnSpec,
        meta: TermMeta,
        agent: Option<&str>,
        hook: ExitHook,
        program: Option<ProgramRecorder>,
    ) -> Result<Arc<TermSession>> {
        if spec.argv.is_empty() {
            return Err(Error::invalid("指令是空的"));
        }
        let client = self.client().ok_or_else(|| {
            Error::External("herdr-core 連線中斷、正在重連；等一下再開會話".into())
        })?;
        let env: Map<String, Value> = session_env(spec)
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        let kind = match meta.kind {
            TermKind::Agent => "agent",
            TermKind::Shell => "shell",
        };
        let rec = client.call(
            "terminal.run",
            json!({
                "argv": spec.argv,
                "cwd": spec.cwd.as_ref().map(|p| p.to_string_lossy().into_owned()),
                "env": env,
                "labels": {LABEL_SID: sid, LABEL_SPINE: self.tag, LABEL_KIND: kind},
                "size": {"cols": spec.cols.max(1), "rows": spec.rows.max(1)},
                "no_parent": true,
                "outlive_parent": true,
                "no_notify": true,
            }),
        )?;
        let address = str_of(&rec, "address")
            .ok_or_else(|| Error::External("herdr-core 的 terminal.run 回覆沒有 address".into()))?
            .to_string();
        let meta = title_or_program(meta, &spec.argv);
        let stored = StoredMeta {
            sid: sid.to_string(),
            address: address.clone(),
            kind: meta.kind,
            title: meta.title.clone(),
            origin: meta.origin.clone(),
            scope: meta.scope.clone(),
            scope_repos: meta.scope_repos.clone(),
            task: meta.task.clone(),
            home_group: meta.home_group.clone(),
            started: now_started(),
            cwd: spec.cwd.as_ref().map(|p| p.to_string_lossy().into_owned()),
            agent: agent.map(str::to_string),
        };
        if let Err(e) = save_meta(&self.spine_dir, &stored) {
            let _ = client.call("terminal.kill", json!({"terminal": address}));
            client.notify("terminal.rm", json!({"terminal": address}));
            return Err(e);
        }
        let link = Arc::new(Link {
            sid: sid.to_string(),
            address,
            channel: Mutex::new(None),
            size: Mutex::new((spec.cols.max(1), spec.rows.max(1))),
            pid: Mutex::new(u32_of(&rec, "pid")),
            recorder: Mutex::new(program),
            reattached: AtomicU32::new(0),
        });
        let sess = self.register(&stored, link.clone(), hook);
        if let Err(e) = self.attach(&sess, &link, 0, false) {
            // 程式可能早就跑完了（attach 不到）：問清楚再決定是「結束了」還是真的接不上
            self.settle(&sess, &link, Some(&e));
        }
        Ok(sess)
    }

    /// attach 一個會話：回覆的 `terminal` 記下 pid 與狀態。`resumed`＝接回來的（不先拿我們的尺寸蓋掉）。
    fn attach(
        self: &Arc<Self>,
        sess: &Arc<TermSession>,
        link: &Arc<Link>,
        history: u32,
        resumed: bool,
    ) -> std::result::Result<(), RpcError> {
        let client = self
            .client()
            .ok_or_else(|| RpcError::unavailable("herdr-core 連線中斷"))?;
        let weak = Arc::downgrade(sess);
        let me = self.me.clone();
        let l2 = link.clone();
        let handler: StreamHandler = Box::new(move |m| match m {
            StreamMsg::Data(b) => {
                if let Some(s) = weak.upgrade() {
                    s.feed(b);
                }
            }
            StreamMsg::Resize(..) => {}
            StreamMsg::Close(reason) => {
                *lock(&l2.channel) = None;
                // 帶原因＝core 關的（程式結束、或 core 要我們重接）；沒帶＝回應我們自己的 detach
                if reason.is_some() {
                    if let (Some(b), Some(s)) = (me.upgrade(), weak.upgrade()) {
                        let l3 = l2.clone();
                        // 讀取執行緒上不能等回覆：另開一條去問
                        let _ = std::thread::Builder::new()
                            .name("hc-settle".into())
                            .spawn(move || b.settle(&s, &l3, None));
                    }
                }
            }
            StreamMsg::Lost(_) => {
                *lock(&l2.channel) = None;
            }
        });
        let (cols, rows) = *lock(&link.size);
        let r = client.call_stream(
            "terminal.attach",
            json!({"terminal": link.address, "cols": cols, "rows": rows,
                   "history": history, "resumed": resumed}),
            handler,
        )?;
        if let Some(ch) = u32_of(&r, "channel") {
            *lock(&link.channel) = Some(ch);
        }
        let t = r.get("terminal").cloned().unwrap_or(Value::Null);
        if let Some(pid) = u32_of(&t, "pid") {
            *lock(&link.pid) = Some(pid);
        }
        self.on_status(&link.address, t.get("status").unwrap_or(&Value::Null));
        Ok(())
    }

    /// 串流沒了（core 關的、或 attach 不到）：問 core 這個終端機現在怎樣。結束了＝收尾；還活著＝重新 attach；
    /// 連線出問題＝不動（重連那邊會處理）。
    fn settle(self: &Arc<Self>, sess: &Arc<TermSession>, link: &Arc<Link>, why: Option<&RpcError>) {
        *lock(&link.channel) = None;
        let Some(client) = self.client() else { return };
        match client.call("terminal.get", json!({"terminal": link.address})) {
            Ok(t) if str_of(&t, "state") == Some("live") => {
                if link.reattached.fetch_add(1, Ordering::SeqCst) >= MAX_REATTACH {
                    let why = why
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "core 一直關掉串流".into());
                    sess.finish(Some(&format!(
                        "\r\n\x1b[2m[接不回這個會話：{why}；它可能還在 herdr-core 裡跑（hc ls）]\x1b[0m"
                    )));
                    return;
                }
                if let Err(e) = self.attach(sess, link, 0, true) {
                    if e.kind != "unavailable" {
                        sess.finish(Some(&format!("\r\n\x1b[2m[接不回這個會話：{e}]\x1b[0m")));
                    }
                }
            }
            Ok(t) => {
                // 跑太快、一次都沒接上（attach 時已經結束）：把它留下的字補進畫面
                if sess.replay().is_empty() {
                    self.feed_leftover(&client, sess, link);
                }
                let normal = str_of(&t, "reason").is_none_or(|r| r == "exited");
                let note = (!normal).then(|| ended_note(&t, false));
                sess.finish(note.as_deref());
                self.forget(link);
            }
            Err(e) if e.kind == "not_found" => {
                sess.finish(Some("\r\n\x1b[2m[herdr-core 裡已經沒有這個終端機]\x1b[0m"));
                self.forget(link);
            }
            Err(_) => {}
        }
    }

    /// 已結束的終端機留下的字（`terminal.read`）補進 ring buffer（換行轉成終端的 CRLF）。
    fn feed_leftover(&self, client: &Client, sess: &TermSession, link: &Link) {
        if let Ok(r) = client.call(
            "terminal.read",
            json!({"terminal": link.address, "lines": RESTORE_HISTORY}),
        ) {
            if let Some(text) = str_of(&r, "text").filter(|t| !t.trim().is_empty()) {
                sess.feed(text.trim_end().replace('\n', "\r\n").as_bytes());
            }
        }
    }

    /// 結束了的會話：從 core 拿掉（`terminal.rm`，不等回覆）、刪中繼資料、不再追蹤。
    fn forget(&self, link: &Link) {
        lock(&self.entries).remove(&link.address);
        remove_meta(&self.spine_dir, &link.sid);
        if let Some(c) = self.client() {
            c.notify("terminal.rm", json!({"terminal": link.address}));
        }
    }

    /// 人按關閉：`terminal.kill`；之後 core 關串流 → [`HcBackend::settle`] 收尾並 `terminal.rm`。
    fn kill(&self, link: &Arc<Link>) {
        let Some(c) = self.client() else { return };
        if let Err(e) = c.call("terminal.kill", json!({"terminal": link.address})) {
            if e.kind == "not_found" {
                let sess = lock(&self.entries)
                    .get(&link.address)
                    .and_then(|e| e.sess.upgrade());
                if let Some(s) = sess {
                    s.finish(None);
                }
                self.forget(link);
            }
        }
    }

    /// 這個 spine 在 core 裡的終端機（含已結束的），對回中繼資料；對不到中繼資料的用標籤補。
    /// 中繼資料檔對不到 core 裡任何終端機的（core 的狀態被清掉了）順手刪掉。列不出來＝空（不刪任何東西）。
    pub fn existing(&self) -> Vec<Existing> {
        let Some(c) = self.client() else {
            return Vec::new();
        };
        let Ok(r) = c.call(
            "terminal.list",
            json!({"all": true, "labels": {LABEL_SPINE: self.tag}}),
        ) else {
            return Vec::new();
        };
        let mut out: Vec<Existing> = Vec::new();
        for t in r
            .get("terminals")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(sid) = t
                .pointer(&format!("/labels/{}", LABEL_SID.replace('/', "~1")))
                .and_then(Value::as_str)
                .filter(|s| {
                    !s.is_empty()
                        && s.chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                })
            else {
                continue;
            };
            let Some(address) = str_of(t, "address") else {
                continue;
            };
            if out.iter().any(|e| e.meta.sid == sid) {
                continue; // 同一個 sid 只接一個
            }
            let meta = load_meta(&self.spine_dir, sid)
                .filter(|m| m.address == address)
                .unwrap_or_else(|| fallback_meta(sid, address, t));
            out.push(Existing {
                meta,
                record: t.clone(),
            });
        }
        if let Ok(rd) = std::fs::read_dir(meta_dir(&self.spine_dir)) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let Some(sid) = name.strip_suffix(".json") else {
                    continue;
                };
                if !out.iter().any(|x| x.meta.sid == sid) {
                    remove_meta(&self.spine_dir, sid);
                }
            }
        }
        out
    }

    /// 接回一個：活著的 attach（帶捲動歷史）；App 關著期間結束的＝照結束處理一次（結束 hook）、從 core 拿掉。
    pub fn adopt(
        self: &Arc<Self>,
        e: Existing,
        hook: ExitHook,
        program: Option<ProgramRecorder>,
    ) -> Option<Arc<TermSession>> {
        let t = &e.record;
        let link = Arc::new(Link {
            sid: e.meta.sid.clone(),
            address: e.meta.address.clone(),
            channel: Mutex::new(None),
            size: Mutex::new((
                t.get("cols")
                    .and_then(Value::as_u64)
                    .and_then(|n| u16::try_from(n).ok())
                    .unwrap_or(crate::term::DEFAULT_COLS),
                t.get("rows")
                    .and_then(Value::as_u64)
                    .and_then(|n| u16::try_from(n).ok())
                    .unwrap_or(crate::term::DEFAULT_ROWS),
            )),
            pid: Mutex::new(u32_of(t, "pid")),
            recorder: Mutex::new(program),
            reattached: AtomicU32::new(0),
        });
        let live = e.live();
        if live {
            // 中繼資料檔可能是用標籤補的：存一份
            let _ = save_meta(&self.spine_dir, &e.meta);
        }
        let sess = self.register(&e.meta, link.clone(), hook);
        if !live {
            if let Some(c) = self.client() {
                self.feed_leftover(&c, &sess, &link);
            }
            sess.finish(Some(&ended_note(t, true)));
            self.forget(&link);
            return Some(sess);
        }
        if let Err(err) = self.attach(&sess, &link, RESTORE_HISTORY, true) {
            self.settle(&sess, &link, Some(&err));
        }
        Some(sess)
    }

    /// 連線意外斷了：退避重連（背景執行緒）。我們自己關的不算（`closing`）。
    fn on_lost(&self, why: String) {
        if self.closing.load(Ordering::SeqCst) {
            return;
        }
        *lock(&self.conn) = None;
        if self.reconnecting.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.me.clone();
        let _ = std::thread::Builder::new()
            .name("hc-reconnect".into())
            .spawn(move || {
                let mut last = why;
                for (i, ms) in BACKOFF_MS.iter().enumerate() {
                    std::thread::sleep(Duration::from_millis(*ms));
                    let Some(b) = me.upgrade() else { return };
                    if b.closing.load(Ordering::SeqCst) {
                        return;
                    }
                    match b.connect(i >= 2) {
                        Ok(()) => {
                            b.reconnecting.store(false, Ordering::SeqCst);
                            b.resync();
                            return;
                        }
                        Err(e) => last = e.to_string(),
                    }
                }
                let Some(b) = me.upgrade() else { return };
                let lost: Vec<Arc<TermSession>> = lock(&b.entries)
                    .drain()
                    .filter_map(|(_, e)| e.sess.upgrade())
                    .collect();
                for s in lost {
                    s.finish(Some(&format!(
                        "\r\n\x1b[2m[和 herdr-core 斷線、重連不上（{last}）；會話可能還在 core 裡跑，重開 App 會再接回]\x1b[0m"
                    )));
                }
                b.reconnecting.store(false, Ordering::SeqCst);
            });
    }

    /// 重連之後：對一遍 core 的清單，活著的重新 attach（只要畫面，歷史已經在 ring buffer 裡），結束的收尾。
    fn resync(self: &Arc<Self>) {
        let Some(c) = self.client() else { return };
        let Ok(r) = c.call(
            "terminal.list",
            json!({"all": true, "labels": {LABEL_SPINE: self.tag}}),
        ) else {
            return;
        };
        let records: Vec<Value> = r
            .get("terminals")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let entries: Vec<(Arc<TermSession>, Arc<Link>)> = lock(&self.entries)
            .values()
            .filter_map(|e| Some((e.sess.upgrade()?, e.link.clone())))
            .collect();
        for (s, link) in entries {
            if !s.alive() {
                continue;
            }
            match records
                .iter()
                .find(|t| str_of(t, "address") == Some(&link.address))
            {
                None => {
                    s.finish(Some("\r\n\x1b[2m[herdr-core 裡已經沒有這個終端機]\x1b[0m"));
                    self.forget(&link);
                }
                Some(t) if str_of(t, "state") != Some("live") => {
                    let normal = str_of(t, "reason").is_none_or(|r| r == "exited");
                    let note = (!normal).then(|| ended_note(t, false));
                    s.finish(note.as_deref());
                    self.forget(&link);
                }
                Some(_) => {
                    if lock(&link.channel).is_none() {
                        if let Err(e) = self.attach(&s, &link, 0, true) {
                            self.settle(&s, &link, Some(&e));
                        }
                    }
                }
            }
        }
    }

    /// App 結束：放手（關掉每條 attach、關掉連線），不殺任何程式、不跑結束 hook。
    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        let conn = lock(&self.conn).take();
        if let Some(c) = conn {
            let chans: Vec<u32> = lock(&self.entries)
                .values()
                .filter_map(|e| lock(&e.link.channel).take())
                .collect();
            for ch in chans {
                c.close_stream(ch);
            }
            c.shutdown();
        }
    }
}

impl Drop for HcBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 對不到中繼資料檔時，從 core 的紀錄補（標題、目錄、種類）。
fn fallback_meta(sid: &str, address: &str, t: &Value) -> StoredMeta {
    let kind = match t
        .pointer(&format!("/labels/{}", LABEL_KIND))
        .and_then(Value::as_str)
    {
        Some("agent") => TermKind::Agent,
        _ => TermKind::Shell,
    };
    let title = str_of(t, "title")
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| match kind {
            TermKind::Agent => "agent".into(),
            TermKind::Shell => "shell".into(),
        });
    let started = t
        .get("created_ms")
        .and_then(Value::as_i64)
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(now_started);
    StoredMeta {
        sid: sid.to_string(),
        address: address.to_string(),
        kind,
        title,
        origin: None,
        scope: None,
        scope_repos: None,
        task: None,
        home_group: None,
        started,
        cwd: str_of(t, "cwd").map(str::to_string),
        agent: None,
    }
}
