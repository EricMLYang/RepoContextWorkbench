//! MCP server：JSON-RPC 分派、tools 清單、拒寫去向（validator 錯誤原樣回 agent）、原語往返。
//! 對應 tests/test_mcpserver.py（在行程內呼叫 handle_message）。

use std::collections::HashSet;
use std::path::Path;

use ctx_cli::mcp::handle_message;
use ctx_core::spine::{self as spine_mod, EventType};
use ctx_core::testkit::{make_git_repo, spine, spine_with_repos};
use serde_json::{json, Value};

/// 這支檔測原語往返，預設開管理層；工作層與分層規則見 tests/agentapi.rs。
fn call(d: &Path, method: &str, params: Value) -> Value {
    handle_message(
        d,
        &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
        Some(true),
        None,
    )
    .unwrap()
}

fn tool(d: &Path, name: &str, args: Value) -> Value {
    call(d, "tools/call", json!({"name": name, "arguments": args}))
}

fn text(r: &Value) -> String {
    r["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn test_initialize_and_tools_list() {
    let (_t, d) = spine();
    let r = call(&d, "initialize", json!({}));
    assert_eq!(r["result"]["serverInfo"]["name"], "repo_context");
    let r = call(&d, "tools/list", json!({}));
    let tools = r["result"]["tools"].as_array().unwrap();
    let names: HashSet<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    // 原語全覆蓋：P1–P13、P15/P16 相關工具都在
    for n in [
        "registry_add",
        "registry_audit",
        "registry_scan",
        "group_add",
        "collect",
        "pack",
        "pack_estimate",
        "brief",
        "spine_append",
        "spine_query",
        "spine_stats",
        "spine_lint",
        "open_loops",
        "unread",
        "collide_submit",
        "route",
        "upstream_check",
        "digest_run",
        "spawn",
    ] {
        assert!(names.contains(n), "缺 {n}");
    }
    assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
}

/// 拆機報告落地輪三工具：掃描（mani 課）、lint（second-brain 課）、
/// 預算函數（Repomix 課）——agent 的介面＝人的介面。
#[test]
fn test_scan_lint_estimate_tools() {
    let (t, d) = spine_with_repos();
    let zone = t.path().join("zone");
    make_git_repo(&zone, "found-me", 0, 0, true);
    let txt = text(&tool(&d, "registry_scan", json!({"dir": zone})));
    assert!(txt.contains("found-me") && txt.contains("候選")); // 預設只列不登記
    let txt = text(&tool(
        &d,
        "registry_scan",
        json!({"dir": zone, "apply": true}),
    ));
    assert!(txt.contains("已登記"));
    let txt = text(&tool(&d, "spine_lint", json!({})));
    assert!(txt.contains("零紅字"));
    let txt = text(&tool(&d, "pack_estimate", json!({"group": "g1"})));
    assert!(txt.contains("repo-a") && txt.contains("tokens") && txt.contains("合計"));
}

#[test]
fn test_notifications_ignored_and_unknowns() {
    let (_t, d) = spine();
    assert!(handle_message(
        &d,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        None,
        None
    )
    .is_none());
    assert_eq!(call(&d, "no/such", json!({}))["error"]["code"], -32601);
    assert_eq!(tool(&d, "no_such_tool", json!({}))["error"]["code"], -32602);
}

#[test]
fn test_validator_error_passthrough_no_dead_letter() {
    let (_t, d) = spine();
    let r = tool(
        &d,
        "spine_append",
        json!({"type": "nope", "source": "agent"}),
    );
    assert_eq!(r["result"]["isError"], true);
    assert!(text(&r).contains("未知事件型別"));
    // MCP 是程式入口：不落 dead-letter（那是人的入口專屬）
    assert!(!d.join("spine").join("dead-letter").exists());
}

#[test]
fn test_append_query_roundtrip_no_default_due() {
    let (_t, d) = spine();
    let r = tool(
        &d,
        "spine_append",
        json!({"type": "decision", "source": "agent", "body": "決定 X"}),
    );
    assert_eq!(r["result"]["isError"], false);
    let r = tool(
        &d,
        "spine_append",
        json!({"type": "open-loop", "source": "agent", "kv": ["#1"], "body": "opened → 驗證 Y"}),
    );
    // 沒帶 due 就沒有期限（刻意差異：不再補預設天數，2026-11-10）
    assert!(!text(&r).contains("due:"), "{}", text(&r));
    let q = text(&tool(&d, "spine_query", json!({"today": true})));
    assert!(q.contains("decision") && q.contains("open-loop"));
    let loops = text(&tool(&d, "open_loops", json!({})));
    assert!(loops.contains("#1"));
}

#[test]
fn test_collide_submit_wait_writes_two_events() {
    let (_t, d) = spine_with_repos();
    let r = tool(
        &d,
        "collide_submit",
        json!({"idea": "已知的快取想法", "group": "g1", "wait": true}),
    );
    assert_eq!(r["result"]["isError"], false);
    assert!(text(&r).contains("判定"));
    let cols: Vec<_> = spine_mod::iter_events(&d)
        .unwrap()
        .into_iter()
        .filter(|e| e.etype == EventType::Collision)
        .collect();
    assert_eq!(cols.len(), 2); // opened ＋ 判定結果
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let want = format!("{today}-a");
    assert!(cols.iter().any(|e| e.kv("id") == Some(want.as_str())));
}

#[test]
fn test_stats_includes_survival() {
    let (_t, d) = spine();
    let txt = text(&tool(&d, "spine_stats", json!({})));
    assert!(txt.contains("靈感命中率") && txt.contains("存活率"));
}

/// 2026-09-08 組為單位輪：agent 也能設關係、讀關係、隨時重抓組情境卡。
#[test]
fn test_relation_and_context_tools() {
    let (_t, d) = spine_with_repos();
    let names: HashSet<String> = call(&d, "tools/list", json!({}))["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    for n in ["registry_relate", "registry_relations", "group_context"] {
        assert!(names.contains(n));
    }
    let txt = text(&tool(
        &d,
        "registry_relate",
        json!({"a": "repo-a", "b": "repo-b", "kind": "pm-of"}),
    ));
    assert!(txt.contains("pm-of"));
    let txt = text(&tool(&d, "registry_relations", json!({"id": "repo-a"})));
    assert!(txt.contains("repo-b") && txt.contains("規劃（PM）"));
    let r = tool(
        &d,
        "registry_relate",
        json!({"a": "repo-a", "b": "repo-a", "kind": "pm-of"}),
    );
    assert_eq!(r["result"]["isError"], true);
    assert!(text(&r).contains("自己"));
    let txt = text(&tool(&d, "group_context", json!({"group": "g1"})));
    assert!(txt.starts_with("# 組情境卡 g1") && txt.contains("## 脈動"));
}

/// F-123：handoff 的 ask 參數——物件、包成 JSON 字串的物件都認；驗不過回 bad_request 且不寫；schema 有 kind 的 enum。
#[test]
fn test_handoff_ask_param() {
    let (tmp, d) = spine_with_repos();
    let a = tmp.path().join("repo-a");
    let cwd = a.to_string_lossy().to_string();
    let r = call(&d, "tools/list", json!({}));
    let ho = r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "handoff")
        .unwrap()
        .clone();
    let ask = &ho["inputSchema"]["properties"]["ask"];
    assert_eq!(ask["type"], "object");
    assert_eq!(ask["required"], json!(["question"]));
    assert_eq!(ask["properties"]["kind"]["enum"][0], "commit");
    assert!(ask["description"].as_str().unwrap().contains("要繼續嗎"));

    let ok = tool(
        &d,
        "handoff",
        json!({"cwd": cwd, "done": "草稿", "next_step": "等使用者挑",
               "ask": {"question": "用 A 還是 B？", "kind": "choose", "options": ["A", "B"],
                       "recommend": 2, "reversible": false, "evidence": ["README.md"]}}),
    );
    let v: Value = serde_json::from_str(&text(&ok)).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["ask"]["recommend"], "B");
    assert_eq!(v["ask"]["reversible"], false);
    let ev = spine_mod::iter_events(&d)
        .unwrap()
        .into_iter()
        .rev()
        .find(|e| e.etype == EventType::Decision)
        .unwrap();
    assert!(ev.body.contains("\n要你決定：用 A 還是 B？\n類別：choose\n選項：A｜B\n建議：B\n可逆：否\n證據：README.md"), "{}", ev.body);

    let s = tool(
        &d,
        "handoff",
        json!({"cwd": cwd, "done": "草稿", "next_step": "等使用者挑",
               "ask": "{\"question\": \"要 commit 嗎？\", \"kind\": \"commit\"}"}),
    );
    let v: Value = serde_json::from_str(&text(&s)).unwrap();
    assert_eq!(v["ask"]["kind"], "commit", "{v}");

    let before = spine_mod::iter_events(&d).unwrap().len();
    let bad = tool(
        &d,
        "handoff",
        json!({"cwd": cwd, "done": "草稿", "next_step": "等使用者挑",
               "ask": {"question": "哪個？", "options": ["A", "B"], "recommend": "C"}}),
    );
    let v: Value = serde_json::from_str(&text(&bad)).unwrap();
    assert_eq!(v["error"], "bad_request", "{v}");
    assert!(v["hint"].as_str().unwrap().contains("1) A"));
    assert_eq!(spine_mod::iter_events(&d).unwrap().len(), before);

    // 沒帶 ask：回傳沒有 ask 鍵
    let plain = tool(
        &d,
        "handoff",
        json!({"cwd": cwd, "done": "x", "next_step": "y"}),
    );
    let v: Value = serde_json::from_str(&text(&plain)).unwrap();
    assert!(v.get("ask").is_none(), "{v}");
}

