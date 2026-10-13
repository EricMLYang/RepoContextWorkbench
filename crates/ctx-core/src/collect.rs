//! P3 採集：本地 git 狀態。對應 `repo_context/collect.py`。
//!
//! 欄位規格抄 gitpane（拆機報告 §2.1）：分支／dirty（＝變更檔案數）／ahead-behind／worktree 數／agent 偵測。
//! external repo 不看 dirty（監控語意不同，v1 §3.1）；上游查詢屬 P4，原型留 stub。
//! dirty_days 以「dirty 檔案中最舊的 mtime」近似（夠用；正式版可改脊椎留痕推算）。
//! agent 偵測讀 agentmark 的存活標記（只感知引擎自己 spawn 的內嵌會話）。
//!
//! 活動脈動（2026-09-08 組為單位輪）：組的監控專注在「活動頻繁度＋近期 commit 內容」——
//! 每 repo 帶 commits_7d／commits_30d／recent_commits（最新 5 則主旨）；group_pulse 做組級彙總，
//! recent_across 把組內近期 commit 攤平成一條時間線。全是資訊不是問句（無事不報仍守）。

use std::path::Path;
use std::process::Command;

use serde::{Serialize, Serializer};

use crate::agentmark;
use crate::fsutil::{expanduser, mtime_secs, now_secs, resolve};
use crate::pystr::take_chars;

/// 採集輸入：registry repo entry 的必要欄位（呼叫端從 registry 取）。
/// `repo_type`／`tier` 用 registry.yaml 的原字串（Python 預設 "mine"／"active"）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSpec {
    pub id: String,
    pub path: std::path::PathBuf,
    pub repo_type: String,
    pub tier: String,
}

impl RepoSpec {
    /// type＝mine、tier＝active（Python `entry.get(..., 預設)`）。
    pub fn new(id: impl Into<String>, path: impl Into<std::path::PathBuf>) -> Self {
        RepoSpec {
            id: id.into(),
            path: path.into(),
            repo_type: "mine".into(),
            tier: "active".into(),
        }
    }
    pub fn with_type(mut self, t: impl Into<String>) -> Self {
        self.repo_type = t.into();
        self
    }
    pub fn with_tier(mut self, t: impl Into<String>) -> Self {
        self.tier = t.into();
        self
    }
}

/// registry 登記項 → 採集 entry（Python `entry.get("type", "mine")`、`entry.get("tier", "active")`）。
impl From<&crate::registry::Repo> for RepoSpec {
    fn from(r: &crate::registry::Repo) -> Self {
        let mut s = RepoSpec::new(r.id(), r.path_str());
        if let Some(t) = r.kind() {
            s.repo_type = t;
        }
        if let Some(t) = r.tier_str() {
            s.tier = t;
        }
        s
    }
}

/// 最近 commit 一則（`git log --format=%ad%x09%h%x09%s --date=short`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitInfo {
    pub date: String,
    pub hash: String,
    pub subject: String,
}

/// 對應 `collect_repo` 回傳的狀態 dict（序列化鍵名與順序同 Python）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepoState {
    pub id: String,
    #[serde(rename = "type")]
    pub repo_type: String,
    pub tier: String,
    pub path: String,
    pub exists: bool,
    pub dirty: usize,
    pub dirty_days: Option<f64>,
    pub last_commit_days: Option<f64>,
    pub ahead: Option<i64>,
    pub behind: Option<i64>,
    pub branch: Option<String>,
    pub last_subject: String,
    pub worktrees: usize,
    pub commits_7d: Option<u64>,
    pub commits_30d: Option<u64>,
    pub recent_commits: Vec<CommitInfo>,
    pub agents: Vec<String>,
    pub note: String,
}

/// 對應 `collect._git`：回傳 (成功與否, stdout 去頭尾空白)。
fn git(repo: &Path, args: &[&str]) -> (bool, String) {
    let (ok, out) = git_raw(repo, args);
    (ok, out.trim().to_string())
}

fn git_raw(repo: &Path, args: &[&str]) -> (bool, String) {
    match Command::new("git").arg("-C").arg(repo).args(args).output() {
        Ok(o) => (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
        ),
        Err(_) => (false, String::new()),
    }
}

