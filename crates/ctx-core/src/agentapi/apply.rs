//! Rust 才有（2026-10-15，F-63）：MCP `workbench_apply`——agent 改工作台設定（建組、改組員、改名、目標、標籤、分層）。
//! 跟 App 助手同一份白名單與驗證（[`crate::assist::validate`]），執行走 App 按鈕用的同一組 action。
//!
//! - 預設只回預覽（每步的人話、錯誤、危險）；agent 給使用者看過、同意後帶 `confirm=true` 才執行。
//! - 畫面動作（換範圍、切分頁、開 repo、開會話對話框）只在 App 做，這裡標為略過。
//! - 碰到主場（所在 repo＋同組成員）以外的 repo，要使用者同意後帶 `cross_scope_ok=true`。
//! - 依序執行，失敗就停；落一筆 presented 事件（source＝呼叫的人：MCP 是 agent、CLI `ctx assist` 是 cli）。

use super::*;
use crate::assist::{self, Step};

/// 只在 App 做的畫面動作。
pub const UI_ONLY: &[&str] = &["switch_scope", "open_tab", "open_repo", "open_session"];

/// 一步碰到哪些 repo（含牌組現在與之後的成員）。
fn touched(data: &registry::RegistryData, s: &Step) -> Vec<String> {
    let a = &s.args;
    let mut ids: Vec<String> = Vec::new();
    if let Some(id) = a.get("id").and_then(Value::as_str) {
        ids.push(id.to_string());
    }
    if let Some(list) = a.get("repos").and_then(Value::as_array) {
        ids.extend(list.iter().filter_map(Value::as_str).map(str::to_string));
    }
    if let Some(g) = a.get("group").and_then(Value::as_str) {
        if let Some(g) = data.group(g) {
            ids.extend(g.members());
        }
    }
    ids
}

/// 驗證過的一步 → App 按鈕用的 action（名稱、payload）。畫面動作回 None。
fn action_of(data: &registry::RegistryData, s: &Step) -> Option<(&'static str, Value)> {
    let a = &s.args;
    let g = |k: &str| a.get(k).cloned().unwrap_or(Value::Null);
    Some(match s.op.as_str() {
        "create_group" => (
            "save_group",
            json!({"name": g("name"), "repos": g("repos")}),
        ),
        "set_members" => (
            "update_group",
            json!({"name": g("group"), "repos": g("repos")}),
        ),
        "rename_group" => {
            let name = a.get("group").and_then(Value::as_str).unwrap_or("");
            let members = data.group(name).map(|x| x.members()).unwrap_or_default();
            (
                "update_group",
                json!({"name": name, "repos": members, "new_name": g("new_name")}),
            )
        }
        "set_goal" => ("set_goal", json!({"group": g("group"), "text": g("text")})),
        "tag" => (
            "tag",
            json!({"id": g("id"), "add": g("add"), "remove": g("remove")}),
        ),
        "set_tier" => (
            "tier",
            json!({"id": g("id"), "tier": g("tier"), "resume_when": g("resume_when")}),
        ),
        _ => return None,
    })
}

/// MCP `workbench_apply`：`steps` 同 App 助手的計畫格式（`[{op, args, 理由?}]`）。
pub fn workbench_apply(
    spine_dir: &Path,
    steps: &Value,
    confirm: bool,
    cwd: Option<&Path>,
    cross_scope_ok: bool,
    source: &str,
) -> Result<Value> {
    let data = registry::load(spine_dir)?;
    let mut plan = assist::validate(&data, &json!({"steps": steps}), "mcp")
        .map_err(|e| agent_error(crate::RejectCode::BadRequest, e.to_string(), None))?;
    let w = where_am_i(spine_dir, cwd)?;
    let home = if cross_scope_ok {
        None
    } else {
        home_ids(spine_dir, &w)?
    };
    for s in &mut plan.steps {
        if s.error.is_some() {
            continue;
        }
        if UI_ONLY.contains(&s.op.as_str()) {
            s.error = Some("畫面動作只在 App 做（請使用者在 App 的 ⌘K 打「?」）".into());
            continue;
        }
        if let Some(home) = &home {
            let out: Vec<String> = touched(&data, s)
                .into_iter()
                .filter(|id| !home.contains(id))
                .collect();
            if !out.is_empty() {
                s.error = Some(format!(
                    "碰到主場以外的 repo：{}（使用者同意後帶 cross_scope_ok=true）",
                    out.join("、")
                ));
            }
        }
    }
    let view = |s: &Step| json!({"op": s.op, "text": s.text, "danger": s.danger, "error": s.error});
    if !confirm {
        return Ok(json!({
            "ok": true,
            "preview": true,
            "steps": plan.steps.iter().map(view).collect::<Vec<_>>(),
            "hint": "先把每一步給使用者看（error 的不會做、danger 會移掉東西）；使用者同意後帶 confirm=true 再呼叫一次。",
        }));
    }
    let mut done: Vec<String> = Vec::new();
    let mut failed: Option<String> = None;
    for s in plan.steps.iter().filter(|s| s.error.is_none()) {
        let Some((name, payload)) = action_of(&data, s) else {
            continue;
        };
        match crate::workbench::actions::handle_action(spine_dir, name, &payload, None) {
            Ok(_) => done.push(s.text.clone()),
            Err(e) => {
                failed = Some(format!("{}：{e}", s.text));
                break;
            }
        }
    }
    let skipped: Vec<String> = plan
        .steps
        .iter()
        .filter(|s| !done.contains(&s.text))
        .filter(|s| failed.as_deref().is_none_or(|f| !f.starts_with(&s.text)))
        .map(|s| s.text.clone())
        .collect();
    let mut body = vec![format!("助手（{source}）：改工作台設定")];
    if !done.is_empty() {
        body.push(format!("做了：{}", done.join("；")));
    }
    if !skipped.is_empty() {
        body.push(format!("略過：{}", skipped.join("；")));
    }
    if let Some(f) = &failed {
        body.push(format!("失敗停下：{f}"));
    }
    let toks: Vec<String> = w
        .repo_id()
        .map(|r| vec![format!("repo:{r}")])
        .unwrap_or_default();
    spine::append_event(
        spine_dir,
        "presented",
        source,
        &toks,
        &body.join("\n"),
        None,
    )?;
    Ok(json!({"ok": failed.is_none(), "done": done, "skipped": skipped, "failed": failed}))
}
