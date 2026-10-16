//! 工作台資料的轉接層：IPC 命令 `get_state`／`get_scan`／`run_action` → `ctx_core::workbench`。
//!
//! 對應 Python `webui/server.py` 的 `/api/state`、`/api/scan`、`POST /api/<action>`。業務邏輯全在引擎
//! （`ctx_core::workbench::{state,actions}`）；這裡只做：拿目前的 spine 與會話清冊、把引擎錯誤轉成
//! IPC 的 `Err(String)`（＝Python 版 `{"error": …}` 那句話）、寫入成功後發 `workbench://changed`。
//! HTTP 的 token／只綁 127.0.0.1 不搬：IPC 只在 App 自己的 WebView 裡，沒有網路面（見測試）。

use std::path::PathBuf;

use ctx_core::workbench::{self, Attention, StateScope};
use serde_json::Value;

use crate::app_state::{AppState, MSG_NO_SPINE};

const MSG_EMPTY_SCOPE: &str = "至少選擇一個 repo；空範圍不代表全部";

fn spine(st: &AppState) -> Result<PathBuf, String> {
    st.spine().ok_or_else(|| MSG_NO_SPINE.to_string())
}

fn err(e: ctx_core::Error) -> String {
    e.to_string()
}

/// `GET /api/state`：`build_state` ＋ `terms`（會話清冊＋回報）＋ `agents`。
/// `group`／`repos`＝畫面目前的範圍（引擎據此算 lane／in_scope；空陣列＝空範圍，不拒絕）。
pub fn get_state(
    st: &AppState,
    group: Option<String>,
    repos: Option<Vec<String>>,
) -> Result<Value, String> {
    let sp = spine(st)?;
    let terms = st.terms_opt().map(|t| t.list()).unwrap_or_default();
    workbench::api_state(
        &sp,
        &StateScope::new(group.as_deref(), repos.as_deref()),
        &terms,
    )
    .map_err(err)
}

/// 未讀數（選單列用）：`get_state` 的 `unread` 長度。
pub fn unread_count(state: &Value) -> usize {
    state["unread"].as_array().map_or(0, Vec::len)
}

/// 要你看的會話數（選單列用）：引擎在 `get_state` 算好的 `attention`（F-80）；舊引擎沒有這個鍵＝0。
pub fn attention_of(state: &Value) -> Attention {
    serde_json::from_value(state["attention"].clone()).unwrap_or_default()
}

/// 現在有哪些會話在「要你允許工具」（引擎 `sessions_asking_permission`）。spine 未設定＝沒有。
pub fn permission_asks_now(st: &AppState) -> Vec<Value> {
    let Some(sp) = st.spine() else {
        return Vec::new();
    };
    let terms: Vec<Value> = st
        .terms_opt()
        .map(|t| t.list().iter().map(workbench::term_value).collect())
        .unwrap_or_default();
    workbench::sessions_asking_permission(&sp, &terms)
}

/// 哪些「要你允許」還沒通知過：回新的那幾筆（同一個會話同一次請求只通知一次），
/// 並把 `seen` 換成現在這份（不再請求的會話拿掉，下次再請求才會再通知）。
pub fn fresh_permission_asks(
    seen: &mut std::collections::HashMap<String, String>,
    asks: &[Value],
) -> Vec<Value> {
    let fresh: Vec<Value> = asks
        .iter()
        .filter(|a| {
            let sid = a["sid"].as_str().unwrap_or("");
            seen.get(sid).map(String::as_str) != a["at"].as_str()
        })
        .cloned()
        .collect();
    *seen = asks
        .iter()
        .filter_map(|a| {
            Some((
                a["sid"].as_str()?.to_string(),
                a["at"].as_str()?.to_string(),
            ))
        })
        .collect();
    fresh
}

/// 視窗藏起來時（沒有 `get_state`）也要算要你看的會話數：直接問引擎（只讀階段檔）。
pub fn attention_now(st: &AppState) -> Option<Attention> {
    let sp = st.spine()?;
    let terms: Vec<Value> = st
        .terms_opt()
        .map(|t| t.list().iter().map(workbench::term_value).collect())
        .unwrap_or_default();
    workbench::session_attention(&sp, &terms).ok()
}