/// F-124：request_decision——沒規則回 wait（附原因）、trust 後回 auto（附答案與 ref）；參數跟 ask 同一組。
#[test]
fn test_request_decision_tool() {
    let (tmp, d) = spine_with_repos();
    let cwd = tmp.path().join("repo-a").to_string_lossy().to_string();
    let r = call(&d, "tools/list", json!({}));
    let t = r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "request_decision")
        .unwrap()
        .clone();
    assert_eq!(t["inputSchema"]["required"], json!(["question"]));
    assert_eq!(t["inputSchema"]["properties"]["kind"]["enum"][0], "commit");
    let args = json!({"cwd": cwd, "question": "要 commit 嗎？", "kind": "commit",
                      "options": ["要", "不要"], "recommend": 1});
    let v: Value =
        serde_json::from_str(&text(&tool(&d, "request_decision", args.clone()))).unwrap();
    assert_eq!(v["decision"], "wait", "{v}");
    assert_eq!(v["reason"]["code"], "no_rule");
    ctx_core::decide::trust(&d, "repo-a", "commit", None).unwrap();
    let v: Value = serde_json::from_str(&text(&tool(&d, "request_decision", args))).unwrap();
    assert_eq!(v["decision"], "auto", "{v}");
    assert_eq!(v["answer"], "要");
    assert_eq!(v["rule"], "repo-a/commit");
    assert!(v["ref"].as_str().unwrap().starts_with("chosen:"));
}

