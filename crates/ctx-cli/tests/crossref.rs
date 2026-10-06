//! 跨 repo 參考輪的 CLI／MCP 入口。對應 tests/test_crossref.py 裡 ctx-core tests/crossref.rs 標「等 ctx-cli」的部分：
//! test_audit_cli_includes_doc_refs、test_mcp_work_tools_include_crossref、test_cli_crossref_flow。

mod common;

use std::path::{Path, PathBuf};

use common::*;
use ctx_cli::mcp::handle_message;
use ctx_core::registry::{self, AddRepo};
use serde_json::{json, Value};

fn commit(repo: &Path, msg: &str) {
    git(repo, &["add", "-A"], &[]);
    git(repo, &["commit", "-q", "-m", msg], &[]);
}

struct World {
    t: tempfile::TempDir,
    sp: PathBuf,
    course: PathBuf,
}

/// notes（知識庫，有書摘目錄）feeds course（課程 repo）；other 是不相關的 repo。（同 ctx-core tests/crossref.rs）
fn world() -> World {
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
    World { t, sp, course }
}

#[test]
fn test_audit_cli_includes_doc_refs() {
    let w = world();
    std::fs::write(w.course.join("AGENTS.md"), "看 `projects/notes/`\n").unwrap();
    let r = ctx(w.t.path()).spine(&w.sp).run(&["registry", "audit"]);
    assert!(
        r.code == 1 && r.stdout.contains("[文件路徑失效]"),
        "{}",
        r.stdout
    );
}

fn mcp(sp: &Path, msg: Value, admin: Option<bool>, cwd: Option<&Path>) -> Value {
    handle_message(sp, &msg, admin, cwd).unwrap()
}

#[test]
fn test_mcp_work_tools_include_crossref() {
    let w = world();
    let r = mcp(
        &w.sp,
        json!({"id": 1, "method": "tools/list"}),
        Some(false),
        None,
    );
    let names: Vec<&str> = r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for n in ["read_from", "ask_repo", "repo_status", "search_knowledge"] {
        assert!(names.contains(&n));
    }
    assert!(!names.contains(&"registry_export"));
    let r = mcp(
        &w.sp,
        json!({"id": 2, "method": "tools/call", "params": {
            "name": "read_from", "arguments": {"ref": "notes:書摘/cache-book/ch01.md"}}}),
        None,
        Some(&w.course),
    );
    let res = &r["result"];
    assert_eq!(res["isError"], false);
    let body: Value = serde_json::from_str(res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(body["content"].as_str().unwrap().contains("快取失效"));
    let r = mcp(
        &w.sp,
        json!({"id": 3, "method": "tools/call", "params": {
            "name": "registry_export", "arguments": {"id": "other", "name": "全部", "path": "."}}}),
        Some(true),
        None,
    );
    let res = &r["result"];
    assert_eq!(res["isError"], false);
    assert!(res["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("other:全部"));
}

#[test]
fn test_cli_crossref_flow() {
    let w = world();
    let c = ctx(w.t.path()).spine(&w.sp);
    let r = c.ok(&["resolve", "notes:書摘"]);
    assert!(r.stdout.trim().lines().next().unwrap().ends_with("books"));
    let r = c.ok(&[
        "read",
        "notes:書摘/cache-book/ch01.md",
        "--cwd",
        s(&w.course),
        "--json",
    ]);
    assert_eq!(r.json()["repo"], "notes");
    let r = c.ok(&[
        "refs",
        "cite",
        "notes:書摘/cache-book/ch02.md",
        "--note",
        "複製進講義",
        "--cwd",
        s(&w.course),
    ]);
    assert!(r.stdout.contains("已記引用"));
    let r = c.ok(&["repo", "notes", "--peek", "--json"]);
    assert_eq!(r.json()["repo"], "notes");
    let r = c.ok(&["registry", "exports", "--json"]);
    assert_eq!(r.json()[0]["name"], "書摘");
    std::fs::write(w.course.join("AGENTS.md"), "看 `projects/notes/`\n").unwrap();
    let r = c.run(&["refs", "check"]);
    assert!(r.code == 1 && r.stdout.contains("projects/notes/"));
    let r = c.ok(&[
        "ask",
        "notes",
        "快取？",
        "--provider",
        "mock",
        "--cwd",
        s(&w.course),
    ]);
    assert!(r.stdout.contains("notes 的回答"));
}
