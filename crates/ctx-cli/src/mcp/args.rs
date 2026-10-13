//! 參數小工具：從 JSON-RPC 的 arguments 取值，與共用的範圍解析。

use super::*;

// ---------------------------------------------------------------- 參數小工具

/// 字串參數（非字串以 Python `str()` 轉；null／缺＝None）。
pub(super) fn arg_s(a: &Args, k: &str) -> Option<String> {
    match a.get(k) {
        None | Some(Value::Null) => None,
        Some(v) => Some(pystr::py_str(v)),
    }
}

/// `a.get(k)` 的真值。
pub(super) fn arg_flag(a: &Args, k: &str) -> bool {
    a.get(k).is_some_and(truthy)
}

/// 字串清單參數（陣列；單一字串視為一項）。
pub(super) fn arg_list(a: &Args, k: &str) -> Vec<String> {
    match a.get(k) {
        Some(Value::Array(xs)) => xs.iter().map(pystr::py_str).collect(),
        Some(Value::Null) | None => Vec::new(),
        Some(v) => vec![pystr::py_str(v)],
    }
}

/// `repos` 參數：逗號字串或陣列。空＝None。
pub(super) fn arg_repos(a: &Args) -> Option<Vec<String>> {
    match a.get("repos") {
        None | Some(Value::Null) => None,
        Some(Value::Array(xs)) if xs.is_empty() => None,
        Some(Value::Array(xs)) => Some(xs.iter().map(pystr::py_str).collect()),
        Some(Value::String(s)) if s.is_empty() => None,
        Some(v) => Some(registry::split_ids(&pystr::py_str(v))),
    }
}

/// 整數參數（數字或數字字串）。
pub(super) fn arg_int(a: &Args, k: &str) -> Option<i64> {
    match a.get(k)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub(super) fn ev_lines(events: &[spine::Event], verbose: bool) -> String {
    let mut out = Vec::new();
    for ev in events {
        out.push(format!("{} {}", ev.date, ev.header()));
        if verbose && !ev.body.is_empty() {
            out.push(format!("   {}", ev.body.replace('\n', "\n   ")));
        }
    }
    if out.is_empty() {
        "（無）".into()
    } else {
        out.join("\n")
    }
}

pub(super) fn scope_ids(a: &Args) -> (Option<String>, Option<Vec<String>>) {
    (arg_s(a, "group"), arg_repos(a))
}

pub(super) fn resolve(d: &Path, a: &Args) -> Result<(String, Vec<registry::Repo>)> {
    let (g, ids) = scope_ids(a);
    registry::resolve_group(d, g.as_deref(), ids.as_deref())
}
