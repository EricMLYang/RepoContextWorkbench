//! L1：本組工作摘要（2026-09-12 UX 檢討 §3）。對應 tests/test_summary.py。
//!
//! 這組測試守的是紀律不是版面：每格要嘛有依據（source），要嘛明講沒有依據；
//! commit 數只能當活動陳述，不准當成進度；範圍跟情境卡、簡報同一份規則。

use std::path::{Path, PathBuf};

use ctx_core::registry::{self, AddRepo};
use ctx_core::spine;
use ctx_core::summary::{self, Summary};
use ctx_core::testkit::{make_git_repo, spine};

fn two_groups() -> (tempfile::TempDir, PathBuf) {
    let (t, d) = spine();
    for name in ["prod", "notes"] {
        let p = make_git_repo(t.path(), name, 1, 0, true);
        registry::add_repo(&d, name, &p, AddRepo::default()).unwrap();
    }
    registry::add_group(&d, "產品開發", &["prod"]).unwrap();
    registry::add_group(&d, "知識研究", &["notes"]).unwrap();
    (t, d)
}

fn build(d: &Path, group: &str) -> Summary {
    summary::build_summary(d, Some(group), None, None, None, None).unwrap()
}

fn ev(d: &Path, etype: &str, tokens: &[&str], body: &str) {
    spine::append_event(d, etype, "agent", tokens, body, None).unwrap();
}

#[test]
fn test_summary_每格都帶依據() {
    let (_t, d) = two_groups();
    registry::set_group_field(&d, "產品開發", "goal", "月底交出可驗收的版本").unwrap();
    ev(
        &d,
        "decision",
        &["group:產品開發"],
        "會話結果：比對完驗收條件\n兩邊差三條",
    );
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    spine::append_event(
        &d,
        "open-loop",
        "hotkey",
        &[
            "#1".to_string(),
            "group:產品開發".into(),
            format!("due:{today}"),
        ],
        "補齊缺的三條驗收條件",
        None,
    )
    .unwrap();
    let s = build(&d, "產品開發");
    let goal = s.goal.as_ref().unwrap();
    assert_eq!(goal.text, "月底交出可驗收的版本");
    assert!(goal.source.contains("registry.yaml"));
    let p = s.progress.as_ref().unwrap();
    assert_eq!(p.text, "會話結果：比對完驗收條件");
    assert_eq!(p.source, "脊椎 decision [agent]");
    assert!(p.at.is_some());
    let n = s.next.as_ref().unwrap();
    assert!(n.text.contains("補齊缺的三條驗收條件"));
    assert!(n.source.starts_with("脊椎 open-loop due:"));
    assert!(s.gaps.is_empty());
}

/// 資訊不足時直接說明——不拿 commit 數充當進度或價值。
#[test]
fn test_summary_沒有依據就直說沒有() {
    let (_t, d) = two_groups();
    let s = build(&d, "知識研究");
    assert!(
        s.goal.is_none()
            && s.goal_gap
                .as_deref()
                .unwrap()
                .contains("尚未登記這組的目標")
    );
    assert!(
        s.progress.is_none()
            && s.progress_gap
                .as_deref()
                .unwrap()
                .contains("脊椎沒有本組的進度紀錄")
    );
    assert!(s.next.is_none() && s.next_gap.is_some());
    assert_eq!(s.gaps.len(), 3);
    // 有 commit，但只以「活動」形式出現在變化欄，不會爬進進度或目標
    assert!(!s.changes.is_empty());
    assert!(s
        .changes
        .iter()
        .all(|c| c.source.contains("git log") || c.source.contains("git status")));
    assert!(s.changes_window.contains("沒有上次紀錄"));
}

/// 別組的進度與未結不得出現在本組摘要（§6.2 的同一條線）。
#[test]
fn test_summary_範圍跟簡報同一份規則() {
    let (_t, d) = two_groups();
    ev(&d, "decision", &["group:產品開發"], "產品組的判斷");
    spine::append_event(
        &d,
        "open-loop",
        "hotkey",
        &["#9", "group:產品開發", "due:2099-01-01"],
        "產品組的未結",
        None,
    )
    .unwrap();
    let s = build(&d, "知識研究");
    assert!(s.progress.is_none() && s.next.is_none());
    assert!(s.changes.iter().all(|c| !c.text.contains("產品組")));
    // 換成產品組就看得到
    let p = build(&d, "產品開發");
    assert_eq!(p.progress.unwrap().text, "產品組的判斷");
    assert!(p.next.unwrap().text.contains("產品組的未結"));
}

#[test]
fn test_summary_臨時範圍與全部講清楚限制() {
    let (_t, d) = two_groups();
    let ids = vec!["prod".to_string(), "notes".to_string()];
    let adhoc = summary::build_summary(&d, None, Some(&ids), None, None, None).unwrap();
    assert!(adhoc.goal.is_none() && adhoc.goal_gap.unwrap().contains("存成牌組"));
    let everything = summary::build_summary(&d, None, None, None, None, None).unwrap();
    assert_eq!(everything.group, "全部");
    assert!(everything.goal_gap.unwrap().contains("跨組總覽"));
}

#[test]
fn test_resume_task_只複述有依據的內容() {
    let (_t, d) = two_groups();
    let empty = build(&d, "知識研究").resume_task;
    assert!(empty.contains("脊椎沒有本組的進度紀錄") && empty.contains("先問我"));
    ev(
        &d,
        "decision",
        &["group:知識研究"],
        "會話結果：整理完三份筆記",
    );
    let filled = build(&d, "知識研究").resume_task;
    assert!(filled.contains("整理完三份筆記") && filled.contains("脊椎 decision [agent]"));
}

/// format_summary（CLI 用）與 JSON 形狀。Python 沒有獨立測試（經 CLI e2e）；在引擎層先釘住。
#[test]
fn format_summary_and_json_shape() {
    let (_t, d) = two_groups();
    let s = build(&d, "知識研究");
    let text = summary::format_summary(&s);
    assert!(text.starts_with("# 本組工作摘要 知識研究（"));
    assert!(text.contains("## 目前目標\n（尚未登記這組的目標"));
    assert!(text.contains("## 變化（近期（沒有上次紀錄可比，改列最近活動））"));
    assert!(text.contains("  依據：git log · notes "));
    let v = serde_json::to_value(&s).unwrap();
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec![
            "group",
            "saved_group",
            "repos",
            "at",
            "goal",
            "goal_gap",
            "progress",
            "progress_gap",
            "since",
            "changes",
            "changes_window",
            "next",
            "next_gap",
            "changes_gap",
            "gaps",
            "resume_task"
        ]
    );
}
