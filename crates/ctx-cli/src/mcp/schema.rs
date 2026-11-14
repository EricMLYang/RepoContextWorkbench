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

/// `handoff` 的 `ask` 物件（F-123）。描述每場會話都花 token：只講什麼時候填、好壞各一例。
pub(super) fn ask_schema() -> Value {
    let mut kind = d_(str_t(), "類別；省略＝other");
    kind["enum"] = json!(ctx_core::ask::AskKind::NAMES);
    let mut v = s_(
        vec![
            ("question", d_(str_t(), "一句不看程式也答得出來的問題")),
            ("kind", kind),
            ("options", d_(arr_t(), "選項（使用者用 1 起算的編號回答）")),
            ("recommend", d_(str_t(), "你建議的選項（文字或編號）")),
            ("reversible", d_(bool_t(), "做了收得回來嗎；預設 true")),
            (
                "evidence",
                d_(arr_t(), "要看的檔：repo 內相對路徑或 <repo>:<路徑>"),
            ),
        ],
        &["question"],
    );
    v["description"] = json!(
        "可省略：下一步在等使用者決定／確認／驗收時才填，使用者會在決策佇列（ctx decide）看到並用編號回答。\
         好：「首頁排序要改成依修改時間（A）還是維持依名稱（B）？」；差：「要繼續嗎？」"
    );
    v
}

pub(super) fn cat(parts: Vec<Vec<(&'static str, Value)>>) -> Vec<(&'static str, Value)> {
    parts.into_iter().flatten().collect()
}
