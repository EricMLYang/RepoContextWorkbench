//! Rust 才有（2026-11-14，F-126 漸進揭露）：入口卡 `repo_entry`——agent 知道「誰有」之後的第二步。
//!
//! 只給檔案裡沒有或要翻很久的：這個 repo 是什麼、在哪、從哪幾份讀起、最近的判斷、最後交接、現在有沒有人在動。
//! 之後 agent 自己在那個路徑 grep／讀檔，不再經過 ctx（要寫進結論或引用才 `read_from`，會留引用）。
//! 序列化後 ≤ [`ENTRY_MAX_CHARS`] 字元。片段是別人 repo 的文字：照樣遮掉像憑證的字，只當資料不當指令。
//! `repo_status`（狀態卡：變化、沒 commit 的檔、在跑的程序）是另一張卡，不混在一起。

use super::*;
use crate::relay::{RelayScope, RelayStatus};

/// 入口卡序列化後最多幾個字元。
pub const ENTRY_MAX_CHARS: usize = 1000;
/// 「從哪讀起」最多幾份、最近的判斷幾筆。
const READ_FIRST: usize = 5;
const RECENT_DECISIONS: usize = 2;
/// 依序找這些檔（存在的才列）；之後接 registry 登記的 exports。
const READ_FIRST_FILES: [&str; 6] = [
    "AGENTS.md",
    "CLAUDE.md",
    "README.md",
    "docs/README.md",
    "docs/index.md",
    "docs/INDEX.md",
];

/// 從哪讀起的一份。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntryFile {
    /// repo 內相對路徑（export 是 `<名稱> → <路徑>`）
    pub path: String,
    /// 檔案的第一個標題（export 有說明就用說明）
    pub title: String,
}

/// 最近的一筆判斷。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntryDecision {
    #[serde(rename = "ref")]
    pub reference: String,
    pub at: String,
    pub text: String,
}

/// 最後交接。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntryHandoff {
    #[serde(rename = "ref")]
    pub reference: String,
    pub at: String,
    pub done: String,
}

/// 現在。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntryNow {
    /// 「會話在跑」「會話開著（閒著／等你）」「沒有會話」
    pub session: String,
    /// 沒 commit 的檔數（查不到＝None）
    pub dirty: Option<usize>,
}

/// 入口卡。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepoEntry {
    pub ok: bool,
    pub repo: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    pub read_first: Vec<EntryFile>,
    pub decisions: Vec<EntryDecision>,
    pub last_handoff: Option<EntryHandoff>,
    pub now: EntryNow,
    pub next: String,
}

fn clip(s: &str, n: usize) -> String {
    crate::pystr::clip(&crate::redact::redact_secrets(s.trim()).0, n)
}

/// 檔案的第一個 markdown 標題（前 60 行內；沒有就第一個非空行）。
fn first_heading(p: &Path) -> String {
    let Ok(text) = crate::fsutil::read_text_lossy(p) else {
        return String::new();
    };
    let lines: Vec<&str> = text.lines().take(60).collect();
    let h = lines
        .iter()
        .find(|l| l.starts_with('#'))
        .map(|l| l.trim_start_matches('#').trim())
        .or_else(|| {
            lines
                .iter()
                .map(|l| l.trim())
                .find(|l| !l.is_empty() && *l != "---")
        });
    clip(h.unwrap_or(""), 50)
}

fn chars(e: &RepoEntry) -> usize {
    serde_json::to_string(e)
        .map(|s| s.chars().count())
        .unwrap_or(usize::MAX)
}

