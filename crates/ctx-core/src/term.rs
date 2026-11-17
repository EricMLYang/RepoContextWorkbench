//! 內嵌 terminal——工作台的底板。對應 `repo_context/term.py`。
//!
//! 2026-09-01 使用者裁定：要 IDE 風格工作台，可在其中直接與 agent 對話（取代 v2 §7「不內嵌 terminal」）。
//!
//! PTY 會話管理（Rust 版用 `portable-pty`：macOS／Linux 走 openpty、Windows 走 ConPTY，一套程式）：
//! - 會話生命週期獨立於連線：工作台視窗關掉／重連，agent 對話不死（除非人按關閉）；
//!   重連 replay ring buffer（保留最後 ~200KB 輸出）。
//! - 輸出以 bytes 廣播給訂閱者（不經解碼——TUI 的 escape sequence 與半個 UTF-8 序列都不會被折斷）。
//! - replay 在 session 鎖內交給新訂閱者，之後的即時輸出才會進來：同一個訂閱者看到的順序不亂。
//!
//! 與 Python 版的分工差異：agent 會話的指令（`session.build` 打包＋prompt）由呼叫端組好再交進來
//! （[`AgentLaunch`]），term 只管 PTY、登入 shell 包裝與 agentmark 標記——不依賴 session 模組。

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};

use crate::program_status::{ProgramEvent, Scanner, QUERY_REPLY};
use crate::session_status::ProgramRecorder;
use crate::{agentmark, Error, Result};

/// ring buffer 上限（位元組）。對應 `term.BUFFER_CAP`。
pub const BUFFER_CAP: usize = 200_000;
/// 程序結束時附在輸出尾巴的提示。對應 `term._EXIT_MSG`。
pub const EXIT_MSG: &str = "\r\n\x1b[2m[進程已結束——關這個分頁或重開一個會話]\x1b[0m\r\n";

/// 預設終端尺寸（與 Python `TermSession(cols=120, rows=32)` 同）。
pub const DEFAULT_COLS: u16 = 120;
pub const DEFAULT_ROWS: u16 = 32;

/// 訂閱者收到的事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermEvent<'a> {
    /// PTY 輸出（attach 當下的第一筆是 replay，可能是空的）。
    Data(&'a [u8]),
    /// 程序結束（[`EXIT_MSG`] 已經以 `Data` 送過）。之後不會再有事件。
    Exited,
}

/// 訂閱者：回傳 `false` 表示送不出去（連線斷了），session 會把它拆掉——對應 Python cb 丟例外就 detach。
pub type Subscriber = Box<dyn FnMut(TermEvent<'_>) -> bool + Send>;

/// attach 的識別碼（`term_close` 用它只拆掉那一條連線）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AttachId(pub u64);

impl std::fmt::Display for AttachId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a{}", self.0)
    }
}

impl std::str::FromStr for AttachId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        s.strip_prefix('a')
            .and_then(|n| n.parse().ok())
            .map(AttachId)
            .ok_or_else(|| Error::invalid(format!("attach id 不合法: {s}")))
    }
}

/// 會話種類（序列化字串與 Python `kind` 相同）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TermKind {
    Shell,
    Agent,
}

/// 要在 PTY 裡跑的指令（已組好的 argv＋cwd＋額外環境變數）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

impl SpawnSpec {
    pub fn new(argv: Vec<String>) -> Self {
        SpawnSpec {
            argv,
            cwd: None,
            env: Vec::new(),
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
        }
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }
}

/// 會話的描述欄位（Python 版是建好 TermSession 後再掛上的屬性）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermMeta {
    pub kind: TermKind,
    pub title: String,
    /// 由哪張卡／哪次碰撞開的（卡 key 或 `collision:<cid>`）。
    pub origin: Option<String>,
    pub scope: Option<String>,
    pub scope_repos: Option<Vec<String>>,
    pub task: Option<String>,
    /// Rust 才有（2026-10-11）：所屬牌組，只給畫面歸類（會話列放進牌組區塊、終端標題「組 › repo」）。
    /// 跟 `scope` 不同：從某組裡對單一 repo 開的會話，`scope` 是 repo、這裡是那個組。
    pub home_group: Option<String>,
}

impl TermMeta {
    pub fn shell(title: impl Into<String>) -> Self {
        TermMeta {
            kind: TermKind::Shell,
            title: title.into(),
            origin: None,
            scope: None,
            scope_repos: None,
            task: None,
            home_group: None,
        }
    }
}

/// `TermManager.list()` 的一筆（欄位名與 Python 相同）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TermInfo {
    pub sid: String,
    pub title: String,
    pub alive: bool,
    pub kind: TermKind,
    pub origin: Option<String>,
    pub scope: Option<String>,
    pub scope_repos: Option<Vec<String>>,
    pub started: String,
    pub task: Option<String>,
    /// 見 [`TermMeta::home_group`]；沒有就不輸出（欄位與 Python 版相同）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_group: Option<String>,
}

/// 會話跑在哪個後端（F-128，Rust 才有）。config `session.backend` 的值就是 [`BackendKind::as_str`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackendKind {
    /// 行程內 PTY（portable-pty）：會話跟 App 同生共死。預設。
    #[serde(rename = "pty")]
    Pty,
    /// herdr-core（`hc`）：會話跑在 App 之外，App 結束、當掉、換版後重開都接得回來。
    #[serde(rename = "herdr-core")]
    HerdrCore,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Pty => "pty",
            BackendKind::HerdrCore => "herdr-core",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "pty" => Some(BackendKind::Pty),
            "herdr-core" | "herdr" | "hc" => Some(BackendKind::HerdrCore),
            _ => None,
        }
    }
}