/// 對應 `collect.collect_repo`：entry → 狀態。
pub fn collect_repo(entry: &RepoSpec) -> RepoState {
    let p = expanduser(&entry.path);
    let mut st = RepoState {
        id: entry.id.clone(),
        repo_type: entry.repo_type.clone(),
        tier: entry.tier.clone(),
        path: p.to_string_lossy().into_owned(),
        exists: p.is_dir(),
        dirty: 0,
        dirty_days: None,
        last_commit_days: None,
        ahead: None,
        behind: None,
        branch: None,
        last_subject: String::new(),
        worktrees: 0,
        commits_7d: None,
        commits_30d: None,
        recent_commits: Vec::new(),
        agents: Vec::new(),
        note: String::new(),
    };
    if !st.exists {
        st.note = "路徑不存在".into();
        return st;
    }
    if st.repo_type == "external" {
        st.note = "external：上游查詢待 P4".into();
        return st;
    }
    // status 不 trim 整份輸出：porcelain 第一行開頭的空白（" M file"）是欄位的一部分。
    // --no-optional-locks：背景採集不拿 index.lock、不回寫 index（只讀不寫使用者 repo；
    // agent 同時在 commit 也不會撞到「index.lock exists」）
    let (ok, out) = git_raw(&p, &["--no-optional-locks", "status", "--porcelain"]);
    if !ok {
        st.note = "非 git repo".into();
        return st;
    }
    let dirty_files: Vec<String> = out
        .lines()
        .filter(|ln| !ln.trim().is_empty())
        .map(|ln| {
            ln.chars()
                .skip(3)
                .collect::<String>()
                .trim()
                .trim_matches('"')
                .to_string()
        })
        .collect();
    st.dirty = dirty_files.len();
    if !dirty_files.is_empty() {
        let mtimes: Vec<f64> = dirty_files
            .iter()
            .filter_map(|f| {
                let name = f.rsplit(" -> ").next().unwrap_or(f);
                let fp = p.join(name);
                if fp.exists() {
                    mtime_secs(&fp).ok()
                } else {
                    None
                }
            })
            .collect();
        if let Some(min) = mtimes.into_iter().reduce(f64::min) {
            st.dirty_days = Some((now_secs() - min) / 86400.0);
        }
    }
    let (ok, out) = git(&p, &["log", "-1", "--format=%ct"]);
    if ok && !out.is_empty() {
        if let Ok(ct) = out.parse::<i64>() {
            st.last_commit_days = Some((now_secs() - ct as f64) / 86400.0);
        }
    }
    let (ok, out) = git(&p, &["rev-list", "--left-right", "--count", "@{u}...HEAD"]);
    if ok && !out.is_empty() {
        let nums: Vec<&str> = out.split_whitespace().collect();
        if let [behind, ahead] = nums[..] {
            if let (Ok(a), Ok(b)) = (ahead.parse(), behind.parse()) {
                st.ahead = Some(a);
                st.behind = Some(b);
            }
        }
    }
    let (ok, out) = git(&p, &["rev-parse", "--abbrev-ref", "HEAD"]);
    st.branch = if ok && !out.is_empty() {
        Some(out)
    } else {
        None
    };
    let (ok, out) = git(&p, &["log", "-1", "--format=%s"]);
    st.last_subject = if ok { out } else { String::new() };
    for (is_7d, since) in [(true, "--since=7.days"), (false, "--since=30.days")] {
        let (ok, out) = git(&p, &["rev-list", "--count", since, "HEAD"]);
        let n = if ok && !out.is_empty() && out.chars().all(|c| c.is_ascii_digit()) {
            out.parse().unwrap_or(0)
        } else {
            0
        };
        if is_7d {
            st.commits_7d = Some(n);
        } else {
            st.commits_30d = Some(n);
        }
    }
    let (ok, out) = git(
        &p,
        &["log", "-5", "--format=%ad%x09%h%x09%s", "--date=short"],
    );
    if ok && !out.is_empty() {
        for ln in out.lines() {
            let parts: Vec<&str> = ln.splitn(3, '\t').collect();
            if let [date, hash, subject] = parts[..] {
                st.recent_commits.push(CommitInfo {
                    date: date.into(),
                    hash: hash.into(),
                    subject: subject.into(),
                });
            }
        }
    }
    let (ok, out) = git(&p, &["worktree", "list", "--porcelain"]);
    if ok && !out.is_empty() {
        // 主 worktree 不算，只數掛出去的（gitpane 的 agent 並行場景）
        st.worktrees = out
            .lines()
            .filter(|ln| ln.starts_with("worktree "))
            .count()
            .saturating_sub(1);
    }
    st
}

