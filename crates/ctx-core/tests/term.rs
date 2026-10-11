//! 內嵌 terminal：L1 PTY 會話（echo／replay／kill）。對應 tests/test_term.py。
//!
//! 未搬：`test_ws_accept_key_rfc_vector`、`test_ws_frame_roundtrip`、`test_ws_terminal_roundtrip`——
//! WebSocket codec／HTTP server 是 Python 版 webui 的傳輸層；Rust 桌面版走 Tauri IPC Channel
//! （`desktop/src-tauri/src/terminal.rs`），沒有 WS。L2 往返改由下面 `attach_replay_then_live_in_order`
//! 驗「replay 先、即時後、同一條訂閱順序不亂」這個 WS handler 原本保證的性質。
#![cfg(unix)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ctx_core::agentmark;
use ctx_core::term::{AgentLaunch, SpawnSpec, TermEvent, TermManager, TermMeta, TermSession};
use ctx_core::testkit::{spine, spine_with_repos};

type Sink = Arc<Mutex<Vec<u8>>>;

fn collector(sink: &Sink) -> Box<dyn FnMut(TermEvent<'_>) -> bool + Send> {
    let sink = sink.clone();
    Box::new(move |ev| {
        if let TermEvent::Data(b) = ev {
            sink.lock().unwrap().extend_from_slice(b);
        }
        true
    })
}

