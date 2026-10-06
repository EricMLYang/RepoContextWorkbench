//! Claude Code hooks 的引擎層測試。Python 沒有 test_hooks.py——hook 的測試散在：
//! - tests/test_agentapi.py 的 hooks 段（四個，搬在 tests/agentapi.rs）
//! - tests/test_crossref.py::test_session_start_injects_neighbor_map_and_drift（這裡）
//! - tests/test_ruminate.py::test_session_start_shows_fresh（這裡）
//! - tests/test_agent_cli_e2e.py::test_hook_commands_via_stdin（引擎部分在這裡：stdin 解析＋輸出；
//!   真行程的 exit code 在 crates/ctx-cli/tests/agent_cli_e2e.rs）

use std::path::{Path, PathBuf};

use ctx_core::agentapi::{self as api, Target};
use ctx_core::hooks;
use ctx_core::registry::{self, AddRepo};
use ctx_core::ruminate;
use ctx_core::testkit::{git, make_git_repo, spine};
use serde_json::{json, Value};

fn commit(repo: &Path, msg: &str) {
    git(repo, &["add", "-A"], &[]);
    git(repo, &["commit", "-q", "-m", msg], &[]);
}

fn context_of(out: &str) -> String {
    let v: Value = serde_json::from_str(out).unwrap();
    v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .to_string()
}

struct XWorld {
    _t: tempfile::TempDir,
    sp: PathBuf,
    notes: PathBuf,
    course: PathBuf,
    other: PathBuf,
}

/// 同 test_crossref 的 world：notes（有書摘）feeds course；other 不相關。
fn xref_world() -> XWorld {
    let (t, sp) = spine();
    let notes = make_git_repo(t.path(), "notes", 0, 0, true);
    let cb = notes.join("books").join("cache-book");
    std::fs::create_dir_all(&cb).unwrap();
    std::fs::write(
        cb.join("ch01.md"),
        "# 第一章 快取失效\n快取失效是最難的問題之一。\n",
    )
    .unwrap();
    std::fs::write(cb.join("ch02.md"), "# 第二章 一致性\n寫入策略與一致性。\n").unwrap();
    std::fs::write(
        notes.join("keys.md"),
        "api_key = 'ABCDEFGHIJKLMNOPQRSTUV'\n",
    )
    .unwrap();
    commit(&notes, "books");
    let course = make_git_repo(t.path(), "course", 0, 0, true);
    let other = make_git_repo(t.path(), "other", 0, 0, true);
    for (rid, p) in [("notes", &notes), ("course", &course), ("other", &other)] {
        registry::add_repo(&sp, rid, p, AddRepo::default()).unwrap();
    }
    registry::add_group(&sp, "課程", &["course"]).unwrap();
    registry::add_group(&sp, "知識", &["notes"]).unwrap();
    registry::set_export(&sp, "notes", "書摘", "books", Some("逐章書摘")).unwrap();
    registry::relate(&sp, "notes", "course", "feeds", None, &["書摘"]).unwrap();
    XWorld {
        _t: t,
        sp,
        notes,
        course,
        other,
    }
}

/// test_crossref.py::test_session_start_injects_neighbor_map_and_drift
#[test]
fn test_session_start_injects_neighbor_map_and_drift() {
    let w = xref_world();
    api::read_from(
        &w.sp,
        "notes:書摘/cache-book/ch01.md",
        None,
        Some(&w.course),
    )
    .unwrap();
    std::fs::write(w.notes.join("books/cache-book/ch01.md"), "改\n").unwrap();
    commit(&w.notes, "改第一章");
    let out = hooks::session_start(
        &w.sp,
        &json!({"session_id": "s1", "cwd": w.course.to_string_lossy()}),
        None,
    );
    let ctx = context_of(&out);
    assert!(
        ctx.contains("鄰居 repo")
            && ctx.contains("`notes:書摘`→books/")
            && ctx.contains("逐章書摘")
    );
    assert!(ctx.contains("改了 1 次") && ctx.contains("read_from"));
    let other = hooks::session_start(
        &w.sp,
        &json!({"session_id": "s2", "cwd": w.other.to_string_lossy()}),
        None,
    );
    assert!(!context_of(&other).contains("鄰居 repo"));
}

const CARD: &str = "# 向量索引的冷啟動與增量重建

增量重建要記水位線，冷啟動時先載入快照再補增量。
倒排索引的分段合併策略會影響查詢延遲；合併太頻繁會拖垮寫入吞吐。
";

/// test_ruminate 的 world：產品（impl，手上在做索引重建）、知識（cards）。
fn rum_world() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let (t, sp) = spine();
    let impl_ = make_git_repo(t.path(), "impl", 0, 0, true);
    let cards = make_git_repo(t.path(), "cards", 0, 0, true);
    registry::add_repo(&sp, "impl", &impl_, AddRepo::default()).unwrap();
    registry::add_repo(&sp, "cards", &cards, AddRepo::default()).unwrap();
    registry::add_group(&sp, "產品", &["impl"]).unwrap();
    registry::add_group(&sp, "知識", &["cards"]).unwrap();
    registry::set_group_field(&sp, "產品", "goal", "把搜尋服務的向量索引改成增量重建").unwrap();
    let t_impl = Target::at(&impl_);
    api::add_todo(
        &sp,
        "冷啟動太慢：載入快照後補增量，量測查詢延遲",
        None,
        &t_impl,
        false,
        "agent",
    )
    .unwrap();
    api::log_decision(
        &sp,
        "倒排索引分段合併改成每小時一次，避免拖垮寫入吞吐",
        &t_impl,
        None,
        false,
        "agent",
    )
    .unwrap();
    (t, sp, impl_, cards)
}

/// test_ruminate.py::test_session_start_shows_fresh
#[test]
fn test_session_start_shows_fresh() {
    let (_t, sp, impl_, cards) = rum_world();
    std::fs::write(cards.join("索引重建.md"), CARD).unwrap();
    ruminate::run(&sp, None, None).unwrap();
    let out = hooks::session_start(
        &sp,
        &json!({"session_id": "s1", "cwd": impl_.to_string_lossy()}),
        None,
    );
    let text = context_of(&out);
    assert!(text.contains("反芻") && text.contains("cards:索引重建.md"));
    let ctx = api::context_for(&sp, &Target::at(&impl_), false).unwrap();
    assert_eq!(ctx["fresh"][0]["ref"], "cards:索引重建.md");
}

/// test_agent_cli_e2e.py::test_hook_commands_via_stdin 的引擎部分：stdin JSON → 輸出字串；
/// session-end 不印東西；壞輸入不炸、回空字串。（真行程、exit code 0、找不到 spine：crates/ctx-cli/tests/agent_cli_e2e.rs）
#[test]
fn test_hook_commands_via_stdin() {
    let (_t, sp, impl_, _) = rum_world();
    let stdin = json!({"session_id": "abc", "cwd": impl_.to_string_lossy()}).to_string();
    let out = hooks::run_hook(&sp, "session-start", &stdin);
    assert!(context_of(&out).contains("你在 impl"));
    assert_eq!(
        hooks::run_hook(&sp, "session-end", r#"{"session_id": "abc"}"#),
        ""
    );
    // 壞輸入＝{}：cwd 退回行程目錄（不在 registry）→ 不輸出
    assert_eq!(hooks::parse_payload("not json"), json!({}));
    assert_eq!(hooks::parse_payload(""), json!({}));
}
