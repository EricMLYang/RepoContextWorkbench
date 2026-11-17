//! herdr-core 後端（F-128）對**真的** hc：環境變數 `CTX_TEST_HC`＝hc 執行檔路徑才跑，沒設就略過（CI 沒有 hc）。
//!
//! 一律隔離：每個測試自己的 `HC_SESSION`（短名）、`XDG_STATE_HOME`／`XDG_CONFIG_HOME` 指到 tempdir、
//! `XDG_RUNTIME_DIR` 指到短路徑（`/tmp/claude-<uid>/hcr-…`，socket 路徑有 103 bytes 上限）；
//! 結束時（含失敗時）`hc server stop` 並刪掉目錄。不會碰使用者預設的 herdr-core。
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ctx_core::hc::locate::HcSetup;
use ctx_core::term::{AgentLaunch, BackendKind, TermManager, TermOptions};

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

/// 一個隔離的 herdr-core：Drop 時 `hc server stop`＋刪目錄。
struct Isolated {
    hc: PathBuf,
    session: String,
    run: PathBuf,
    home: tempfile::TempDir,
}

impl Isolated {
    fn new(tag: &str) -> Option<Isolated> {
        let hc = std::env::var_os("CTX_TEST_HC").map(PathBuf::from)?;
        assert!(hc.is_file(), "CTX_TEST_HC 指到的不是檔案：{}", hc.display());
        // SAFETY: getuid 不會失敗
        let uid = unsafe { libc::getuid() };
        let base = PathBuf::from(format!("/tmp/claude-{uid}"));
        std::fs::create_dir_all(&base).unwrap();
        let run = tempfile::Builder::new()
            .prefix("hcr-")
            .tempdir_in(&base)
            .unwrap()
            .keep();
        let n = std::process::id() % 100_000;
        Some(Isolated {
            hc,
            session: format!("ct{tag}{n}"),
            run,
            home: tempfile::tempdir().unwrap(),
        })
    }

    fn env(&self) -> Vec<(String, String)> {
        let p = |x: &Path| x.to_string_lossy().into_owned();
        vec![
            ("XDG_RUNTIME_DIR".into(), p(&self.run)),
            ("XDG_STATE_HOME".into(), p(&self.home.path().join("state"))),
            (
                "XDG_CONFIG_HOME".into(),
                p(&self.home.path().join("config")),
            ),
        ]
    }

    fn opts(&self) -> TermOptions {
        TermOptions { hc_env: self.env() }
    }

    fn setup(&self) -> HcSetup {
        HcSetup {
            exe: self.hc.clone(),
            session: self.session.clone(),
            env: self.env(),
        }
    }

    /// spine：config 指到這個 hc、這個 session。
    fn spine(&self) -> (tempfile::TempDir, PathBuf) {
        std::env::set_var("SHELL", "/bin/sh"); // 登入 shell 包裝用 sh（不跑使用者的 zshrc）
        let (t, d) = ctx_core::testkit::spine_with_repos();
        let cfg = std::fs::read_to_string(d.join("config.yaml")).unwrap_or_default();
        std::fs::write(
            d.join("config.yaml"),
            format!(
                "{cfg}\nsession:\n  backend: herdr-core\n  hc_path: {}\n  hc_session: {}\n",
                self.hc.display(),
                self.session
            ),
        )
        .unwrap();
        (t, d)
    }

