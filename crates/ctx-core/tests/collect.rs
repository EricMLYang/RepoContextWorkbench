//! 對應 `tests/test_collect_brief.py` 裡 **collect** 相關的測試（brief 另搬）。
//! fixture 不經 registry：entries 由 `common::spine_with_repos` 手組。

mod common;

use std::collections::HashMap;

use ctx_core::agentmark;
use ctx_core::collect::{self, RepoSpec, RepoState};
use ctx_core::testkit::{git, make_git_repo};

fn by_id(states: Vec<RepoState>) -> HashMap<String, RepoState> {
    states.into_iter().map(|s| (s.id.clone(), s)).collect()
}

#[test]
fn test_collect_states() {
    let w = common::spine_with_repos();
    let entries = w.specs();
    let states = by_id(collect::collect_group(&entries, None));
    assert_eq!(states["repo-a"].dirty, 0);
    assert_eq!(states["repo-b"].dirty, 2);
    assert!(states["repo-b"].dirty_days.unwrap() >= 4.5); // fixture 設 5 天
    assert!(states["repo-a"].last_commit_days.is_some());
    assert!(matches!(
        states["repo-a"].branch.as_deref(),
        Some("main") | Some("master")
    ));
    assert_eq!(states["repo-a"].last_subject, "init");
    // fixture repo 無 remote → ahead/behind 查不到＝None（fetch 後才準的 caveat）
    assert!(states["repo-a"].ahead.is_none());
    let table = collect::format_table(&collect::collect_group(&entries, None));
    assert!(table.contains("↑未push") && table.contains("branch"));
}

/// gitpane 欄位課：worktree 數（agent 並行開發的訊號）。
#[test]
fn test_collect_worktrees() {
    let w = common::spine_with_repos();
    let wt = w.tmp.join("wt-a");
    git(
        &w.repo_a,
        &[
            "worktree",
            "add",
            "-q",
            wt.to_str().unwrap(),
            "-b",
            "wt-branch",
        ],
        &[],
    );
    let st = collect::collect_repo(&RepoSpec::new("repo-a", &w.repo_a));
    assert_eq!(st.worktrees, 1);
    let st_b = collect::collect_repo(&RepoSpec::new("repo-b", &w.repo_b));
    assert_eq!(st_b.worktrees, 0);
}

/// gitpane 課的 agent 偵測：存活會話標在 cwd 所屬 repo；pid 死了的殘骸自動清。
/// 只在 unix 跑：Windows 的 pid 活性檢查尚未實作（一律當活著）。
#[cfg(unix)]
#[test]
fn test_collect_agent_detection() {
    let w = common::spine_with_repos();
    // 活會話（用自己的 pid＝必活）
    agentmark::mark(
        &w.spine,
        "t1",
        &w.repo_a,
        "claude",
        i64::from(std::process::id()),
    )
    .unwrap();
    // 殘骸：spawn 一個立即結束的進程，拿它的 pid
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead_pid = i64::from(child.id());
    child.wait().unwrap();
    agentmark::mark(&w.spine, "t2", &w.repo_a, "codex", dead_pid).unwrap();
    let entries = w.specs();
    let states = by_id(collect::collect_group(&entries, Some(&w.spine)));
    assert_eq!(states["repo-a"].agents, vec!["claude"]); // 殘骸不算
    assert!(states["repo-b"].agents.is_empty());
    // 殘骸 marker 已被清掉
    assert!(!w
        .spine
        .join(".state")
        .join("agent_sessions")
        .join("t2.json")
        .exists());
    // 不帶 spine_dir（舊呼叫路徑）不標，也不炸
    let states2 = by_id(collect::collect_group(&entries, None));
    assert!(states2["repo-a"].agents.is_empty());
    agentmark::unmark(&w.spine, "t1");
}

#[test]
fn test_collect_external_skips_dirty() {
    let tmp = tempfile::tempdir().unwrap();
    let p = make_git_repo(tmp.path(), "ext-repo", 0, 3, true);
    // Python 版經 registry.add_repo(type="external")；這裡直接給 entry
    let st = collect::collect_repo(&RepoSpec::new("ext-repo", &p).with_type("external"));
    assert!(st.dirty == 0 && st.note.contains("P4"));
}