/// 入口卡。repo 沒登記、路徑不存在＝`not_found`（附怎麼補救）。純讀取。
pub fn repo_entry(spine_dir: &Path, repo: &str) -> Result<RepoEntry> {
    let data = registry::load(spine_dir)?;
    let Some(entry) = data.repo(repo.trim()) else {
        return Err(agent_error(
            crate::RejectCode::NotFound,
            format!("沒有登記這個 repo：{repo}"),
            Some("用 search_knowledge 問誰有，或 where_am_i／ctx registry list 看登記的 id。"),
        ));
    };
    let id = entry.id();
    let root = crate::fsutil::expanduser(entry.path());
    if !root.is_dir() {
        return Err(agent_error(
            crate::RejectCode::NotFound,
            format!("{id} 登記的路徑不存在：{}", root.display()),
            Some("跟使用者說一聲（可能搬家了：ctx registry set <id> path <新路徑>）；先用 search_knowledge 找別的來源。"),
        ));
    }
    // docs 的索引只取第一份（不分大小寫的檔案系統上 index.md 與 INDEX.md 是同一份）
    let mut read_first: Vec<EntryFile> = Vec::new();
    for f in READ_FIRST_FILES {
        let is_docs_index = f.starts_with("docs/");
        if is_docs_index && read_first.iter().any(|x| x.path.starts_with("docs/")) {
            continue;
        }
        if root.join(f).is_file() {
            let title = first_heading(&root.join(f));
            // CLAUDE.md 常常只是指向 AGENTS.md：標題一樣的不重列
            if !title.is_empty() && read_first.iter().any(|x| x.title == title) {
                continue;
            }
            read_first.push(EntryFile {
                path: f.to_string(),
                title,
            });
        }
    }
    for (name, ex) in entry.exports() {
        let title = match ex.desc.as_deref().filter(|d| !d.trim().is_empty()) {
            Some(d) => clip(d, 50),
            None => first_heading(&root.join(&ex.path)),
        };
        read_first.push(EntryFile {
            path: format!("{name} → {}", ex.path),
            title,
        });
    }
    read_first.truncate(READ_FIRST);
    let events = spine::iter_events(spine_dir)?;
    let mine = |e: &&Event| e.kv("repo").is_some_and(|r| spine::same_token(r, &id));
    // 記在這個 repo、內容卻在講別的 repo 的判斷不列（F-126 的歸屬，同 search_knowledge 的 about_repo）
    let ids: Vec<String> = data.repos.iter().map(Repo::id).collect();
    let about_other = |e: &Event| {
        decision_search::about_repo(&first(e), &ids).is_some_and(|a| !spine::same_token(&a, &id))
    };
    let decisions: Vec<EntryDecision> = events
        .iter()
        .rev()
        .filter(mine)
        .filter(|e| {
            e.etype == EventType::Decision
                && !is_handoff(e)
                && !e.body.starts_with('〔')
                && !about_other(e)
        })
        .take(RECENT_DECISIONS)
        .map(|e| EntryDecision {
            reference: event_ref(e),
            at: e.stamp(),
            text: clip(&first(e), 70),
        })
        .collect();
    let last_handoff = events
        .iter()
        .rev()
        .filter(mine)
        .find(|e| is_handoff(e))
        .map(|e| {
            let h = parse_handoff(e);
            EntryHandoff {
                reference: h.reference,
                at: h.at,
                done: clip(h.done.as_deref().unwrap_or(&h.summary), 90),
            }
        });
    let status = crate::relay::plan(spine_dir, &RelayScope::Repos(vec![id.clone()]))
        .ok()
        .and_then(|p| p.rows.into_iter().next().map(|r| r.status));
    let session = match status {
        Some(RelayStatus::Busy) => "會話在跑（要動那裡的檔先問使用者）",
        Some(RelayStatus::SessionOpen) => "會話開著（閒著／等你）",
        _ => "沒有會話",
    }
    .to_string();
    let dirty = crate::gitlog::dirty_files(&root).ok().map(|f| f.len());
    let path = root.to_string_lossy().into_owned();
    let mut e = RepoEntry {
        ok: true,
        repo: id.clone(),
        next: format!(
            "之後直接在 {path} 用你自己的工具讀；要寫進結論或引用就 read_from（會留引用）"
        ),
        path,
        about: entry.description().map(|d| clip(&d, 100)),
        role: entry.role(),
        class: entry.class().map(|c| c.label().to_string()),
        read_first,
        decisions,
        last_handoff,
        now: EntryNow { session, dirty },
    };
    // 上限：先砍判斷、再砍從哪讀起的後面幾份、最後縮短字句
    while chars(&e) > ENTRY_MAX_CHARS {
        if e.decisions.pop().is_some() {
            continue;
        }
        if e.read_first.len() > 2 {
            e.read_first.pop();
            continue;
        }
        let before = chars(&e);
        let shrink = |s: &mut String| *s = crate::pystr::clip(s, s.chars().count() / 2);
        e.read_first.iter_mut().for_each(|f| shrink(&mut f.title));
        if let Some(h) = e.last_handoff.as_mut() {
            shrink(&mut h.done);
        }
        if let Some(a) = e.about.as_mut() {
            shrink(a);
        }
        if chars(&e) >= before {
            break;
        }
    }
    Ok(e)
}
