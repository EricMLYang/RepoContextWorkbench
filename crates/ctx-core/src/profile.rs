//! repo 名片（2026-10-13，Rust 才有）——一句話介紹、協作角色、搜尋關鍵字。給兩個地方用：
//! - **組員名冊**：agent 開場就知道同組有誰、在哪（絕對路徑）、扮演什麼角色、現在有沒有會話在跑。
//! - **先找 repo 的搜尋**：repo 多了以後，先回答「是哪個 repo」，再往檔案找。
//!
//! 最小打擾（VISION §3）：**只讀使用者的 repo，不寫進去**。名片欄位存在 registry（`description`／`role`／
//! `keywords`）；沒填介紹時從 repo 現有的 README／AGENTS.md 截第一句當「推測」，不另外存。
//! 名冊與排名都是每次現算，組員增減不用改任何東西。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;

use crate::fsutil::read_text_lossy;
use crate::knowledge::{round2, tokenize};
use crate::pystr::take_chars;
use crate::registry::Repo;

/// 推測介紹最多幾個字元。
const DESC_MAX: usize = 120;
/// 讀 README 時只看前段（一句話介紹不會在後面）。
const README_HEAD: usize = 16_000;
/// 依序找這些檔的第一句。
const README_NAMES: [&str; 5] = ["README.md", "readme.md", "Readme.md", "README", "AGENTS.md"];

/// 介紹從哪來。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DescSource {
    /// 人或 agent 寫在 registry 的。
    Registry,
    /// 沒填，從 repo 的 README／AGENTS.md 截的（推測）。
    Readme,
}

/// 一個 repo 的名片。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepoProfile {
    pub id: String,
    /// 展開後的絕對路徑。
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description_source: Option<DescSource>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// 組出名片：registry 有介紹就用，沒有就從 repo 截一句（只讀）。
pub fn profile(r: &Repo) -> RepoProfile {
    let path = r.path();
    let (description, description_source) = match r.description() {
        Some(d) => (Some(d.trim().to_string()), Some(DescSource::Registry)),
        None => match readme_line(&path) {
            Some(d) => (Some(d), Some(DescSource::Readme)),
            None => (None, None),
        },
    };
    RepoProfile {
        id: r.id(),
        path: path.to_string_lossy().into_owned(),
        role: r.role().map(|s| s.trim().to_string()),
        description,
        description_source,
        keywords: r.keywords(),
        tags: r.tags(),
    }
}

/// repo 的第一句說明：README（或 AGENTS.md）裡第一行像句子的文字，去掉 Markdown 記號、截到 [`DESC_MAX`]。
/// 標題、徽章、HTML、程式碼區塊、表格、清單、frontmatter 都跳過。找不到＝None。
pub fn readme_line(dir: &Path) -> Option<String> {
    README_NAMES.iter().find_map(|name| {
        let text = read_text_lossy(&dir.join(name)).ok()?;
        first_sentence(take_chars(&text, README_HEAD))
    })
}

fn first_sentence(text: &str) -> Option<String> {
    let mut lines = text.lines().peekable();
    if lines.peek().is_some_and(|l| l.trim() == "---") {
        lines.next();
        for l in lines.by_ref() {
            if l.trim() == "---" {
                break;
            }
        }
    }
    let mut fence = false;
    for raw in lines {
        let l = raw.trim();
        if l.starts_with("```") || l.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if fence || l.is_empty() {
            continue;
        }
        let skip = [
            "#", "<", "![", "[![", "|", "---", "===", "- ", "* ", "+ ", "@",
        ];
        if skip.iter().any(|p| l.starts_with(p))
            || l.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            continue;
        }
        let cleaned = clean_md(l.trim_start_matches('>').trim());
        if cleaned.chars().count() < 4 {
            continue;
        }
        return Some(if cleaned.chars().count() > DESC_MAX {
            format!("{}…", take_chars(&cleaned, DESC_MAX))
        } else {
            cleaned
        });
    }
    None
}

