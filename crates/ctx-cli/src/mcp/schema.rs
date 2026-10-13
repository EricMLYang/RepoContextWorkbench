//! inputSchema 的簡寫。

use super::*;

// ---------------------------------------------------------------- inputSchema 簡寫

pub(super) fn str_t() -> Value {
    json!({"type": "string"})
}
pub(super) fn bool_t() -> Value {
    json!({"type": "boolean"})
}
pub(super) fn int_t() -> Value {
    json!({"type": "integer"})
}
pub(super) fn arr_t() -> Value {
    json!({"type": "array", "items": {"type": "string"}})
}

/// Python `_d(base, desc, **extra)`：`{**base, "description": desc, **extra}`。
pub(super) fn d_(base: Value, desc: &str) -> Value {
    let mut m = match base {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    m.insert("description".into(), json!(desc));
    Value::Object(m)
}

/// Python `_s(**props)`：`{"type":"object","properties":{...},"required":[...]}`。
pub(super) fn s_(props: Vec<(&str, Value)>, req: &[&str]) -> Value {
    let mut p = Map::new();
    for (k, v) in props {
        p.insert(k.into(), v);
    }
    json!({"type": "object", "properties": p, "required": req})
}

pub(super) fn scope_props() -> Vec<(&'static str, Value)> {
    vec![
        ("group", d_(str_t(), "組名；省略＝依 cwd 推出的主場")),
        (
            "repos",
            d_(str_t(), "逗號分隔 repo id（臨時組合，優先於 group）"),
        ),
    ]
}
pub(super) fn cwd_props() -> Vec<(&'static str, Value)> {
    vec![(
        "cwd",
        d_(
            str_t(),
            "用哪個目錄推主場；省略＝server 啟動目錄（通常就是你的專案）",
        ),
    )]
}
pub(super) fn cross_props() -> Vec<(&'static str, Value)> {
    vec![(
        "cross_scope_ok",
        d_(
            bool_t(),
            "寫入主場以外的範圍。只有在使用者明確同意後才設 true",
        ),
    )]
}

pub(super) fn cat(parts: Vec<Vec<(&'static str, Value)>>) -> Vec<(&'static str, Value)> {
    parts.into_iter().flatten().collect()
}
