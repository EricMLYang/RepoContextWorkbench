//! 組的維護（2026-09-25 使用者回報：「分群找不到如何新增或刪除 repo」）。對應 tests/test_groups.py。
//!
//! 以前組只能建、不能改：加減組員、改名、刪組全都沒有入口。四個入口（registry／CLI／
//! 工作台／MCP）走同一組 registry 函式；registry 以外的入口每次改動都落一筆 decision。

use std::path::{Path, PathBuf};

use ctx_core::ops::{self, Via};
use ctx_core::registry::{self, AddRepo};
use ctx_core::spine::{self, EventType};
use ctx_core::testkit::{make_git_repo, spine_with_repos};
use serde_json::json;

/// g1＝repo-a,repo-b；另登記 repo-c（不在任何組）。
fn three() -> (tempfile::TempDir, PathBuf) {
    let (tmp, d) = spine_with_repos();
    registry::add_repo(
        &d,
        "repo-c",
        make_git_repo(tmp.path(), "repo-c", 0, 0, true),
        AddRepo::default(),
    )
    .unwrap();
    (tmp, d)
}

fn members(d: &Path, name: &str) -> Vec<String> {
    registry::get_group(d, name).unwrap().members()
}

fn decisions(d: &Path) -> Vec<spine::Event> {
    spine::iter_events(d)
        .unwrap()
        .into_iter()
        .filter(|e| e.etype == EventType::Decision)
        .collect()
}

fn err<T: std::fmt::Debug>(r: ctx_core::Result<T>) -> String {
    r.unwrap_err().to_string()
}

// ── registry ────────────────────────────────────────────────────────────

#[test]
fn test_add_members_idempotent_and_validated() {
    let (_t, d) = three();
    let g = registry::add_group_members(&d, "g1", &["repo-c", "repo-a"]).unwrap();
    assert_eq!(g.members(), vec!["repo-a", "repo-b", "repo-c"]); // 已在組裡的不重複
    assert!(err(registry::add_group_members(&d, "g1", &["ghost"])).contains("未登記"));
    assert!(err(registry::add_group_members(&d, "nope", &["repo-a"])).contains("組不存在"));
    assert_eq!(members(&d, "g1"), vec!["repo-a", "repo-b", "repo-c"]); // 失敗不寫半套
}

#[test]
fn test_remove_members_keeps_registry_and_allows_empty() {
    let (_t, d) = three();
    let g = registry::remove_group_members(&d, "g1", &["repo-b"]).unwrap();
    assert_eq!(g.members(), vec!["repo-a"]);
    assert!(registry::get_repo(&d, "repo-b").is_ok()); // 只是移出組，登記還在
    assert!(err(registry::remove_group_members(&d, "g1", &["repo-c"])).contains("不在組"));
    assert!(registry::remove_group_members(&d, "g1", &["repo-a"])
        .unwrap()
        .members()
        .is_empty());
}

#[test]
fn test_set_members_replaces_and_dedupes() {
    let (_t, d) = three();
    let g = registry::set_group_members(&d, "g1", &["repo-c", "repo-a", "repo-c"]).unwrap();
    assert_eq!(g.members(), vec!["repo-c", "repo-a"]);
    assert!(err(registry::set_group_members(&d, "g1", &["repo-a", "ghost"])).contains("未登記"));
}

#[test]
fn test_rename_keeps_fields_and_moves_materials() {
    let (_t, d) = three();
    registry::set_group_field(&d, "g1", "goal", "做完 X").unwrap();
    let mat = d.join("groups/g1/materials");
    std::fs::create_dir_all(&mat).unwrap();
    std::fs::write(mat.join("note.md"), "料\n").unwrap();
    let r = registry::rename_group(&d, "g1", "產品研究").unwrap();
    assert!(r.moved_dir);
    let g = registry::get_group(&d, "產品研究").unwrap();
    assert!(g.members() == vec!["repo-a", "repo-b"] && g.goal().as_deref() == Some("做完 X"));
    assert!(d.join("groups/產品研究/materials/note.md").exists());
    assert!(!d.join("groups/g1").exists());
    assert!(err(registry::get_group(&d, "g1")).contains("組不存在"));
}

#[test]
fn test_rename_rejects_bad_names() {
    let (_t, d) = three();
    registry::add_group(&d, "g2", &["repo-c"]).unwrap();
    assert!(err(registry::rename_group(&d, "g1", "g2")).contains("組已存在"));
    for bad in ["", "  ", "a/b", "..", "a\\b"] {
        assert!(
            registry::rename_group(&d, "g1", bad).is_err(),
            "{bad:?} 應被拒"
        );
    }
    assert!(err(registry::rename_group(&d, "nope", "x")).contains("組不存在"));
}

/// 新名字的資料夾已經有東西（例如以前刪掉的同名組留下的料）→ 不合併、不覆蓋。
#[test]
fn test_rename_refuses_to_clobber_existing_dir() {
    let (_t, d) = three();
    std::fs::create_dir_all(d.join("groups/g1/briefs")).unwrap();
    std::fs::create_dir_all(d.join("groups/g9/briefs")).unwrap();
    assert!(err(registry::rename_group(&d, "g1", "g9")).contains("資料夾已存在"));
    assert_eq!(members(&d, "g1"), vec!["repo-a", "repo-b"]);
}