fn count(hay: &[u8], needle: &[u8]) -> usize {
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

fn wait_until(secs: u64, mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    f()
}

#[test]
fn test_session_echo_replay_kill() {
    let tmp = tempfile::tempdir().unwrap();
    let got: Sink = Default::default();
    let s = TermSession::spawn(
        "t1",
        &SpawnSpec::new(vec!["/bin/cat".into()]).cwd(tmp.path()),
        TermMeta::shell(""),
        None,
    )
    .unwrap();
    assert_eq!(s.title(), "cat"); // 沒給標題＝argv[0] 的檔名
    s.attach(collector(&got));
    s.write(b"PTY_OK_42\n");
    wait_until(10, || count(&got.lock().unwrap(), b"PTY_OK_42") >= 2);
    assert!(count(&got.lock().unwrap(), b"PTY_OK_42") >= 2); // pty echo ＋ cat 回吐
                                                             // 晚到的訂閱者拿 replay（會話生命週期獨立於連線）
    let late: Sink = Default::default();
    s.attach(collector(&late));
    assert!(count(&late.lock().unwrap(), b"PTY_OK_42") >= 1);
    s.resize(100, 30); // 不炸即可
    s.kill();
    assert!(wait_until(10, || !s.alive()));
    assert!(!s.alive());
}

#[test]
fn test_manager_agent_spawns_via_login_shell() {
    // Python 版經 session.build 打包＋假 agent（/bin/echo）印出帶料 prompt。
    // session 模組另一波搬；這裡由呼叫端給已組好的 argv，驗：登入 shell 包裝→PTY spawn→
    // 輸出真的出現在終端、標題「agent · 全部」、agentmark 標記與結束後清除。
    std::env::set_var("SHELL", "/bin/sh");
    let (_tmp, d) = spine_with_repos();
    let m = TermManager::new(&d);
    let s = m
        .create_agent(AgentLaunch {
            agent: "fake".into(),
            argv: vec!["/bin/echo".into(), "# 文件地圖 it's ok".into()],
            cwd: d.clone(),
            ..Default::default()
        })
        .unwrap(); // 不給 group/repos＝（全部）
    assert_eq!(s.title(), "fake · 全部");
    assert_eq!(s.info().home_group, None);
    let marker = "文件地圖 it's ok".as_bytes();
    assert!(wait_until(15, || count(&s.replay(), marker) >= 1));
    // 程序結束 → reader 收尾清 marker
    assert!(wait_until(10, || !s.alive()));
    assert!(wait_until(5, || agentmark::live(&d).is_empty()));
    assert!(String::from_utf8_lossy(&s.replay()).contains("進程已結束"));
    m.kill_all();
}

#[test]
fn test_manager_shell_and_kill() {
    let (_tmp, d) = spine();
    let m = TermManager::new(&d);
    let s = m.create_shell(None, None).unwrap();
    let l = m.list();
    assert!(l[0].sid == s.sid() && l[0].alive);
    assert_eq!(l[0].title, "shell · spine");
    assert!(m.kill(s.sid()) && m.list().is_empty());
    assert!(!m.kill("t999")); // 未知會話 → false
}

#[test]
fn attach_replay_then_live_in_order() {
    // 取代 test_ws_terminal_roundtrip：輸入進 PTY、輸出經訂閱回來；replay 先到、之後是即時輸出；
    // 多個 attach 各自收得到；detach 只拆那一條。
    let tmp = tempfile::tempdir().unwrap();
    let s = TermSession::spawn(
        "t1",
        &SpawnSpec::new(vec!["/bin/sh".into()]).cwd(tmp.path()),
        TermMeta::shell("sh"),
        None,
    )
    .unwrap();
    s.write(b"echo FIRST_$((40+2))\n");
    assert!(wait_until(10, || count(&s.replay(), b"FIRST_42") >= 1));
    let a: Sink = Default::default();
    let b: Sink = Default::default();
    let id_a = s.attach(collector(&a));
    let _id_b = s.attach(collector(&b));
    assert!(count(&a.lock().unwrap(), b"FIRST_42") >= 1); // replay 第一筆就有
    assert!(s.detach(id_a));
    assert!(!s.detach(id_a));
    s.write(b"echo SECOND_$((40+3))\n");
    assert!(wait_until(10, || count(&b.lock().unwrap(), b"SECOND_43") >= 1));
    let bb = b.lock().unwrap().clone();
    let first = bb.windows(8).position(|w| w == b"FIRST_42").unwrap();
    let second = bb.windows(9).position(|w| w == b"SECOND_43").unwrap();
    assert!(first < second);
    assert_eq!(count(&a.lock().unwrap(), b"SECOND_43"), 0); // 拆掉的那條收不到
    s.kill();
    assert!(wait_until(10, || !s.alive()));
}

#[test]
fn exit_event_reaches_subscribers_and_buffer_is_capped() {
    let tmp = tempfile::tempdir().unwrap();
    let exited = Arc::new(Mutex::new(false));
    let e2 = exited.clone();
    let s = TermSession::spawn(
        "t1",
        // 輸出 ~300KB 後結束：ring buffer 只留最後 ~200KB（至少一塊）
        &SpawnSpec::new(vec![
            "/bin/sh".into(),
            "-c".into(),
            "i=0; while [ $i -lt 3000 ]; do printf '%0100d\\n' $i; i=$((i+1)); done".into(),
        ])
        .cwd(tmp.path()),
        TermMeta::shell("sh"),
        None,
    )
    .unwrap();
    s.attach(Box::new(move |ev| {
        if ev == TermEvent::Exited {
            *e2.lock().unwrap() = true;
        }
        true
    }));
    assert!(wait_until(15, || *exited.lock().unwrap()));
    let r = s.replay();
    assert!(r.len() <= ctx_core::term::BUFFER_CAP + 70_000);
    assert!(String::from_utf8_lossy(&r).contains("進程已結束"));
}

/// Rust 才有（2026-10-12）：所屬牌組只給畫面歸類——對單一 repo 開時 scope 是 repo、home_group 是組；
/// 對整組開沒給 home_group 就用 group。沒有就不輸出這個鍵（跟 Python 版欄位一樣）。
#[test]
fn test_agent_home_group_for_display() {
    std::env::set_var("SHELL", "/bin/sh");
    let (_tmp, d) = spine_with_repos();
    let m = TermManager::new(&d);
    let launch = |repo: Option<&str>, group: Option<&str>, home: Option<&str>| AgentLaunch {
        agent: "fake".into(),
        argv: vec!["/bin/echo".into(), "x".into()],
        cwd: d.clone(),
        repo: repo.map(Into::into),
        repos: repo.map(|r| vec![r.to_string()]),
        group: group.map(Into::into),
        home_group: home.map(Into::into),
        ..Default::default()
    };
    let one = m
        .create_agent(launch(Some("repo-a"), None, Some("g1")))
        .unwrap()
        .info();
    assert_eq!(
        (one.scope.as_deref(), one.home_group.as_deref()),
        (Some("repo-a"), Some("g1"))
    );
    let whole = m
        .create_agent(launch(None, Some("g1"), None))
        .unwrap()
        .info();
    assert_eq!(whole.home_group.as_deref(), Some("g1"));
    let shell = m.create_shell(None, None).unwrap().info();
    assert!(serde_json::to_value(&shell)
        .unwrap()
        .get("home_group")
        .is_none());
    m.kill_all();
}
