//! `ctx`：CLI（clap）＋MCP server（stdio）＋Claude Code hooks 入口。對應 `repo_context/cli/`、
//! `repo_context/mcpserver.py`。引擎邏輯全在 `ctx_core`，這裡只做轉接：解析參數、把結果印成
//! 跟 Python 版一樣的人話或 JSON、把錯誤轉成約定的 exit code。

pub mod cli;
pub mod mcp;
pub mod pyjson;

use serde_json::Value;

/// Python 真值判斷（`if x:`）。
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
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
