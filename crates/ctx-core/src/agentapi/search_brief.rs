//! Rust 才有（2026-11-14，F-126 漸進揭露）：`search_knowledge` 的精簡模式——先回答「誰有、為什麼」，一步只給一小塊。
//!
//! 為什麼要有：完整結果一次約 11 KB、五段並列，`files` 與 `repos` 還常指向不同 repo，agent 只看前面的 `files` 就被帶去錯的地方；
//! 找檔案本身 agent 自己 grep 做得更好。精簡模式只給檔案裡沒有的：最該去的 repo（為什麼是它）、第二三名、
//! 最多 3 個起點（保證含那個 repo 的）、最多 2 筆相關判斷，以及下一步叫什麼（`repo_entry` → 自己讀）。
//! 序列化後 ≤ [`BRIEF_MAX_CHARS`] 字元。不跑程式碼搜尋。
//!
//! 「最該去的 repo」＝四個訊號加總（各自除以該訊號的最大值，所以都在 0～1）：
//! - 命中檔的分數加總（`repos`，前 50 名命中檔照 repo 加總）——「很多檔都在講」
//! - 最高分那份檔（`RepoHit::best_score`）——「有一份檔正中」；跟上一項等重，免得中等分數的檔很多的 repo 一直贏
//! - 名片命中（`repo_profiles`，×0.5）——人寫的介紹／角色／關鍵字，當破平手的訊號
//! - 相關判斷（×0.3，判斷內容在講哪個 repo 就算給它，見 `about_repo`）
//!
//! 只有名片或判斷、沒有任何命中檔的 repo 也能當候選；全部都沒有＝`answer_repo` 是 None，不硬湊。

use super::*;
use crate::knowledge::{RepoHit, SearchResult};
use crate::profile::ProfileHit;

/// 精簡模式序列化後最多幾個字元。
pub const BRIEF_MAX_CHARS: usize = 1500;
/// 精簡模式最多幾個起點、幾筆判斷、幾個其他候選。
const BRIEF_REFS: usize = 3;
const BRIEF_DECISIONS: usize = 2;
const BRIEF_ALSO: usize = 2;
/// 精簡模式內部取幾份命中檔（只用來挑起點；不全回）。
const BRIEF_FILES: usize = 20;

/// 搜尋的原料：完整模式與精簡模式共用。
pub(super) struct SearchParts {
    pub label: String,
    pub res: SearchResult,
    pub profiles: Vec<ProfileHit>,
    pub decisions: Vec<Value>,
    pub code: Option<Vec<crossref::CodeHit>>,
    /// 跟所在 repo 有關係的 repo → 理由（`crossref::search_boosts`）
    pub related: Vec<(String, String)>,
    pub entries: Vec<Repo>,
}

pub(super) fn search_parts(
    spine_dir: &Path,
    query: &str,
    t: &Target,
    opts: &SearchOpts,
) -> Result<SearchParts> {
    if query.trim().is_empty() {
        return Err(agent_error(
            crate::RejectCode::BadRequest,
            "query 不能是空的",
            Some("給一段文字、一個想法或一段卡片內容。"),
        ));
    }
    let w = where_am_i(spine_dir, t.cwd.as_deref())?;
    let explicit = t.group.as_deref().is_some_and(|g| !g.is_empty())
        || t.repos.as_ref().is_some_and(|r| !r.is_empty());
    let (label, entries) = if opts.everywhere && !explicit {
        ("全部".to_string(), registry::load(spine_dir)?.repos)
    } else {
        let s = scope_of(spine_dir, &w, t)?;
        let e = entries_of(spine_dir, &s)?;
        (s.label, e)
    };
    let refs: Vec<RepoRef> = entries.iter().map(RepoRef::from).collect();
    let boosts = crossref::search_boosts(spine_dir, w.repo_id())?;
    // 程式碼搜尋跟知識搜尋互不相干：平行跑（Rust 才有，2026-10-14；真資料約 395ms＋220ms → 約 400ms）
    let (res, code) = std::thread::scope(|sc| {
        let code = opts.code.then(|| {
            sc.spawn(|| crossref::code_search(&entries, query, (opts.limit / 2).max(3), 40))
        });
        let res = knowledge::search(
            &refs,
            query,
            opts.limit,
            Some(spine_dir),
            &opts.exclude_paths,
            &boosts,
        );
        // code_search 不會 panic；萬一有，就在這裡重跑一次（跟原本循序時一樣把 panic 帶上來）
        let code = code.map(|h| {
            h.join().unwrap_or_else(|_| {
                crossref::code_search(&entries, query, (opts.limit / 2).max(3), 40)
            })
        });
        (res, code)
    });
    let profiles = profile_hits(&entries, query);
    let decisions = search_decisions(spine_dir, query, &w, t, opts.everywhere && !explicit, 5)?;
    let related = boosts
        .repos
        .iter()
        .map(|(k, (_, why))| (k.clone(), why.clone()))
        .collect();
    Ok(SearchParts {
        label,
        res,
        profiles,
        decisions,
        code,
        related,
        entries,
    })
}

