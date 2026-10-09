//! 相關 repo 建議（2026-10-10，ideation F-33；Rust 才有，Python 沒有對應模組）。
//!
//! 關係層（`registry relate`）要人手動設，真資料 15 個 repo 一條關係都沒設——鄰居地圖、跨 repo 參考、
//! 組情境卡都因此空著。這裡從已經有的資料**建議**，人確認後才 relate：
//!
//! - **直接提到**（主要訊號）：A 的 md 寫到 B 的 repo id 或目錄名（Obsidian 的「未連結提及」）。
//!   真資料：PM 中樞 repo ↔ 各專案 repo 動輒 40 份以上——正是「PM 頭管各專案」的實況。
//! - **引用紀錄**：A 的會話 read_from 過 B（`refs/cites.jsonl`）。
//! - **共同主題詞**（只當佐證、不單獨成立）：只出現在這兩個 repo 的詞，過反芻同一套濾網
//!   （像一個詞、不是數字日期）。單獨用會被大型筆記庫（真資料一個近 3,000 份的卡片 repo）壓垮——它跟誰都共享上千個詞。
//!
//! 刻意不用「同一段時間一起 commit」：使用者每天在多個 repo 之間切換，這個訊號太吵。
//! 結果寫進 `.state/relation_suggest.json`（反芻每天跑時更新），開場只讀檔。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::knowledge::{self, Doc};
use crate::pack::RepoRef;
use crate::ruminate::{self, CharStats};
use crate::{crossref, registry, Result};

/// 至少幾份文件提到對方才建議。
pub const MIN_MENTIONS: usize = 3;
/// 至少幾筆引用才建議（沒有提到時）。
pub const MIN_CITES: usize = 2;
/// 最多列幾筆建議。
pub const MAX_SUGGESTIONS: usize = 10;

/// 一筆建議（兩個 repo 依 id 排序，`a` < `b`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub a: String,
    pub b: String,
    /// a 的文件提到 b 的份數
    pub a_mentions_b: usize,
    /// b 的文件提到 a 的份數
    pub b_mentions_a: usize,
    /// 引用紀錄（兩個方向合計）
    pub cites: usize,
    /// 提到對方的檔（各方向最多兩份，`repo:相對路徑`）
    pub files: Vec<String>,
    /// 只出現在這兩個 repo 的主題詞（佐證）
    pub terms: Vec<String>,
    pub score: f64,
    /// 推測的方向與種類：(from, to, kind)。目前只推「中樞」：一個 repo 單向提到很多 repo＝它是 PM（pm-of）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<(String, String, String)>,
}

/// 一個 repo 當主要提到者的建議數達到這麼多，就推測它是這些 repo 的 PM（pm-of）。
pub const HUB_MIN: usize = 4;

impl Suggestion {
    /// 一句理由（人話）。
    pub fn reason(&self) -> String {
        let mut parts = Vec::new();
        if let Some((from, to, kind)) = &self.hint {
            parts.push(format!("推測 {from} {kind} {to}"));
        }
        let m = self.a_mentions_b + self.b_mentions_a;
        if m > 0 {
            parts.push(format!(
                "{m} 份文件互相提到（{} 提到 {} {} 份、反向 {} 份）",
                self.a, self.b, self.a_mentions_b, self.b_mentions_a
            ));
        }
        if self.cites > 0 {
            parts.push(format!("引用 {} 次", self.cites));
        }
        if !self.terms.is_empty() {
            parts.push(format!("共同主題詞：{}", self.terms.join("、")));
        }
        parts.join("；")
    }
}

fn cache_path(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("relation_suggest.json")
}