#[test]
fn test_remove_group_keeps_materials_and_repos() {
    let (_t, d) = three();
    std::fs::create_dir_all(d.join("groups/g1/briefs")).unwrap();
    let r = registry::remove_group(&d, "g1").unwrap();
    assert!(r.kept_dir.is_some() && d.join("groups/g1/briefs").exists());
    assert!(registry::load(&d).unwrap().groups.is_empty());
    assert_eq!(registry::load(&d).unwrap().repos.len(), 3);
    assert!(err(registry::remove_group(&d, "g1")).contains("組不存在"));
}

/// config 排程指到這個組 → 改名／刪組時提醒（config.yaml 是人寫的，不代改）。
#[test]
fn test_schedule_refs_reported() {
    let (_t, d) = three();
    std::fs::write(
        d.join("config.yaml"),
        "schedule:\n  - {task: brief, at: '09:00', group: g1}\n",
    )
    .unwrap();
    assert_eq!(
        registry::rename_group(&d, "g1", "g2")
            .unwrap()
            .schedule_refs,
        vec!["brief@09:00"]
    );
    assert!(registry::remove_group(&d, "g2")
        .unwrap()
        .schedule_refs
        .is_empty());
}

// ── CLI ─────────────────────────────────────────────────────────────────

// 未搬：test_cli_group_maintenance——等 ctx-cli（subprocess 跑 `ctx group add-member/...`，
// 驗 stdout／exit code；引擎部分已由上面 registry 測試與下面 ops 測試涵蓋）。

// ── 工作台 action ──────────────────────────────────────────────────────

fn ui(d: &Path, name: &str, payload: serde_json::Value) -> ctx_core::Result<serde_json::Value> {
    ctx_core::workbench::handle_action(d, name, &payload, None)
}

#[test]
fn test_webui_update_and_remove_group() {
    let (_t, d) = three();
    let r = ui(
        &d,
        "update_group",
        json!({"name": "g1", "repos": ["repo-c"], "new_name": "g1b"}),
    )
    .unwrap();
    assert_eq!(r["ok"], true);
    assert_eq!(r["name"], "g1b");
    assert_eq!(members(&d, "g1b"), vec!["repo-c"]);
    assert!(ui(&d, "update_group", json!({"name": "g1b", "repos": []})).is_err());
    assert!(ui(
        &d,
        "update_group",
        json!({"name": "nope", "repos": ["repo-a"]})
    )
    .is_err());
    let r = ui(&d, "remove_group", json!({"name": "g1b"})).unwrap();
    assert_eq!(r["ok"], true);
    assert!(registry::load(&d).unwrap().groups.is_empty());
    assert_eq!(
        decisions(&d)
            .iter()
            .filter(|e| e.body.contains("（從工作台）"))
            .count(),
        2
    );
}

#[test]
fn test_webui_update_group_same_name_only_members() {
    let (_t, d) = three();
    let r = ui(
        &d,
        "update_group",
        json!({"name": "g1", "repos": ["repo-a", "repo-c"], "new_name": "g1"}),
    )
    .unwrap();
    assert_eq!(r["ok"], true);
    assert_eq!(members(&d, "g1"), vec!["repo-a", "repo-c"]);
}

// ── MCP（管理層）───────────────────────────────────────────────────────

/// 原測試經 mcpserver.handle_message 呼叫；MCP server 在 ctx-cli，這裡直接打它背後的 ops.call(via=mcp)。
/// 「tools/list 非管理層看不到 group_update」那段等 ctx-cli 的 MCP server 再補。
#[test]
fn test_mcp_group_update_and_remove() {
    let (_t, d) = three();
    ops::call(
        &d,
        "group_update",
        json!({"name": "g1", "add": ["repo-c"], "remove": ["repo-a"]}),
        Via::Mcp,
    )
    .unwrap();
    assert_eq!(members(&d, "g1"), vec!["repo-b", "repo-c"]);
    ops::call(
        &d,
        "group_update",
        json!({"name": "g1", "rename": "g2"}),
        Via::Mcp,
    )
    .unwrap();
    assert_eq!(members(&d, "g2"), vec!["repo-b", "repo-c"]);
    assert!(ops::call(
        &d,
        "group_update",
        json!({"name": "g2", "add": ["ghost"]}),
        Via::Mcp
    )
    .is_err());
    ops::call(&d, "group_remove", json!({"name": "g2"}), Via::Mcp).unwrap();
    assert!(registry::load(&d).unwrap().groups.is_empty());
    assert_eq!(
        decisions(&d)
            .iter()
            .filter(|e| e.body.contains("（從 MCP）"))
            .count(),
        3
    );
    // MCP 留痕 source＝agent（要讓使用者看得到）
    assert!(decisions(&d).iter().all(|e| e.source == "agent"));
}

