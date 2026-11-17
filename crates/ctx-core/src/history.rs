//! 過去的 agent 會話（2026-10-16，ideation F-72；Rust 才有）：範圍內 repo 的 Claude Code 逐字稿，一鍵接續。
//! 2026-11-14：Codex 的會話紀錄也列進來，每筆帶 `agent` 與接續指令。
//! 2026-11-18（F-134）：兩家都經過 [`crate::convo`] 的掃描器，這裡只管範圍、排序與清單的欄位。
//!
//! App 關掉、終端關掉之後，跟 agent 談到一半的會話就只剩 `~/.claude/projects` 裡的逐字稿——
//! 「回到會話」只回得到還開著的。這裡依範圍列出過去的互動會話（標題、時間、最後回覆），
//! App 用 `claude --resume <id>` 在原目錄接續（參考 Orca 的 Agent Session History）。
//!
//! - 只看範圍內 repo 的專案資料夾：Claude Code 把 cwd 的非英數字元都換成 `-` 當資料夾名，
//!   repo 本身與它的子目錄（名稱多一段 `-…`）才讀，再用逐字稿裡的 cwd 確認真的在 repo 底下。
//! - 每份只讀頭尾（[`crate::convo::claude`]），不整份讀。
//! - 程式化呼叫（`claude -p`：App 助手、`ask_repo`、碰撞判定…，`entrypoint` 是 `sdk-*`）不列——不是人談過的會話。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

use crate::config::AgentKind;
use crate::convo::{Session, Sources};
use crate::registry;
use crate::transcript;
use crate::Result;

/// 一場過去的會話。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PastSession {
    /// 那一家 CLI 自己的會話 id（`claude --resume <id>`／`codex resume <id>` 用）。
    pub id: String,
    /// 哪一家的會話：`claude`｜`codex`（[`AgentKind::name`]；2026-11-14）。接續時當 `term_create` 的 `agent`。
    pub agent: &'static str,
    /// 在終端自己接續的指令（`cd <目錄> && claude --resume <id>`）；畫面的「複製指令」直接用。
    pub resume: String,
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

pub use crate::convo::claude::project_dir_name;

/// 會話 id 看起來對不對（只收英數與 `-`，避免拼進指令時出事）。
pub fn valid_session_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn local_minute(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// 在終端自己接續的指令：`cd <目錄> && <那一家的接續指令>`。
pub fn resume_shell_command(kind: AgentKind, id: &str, cwd: &str) -> String {
    let cmd = kind.resume_command(id).unwrap_or_default();
    format!("cd {} && {cmd}", crate::agentsetup::cmdline(&[cwd]))
}

/// 範圍內 repo 過去的互動會話（Claude Code 與 Codex），最近的在前，最多 `limit` 筆。
/// `root`＝Claude Code 的專案資料夾根目錄；None＝預設 [`crate::backfill::projects_root`]，並且也讀
/// Codex 的紀錄（`$CODEX_HOME`）。給了 `root`（測試給暫存目錄）就只讀那裡的 Claude Code 逐字稿。
pub fn past_sessions(
    spine_dir: &Path,
    group: Option<&str>,
    repos: Option<&[String]>,
    limit: usize,
    root: Option<&Path>,
) -> Result<Vec<PastSession>> {
    let src = match root {
        Some(r) => Sources::claude_only(r),
        None => Sources::default_homes(),
    };
    past_sessions_from(spine_dir, group, repos, limit, &src)
}

/// 同 [`past_sessions`]，兩家的紀錄根目錄都由呼叫端給（`codex_home` None＝不讀 Codex 的）。
pub fn past_sessions_in(
    spine_dir: &Path,
    group: Option<&str>,
    repos: Option<&[String]>,
    limit: usize,
    root: &Path,
    codex_home: Option<&Path>,
) -> Result<Vec<PastSession>> {
    past_sessions_from(
        spine_dir,
        group,
        repos,
        limit,
        &Sources::new(Some(root), codex_home),
    )
}

/// 一場正規化的會話 → 清單的一列（`repo` 由呼叫端依 cwd 決定）。
pub fn past_session(s: Session, repo: String, now: SystemTime) -> PastSession {
    // 接續上一場的會話，開頭可能只有壓縮摘要（N-40）：仍列出，第一句標明是接續的
    let prompt = s
        .prompt
        .as_deref()
        .or(s.continued.then_some("（從上一場的壓縮摘要接續）"))
        .map(|p| transcript::one_line(p, 120))
        .unwrap_or_default();
    PastSession {
        resume: resume_shell_command(s.agent, &s.id, &s.cwd),
        agent: s.agent.name(),
        repo,
        title: s.title.unwrap_or_else(|| prompt.clone()),
        prompt: Some(prompt),
        reply: s.reply.as_deref().map(|r| transcript::one_line(r, 160)),
        branch: s.branch,
        started: s.started,
        updated: local_minute(s.mtime),
        live: now
            .duration_since(s.mtime)
            .map(|d| d.as_secs() < 120)
            .unwrap_or(true),
        path: s.path.to_string_lossy().into_owned(),
        cwd: s.cwd,
        id: s.id,
    }
}

/// 同 [`past_sessions`]，掃描器由呼叫端給（F-134：兩家都只經過 [`crate::convo::Scanner`]）。
pub fn past_sessions_from(
    spine_dir: &Path,
    group: Option<&str>,
    repos: Option<&[String]>,
    limit: usize,
    src: &Sources,
) -> Result<Vec<PastSession>> {
    let (_, entries) = registry::resolve_group(spine_dir, group, repos)?;
    // 路徑長的先比（巢狀 repo 歸給最深的那個）
    let targets = crate::convo::targets_of(&entries);
    let roots: Vec<PathBuf> = targets.iter().map(|(_, p)| p.clone()).collect();
    let now = SystemTime::now();
    let mut found: Vec<(SystemTime, PastSession)> = Vec::new();
    for sc in src.all() {
        for s in sc.sessions(&roots) {
            let Some(repo) = crate::convo::repo_of(&targets, &s.cwd) else {
                continue;
            };
            let repo = repo.to_string();
            found.push((s.mtime, past_session(s, repo, now)));
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