/// 最該去的 repo。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnswerRepo {
    pub repo: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// 為什麼是它（命中幾份檔、名片命中、相關判斷、跟你所在 repo 的關係）
    pub why: Vec<String>,
}

/// 第二、三名。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlsoRepo {
    pub repo: String,
    pub why: String,
}

/// 一個起點（可直接給 `read_from`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BriefRef {
    #[serde(rename = "ref")]
    pub reference: String,
    pub line: Option<usize>,
    pub text: String,
    pub why: String,
}

/// 一筆相關判斷。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BriefDecision {
    #[serde(rename = "ref")]
    pub reference: String,
    pub at: String,
    pub repo: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reopen_when: Option<String>,
    /// 內容主要在講別的 repo（跟記錄它的 repo 不同時才有）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about_repo: Option<String>,
}

/// `search_knowledge` 的精簡回傳。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchBrief {
    pub ok: bool,
    pub query: String,
    pub scope: String,
    pub answer_repo: Option<AnswerRepo>,
    /// answer_repo 是 None 時說一句
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub also: Vec<AlsoRepo>,
    pub refs: Vec<BriefRef>,
    pub decisions: Vec<BriefDecision>,
    pub next: String,
}

fn clip(s: &str, n: usize) -> String {
    crate::pystr::clip(s.trim(), n)
}

/// 一個 repo 的四個訊號（0～1）。
#[derive(Debug, Default, Clone)]
struct Signals {
    agg: f64,
    best: f64,
    profile: f64,
    decisions: f64,
    files: usize,
    n_decisions: usize,
}

impl Signals {
    fn total(&self) -> f64 {
        self.agg + self.best + 0.5 * self.profile + 0.3 * self.decisions
    }
}

fn norm(x: f64, max: f64) -> f64 {
    if max > 0.0 {
        x / max
    } else {
        0.0
    }
}

/// 判斷算給哪個 repo：內容在講的（`about_repo`），沒有就是記錄它的。
fn decision_repo(d: &Value) -> Option<String> {
    d["about_repo"]
        .as_str()
        .or_else(|| d["repo"].as_str())
        .filter(|r| !r.is_empty())
        .map(str::to_string)
}