/// 一個會話後端的 I/O 介面（F-128）：寫入、改大小、殺掉、放手。
///
/// 輸出不經過這個介面：後端把讀到的位元組交給 [`TermSession::feed`]、程序結束時呼叫 [`TermSession::finish`]，
/// ring buffer、訂閱者、「先 replay 再即時」的順序保證都在 [`TermSession`] 這一層，兩個後端共用一份。
/// 開會話與列出現存會話是後端自己的建構函式（PTY：[`TermSession::spawn_with_status`]；herdr-core：`hc::HcBackend`）。
pub(crate) trait SessionIo: Send + Sync {
    fn backend(&self) -> BackendKind;
    fn pid(&self) -> Option<u32>;
    /// 寫進程式的輸入。回 `false`＝再也寫不進去了（程式已經不在）；暫時寫不進去（重連中）要回 `true`。
    fn write(&self, data: &[u8]) -> bool;
    fn resize(&self, cols: u16, rows: u16);
    /// 結束程式（人按關閉）。
    fn kill(&self);
    /// 輸出已經結束（[`TermSession::finish`] 呼叫）：放掉寫入端之類的資源。
    fn closed(&self) {}
    /// App 結束：放手但不殺（只有跑在 App 之外的後端有意義）。
    fn detach(&self) {}
}

struct Shared {
    buf: VecDeque<Vec<u8>>,
    buf_len: usize,
    subs: Vec<(AttachId, Subscriber)>,
    next_attach: u64,
}

pub(crate) type ExitHook = Box<dyn FnOnce() + Send>;

/// 一個會話。對應 `term.TermSession`。後端（PTY 或 herdr-core）只管 I/O，見 [`SessionIo`]。
pub struct TermSession {
    sid: String,
    meta: TermMeta,
    started: String,
    /// 開在哪個目錄（給「這個會話的 Terminal」沿用；不輸出到 `TermInfo`）。
    cwd: Option<PathBuf>,
    alive: AtomicBool,
    /// 結束收尾做過了沒（[`TermSession::finish`] 只做一次）
    ended: AtomicBool,
    shared: Mutex<Shared>,
    io: Box<dyn SessionIo>,
    on_exit: Mutex<Option<ExitHook>>,
}

fn joined(buf: &VecDeque<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::with_capacity(buf.iter().map(Vec::len).sum());
    for chunk in buf {
        out.extend_from_slice(chunk);
    }
    out
}

/// 上一層 Claude Code 會話留在本程序環境裡的身分變數。
///
/// App 若是從某場 Claude 會話裡 `open` 出來的（例如 `install-app.sh --reopen`），macOS 會把呼叫端的環境
/// 整包交給 App；`CommandBuilder::new` 又預設複製本程序環境，於是 App 開的每一場 claude 都以為自己是
/// 那場的子會話（`CLAUDE_CODE_CHILD_SESSION`）——不存 transcript，之後 `/resume` 找不到。
/// PTY 裡跑的會話是獨立的一場，這組變數一律不往下傳（herdr-core 後端：叫起 core 時就拿掉，見 `hc::locate`）。
pub const CLAUDE_SESSION_ENV: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_PID",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_SESSION_ATTENDED",
];

fn scrub_claude_session_env(cmd: &mut CommandBuilder) {
    for k in CLAUDE_SESSION_ENV {
        cmd.env_remove(k);
    }
}

/// 每個會話都帶的環境變數（先放這些，再放 [`SpawnSpec::env`]——呼叫端給的優先）。兩個後端共用。
pub(crate) fn session_env(spec: &SpawnSpec) -> Vec<(String, String)> {
    let mut env = vec![
        ("PYTHONUTF8".to_string(), "1".to_string()),
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
    ];
    env.extend(spec.env.iter().cloned());
    env
}

/// 沒給標題＝argv[0] 的檔名。
pub(crate) fn title_or_program(meta: TermMeta, argv: &[String]) -> TermMeta {
    if !meta.title.is_empty() {
        return meta;
    }
    let prog = argv.first().cloned().unwrap_or_default();
    let title = Path::new(&prog)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or(prog);
    TermMeta { title, ..meta }
}

/// 現在的時間（會話開始時間的格式）。
pub(crate) fn now_started() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // 訂閱者 panic 不該讓整個會話卡死：毒化的鎖照樣拿來用
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 行程內 PTY 後端（portable-pty）。
struct PtyIo {
    pid: Option<u32>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    /// 子程序已被回收（waiter 執行緒設）。kill 等它決定要不要補 SIGKILL。
    reaped: Arc<(Mutex<bool>, Condvar)>,
}

impl PtyIo {
    fn is_reaped(&self) -> bool {
        *lock(&self.reaped.0)
    }

    /// 等子程序被回收，最多 `timeout`。回傳是否已回收。
    fn wait_reaped(&self, timeout: Duration) -> bool {
        let (m, cv) = &*self.reaped;
        let g = lock(m);
        let (g, _) = cv
            .wait_timeout_while(g, timeout, |done| !*done)
            .unwrap_or_else(|e| e.into_inner());
        *g
    }
}

impl SessionIo for PtyIo {
    fn backend(&self) -> BackendKind {
        BackendKind::Pty
    }

    fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn write(&self, data: &[u8]) -> bool {
        let mut w = lock(&self.writer);
        match w.as_mut() {
            Some(wr) => wr.write_all(data).and_then(|_| wr.flush()).is_ok(),
            None => true, // 輸出已結束（finish 會把會話標成結束），這裡不必再報
        }
    }

    /// 對應 `TermSession.resize`：失敗不報錯（Python 吞 OSError）。
    fn resize(&self, cols: u16, rows: u16) {
        let _ = lock(&self.master).resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    /// 對應 `TermSession.kill`：先客氣（SIGHUP＋SIGTERM），3 秒內沒走就 SIGKILL。Windows 走 ConPTY 的 terminate。
    fn kill(&self) {
        if self.is_reaped() {
            return;
        }
        #[cfg(unix)]
        if let Some(pid) = self.pid.and_then(|p| libc::pid_t::try_from(p).ok()) {
            // 互動式 shell 會忽略 SIGTERM（Python 版因此常要等滿 3 秒）；先送 SIGHUP——
            // 等同「關掉終端機」，shell 與 claude 都會收尾離開。
            // SAFETY: 對自己 spawn 的子程序送訊號；pid 由 waiter 回收前不會被重用。
            unsafe {
                libc::kill(pid, libc::SIGHUP);
                libc::kill(pid, libc::SIGTERM);
            }
            if self.wait_reaped(Duration::from_secs(3)) {
                return;
            }
        }
        let _ = lock(&self.killer).kill();
    }

    fn closed(&self) {
        *lock(&self.writer) = None;
    }
}

impl TermSession {
    /// 對應 `TermSession.__init__`：開 PTY、spawn、起 reader 執行緒。`on_exit` 在程序自然結束後呼叫。
    pub fn spawn(
        sid: impl Into<String>,
        spec: &SpawnSpec,
        meta: TermMeta,
        on_exit: Option<ExitHook>,
    ) -> Result<Arc<TermSession>> {
        Self::spawn_with_status(sid, spec, meta, on_exit, None)
    }

