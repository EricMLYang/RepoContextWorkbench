//! 工作台（桌面 App）的畫面資料與行動按鈕。對應 Python `repo_context/webui/`（state／actions）。
//!
//! 三個一級物件：牌（組）／收件匣（統一卡片）／會話（terminal）。殼原則（v2 §1）不變：
//! - `get_state`（[`state::api_state`]）輕量——純脊椎／registry 檔案讀，可以常常叫
//! - `get_scan`（[`state::build_scan`]）才跑 P3 採集（會打 git），開頁／切組／勾選／手動才跑
//! - 所有行動按鈕（[`actions::handle_action`]）都只是呼叫引擎原語再落脊椎
//!
//! Python 版的 HTTP server／WebSocket（`webui/server.py`、`ws.py`）不搬：桌面版走 Tauri IPC
//! （`desktop/IPC.md`）。server.py 裡的業務邏輯（`/api/state` 補 `terms`／`agents`）搬到
//! [`state::api_state`]；token 驗證屬於 HTTP 傳輸層，不搬。
//!
//! 與 Python 版刻意的差異（2026-10-06，AGENTS.md「前端不放業務邏輯」）：原本前端自己算的
//! 範圍篩選（卡片／事件／關係屬不屬於目前範圍）、關係任務文案、會話起手任務，改由這裡多帶欄位
//! （`scope`、`lane`、`in_scope`、`task`、`session_tasks`、問句卡的 provenance）。Python 版同步加了
//! 同樣的欄位，兩邊 JSON 仍一致。

pub mod actions;
pub mod state;

pub use actions::{handle_action, Terms, ACTIONS};
pub use state::{
    annotate_terms, api_state, build_cards, build_scan, build_state, event_scopes, normalize_dest,
    relation_task, session_draft, session_report, Prov, StateScope, STATUS_PROMPT,
};