fn rank(parts: &SearchParts) -> Vec<(String, Signals)> {
    let repos: &[RepoHit] = &parts.res.repos;
    let max_agg = repos.iter().map(|r| r.score).fold(0.0, f64::max);
    let max_best = repos.iter().map(|r| r.best_score).fold(0.0, f64::max);
    let max_prof = parts.profiles.iter().map(|p| p.score).fold(0.0, f64::max);
    let mut dec_sum: Vec<(String, f64, usize)> = Vec::new();
    for d in &parts.decisions {
        let Some(r) = decision_repo(d) else { continue };
        let sc = d["score"].as_f64().unwrap_or(0.0);
        match dec_sum.iter_mut().find(|x| x.0 == r) {
            Some(x) => {
                x.1 += sc;
                x.2 += 1;
            }
            None => dec_sum.push((r, sc, 1)),
        }
    }
    let max_dec = dec_sum.iter().map(|x| x.1).fold(0.0, f64::max);
    let mut out: Vec<(String, Signals)> = Vec::new();
    fn slot(out: &mut Vec<(String, Signals)>, id: &str) -> usize {
        match out.iter().position(|(r, _)| r == id) {
            Some(i) => i,
            None => {
                out.push((id.to_string(), Signals::default()));
                out.len() - 1
            }
        }
    }
    for r in repos {
        let i = slot(&mut out, &r.repo);
        out[i].1.agg = norm(r.score, max_agg);
        out[i].1.best = norm(r.best_score, max_best);
        out[i].1.files = r.matching_files;
    }
    for p in &parts.profiles {
        let i = slot(&mut out, &p.repo);
        out[i].1.profile = norm(p.score, max_prof);
    }
    for (r, sc, n) in &dec_sum {
        let i = slot(&mut out, r);
        out[i].1.decisions = norm(*sc, max_dec);
        out[i].1.n_decisions = *n;
    }
    // 只留範圍內登記的 repo（判斷可能記在別的 repo）
    out.retain(|(r, _)| parts.entries.iter().any(|e| spine::same_token(&e.id(), r)));
    out.sort_by(|a, b| b.1.total().total_cmp(&a.1.total()).then(a.0.cmp(&b.0)));
    out
}

fn why_of(parts: &SearchParts, id: &str, s: &Signals) -> Vec<String> {
    let mut why = Vec::new();
    if s.files > 0 {
        let best = parts
            .res
            .repos
            .iter()
            .find(|r| r.repo == id)
            .map(|r| r.best_file.clone())
            .unwrap_or_default();
        why.push(format!(
            "命中 {} 份檔，最相關的是 {}",
            s.files,
            clip(&best, 60)
        ));
    }
    if let Some(p) = parts.profiles.iter().find(|p| p.repo == id) {
        why.push(format!("名片：{}", p.why.join("、")));
    }
    if s.n_decisions > 0 {
        why.push(format!("有 {} 筆相關判斷", s.n_decisions));
    }
    if let Some((_, r)) = parts.related.iter().find(|(r, _)| spine::same_token(r, id)) {
        why.push(r.clone());
    }
    why
}

fn short_why(s: &Signals) -> String {
    let mut bits = Vec::new();
    if s.files > 0 {
        bits.push(format!("{} 份檔", s.files));
    }
    if s.profile > 0.0 {
        bits.push("名片命中".into());
    }
    if s.n_decisions > 0 {
        bits.push(format!("{} 筆判斷", s.n_decisions));
    }
    bits.join("、")
}

fn chars(b: &SearchBrief) -> usize {
    serde_json::to_string(b)
        .map(|s| s.chars().count())
        .unwrap_or(usize::MAX)
}