    /// 同 [`TermSession::spawn`]，`program`＝要讀這個會話的 OSC 7501（F-130）：reader 執行緒掃輸出、
    /// 回應探詢、把回報交給這個寫入器。None＝不解析也不回應（shell 會話、config 關掉時）。
    pub fn spawn_with_status(
        sid: impl Into<String>,
        spec: &SpawnSpec,
        meta: TermMeta,
        on_exit: Option<ExitHook>,
        program: Option<ProgramRecorder>,
    ) -> Result<Arc<TermSession>> {
        let sid = sid.into();
        let Some(prog) = spec.argv.first() else {
            return Err(Error::invalid("指令是空的"));
        };
        let pty = portable_pty::native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: spec.rows,
                cols: spec.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::External(format!("開 PTY 失敗：{e}")))?;
        let mut cmd = CommandBuilder::new(prog);
        cmd.args(&spec.argv[1..]);
        if let Some(cwd) = &spec.cwd {
            cmd.cwd(cwd);
        }
        scrub_claude_session_env(&mut cmd);
        for (k, v) in session_env(spec) {
            cmd.env(k, v);
        }
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| Error::External(format!("啟動 {prog} 失敗：{e}")))?;
        // slave 端交給子程序後自己這份要關，否則子程序結束時 master 讀不到 EOF
        drop(pair.slave);
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| Error::External(format!("PTY 讀取端：{e}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| Error::External(format!("PTY 寫入端：{e}")))?;
        let reaped = Arc::new((Mutex::new(false), Condvar::new()));
        let io = PtyIo {
            pid: child.process_id(),
            writer: Mutex::new(Some(writer)),
            master: Mutex::new(pair.master),
            killer: Mutex::new(child.clone_killer()),
            reaped: reaped.clone(),
        };
        // 開始時間＝之後判斷「這個會話期間有沒有留痕」的起點（2026-09-12 檢討 §5）
        let sess = Self::from_io(
            sid,
            title_or_program(meta, &spec.argv),
            now_started(),
            spec.cwd.clone(),
            Box::new(io),
            on_exit,
        );
        // waiter：回收子程序（不留 zombie），並讓 kill 知道它死了沒
        std::thread::Builder::new()
            .name(format!("term-wait-{}", sess.sid))
            .spawn(move || {
                let _ = child.wait();
                let (m, cv) = &*reaped;
                *lock(m) = true;
                cv.notify_all();
            })?;
        let s2 = sess.clone();
        std::thread::Builder::new()
            .name(format!("term-read-{}", sess.sid))
            .spawn(move || s2.reader_loop(reader, program))?;
        Ok(sess)
    }

    /// 用一個已經開好的後端 I/O 建會話（輸出之後由後端呼叫 [`TermSession::feed`]／[`TermSession::finish`] 餵進來）。
    pub(crate) fn from_io(
        sid: String,
        meta: TermMeta,
        started: String,
        cwd: Option<PathBuf>,
        io: Box<dyn SessionIo>,
        on_exit: Option<ExitHook>,
    ) -> Arc<TermSession> {
        Arc::new(TermSession {
            sid,
            meta,
            started,
            cwd,
            alive: AtomicBool::new(true),
            ended: AtomicBool::new(false),
            shared: Mutex::new(Shared {
                buf: VecDeque::new(),
                buf_len: 0,
                subs: Vec::new(),
                next_attach: 1,
            }),
            io,
            on_exit: Mutex::new(on_exit),
        })
    }

    fn reader_loop(&self, mut reader: Box<dyn Read + Send>, program: Option<ProgramRecorder>) {
        let mut buf = vec![0u8; 65536];
        let mut status = program.map(|rec| (Scanner::new(), rec));
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => self.on_output(&buf[..n], status.as_mut()),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break, // macOS／Linux：slave 全關後讀 master 是 EIO
            }
        }
        self.finish(None);
    }

    /// 程序結束（後端偵測到的）：結束提示進 ring buffer、通知訂閱者、放掉 I/O、跑結束 hook。只做一次。
    /// `note`＝結束提示前面多一行說明（例：「在 App 關著的時候結束了」）。
    pub(crate) fn finish(&self, note: Option<&str>) {
        {
            // 結束三件事在同一把鎖裡做完：之後 attach 的人一定拿得到含結束提示的 replay
            let mut sh = lock(&self.shared);
            if self.ended.swap(true, Ordering::SeqCst) {
                return;
            }
            self.alive.store(false, Ordering::SeqCst);
            if let Some(n) = note {
                Self::push_locked(&mut sh, n.as_bytes());
            }
            Self::push_locked(&mut sh, EXIT_MSG.as_bytes());
            for (_, cb) in sh.subs.iter_mut() {
                cb(TermEvent::Exited);
            }
            sh.subs.clear();
        }
        self.io.closed();
        let hook = lock(&self.on_exit).take();
        if let Some(f) = hook {
            f();
        }
    }

    /// 一段即時輸出（只有 reader 執行緒會來；attach 時的 replay 不經過這裡，所以重播不會再回應探詢）：
    /// 先掃 OSC 7501、有探詢就回，再原樣往前端送，最後記回報。
    fn on_output(&self, data: &[u8], status: Option<&mut (Scanner, ProgramRecorder)>) {
        let Some((scanner, rec)) = status else {
            self.push(data);
            return;
        };
        let mut queries = 0usize;
        let mut reports = Vec::new();
        scanner.feed(data, |ev| match ev {
            ProgramEvent::Query => queries += 1,
            ProgramEvent::Report(r) => reports.push(r),
        });
        // 探詢的回應要在 push 之前寫回 PTY：Claude Code 送完 `OSC 7501;?` 緊接著送 DA1（`ESC [ c`），
        // 它只認「7501 的回應比 DA1 的回應先到」（2026-11-18 實測：順序反過來，整場都不送狀態）。
        // DA1 是前端 xterm.js 收到這段輸出後才回的（經 IPC，比較晚），所以引擎在這裡、往前端送之前就回，順序自然是對的。
        // `write` 自己會跳過已結束的會話。
        for _ in 0..queries {
            self.write(QUERY_REPLY);
        }
        self.push(data);
        for r in reports {
            rec.record(r, None);
        }
    }

    /// 後端讀到的即時輸出（herdr-core：attach 串流的第一畫面與之後的位元組）。
    pub(crate) fn feed(&self, data: &[u8]) {
        if !data.is_empty() {
            self.push(data);
        }
    }

    /// 對應 `TermSession._push`：進 ring buffer（超過上限丟最舊、至少留一塊）並廣播。
    fn push(&self, data: &[u8]) {
        Self::push_locked(&mut lock(&self.shared), data);
    }

    fn push_locked(sh: &mut Shared, data: &[u8]) {
        sh.buf.push_back(data.to_vec());
        sh.buf_len += data.len();
        while sh.buf_len > BUFFER_CAP && sh.buf.len() > 1 {
            if let Some(old) = sh.buf.pop_front() {
                sh.buf_len -= old.len();
            }
        }
        // 在鎖內送：保證 attach 時的 replay 與之後的即時輸出順序一致
        sh.subs.retain_mut(|(_, cb)| cb(TermEvent::Data(data)));
    }

    pub fn sid(&self) -> &str {
        &self.sid
    }

    pub fn title(&self) -> &str {
        &self.meta.title
    }

    pub fn meta(&self) -> &TermMeta {
        &self.meta
    }

    pub fn started(&self) -> &str {
        &self.started
    }

    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }

    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// 這個會話跑在哪個後端。
    pub fn backend(&self) -> BackendKind {
        self.io.backend()
    }

    /// 對應 `TermSession.pid`。
    pub fn pid(&self) -> Option<u32> {
        self.io.pid()
    }

    /// 對應 `TermSession.write`：程序已結束就靜默丟掉。
    pub fn write(&self, data: &[u8]) {
        if !self.alive() {
            return;
        }
        if !self.io.write(data) {
            self.alive.store(false, Ordering::SeqCst);
        }
    }

    /// 對應 `TermSession.resize`：失敗不報錯（Python 吞 OSError）。
    pub fn resize(&self, cols: u16, rows: u16) {
        self.io.resize(cols, rows);
    }

    /// 目前 ring buffer 的內容（replay）。
    pub fn replay(&self) -> Vec<u8> {
        joined(&lock(&self.shared).buf)
    }

    /// 對應 `TermSession.attach`：註冊訂閱者，**先**以 `Data(replay)` 呼叫它一次（在鎖內），之後才收即時輸出。
    pub fn attach(&self, mut cb: Subscriber) -> AttachId {
        let mut sh = lock(&self.shared);
        let id = AttachId(sh.next_attach);
        sh.next_attach += 1;
        let replay = joined(&sh.buf);
        if cb(TermEvent::Data(&replay)) && !self.ended.load(Ordering::SeqCst) {
            sh.subs.push((id, cb));
        }
        id
    }

    /// 對應 `TermSession.detach`。回傳是否真的拆掉了一個。
    pub fn detach(&self, id: AttachId) -> bool {
        let mut sh = lock(&self.shared);
        let before = sh.subs.len();
        sh.subs.retain(|(a, _)| *a != id);
        sh.subs.len() != before
    }

    /// 目前掛著幾條連線。
    pub fn attach_count(&self) -> usize {
        lock(&self.shared).subs.len()
    }

    /// 對應 `TermSession.kill`：結束程式（PTY：SIGHUP＋SIGTERM、3 秒後 SIGKILL；herdr-core：`terminal.kill`）。
    pub fn kill(&self) {
        self.alive.store(false, Ordering::SeqCst);
        self.io.kill();
    }

    /// App 結束：跑在 App 之外的會話放手不殺；行程內的照舊殺掉。
    pub(crate) fn release(&self) {
        match self.io.backend() {
            BackendKind::Pty => self.kill(),
            BackendKind::HerdrCore => self.io.detach(),
        }
    }

    /// `list()` 的一筆。
    pub fn info(&self) -> TermInfo {
        TermInfo {
            sid: self.sid.clone(),
            title: self.meta.title.clone(),
            alive: self.alive(),
            kind: self.meta.kind,
            origin: self.meta.origin.clone(),
            scope: self.meta.scope.clone(),
            scope_repos: self.meta.scope_repos.clone(),
            started: self.started.clone(),
            task: self.meta.task.clone(),
            home_group: self.meta.home_group.clone(),
        }
    }
}