/// 讀上次算好的建議（沒有或壞掉＝空）。
pub fn load(spine_dir: &Path) -> Vec<Suggestion> {
    std::fs::read_to_string(cache_path(spine_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 算建議並寫進快取。已經有關係的（任一方向）不列。
pub fn compute(spine_dir: &Path) -> Result<Vec<Suggestion>> {
    let reg = registry::load(spine_dir)?;
    let refs: Vec<RepoRef> = reg.repos.iter().map(RepoRef::from).collect();
    let docs = knowledge::build_docs(&refs, Some(spine_dir));
    let related: HashSet<(String, String)> = registry::relations(spine_dir, None)?
        .into_iter()
        .map(|r| pair(&r.from, &r.to))
        .collect();
    // 每個 repo 會被認出的名字：id，以及目錄名（id 跟目錄名常不同，例 id `pm` 的目錄叫 `pm_work`）
    let names: Vec<(String, Vec<String>)> = reg
        .repos
        .iter()
        .map(|r| {
            let mut ns = vec![r.id().to_lowercase()];
            if let Some(dir) = r
                .path()
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
            {
                if !ns.contains(&dir) {
                    ns.push(dir);
                }
            }
            (r.id(), ns)
        })
        .collect();
    let suggestions = suggest(&docs, &names, &crossref::load_cites(spine_dir), &related);
    if let Some(parent) = cache_path(spine_dir).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = cache_path(spine_dir).with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string(&suggestions)?)?;
    std::fs::rename(&tmp, cache_path(spine_dir))?;
    Ok(suggestions)
}

fn pair(x: &str, y: &str) -> (String, String) {
    if x <= y {
        (x.to_string(), y.to_string())
    } else {
        (y.to_string(), x.to_string())
    }
}

/// 純函數：文件、各 repo 的名字、引用紀錄、已有關係 → 建議（分數高的在前）。
pub fn suggest(
    docs: &[Doc],
    names: &[(String, Vec<String>)],
    cites: &[crossref::Cite],
    related: &HashSet<(String, String)>,
) -> Vec<Suggestion> {
    // (提到的一方, 被提到的一方) → 檔
    let mut mentions: HashMap<(String, String), Vec<String>> = HashMap::new();
    for d in docs {
        for (id, ns) in names {
            if *id != d.repo && ns.iter().any(|n| d.tf.contains_key(n)) {
                mentions
                    .entry((d.repo.clone(), id.clone()))
                    .or_default()
                    .push(format!("{}:{}", d.repo, d.rel));
            }
        }
    }
    let mut cite_n: HashMap<(String, String), usize> = HashMap::new();
    for c in cites
        .iter()
        .filter(|c| c.consumer != c.provider && !c.consumer.is_empty())
    {
        *cite_n.entry(pair(&c.consumer, &c.provider)).or_default() += 1;
    }
    let candidates: HashSet<(String, String)> = mentions
        .keys()
        .map(|(x, y)| pair(x, y))
        .chain(cite_n.keys().cloned())
        .filter(|p| !related.contains(p))
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    let exclusive = exclusive_terms(docs, &candidates);
    let mut out: Vec<Suggestion> = candidates
        .into_iter()
        .filter_map(|(a, b)| {
            let ab = mentions.get(&(a.clone(), b.clone()));
            let ba = mentions.get(&(b.clone(), a.clone()));
            let (n_ab, n_ba) = (ab.map_or(0, Vec::len), ba.map_or(0, Vec::len));
            let cites = cite_n.get(&(a.clone(), b.clone())).copied().unwrap_or(0);
            if n_ab + n_ba < MIN_MENTIONS && cites < MIN_CITES {
                return None;
            }
            // 舉證優先挑一般文件（點開頭目錄是工具設定：.agents/prompts…）
            let pick = |v: &Vec<String>| -> Vec<String> {
                let mut v = v.clone();
                v.sort_by_key(|f| {
                    f.split(':')
                        .nth(1)
                        .is_some_and(|r| r.starts_with('.') || r.contains("/."))
                });
                v.into_iter().take(2).collect()
            };
            let files: Vec<String> = ab.into_iter().chain(ba).flat_map(pick).collect();
            let terms = exclusive
                .get(&(a.clone(), b.clone()))
                .map(|t| t.iter().take(6).cloned().collect())
                .unwrap_or_default();
            let score = (n_ab + n_ba) as f64 + 3.0 * cites as f64;
            Some(Suggestion {
                a,
                b,
                a_mentions_b: n_ab,
                b_mentions_a: n_ba,
                cites,
                files,
                terms,
                score,
                hint: None,
            })
        })
        .collect();
    // 中樞：當主要提到者（提到對方的份數 ≥ 反向的兩倍）的建議數 ≥ HUB_MIN
    let dominant = |s: &Suggestion| -> Option<(String, String)> {
        if s.a_mentions_b >= 2 * s.b_mentions_a.max(1) {
            Some((s.a.clone(), s.b.clone()))
        } else if s.b_mentions_a >= 2 * s.a_mentions_b.max(1) {
            Some((s.b.clone(), s.a.clone()))
        } else {
            None
        }
    };
    let mut hub_n: HashMap<String, usize> = HashMap::new();
    for s in &out {
        if let Some((from, _)) = dominant(s) {
            *hub_n.entry(from).or_default() += 1;
        }
    }
    for s in &mut out {
        if let Some((from, to)) = dominant(s) {
            if hub_n.get(&from).copied().unwrap_or(0) >= HUB_MIN {
                s.hint = Some((from, to, "pm-of".into()));
            }
        }
    }
    out.sort_by(|x, y| {
        y.score
            .total_cmp(&x.score)
            .then_with(|| (&x.a, &x.b).cmp(&(&y.a, &y.b)))
    });
    out.truncate(MAX_SUGGESTIONS);
    out
}

/// 只出現在某一對 repo（且各自至少兩份文件）的主題詞，依兩邊文件數排序。只算候選的那幾對。
fn exclusive_terms(
    docs: &[Doc],
    candidates: &HashSet<(String, String)>,
) -> HashMap<(String, String), Vec<String>> {
    let stats = CharStats::from_docs(docs.iter());
    // 詞 → repo → 份數
    let mut df: HashMap<&str, BTreeMap<&str, usize>> = HashMap::new();
    for d in docs {
        for t in d.tf.keys() {
            *df.entry(t.as_str())
                .or_default()
                .entry(d.repo.as_str())
                .or_default() += 1;
        }
    }
    let mut out: HashMap<(String, String), Vec<(usize, String)>> = HashMap::new();
    for (t, per) in df {
        if per.len() != 2 || !ruminate::meaningful(t) || !stats.word_like(t) {
            continue;
        }
        let mut it = per.iter();
        let (Some((a, na)), Some((b, nb))) = (it.next(), it.next()) else {
            continue;
        };
        if *na < 2 || *nb < 2 || t.chars().count() < 2 {
            continue;
        }
        let p = pair(a, b);
        if candidates.contains(&p) {
            out.entry(p).or_default().push((na + nb, t.to_string()));
        }
    }
    out.into_iter()
        .map(|(p, mut v)| {
            v.sort_by(|x, y| y.0.cmp(&x.0).then_with(|| x.1.cmp(&y.1)));
            (p, v.into_iter().map(|(_, t)| t).collect())
        })
        .collect()
}
