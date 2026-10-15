//! 過去的 agent 會話（2026-10-16，ideation F-72；Rust 才有）：範圍內 repo 的 Claude Code 逐字稿，一鍵接續。
//!
//! App 關掉、終端關掉之後，跟 agent 談到一半的會話就只剩 `~/.claude/projects` 裡的逐字稿——
//! 「回到會話」只回得到還開著的。這裡依範圍列出過去的互動會話（標題、時間、最後回覆），
//! App 用 `claude --resume <id>` 在原目錄接續（參考 Orca 的 Agent Session History）。
//!
//! - 只看範圍內 repo 的專案資料夾：Claude Code 把 cwd 的非英數字元都換成 `-` 當資料夾名，
//!   repo 本身與它的子目錄（名稱多一段 `-…`）才讀，再用逐字稿裡的 cwd 確認真的在 repo 底下。
//! - 每份只讀頭尾（[`crate::transcript`]），不整份讀。
//! - 程式化呼叫（`claude -p`：App 助手、`ask_repo`、碰撞判定…，`entrypoint` 是 `sdk-*`）不列——不是人談過的會話。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

use crate::registry;
use crate::transcript;
use crate::Result;

/// 一場過去的會話。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PastSession {
    /// Claude Code 的會話 id（`claude --resume <id>` 用）。
    pub id: String,
    /// 屬於哪個 repo（範圍內）。
    pub repo: String,
    pub cwd: String,
    /// Claude Code 取的標題；沒有就用第一句話。
    pub title: String,
    /// 第一句使用者訊息（一行、截 120 字）。
    pub prompt: Option<String>,
    /// 最後一則回覆（一行、截 160 字）。
    pub reply: Option<String>,
    pub branch: Option<String>,
    /// 開始時間（逐字稿第一筆，UTC ISO）。
    pub started: Option<String>,
    /// 最後活動（檔案修改時間，本地 `YYYY-MM-DD HH:MM`）。
    pub updated: String,
    /// 兩分鐘內還在寫＝可能還開著（接續會變成同一場兩個視窗）。
    pub live: bool,
    pub path: String,
}