/// 總管會話的 origin（F-127）：標題與範圍標成「總管」，會話列認得出來。
pub const STEWARD_ORIGIN: &str = "steward";

/// 對應 `term.agent_title`：會話標題＝任務摘要（2026-09-02 檢討 F5：會話要有主，開三個分得出誰是誰）。
/// 沒任務才退回 `agent · scope`。`limit` 以字元計。
pub fn agent_title(agent: &str, scope: &str, task: Option<&str>, limit: usize) -> String {
    if let Some(task) = task.filter(|t| !t.is_empty()) {
        let trimmed = task.trim();
        let head = crate::pystr::splitlines(trimmed)
            .first()
            .copied()
            .unwrap_or("");
        let t = format!("{agent} · {head}");
        if t.chars().count() <= limit {
            return t;
        }
        let cut: String = t.chars().take(limit.saturating_sub(1)).collect();
        return cut + "…";
    }
    format!("{agent} · {scope}")
}

/// 對應 `term.default_shell`。
pub fn default_shell() -> Vec<String> {
    if cfg!(windows) {
        return vec![std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())];
    }
    let sh = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| crate::exe::which("zsh").map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "/bin/sh".into());
    vec![sh]
}

/// Python `shlex.quote`。
pub fn shlex_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    let safe = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }
}

/// 經登入 shell 跑：PATH／profile（~/.local/bin、npm-global…）天然正確，不受「工作台是怎麼被啟動的」影響；
/// command not found 也會顯示在終端裡。Windows 原樣回傳。對應 `TermManager.create_agent` 裡的包裝。
pub fn login_shell_wrap(argv: &[String]) -> Vec<String> {
    if cfg!(windows) {
        return argv.to_vec();
    }
    let sh = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/zsh".into());
    let joined: Vec<String> = argv.iter().map(|a| shlex_quote(a)).collect();
    vec![sh, "-lc".into(), format!("exec {}", joined.join(" "))]
}

/// 開 agent 會話所需的一切（指令已由呼叫端用 `session::build` 組好）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentLaunch {
    /// agent 名（config `agents` 的鍵，例 `claude`）。
    pub agent: String,
    /// 已組好的 agent 指令（尚未包登入 shell）。
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub group: Option<String>,
    pub repos: Option<Vec<String>>,
    pub repo: Option<String>,
    pub task: Option<String>,
    pub origin: Option<String>,
    /// 所屬牌組（畫面歸類用，見 [`TermMeta::home_group`]）；沒給就用 `group`。
    pub home_group: Option<String>,
}

type ExitListener = Arc<dyn Fn(&str) + Send + Sync>;

/// 結束通知：有人聽就直接通知；還沒人聽（manager 剛建好時，接回發現在 App 關著期間就結束的會話）先排著，
/// [`TermManager::set_on_exit`] 時補發——那些會話也照「結束」處理一次。
#[derive(Default)]
struct ExitNotice {
    listener: Option<ExitListener>,
    queued: Vec<String>,
}

/// 會話後端的現況（F-128，Rust 才有）：設定要哪個、實際用哪個、為什麼退回。
/// App 的 `get_state` 帶 `session_backend`（前端在會話區畫 `warning`）；`ctx health` 讀 `.state/session_backend.json`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendStatus {
    /// config `session.backend` 要的
    pub configured: BackendKind,
    /// 這次真的在用的（設成 herdr-core 但找不到 hc、或叫不起來＝退回 pty）
    pub active: BackendKind,
    /// 要讓使用者看到的一行（退回的原因、設定寫錯）；沒事＝不輸出
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hc_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hc_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hc_session: Option<String>,
    /// App 什麼時候決定的（`YYYY-MM-DD HH:MM`）
    pub at: String,
}

impl BackendStatus {
    fn plain(kind: BackendKind) -> Self {
        BackendStatus {
            configured: kind,
            active: kind,
            warning: None,
            hc_path: None,
            hc_version: None,
            hc_session: None,
            at: now_started(),
        }
    }
}

/// `.state/session_backend.json`：App 上次開會話清冊時用的後端（`ctx health` 讀）。
pub fn backend_status_path(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("session_backend.json")
}

