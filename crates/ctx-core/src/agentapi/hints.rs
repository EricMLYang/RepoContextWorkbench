//! Rust 才有（Python 沒有）：提醒（N-22）：hook 開場與 MCP context_for 共用的一份。
//! 從 agentapi.rs 搬出來（2026-10-10，ideation N-24），行為不變；`agentapi::` 底下的路徑照舊。

use super::*;

/// 要處理的提醒（依序）：未結體檢、該重看、判斷回顧、關係建議、有條件的判斷。每筆 `{kind, text}`。
/// hook 開場渲染成「## 提醒」一段；MCP `context_for` 回 `hints`——同一份。
pub(super) fn collect_hints(spine_dir: &Path, t: &Target, ctx: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut push = |kind: &str, text: String| out.push(json!({"kind": kind, "text": text}));
    if let Some(h) = review_hint(spine_dir, t) {
        push("review", h);
    }
    let reopen: Vec<&Value> = ctx["reopen"].as_array().into_iter().flatten().collect();
    for r in reopen.iter().filter(|r| r["due"] == true).take(2) {
        push(
            "reopen",
            format!(
                "該重看：{}（何時重看：{}）[{}]——要改就 log_decision 帶 supersedes",
                r["text"].as_str().unwrap_or(""),
                r["when"].as_str().unwrap_or(""),
                r["ref"].as_str().unwrap_or("")
            ),
        );
    }
    if let Some(h) = retro_hint(spine_dir, t) {
        push("retro", h);
    }
    if let Some(h) = relation_hint(spine_dir, ctx) {
        push("relation", h);
    }
    for r in reopen.iter().filter(|r| r["due"] != true).take(1) {
        push(
            "reopen_cond",
            format!(
                "有條件的判斷：{}（何時重看：{}）[{}]",
                r["text"].as_str().unwrap_or(""),
                r["when"].as_str().unwrap_or(""),
                r["ref"].as_str().unwrap_or("")
            ),
        );
    }
    out
}

/// 上次算好的關係建議裡，跟主場範圍有關的第一筆（`.state/relation_suggest.json`）。
fn relation_hint(spine_dir: &Path, ctx: &Value) -> Option<String> {
    let repos: Vec<&str> = ctx["scope"]["repos"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let s = crate::relsuggest::load(spine_dir)
        .into_iter()
        .find(|s| repos.contains(&s.a.as_str()) || repos.contains(&s.b.as_str()))?;
    Some(format!(
        "關係建議：{} ↔ {} 還沒設關係（{}）——問使用者要不要設，`ctx registry suggest` 看全部",
        s.a,
        s.b,
        s.reason()
    ))
}
