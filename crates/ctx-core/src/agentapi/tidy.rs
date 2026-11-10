//! Rust 才有（2026-11-10）：未結清理——把範圍內的未結分成「可以直接關」與「要留著」，並能一次關一批。
//!
//! 為什麼要有：真資料 230 項未結裡 226 項是 agent 開的、202 項的期限是自動加的 14 天，
//! 「逾期」因此不是訊號；人沒有一個地方能掃一眼、整批清掉，清單只增不減。
//! 分類全靠已有的資料（體檢的證據、到期日、文字），不叫模型。

use super::*;

/// 文字看起來是「等使用者決定／確認」的問題，不是一件要做的事。
static ASKS_USER: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(
        r"使用者決定|使用者確認|由使用者|請使用者|待使用者|要不要|是否同意|是否要|待確認|待決定",
    )
    .expect("寫死的 regex")
});

/// 這項未結是不是在等使用者決定（不是 agent 能動手的事）。
pub fn asks_user(text: &str) -> bool {
    ASKS_USER.is_match(text)
}

/// 一項未結歸到哪一類（順序＝畫面與 CLI 列出的順序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TidyBucket {
    /// 之後的 commit／判斷看起來已經做完（附證據）。
    Done,
    /// 逾期超過 [`STALE_OVERDUE_DAYS`] 天，或體檢說擱置太久。
    Shelved,
    /// 在等使用者決定：回答了就能關。
    Ask,
    /// 還活著的事。
    Live,
}

impl TidyBucket {
    pub const ALL: [TidyBucket; 4] = [
        TidyBucket::Done,
        TidyBucket::Shelved,
        TidyBucket::Ask,
        TidyBucket::Live,
    ];

    /// CLI／MCP 用的名字（跟 JSON 的 `bucket` 同一個字）。
    pub fn as_str(self) -> &'static str {
        match self {
            TidyBucket::Done => "done",
            TidyBucket::Shelved => "shelved",
            TidyBucket::Ask => "ask",
            TidyBucket::Live => "live",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.as_str() == s.trim())
    }

    pub fn label(self) -> &'static str {
        match self {
            TidyBucket::Done => "可能已做完",
            TidyBucket::Shelved => "擱置太久",
            TidyBucket::Ask => "等你決定",
            TidyBucket::Live => "還在進行",
        }
    }

    /// 整批關掉這一類時寫進事件的說明。
    pub fn close_note(self) -> &'static str {
        match self {
            TidyBucket::Done => "清理：之後的 commit／判斷已經做掉",
            TidyBucket::Shelved => "清理：擱置太久沒動靜（還要做再重開）",
            TidyBucket::Ask => "清理：不再追這個決定",
            TidyBucket::Live => "清理：不做了",
        }
    }
}