/// 2026-09-08 組為單位輪：監控專注活動頻繁度＋近期 commit 內容。
#[test]
fn test_collect_activity_and_pulse() {
    let w = common::spine_with_repos();
    let entries = w.specs();
    let ra = &w.repo_a;
    std::fs::write(ra.join("x.md"), "x\n").unwrap();
    git(ra, &["add", "-A"], &[]);
    git(ra, &["commit", "-q", "-m", "feat: 加 x 頁"], &[]);
    let list = collect::collect_group(&entries, None);
    let states = by_id(list.clone());
    let (a, b) = (&states["repo-a"], &states["repo-b"]);
    assert!(a.commits_7d == Some(2) && a.commits_30d == Some(2)); // init（1 天前）＋ 今天
    assert!(b.commits_7d == Some(1) && b.commits_30d == Some(1)); // init（6 天前）
    assert_eq!(a.recent_commits[0].subject, "feat: 加 x 頁");
    let keys: Vec<String> = serde_json::to_value(&a.recent_commits[0])
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(keys, vec!["date", "hash", "subject"]);
    let pulse = collect::group_pulse(&list);
    assert!(pulse.commits_7d == 3 && pulse.commits_30d == 3);
    assert_eq!(pulse.most_active.as_deref(), Some("repo-a"));
    let latest = pulse.latest.as_ref().unwrap();
    assert!(latest.repo == "repo-a" && latest.subject.contains("加 x 頁"));
    assert!(pulse.quiet_30d.is_empty()); // 兩個都有 30 天內 commit
    let table = collect::format_table(&list);
    assert!(table.contains("7d"));
    // 近期 commit 清單（跨 repo 按日期新→舊）
    let recent = collect::recent_across(&list, 10);
    assert!(recent[0].repo == "repo-a" && recent[0].subject == "feat: 加 x 頁");
    assert_eq!(recent.len(), 3);
}

// 未搬（brief 模組不在這波，且需 spine/registry）：test_brief_shape、test_brief_quiet_when_all_clean、
// test_brief_over_three_questions_warns、test_question_cards_structured、test_defer_label_matches_behaviour、
// test_brief_loops_stay_inside_the_named_group、test_brief_has_pulse_line。

// ---- 以下為 Rust 版補測（非 Python 一對一）----

/// 狀態序列化鍵名＝Python dict 鍵名（`type` 不是 `repo_type`），--json 對拍靠這個。
#[test]
fn repo_state_json_keys_match_python() {
    let w = common::spine_with_repos();
    let st = collect::collect_repo(&RepoSpec::new("repo-a", &w.repo_a));
    let v = serde_json::to_value(&st).unwrap();
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec![
            "id",
            "type",
            "tier",
            "path",
            "exists",
            "dirty",
            "dirty_days",
            "last_commit_days",
            "ahead",
            "behind",
            "branch",
            "last_subject",
            "worktrees",
            "commits_7d",
            "commits_30d",
            "recent_commits",
            "agents",
            "note"
        ]
    );
    let missing = collect::collect_repo(&RepoSpec::new("ghost", w.tmp.join("nope")));
    assert!(!missing.exists && missing.note == "路徑不存在");
    let pulse = serde_json::to_value(collect::group_pulse(&[st])).unwrap();
    assert_eq!(pulse["per_repo_7d"]["repo-a"], 1);
}

#[test]
fn pulse_line_shapes() {
    let w = common::spine_with_repos();
    let list = collect::collect_group(&w.specs(), None);
    let line = collect::pulse_line(&collect::group_pulse(&list));
    assert!(
        line.starts_with("近 7 天 2 commits／30 天 2（repo-a 1、repo-b 1）；最新：repo-a "),
        "{line}"
    );
    assert_eq!(
        collect::pulse_line(&collect::group_pulse(&[])),
        "（沒有可採集的 mine repo）"
    );
}
