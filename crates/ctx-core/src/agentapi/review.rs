//! Rust 才有（Python 沒有）：未結事項體檢（F-21）：哪些可能已處理（附證據）、哪些逾期／擱置沒動靜。
//! 從 agentapi.rs 搬出來（2026-10-10，ideation N-24），行為不變；`agentapi::` 底下的路徑照舊。

use super::*;

/// 擱置超過這麼多天、期間沒人提到也找不到相關動靜＝「擱置太久」。
pub const REVIEW_STALE_DAYS: i64 = 14;
/// 證據蓋住未結事項文字的比例達到這麼多，才說「可能已處理」。
const REVIEW_DONE_COVER: f64 = 0.45;
/// 列為證據的最低覆蓋率。
const REVIEW_EVIDENCE_COVER: f64 = 0.2;

pub(super) static BACKFILL_TAIL: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"[（(]回填自[^）)]*[）)]").expect("寫死的 regex"));
static CJK_RUN: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"[\x{3400}-\x{9FFF}\x{F900}-\x{FAFF}]+").expect("寫死的 regex")
});

/// 一筆可能的證據（opened 之後的 commit 或判斷／交接的「完成」）。
struct Cand {
    kind: &'static str,
    reference: String,
    at: String,
    text: String,
    terms: HashSet<String>,
}

/// 證據蓋住了未結事項文字的哪些片段、比例多少。
///
/// 中文不是數零散的 bigram（跨詞邊界的「作台」「場注」會亂湊分數），而是找**連續命中**的段落：
/// 「開場注入」四個字三個 bigram 都在證據裡＝一個片段「開場注入」；英數字詞各算一個。
/// 比例＝蓋住的中文字數＋命中的英數字詞數，除以未結事項的中文字數＋英數字詞數。
pub(super) fn coverage(text: &str, terms: &HashSet<String>) -> (Vec<String>, f64) {
    CoverText::new(text).cover(terms)
}

/// 未結事項文字斷好的樣子（英數字詞＋各中文段的 bigram），同一項對很多證據比時只斷一次（N-12）。
struct CoverText {
    words: Vec<String>,
    /// 每個中文段：(字, 各位置的 bigram)
    segs: Vec<(Vec<char>, Vec<String>)>,
}

impl CoverText {
    fn new(text: &str) -> Self {
        let low = text.to_lowercase();
        let words = knowledge::tokenize(&CJK_RUN.replace_all(&low, " "))
            .into_iter()
            .filter(|w| ruminate::meaningful(w) && w.chars().count() >= 2)
            .collect();
        let segs = CJK_RUN
            .find_iter(&low)
            .map(|seg| {
                let cs: Vec<char> = seg.as_str().chars().collect();
                let bigrams = cs.windows(2).map(|w| w.iter().collect()).collect();
                (cs, bigrams)
            })
            .collect();
        CoverText { words, segs }
    }