    /// `hc <args>`（隔離環境），回 stdout。
    fn hc(&self, args: &[&str]) -> String {
        let out = self.setup().command().args(args).output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

impl Drop for Isolated {
    fn drop(&mut self) {
        let _ = self
            .setup()
            .command()
            .args(["server", "stop"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        let _ = std::fs::remove_dir_all(&self.run);
    }
}

macro_rules! isolated_or_skip {
    ($tag:expr) => {
        match Isolated::new($tag) {
            Some(i) => i,
            None => {
                eprintln!("略過：沒設 CTX_TEST_HC");
                return;
            }
        }
    };
}

fn agent(d: &Path, script: &str) -> AgentLaunch {
    AgentLaunch {
        agent: "fake".into(),
        argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
        cwd: d.to_path_buf(),
        ..Default::default()
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn real_output_and_echo() {
    let iso = isolated_or_skip!("a");
    let (_t, d) = iso.spine();
    let m = TermManager::with_options(&d, iso.opts());
    assert_eq!(
        m.backend_status().active,
        BackendKind::HerdrCore,
        "{:?}",
        m.backend_status()
    );
    eprintln!("backend: {:?}", m.backend_status());
    let s = m
        .create_agent(agent(&d, "echo READY_$((20+1)); exec /bin/sh"))
        .unwrap();
    let got: Arc<Mutex<Vec<u8>>> = Default::default();
    let g = got.clone();
    s.attach(Box::new(move |ev| {
        if let ctx_core::term::TermEvent::Data(b) = ev {
            g.lock().unwrap().extend_from_slice(b);
        }
        true
    }));
    assert!(
        wait_until(15, || text(&got.lock().unwrap()).contains("READY_21")),
        "沒收到輸出：{:?}",
        text(&got.lock().unwrap())
    );
    s.write(b"echo hi_$((40+2))\n");
    assert!(
        wait_until(10, || text(&got.lock().unwrap()).contains("hi_42")),
        "沒收到回顯：{:?}",
        text(&got.lock().unwrap())
    );
    eprintln!("output: {:?}", text(&got.lock().unwrap()));
    assert!(m.kill(s.sid()));
    assert!(wait_until(10, || !iso
        .hc(&["ls", "--all", "--json"])
        .contains("ctx.sid")));
    m.shutdown();
}

#[test]
fn real_session_survives_manager_drop() {
    let iso = isolated_or_skip!("b");
    let (_t, d) = iso.spine();
    let m = TermManager::with_options(&d, iso.opts());
    assert_eq!(m.backend_status().active, BackendKind::HerdrCore);
    let s = m.create_agent(agent(&d, "exec /bin/sh")).unwrap();
    let sid = s.sid().to_string();
    s.write(b"echo before_$((1+1))\n");
    assert!(wait_until(15, || text(&s.replay()).contains("before_2")));
    // 模擬 App 結束：放手、整個 manager 丟掉
    m.shutdown();
    drop(s);
    drop(m);
    std::thread::sleep(Duration::from_millis(300));

    let m2 = TermManager::with_options(&d, iso.opts());
    let l = m2.list();
    eprintln!("after restart: {l:?}");
    let info = l.iter().find(|i| i.sid == sid).expect("會話沒被接回");
    assert!(info.alive);
    let s2 = m2.get(&sid).unwrap();
    assert_eq!(s2.backend(), BackendKind::HerdrCore);
    assert!(
        wait_until(10, || text(&s2.replay()).contains("before_2")),
        "replay 沒有先前的輸出：{:?}",
        text(&s2.replay())
    );
    s2.write(b"echo after_$((2+3))\n");
    assert!(
        wait_until(10, || text(&s2.replay()).contains("after_5")),
        "接回後輸入沒有作用：{:?}",
        text(&s2.replay())
    );
    eprintln!("replay after restart: {:?}", text(&s2.replay()));
    // 新開的會話 sid 接著編，不撞
    let s3 = m2.create_agent(agent(&d, "exec /bin/sh")).unwrap();
    assert_ne!(s3.sid(), sid);
    m2.kill_all();
    m2.shutdown();
}

#[test]
fn real_session_ended_while_away_runs_exit_hook_once() {
    let iso = isolated_or_skip!("c");
    let (_t, d) = iso.spine();
    let m = TermManager::with_options(&d, iso.opts());
    let s = m
        .create_agent(agent(&d, "echo bye_soon; sleep 1; exit 3"))
        .unwrap();
    let sid = s.sid().to_string();
    assert!(wait_until(10, || text(&s.replay()).contains("bye_soon")));
    m.shutdown();
    drop(s);
    drop(m);
    // App 關著的期間程式結束
    std::thread::sleep(Duration::from_secs(3));

    let m2 = TermManager::with_options(&d, iso.opts());
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let c2 = calls.clone();
    m2.set_on_exit(move |sid| c2.lock().unwrap().push(sid.to_string()));
    assert_eq!(
        *calls.lock().unwrap(),
        vec![sid.clone()],
        "結束 hook 要剛好一次"
    );
    let s2 = m2.get(&sid).expect("結束的會話也要列出來（給接續）");
    assert!(!s2.alive());
    let r = text(&s2.replay());
    eprintln!("ended replay: {r:?}");
    assert!(
        r.contains("App 關著的時候結束了") && r.contains("結束碼 3"),
        "{r}"
    );
    assert!(
        ctx_core::hc::load_meta(&d, &sid).is_none(),
        "中繼資料要清掉"
    );
    assert!(ctx_core::agentmark::live(&d).iter().all(|a| a.sid != sid));
    // core 裡也拿掉了
    assert!(wait_until(5, || !iso
        .hc(&["ls", "--all", "--json"])
        .contains(&sid)));
    // 再開一次：不會再通知
    m2.shutdown();
    drop(s2);
    drop(m2);
    let m3 = TermManager::with_options(&d, iso.opts());
    let n = Arc::new(AtomicU32::new(0));
    let n2 = n.clone();
    m3.set_on_exit(move |_| {
        n2.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(n.load(Ordering::SeqCst), 0);
    assert!(m3.get(&sid).is_none());
    m3.shutdown();
}

#[test]
fn real_program_status_is_recorded() {
    let iso = isolated_or_skip!("d");
    let (_t, d) = iso.spine();
    let m = TermManager::with_options(&d, iso.opts());
    let s = m
        .create_agent(agent(
            &d,
            r"printf '\033]7501;state=working:app=fake\033\\'; echo status_sent; sleep 30",
        ))
        .unwrap();
    let sid = s.sid().to_string();
    assert!(
        wait_until(10, || ctx_core::session_status::read_program(&d, &sid)
            .is_some_and(|v| v["program"]["state"] == "working")),
        "程式回報沒記到：{:?}／輸出 {:?}",
        ctx_core::session_status::read_program(&d, &sid),
        text(&s.replay())
    );
    let v = ctx_core::session_status::read_program(&d, &sid).unwrap();
    eprintln!("program.json: {v}");
    assert_eq!(v["program"]["app"], "fake");
    m.kill_all();
    m.shutdown();
}

#[test]
fn real_reconnects_after_core_is_killed() {
    let iso = isolated_or_skip!("e");
    let (_t, d) = iso.spine();
    let m = TermManager::with_options(&d, iso.opts());
    let s = m.create_agent(agent(&d, "exec /bin/sh")).unwrap();
    s.write(b"echo one_$((0+1))\n");
    assert!(wait_until(15, || text(&s.replay()).contains("one_1")));
    let st: serde_json::Value =
        serde_json::from_str(&iso.hc(&["server", "status", "--json"])).unwrap();
    let pid = st["pid"].as_i64().unwrap() as libc::pid_t;
    // 強制殺掉 core（holder 會把它拉起來；程式不受影響）
    // SAFETY: 對隔離環境裡我們自己叫起來的 core 送訊號
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
    let t0 = Instant::now();
    assert!(
        wait_until(30, || {
            s.write(b"echo two_$((1+1))\n");
            std::thread::sleep(Duration::from_millis(300));
            text(&s.replay()).contains("two_2")
        }),
        "core 重啟後沒接回：{:?}",
        text(&s.replay())
    );
    eprintln!("reconnected and typed after {:?}", t0.elapsed());
    assert!(s.alive());
    m.kill_all();
    m.shutdown();
}

/// 量輸入延遲（只印、不判斷）：打一個字到 `cat`，等回顯出現在訂閱者那邊；行程內 PTY 與 herdr-core 各 50 次。
#[test]
fn real_input_echo_latency() {
    let iso = isolated_or_skip!("f");
    let (_t, d) = iso.spine();
    let measure = |m: &TermManager| -> Vec<Duration> {
        let s = m
            .create_agent(agent(&d, "stty raw -echo; exec cat"))
            .unwrap();
        let got: Arc<Mutex<Vec<u8>>> = Default::default();
        let g = got.clone();
        let cv = Arc::new((Mutex::new(()), std::sync::Condvar::new()));
        let cv2 = cv.clone();
        s.attach(Box::new(move |ev| {
            if let ctx_core::term::TermEvent::Data(b) = ev {
                g.lock().unwrap().extend_from_slice(b);
                cv2.1.notify_all();
            }
            true
        }));
        std::thread::sleep(Duration::from_millis(800));
        let mut out = Vec::new();
        for i in 0..50u8 {
            let c = b'a' + (i % 26);
            let before = got.lock().unwrap().len();
            let t0 = Instant::now();
            s.write(&[c]);
            let mut g = cv.0.lock().unwrap();
            while t0.elapsed() < Duration::from_secs(2) {
                if got.lock().unwrap()[before..].contains(&c) {
                    break;
                }
                g = cv.1.wait_timeout(g, Duration::from_millis(5)).unwrap().0;
            }
            out.push(t0.elapsed());
        }
        m.kill_all();
        out.sort();
        out
    };
    let hc = TermManager::with_options(&d, iso.opts());
    assert_eq!(hc.backend_status().active, BackendKind::HerdrCore);
    let h = measure(&hc);
    hc.shutdown();
    // 同一個 spine 改回 pty
    let cfg = std::fs::read_to_string(d.join("config.yaml")).unwrap();
    std::fs::write(
        d.join("config.yaml"),
        cfg.replace("backend: herdr-core", "backend: pty"),
    )
    .unwrap();
    let pty = TermManager::new(&d);
    assert_eq!(pty.backend_status().active, BackendKind::Pty);
    let p = measure(&pty);
    let q = |v: &[Duration], f: f64| v[((v.len() as f64 - 1.0) * f) as usize];
    eprintln!(
        "輸入→回顯 中位數／p90／最大：pty {:?}／{:?}／{:?}；herdr-core {:?}／{:?}／{:?}",
        q(&p, 0.5),
        q(&p, 0.9),
        p[p.len() - 1],
        q(&h, 0.5),
        q(&h, 0.9),
        h[h.len() - 1]
    );
}