/// 讀 App 上次記的後端現況；沒有或壞掉＝None。
pub fn read_backend_status(spine_dir: &Path) -> Option<BackendStatus> {
    let s = std::fs::read_to_string(backend_status_path(spine_dir)).ok()?;
    serde_json::from_str(&s).ok()
}

enum Backend {
    Pty,
    Hc(Arc<crate::hc::HcBackend>),
}

/// 建 [`TermManager`] 的額外選項。
#[derive(Debug, Clone, Default)]
pub struct TermOptions {
    /// 叫 `hc` 時另外帶的環境變數——只給測試隔離 herdr-core 用（`XDG_STATE_HOME`、`XDG_RUNTIME_DIR`…）。
    pub hc_env: Vec<(String, String)>,
}

/// 工作台的會話清冊。對應 `term.TermManager`。agent 會話帶料、shell 會話裸開。
pub struct TermManager {
    spine_dir: PathBuf,
    sessions: Mutex<Vec<Arc<TermSession>>>,
    ids: AtomicU64,
    on_exit: Arc<Mutex<ExitNotice>>,
    backend: Backend,
    status: BackendStatus,
}

impl TermManager {
    /// 新的會話清冊。後端照 config `session.backend`（預設行程內 PTY）。
    /// - PTY：sid 從 t1 重新編，前一次（App 上次開著時）留下的會話狀態檔一併清掉，免得新的 t1 撿到舊的「等你」。
    /// - herdr-core：先把這個 spine 還在 core 裡的會話接回來（狀態檔留著），只清掉別的；sid 接著編。
    pub fn new(spine_dir: impl Into<PathBuf>) -> Self {
        Self::with_options(spine_dir, TermOptions::default())
    }

    pub fn with_options(spine_dir: impl Into<PathBuf>, opts: TermOptions) -> Self {
        let spine_dir = spine_dir.into();
        let (backend, status) = choose_backend(&spine_dir, &opts);
        let m = TermManager {
            ids: AtomicU64::new(crate::hc::next_free_id(&spine_dir)),
            spine_dir,
            sessions: Mutex::new(Vec::new()),
            on_exit: Arc::new(Mutex::new(ExitNotice::default())),
            backend,
            status,
        };
        match &m.backend {
            Backend::Pty => crate::session_status::clear_all(&m.spine_dir),
            Backend::Hc(hc) => m.restore(hc.clone()),
        }
        let _ = std::fs::create_dir_all(m.spine_dir.join(".state"));
        if let Ok(b) = serde_json::to_vec(&m.status) {
            let _ = crate::fsutil::write_atomic(&backend_status_path(&m.spine_dir), &b);
        }
        m
    }

    pub fn spine_dir(&self) -> &Path {
        &self.spine_dir
    }

    /// 這次用的後端與退回的原因（F-128）。
    pub fn backend_status(&self) -> &BackendStatus {
        &self.status
    }

    /// 任一會話程序結束時呼叫（參數是 sid）——桌面殼拿來發 `workbench://changed`。
    /// 設之前就結束的（接回時發現早已結束的 herdr-core 會話）在這裡補發一次。
    pub fn set_on_exit(&self, f: impl Fn(&str) + Send + Sync + 'static) {
        let f: ExitListener = Arc::new(f);
        let queued = {
            let mut n = lock(&self.on_exit);
            n.listener = Some(f.clone());
            std::mem::take(&mut n.queued)
        };
        for sid in queued {
            f(&sid);
        }
    }

    fn next_sid(&self) -> String {
        format!("t{}", self.ids.fetch_add(1, Ordering::SeqCst))
    }

    /// 接回來的 sid 之後才編新的（`t7` 接回來了，下一個至少是 `t8`）。
    fn bump_ids(&self, sid: &str) {
        if let Some(n) = sid.strip_prefix('t').and_then(|n| n.parse::<u64>().ok()) {
            self.ids.fetch_max(n + 1, Ordering::SeqCst);
        }
    }

    fn exit_hook(&self, sid: &str, unmark_agent: bool) -> ExitHook {
        let notice = self.on_exit.clone();
        let spine = self.spine_dir.clone();
        let sid = sid.to_string();
        Box::new(move || {
            if unmark_agent {
                agentmark::unmark(&spine, &sid);
                crate::session_status::retire(&spine, &sid);
            }
            let listener = {
                let mut n = lock(&notice);
                if n.listener.is_none() {
                    n.queued.push(sid.clone());
                }
                n.listener.clone()
            };
            if let Some(l) = listener {
                l(&sid);
            }
        })
    }

    /// 照這次的後端開一個會話。`agent`＝agent 名（shell 是 None；herdr-core 後端存進中繼資料，接回時重標 agentmark）。
    fn spawn(
        &self,
        sid: &str,
        spec: &SpawnSpec,
        meta: TermMeta,
        agent: Option<&str>,
        program: Option<ProgramRecorder>,
    ) -> Result<Arc<TermSession>> {
        let hook = self.exit_hook(sid, meta.kind == TermKind::Agent);
        match &self.backend {
            Backend::Pty => TermSession::spawn_with_status(sid, spec, meta, Some(hook), program),
            Backend::Hc(hc) => hc.spawn(sid, spec, meta, agent, hook, program),
        }
    }

