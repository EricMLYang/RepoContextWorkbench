//! 工作台資料的轉接層：IPC 命令 `get_state`／`get_scan`／`run_action` → 引擎。
//!
//! 對應 Python `webui/server.py` 的 `/api/state`、`/api/scan`、`POST /api/<action>` 與 `webui/actions.handle_action`。
//! 目前只接上了會話（PTY）相關的 action；其餘回 [`NOT_YET`]。
//!
//! TODO(第三波 ctx_core::workbench::{state,actions})：`webui/state.py`、`webui/actions.py` 搬進
//! `crates/ctx-core/src/workbench/{state,actions}.rs` 之後：
//! - `get_state` 改呼叫 `ctx_core::workbench::state::build_state(spine)`，再補上
//!   `terms`（`annotate_terms(spine, terms.list())`）與 `agents`（config `agents` 的鍵）。
//! - `get_scan` 改呼叫 `ctx_core::workbench::state::build_scan(spine, group, repos)`。
//! - `run_action` 的非會話 action 改走 `ctx_core::workbench::actions::handle_action(spine, name, payload, terms)`；
//!   `session_draft`／`session_result` 需要 `TermInfo`，從 `terms.get(sid).info()` 給。
//! - `term_create` 的 agent 會話：等 `ctx_core::session::build`（另一波在搬）組出 cwd＋argv，
//!   再交給 `TermManager::create_agent(AgentLaunch{..})`。

use std::path::PathBuf;

use ctx_core::term::TermManager;
use serde_json::{json, Value};

use crate::app_state::{AppState, MSG_NO_SPINE};

/// 還沒搬到 Rust 的功能（前端照一般錯誤顯示）。
pub const NOT_YET: &str = "尚未實作";

/// `webui/actions.py` 的 `ACTIONS` 名單（順序照抄）。
pub const ACTIONS: &[&str] = &[
    "ack",
    "relate",
    "unrelate",
    "collide",
    "collide_rerun",
    "ignore",
    "close_loop",
    "loop_defer",
    "defer",
    "tier",
    "remove_repo",
    "route_collision",
    "tag",
    "save_group",
    "update_group",
    "remove_group",
    "set_goal",
    "session_draft",
    "scan_dir",
    "register_repos",
    "brief",
    "term_create",
    "term_kill",
    "session_result",
];

const MSG_EMPTY_SCOPE: &str = "至少選擇一個 repo；空範圍不代表全部";

fn spine(st: &AppState) -> Result<PathBuf, String> {
    st.spine().ok_or_else(|| MSG_NO_SPINE.to_string())
}

/// `GET /api/state`。TODO(第三波)：見模組註解。
pub fn get_state(st: &AppState) -> Result<Value, String> {
    let _spine = spine(st)?;
    Err(NOT_YET.into())
}

/// `GET /api/scan`。`repos` 優先於 `group`；空陣列一律拒絕。TODO(第三波)：見模組註解。
pub fn get_scan(
    st: &AppState,
    _group: Option<String>,
    repos: Option<Vec<String>>,
) -> Result<Value, String> {
    if repos.as_ref().is_some_and(Vec::is_empty) {
        return Err(MSG_EMPTY_SCOPE.into());
    }
    let _spine = spine(st)?;
    Err(NOT_YET.into())
}

fn s(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// `POST /api/<name>` → `handle_action`。錯誤字串照 Python 版的 `{"error": …}`。
pub fn run_action(st: &AppState, name: &str, payload: &Value) -> Result<Value, String> {
    if payload
        .get("repos")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        return Err(MSG_EMPTY_SCOPE.into());
    }
    if !ACTIONS.contains(&name) {
        return Err(format!("未知 action: {name}"));
    }
    let terms = st.terms()?;
    let out = match name {
        "term_create" => term_create(&terms, payload)?,
        "term_kill" => term_kill(&terms, payload)?,
        // TODO(第三波 ctx_core::workbench::actions)：其餘 action
        _ => return Err(format!("{NOT_YET}：{name}")),
    };
    st.notify_changed(name);
    Ok(out)
}

/// 對應 `_a_term_create`。
fn term_create(terms: &TermManager, payload: &Value) -> Result<Value, String> {
    let kind = s(payload, "kind").unwrap_or_else(|| "shell".into());
    if kind == "agent" {
        // TODO(session 模組)：ctx_core::session::build(spine, group, repos, repo, task, agent)
        //   → (cwd, argv, _pack)，再 terms.create_agent(AgentLaunch { agent, argv, cwd, group,
        //   repos, repo, task, origin, env: vec![] })。
        return Err(format!("{NOT_YET}：agent 會話（等 session 模組搬到 Rust）"));
    }
    let cwd = s(payload, "cwd").map(PathBuf::from);
    let sess = terms
        .create_shell(cwd.as_deref(), s(payload, "origin"))
        .map_err(|e| e.to_string())?;
    Ok(json!({"ok": true, "sid": sess.sid(), "title": sess.title()}))
}

/// 對應 `_a_term_kill`。
fn term_kill(terms: &TermManager, payload: &Value) -> Result<Value, String> {
    let sid = payload.get("sid").and_then(Value::as_str).unwrap_or("");
    if !terms.kill(sid) {
        let shown = payload
            .get("sid")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| "None".into());
        return Err(format!("未知會話: {shown}"));
    }
    Ok(json!({"ok": true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::Notifier;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Rec(Mutex<Vec<String>>);
    impl Notifier for Rec {
        fn changed(&self, reason: &str) {
            self.0.lock().unwrap().push(reason.into());
        }
        fn session_exited(&self, _sid: &str, _title: &str) {}
    }

    fn state_with_spine() -> (tempfile::TempDir, AppState, Arc<Rec>) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("registry.yaml"), "repos: []\n").unwrap();
        let rec = Arc::new(Rec::default());
        let st = AppState::new(rec.clone());
        st.activate(tmp.path()).unwrap();
        (tmp, st, rec)
    }

    #[test]
    fn empty_scope_and_unknown_action_rejected() {
        let rec = Arc::new(Rec::default());
        let st = AppState::new(rec);
        assert_eq!(
            run_action(&st, "brief", &json!({"repos": []})).unwrap_err(),
            MSG_EMPTY_SCOPE
        );
        assert_eq!(
            get_scan(&st, None, Some(vec![])).unwrap_err(),
            MSG_EMPTY_SCOPE
        );
        assert_eq!(
            run_action(&st, "nope", &json!({})).unwrap_err(),
            "未知 action: nope"
        );
        assert_eq!(
            run_action(&st, "ack", &json!({})).unwrap_err(),
            MSG_NO_SPINE
        );
        assert_eq!(get_state(&st).unwrap_err(), MSG_NO_SPINE);
    }

    #[cfg(unix)]
    #[test]
    fn term_create_and_kill_shell() {
        let (_tmp, st, rec) = state_with_spine();
        let out = run_action(&st, "term_create", &json!({"kind": "shell"})).unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(out["title"], "shell · spine");
        let sid = out["sid"].as_str().unwrap().to_string();
        assert_eq!(st.terms().unwrap().list().len(), 1);
        assert_eq!(
            run_action(&st, "term_kill", &json!({"sid": sid})).unwrap(),
            json!({"ok": true})
        );
        assert_eq!(
            run_action(&st, "term_kill", &json!({"sid": "t999"})).unwrap_err(),
            "未知會話: t999"
        );
        assert!(rec.0.lock().unwrap().contains(&"term_create".to_string()));
        assert_eq!(
            run_action(&st, "ack", &json!({})).unwrap_err(),
            "尚未實作：ack"
        );
    }
}
