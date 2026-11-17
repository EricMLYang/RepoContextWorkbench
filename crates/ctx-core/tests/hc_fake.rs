//! herdr-core 後端（F-128）對假的 core：測試裡起一個 UnixListener，照協定回（hello→分幀→JSON-RPC→串流）。
//! 不需要真的 hc：驗 CREDIT 流量控制、JSON-RPC 錯誤的 kind／hint、attach 的第一畫面＋即時輸出、狀態事件、
//! 斷線重連後重新 attach、關閉時 kill＋rm、找不到 hc 時退回 PTY。
#![cfg(unix)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use ctx_core::hc::client::{Client, StreamMsg};
use ctx_core::hc::frame::{self, Decoder, Kind};
use ctx_core::term::{AgentLaunch, BackendKind, TermManager, TermOptions};
use serde_json::{json, Value};

fn wait_until(secs: u64, mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    f()
}

/// 假 core 的一條連線。
struct Conn {
    w: Mutex<UnixStream>,
    credit: Mutex<HashMap<u32, i64>>,
    cv: Condvar,
}

impl Conn {
    fn send(&self, ch: u32, kind: Kind, payload: &[u8]) {
        let _ = self
            .w
            .lock()
            .unwrap()
            .write_all(&frame::encode(ch, kind, payload).unwrap());
    }

    fn reply(&self, id: u64, result: Value) {
        let m = json!({"jsonrpc": "2.0", "id": id, "result": result});
        self.send(0, Kind::Data, m.to_string().as_bytes());
    }

    fn error(&self, id: u64, msg: &str, kind: &str, hint: &str) {
        let m = json!({"jsonrpc": "2.0", "id": id,
                       "error": {"code": -32000, "message": msg, "data": {"kind": kind, "hint": hint}}});
        self.send(0, Kind::Data, m.to_string().as_bytes());
    }

    /// 照額度送：送之前等到額度夠（真的 core 也是這樣，額度用完就停）。
    fn send_with_credit(&self, ch: u32, data: &[u8]) -> bool {
        let mut c = self.credit.lock().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while *c.entry(ch).or_insert(frame::WINDOW as i64) < data.len() as i64 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            c = self.cv.wait_timeout(c, left).unwrap().0;
        }
        *c.get_mut(&ch).unwrap() -= data.len() as i64;
        drop(c);
        self.send(ch, Kind::Data, data);
        true
    }
}

type Handler = Arc<dyn Fn(&Arc<Conn>, u64, &str, &Value) + Send + Sync>;
type Frames = Arc<Mutex<Vec<(u32, Kind, Vec<u8>)>>>;
type StreamIn = Arc<dyn Fn(&Arc<Conn>, u32, Kind, &[u8]) + Send + Sync>;

/// 假 core：每條連線一個執行緒；記下收到的每個 frame。
struct FakeCore {
    path: PathBuf,
    conns: Arc<Mutex<Vec<Arc<Conn>>>>,
    frames: Frames,
    methods: Arc<Mutex<Vec<String>>>,
}

