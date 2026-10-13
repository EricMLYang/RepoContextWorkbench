//! Rust 才有（2026-10-13）：repo 名片的 agent 入口——`describe_repo` 寫、`context_for` 的組員名冊、
//! `search_knowledge` 的 repo 排名都在這裡組。名片本身見 [`crate::profile`]。

use super::*;
use crate::ops::{self, Via};
use crate::profile::{self, ProfileHit, RosterEntry};

/// `search_knowledge` 依名片排出的 repo 最多幾個。
pub const PROFILE_HITS: usize = 5;

/// `describe_repo` 的回傳：更新後的登記項＋給人看的一行。
#[derive(Debug, Clone, Serialize)]
pub struct RepoDescribed {
    pub ok: bool,
    pub repo: Value,
    pub said: String,
}

/// agent 改 repo 名片。沒給 id＝所在的 repo；只能改主場（所在 repo＋同組成員），跨組要使用者先同意。
pub fn describe_repo(
    spine_dir: &Path,
    id: Option<&str>,
    d: &registry::Describe,
    cwd: Option<&Path>,
    cross_scope_ok: bool,
) -> Result<RepoDescribed> {
    let w = where_am_i(spine_dir, cwd)?;
    let Some(id) = id
        .filter(|s| !s.is_empty())
        .or(w.repo_id())
        .map(str::to_string)
    else {
        return Err(agent_error(
            crate::RejectCode::BadRequest,
            "不在已登記的 repo 裡，要給 id",
            Some("用 where_am_i 看所在位置，或直接給 repo id。"),
        ));
    };
    if !cross_scope_ok {
        if let Some(home) = home_ids(spine_dir, &w)? {
            if !home.contains(&id) {
                return Err(agent_error(
                    crate::RejectCode::CrossScope,
                    format!(
                        "「{id}」不在你的主場（{}／同組成員）。",
                        w.repo_id().unwrap_or_default()
                    ),
                    Some("先問使用者是否要改別組 repo 的名片；同意後帶 cross_scope_ok=true 重試。"),
                ));
            }
        }
    }
    let args = json!({
        "id": id,
        "description": d.description,
        "role": d.role,
        "keywords": d.keywords,
    });
    let (repo, said) =
        ops::call(spine_dir, "registry_describe", args, Via::Mcp).map_err(|e| match e {
            Error::Invalid(m) => agent_error(crate::RejectCode::BadRequest, m, None),
            e => e,
        })?;
    Ok(RepoDescribed {
        ok: true,
        repo,
        said,
    })
}

/// 範圍內的組員名冊（範圍只有一個 repo 時不給：沒有「同組」可言）。
pub(super) fn roster_of(spine_dir: &Path, entries: &[Repo], w: &Where) -> Vec<RosterEntry> {
    if entries.len() < 2 {
        return Vec::new();
    }
    profile::roster(spine_dir, entries, w.repo_id())
}

/// 依名片排 repo（給 search_knowledge）。
pub(super) fn profile_hits(entries: &[Repo], query: &str) -> Vec<ProfileHit> {
    let profiles: Vec<_> = entries.iter().map(profile::profile).collect();
    profile::rank(&profiles, query, PROFILE_HITS)
}