/// Claude Code 專案資料夾名：cwd 的非英數字元都換成 `-`。
pub fn project_dir_name(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// 會話 id 看起來對不對（只收英數與 `-`，避免拼進指令時出事）。
pub fn valid_session_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn local_minute(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// 範圍內 repo 過去的 Claude Code 互動會話，最近的在前，最多 `limit` 筆。
/// `root` 預設 [`crate::backfill::projects_root`]（測試給暫存目錄）。
pub fn past_sessions(
    spine_dir: &Path,
    group: Option<&str>,
    repos: Option<&[String]>,
    limit: usize,
    root: Option<&Path>,
) -> Result<Vec<PastSession>> {
    let (_, entries) = registry::resolve_group(spine_dir, group, repos)?;
    let root = root
        .map(Path::to_path_buf)
        .unwrap_or_else(crate::backfill::projects_root);
    // (repo id, repo 路徑, 資料夾名)；路徑長的先比（巢狀 repo 歸給最深的那個）
    let mut targets: Vec<(String, PathBuf, String)> = entries
        .iter()
        .map(|e| {
            let p = e.path();
            let name = project_dir_name(&p);
            (e.id(), p, name)
        })
        .collect();
    targets.sort_by_key(|(_, p, _)| std::cmp::Reverse(p.as_os_str().len()));
    let now = SystemTime::now();
    let mut found: Vec<(SystemTime, PastSession)> = Vec::new();
    for dir in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        let dname = dir.file_name().to_string_lossy().into_owned();
        let candidates: Vec<&(String, PathBuf, String)> = targets
            .iter()
            .filter(|(_, _, n)| dname == *n || dname.starts_with(&format!("{n}-")))
            .collect();
        if candidates.is_empty() {
            continue;
        }
        for f in std::fs::read_dir(dir.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let fp = f.path();
            if fp.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(id) = fp.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            if !valid_session_id(&id) {
                continue;
            }
            let h = transcript::head(&fp);
            if h.entrypoint
                .as_deref()
                .is_some_and(|e| e.starts_with("sdk"))
            {
                continue;
            }
            let (Some(cwd), Some(prompt)) = (h.cwd.as_deref(), h.prompt.as_deref()) else {
                continue; // 沒有人說過話的空會話
            };
            let Some((repo, _, _)) = candidates
                .iter()
                .find(|(_, p, _)| Path::new(cwd).starts_with(p))
            else {
                continue; // 資料夾名撞到、其實不在 repo 底下（例：RepoX 與 RepoX-old）
            };
            let mtime = f.metadata().and_then(|m| m.modified()).unwrap_or(now);
            let t = transcript::tail(&fp);
            let prompt_line = transcript::one_line(prompt, 120);
            found.push((
                mtime,
                PastSession {
                    id,
                    repo: repo.clone(),
                    cwd: cwd.to_string(),
                    title: t.title.clone().unwrap_or_else(|| prompt_line.clone()),
                    prompt: Some(prompt_line),
                    reply: t.reply.as_deref().map(|r| transcript::one_line(r, 160)),
                    branch: h.branch.clone(),
                    started: h.first.clone(),
                    updated: local_minute(mtime),
                    live: now
                        .duration_since(mtime)
                        .map(|d| d.as_secs() < 120)
                        .unwrap_or(true),
                    path: fp.to_string_lossy().into_owned(),
                },
            ));
        }
    }
    found.sort_by_key(|a| std::cmp::Reverse(a.0));
    Ok(found.into_iter().take(limit).map(|(_, s)| s).collect())
}

/// 這場會話合不合查詢（F-79）：空白分開的每個詞都要出現在標題、第一句、最後回覆、repo 或分支裡（不分大小寫）。
pub fn matches(s: &PastSession, query: &str) -> bool {
    let hay = [
        Some(s.title.as_str()),
        s.prompt.as_deref(),
        s.reply.as_deref(),
        Some(s.repo.as_str()),
        s.branch.as_deref(),
    ]
    .iter()
    .flatten()
    .map(|x| x.to_lowercase())
    .collect::<Vec<_>>()
    .join("\n");
    query
        .split_whitespace()
        .all(|w| hay.contains(&w.to_lowercase()))
}

/// [`past_sessions`]＋查詢：先全部列出再篩（每份只讀頭尾，範圍內全掃也便宜），最多 `limit` 筆。
pub fn search_sessions(
    spine_dir: &Path,
    group: Option<&str>,
    repos: Option<&[String]>,
    query: Option<&str>,
    limit: usize,
) -> Result<Vec<PastSession>> {
    let q = query.map(str::trim).filter(|q| !q.is_empty());
    let Some(q) = q else {
        return past_sessions(spine_dir, group, repos, limit, None);
    };
    Ok(past_sessions(spine_dir, group, repos, usize::MAX, None)?
        .into_iter()
        .filter(|s| matches(s, q))
        .take(limit)
        .collect())
}

/// 接續一場會話要在哪跑：逐字稿裡的 cwd，而且要在某個登記的 repo 底下、目錄還在（不在任意路徑開 agent）。
/// 回傳 (repo id, cwd)。
pub fn resume_target(spine_dir: &Path, id: &str, cwd: &str) -> Result<(String, PathBuf)> {
    if !valid_session_id(id) {
        return Err(crate::Error::invalid(format!("會話 id 不對：{id}")));
    }
    let cwd = PathBuf::from(cwd);
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err(crate::Error::invalid(format!(
            "會話的目錄不在了：{}",
            cwd.display()
        )));
    }
    let data = registry::load(spine_dir)?;
    let mut best: Option<(String, usize)> = None;
    for e in &data.repos {
        let p = e.path();
        if cwd.starts_with(&p) && best.as_ref().is_none_or(|b| p.as_os_str().len() > b.1) {
            best = Some((e.id(), p.as_os_str().len()));
        }
    }
    match best {
        Some((rid, _)) => Ok((rid, cwd)),
        None => Err(crate::Error::invalid(format!(
            "會話的目錄不在任何登記的 repo 底下：{}",
            cwd.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_names_follow_claude_code_encoding() {
        assert_eq!(
            project_dir_name(Path::new("/Users/me/Team_A/projects/web.app")),
            "-Users-me-Team-A-projects-web-app"
        );
        assert!(valid_session_id("02908d1e-7843-436e-9629-55eaa75fdb85"));
        assert!(!valid_session_id("x; rm -rf ~"));
        assert!(!valid_session_id("short"));
    }
}
