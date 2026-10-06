//! PTY ⇄ 前端轉接（IPC.md「終端串流」）。對應 Python `server._handle_ws`，傳輸層換成 Tauri Channel。
//!
//! 順序保證：`TermSession::attach` 在 session 鎖內先把 replay 交給訂閱者，之後的即時輸出才會送——
//! 所以同一條 Channel 上一定是「replay → live」。程序結束送 `{"t":"closed","reason":"exited"}`。
//! `term_close` 只拆那一條 attach，不殺程序（會話生命週期獨立於視窗）。

use serde::Serialize;
use serde_json::json;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;

use ctx_core::term::{AttachId, TermEvent, TermSession};

use crate::app_state::AppState;

#[derive(Debug, Serialize)]
pub struct TermOpenResult {
    pub attach_id: String,
}

fn session(st: &AppState, sid: &str) -> Result<std::sync::Arc<TermSession>, String> {
    st.terms_opt()
        .and_then(|t| t.get(sid))
        .ok_or_else(|| format!("未知會話: {sid}"))
}

fn closed(reason: &str) -> InvokeResponseBody {
    InvokeResponseBody::Json(json!({"t": "closed", "reason": reason}).to_string())
}

#[tauri::command]
pub fn term_open(
    state: State<'_, AppState>,
    sid: String,
    on_event: Channel<InvokeResponseBody>,
) -> Result<TermOpenResult, String> {
    let sess = session(&state, &sid)?;
    let id = sess.attach(Box::new(move |ev| match ev {
        TermEvent::Data([]) => true,
        TermEvent::Data(b) => on_event.send(InvokeResponseBody::Raw(b.to_vec())).is_ok(),
        TermEvent::Exited => {
            let _ = on_event.send(closed("exited"));
            false
        }
    }));
    Ok(TermOpenResult {
        attach_id: id.to_string(),
    })
}

#[tauri::command]
pub fn term_write(state: State<'_, AppState>, sid: String, data: String) -> Result<(), String> {
    session(&state, &sid)?.write(data.as_bytes());
    Ok(())
}

#[tauri::command]
pub fn term_resize(
    state: State<'_, AppState>,
    sid: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    session(&state, &sid)?.resize(cols.max(1), rows.max(1));
    Ok(())
}

#[tauri::command]
pub fn term_close(
    state: State<'_, AppState>,
    sid: String,
    attach_id: String,
) -> Result<(), String> {
    let id: AttachId = attach_id
        .parse()
        .map_err(|e: ctx_core::Error| e.to_string())?;
    session(&state, &sid)?.detach(id);
    Ok(())
}