impl FakeCore {
    fn start(path: &Path, on_call: Handler, on_stream: StreamIn) -> FakeCore {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let _ = std::fs::remove_file(path);
        let l = UnixListener::bind(path).unwrap();
        let conns: Arc<Mutex<Vec<Arc<Conn>>>> = Default::default();
        let frames: Frames = Default::default();
        let methods: Arc<Mutex<Vec<String>>> = Default::default();
        let (c2, f2, m2) = (conns.clone(), frames.clone(), methods.clone());
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { return };
                let conn = Arc::new(Conn {
                    w: Mutex::new(s.try_clone().unwrap()),
                    credit: Mutex::new(HashMap::new()),
                    cv: Condvar::new(),
                });
                c2.lock().unwrap().push(conn.clone());
                let (f3, m3, on_call, on_stream) =
                    (f2.clone(), m2.clone(), on_call.clone(), on_stream.clone());
                std::thread::spawn(move || {
                    // plain 模式的 hello
                    let mut line = Vec::new();
                    let mut b = [0u8; 1];
                    while s.read(&mut b).unwrap_or(0) == 1 && b[0] != b'\n' {
                        line.push(b[0]);
                    }
                    let hello: Value = serde_json::from_slice(&line).unwrap_or(Value::Null);
                    if hello["params"]["framed"] != true {
                        return;
                    }
                    let r = json!({"jsonrpc": "2.0", "id": 0, "result":
                        {"framed": true, "version": "0.0-fake", "generation": 1, "features": [], "methods": []}});
                    conn.w
                        .lock()
                        .unwrap()
                        .write_all(format!("{r}\n").as_bytes())
                        .unwrap();
                    let mut dec = Decoder::new();
                    let mut buf = vec![0u8; 65536];
                    loop {
                        let n = match s.read(&mut buf) {
                            Ok(0) | Err(_) => return,
                            Ok(n) => n,
                        };
                        dec.push(&buf[..n]);
                        while let Ok(Some(f)) = dec.next_frame() {
                            f3.lock()
                                .unwrap()
                                .push((f.channel, f.kind, f.payload.clone()));
                            if f.kind == Kind::Credit {
                                let n = u32::from_le_bytes(f.payload[..4].try_into().unwrap());
                                *conn
                                    .credit
                                    .lock()
                                    .unwrap()
                                    .entry(f.channel)
                                    .or_insert(frame::WINDOW as i64) += i64::from(n);
                                conn.cv.notify_all();
                            } else if f.channel == 0 && f.kind == Kind::Data {
                                let m: Value = serde_json::from_slice(&f.payload).unwrap();
                                let method = m["method"].as_str().unwrap_or("").to_string();
                                m3.lock().unwrap().push(method.clone());
                                on_call(
                                    &conn,
                                    m["id"].as_u64().unwrap_or(0),
                                    &method,
                                    &m["params"],
                                );
                            } else {
                                on_stream(&conn, f.channel, f.kind, &f.payload);
                            }
                        }
                    }
                });
            }
        });
        FakeCore {
            path: path.to_path_buf(),
            conns,
            frames,
            methods,
        }
    }

    /// 模擬 core 重啟：把現在的連線全部切掉（listener 還在，client 會重連）。
    fn drop_connections(&self) {
        for c in self.conns.lock().unwrap().drain(..) {
            let _ = c.w.lock().unwrap().shutdown(std::net::Shutdown::Both);
        }
    }

    fn conn(&self) -> Option<Arc<Conn>> {
        self.conns.lock().unwrap().last().cloned()
    }

    fn called(&self, method: &str) -> usize {
        self.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == method)
            .count()
    }
}