    fn cover(&self, terms: &HashSet<String>) -> (Vec<String>, f64) {
        let mut pieces: Vec<String> = Vec::new();
        let (mut hit, mut total) = (0usize, self.words.len());
        for w in &self.words {
            if terms.contains(w) {
                hit += 1;
                pieces.push(w.clone());
            }
        }
        for (cs, bigrams) in &self.segs {
            total += cs.len();
            let mut i = 0;
            while i < bigrams.len() {
                if !terms.contains(&bigrams[i]) {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < bigrams.len() && terms.contains(&bigrams[i]) {
                    i += 1;
                }
                // bigram start..i 都命中＝字 start..=i 被蓋住
                let piece: String = cs[start..=i].iter().collect();
                hit += piece.chars().count();
                pieces.push(piece);
            }
        }
        let ratio = if total == 0 {
            0.0
        } else {
            hit as f64 / total as f64
        };
        (pieces, ratio)
    }
}

/// 有沒有提到 `#N`（後面不能再接數字：`#1` 不算提到 `#12`）。
fn mentions(text: &str, num: &str) -> bool {
    !num.is_empty()
        && text.match_indices(num).any(|(i, _)| {
            !text[i + num.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
        })
}

/// 文字裡提到的未結編號：`#` 後面整段數字（「#12」；「#123」不算提到 #12）——跟 [`mentions`] 同一個規則，
/// 每個事件先算一次（N-34：原本每一項未結都對每個事件做字串搜尋）。
fn mention_nums(text: &str) -> HashSet<String> {
    let b = text.as_bytes();
    let mut out = HashSet::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'#' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > i + 1 {
            out.insert(text[i..j].to_string());
        }
        i = j;
    }
    out
}

/// Rust 才有（2026-10-18）：做事的 commit（`feat`／`fix`／`perf`／`refactor` 開頭，可帶 `(scope)`）。
/// 這種 commit 的標題點名 `#N`＝為那項做的事，算「可能已處理」的證據；`docs`／`chore`／沒前綴的提到 `#N`
/// 只算有人提到（「#7 端到端在副本上走通」不等於 #7 做完）。
fn is_work_commit(subject: &str) -> bool {
    let head: String = subject
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    matches!(head.as_str(), "feat" | "fix" | "perf" | "refactor")
}

/// 交接只看「完成：」那行（剩下／下次是還沒做的事）；其他判斷看全文。
fn evidence_text(ev: &Event) -> String {
    if is_handoff(ev) {
        splitlines(&ev.body)
            .iter()
            .find_map(|l| l.strip_prefix("完成："))
            .unwrap_or("")
            .to_string()
    } else {
        ev.body.replace('\n', "；")
    }
}

/// Rust 才有（F-21）：未結事項體檢——每項看 opened 之後範圍內的 commit 與判斷／交接的「完成」，
/// 找「可能已處理」的證據（證據蓋住事項文字的比例夠高），或「擱置太久」（久沒人提、也沒相關動靜）。
/// 提到 `#N` 的事件只算「有人提到」不算證據（「#8 決定後才…」不是做完了；開出這項的交接也會提到它）。
/// 只提議、不自動關：附證據讓人或 agent 確認後用 close_todo。
pub fn review_todos(spine_dir: &Path, t: &Target) -> Result<Value> {
    review_todos_at(spine_dir, t, None)
}

/// 同 [`review_todos`]，可指定「現在」（測試用）。
pub fn review_todos_at(
    spine_dir: &Path,
    t: &Target,
    now: Option<chrono::NaiveDateTime>,
) -> Result<Value> {
    let w = where_am_i(spine_dir, t.cwd.as_deref())?;
    let s = scope_of(spine_dir, &w, t)?;
    let now = now.unwrap_or_else(|| Local::now().naive_local());
    let td = now.format("%Y-%m-%d").to_string();
    let loops = open_loops(spine_dir, &s)?;
    let events: Vec<Event> = spine::iter_events(spine_dir)?
        .into_iter()
        .filter(|ev| s.contains(ev))
        .collect();
    let since = loops.iter().map(Event::stamp).min().unwrap_or_default();
    // 範圍內各 repo 自最早一筆未結之後的 commit（一次撈，各項再依自己的 opened 篩）
    let mut commits: Vec<Cand> = Vec::new();
    if !since.is_empty() {
        for r in entries_of(spine_dir, &s)? {
            let root = r.path();
            if !root.is_dir() {
                continue;
            }
            let (rc, out) = crossref::git(
                &root,
                &[
                    "log",
                    &format!("--since={since}"),
                    "--format=%h%x09%ad%x09%s",
                    "--date=format:%Y-%m-%d %H:%M",
                    "-n",
                    "400",
                ],
                std::time::Duration::from_secs(10),
            );
            if rc != 0 {
                continue;
            }
            for ln in out.lines() {
                let mut it = ln.splitn(3, '\t');
                if let (Some(h), Some(at), Some(subj)) = (it.next(), it.next(), it.next()) {
                    commits.push(Cand {
                        kind: "commit",
                        reference: format!("{}@{h}", r.id()),
                        at: at.to_string(),
                        text: subj.to_string(),
                        terms: knowledge::tokenize(subj).into_iter().collect(),
                    });
                }
            }
        }
    }
    // N-34：每個事件的時間與提到的編號（內文＋token）先算一次——下面每一項未結都要看全部事件
    let stamps: Vec<String> = events.iter().map(Event::stamp).collect();
    let mentioned_in: Vec<HashSet<String>> = events
        .iter()
        .map(|ev| {
            let mut m = mention_nums(&ev.body);
            m.extend(ev.tokens.iter().filter(|t| t.starts_with('#')).cloned());
            m
        })
        .collect();
    // N-12：判斷／結果事件先斷詞一次（舊版每一項未結都把之後的事件重斷一次＝未結數 × 事件數）
    let ev_pre: Vec<(&Event, &HashSet<String>, Cand)> = events
        .iter()
        .zip(&mentioned_in)
        .filter(|(ev, _)| matches!(ev.etype, EventType::Decision | EventType::Outcome))
        .map(|(ev, m)| {
            let body = evidence_text(ev);
            let c = Cand {
                kind: if is_handoff(ev) { "handoff" } else { "event" },
                reference: format!(
                    "{}:{}",
                    ev.etype.as_str(),
                    ev.kv("id")
                        .map(str::to_string)
                        .unwrap_or_else(|| ev.stamp())
                ),
                at: ev.stamp(),
                text: crate::pystr::take_chars(&body, 160).to_string(),
                terms: knowledge::tokenize(&body).into_iter().collect(),
            };
            (ev, m, c)
        })
        .collect();
    let mut items: Vec<Value> = Vec::new();
    let (mut n_done, mut n_stale) = (0usize, 0usize);
    for lp in &loops {
        let num = lp.num().unwrap_or("").to_string();
        let opened = lp.stamp();
        let raw = lp.first_line().to_string();
        let backfilled = BACKFILL_TAIL.is_match(&raw);
        let text = BACKFILL_TAIL.replace_all(&raw, "").trim().to_string();
        // 開出這項的交接（同一分鐘、「剩下：#N …」）不算後來有人提到
        let last_mention = events
            .iter()
            .zip(&stamps)
            .zip(&mentioned_in)
            .filter(|((ev, at), m)| {
                at.as_str() > opened.as_str() && ev.num() != Some(num.as_str()) && m.contains(&num)
            })
            .map(|((_, at), _)| at.clone())
            .max();
        let ev_cands = ev_pre
            .iter()
            .filter(|(ev, m, c)| {
                c.at >= opened && ev.num() != Some(num.as_str()) && !m.contains(&num)
            })
            .map(|(_, _, c)| c);
        // commit 標題點名這項（做事的 commit）：直接當證據，不用比對文字
        let named: Vec<&Cand> = commits
            .iter()
            .filter(|c| c.at >= opened && is_work_commit(&c.text) && mentions(&c.text, &num))
            .collect();
        let cover_text = CoverText::new(&text);
        let mut scored: Vec<(f64, &Cand, Vec<String>)> = commits
            .iter()
            .filter(|c| c.at >= opened && !mentions(&c.text, &num))
            .chain(ev_cands)
            .map(|c| {
                let (pieces, ratio) = cover_text.cover(&c.terms);
                (ratio, c, pieces)
            })
            .filter(|(r, _, _)| *r >= REVIEW_EVIDENCE_COVER)
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| b.1.at.cmp(&a.1.at)));
        let best = scored.first().map(|x| x.0).unwrap_or(0.0);
        let age_days = chrono::NaiveDateTime::parse_from_str(&opened, "%Y-%m-%d %H:%M")
            .map(|o| (now - o).num_days())
            .unwrap_or(0);
        let d = loop_dict(lp, Some(&td));
        let overdue = d["overdue"] == true;
        let (verdict, why) = if !named.is_empty() {
            n_done += 1;
            (
                "maybe_done",
                format!(
                    "commit 標題點名了 {num}——看那個 commit，確認做完就 close（可能只做了一部分）"
                ),
            )
        } else if best >= REVIEW_DONE_COVER {
            n_done += 1;
            (
                "maybe_done",
                "之後有 commit／判斷蓋住了這項的大部分內容——看證據，確認做完就 close".to_string(),
            )
        } else if (age_days >= REVIEW_STALE_DAYS || overdue)
            && last_mention.is_none()
            && scored.is_empty()
        {
            n_stale += 1;
            let when = if overdue {
                format!("已逾期（due {}）", d["due"].as_str().unwrap_or(""))
            } else {
                format!("開了 {age_days} 天")
            };
            (
                "stale",
                format!(
                    "{when}，期間沒人提到、也沒有相關動靜——還要做嗎？要就重開一筆給新的 due，不要就 close"
                ),
            )
        } else if let Some(m) = &last_mention {
            ("active", format!("{m} 有人提到"))
        } else if best > 0.0 {
            (
                "active",
                format!(
                    "有相關動靜（最多蓋住 {}%），看起來還沒做完",
                    (best * 100.0).round()
                ),
            )
        } else {
            ("active", "還沒到期或開沒多久".to_string())
        };
        let evidence: Vec<Value> = named
            .iter()
            .map(|c| {
                json!({"kind": c.kind, "ref": c.reference, "at": c.at, "text": c.text,
                       "cover": 1.0, "shared": [num.clone()]})
            })
            .chain(scored.iter().map(|(r, c, pieces)| {
                json!({"kind": c.kind, "ref": c.reference, "at": c.at, "text": c.text,
                       "cover": knowledge::round2(*r), "shared": pieces})
            }))
            .take(3)
            .collect();
        items.push(json!({
            "id": num, "text": text, "opened": opened, "age_days": age_days,
            "due": d["due"], "overdue": d["overdue"], "backfilled": backfilled,
            "verdict": verdict, "why": why, "last_mention": last_mention, "evidence": evidence,
        }));
    }
    // 先列可能已處理、再列擱置太久，同類照原順序
    let rank = |v: &Value| match v["verdict"].as_str() {
        Some("maybe_done") => 0,
        Some("stale") => 1,
        _ => 2,
    };
    items.sort_by_key(rank);
    // 同一份結果給桌面 App（收件匣只做純檔案讀，不自己跑 git）：寫進 .state/review.json
    let _ = save_review_cache(spine_dir, &items, &now);
    Ok(json!({
        "ok": true, "scope": scope_json(&s),
        "counts": {"maybe_done": n_done, "stale": n_stale, "active": loops.len() - n_done - n_stale},
        "items": items,
    }))
}