/// `GET /api/scan`。`repos` 優先於 `group`；空陣列一律拒絕。
pub fn get_scan(
    st: &AppState,
    group: Option<String>,
    repos: Option<Vec<String>>,
) -> Result<Value, String> {
    if repos.as_ref().is_some_and(Vec::is_empty) {
        return Err(MSG_EMPTY_SCOPE.into());
    }
    let sp = spine(st)?;
    workbench::build_scan(&sp, group.as_deref(), repos.as_deref()).map_err(err)
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
    if !workbench::ACTIONS.iter().any(|(n, _)| *n == name) {
        return Err(format!("未知 action: {name}"));
    }
    let sp = spine(st)?;
    let terms = st.terms()?;
    let out = workbench::handle_action(&sp, name, payload, Some(terms.as_ref())).map_err(err)?;
    if !workbench::is_read_only(name) {
        st.notify_changed(name);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::Notifier;
    use serde_json::json;
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
        std::env::set_var("REPOCTX_NO_BACKGROUND", "1");
        let tmp = tempfile::tempdir().unwrap();
        ctx_core::init::init_spine(tmp.path()).unwrap();
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
        assert_eq!(get_state(&st, None, None).unwrap_err(), MSG_NO_SPINE);
    }

    /// 對應 test_webui.py 的 test_inv_binds_localhost_only／test_inv_requires_token：
    /// 桌面版的工作台資料只走 Tauri IPC（同一個行程的 WebView），沒有開任何本機埠——
    /// 所以沒有「綁哪個位址」「帶不帶 token」的問題。這裡釘住：state／scan／action 都不需要網路就能用。
    #[test]
    fn ipc_has_no_network_surface() {
        let (_tmp, st, _rec) = state_with_spine();
        let s = get_state(&st, None, None).unwrap();
        assert_eq!(s["terms"], json!([]));
        assert_eq!(s["agents"], json!(["claude"]));
        assert_eq!(get_scan(&st, None, None).unwrap()["group"], "全部");
        // 引擎錯誤 → Err(那句話)
        assert_eq!(
            run_action(&st, "collide", &json!({"idea": " "})).unwrap_err(),
            "想法不可為空"
        );
    }

    #[test]
    fn permission_asks_notify_once_per_request() {
        let mut seen = std::collections::HashMap::new();
        let ask = |sid: &str, at: &str| json!({"sid": sid, "at": at, "title": sid, "text": "x"});
        let a1 = [ask("t1", "2026-10-14 01:00")];
        assert_eq!(fresh_permission_asks(&mut seen, &a1).len(), 1);
        assert!(fresh_permission_asks(&mut seen, &a1).is_empty());
        // 多一個會話請求：只通知新的
        let a2 = [ask("t1", "2026-10-14 01:00"), ask("t2", "2026-10-14 01:01")];
        let f = fresh_permission_asks(&mut seen, &a2);
        assert_eq!((f.len(), f[0]["sid"].as_str()), (1, Some("t2")));
        // t1 允許完又來一次（不同時間）：再通知
        let a3 = [ask("t1", "2026-10-14 01:05")];
        assert_eq!(fresh_permission_asks(&mut seen, &a3).len(), 1);
        // 都處理完：清空，之後同一分鐘再請求也會通知
        assert!(fresh_permission_asks(&mut seen, &[]).is_empty());
        assert_eq!(fresh_permission_asks(&mut seen, &a3).len(), 1);
    }

    #[test]
    fn attention_comes_from_the_engine_state() {
        let st = json!({"attention": {"needs_you": 2, "finished_unseen": 1}});
        assert_eq!(
            attention_of(&st),
            Attention {
                needs_you: 2,
                finished_unseen: 1,
                ..Default::default()
            }
        );
        // 舊引擎沒有這個鍵、或形狀不對：當 0，不炸
        assert_eq!(attention_of(&json!({})), Attention::default());
        assert_eq!(
            attention_of(&json!({"attention": "x"})),
            Attention::default()
        );
    }

    /// 寫入動作成功才發 changed；唯讀的不發；失敗的不發。
    #[test]
    fn write_actions_notify_changed() {
        let (_tmp, st, rec) = state_with_spine();
        run_action(&st, "ack", &json!({})).unwrap();
        run_action(&st, "collide", &json!({"idea": "x"})).unwrap();
        let _ = run_action(&st, "scan_dir", &json!({"dir": "/nonexistent-dir"}));
        let _ = run_action(&st, "tag", &json!({"id": "ghost", "add": ["x"]}));
        assert_eq!(*rec.0.lock().unwrap(), vec!["ack", "collide"]);
        let s = get_state(&st, None, None).unwrap();
        assert_eq!(unread_count(&s), s["unread"].as_array().unwrap().len());
        // 剛丟的碰撞以「處理中」卡出現（判定在背景跑；測試關掉背景）
        assert!(s["cards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["kind"] == "pending"));
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
        // state 帶會話清冊（shell 不回報）
        let s = get_state(&st, None, None).unwrap();
        assert_eq!(s["terms"][0]["sid"], sid.as_str());
        assert_eq!(s["terms"][0]["report_basis"], "shell 會話不回報");
        // session_draft 從 terms.get(sid).info() 拿資料
        let d = run_action(&st, "session_draft", &json!({"sid": sid})).unwrap();
        assert!(d["text"].as_str().unwrap().contains("會話：shell · spine"));
        assert_eq!(
            run_action(&st, "term_kill", &json!({"sid": sid})).unwrap(),
            json!({"ok": true})
        );
        assert_eq!(
            run_action(&st, "term_kill", &json!({"sid": "t999"})).unwrap_err(),
            "未知會話: t999"
        );
        assert!(rec.0.lock().unwrap().contains(&"term_create".to_string()));
    }
}