/// `tidy_todos`：範圍內的未結分四類。每項 `{id, text, bucket, why, due, overdue, opened, age_days, evidence}`
/// （evidence 最多一筆，給人一眼判斷）；`counts` 各類數量；`suggest_close` 是「可能已做完＋擱置太久」的編號。
pub fn tidy_todos(spine_dir: &Path, t: &Target) -> Result<Value> {
    let rv = review_todos(spine_dir, t)?;
    let td = today();
    let mut items: Vec<Value> = Vec::new();
    let mut counts = [0usize; 4];
    for it in rv["items"].as_array().into_iter().flatten() {
        let text = it["text"].as_str().unwrap_or("");
        let shelved = loop_stale(it["due"].as_str(), &td);
        let (bucket, why) = match it["verdict"].as_str() {
            Some("maybe_done") => (
                TidyBucket::Done,
                it["why"].as_str().unwrap_or("").to_string(),
            ),
            Some("stale") => (
                TidyBucket::Shelved,
                it["why"].as_str().unwrap_or("").to_string(),
            ),
            _ if shelved => (
                TidyBucket::Shelved,
                format!(
                    "逾期超過 {STALE_OVERDUE_DAYS} 天（due {}）還沒結",
                    it["due"].as_str().unwrap_or("")
                ),
            ),
            _ if ASKS_USER.is_match(text) => (
                TidyBucket::Ask,
                "這是在等你決定的問題：回答了就能關".to_string(),
            ),
            _ => (
                TidyBucket::Live,
                it["why"].as_str().unwrap_or("").to_string(),
            ),
        };
        counts[bucket as usize] += 1;
        // 證據只在「可能已做完」有意義（其他類的是低覆蓋率的湊巧，列出來反而誤導）
        let evidence: Vec<&Value> = if bucket == TidyBucket::Done {
            it["evidence"]
                .as_array()
                .into_iter()
                .flatten()
                .take(1)
                .collect()
        } else {
            Vec::new()
        };
        items.push(json!({
            "id": it["id"], "text": text, "bucket": bucket, "why": why,
            "due": it["due"], "overdue": it["overdue"], "opened": it["opened"],
            "age_days": it["age_days"], "evidence": evidence,
        }));
    }
    let rank = |v: &Value| {
        TidyBucket::ALL
            .iter()
            .position(|b| v["bucket"] == b.as_str())
            .unwrap_or(TidyBucket::ALL.len())
    };
    items.sort_by_key(rank);
    let suggest: Vec<&Value> = items
        .iter()
        .filter(|v| v["bucket"] == "done" || v["bucket"] == "shelved")
        .map(|v| &v["id"])
        .collect();
    Ok(json!({
        "ok": true, "scope": rv["scope"],
        "counts": {
            "done": counts[0], "shelved": counts[1], "ask": counts[2], "live": counts[3],
            "total": items.len(),
        },
        "suggest_close": suggest,
        "items": items,
    }))
}

/// 一次關一批未結。個別失敗（找不到、不在主場）不擋其他項：回 `closed`（`{id, closed}`）與 `failed`（`{id, error}`）。
/// 全部失敗才回錯誤。
pub fn close_todos<S: AsRef<str>>(
    spine_dir: &Path,
    ids: &[S],
    note: Option<&str>,
    cwd: Option<&Path>,
    cross_scope_ok: bool,
    source: &str,
) -> Result<Value> {
    let mut closed: Vec<Value> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut first_err: Option<Error> = None;
    for id in ids {
        let id = id.as_ref().trim();
        if id.is_empty() || !seen.insert(id.trim_start_matches('#').to_string()) {
            continue;
        }
        match close_todo(spine_dir, id, note, cwd, cross_scope_ok, source) {
            Ok(c) => closed.push(json!({"id": c.id, "closed": c.closed})),
            Err(e) => {
                failed.push(json!({"id": id, "error": e.to_string()}));
                first_err.get_or_insert(e);
            }
        }
    }
    if closed.is_empty() {
        if let Some(e) = first_err {
            return Err(e);
        }
    }
    Ok(json!({"ok": true, "closed": closed, "failed": failed}))
}

/// 把某幾類整批關掉（`ctx todo tidy --close shelved,done`、App 的「關掉這一類」）。回 [`close_todos`] 的形狀。
pub fn close_buckets(
    spine_dir: &Path,
    buckets: &[TidyBucket],
    t: &Target,
    cross_scope_ok: bool,
    source: &str,
) -> Result<Value> {
    let tidy = tidy_todos(spine_dir, t)?;
    let mut closed: Vec<Value> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    for b in buckets {
        let ids: Vec<&str> = tidy["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v["bucket"] == b.as_str())
            .filter_map(|v| v["id"].as_str())
            .collect();
        if ids.is_empty() {
            continue;
        }
        match close_todos(
            spine_dir,
            &ids,
            Some(b.close_note()),
            t.cwd.as_deref(),
            cross_scope_ok,
            source,
        ) {
            Ok(r) => {
                closed.extend(r["closed"].as_array().cloned().unwrap_or_default());
                failed.extend(r["failed"].as_array().cloned().unwrap_or_default());
            }
            Err(e) => failed.extend(
                ids.iter()
                    .map(|id| json!({"id": id, "error": e.to_string()})),
            ),
        }
    }
    Ok(json!({"ok": true, "closed": closed, "failed": failed}))
}