    /// herdr-core：列出這個 spine 還在 core 裡的會話、接回來（開 manager 時一次）。
    /// 在 App 關著的期間結束的：照「結束」處理一次（結束 hook，沒交接的卡片照常出現），然後從 core 拿掉。
    fn restore(&self, hc: Arc<crate::hc::HcBackend>) {
        let found = hc.existing();
        let keep: Vec<String> = found.iter().map(|e| e.sid().to_string()).collect();
        crate::session_status::clear_all_except(&self.spine_dir, &keep);
        let program_on = crate::config::load(&self.spine_dir)
            .map(|c| c.program_status())
            .unwrap_or(true);
        for e in found {
            let sid = e.sid().to_string();
            self.bump_ids(&sid);
            let agent = e.kind() == TermKind::Agent;
            let program = (agent && program_on).then(|| {
                ProgramRecorder::new(crate::session_status::path_for(&self.spine_dir, &sid))
            });
            let hook = self.exit_hook(&sid, agent);
            let name = e.meta.agent.clone();
            let Some(s) = hc.adopt(e, hook, program) else {
                continue;
            };
            if agent && s.alive() {
                if let (Some(cwd), Some(name)) = (s.cwd(), name) {
                    let pid = s.pid().map(i64::from).unwrap_or(0);
                    let _ = agentmark::mark(&self.spine_dir, &sid, cwd, &name, pid);
                }
            }
            lock(&self.sessions).push(s);
        }
    }

    /// 對應 `TermManager.create_shell`：沒給 cwd 就開在 spine。
    pub fn create_shell(
        &self,
        cwd: Option<&Path>,
        origin: Option<String>,
    ) -> Result<Arc<TermSession>> {
        let sid = self.next_sid();
        let title = if cwd.is_some() {
            "shell"
        } else {
            "shell · spine"
        };
        let spec = SpawnSpec::new(default_shell()).cwd(cwd.unwrap_or(&self.spine_dir));
        let meta = TermMeta {
            origin,
            ..TermMeta::shell(title)
        };
        let s = self.spawn(&sid, &spec, meta, None, None)?;
        lock(&self.sessions).push(s.clone());
        Ok(s)
    }

    /// 對應 `TermManager.create_agent`（指令組裝那段在呼叫端）：登入 shell 包裝→spawn→agentmark 標記。
    pub fn create_agent(&self, launch: AgentLaunch) -> Result<Arc<TermSession>> {
        let sid = self.next_sid();
        let steward = launch.origin.as_deref() == Some(STEWARD_ORIGIN);
        let scope = launch
            .repo
            .clone()
            .or_else(|| steward.then(|| "總管".to_string()))
            .or_else(|| launch.group.clone())
            .unwrap_or_else(|| {
                if launch.repos.is_some() {
                    "臨時".into()
                } else {
                    "全部".into()
                }
            });
        let agent = if launch.agent.is_empty() {
            "claude".to_string()
        } else {
            launch.agent.clone()
        };
        // 會話狀態（Rust 才有）：Claude Code 的 hook 照這個路徑寫「做完了等你／處理中」
        let mut env = launch.env.clone();
        env.push((
            crate::session_status::ENV.to_string(),
            crate::session_status::path_for(&self.spine_dir, &sid)
                .to_string_lossy()
                .into_owned(),
        ));
        // 屬於多個組的 repo：會話裡的 hook／MCP 照開會話時的牌組算主場（agentapi::HOME_GROUP_ENV）
        if let Some(g) = launch
            .home_group
            .as_ref()
            .or(launch.group.as_ref())
            .filter(|g| !g.is_empty())
        {
            env.push((crate::agentapi::HOME_GROUP_ENV.to_string(), g.clone()));
        }
        let spec = SpawnSpec {
            argv: login_shell_wrap(&launch.argv),
            cwd: Some(launch.cwd.clone()),
            env,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
        };
        let meta = TermMeta {
            kind: TermKind::Agent,
            title: agent_title(&agent, &scope, launch.task.as_deref(), 40),
            origin: launch.origin.clone(),
            scope: Some(scope),
            scope_repos: launch.repos.clone(),
            task: launch.task.clone(),
            home_group: launch.home_group.clone().or_else(|| launch.group.clone()),
        };
        // F-130：agent 自己在終端報的狀態（OSC 7501）；config `session.program_status: off` 就整個不讀、不回應探詢。
        // herdr-core 後端：core 自己解析、自己回探詢，這個寫入器改吃 core 的狀態事件（見 `hc`）
        let program = crate::config::load(&self.spine_dir)
            .map(|c| c.program_status())
            .unwrap_or(true)
            .then(|| ProgramRecorder::new(crate::session_status::path_for(&self.spine_dir, &sid)));
        let s = self.spawn(&sid, &spec, meta, Some(&agent), program)?;
        // agent 偵測（gitpane 課）：標記存活會話，P3 採集據此標「🤖 誰在跑」
        let pid = s.pid().map(i64::from).unwrap_or(0);
        agentmark::mark(&self.spine_dir, &sid, &launch.cwd, &agent, pid)?;
        lock(&self.sessions).push(s.clone());
        Ok(s)
    }

    /// 對應 `TermManager.get`。
    pub fn get(&self, sid: &str) -> Option<Arc<TermSession>> {
        lock(&self.sessions).iter().find(|s| s.sid == sid).cloned()
    }

    /// 對應 `TermManager.kill`：移出清冊並殺程序；未知 sid 回 `false`。
    pub fn kill(&self, sid: &str) -> bool {
        let s = {
            let mut v = lock(&self.sessions);
            let Some(i) = v.iter().position(|s| s.sid == sid) else {
                return false;
            };
            v.remove(i)
        };
        s.kill();
        agentmark::unmark(&self.spine_dir, sid); // reader 收尾可能晚到，這裡先清
        true
    }