/// 開場注入用的一行摘要：有可能已處理或逾期／擱置沒動靜的才回傳。
pub fn review_hint(spine_dir: &Path, t: &Target) -> Option<String> {
    let v = review_todos(spine_dir, t).ok()?;
    let done = v["counts"]["maybe_done"].as_u64().unwrap_or(0);
    let stale = v["counts"]["stale"].as_u64().unwrap_or(0);
    if done + stale == 0 {
        return None;
    }
    let ids = |verdict: &str| -> String {
        v["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|it| it["verdict"] == verdict)
            .filter_map(|it| it["id"].as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut parts = Vec::new();
    if done > 0 {
        parts.push(format!("可能已處理 {}", ids("maybe_done")));
    }
    if stale > 0 {
        parts.push(format!("逾期或擱置沒動靜 {}", ids("stale")));
    }
    Some(format!(
        "體檢：{}——`ctx todo review` 看證據；做完就 close，還要做就重開給新 due（先問使用者）",
        parts.join("；")
    ))
}

fn review_cache_path(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("review.json")
}

/// 把這次體檢的結果併進 `.state/review.json`（`{"items": {"#N": {verdict, why, evidence, at}}}`）。
/// 範圍外的項目保留上次的結果（別的組開場時會更新）。
fn save_review_cache(spine_dir: &Path, items: &[Value], now: &chrono::NaiveDateTime) -> Result<()> {
    let at = now.format("%Y-%m-%d %H:%M").to_string();
    crate::locking::update_json_locked(spine_dir, &review_cache_path(spine_dir), |v: &mut Value| {
        if !v.is_object() {
            *v = json!({"items": {}});
        }
        if !v["items"].is_object() {
            v["items"] = json!({});
        }
        let cache = v["items"].as_object_mut().expect("上面確保是物件");
        for it in items {
            if let Some(id) = it["id"].as_str().filter(|x| !x.is_empty()) {
                cache.insert(
                    id.to_string(),
                    json!({"verdict": it["verdict"], "why": it["why"],
                               "evidence": it["evidence"], "at": at}),
                );
            }
        }
    })
}

/// `.state/review.json` 的 items（讀不到或壞掉＝空）。
pub fn load_review_cache(spine_dir: &Path) -> Map<String, Value> {
    std::fs::read_to_string(review_cache_path(spine_dir))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("items").and_then(Value::as_object).cloned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// mention_nums（先算一次）跟 mentions（逐項搜尋）的判斷要一模一樣。
    #[test]
    fn work_commit_prefixes() {
        assert!(is_work_commit("fix(desktop)：新建牌組不再預勾（#72）"));
        assert!(is_work_commit("feat：x"));
        assert!(is_work_commit("perf(search)：y"));
        assert!(!is_work_commit("docs：#7 端到端走通"));
        assert!(!is_work_commit("chore(scripts)：e2e（#7）"));
        assert!(!is_work_commit("featherweight #3"));
        assert!(!is_work_commit("修好 #3"));
    }

    #[test]
    fn mention_nums_agrees_with_mentions() {
        let texts = [
            "做完 #12",
            "#123 不是 #12",
            "##12 也算",
            "見#12a",
            "#12、#7 與 #70",
            "沒有編號",
            "#",
            "結尾 #9",
            "中文＃12 全形不算",
        ];
        let nums = ["#12", "#123", "#7", "#70", "#9", "#1"];
        for t in texts {
            let set = mention_nums(t);
            for n in nums {
                assert_eq!(set.contains(n), mentions(t, n), "{t:?} {n}");
            }
        }
    }
}
