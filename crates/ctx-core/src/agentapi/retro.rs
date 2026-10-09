//! Rust 才有（Python 沒有）：判斷回顧（F-23）：滿 14 天還沒記結果的判斷。
//! 從 agentapi.rs 搬出來（2026-10-10，ideation N-24），行為不變；`agentapi::` 底下的路徑照舊。

use super::*;

/// 判斷滿這麼多天、還沒記結果，就提醒回顧（跟未結事項預設期限一樣）。
pub const RETRO_DAYS: i64 = 14;

/// 待回顧的判斷（舊的在前）：範圍內、人或 agent 寫的（有 id）、不是交接、沒被取代、滿 [`RETRO_DAYS`] 天、
/// 還沒有 outcome 指回它、
/// 沒延後（跟「該重看」共用 `.state/reopen_snooze.json`）。`scope`＝None 是全部。
pub fn retro_candidates(
    spine_dir: &Path,
    events: &[Event],
    scope: Option<(&str, &[String])>,
    now: chrono::NaiveDateTime,
) -> Vec<Value> {
    let superseded = superseded_map(events);
    let snoozes = reopen_snoozes(spine_dir);
    let today = now.format("%Y-%m-%d").to_string();
    let cutoff = (now - chrono::Duration::days(RETRO_DAYS))
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let has_outcome: HashSet<&str> = events
        .iter()
        .filter(|e| e.etype == EventType::Outcome)
        .filter_map(|e| e.kv("ref"))
        .collect();
    events
        .iter()
        .filter(|e| e.etype == EventType::Decision && !is_handoff(e) && e.stamp() <= cutoff)
        // 只回顧人、agent 或回填（從過去會話整理出的）判斷：工具自己留的操作紀錄（〔registry〕…）不算；沒有 id 的 ref 只有時間，
        // 不同天同一分鐘會撞，結果會指錯
        .filter(|e| {
            matches!(e.source.as_str(), "agent" | "manual" | "backfill")
                && e.kv("id").is_some_and(|i| !i.is_empty())
                && !e.body.starts_with('〔')
        })
        .filter(|e| scope.is_none_or(|(g, ids)| spine::in_scope(e, Some(g), ids)))
        .filter(|e| {
            let r = event_ref(e);
            !superseded.contains_key(&r)
                && !has_outcome.contains(r.as_str())
                && !snoozes
                    .get(&r)
                    .and_then(Value::as_str)
                    .is_some_and(|u| u > today.as_str())
        })
        .map(|e| {
            let days = chrono::NaiveDateTime::parse_from_str(&e.stamp(), "%Y-%m-%d %H:%M")
                .map(|t| (now - t).num_days())
                .unwrap_or(0);
            json!({"ref": event_ref(e), "at": e.stamp(), "days": days, "text": first(e),
                   "group": kv_opt(e, "group"), "repo": kv_opt(e, "repo")})
        })
        .collect()
}

/// 開場一行：主場範圍內最舊的待回顧判斷。
pub fn retro_hint(spine_dir: &Path, t: &Target) -> Option<String> {
    let w = where_am_i(spine_dir, t.cwd.as_deref()).ok()?;
    let s = scope_of(spine_dir, &w, t).ok()?;
    let g = s.group.clone()?;
    let events = spine::iter_events(spine_dir).ok()?;
    let all = retro_candidates(
        spine_dir,
        &events,
        Some((&g, s.ids())),
        Local::now().naive_local(),
    );
    let first = all.first()?;
    Some(format!(
        "判斷回顧（共 {} 筆待回顧）：{}（{} 天前）「{}」後來怎樣？有結果就 log_decision 帶 outcome_of={}",
        all.len(),
        first["ref"].as_str().unwrap_or(""),
        first["days"],
        crate::pystr::take_chars(first["text"].as_str().unwrap_or(""), 50),
        first["ref"].as_str().unwrap_or("")
    ))
}
