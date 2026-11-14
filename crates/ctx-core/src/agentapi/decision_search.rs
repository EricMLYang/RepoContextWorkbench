//! Rust 才有（Python 沒有）：判斷搜尋（F-22）：過去的判斷也搜得到。
//! 從 agentapi.rs 搬出來（2026-10-10，ideation N-24），行為不變；`agentapi::` 底下的路徑照舊。

use super::*;

/// 過去的判斷（decision，含交接）裡跟查詢最相關的幾筆：BM25（同 knowledge）。
/// 每筆帶 ref、日期、第一行、命中的那一行、被誰取代（`superseded_by`）、之後的結果（outcome，`ref:` 指回它的）。
/// `everywhere`＝不限範圍；否則只看主場範圍內的判斷。
pub(super) fn search_decisions(
    spine_dir: &Path,
    query: &str,
    w: &Where,
    t: &Target,
    everywhere: bool,
    limit: usize,
) -> Result<Vec<Value>> {
    // 查詢詞去掉含虛字的 bigram（「不用」「的是」）：判斷大多很短，一個虛字詞就能把無關的判斷拉上來
    let toks: Vec<String> = knowledge::tokenize(query);
    let strong: Vec<String> = toks
        .iter()
        .filter(|t| ruminate::meaningful(t))
        .cloned()
        .collect();
    let qcount = knowledge::count(if strong.is_empty() { toks } else { strong });
    if qcount.is_empty() {
        return Ok(Vec::new());
    }
    let all = spine::iter_events(spine_dir)?;
    let s = if everywhere {
        None
    } else {
        Some(scope_of(spine_dir, w, t)?)
    };
    let decisions: Vec<&Event> = all
        .iter()
        .filter(|ev| ev.etype == EventType::Decision)
        .filter(|ev| s.as_ref().is_none_or(|s| s.contains(ev)))
        .collect();
    if decisions.is_empty() {
        return Ok(Vec::new());
    }
    let docs: Vec<knowledge::Doc> = decisions
        .iter()
        .map(|ev| {
            let c = knowledge::count(knowledge::tokenize(&ev.body));
            let len: u64 = c.iter().map(|x| x.1).sum();
            knowledge::Doc {
                repo: kv_opt(ev, "repo").unwrap_or_default(),
                path: String::new(),
                rel: String::new(),
                tf: c.into_iter().map(|(k, v)| (k, v as f64)).collect(),
                len: len as f64,
                htf: Default::default(),
                hlen: 0.0,
                mtime: 0.0,
            }
        })
        .collect();
    let scores = knowledge::bm25(&qcount, &docs, false);
    let superseded = superseded_map(&all);
    let qset: HashSet<&str> = qcount.iter().map(|(t, _)| t.as_str()).collect();
    // 被取代的判斷打五折（目前有效的排前面，沿革仍查得到）
    let mut ranked: Vec<(f64, &Event)> = scores
        .into_iter()
        .zip(decisions)
        .map(|(sc, ev)| {
            let k = if superseded.contains_key(&event_ref(ev)) {
                0.5
            } else {
                1.0
            };
            (sc * k, ev)
        })
        .filter(|(sc, _)| *sc > 0.0)
        .collect();
    ranked.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| b.1.stamp().cmp(&a.1.stamp()))
    });
    // 只留跟最佳結果同一個量級的（分數 < 最高分 30% 的多半只是碰到一兩個常見詞）
    let top = ranked.first().map(|x| x.0).unwrap_or(0.0);
    ranked.retain(|(sc, _)| *sc >= top * 0.3);
    // F-126：內容主要在講別的 repo 的判斷（第一行以那個 repo 的 id 開頭）——限定範圍搜尋時排到後面
    let ids: Vec<String> = registry::load(spine_dir)?
        .repos
        .iter()
        .map(Repo::id)
        .collect();
    let about = |ev: &Event| -> Option<String> {
        let a = about_repo(&first(ev), &ids)?;
        let own = ev.kv("repo").unwrap_or("");
        (!spine::same_token(own, &a)).then_some(a)
    };
    if !everywhere {
        ranked.sort_by_key(|(_, ev)| about(ev).is_some());
    }
    Ok(ranked
        .into_iter()
        .take(limit)
        .map(|(sc, ev)| {
            let r = event_ref(ev);
            // 命中最多查詢詞的那一行
            let hit = splitlines(&ev.body)
                .into_iter()
                .max_by_key(|ln| {
                    knowledge::tokenize(ln)
                        .iter()
                        .filter(|t| qset.contains(t.as_str()))
                        .count()
                })
                .unwrap_or("")
                .to_string();
            let outcomes: Vec<Value> = all
                .iter()
                .filter(|o| o.etype == EventType::Outcome && o.kv("ref") == Some(r.as_str()))
                .map(|o| json!({"at": o.stamp(), "text": first(o)}))
                .collect();
            // 命中的就是第一行＝不另給（免得重複）
            let text = first(ev);
            let hit = if text.starts_with(crate::pystr::take_chars(&hit, 40)) {
                String::new()
            } else {
                crate::pystr::take_chars(&hit, 160).to_string()
            };
            let mut v = json!({
                "ref": r, "at": ev.stamp(), "text": text,
                "hit": hit, "score": knowledge::round2(sc),
                "group": kv_opt(ev, "group"), "repo": kv_opt(ev, "repo"),
            });
            if let Some(by) = superseded.get(&event_ref(ev)) {
                v["superseded_by"] = json!(by);
            }
            if let Some(cond) = reopen_when(ev) {
                v["reopen_when"] = json!(cond);
            }
            if let Some(a) = about(ev) {
                v["about_repo"] = json!(a);
            }
            if !outcomes.is_empty() {
                v["outcomes"] = Value::Array(outcomes);
            }
            v
        })
        .collect())
}

/// 判斷的第一行明確以某個登記 repo 的 id 開頭（去掉「交接：」與〔…〕前綴後；id 之後要接空白、全形冒號、「的」、
/// 括號或標點）＝這筆主要在講那個 repo。保守：不在開頭的不算，最長的 id 優先。
pub(crate) fn about_repo(first_line: &str, ids: &[String]) -> Option<String> {
    let mut t = first_line.trim();
    if let Some(rest) = t.strip_prefix("交接：") {
        t = rest.trim_start();
    }
    if t.starts_with('〔') {
        t = t.split_once('〕').map_or(t, |(_, r)| r.trim_start());
    }
    let mut best: Option<&String> = None;
    for id in ids {
        if let Some(rest) = t.strip_prefix(id.as_str()) {
            let boundary = rest
                .chars()
                .next()
                .is_none_or(|c| c.is_whitespace() || "：:的（(，,、。；;／/".contains(c));
            if boundary && best.is_none_or(|b| id.len() > b.len()) {
                best = Some(id);
            }
        }
    }
    best.cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_repo_is_conservative() {
        let ids: Vec<String> = ["RepoContextWorkbench", "Repo", "draftagent"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            about_repo("交接：RepoContextWorkbench 的接力第 2 輪", &ids).as_deref(),
            Some("RepoContextWorkbench")
        );
        assert_eq!(
            about_repo("draftagent：改 verifier", &ids).as_deref(),
            Some("draftagent")
        );
        assert_eq!(
            about_repo("改 draftagent 的 verifier", &ids),
            None,
            "不在開頭不算"
        );
        assert_eq!(
            about_repo("draftagents 的事", &ids),
            None,
            "id 後面要有邊界"
        );
        assert_eq!(
            about_repo("〔registry〕draftagent 名片：…", &ids).as_deref(),
            Some("draftagent")
        );
    }
}
