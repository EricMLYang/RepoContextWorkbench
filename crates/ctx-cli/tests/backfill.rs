//! 回填的 CLI 入口。對應 tests/test_backfill.py 的 test_cli_accept_positional_repo
//! （ctx-core tests/backfill.rs 標「等 ctx-cli」）。

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::*;
use ctx_core::backfill::{self as bf, RunOpts};
use ctx_core::registry::{self, AddRepo};
use ctx_core::Result;
use serde_json::json;

fn write_session(
    root: &Path,
    proj: &str,
    sid: &str,
    cwd: &Path,
    turns: &[(&str, &str)],
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
                         "message": {"role": "assistant", "content": [{"type": "text", "text": a}]}}));
    }
    let p = d.join(format!("{sid}.jsonl"));
    let text: Vec<String> = rows.iter().map(|r| r.to_string()).collect();
    std::fs::write(&p, text.join("\n")).unwrap();
    let t = SystemTime::now() - Duration::from_secs(5 * 3600);
    std::fs::File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_accessed(t).set_modified(t))
        .unwrap();
    p
}

/// 假模型（同 ctx-core tests/backfill.rs 的 FakeCall）。
fn fake_call(prompt: &str) -> Result<String> {
    if prompt.contains("逐場整理") {
        return Ok(format!(
            "好的：{}",
            json!({"decisions": [{"text": "索引改增量重建", "basis": "全量太慢", "sid": "s1aaaaaa", "date": "2026-09-10"}],
                   "open_todos": [{"text": "補壓測", "sid": "s1aaaaaa", "date": "2026-09-10", "uncertain": false}],
                   "handoff": {"done": "加水位線", "next": "跑壓測", "sid": "s1aaaaaa", "date": "2026-09-10"},
                   "goal_hints": [{"text": "讓搜尋更快", "sid": "s1aaaaaa"}]})
        ));
    }
    Ok(json!({"summary": "改索引", "decisions": [{"text": "索引改增量重建", "basis": "全量太慢", "turn": "t1"}],
              "todos": [{"text": "補壓測", "turn": "t2"}], "done": ["加水位線"], "next": "跑壓測",
              "goal_hints": ["讓搜尋更快"]})
    .to_string())
}

/// 真 CLI：`accept <repo> d1 t1`；「--repo 在後、編號在前」也吃得下。
#[test]
fn test_cli_accept_positional_repo() {
    let (t, sp) = spine();
    let impl_ = make_git_repo(t.path(), "impl", 0, 0, true);
    registry::add_repo(&sp, "impl", &impl_, AddRepo::default()).unwrap();
    registry::add_group(&sp, "產品", &["impl"]).unwrap();
    let root = t.path().join("projects");
    write_session(&root, "a", "s1", &impl_, &[("改索引", "好")]);
    let call = fake_call;
    bf::run(
        &sp,
        &call,
        &RunOpts {
            root: Some(root),
            jobs: 1,
            ..Default::default()
        },
        &|_: &str| {},
    )
    .unwrap();
    let c = ctx(t.path()).spine(&sp);
    let r = c.run(&["backfill", "accept", "impl", "d1", "t1"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("寫進 spine 2 筆"));
    let r = c.run(&["backfill", "accept", "d1", "--repo", "impl"]);
    assert!(r.code == 0 && r.stdout.contains("都收過了"), "{}", r.all());
}