#[test]
fn client_credit_flow_errors_and_close() {
    let t = tempfile::tempdir().unwrap();
    let sock = t.path().join("c.sock");
    const TOTAL: usize = 40 * 16 * 1024; // 640 KiB，超過一開始的 256 KiB 額度：不回 CREDIT 就收不完
    let on_call: Handler = Arc::new(|c, id, method, _p| match method {
        "boom" => c.error(id, "no terminal t9", "not_found", "see `hc ls --all`"),
        "terminal.attach" => {
            c.reply(id, json!({"channel": 5, "terminal": {"address": "t1"}}));
            let c = c.clone();
            std::thread::spawn(move || {
                let chunk = vec![b'x'; 16 * 1024];
                for _ in 0..TOTAL / chunk.len() {
                    if !c.send_with_credit(5, &chunk) {
                        return; // 額度沒回來：client 沒照規格回 CREDIT
                    }
                }
                c.send(
                    5,
                    Kind::Close,
                    br#"{"reason":"exited","exit_code":0,"signal":null}"#,
                );
            });
        }
        _ => c.reply(id, json!({})),
    });
    let got_in: Arc<Mutex<Vec<u8>>> = Default::default();
    let gi = got_in.clone();
    let on_stream: StreamIn = Arc::new(move |_c, ch, kind, p| {
        if ch == 5 && kind == Kind::Data {
            gi.lock().unwrap().extend_from_slice(p);
        }
    });
    let fake = FakeCore::start(&sock, on_call, on_stream);
    let lost = Arc::new(AtomicBool::new(false));
    let l2 = lost.clone();
    let c = Client::connect(&sock, move |_| l2.store(true, Ordering::SeqCst)).unwrap();
    assert_eq!(c.version(), "0.0-fake");

    // JSON-RPC 錯誤：kind 與 hint 都留著
    let e = c.call("boom", json!({})).unwrap_err();
    assert_eq!(
        (e.kind.as_str(), e.hint.as_str()),
        ("not_found", "see `hc ls --all`")
    );
    let text = ctx_core::Error::from(e).to_string();
    assert!(
        text.contains("no terminal t9") && text.contains("not_found") && text.contains("hc ls"),
        "{text}"
    );

    // attach：處理器在回覆到時掛上；收完 600 KiB（中間要回 CREDIT）、再收到 CLOSE 的原因
    let bytes = Arc::new(Mutex::new(0usize));
    let reason: Arc<Mutex<Option<Value>>> = Default::default();
    let (b2, r2) = (bytes.clone(), reason.clone());
    let r = c
        .call_stream(
            "terminal.attach",
            json!({"terminal": "t1", "cols": 80, "rows": 24}),
            Box::new(move |m| match m {
                StreamMsg::Data(d) => *b2.lock().unwrap() += d.len(),
                StreamMsg::Close(v) => *r2.lock().unwrap() = v,
                _ => {}
            }),
        )
        .unwrap();
    assert_eq!(r["channel"], 5);
    assert!(
        wait_until(10, || reason.lock().unwrap().is_some()),
        "只收到 {} bytes：CREDIT 沒照規格回",
        bytes.lock().unwrap()
    );
    assert_eq!(*bytes.lock().unwrap(), TOTAL);
    assert_eq!(reason.lock().unwrap().as_ref().unwrap()["reason"], "exited");
    let credited = || -> usize {
        fake.frames
            .lock()
            .unwrap()
            .iter()
            .filter(|(ch, k, _)| *ch == 5 && *k == Kind::Credit)
            .map(|(_, _, p)| u32::from_le_bytes(p[..4].try_into().unwrap()) as usize)
            .sum()
    };
    assert!(
        wait_until(5, || credited() == TOTAL),
        "每個 DATA 都要還額度：{}",
        credited()
    );

    // 輸入是 attach channel 上的 DATA
    assert!(c.send(5, Kind::Data, b"typed"));
    assert!(wait_until(5, || got_in.lock().unwrap().as_slice() == b"typed"));

    // 我們自己關：不算斷線
    c.shutdown();
    std::thread::sleep(Duration::from_millis(200));
    assert!(!lost.load(Ordering::SeqCst));
    drop(fake);
}

/// 假 core 的終端機清冊（夠 manager 用的那幾個方法）。
#[derive(Default)]
struct World {
    terms: Vec<Value>,
    /// 每個 attach channel 對到哪個終端機
    chans: HashMap<u32, String>,
    next_ch: u32,
    events_ch: Option<u32>,
    fail_run: bool,
}

fn manager_fake(sock: &Path) -> (FakeCore, Arc<Mutex<World>>) {
    let world: Arc<Mutex<World>> = Arc::new(Mutex::new(World {
        next_ch: 1,
        ..Default::default()
    }));
    let w = world.clone();
    let on_call: Handler = Arc::new(move |c, id, method, p| {
        let mut w = w.lock().unwrap();
        match method {
            "events.subscribe" => {
                let ch = w.next_ch;
                w.next_ch += 1;
                w.events_ch = Some(ch);
                let terms = w.terms.clone();
                c.reply(id, json!({"channel": ch, "seq": 0, "terminals": terms}));
            }
            "terminal.run" if w.fail_run => c.error(
                id,
                "cannot start that",
                "spawn_failed",
                "check the holder log",
            ),
            "terminal.run" => {
                let address = format!("t{}", w.terms.len() + 1);
                let rec = json!({"address": address, "state": "live", "labels": p["labels"],
                                 "pid": 4242, "cols": 120, "rows": 32, "cwd": p["cwd"], "status": null});
                w.terms.push(rec.clone());
                c.reply(id, rec);
            }
            "terminal.attach" => {
                let a = p["terminal"].as_str().unwrap_or("").to_string();
                let Some(rec) = w.terms.iter().find(|t| t["address"] == a.as_str()).cloned() else {
                    return c.error(id, "no such", "not_found", "");
                };
                let ch = w.next_ch;
                w.next_ch += 1;
                w.chans.insert(ch, a);
                c.reply(id, json!({"channel": ch, "terminal": rec, "automatic_input": false, "resize_notifications": false}));
                c.send(ch, Kind::Data, b"PAINT\r\n");
            }
            "terminal.list" => {
                let terms = w.terms.clone();
                c.reply(
                    id,
                    json!({"terminals": terms, "hidden_finished": 0, "machines": []}),
                );
            }
            "terminal.get" => {
                let a = p["terminal"].as_str().unwrap_or("");
                match w.terms.iter().find(|t| t["address"] == a) {
                    Some(t) => c.reply(id, t.clone()),
                    None => c.error(id, "no terminal", "not_found", ""),
                }
            }
            "terminal.kill" => {
                let a = p["terminal"].as_str().unwrap_or("").to_string();
                for t in w.terms.iter_mut().filter(|t| t["address"] == a.as_str()) {
                    t["state"] = json!("finished");
                    t["reason"] = json!("killed");
                }
                let rec = w.terms.iter().find(|t| t["address"] == a.as_str()).cloned();
                c.reply(id, rec.unwrap_or(Value::Null));
                for (ch, _) in w.chans.iter().filter(|(_, x)| **x == a) {
                    c.send(
                        *ch,
                        Kind::Close,
                        br#"{"reason":"killed","exit_code":null,"signal":15}"#,
                    );
                }
            }
            "terminal.rm" => {
                let a = p["terminal"].as_str().unwrap_or("").to_string();
                w.terms.retain(|t| t["address"] != a.as_str());
                c.reply(id, json!({"removed": a}));
            }
            _ => c.reply(id, json!({})),
        }
    });
    let w2 = world.clone();
    // 輸入回顯成 `echo:<輸入>`
    let on_stream: StreamIn = Arc::new(move |c, ch, kind, p| {
        if kind == Kind::Data && w2.lock().unwrap().chans.contains_key(&ch) {
            let mut out = b"echo:".to_vec();
            out.extend_from_slice(p);
            c.send(ch, Kind::Data, &out);
        }
    });
    (FakeCore::start(sock, on_call, on_stream), world)
}

