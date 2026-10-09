//! Rust 才有（Python 沒有）：判斷匯出成 AgDR md（F-40）。
//! 從 agentapi.rs 搬出來（2026-10-10，ideation N-24），行為不變；`agentapi::` 底下的路徑照舊。

use super::*;

/// 把範圍內的判斷寫成 AgDR 格式的 md（`<dir>/AgDR-<id>.md`）：不掛 ctx MCP 的 agent 也讀得到 repo 裡的檔。
/// 只匯出人／agent／回填寫的、有 id、不是交接的判斷。被取代的標 `status: superseded` 並寫明被誰取代——
/// 舊紀錄自己身上要看得到（ADR 實務）。帶何時重看與之後的結果。內容沒變的檔不重寫。只寫 `dir`，不碰別處。
pub fn export_agdr(spine_dir: &Path, t: &Target, dir: &Path) -> Result<Value> {
    let w = where_am_i(spine_dir, t.cwd.as_deref())?;
    let s = scope_of(spine_dir, &w, t)?;
    let events = spine::iter_events(spine_dir)?;
    let superseded = superseded_map(&events);
    std::fs::create_dir_all(dir)?;
    let (mut written, mut unchanged) = (Vec::new(), 0usize);
    for ev in events.iter().filter(|e| {
        e.etype == EventType::Decision
            && s.contains(e)
            && !is_handoff(e)
            && !e.body.starts_with('〔')
            && matches!(e.source.as_str(), "agent" | "manual" | "backfill")
    }) {
        let Some(id) = ev.kv("id").filter(|i| !i.is_empty()) else {
            continue;
        };
        let r = event_ref(ev);
        let lines = splitlines(&ev.body);
        let title = lines.first().copied().unwrap_or("");
        let rest: Vec<&str> = lines
            .iter()
            .skip(1)
            .copied()
            .filter(|l| !l.starts_with("取代：") && !l.starts_with("何時重看："))
            .collect();
        let supersedes: Vec<String> = SUPERSEDE_LINE
            .captures_iter(&ev.body)
            .map(|c| c[1].to_string())
            .collect();
        let mut fm = vec![
            "---".to_string(),
            format!("id: {r}"),
            format!("timestamp: {}T{}", ev.date, ev.time),
            format!("agent: {}", ev.source),
            format!(
                "status: {}",
                if superseded.contains_key(&r) {
                    "superseded"
                } else {
                    "executed"
                }
            ),
        ];
        if !supersedes.is_empty() {
            fm.push(format!("supersedes: [{}]", supersedes.join(", ")));
        }
        if let Some(by) = superseded.get(&r) {
            fm.push(format!("superseded_by: {by}"));
        }
        for k in ["group", "repo"] {
            if let Some(v) = ev.kv(k).filter(|v| !v.is_empty()) {
                fm.push(format!("{k}: {v}"));
            }
        }
        fm.push("source: repo-context spine".into());
        fm.push("---".into());
        let mut md = fm;
        md.push(String::new());
        md.push(format!("# {title}"));
        if let Some(by) = superseded.get(&r) {
            md.push(String::new());
            md.push(format!("> 已被 `{by}` 取代。"));
        }
        if !rest.is_empty() {
            md.push(String::new());
            md.extend(rest.iter().map(|l| l.to_string()));
        }
        if let Some(cond) = reopen_when(ev) {
            md.extend([String::new(), "## 何時重看".into(), cond]);
        }
        let outcomes: Vec<String> = events
            .iter()
            .filter(|o| o.etype == EventType::Outcome && o.kv("ref") == Some(r.as_str()))
            .map(|o| format!("- {}：{}", o.stamp(), first(o)))
            .collect();
        if !outcomes.is_empty() {
            md.extend([String::new(), "## 結果".into()]);
            md.extend(outcomes);
        }
        let text = md.join("\n") + "\n";
        let path = dir.join(format!("AgDR-{id}.md"));
        if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
            unchanged += 1;
            continue;
        }
        std::fs::write(&path, text)?;
        written.push(
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
    }
    Ok(json!({
        "ok": true, "scope": scope_json(&s), "dir": dir.to_string_lossy(),
        "written": written, "unchanged": unchanged,
    }))
}
