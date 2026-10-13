//! 會話：清冊上的回報、交接草稿。

use super::*;

// ── 會話 ───────────────────────────────────────────────────────────────

pub(super) fn tget<'a>(term: &'a Value, k: &str) -> Option<&'a str> {
    term.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// 對應 `state._session_scope`：會話範圍 → (group, ids)。認不出來就回 (None, None)＝不限範圍。
pub(super) fn session_scope(
    term: &Value,
    data: &RegistryData,
) -> (Option<String>, Option<Vec<String>>) {
    if let Some(scope) = term.get("scope").and_then(Value::as_str) {
        if let Some(g) = data.group(scope) {
            return (Some(scope.to_string()), Some(g.members()));
        }
        if data.repo(scope).is_some() {
            return (None, Some(vec![scope.to_string()]));
        }
    }
    let members: Vec<String> = term
        .get("scope_repos")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    (None, (!members.is_empty()).then_some(members))
}

pub(super) fn in_scope(ev: &Event, group: Option<&str>, ids: Option<&[String]>) -> bool {
    spine::in_scope(ev, group, ids.unwrap_or(&[]))
}

/// 對應 `state.session_report`：會話開始後、範圍內的最後一筆脊椎留痕；沒有就 None。
///
/// 回傳帶 source／type／時間——使用者要看得出這筆是 agent 寫的還是工作台寫的，
/// 才判斷得了「現在輪到誰」。不做任何由程序狀態推進度的推論。
pub fn session_report(term: &Value, events: &[Event], data: &RegistryData) -> Option<Value> {
    let started = tget(term, "started")?;
    let (group, ids) = session_scope(term, data);
    let mut best: Option<&Event> = None;
    for ev in events {
        if REPORT_SKIP_TYPES.contains(&ev.etype) || ev.stamp().as_str() < started {
            continue;
        }
        if !in_scope(ev, group.as_deref(), ids.as_deref()) {
            continue;
        }
        if best.is_none_or(|b| (ev.date.as_str(), ev.time.as_str()) >= (&b.date, &b.time)) {
            best = Some(ev);
        }
    }
    let best = best?;
    Some(
        json!({"text": first_or_tokens(best), "type": best.etype.as_str(),
                "source": best.source, "at": best.stamp()}),
    )
}

/// 對應 `state.annotate_terms`：替會話清冊補上回報狀態（清冊本身由 TermManager 給，這裡只加脊椎那一半）。
/// `terms` 是 `TermManager::list()` 序列化後的物件（或測試給的部分欄位）；未知欄位原樣保留。
pub fn annotate_terms(spine_dir: impl AsRef<Path>, terms: &[Value]) -> Result<Vec<Value>> {
    let spine_dir = spine_dir.as_ref();
    let events = spine::iter_events(spine_dir)?;
    let data = registry::load(spine_dir)?;
    Ok(annotate_terms_with(spine_dir, terms, &events, &data))
}

pub(super) fn annotate_terms_with(
    spine_dir: &Path,
    terms: &[Value],
    events: &[Event],
    data: &RegistryData,
) -> Vec<Value> {
    terms
        .iter()
        .map(|t| {
            let mut rec = t.as_object().cloned().unwrap_or_default();
            if t.get("kind").and_then(Value::as_str) != Some("agent") {
                rec.insert("report".into(), Value::Null);
                rec.insert("report_basis".into(), json!("shell 會話不回報"));
            } else {
                let (group, ids) = session_scope(t, data);
                rec.insert(
                    "report".into(),
                    session_report(t, events, data).unwrap_or(Value::Null),
                );
                // agent 自己回報的階段（report_status）——只認會話開始後的，舊的不算
                let started = tget(t, "started").unwrap_or("~");
                let st = status::get_status(spine_dir, group.as_deref(), ids.as_deref())
                    .filter(|s| s.as_object().is_some_and(|o| !o.is_empty()))
                    .filter(|s| s.get("at").and_then(Value::as_str).unwrap_or("") >= started);
                rec.insert("agent_status".into(), st.unwrap_or(Value::Null));
                // 依據要說得出口：範圍認得出來就講範圍，認不出來就說沒再過濾
                let basis = match (&group, &ids) {
                    (Some(g), _) => format!("會話開始後、範圍內（{g}）的脊椎留痕"),
                    (None, Some(ids)) => {
                        format!("會話開始後、範圍內（{}）的脊椎留痕", ids.join("、"))
                    }
                    (None, None) => {
                        "會話開始後的脊椎留痕（此會話範圍是全部 repo，未再依組過濾）".into()
                    }
                };
                rec.insert("report_basis".into(), json!(basis));
            }
            Value::Object(rec)
        })
        .collect()
}