/// 對應 `collect._annotate_agents`：存活 agent 會話的 cwd 落在哪個 repo 底下就標誰。
fn annotate_agents(states: &mut [RepoState], spine_dir: &Path) {
    let sessions = agentmark::live(spine_dir);
    if sessions.is_empty() {
        return;
    }
    for s in states.iter_mut() {
        let rp = resolve(Path::new(&s.path));
        for m in &sessions {
            let cwd = resolve(Path::new(&m.cwd));
            if cwd.starts_with(&rp) {
                let name = m
                    .agent
                    .clone()
                    .filter(|a| !a.is_empty())
                    .unwrap_or_else(|| "agent".into());
                s.agents.push(name);
            }
        }
    }
}

/// 對應 `collect.collect_group`。有 `spine_dir` 才做 agent 偵測。
pub fn collect_group(entries: &[RepoSpec], spine_dir: Option<&Path>) -> Vec<RepoState> {
    let mut states: Vec<RepoState> = entries.iter().map(collect_repo).collect();
    if let Some(sd) = spine_dir {
        annotate_agents(&mut states, sd);
    }
    states
}

/// 最新一則 commit（帶 repo）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepoCommit {
    pub repo: String,
    pub date: String,
    pub hash: String,
    pub subject: String,
}

/// 對應 `group_pulse` 回傳的 dict（鍵名、順序同 Python）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pulse {
    pub repos: usize,
    pub commits_7d: u64,
    pub commits_30d: u64,
    /// 有序 map（repo id → 7 天 commit 數），序列化成 JSON 物件。
    #[serde(serialize_with = "ser_ordered_map")]
    pub per_repo_7d: Vec<(String, u64)>,
    pub active_7d: Vec<String>,
    pub quiet_30d: Vec<String>,
    pub most_active: Option<String>,
    pub latest: Option<RepoCommit>,
}

fn ser_ordered_map<S: Serializer>(
    v: &[(String, u64)],
    s: S,
) -> std::result::Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut m = s.serialize_map(Some(v.len()))?;
    for (k, n) in v {
        m.serialize_entry(k, n)?;
    }
    m.end()
}

/// 對應 `collect.group_pulse`：組級活動脈動——7d/30d 總 commit、每 repo 7d 數、最活躍、30 天沉默者、最新一則 commit。
pub fn group_pulse(states: &[RepoState]) -> Pulse {
    let mine: Vec<&RepoState> = states
        .iter()
        .filter(|s| s.repo_type == "mine" && s.exists)
        .collect();
    let mut per: Vec<(String, u64)> = Vec::new();
    for s in &mine {
        let n = s.commits_7d.unwrap_or(0);
        match per.iter_mut().find(|(k, _)| *k == s.id) {
            Some(e) => e.1 = n, // dict 同鍵覆寫，位置不變
            None => per.push((s.id.clone(), n)),
        }
    }
    let mut latest: Option<RepoCommit> = None;
    for s in &mine {
        // recent_commits 已是新→舊，只看第一則
        if let Some(c) = s.recent_commits.first() {
            let newer = match &latest {
                None => true,
                Some(l) => (c.date.as_str(), s.id.as_str()) > (l.date.as_str(), l.repo.as_str()),
            };
            if newer {
                latest = Some(RepoCommit {
                    repo: s.id.clone(),
                    date: c.date.clone(),
                    hash: c.hash.clone(),
                    subject: c.subject.clone(),
                });
            }
        }
    }
    let max = per.iter().map(|(_, v)| *v).max().unwrap_or(0);
    let most_active = if max > 0 {
        per.iter().find(|(_, v)| *v == max).map(|(k, _)| k.clone())
    } else {
        None
    };
    Pulse {
        repos: mine.len(),
        commits_7d: per.iter().map(|(_, v)| v).sum(),
        commits_30d: mine.iter().map(|s| s.commits_30d.unwrap_or(0)).sum(),
        active_7d: per
            .iter()
            .filter(|(_, v)| *v > 0)
            .map(|(k, _)| k.clone())
            .collect(),
        quiet_30d: mine
            .iter()
            .filter(|s| s.commits_30d.unwrap_or(0) == 0)
            .map(|s| s.id.clone())
            .collect(),
        per_repo_7d: per,
        most_active,
        latest,
    }
}