    /// 對應 `TermManager.list`（建立順序）。
    pub fn list(&self) -> Vec<TermInfo> {
        lock(&self.sessions).iter().map(|s| s.info()).collect()
    }

    /// 對應 `TermManager.kill_all`：兩個後端的會話都殺。
    pub fn kill_all(&self) {
        let sids: Vec<String> = lock(&self.sessions).iter().map(|s| s.sid.clone()).collect();
        for sid in sids {
            self.kill(&sid);
        }
    }

    /// App 結束（F-128）：行程內 PTY 的會話殺掉（跟以前一樣）；跑在 herdr-core 的放手不殺——
    /// agentmark 與會話狀態檔都留著，下次開 App 接回來。
    pub fn shutdown(&self) {
        let all: Vec<Arc<TermSession>> = lock(&self.sessions).clone();
        for s in all {
            match s.backend() {
                BackendKind::Pty => {
                    self.kill(&s.sid);
                }
                BackendKind::HerdrCore => s.release(),
            }
        }
        if let Backend::Hc(hc) = &self.backend {
            hc.shutdown();
        }
    }
}

/// 照 config 決定後端；設成 herdr-core 但起不來＝退回 PTY，原因放進 `warning`。
fn choose_backend(spine_dir: &Path, opts: &TermOptions) -> (Backend, BackendStatus) {
    let cfg = crate::config::load(spine_dir).ok();
    let configured = cfg
        .as_ref()
        .map(|c| c.session_backend())
        .unwrap_or(Ok(BackendKind::Pty));
    match configured {
        Err(raw) => (
            Backend::Pty,
            BackendStatus {
                warning: Some(format!(
                    "config session.backend 不認得「{raw}」（可用 pty、herdr-core）：這次用行程內 PTY"
                )),
                ..BackendStatus::plain(BackendKind::Pty)
            },
        ),
        Ok(BackendKind::Pty) => (Backend::Pty, BackendStatus::plain(BackendKind::Pty)),
        Ok(BackendKind::HerdrCore) => {
            let hc_path = cfg.as_ref().and_then(|c| c.hc_path());
            let session = cfg
                .as_ref()
                .map(|c| c.hc_session())
                .unwrap_or_else(|| crate::config::DEFAULT_HC_SESSION.to_string());
            let base = BackendStatus {
                hc_session: Some(session.clone()),
                ..BackendStatus::plain(BackendKind::HerdrCore)
            };
            match crate::hc::HcBackend::start(
                spine_dir,
                hc_path.as_deref(),
                &session,
                opts.hc_env.clone(),
            ) {
                Ok(hc) => {
                    let status = BackendStatus {
                        hc_path: Some(hc.exe().to_string_lossy().into_owned()),
                        hc_version: Some(hc.version().to_string()).filter(|v| !v.is_empty()),
                        ..base
                    };
                    (Backend::Hc(hc), status)
                }
                Err(e) => (
                    Backend::Pty,
                    BackendStatus {
                        active: BackendKind::Pty,
                        warning: Some(format!(
                            "會話後端設成 herdr-core，但{}——這次改用行程內 PTY（關掉 App 時會話會一起結束）",
                            e.reason
                        )),
                        hc_path: e.exe.map(|p| p.to_string_lossy().into_owned()),
                        ..base
                    },
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_title_uses_task_head_and_truncates() {
        assert_eq!(agent_title("claude", "g1", None, 40), "claude · g1");
        assert_eq!(
            agent_title("claude", "g1", Some("  修 bug\n細節"), 40),
            "claude · 修 bug"
        );
        let long = "字".repeat(60);
        let t = agent_title("claude", "g1", Some(&long), 40);
        assert_eq!(t.chars().count(), 40);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn shlex_quote_matches_python() {
        assert_eq!(shlex_quote(""), "''");
        assert_eq!(shlex_quote("abc-1.2/x"), "abc-1.2/x");
        assert_eq!(shlex_quote("a b"), "'a b'");
        assert_eq!(shlex_quote("it's"), "'it'\"'\"'s'");
        assert_eq!(shlex_quote("中文"), "'中文'");
    }

    #[test]
    fn spawn_scrubs_parent_claude_session_env() {
        // 模擬「App 是從 Claude 會話裡 open 出來的」：本程序環境帶著上一場的身分
        let mut cmd = CommandBuilder::new("sh");
        for k in CLAUDE_SESSION_ENV {
            cmd.env(k, "1");
        }
        cmd.env("HOME", "/tmp/keep");
        scrub_claude_session_env(&mut cmd);
        for k in CLAUDE_SESSION_ENV {
            assert!(cmd.get_env(k).is_none(), "{k} 不該傳進 PTY 會話");
        }
        assert_eq!(
            cmd.get_env("HOME").map(|v| v.to_str()),
            Some(Some("/tmp/keep"))
        );
        // 呼叫端明確要求的環境仍照設（順序：先清、後加）
        cmd.env("CLAUDE_CODE_SESSION_ID", "mine");
        assert_eq!(
            cmd.get_env("CLAUDE_CODE_SESSION_ID").map(|v| v.to_str()),
            Some(Some("mine"))
        );
    }

    #[test]
    fn attach_id_roundtrip() {
        let id: AttachId = "a7".parse().unwrap();
        assert_eq!(id, AttachId(7));
        assert_eq!(id.to_string(), "a7");
        assert!("x".parse::<AttachId>().is_err());
    }
}
