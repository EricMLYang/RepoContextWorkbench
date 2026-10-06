//! `ctx`：CLI（clap）＋MCP server（stdio）＋Claude Code hooks 入口。對應 `repo_context/cli/`、
//! `repo_context/mcpserver.py`。引擎邏輯全在 `ctx_core`，這裡只做轉接：解析參數、把結果印成
//! 跟 Python 版一樣的人話或 JSON、把錯誤轉成約定的 exit code。

pub mod cli;
pub mod mcp;
pub use ctx_core::pyjson;

pub use ctx_core::pystr::truthy;

/// 關係備註接在行尾：有備註就 `"  <note>"`，沒有就空字串。
pub fn note_tail(note: &Option<String>) -> String {
    match note {
        Some(n) if !n.is_empty() => format!("  {n}"),
        _ => String::new(),
    }
}

/// Python `str(float)`：整數值也帶 `.0`。
pub fn py_float(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 1e16 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}