/// [`TermInfo`] → 給 [`annotate_terms`]／[`session_draft`] 的物件（欄位同 Python `TermManager.list()`）。
pub fn term_value(info: &TermInfo) -> Value {
    serde_json::to_value(info).unwrap_or(Value::Null)
}

/// 對應 `state.session_draft`：交接草稿＝會話期間的脊椎留痕彙整。是草稿不是成果，尚未寫入任何事件。
///
/// 2026-09-12 檢討 §5：「記錄結果」原本是空白表單，使用者得自己翻終端整理。
/// 這裡只把已經存在的留痕排好給人確認——沒有留痕就直說沒有，不編造成果。
pub fn session_draft(spine_dir: impl AsRef<Path>, term: &Value) -> Result<Value> {
    let spine_dir = spine_dir.as_ref();
    let events = spine::iter_events(spine_dir)?;
    let data = registry::load(spine_dir)?;
    let started = tget(term, "started");
    let (group, ids) = session_scope(term, &data);
    let traces: Vec<&Event> = events
        .iter()
        .filter(|ev| {
            !REPORT_SKIP_TYPES.contains(&ev.etype)
                && started.is_none_or(|s| ev.stamp().as_str() >= s)
                && in_scope(ev, group.as_deref(), ids.as_deref())
        })
        .collect();
    let loops: Vec<Event> = spine::open_loops_of(&events)
        .into_iter()
        .filter(|ev| in_scope(ev, group.as_deref(), ids.as_deref()))
        .collect();
    let title = tget(term, "title")
        .or_else(|| tget(term, "sid"))
        .unwrap_or("None");
    let mut lines = vec![
        format!(
            "會話：{title}（範圍 {}{}）",
            tget(term, "scope").unwrap_or("本機 Shell"),
            started.map(|s| format!("，開始於 {s}")).unwrap_or_default()
        ),
        String::new(),
        "完成了什麼（以下是會話期間的脊椎留痕，請確認後再存）：".into(),
    ];
    if traces.is_empty() {
        lines.push("- （會話期間沒有留痕，請自己補；下次可請 agent 收尾時呼叫 handoff）".into());
    } else {
        for ev in &traces[traces.len().saturating_sub(8)..] {
            lines.push(format!(
                "- {} {} {} [{}] {}",
                ev.date,
                ev.time,
                ev.etype,
                ev.source,
                first_or_tokens(ev)
            ));
        }
    }
    lines.push(String::new());
    lines.push("還沒結束的事：".into());
    if loops.is_empty() {
        lines.push("- （本範圍目前沒有未結事項）".into());
    } else {
        for ev in loops.iter().take(6) {
            let num = ev.num().unwrap_or("#?");
            let first = if ev.body.is_empty() {
                ""
            } else {
                splitlines(&ev.body).first().copied().unwrap_or("")
            };
            let due = ev
                .kv("due")
                .filter(|d| !d.is_empty())
                .map(|d| format!("（due {d}）"))
                .unwrap_or_default();
            lines.push(format!("- {num}「{first}」{due}"));
        }
    }
    lines.extend(["".into(), "下次從哪裡接回：".into(), "- （請補上）".into()]);
    Ok(json!({
        "text": lines.join("\n"),
        "traces": traces.len(),
        "loops": loops.len(),
        "note": "草稿由脊椎留痕組成，尚未儲存；存檔會落一筆 decision，不代表成果已被採納。",
    }))
}
