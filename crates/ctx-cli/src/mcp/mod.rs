//! MCP server——兩層工具（2026-09-25 Agent 友善輪）。對應 `repo_context/mcpserver.py`。
//!
//! **工作層（預設）**：照 agent 一次工作的節奏切——where_am_i／context_for／next_work／
//! log_decision／add_todo／close_todo／report_status／handoff／search_knowledge，
//! 加上跨 repo 參考輪的 read_from／ask_repo／repo_status。
//! 依 server 的 cwd（或參數 cwd）推出主場範圍；回傳 JSON（帶可引用的 ref／id）；
//! 錯誤是結構化的 {ok:false, error, message, hint}；寫入主場以外會被拒（cross_scope）。
//! 邏輯全在 `ctx_core::agentapi`，CLI `--json` 與 hooks 同源。
//!
//! **管理層（`ctx mcp --admin` 或 config `mcp.admin: true` 才開）**：registry／組／採集／
//! 打包／碰撞／升格等維護工具——給人（或人明確授權的 agent）用，預設不塞給 agent。
//!
//! stdio 傳輸（newline-delimited JSON-RPC 2.0），只用 serde_json 手寫、零額外依賴（耗材原則）。
//! 拒寫去向（P10 檢討④）：validator 錯誤**原樣**回給 agent（isError tool result）供自我修正；
//! dead-letter 是人的入口（hotkey）專屬，MCP 不走。

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use chrono::Local;
use ctx_core::agentapi::{self, SearchOpts, Target};
use ctx_core::collect::{self, RepoSpec};
use ctx_core::ops::{self, Via};
use ctx_core::pack::{self, RepoRef};
use ctx_core::{
    brief, collide, config, context, crossref, digest, notify, pystr, registry, route, spawn,
    spine, upstream, Error, Result,
};
use serde_json::{json, Map, Value};
use std::sync::LazyLock;

use crate::cli::common::pct0;
use crate::{pyjson, truthy};

pub const PROTOCOL_VERSION: &str = "2024-11-05";

type Args = Map<String, Value>;

mod admin;
mod args;
mod schema;
mod tools;
mod work;

use admin::*;
use args::*;
use schema::*;
pub use tools::{ADMIN_TOOLS, WORK_TOOLS};
use work::*;

// ---------------------------------------------------------------- 工作層

type WorkFn = fn(&Path, &Args, &Path) -> Result<Value>;
type AdminFn = Box<dyn Fn(&Path, &Args) -> Result<String> + Send + Sync>;

/// 一個工具：名稱、說明、inputSchema、實作。
pub struct Tool {
    pub name: String,
    pub description: String,
    pub schema: Value,
    kind: ToolKind,
}

enum ToolKind {
    Work(WorkFn),
    Admin(AdminFn),
}

/// 對應 `_admin_enabled`：admin=None＝看 config `mcp.admin`。
fn admin_enabled(spine_dir: &Path, admin: Option<bool>) -> bool {
    if let Some(a) = admin {
        return a;
    }
    config::load(spine_dir)
        .ok()
        .and_then(|c| c.get("mcp.admin").map(truthy))
        .unwrap_or(false)
}

/// 對應 `tool_table`：工作層＋（管理層開時）管理層。
pub fn tool_table(spine_dir: &Path, admin: Option<bool>) -> Vec<&'static Tool> {
    let mut out: Vec<&'static Tool> = WORK_TOOLS.iter().collect();
    if admin_enabled(spine_dir, admin) {
        out.extend(ADMIN_TOOLS.iter());
    }
    out
}

fn text_of(v: &Value) -> String {
    pyjson::dumps_indent(v)
}

/// 工作層錯誤 → `{ok:false,error,message,hint?}`：AgentError（Rejected）照原 code；
/// 其他例外（Python 的 `except Exception`）一律 `failed`。
fn work_error(e: &Error) -> Value {
    match e {
        Error::Rejected { .. } => agentapi::error_dict(e),
        other => json!({"ok": false, "error": "failed", "message": other.to_string()}),
    }
}