fn true_exe() -> &'static str {
    if Path::new("/usr/bin/true").exists() {
        "/usr/bin/true"
    } else {
        "/bin/true"
    }
}

fn hc_spine(hc_path: &str) -> (tempfile::TempDir, PathBuf) {
    let (t, d) = ctx_core::testkit::spine_with_repos();
    let cfg = std::fs::read_to_string(d.join("config.yaml")).unwrap_or_default();
    std::fs::write(
        d.join("config.yaml"),
        format!("{cfg}\nsession:\n  backend: herdr-core\n  hc_path: {hc_path}\n"),
    )
    .unwrap();
    (t, d)
}

#[test]
fn manager_on_fake_core_survives_reconnect_and_kills_with_rm() {
    let rt = tempfile::tempdir().unwrap();
    let sock = rt.path().join("rt/herdr-core/ctx.sock");
    let (fake, world) = manager_fake(&sock);
    let (_t, d) = hc_spine(true_exe());
    let opts = TermOptions {
        hc_env: vec![
            (
                "XDG_RUNTIME_DIR".into(),
                rt.path().join("rt").to_string_lossy().into_owned(),
            ),
            (
                "XDG_STATE_HOME".into(),
                rt.path().join("st").to_string_lossy().into_owned(),
            ),
            (
                "XDG_CONFIG_HOME".into(),
                rt.path().join("cf").to_string_lossy().into_owned(),
            ),
        ],
    };
    let m = TermManager::with_options(&d, opts);
    let st = m.backend_status().clone();
    assert_eq!(st.active, BackendKind::HerdrCore, "{st:?}");
    assert_eq!(st.hc_version.as_deref(), Some("0.0-fake"));
    let exits = Arc::new(AtomicU32::new(0));
    let e2 = exits.clone();
    m.set_on_exit(move |_| {
        e2.fetch_add(1, Ordering::SeqCst);
    });

    // JSON-RPC 錯誤傳到開會話的呼叫端（kind、hint 都在）
    world.lock().unwrap().fail_run = true;
    let err = m.create_shell(None, None).err().unwrap().to_string();
    assert!(
        err.contains("cannot start that")
            && err.contains("spawn_failed")
            && err.contains("holder log"),
        "{err}"
    );
    world.lock().unwrap().fail_run = false;

    let s = m
        .create_agent(AgentLaunch {
            agent: "fake".into(),
            argv: vec!["/bin/echo".into(), "hi".into()],
            cwd: d.clone(),
            ..Default::default()
        })
        .unwrap();
    let sid = s.sid().to_string();
    assert_eq!(s.backend(), BackendKind::HerdrCore);
    assert_eq!(s.pid(), Some(4242));
    // 中繼資料存在 .state/hc_sessions/<sid>.json，標籤帶 sid／spine／kind
    let meta = ctx_core::hc::load_meta(&d, &sid).unwrap();
    assert_eq!(meta.address, "t1");
    let labels = world.lock().unwrap().terms[0]["labels"].clone();
    assert_eq!(labels["ctx.sid"], sid.as_str());
    assert_eq!(labels["ctx.kind"], "agent");
    assert_eq!(labels["ctx.spine"], ctx_core::hc::spine_tag(&d).as_str());

    // 第一畫面進 replay；輸入走 attach channel，回顯回來
    assert!(wait_until(5, || String::from_utf8_lossy(&s.replay()).contains("PAINT")));
    s.write(b"abc");
    assert!(wait_until(5, || String::from_utf8_lossy(&s.replay())
        .contains("echo:abc")));

    // 狀態事件 → 程式回報檔
    let ev = json!({"jsonrpc": "2.0", "method": "event", "params": {"seq": 1, "type": "status", "terminal": "t1",
                    "data": {"status": {"state": "working", "app": "fake"}, "records": {}, "reported": true}}});
    let ech = world.lock().unwrap().events_ch.unwrap();
    fake.conn()
        .unwrap()
        .send(ech, Kind::Data, format!("{ev}\n").as_bytes());
    assert!(wait_until(5, || ctx_core::session_status::read_program(
        &d, &sid
    )
    .is_some_and(|v| v["program"]["state"] == "working")));

    // core 重啟（連線全斷）：重連、重新訂事件、重新 attach；期間會話一直算活著，之後還能輸入
    let attaches = fake.called("terminal.attach");
    fake.drop_connections();
    assert!(
        wait_until(10, || fake.called("terminal.attach") > attaches),
        "沒有重新 attach"
    );
    assert!(s.alive());
    assert!(wait_until(5, || {
        s.write(b"def");
        String::from_utf8_lossy(&s.replay()).contains("echo:def")
    }));
    assert!(fake.called("events.subscribe") >= 2);

    // 關閉：terminal.kill → core 關串流 → 問過確定結束 → 收尾（結束 hook 一次）＋ terminal.rm＋刪中繼資料
    assert!(m.kill(&sid));
    assert!(
        wait_until(5, || fake.called("terminal.rm") == 1),
        "沒有 terminal.rm"
    );
    assert!(wait_until(5, || exits.load(Ordering::SeqCst) == 1));
    assert!(!s.alive());
    assert!(ctx_core::hc::load_meta(&d, &sid).is_none());
    m.shutdown();
    let _ = std::fs::remove_file(&fake.path);
}

