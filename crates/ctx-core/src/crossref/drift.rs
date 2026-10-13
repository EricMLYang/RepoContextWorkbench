//! drift（引用過的上游檔之後又改了）＋建議 exports。

use super::*;

// ---------------------------------------------------------------- drift ＋ 建議 exports

/// 上游最新一則改動。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LatestChange {
    pub hash: String,
    pub date: String,
    pub subject: String,
}

/// [`drift`] 的一筆（鍵名、順序同 Python）。`changes=None`＝引用時的 commit 已不在歷史裡（附 note）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Drift {
    pub consumer: String,
    pub provider: String,
    pub rel: String,
    pub cited_commit: String,
    pub cited_at: String,
    pub changes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<LatestChange>,
}

/// 對應 `crossref.drift`：引用過的上游檔，在引用當時的 commit 之後又改過 → 下游可能過期。
/// 每個 (consumer, provider, rel) 只看最近一次引用。
pub fn drift(
    spine_dir: &Path,
    consumer: Option<&str>,
    provider: Option<&str>,
) -> Result<Vec<Drift>> {
    let mut latest: BTreeMap<(String, String, String), Cite> = BTreeMap::new();
    for c in load_cites(spine_dir) {
        if consumer.is_some_and(|x| !x.is_empty() && c.consumer != x)
            || provider.is_some_and(|x| !x.is_empty() && c.provider != x)
        {
            continue;
        }
        let key = (c.consumer.clone(), c.provider.clone(), c.rel.clone());
        if latest.get(&key).is_none_or(|old| c.at >= old.at) {
            latest.insert(key, c);
        }
    }
    let roots: HashMap<String, PathBuf> = registry::load(spine_dir)?
        .repos
        .iter()
        .map(|r| (r.id(), r.path()))
        .collect();
    let mut out = Vec::new();
    for ((cons, prov, rel), c) in latest {
        let Some(root) = roots.get(&prov).filter(|r| r.is_dir()) else {
            continue;
        };
        let Some(commit) = c.commit.clone().filter(|s| !s.is_empty()) else {
            continue;
        };
        let range = format!("{commit}..HEAD");
        let (rc, log) = git(
            root,
            &[
                "log",
                "--format=%h%x09%ad%x09%s",
                "--date=short",
                &range,
                "--",
                &rel,
            ],
            GIT_TIMEOUT,
        );
        if rc != 0 {
            out.push(Drift {
                consumer: cons,
                provider: prov,
                rel,
                cited_commit: commit,
                cited_at: c.at,
                changes: None,
                note: Some("引用時的 commit 已不在歷史裡（rebase？），無法比對".into()),
                latest: None,
            });
            continue;
        }
        let commits: Vec<Vec<&str>> = log
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.splitn(3, '\t').collect())
            .collect();
        if let Some(first) = commits.first() {
            let part = |i: usize| first.get(i).copied().unwrap_or("").to_string();
            out.push(Drift {
                consumer: cons,
                provider: prov,
                rel,
                cited_commit: commit,
                cited_at: c.at,
                changes: Some(commits.len()),
                note: None,
                latest: Some(LatestChange {
                    hash: part(0),
                    date: part(1),
                    subject: part(2),
                }),
            });
        }
    }
    Ok(out)
}

/// 對應 `crossref.drift_lines`。Python 預設 `limit=5`。
pub fn drift_lines(items: &[Drift], limit: usize) -> Vec<String> {
    let mut lines: Vec<String> = items
        .iter()
        .take(limit)
        .map(|d| match (&d.changes, &d.latest) {
            (Some(n), Some(l)) => format!(
                "- {} 引用的 {}:{}（{}）之後改了 {n} 次；最新 {}「{}」",
                d.consumer,
                d.provider,
                d.rel,
                take_chars(&d.cited_at, 10),
                l.date,
                l.subject
            ),
            _ => format!(
                "- {} 引用的 {}:{}：{}",
                d.consumer,
                d.provider,
                d.rel,
                d.note.as_deref().unwrap_or("")
            ),
        })
        .collect();
    if items.len() > limit {
        lines.push(format!(
            "- …另 {} 筆（ctx refs drift 看全部）",
            items.len() - limit
        ));
    }
    lines
}