/// 精簡模式：先回答「誰有、為什麼」（序列化後 ≤ [`BRIEF_MAX_CHARS`] 字元）。不跑程式碼搜尋。
pub fn search_brief(
    spine_dir: &Path,
    query: &str,
    t: &Target,
    opts: &SearchOpts,
) -> Result<SearchBrief> {
    let opts = SearchOpts {
        code: false,
        limit: BRIEF_FILES.max(opts.limit),
        ..opts.clone()
    };
    let parts = search_parts(spine_dir, query, t, &opts)?;
    let ranked = rank(&parts);
    let qtoks = knowledge::tokenize(query);
    let answer = ranked.first().map(|(id, s)| {
        let e = parts
            .entries
            .iter()
            .find(|e| spine::same_token(&e.id(), id));
        AnswerRepo {
            repo: id.clone(),
            path: e.map(Repo::path_str).unwrap_or_default(),
            about: e.and_then(Repo::description).map(|x| clip(&x, 80)),
            role: e.and_then(Repo::role),
            why: why_of(&parts, id, s),
        }
    });
    let also: Vec<AlsoRepo> = ranked
        .iter()
        .skip(1)
        .take(BRIEF_ALSO)
        .map(|(id, s)| AlsoRepo {
            repo: id.clone(),
            why: short_why(s),
        })
        .collect();
    // 起點：命中檔前幾名，保證至少一個來自 answer_repo
    let to_ref = |repo: &str, file: &str, path: &str, why: &[String]| -> BriefRef {
        let snip = knowledge::snippets(path, &qtoks, 1).into_iter().next();
        BriefRef {
            reference: format!("{repo}:{file}"),
            line: snip.as_ref().map(|s| s.line),
            text: snip.map(|s| clip(&s.text, 60)).unwrap_or_default(),
            why: why.join("、"),
        }
    };
    let mut refs: Vec<BriefRef> = parts
        .res
        .files
        .iter()
        .take(BRIEF_REFS)
        .map(|f| to_ref(&f.repo, &f.file, &f.path, &f.why))
        .collect();
    if let Some(a) = &answer {
        let has = refs
            .iter()
            .any(|r| r.reference.starts_with(&format!("{}:", a.repo)));
        if !has {
            let own = parts
                .res
                .files
                .iter()
                .find(|f| f.repo == a.repo)
                .map(|f| to_ref(&f.repo, &f.file, &f.path, &f.why))
                .or_else(|| {
                    parts
                        .res
                        .repos
                        .iter()
                        .find(|r| r.repo == a.repo && !r.best_file.is_empty())
                        .map(|r| to_ref(&r.repo, &r.best_file, &r.best_path, &[]))
                });
            if let Some(own) = own {
                if refs.len() >= BRIEF_REFS {
                    refs.pop();
                }
                refs.insert(0, own);
            }
        }
    }
    // 判斷：沒有歸屬疑慮的優先
    let mut decs: Vec<&Value> = parts.decisions.iter().collect();
    decs.sort_by_key(|d| d.get("about_repo").is_some());
    let decisions: Vec<BriefDecision> = decs
        .into_iter()
        .take(BRIEF_DECISIONS)
        .map(|d| BriefDecision {
            reference: d["ref"].as_str().unwrap_or("").to_string(),
            at: d["at"].as_str().unwrap_or("").to_string(),
            repo: d["repo"].as_str().map(str::to_string),
            text: clip(d["text"].as_str().unwrap_or(""), 60),
            reopen_when: d["reopen_when"].as_str().map(|x| clip(x, 40)),
            about_repo: d["about_repo"].as_str().map(str::to_string),
        })
        .collect();
    let next = match &answer {
        Some(a) => format!(
            "repo_entry {} 看從哪讀起與現況；read_from <ref> 讀內容；要完整結果帶 detail=full",
            a.repo
        ),
        None => "換個說法再搜，或帶 detail=full 看完整結果".into(),
    };
    let mut b = SearchBrief {
        ok: true,
        query: clip(query, 120),
        scope: parts.label,
        note: answer
            .is_none()
            .then(|| "沒找到明顯相關的 repo".to_string()),
        answer_repo: answer,
        also,
        refs,
        decisions,
        next,
    };
    // 上限：先砍判斷、再砍不是 answer_repo 的起點、再砍其他候選、最後縮短字句
    while chars(&b) > BRIEF_MAX_CHARS {
        if b.decisions.pop().is_some() {
            continue;
        }
        if b.refs.len() > 1 {
            b.refs.pop();
            continue;
        }
        if b.also.pop().is_some() {
            continue;
        }
        let shrink = |s: &mut String| *s = clip(s, s.chars().count() / 2);
        let before = chars(&b);
        if let Some(a) = b.answer_repo.as_mut() {
            a.why.iter_mut().for_each(shrink);
            if let Some(x) = a.about.as_mut() {
                shrink(x);
            }
        }
        b.refs.iter_mut().for_each(|r| {
            shrink(&mut r.text);
            shrink(&mut r.why);
        });
        if chars(&b) >= before {
            break; // 縮不動了（查詢本身太長）
        }
    }
    Ok(b)
}
