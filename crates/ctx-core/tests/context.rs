//! L1：組情境卡（P16 帶料升級）——預設 agent 以「一組 repo」為工作單位的那份開場料。對應 tests/test_context.py。

use chrono::{Duration, Local};
use ctx_core::context;
use ctx_core::registry;
use ctx_core::spine;
use ctx_core::testkit::spine_with_repos;

const NONE: &[&str] = &[];

#[test]
fn test_context_card_sections() {
    let (_t, d) = spine_with_repos();
    registry::relate(&d, "repo-a", "repo-b", "pm-of", None, NONE).unwrap();
    let now = Local::now().naive_local();
    spine::append_event(
        &d,
        "open-loop",
        "hotkey",
        &["#7", "group:g1", "due:2099-01-01"],
        "opened →「驗證 Y」",
        Some(now - Duration::minutes(5)),
    )
    .unwrap();
    spine::append_event(
        &d,
        "decision",
        "manual",
        &["group:g1"],
        "決定先做 a",
        Some(now - Duration::minutes(4)),
    )
    .unwrap();
    spine::append_event(
        &d,
        "decision",
        "manual",
        &["group:other"],
        "別組的事",
        Some(now - Duration::minutes(3)),
    )
    .unwrap();
    let text = context::build_context(&d, Some("g1"), None, None, None).unwrap();
    assert!(text.starts_with("# 組情境卡 g1"));
    for sec in [
        "## 成員與角色",
        "## 脈動",
        "## 未結",
        "## 最近事件",
        "## 你的角色",
    ] {
        assert!(text.contains(sec), "{sec}");
    }
    // 成員列帶 tier／關係讀法／近 7 天 commit 數
    assert!(
        text.contains("repo-a")
            && text.contains("規劃（PM）→ repo-b")
            && text.contains("PM 是 repo-a")
    );
    assert!(text.contains("近 7 天"));
    // 本組 loops 與事件進來，別組的不進
    assert!(text.contains("#7") && text.contains("驗證 Y"));
    assert!(text.contains("決定先做 a") && !text.contains("別組的事"));
    // 角色說明是通用措辭：講「組管家」，並點名留痕工具
    assert!(text.contains("組管家") && text.contains("log_decision") && text.contains("handoff"));
}

#[test]
fn test_context_role_prompt_override() {
    let (_t, d) = spine_with_repos();
    std::fs::write(
        d.join("config.yaml"),
        "session:\n  role_prompt: |\n    你是我的私人助理，名字叫小助。\n",
    )
    .unwrap();
    let text = context::build_context(&d, Some("g1"), None, None, None).unwrap();
    assert!(text.contains("小助") && !text.contains("組管家"));
}

#[test]
fn test_context_adhoc_scope() {
    let (_t, d) = spine_with_repos();
    let repos = vec!["repo-b".to_string()];
    let text = context::build_context(&d, None, Some(&repos), None, None).unwrap();
    let members = text.split("## 脈動").next().unwrap();
    assert!(members.contains("repo-b") && !members.contains("- repo-a"));
}
