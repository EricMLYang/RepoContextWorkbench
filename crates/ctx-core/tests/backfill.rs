//! 回填（2026-09-26）：從過去的 Claude Code 會話把判斷／待辦／交接撈回 spine。對應 tests/test_backfill.py。
//!
//! 釘住：① cwd 對到 repo（含 Drive 舊路徑依資料夾名）、還在寫的／對不到的略過；
//! ② 壓縮只留原話、agent 文字、工具一行（tool_result／系統標記丟掉）、輪次編號；
//! ③ 可續跑：做過的不重做、單場失敗不擋整批；④ 候選不自動進 spine，accept 才寫、source=backfill、
//! 不重收、交接是最新一筆；⑤ 反芻吃得到收下的判斷。
//!
//! 模型呼叫一律注入 FakeCall（不打真 claude）。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use ctx_core::agentapi::{self as api, Target};
use ctx_core::backfill::{self as bf, RunOpts};
use ctx_core::registry::{self, AddRepo};
use ctx_core::testkit::{make_git_repo, spine};
use ctx_core::{spine as spine_mod, Error, Result};
use serde_json::{json, Value};

fn write_session(
    root: &Path,
    proj: &str,
    sid: &str,
    cwd: &Path,
    turns: &[(&str, &str)],
    age_hours: u64,
) -> PathBuf {
    let d = root.join(proj);
    std::fs::create_dir_all(&d).unwrap();
    let cwd = cwd.to_string_lossy();
    let ts = "2026-09-10T10:00:00Z";
    let mut rows = vec![json!({"type": "ai-title", "aiTitle": "索引重建"})];
    for (u, a) in turns {
        rows.push(json!({"type": "user", "cwd": cwd, "timestamp": ts,
                         "message": {"role": "user", "content": u}}));
        rows.push(json!({"type": "assistant", "cwd": cwd, "timestamp": ts,
                         "message": {"role": "assistant", "content": [
                             {"type": "thinking", "thinking": "祕密思考"},
                             {"type": "text", "text": a},
                             {"type": "tool_use", "name": "Edit", "input": {"file_path": "/x/a.py"}}]}}));
        rows.push(json!({"type": "user", "cwd": cwd, "timestamp": ts, "message": {
            "role": "user", "content": [{"type": "tool_result", "content": "巨量輸出".repeat(50)}]}}));
    }
    let p = d.join(format!("{sid}.jsonl"));
    let text: Vec<String> = rows.iter().map(|r| r.to_string()).collect();
    std::fs::write(&p, text.join("\n")).unwrap();
    let t = SystemTime::now() - Duration::from_secs(age_hours * 3600);
    std::fs::File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_accessed(t).set_modified(t))
        .unwrap();
    p
}

fn extracted() -> Value {
    json!({"summary": "改索引", "decisions": [{"text": "索引改增量重建", "basis": "全量太慢", "turn": "t1"}],
           "todos": [{"text": "補壓測", "turn": "t2"}], "done": ["加水位線"], "next": "跑壓測",
           "goal_hints": ["讓搜尋更快"]})
}

fn consolidated() -> Value {
    json!({"decisions": [{"text": "索引改增量重建", "basis": "全量太慢", "sid": "s1aaaaaa", "date": "2026-09-10"}],
           "open_todos": [{"text": "補壓測", "sid": "s1aaaaaa", "date": "2026-09-10", "uncertain": false}],
           "handoff": {"done": "加水位線", "next": "跑壓測", "sid": "s1aaaaaa", "date": "2026-09-10"},
           "goal_hints": [{"text": "讓搜尋更快", "sid": "s1aaaaaa"}]})
}

/// 假模型：記下 prompt；含 fail 標記就丟 AgentUnsure（External）。
struct FakeCall {
    prompts: Mutex<Vec<String>>,
    fail: Vec<&'static str>,
}

