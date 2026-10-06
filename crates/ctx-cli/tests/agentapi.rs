//! Agent 介面的 MCP 分層。對應 tests/test_agentapi.py 裡 ctx-core tests/agentapi.rs 標「等 ctx-cli」的部分：
//! test_mcp_default_exposes_only_work_tools、test_mcp_admin_via_config、
//! test_mcp_work_tools_return_json_and_structured_errors、test_mcp_admin_open_loops_is_scoped。
//! （test_workbench_session_shows_agent_status 屬 workbench，不在這裡。）

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ctx_cli::mcp::{handle_message, ADMIN_TOOLS, WORK_TOOLS};
use ctx_core::agentapi::{self as api, Target};
use ctx_core::registry::{self, AddRepo};
use ctx_core::testkit::{make_git_repo, spine};
use serde_json::{json, Value};

struct World {
    _t: tempfile::TempDir,
    sp: PathBuf,
    impl_: PathBuf,
    notes: PathBuf,
}

/// 兩組：產品（plan＋impl）、研究（notes）。impl 底下有子目錄。
fn two_groups() -> World {
    let (t, sp) = spine();
    let plan = make_git_repo(t.path(), "plan", 0, 0, true);
    let impl_ = make_git_repo(t.path(), "impl", 0, 0, true);
    std::fs::create_dir_all(impl_.join("src").join("deep")).unwrap();
    let notes = make_git_repo(t.path(), "notes", 0, 0, true);
    for (rid, p) in [("plan", &plan), ("impl", &impl_), ("notes", &notes)] {
        registry::add_repo(&sp, rid, p, AddRepo::default()).unwrap();
    }
    registry::add_group(&sp, "產品", &["plan", "impl"]).unwrap();
    registry::add_group(&sp, "研究", &["notes"]).unwrap();
    World {
        _t: t,
        sp,
        impl_,
        notes,
    }
}

fn mcp(sp: &Path, method: &str, params: Value, admin: Option<bool>, cwd: Option<&Path>) -> Value {
    handle_message(
        sp,
        &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
        admin,
        cwd,
    )
    .unwrap()
}

fn names(r: &Value) -> HashSet<String> {
    r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn test_mcp_default_exposes_only_work_tools() {
    let w = two_groups();
    let got = names(&mcp(&w.sp, "tools/list", json!({}), None, None));
    let work: HashSet<String> = WORK_TOOLS.iter().map(|t| t.name.clone()).collect();
    assert_eq!(got, work);
    assert!(!got.contains("spawn") && !got.contains("unread"));
    let full_r = mcp(&w.sp, "tools/list", json!({}), Some(true), None);
    let full = names(&full_r);
    assert!(ADMIN_TOOLS.iter().all(|t| full.contains(&t.name)));
    // 說明不再用內部規格代號
    for t in full_r["result"]["tools"].as_array().unwrap() {
        assert!(
            !t["description"].as_str().unwrap().starts_with("P1"),
            "{}",
            t["name"]
        );
    }
    let r = mcp(
        &w.sp,
        "tools/call",
        json!({"name": "spawn", "arguments": {}}),
        None,
        None,
    );
    assert!(r["error"]["message"].as_str().unwrap().contains("--admin"));
}

#[test]
fn test_mcp_admin_via_config() {
    let w = two_groups();
    std::fs::write(w.sp.join("config.yaml"), "mcp:\n  admin: true\n").unwrap();
    let got = names(&mcp(&w.sp, "tools/list", json!({}), None, None));
    assert!(got.contains("registry_add"));
}

fn body(r: &Value) -> Value {
    serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn test_mcp_work_tools_return_json_and_structured_errors() {
    let w = two_groups();
    let r = mcp(
        &w.sp,
        "tools/call",
        json!({"name": "add_todo", "arguments": {"text": "驗證"}}),
        None,
        Some(&w.impl_),
    );
    let b = body(&r);
    assert!(b["ok"] == true && b["id"] == "#1" && b["todo"]["group"] == "產品");
    let r = mcp(
        &w.sp,
        "tools/call",
        json!({"name": "add_todo", "arguments": {"text": "跨組", "group": "研究"}}),
        None,
        Some(&w.impl_),
    );
    assert_eq!(r["result"]["isError"], true);
    let err = body(&r);
    assert!(err["error"] == "cross_scope" && err["hint"].as_str().is_some_and(|h| !h.is_empty()));
    let r = mcp(
        &w.sp,
        "tools/call",
        json!({"name": "context_for", "arguments": {"card": false}}),
        None,
        Some(&w.impl_),
    );
    let c = body(&r);
    assert!(c["where"]["repo"]["id"] == "impl" && c.get("card").is_none());
    let list = mcp(&w.sp, "tools/list", json!({}), None, None);
    let schema = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "report_status")
        .unwrap()["inputSchema"]
        .clone();
    assert_eq!(schema["properties"]["status"]["enum"], json!(api::STATUSES));
}

#[test]
fn test_mcp_admin_open_loops_is_scoped() {
    let w = two_groups();
    api::add_todo(
        &w.sp,
        "產品的事",
        None,
        &Target::at(&w.impl_),
        false,
        "agent",
    )
    .unwrap();
    api::add_todo(
        &w.sp,
        "研究的事",
        None,
        &Target::at(&w.notes),
        false,
        "agent",
    )
    .unwrap();
    let r = mcp(
        &w.sp,
        "tools/call",
        json!({"name": "open_loops", "arguments": {"group": "研究"}}),
        Some(true),
        None,
    );
    let text = r["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("研究的事") && !text.contains("產品的事"));
}
