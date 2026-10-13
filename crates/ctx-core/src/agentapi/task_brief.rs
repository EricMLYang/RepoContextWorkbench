//! Rust 才有（Python 沒有）：任務包（F-42）：把一項未結組成給新會話的交辦 markdown。
//! 從 agentapi.rs 搬出來（2026-10-10，ideation N-24），行為不變；`agentapi::` 底下的路徑照舊。

use super::*;

/// 任務包裡的判斷至少要蓋住待辦文字的這個比例。
const BRIEF_DECISION_COVER: f64 = 0.2;

/// 把一個未結事項組成給全新會話的任務包（markdown）：目標、背景、輸入、限制、完成條件。
/// 全由引擎現有的資料組：組目標、相關的有效判斷（判斷搜尋，帶取代與重看）、上次交接、相關檔（知識搜尋）。
/// 範圍＝那一項所屬的組（沒有組就用 repo 或主場）。參考 MetaClaw／Brief-and-Drill 交辦模板：只給最少必要的背景。
pub fn todo_brief(spine_dir: &Path, num: &str, t: &Target) -> Result<Value> {
    let num = if num.starts_with('#') {
        num.to_string()
    } else {
        format!("#{num}")
    };
    let lp = spine::open_loops(spine_dir)?
        .into_iter()
        .find(|ev| ev.num() == Some(num.as_str()))
        .ok_or_else(|| {
            agent_error(
                crate::RejectCode::NotFound,
                format!("找不到未結事項 {num}"),
                Some("用 next_work 看目前的未結事項編號。"),
            )
        })?;
    let target = match (lp.kv("group"), lp.kv("repo")) {
        (Some(g), _) if !g.is_empty() => Target {
            cwd: None,
            group: Some(g.to_string()),
            repos: None,
        },
        (_, Some(r)) if !r.is_empty() => Target {
            cwd: None,
            group: None,
            repos: Some(vec![r.to_string()]),
        },
        _ => t.clone(),
    };
    let w = where_am_i(spine_dir, target.cwd.as_deref().or(t.cwd.as_deref()))?;
    let s = scope_of(spine_dir, &w, &target)?;
    let text = BACKFILL_TAIL
        .replace_all(lp.first_line(), "")
        .trim()
        .to_string();
    let goal = s
        .group
        .as_deref()
        .and_then(|g| registry::get_group(spine_dir, g).ok())
        .and_then(|g| g.goal());
    let events = spine::iter_events(spine_dir)?;
    let handoff = last_handoff(&events, &s);
    // 任務包只放真的相關的判斷：判斷內容要蓋住待辦文字一定比例（BM25 前三名常只是碰到一兩個詞）
    let decisions: Vec<Value> = search_decisions(spine_dir, &text, &w, &target, false, 5)?
        .into_iter()
        .filter(|d| {
            let body = format!(
                "{} {}",
                d["text"].as_str().unwrap_or(""),
                d["hit"].as_str().unwrap_or("")
            );
            let terms: HashSet<String> = knowledge::tokenize(&body).into_iter().collect();
            coverage(&text, &terms).1 >= BRIEF_DECISION_COVER
        })
        .take(3)
        .collect();
    let files = search_knowledge(
        spine_dir,
        &text,
        &target,
        &SearchOpts {
            everywhere: false,
            limit: 5,
            exclude_paths: Vec::new(),
            code: false,
        },
    )
    .map(|v| v["files"].as_array().cloned().unwrap_or_default())
    .unwrap_or_default();

    let mut md: Vec<String> = vec![
        format!("# 任務包：{num} {text}"),
        String::new(),
        "## 目標".into(),
        format!("- {text}"),
    ];
    if let Some(d) = lp.kv("due").filter(|d| !d.is_empty()) {
        md.push(format!("- 期限：{d}"));
    }
    md.extend([String::new(), "## 背景".into()]);
    md.push(format!("- 範圍：{}（{}）", s.label, s.ids().join("、")));
    if let Some(g) = &goal {
        md.push(format!("- 組目標：{g}"));
    }
    if let Some(h) = &handoff {
        md.push(format!(
            "- 上次交接（{}）：{}",
            h["at"].as_str().unwrap_or(""),
            h["summary"].as_str().unwrap_or("")
        ));
    }
    let mut reopen: Vec<String> = Vec::new();
    for d in decisions
        .iter()
        .filter(|d| d.get("superseded_by").is_none())
    {
        md.push(format!(
            "- 相關判斷 {}（{}）：{}",
            d["ref"].as_str().unwrap_or(""),
            d["at"].as_str().unwrap_or(""),
            d["text"].as_str().unwrap_or("")
        ));
        if let Some(c) = d["reopen_when"].as_str() {
            reopen.push(format!("{}：{c}", d["ref"].as_str().unwrap_or("")));
        }
    }
    md.extend([String::new(), "## 輸入（先讀這些，用 read_from）".into()]);
    if files.is_empty() {
        md.push("- （沒有找到明顯相關的檔，先用 search_knowledge 找）".into());
    }
    for f in &files {
        md.push(format!("- `{}`", f["ref"].as_str().unwrap_or("")));
    }
    md.extend([String::new(), "## 限制".into()]);
    md.push(format!(
        "- 只在範圍「{}」內寫入；要動別組先問使用者",
        s.label
    ));
    for r in &reopen {
        md.push(format!("- 相關判斷的重看條件（碰到就停下來問）：{r}"));
    }
    md.push("- 卡住或要等人：report_status".into());
    md.extend([
        String::new(),
        "## 完成條件".into(),
        format!("- 做完：close_todo {num}，note 寫怎麼結的（附 commit 或檔案）"),
        "- 做不完：handoff 交代做到哪、剩什麼、下次從哪接".into(),
    ]);
    Ok(json!({
        "ok": true, "id": num, "scope": scope_json(&s),
        "decisions": decisions, "files": files,
        "markdown": md.join("\n"),
    }))
}