/// 去掉粗體／斜體／行內程式碼記號，連結只留文字。
fn clean_md(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('[') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after
            .find("](")
            .and_then(|j| after[j..].find(')').map(|k| (j, j + k)))
        {
            Some((j, end)) => {
                out.push_str(&after[..j]);
                rest = &after[end + 1..];
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "")
        .replace("__", "")
        .replace('`', "")
        .trim()
        .to_string()
}

/// 依名片排出跟查詢最相關的 repo。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProfileHit {
    pub repo: String,
    pub score: f64,
    /// 哪些欄位命中（「介紹」「角色」「關鍵字」「tag」「名稱」）。
    pub why: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// 名片欄位與權重：結構化、人挑過的詞（名稱、角色、關鍵字）比介紹句準；推測的介紹再打折。
fn fields(p: &RepoProfile) -> Vec<(&'static str, f64, HashSet<String>)> {
    let set = |s: &str| tokenize(s).into_iter().collect::<HashSet<_>>();
    let desc_label = match p.description_source {
        Some(DescSource::Readme) => "介紹（推測）",
        _ => "介紹",
    };
    let desc_w = match p.description_source {
        Some(DescSource::Readme) => 0.5,
        _ => 1.0,
    };
    vec![
        ("名稱", 2.0, set(&p.id.replace(['_', '-'], " "))),
        ("角色", 2.0, set(p.role.as_deref().unwrap_or(""))),
        ("關鍵字", 2.0, set(&p.keywords.join(" "))),
        ("標籤", 1.5, set(&p.tags.join(" "))),
        (
            desc_label,
            desc_w,
            set(p.description.as_deref().unwrap_or("")),
        ),
    ]
}

/// 對 `query` 排名片：每個查詢詞 × 命中欄位的權重 × idf（越少 repo 有的詞越有鑑別力）。分數 0 的不回。
pub fn rank(profiles: &[RepoProfile], query: &str, limit: usize) -> Vec<ProfileHit> {
    let q: HashSet<String> = tokenize(query).into_iter().collect();
    if q.is_empty() || profiles.is_empty() {
        return Vec::new();
    }
    let per: Vec<_> = profiles.iter().map(fields).collect();
    let mut df: HashMap<&str, usize> = HashMap::new();
    for f in &per {
        for t in &q {
            if f.iter().any(|(_, _, s)| s.contains(t)) {
                *df.entry(t.as_str()).or_default() += 1;
            }
        }
    }
    let n = profiles.len() as f64;
    let mut hits: Vec<ProfileHit> = profiles
        .iter()
        .zip(&per)
        .filter_map(|(p, f)| {
            let mut score = 0.0;
            let mut why: Vec<String> = Vec::new();
            for t in &q {
                let Some(&d) = df.get(t.as_str()) else {
                    continue;
                };
                let idf = (1.0 + n / d as f64).ln();
                for (label, w, s) in f {
                    if s.contains(t) {
                        score += w * idf;
                        if !why.iter().any(|x| x == label) {
                            why.push((*label).to_string());
                        }
                    }
                }
            }
            (score > 0.0).then(|| ProfileHit {
                repo: p.id.clone(),
                score: round2(score),
                why: why.into_iter().map(|w| format!("{w}命中")).collect(),
                role: p.role.clone(),
                description: p.description.clone(),
            })
        })
        .collect();
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    hits.truncate(limit);
    hits
}

/// 組員名冊的一列：名片＋現在有幾個 agent 會話開在這個 repo（Claude Code hooks 記的開場標記）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RosterEntry {
    #[serde(flatten)]
    pub profile: RepoProfile,
    /// 開著的 agent 會話數（含呼叫者自己）。
    pub sessions: usize,
    /// 呼叫者所在的 repo。
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub you: bool,
}

/// 範圍內每個 repo 一列（照 registry 順序）。`here`＝呼叫者所在 repo id。
pub fn roster(spine_dir: &Path, entries: &[Repo], here: Option<&str>) -> Vec<RosterEntry> {
    let open = crate::hooks::open_sessions_by_repo(spine_dir);
    entries
        .iter()
        .map(|r| {
            let profile = profile(r);
            RosterEntry {
                sessions: open.get(&profile.id).copied().unwrap_or(0),
                you: here == Some(profile.id.as_str()),
                profile,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sentence_skips_markup_and_cleans() {
        let md = "---\ntitle: x\n---\n# 標題\n\n[![ci](b.svg)](x)\n<p>hi</p>\n\
                  > **這是什麼**：把一群 [repo](https://x) 分成組，`ctx` 管脈絡。\n\n更多";
        assert_eq!(
            first_sentence(md).as_deref(),
            Some("這是什麼：把一群 repo 分成組，ctx 管脈絡。")
        );
        assert_eq!(
            first_sentence("# 只有標題\n\n- 清單\n```\ncode\n```\n"),
            None
        );
        let long = "字".repeat(200);
        assert_eq!(first_sentence(&long).unwrap().chars().count(), DESC_MAX + 1);
    }

    fn prof(id: &str, role: Option<&str>, desc: Option<&str>, src: DescSource) -> RepoProfile {
        RepoProfile {
            id: id.into(),
            path: format!("/w/{id}"),
            role: role.map(String::from),
            description: desc.map(String::from),
            description_source: desc.map(|_| src),
            keywords: Vec::new(),
            tags: Vec::new(),
        }
    }

    #[test]
    fn rank_prefers_structured_fields_and_says_why() {
        let ps = vec![
            prof(
                "card_notes",
                Some("知識庫"),
                Some("書摘與主題閱讀"),
                DescSource::Registry,
            ),
            prof("impl", None, Some("快取與書摘的實作"), DescSource::Readme),
            prof("other", None, None, DescSource::Registry),
        ];
        let hits = rank(&ps, "書摘 知識庫", 5);
        assert_eq!(hits[0].repo, "card_notes");
        assert!(hits[0].why.contains(&"角色命中".to_string()));
        assert!(hits[0].why.contains(&"介紹命中".to_string()));
        assert_eq!(hits[1].repo, "impl");
        assert_eq!(hits[1].why, vec!["介紹（推測）命中".to_string()]);
        assert_eq!(hits.len(), 2);
        assert!(rank(&ps, "   ", 5).is_empty());
        // 名稱的底線拆開也找得到
        assert_eq!(rank(&ps, "notes", 5)[0].repo, "card_notes");
    }
}