#[test]
fn missing_hc_falls_back_to_pty_with_a_visible_reason() {
    let (_t, d) = hc_spine("/nonexistent/herdr/hc");
    let m = TermManager::new(&d);
    let st = m.backend_status();
    assert_eq!(st.configured, BackendKind::HerdrCore);
    assert_eq!(st.active, BackendKind::Pty);
    let w = st.warning.clone().unwrap();
    assert!(
        w.contains("session.hc_path") && w.contains("/nonexistent/herdr/hc"),
        "{w}"
    );
    // 照樣能開行程內的會話
    let s = m.create_shell(None, None).unwrap();
    assert_eq!(s.backend(), BackendKind::Pty);
    m.kill_all();
    // ctx health 看得到：設定的問題、App 上次退回 PTY 的原因
    let h = ctx_core::health::session_backend(&d);
    assert_eq!(h["configured"], "herdr-core");
    assert!(
        h["problem"].as_str().unwrap().contains("session.hc_path"),
        "{h}"
    );
    assert_eq!(h["app"]["active"], "pty");
    assert!(h["app"]["warning"].as_str().is_some());
}

#[test]
fn unknown_backend_value_warns_and_uses_pty() {
    let (_t, d) = ctx_core::testkit::spine();
    let cfg = std::fs::read_to_string(d.join("config.yaml")).unwrap_or_default();
    std::fs::write(
        d.join("config.yaml"),
        format!("{cfg}\nsession:\n  backend: tmux\n"),
    )
    .unwrap();
    let m = TermManager::new(&d);
    assert_eq!(m.backend_status().active, BackendKind::Pty);
    assert!(m
        .backend_status()
        .warning
        .as_deref()
        .unwrap()
        .contains("tmux"));
}