/// 對應 `handle_message`：單則 JSON-RPC → 回應（notification 回 None）。
/// admin=None＝看 config `mcp.admin`；cwd＝推主場用的目錄（預設 `$CTX_CWD` 或 server 行程的 cwd）。
pub fn handle_message(
    spine_dir: &Path,
    msg: &Value,
    admin: Option<bool>,
    cwd: Option<&Path>,
) -> Option<Value> {
    let mid = msg.get("id").cloned().unwrap_or(Value::Null);
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    if method.starts_with("notifications/") {
        return None;
    }
    let ok = |result: Value| json!({"jsonrpc": "2.0", "id": mid, "result": result});
    if method == "initialize" {
        return Some(ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "repo_context", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "repo-context：跨 repo 的工作脈絡與留痕。開工先 context_for，\
                做了判斷 log_decision、發現待辦 add_todo、等人或卡住 report_status，\
                收尾一定 handoff。範圍依 cwd 推出，寫到別組會被拒，要先問使用者。",
        })));
    }
    if method == "ping" {
        return Some(ok(json!({})));
    }
    let table = tool_table(spine_dir, admin);
    if method == "tools/list" {
        let tools: Vec<Value> = table
            .iter()
            .map(|t| json!({"name": t.name, "description": t.description, "inputSchema": t.schema}))
            .collect();
        return Some(ok(json!({ "tools": tools })));
    }
    if method == "tools/call" {
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let args: Args = match params.get("arguments") {
            Some(Value::Object(m)) => m.clone(),
            _ => Map::new(),
        };
        let Some(tool) = table.iter().find(|t| t.name == name) else {
            let shown = params
                .get("name")
                .map(pystr::py_str)
                .unwrap_or_else(|| "None".into());
            let hint = if ADMIN_TOOLS.iter().any(|t| t.name == name) {
                "（這是管理層工具，需用 `ctx mcp --admin` 啟動）"
            } else {
                ""
            };
            return Some(json!({"jsonrpc": "2.0", "id": mid,
                "error": {"code": -32602, "message": format!("未知 tool: {shown}{hint}")}}));
        };
        let here: PathBuf = match cwd {
            Some(c) => c.to_path_buf(),
            None => std::env::var_os("CTX_CWD")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from(".")),
        };
        let started = std::time::Instant::now();
        let (text, is_error) = match &tool.kind {
            ToolKind::Work(f) => match f(spine_dir, &args, &here) {
                Ok(v) => (text_of(&v), false),
                Err(e) => (text_of(&work_error(&e)), true),
            },
            // 錯誤原樣回 agent 自我修正（含 validator 拒寫）
            ToolKind::Admin(f) => match f(spine_dir, &args) {
                Ok(s) => (s, false),
                Err(e) => (e.to_string(), true),
            },
        };
        // Rust 才有（N-07）：每次呼叫記一行（ctx health 算工具用量與延遲）
        ctx_core::health::record_call(spine_dir, name, started.elapsed().as_millis(), !is_error);
        return Some(ok(json!({
            "content": [{"type": "text", "text": text}],
            "isError": is_error,
        })));
    }
    Some(json!({"jsonrpc": "2.0", "id": mid,
        "error": {"code": -32601, "message": format!("未知 method: {method}")}}))
}

/// 對應 `serve`：stdio loop，一行一則 JSON-RPC。
pub fn serve(spine_dir: &Path, admin: Option<bool>) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if !msg.is_object() {
            continue;
        }
        if let Some(resp) = handle_message(spine_dir, &msg, admin, None) {
            let mut out = stdout.lock();
            writeln!(out, "{}", pyjson::dumps(&resp))?;
            out.flush()?;
        }
    }
    Ok(())
}