/// 對應 `collect.recent_across`：組內近期 commit 攤平成一條時間線（新→舊）。Python 預設 `limit=20`。
pub fn recent_across(states: &[RepoState], limit: usize) -> Vec<RepoCommit> {
    let mut rows: Vec<RepoCommit> = states
        .iter()
        .flat_map(|s| {
            s.recent_commits.iter().map(move |c| RepoCommit {
                repo: s.id.clone(),
                date: c.date.clone(),
                hash: c.hash.clone(),
                subject: c.subject.clone(),
            })
        })
        .collect();
    // Python sort(reverse=True) 是穩定的：同鍵保持原順序
    rows.sort_by(|a, b| {
        (b.date.as_str(), b.repo.as_str()).cmp(&(a.date.as_str(), a.repo.as_str()))
    });
    rows.truncate(limit);
    rows
}

/// 對應 `collect.pulse_line`：脈動一行（簡報／CLI／情境卡用）。
pub fn pulse_line(pulse: &Pulse) -> String {
    if pulse.repos == 0 {
        return "（沒有可採集的 mine repo）".into();
    }
    let mut tops: Vec<&(String, u64)> = pulse.per_repo_7d.iter().collect();
    tops.sort_by_key(|kv| std::cmp::Reverse(kv.1));
    let tops: Vec<String> = tops
        .iter()
        .filter(|kv| kv.1 > 0)
        .take(4)
        .map(|(k, v)| format!("{k} {v}"))
        .collect();
    let mut line = format!(
        "近 7 天 {} commits／30 天 {}",
        pulse.commits_7d, pulse.commits_30d
    );
    if tops.is_empty() {
        line.push_str("（全組無新 commit）");
    } else {
        line.push_str(&format!("（{}）", tops.join("、")));
    }
    if let Some(l) = &pulse.latest {
        line.push_str(&format!("；最新：{} {}「{}」", l.repo, l.date, l.subject));
    }
    if !pulse.quiet_30d.is_empty() {
        line.push_str(&format!("；30 天沉默：{}", pulse.quiet_30d.join("、")));
    }
    line
}

/// 對應 `collect.format_table`。
pub fn format_table(states: &[RepoState]) -> String {
    let mut lines = vec![format!(
        "{:<24} {:<8} {:<12} {:>5} {:>7} {:>9} {:>3} {:>3} {:>7} {:>6} {:>3} {:<10} note",
        "repo",
        "tier",
        "branch",
        "dirty",
        "dirty天",
        "末commit天",
        "7d",
        "30d",
        "↑未push",
        "↓落後",
        "wt",
        "agent"
    )];
    let opt = |v: Option<String>| v.unwrap_or_else(|| "-".into());
    for s in states {
        let dd = opt(s.dirty_days.map(|d| format!("{d:.1}")));
        let lc = opt(s.last_commit_days.map(|d| format!("{d:.1}")));
        let ah = opt(s.ahead.map(|v| v.to_string()));
        let bh = opt(s.behind.map(|v| v.to_string()));
        let br = take_chars(
            s.branch.as_deref().filter(|b| !b.is_empty()).unwrap_or("-"),
            12,
        );
        let wt = s.worktrees.to_string();
        let ag = if s.agents.is_empty() {
            "-".to_string()
        } else {
            s.agents.join(",")
        };
        let c7 = opt(s.commits_7d.map(|v| v.to_string()));
        let c30 = opt(s.commits_30d.map(|v| v.to_string()));
        lines.push(format!(
            "{:<24} {:<8} {:<12} {:>5} {:>7} {:>9} {:>3} {:>3} {:>7} {:>6} {:>3} {:<10} {}",
            s.id, s.tier, br, s.dirty, dd, lc, c7, c30, ah, bh, wt, ag, s.note
        ));
    }
    lines.join("\n")
}
