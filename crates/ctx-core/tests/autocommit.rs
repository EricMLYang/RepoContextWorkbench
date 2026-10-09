//! Rust 才有（N-01）：會話結束時的 spine 自動 commit（`spine::autocommit`）。

use chrono::{Duration, Local};
use ctx_core::spine::{append_event, autocommit, AutoCommit, NO_TOKENS};
use ctx_core::testkit::{git, spine};

fn commit_count(d: &std::path::Path) -> usize {
    let out = std::process::Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(d)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

/// spine 做一個 `hours_ago` 小時前的初始 commit，再寫一筆事件（＝有變更）。
fn spine_with_history(hours_ago: i64) -> (tempfile::TempDir, std::path::PathBuf) {
    let (tmp, d) = spine();
    let when = (Local::now() - Duration::hours(hours_ago)).to_rfc3339();
    git(&d, &["add", "-A"], &[]);
    git(
        &d,
        &["commit", "-q", "-m", "init"],
        &[("GIT_AUTHOR_DATE", &when), ("GIT_COMMITTER_DATE", &when)],
    );
    append_event(&d, "decision", "manual", NO_TOKENS, "選 A", None).unwrap();
    (tmp, d)
}

fn set_identity(d: &std::path::Path) {
    git(d, &["config", "user.name", "t"], &[]);
    git(d, &["config", "user.email", "t@t"], &[]);
}

#[test]
fn autocommit_commits_when_last_commit_is_old() {
    let (_t, d) = spine_with_history(8);
    set_identity(&d);
    assert_eq!(
        autocommit(&d, None).unwrap(),
        AutoCommit::Committed { push: false }
    );
    assert_eq!(commit_count(&d), 2);
    assert_eq!(autocommit(&d, None).unwrap(), AutoCommit::Clean);
}

#[test]
fn autocommit_leaves_recent_history_to_the_timer() {
    let (_t, d) = spine_with_history(1);
    assert_eq!(autocommit(&d, None).unwrap(), AutoCommit::Recent);
    assert_eq!(commit_count(&d), 1);
}

#[test]
fn autocommit_does_not_start_history_for_the_user() {
    let (_t, d) = spine(); // git init 了但一個 commit 都沒有
    append_event(&d, "decision", "manual", NO_TOKENS, "選 A", None).unwrap();
    assert_eq!(autocommit(&d, None).unwrap(), AutoCommit::NoHistory);
}

#[test]
fn autocommit_can_be_turned_off() {
    let (_t, d) = spine_with_history(8);
    let cfg = d.join("config.yaml");
    let text = std::fs::read_to_string(&cfg).unwrap();
    std::fs::write(
        &cfg,
        text.replacen("thresholds:", "thresholds:\n  spine_autocommit_hours: 0", 1),
    )
    .unwrap();
    assert_eq!(autocommit(&d, None).unwrap(), AutoCommit::Disabled);
    assert_eq!(commit_count(&d), 1);
}
