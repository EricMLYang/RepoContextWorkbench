//! 程式碼定位（L1 的 code 半邊）。

use super::*;

// ---------------------------------------------------------------- 程式碼定位（L1 的 code 半邊）

static ASCII_TERM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_\-\.]{2,}").expect("寫死的 regex"));
static CJK_TERM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\x{3400}-\x{9FFF}\x{F900}-\x{FAFF}]{2,8}").expect("寫死的 regex")
});
const CODE_STOP: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "what", "how", "where", "which", "does",
    "into", "about", "when", "use", "are",
];

/// 對應 `crossref.query_terms`：查詢 → git grep 用的字面詞：英數識別字（≥3 字）＋中文詞段（2～8 字）。
/// Python 預設 `limit=8`。
pub fn query_terms(query: &str, limit: usize) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for m in ASCII_TERM.find_iter(query) {
        let t = m.as_str();
        if !CODE_STOP.contains(&t.to_lowercase().as_str()) && !terms.iter().any(|x| x == t) {
            terms.push(t.to_string());
        }
    }
    for m in CJK_TERM.find_iter(query) {
        let t = m.as_str();
        if !terms.iter().any(|x| x == t) {
            terms.push(t.to_string());
        }
    }
    terms.truncate(limit);
    terms
}

/// 程式碼命中一份檔（對應 `code_search` 回傳的 dict）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CodeHit {
    pub repo: String,
    pub file: String,
    pub path: String,
    pub score: f64,
    pub why: Vec<String>,
    pub snippets: Vec<Snippet>,
}

/// 對應 `crossref.code_search`：程式碼不建索引（改得快、索引跟不上），直接 `git grep -F`，排除 *.md
/// （md 走 BM25）。分數＝命中幾個不同的查詢詞，同分看命中行數。Python 預設 `limit=8, per_repo=40`。
pub fn code_search(entries: &[Repo], query: &str, limit: usize, per_repo: usize) -> Vec<CodeHit> {
    let terms = query_terms(query, 8);
    if terms.is_empty() {
        return Vec::new();
    }
    let lows: Vec<String> = terms.iter().map(|t| t.to_lowercase()).collect();
    let mut args: Vec<&str> = vec!["grep", "-n", "-I", "-i", "-F", "--max-count=20"];
    for t in &terms {
        args.push("-e");
        args.push(t);
    }
    args.extend([
        "--",
        ".",
        ":(exclude)*.md",
        ":(exclude)*.lock",
        ":(exclude)*-lock.json",
        ":(exclude)*.min.js",
        ":(exclude)*.svg",
    ]);
    // 各 repo 的 git grep 同時跑（Rust 才有：依序跑時 15 個 repo 要 0.6 秒以上）；結果照 repo 順序合併
    let outs: Vec<Option<(i32, String)>> = std::thread::scope(|sc| {
        let hs: Vec<_> = entries
            .iter()
            .map(|e| {
                let args = &args;
                sc.spawn(move || {
                    let root = e.path();
                    (root.is_dir() && e.kind().as_deref() != Some("external"))
                        .then(|| git(&root, args, Duration::from_secs(10)))
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().ok().flatten()).collect()
    });
    let mut rows: Vec<CodeHit> = Vec::new();
    for (e, res) in entries.iter().zip(outs) {
        let Some((rc, out)) = res else {
            continue;
        };
        if rc != 0 || out.is_empty() {
            continue;
        }
        let root = e.path();
        let mut per_file: Vec<(String, Vec<(usize, String)>)> = Vec::new();
        let mut idx: HashMap<String, usize> = HashMap::new();
        for ln in out.lines().take(2000) {
            let parts: Vec<&str> = ln.splitn(3, ':').collect();
            if parts.len() != 3
                || parts[1].is_empty()
                || !parts[1].chars().all(|c| c.is_ascii_digit())
            {
                continue;
            }
            let Ok(n) = parts[1].parse::<usize>() else {
                continue;
            };
            let i = *idx.entry(parts[0].to_string()).or_insert_with(|| {
                per_file.push((parts[0].to_string(), Vec::new()));
                per_file.len() - 1
            });
            per_file[i].1.push((n, parts[2].to_string()));
        }
        for (f, hits) in per_file.into_iter().take(per_repo) {
            let low = hits
                .iter()
                .map(|h| h.1.to_lowercase())
                .collect::<Vec<_>>()
                .join(" ");
            let matched: Vec<&String> = terms
                .iter()
                .zip(&lows)
                .filter(|(_, l)| low.contains(l.as_str()))
                .map(|(t, _)| t)
                .collect();
            let hit_count = |h: &(usize, String)| {
                let hl = h.1.to_lowercase();
                lows.iter().filter(|l| hl.contains(l.as_str())).count()
            };
            let mut best: Vec<&(usize, String)> = hits.iter().collect();
            best.sort_by_key(|h| std::cmp::Reverse(hit_count(h)));
            best.truncate(2);
            best.sort();
            rows.push(CodeHit {
                repo: e.id(),
                file: f.clone(),
                path: root.join(&f).to_string_lossy().into_owned(),
                score: matched.len() as f64 + hits.len().min(20) as f64 / 100.0,
                why: vec![format!(
                    "程式碼含 {}",
                    matched
                        .iter()
                        .take(4)
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join("、")
                )],
                snippets: best
                    .iter()
                    .map(|(n, t)| Snippet {
                        line: *n,
                        text: take_chars(t.trim(), 160).to_string(),
                    })
                    .collect(),
            });
        }
    }
    rows.sort_by(|a, b| b.score.total_cmp(&a.score));
    for r in &mut rows {
        r.score = crate::knowledge::round2(r.score);
    }
    rows.truncate(limit);
    rows
}