/// 改名失敗（撞名）→ 組員也不能被改掉。
#[test]
fn test_webui_update_group_no_half_apply() {
    let (_t, d) = three();
    registry::add_group(&d, "g2", &["repo-c"]).unwrap();
    let e = ui(
        &d,
        "update_group",
        json!({"name": "g1", "repos": ["repo-c"], "new_name": "g2"}),
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("組已存在"), "{e}");
    assert_eq!(members(&d, "g1"), vec!["repo-a", "repo-b"]);
}

/// 自己（CLI／工作台）改組只留痕不進收件匣；agent 經 MCP 改的要讓使用者看得到。
#[test]
fn test_own_edits_stay_out_of_inbox_agent_edits_surface() {
    use ctx_core::workbench::{build_state, StateScope};
    let (_t, d) = three();
    ops::call(
        &d,
        "group_update",
        json!({"name": "g1", "add": ["repo-c"]}),
        Via::Cli,
    )
    .unwrap();
    ops::call(
        &d,
        "group_update",
        json!({"name": "g1", "remove": ["repo-c"]}),
        Via::Ui,
    )
    .unwrap();
    let st = build_state(&d, &StateScope::default()).unwrap();
    let text = |c: &serde_json::Value| {
        let mut parts: Vec<String> = c["lines"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        parts.push(c["title"].as_str().unwrap_or("").to_string());
        parts.join(" ")
    };
    assert!(!st["cards"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| text(c).contains("編輯組")));
    ops::call(
        &d,
        "group_update",
        json!({"name": "g1", "rename": "g9"}),
        Via::Mcp,
    )
    .unwrap();
    let st = build_state(&d, &StateScope::default()).unwrap();
    assert!(st["cards"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c.to_string().contains("改名 g1 → g9")));
}

/// MCP 管理層的寫入工具＝操作表的全部操作（schema 同源）。tool_table 在 ctx-cli 的 MCP server，
/// 這裡先驗 schema 形狀與錯誤訊息；「tools[name] == o.schema()」等 MCP server 搬過來再補。
#[test]
fn test_ops_table_is_the_single_source() {
    let (_t, d) = three();
    let names: Vec<&str> = ops::ops().iter().map(|o| o.name).collect();
    assert_eq!(
        names,
        vec![
            "registry_add",
            "registry_remove",
            "registry_set",
            "registry_tier",
            "registry_tag",
            "registry_relate",
            "registry_unrelate",
            "registry_export",
            "group_add",
            "group_update",
            "group_remove",
            "group_goal"
        ]
    );
    assert_eq!(
        ops::get("group_add").unwrap().schema(),
        json!({"type": "object",
               "properties": {"name": {"type": "string", "description": "組名"},
                              "members": {"type": "array", "items": {"type": "string"},
                                          "description": "repo id 清單"}},
               "required": ["name", "members"]})
    );
    let e = ops::call(
        &d,
        "group_remove",
        json!({"name": "g1", "force": true}),
        Via::Cli,
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("不認得參數"), "{e}");
    let e = ops::call(&d, "group_add", json!({"name": "x"}), Via::Cli)
        .unwrap_err()
        .to_string();
    assert!(e.contains("缺參數"), "{e}");
}

/// 改設定（建組、改組、改目標）不是工作進度：摘要的「上次進度」不拿它充數。
#[test]
fn test_admin_edits_are_not_progress() {
    let (_t, d) = three();
    ops::call(
        &d,
        "group_update",
        json!({"name": "g1", "add": ["repo-c"]}),
        Via::Ui,
    )
    .unwrap();
    ops::call(
        &d,
        "group_goal",
        json!({"name": "g1", "text": "做完 X"}),
        Via::Cli,
    )
    .unwrap();
    let s = ctx_core::summary::build_summary(&d, Some("g1"), None, None, None, None).unwrap();
    assert!(s.progress.is_none());
    assert!(s.progress_gap.unwrap().contains("沒有本組的進度紀錄"));
    spine::append_event(
        &d,
        "decision",
        "monitor",
        &["group:g1"],
        "會話結果：做完第一步",
        None,
    )
    .unwrap();
    let s = ctx_core::summary::build_summary(&d, Some("g1"), None, None, None, None).unwrap();
    assert!(s.progress.unwrap().text.contains("會話結果"));
}

// ADMIN_TAG 前綴（Python 沒有獨立測試）：
#[test]
fn admin_edits_carry_admin_tag() {
    let (_t, d) = three();
    let (_, say) = ops::call(
        &d,
        "group_goal",
        json!({"name": "g1", "text": "做完 X"}),
        Via::Cli,
    )
    .unwrap();
    assert_eq!(say, "目標已記在 g1");
    let ev = decisions(&d).pop().unwrap();
    assert_eq!(
        ev.body,
        format!("{}本組目標：做完 X（從 CLI）", spine::ADMIN_TAG)
    );
    assert_eq!(ev.source, "cli");
    assert_eq!(ev.tokens, vec!["group:g1"]);
}