impl FakeCall {
    fn new(fail: &[&'static str]) -> Self {
        FakeCall {
            prompts: Mutex::new(Vec::new()),
            fail: fail.to_vec(),
        }
    }
    fn call(&self, prompt: &str) -> Result<String> {
        self.prompts.lock().unwrap().push(prompt.to_string());
        if self.fail.iter().any(|s| prompt.contains(s)) {
            return Err(Error::External("boom".into()));
        }
        if prompt.contains("逐場整理") {
            return Ok(format!("好的：{}", consolidated()));
        }
        Ok(extracted().to_string())
    }
    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
}

fn quiet(_: &str) {}

fn run(sp: &Path, fc: &FakeCall, root: &Path, jobs: usize) -> bf::RunReport {
    let call = |p: &str| fc.call(p);
    bf::run(
        sp,
        &call,
        &RunOpts {
            root: Some(root.to_path_buf()),
            jobs,
            ..Default::default()
        },
        &quiet,
    )
    .unwrap()
}

struct World {
    t: tempfile::TempDir,
    sp: PathBuf,
    impl_: PathBuf,
    root: PathBuf,
}

fn world() -> World {
    let (t, sp) = spine();
    let impl_ = make_git_repo(t.path(), "impl", 0, 0, true);
    registry::add_repo(&sp, "impl", &impl_, AddRepo::default()).unwrap();
    registry::add_group(&sp, "產品", &["impl"]).unwrap();
    let root = t.path().join("projects");
    World { t, sp, impl_, root }
}

#[test]
fn test_scan_maps_cwd_and_skips() {
    let w = world();
    write_session(
        &w.root,
        "a",
        "s1",
        &w.impl_.join("src"),
        &[("改索引", "好")],
        5,
    );
    write_session(
        &w.root,
        "b",
        "s2",
        &w.t.path().join("Drive/old/impl"),
        &[("舊路徑", "好")],
        5,
    );
    write_session(
        &w.root,
        "c",
        "s3",
        &w.t.path().join("elsewhere"),
        &[("別的", "好")],
        5,
    );
    write_session(&w.root, "d", "s4", &w.impl_, &[("正在寫", "好")], 0);
    let recs = bf::scan(&w.sp, Some(&w.root), None).unwrap();
    let st = |sid: &str| {
        let r = recs.iter().find(|r| r.sid == sid).unwrap();
        (r.repo.clone(), r.status.clone())
    };
    assert_eq!(st("s1"), (Some("impl".into()), "pending".into())); // 子目錄
    assert_eq!(st("s2"), (Some("impl".into()), "pending".into())); // Drive 舊路徑：資料夾名對上
    assert_eq!(st("s3"), (None, "unmapped".into()));
    assert_eq!(st("s4").1, "live"); // 還在寫的不碰
}

#[test]
fn test_compact_keeps_words_drops_noise() {
    let w = world();
    let p = write_session(
        &w.root,
        "a",
        "s1",
        &w.impl_,
        &[
            (
                "<system-reminder>別理我</system-reminder>改索引吧",
                "我改成增量",
            ),
            ("再補壓測", "好"),
        ],
        5,
    );
    let text = bf::compact(&p).unwrap().join("\n");
    assert!(text.contains("[t1] 【使用者】改索引吧") && text.contains("[t2] 【使用者】再補壓測"));
    assert!(text.contains("【agent】我改成增量") && text.contains("〔工具 Edit〕/x/a.py"));
    assert!(!text.contains("別理我") && !text.contains("巨量輸出") && !text.contains("祕密思考"));
}

#[test]
fn test_chunks_split_by_size() {
    let lines = vec!["a".repeat(30); 10];
    let parts = bf::chunks(&lines, 100);
    assert!(parts.len() == 4 && parts.iter().all(|p| p.chars().count() <= 100));
}

#[test]
fn test_run_resumable_and_failure_does_not_stop() {
    let w = world();
    write_session(&w.root, "a", "s1", &w.impl_, &[("改索引", "好")], 5);
    write_session(
        &w.root,
        "a",
        "s2",
        &w.impl_,
        &[("壞掉這場 s2marker", "好")],
        5,
    );
    let fc = FakeCall::new(&["s2marker"]);
    let res = run(&w.sp, &fc, &w.root, 1);
    assert!(res.extracted == 2 && res.failed.len() == 1 && res.consolidated == vec!["impl"]);
    let n = fc.prompts().len();
    let res = run(&w.sp, &FakeCall::new(&[]), &w.root, 1); // 續跑：只補失敗那場
    assert!(res.extracted == 1 && res.failed.is_empty());
    assert!(n >= 2);
}

#[test]
fn test_candidates_not_written_until_accept() {
    let w = world();
    write_session(&w.root, "a", "s1", &w.impl_, &[("改索引", "好")], 5);
    run(&w.sp, &FakeCall::new(&[]), &w.root, 1);
    let backfilled = || -> Vec<spine_mod::Event> {
        spine_mod::iter_events(&w.sp)
            .unwrap()
            .into_iter()
            .filter(|e| e.source == "backfill")
            .collect()
    };
    assert!(backfilled().is_empty());
    let text = bf::review_text(&w.sp, None).unwrap();
    assert!(
        text.contains("[d1]")
            && text.contains("[t1]")
            && text.contains("[h]")
            && text.contains("讓搜尋更快")
    );

    let refs = bf::accept(&w.sp, "impl", &["all"]).unwrap();
    assert_eq!(refs.len(), 3);
    let evs = backfilled();
    assert!(evs
        .iter()
        .all(|e| e.kv("repo") == Some("impl") && e.kv("group") == Some("產品")));
    assert!(evs[0].body.contains("回填自 2026-09-10"));
    let ctx = api::context_for(&w.sp, &Target::at(&w.impl_), false).unwrap();
    assert!(ctx["last_handoff"]["summary"]
        .as_str()
        .unwrap()
        .starts_with("加水位線")); // 交接最後寫＝上次做到哪
    assert!(ctx["open_loops"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["text"].as_str().unwrap().contains("補壓測")));
    assert!(bf::accept(&w.sp, "impl", &["all"]).unwrap().is_empty()); // 不重收
    assert!(bf::review_text(&w.sp, None).unwrap().contains("✔ 已收"));
    assert!(bf::accept(&w.sp, "impl", &["d99"]).is_err());
}

#[test]
fn test_goal_is_never_written() {
    let w = world();
    write_session(&w.root, "a", "s1", &w.impl_, &[("改索引", "好")], 5);
    run(&w.sp, &FakeCall::new(&[]), &w.root, 1);
    bf::accept(&w.sp, "impl", &["all"]).unwrap();
    assert!(registry::get_group(&w.sp, "產品").unwrap().goal().is_none()); // 目標只由使用者給
}

/// 先只登記上層 repo、後來子專案也登記了：已抽過的會話改歸子專案，兩邊都重彙整，不重抽。
#[test]
fn test_remap_when_more_specific_repo_registered() {
    let w = world();
    let sub = w.impl_.join("projects").join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    write_session(&w.root, "a", "s1", &sub, &[("改索引", "好")], 5);
    run(&w.sp, &FakeCall::new(&[]), &w.root, 1);
    assert!(!bf::candidates(&w.sp, Some("impl")).unwrap().is_empty());
    registry::add_repo(&w.sp, "sub", &sub, AddRepo::default()).unwrap();
    let fc = FakeCall::new(&[]);
    let res = run(&w.sp, &fc, &w.root, 1);
    assert!(res.extracted == 0 && res.consolidated == vec!["sub"]);
    assert!(fc.prompts().iter().all(|p| p.contains("逐場整理"))); // 只重彙整，沒重抽
    assert!(!bf::candidates(&w.sp, Some("sub")).unwrap().is_empty());
    assert!(bf::candidates(&w.sp, Some("impl")).unwrap().is_empty()); // 沒會話了的 repo 候選清掉
}

#[test]
fn test_parallel_jobs_same_result() {
    let w = world();
    for i in 0..6 {
        let u = format!("改索引 {i}");
        write_session(&w.root, "a", &format!("s{i}"), &w.impl_, &[(&u, "好")], 5);
    }
    let res = run(&w.sp, &FakeCall::new(&[]), &w.root, 4);
    assert!(res.extracted == 6 && res.failed.is_empty() && res.consolidated == vec!["impl"]);
    let n = std::fs::read_dir(w.sp.join(".state/backfill/sessions"))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .and_then(|x| x.to_str())
                == Some("json")
        })
        .count();
    assert_eq!(n, 6);
}

/// 改掛會話後、彙整前被中斷：下一輪要看出候選與實際會話對不上，清掉／重彙整。
#[test]
fn test_interrupted_remap_heals_on_next_run() {
    let w = world();
    let sub = w.impl_.join("projects").join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    write_session(&w.root, "a", "s1", &sub, &[("改索引", "好")], 5);
    run(&w.sp, &FakeCall::new(&[]), &w.root, 1);
    registry::add_repo(&w.sp, "sub", &sub, AddRepo::default()).unwrap();
    // 模擬：改掛寫進去了，但沒彙整就被殺
    let p = w.sp.join(".state/backfill/sessions/s1.json");
    let mut s: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    s["repo"] = json!("sub");
    std::fs::write(&p, s.to_string()).unwrap();
    let res = run(&w.sp, &FakeCall::new(&[]), &w.root, 1);
    assert_eq!(res.consolidated, vec!["sub"]);
    assert!(bf::candidates(&w.sp, Some("impl")).unwrap().is_empty());
    assert!(!bf::candidates(&w.sp, Some("sub")).unwrap().is_empty());
}

// 未搬：test_cli_accept_positional_repo——等 ctx-cli（真 CLI 的位置參數解析與輸出「寫進 spine 2 筆」／「都收過了」）。