/// [`suggest_exports`] 的一筆。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportSuggestion {
    pub repo: String,
    pub path: String,
    pub cites: usize,
    pub consumers: Vec<String>,
    pub command: String,
}

/// 對應 `crossref.suggest_exports`：常被別的 repo 引用、卻沒被任何 export 涵蓋的目錄 → 建議宣告成 export。
/// depth＝取引用路徑的前幾層目錄當候選。Python 預設 `min_count=3, depth=1`。
pub fn suggest_exports(
    spine_dir: &Path,
    min_count: usize,
    depth: usize,
) -> Result<Vec<ExportSuggestion>> {
    let data = registry::load(spine_dir)?;
    let exp: HashMap<String, Vec<String>> = data
        .repos
        .iter()
        .map(|r| {
            (
                r.id(),
                registry::exports_of(r)
                    .into_iter()
                    .map(|(_, v)| v.path.trim_end_matches('/').to_string())
                    .collect(),
            )
        })
        .collect();
    // Counter＋most_common：依次數降冪，同數照首次出現順序
    let mut counts: Vec<((String, String), usize, BTreeSet<String>)> = Vec::new();
    for c in load_cites(spine_dir) {
        let covered = exp.get(&c.provider).is_some_and(|ps| {
            ps.iter().any(|p| {
                p == "." || p.is_empty() || c.rel == *p || c.rel.starts_with(&format!("{p}/"))
            })
        });
        if covered {
            continue;
        }
        let parts: Vec<&str> = c.rel.split('/').collect();
        let cand = if parts.len() > depth {
            parts[..depth].join("/")
        } else if parts.len() > 1 {
            parts[0].to_string()
        } else {
            c.rel.clone()
        };
        let key = (c.provider.clone(), cand);
        match counts.iter_mut().find(|x| x.0 == key) {
            Some(x) => {
                x.1 += 1;
                x.2.insert(c.consumer.clone());
            }
            None => counts.push((key, 1, BTreeSet::from([c.consumer.clone()]))),
        }
    }
    counts.sort_by_key(|x| std::cmp::Reverse(x.1));
    Ok(counts
        .into_iter()
        .filter(|x| x.1 >= min_count)
        .map(|((prov, cand), n, cons)| ExportSuggestion {
            command: format!("ctx registry export {prov} <名稱> {cand}"),
            repo: prov,
            path: cand,
            cites: n,
            consumers: cons.into_iter().collect(),
        })
        .collect())
}

/// `ctx refs check`（CLI 與 MCP 管理層共用的資料，2026-10-13 從兩個入口搬來）。
#[derive(Debug, Clone, Serialize)]
pub struct RefsCheck {
    /// agent 文件裡失效的跨 repo 路徑。
    pub broken: Vec<BrokenRef>,
    /// 引用過、之後又改了的上游檔。
    pub drift: Vec<Drift>,
    /// 常被引用、建議宣告成 export 的目錄。
    pub suggest: Vec<ExportSuggestion>,
}

/// 跨 repo 參考體檢：`repo` 只看這個 repo 的文件與它引用的上游；`min` 被引用幾次以上才建議 export。
pub fn check(spine_dir: &Path, repo: Option<&str>, min: usize) -> Result<RefsCheck> {
    let only: Option<Vec<String>> = repo.map(|r| vec![r.to_string()]);
    Ok(RefsCheck {
        broken: lint_doc_refs(spine_dir, only.as_deref())?,
        drift: drift(spine_dir, repo, None)?,
        suggest: suggest_exports(spine_dir, min, 1)?,
    })
}
