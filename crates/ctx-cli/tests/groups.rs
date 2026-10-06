//! 組維護的 CLI／MCP 入口。對應 tests/test_groups.py 裡 ctx-core tests/groups.rs 標「等 ctx-cli」的部分。

mod common;

use std::path::{Path, PathBuf};

use common::*;
use ctx_cli::mcp::{handle_message, tool_table};
use ctx_core::ops::{self, Via};
use ctx_core::registry::{self, AddRepo};
use ctx_core::spine::{self as spine_mod, EventType};
use serde_json::{json, Value};

/// g1＝repo-a,repo-b；另登記 repo-c（不在任何組）。
fn three() -> (tempfile::TempDir, PathBuf) {
    let (t, d) = spine_with_repos();
    let rc = make_git_repo(t.path(), "repo-c", 0, 0, true);
    registry::add_repo(&d, "repo-c", &rc, AddRepo::default()).unwrap();
    (t, d)
}

fn members(d: &Path, name: &str) -> Vec<String> {
    registry::get_group(d, name).unwrap().members()
}

fn decisions(d: &Path) -> Vec<spine_mod::Event> {
    spine_mod::iter_events(d)
        .unwrap()
        .into_iter()
        .filter(|e| e.etype == EventType::Decision)
        .collect()
}

#[test]
fn test_cli_group_maintenance() {
    let (t, d) = three();
    let c = ctx(t.path()).spine(&d);
    let r = c.ok(&["group", "add-member", "g1", "repo-c"]);
    assert!(r.stdout.contains("repo-c"));
    c.ok(&["group", "remove-member", "g1", "repo-a,repo-b"]);
    assert_eq!(members(&d, "g1"), vec!["repo-c"]);
    c.ok(&["group", "rename", "g1", "新名"]);
    assert_eq!(members(&d, "新名"), vec!["repo-c"]);
    let r = c.run(&["group", "remove-member", "新名", "ghost"]);
    assert!(r.code != 0 && r.all().contains("不在組"));
    c.ok(&["group", "remove", "新名"]);
    assert!(registry::load(&d).unwrap().groups.is_empty());
    let n = decisions(&d)
        .iter()
        .filter(|e| e.body.contains("（從 CLI）"))
        .count();
    assert_eq!(n, 4); // 每次改動都留痕
}

fn tool(d: &Path, name: &str, args: Value) -> Value {
    handle_message(
        d,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": name, "arguments": args}}),
        Some(true),
        None,
    )
    .unwrap()
}

/// test_mcp_group_update_and_remove 經真的 MCP 分派（ctx-core 版直接打 ops.call）；
/// 加上「非管理層的 tools/list 看不到 group_update／group_remove」。
#[test]
fn test_mcp_group_update_and_remove() {
    let (_t, d) = three();
    let r = tool(
        &d,
        "group_update",
        json!({"name": "g1", "add": ["repo-c"], "remove": ["repo-a"]}),
    );
    assert_eq!(r["result"]["isError"], false, "{r}");
    assert_eq!(members(&d, "g1"), vec!["repo-b", "repo-c"]);
    tool(&d, "group_update", json!({"name": "g1", "rename": "g2"}));
    assert_eq!(members(&d, "g2"), vec!["repo-b", "repo-c"]);
    let r = tool(&d, "group_update", json!({"name": "g2", "add": ["ghost"]}));
    assert_eq!(r["result"]["isError"], true);
    let r = tool(&d, "group_remove", json!({"name": "g2"}));
    assert_eq!(r["result"]["isError"], false);
    assert!(registry::load(&d).unwrap().groups.is_empty());
    let n = decisions(&d)
        .iter()
        .filter(|e| e.body.contains("（從 MCP）"))
        .count();
    assert_eq!(n, 3);
    let work = handle_message(
        &d,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        Some(false),
        None,
    )
    .unwrap();
    let names: Vec<&str> = work["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"group_update") && !names.contains(&"group_remove"));
    // 只在管理層
}

/// MCP 管理層的寫入工具＝操作表的全部操作（加一個操作就自動多一個工具，schema 同源）。
#[test]
fn test_ops_table_is_the_single_source() {
    let (_t, d) = three();
    let tools = tool_table(&d, Some(true));
    for o in ops::ops() {
        let t = tools.iter().find(|t| t.name == o.name).unwrap();
        assert_eq!(t.schema, o.schema());
    }
    let e = ops::call(
        &d,
        "group_remove",
        json!({"name": "g1", "force": true}),
        Via::Cli,
    )
    .unwrap_err();
    assert!(e.to_string().contains("不認得參數"));
    let e = ops::call(&d, "group_add", json!({"name": "x"}), Via::Cli).unwrap_err();
    assert!(e.to_string().contains("缺參數"));
}