/// F-126：search_knowledge 預設精簡（answer_repo、refs、next，≤ 1500 字）、detail=full 拿完整；repo_entry 入口卡。
#[test]
fn test_search_brief_default_and_repo_entry() {
    let (tmp, d) = spine_with_repos();
    let cwd = tmp.path().join("repo-a").to_string_lossy().to_string();
    let v: Value = serde_json::from_str(&text(&tool(
        &d,
        "search_knowledge",
        json!({"cwd": cwd, "query": "快取 LRU"}),
    )))
    .unwrap();
    assert!(
        v.get("answer_repo").is_some() && v.get("files").is_none(),
        "{v}"
    );
    assert!(serde_json::to_string(&v).unwrap().chars().count() <= 1500);
    assert!(v["next"].as_str().unwrap().contains("repo_entry"));
    let f: Value = serde_json::from_str(&text(&tool(
        &d,
        "search_knowledge",
        json!({"cwd": cwd, "query": "快取 LRU", "detail": "full"}),
    )))
    .unwrap();
    assert!(
        f.get("files").is_some() && f.get("answer_repo").is_none(),
        "{f}"
    );
    let e: Value =
        serde_json::from_str(&text(&tool(&d, "repo_entry", json!({"repo": "repo-a"})))).unwrap();
    assert_eq!(e["repo"], "repo-a");
    assert!(e["read_first"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["path"] == "README.md"));
    let missing: Value =
        serde_json::from_str(&text(&tool(&d, "repo_entry", json!({"repo": "nope"})))).unwrap();
    assert_eq!(missing["error"], "not_found");
}
