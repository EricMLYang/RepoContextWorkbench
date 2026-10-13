//! repo 狀態卡（2026-09-25 跨 repo 參考輪）——「這個 repo 現在怎樣」一張卡看完。對應 `repo_context/repocard.py`。
//!
//! pulse 只給 commit 主旨，看不出 repo 的狀況。這張卡依「對使用者多有用」排：
//! 1. 你上次看過之後的變化（水位線記在 `.state/seen/<id>.json`；沒看過＝最近 5 則）
//! 2. 放著沒 commit 的改動與放了幾天（忘了收尾的工作藏在這）
//! 3. 變化集中在哪些目錄（numstat 依頂層目錄彙總，比主旨更說得出在忙什麼）
//! 4. 正在跑的 dev server／程序，以及它啟動後有沒有檔案又改過（「要不要重開」）
//! 5. agent 最後一次交接／沒交接就結束的會話
//! 6. agent 文件裡指向別的 repo 卻失效的路徑、引用過的上游有沒有變
//! 7. 還沒 push 的 commit
//!
//! 水位線只有人看卡（CLI 不帶 --peek）才前進；agent 透過 MCP 讀不動它——那是人的「已讀」。
//!
//! 程序偵測照 Python：`ps -axo pid=,lstart=,command=` 列程序、`lsof -a -d cwd -Fpn` 找 cwd 在 repo 內的；
//! 沒有 ps／lsof（Windows）就回空。與 Python 的差異：自身程序的標記多認 `/ctx `（Rust 版 `ctx` 以完整路徑啟動時）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{Local, NaiveDateTime, TimeZone};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use crate::collect::{self, RepoSpec};
use crate::crossref::{self, git, BrokenRef, Drift, GIT_TIMEOUT};
use crate::fsutil::{expanduser, mtime_secs, now_secs, resolve as py_resolve};
use crate::pystr::take_chars;
use crate::{registry, spine, Error, Result};

const DEV_NAMES: &[&str] = &[
    "node", "python", "python3", "bun", "deno", "ruby", "java", "uvicorn", "gunicorn", "php",
    "dotnet", "go", "cargo", "flask", "rails", "hugo",
];
const SELF_MARKERS: &[&str] = &["repo_context", " ctx ", "/ctx "];

// ---------------------------------------------------------------- 水位線

fn seen_path(spine_dir: &Path, repo_id: &str) -> PathBuf {
    let safe: String = repo_id
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    spine_dir
        .join(".state")
        .join("seen")
        .join(format!("{safe}.json"))
}

/// 水位線（`.state/seen/<id>.json`）：`{head, at}`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen {
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub at: String,
}

/// 對應 `repocard.get_seen`（沒有或壞掉＝None）。
pub fn get_seen(spine_dir: &Path, repo_id: &str) -> Option<Seen> {
    std::fs::read_to_string(seen_path(spine_dir, repo_id))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

/// 對應 `repocard.mark_seen`：把水位線推到 head。
pub fn mark_seen(
    spine_dir: &Path,
    repo_id: &str,
    head: &str,
    now: Option<NaiveDateTime>,
) -> Result<()> {
    let now = now.unwrap_or_else(|| Local::now().naive_local());
    let p = seen_path(spine_dir, repo_id);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let v = serde_json::json!({"head": head, "at": now.format("%Y-%m-%d %H:%M").to_string()});
    std::fs::write(&p, crate::pyjson::dumps(&v))?;
    Ok(())
}

// ---------------------------------------------------------------- 各段

/// 一則 commit。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Commit {
    pub hash: String,
    pub date: String,
    pub subject: String,
}

fn commits(root: &Path, rng: Option<&str>, limit: usize) -> Option<Vec<Commit>> {
    let n = format!("-{limit}");
    let mut args = vec![
        "log",
        n.as_str(),
        "--format=%h%x09%ad%x09%s",
        "--date=short",
    ];
    if let Some(r) = rng {
        args.push(r);
    }
    let (rc, out) = git(root, &args, GIT_TIMEOUT);
    if rc != 0 {
        return None;
    }
    Some(
        out.lines()
            .filter_map(|ln| {
                let p: Vec<&str> = ln.splitn(3, '\t').collect();
                (p.len() == 3).then(|| Commit {
                    hash: p[0].into(),
                    date: p[1].into(),
                    subject: p[2].into(),
                })
            })
            .collect(),
    )
}

/// 改動依頂層目錄彙總的一列。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Area {
    pub area: String,
    pub files: usize,
    pub added: u64,
    pub deleted: u64,
}

/// 對應 `repocard.areas`：rng（例 `abc123..HEAD` 或 `--since=7.days`）內的改動依頂層目錄彙總，改動量大的在前。
pub fn areas(root: &Path, rng: &[String]) -> Vec<Area> {
    let mut args = vec!["log", "--numstat", "--format="];
    args.extend(rng.iter().map(String::as_str));
    let (rc, out) = git(root, &args, GIT_TIMEOUT);
    if rc != 0 || out.is_empty() {
        return Vec::new();
    }
    let mut agg: Vec<(String, HashSet<String>, u64, u64)> = Vec::new();
    for ln in out.lines() {
        let p: Vec<&str> = ln.split('\t').collect();
        if p.len() != 3 {
            continue;
        }
        let f = p[2]
            .rsplit(" => ")
            .next()
            .unwrap_or(p[2])
            .trim_matches(['{', '}'])
            .to_string();
        let top = match f.split_once('/') {
            Some((t, _)) => format!("{t}/"),
            None => "（根目錄）".into(),
        };
        let num = |s: &str| {
            if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
                s.parse().unwrap_or(0)
            } else {
                0
            }
        };
        let i = match agg.iter().position(|x| x.0 == top) {
            Some(i) => i,
            None => {
                agg.push((top, HashSet::new(), 0, 0));
                agg.len() - 1
            }
        };
        agg[i].1.insert(f);
        agg[i].2 += num(p[0]);
        agg[i].3 += num(p[1]);
    }
    let mut rows: Vec<Area> = agg
        .into_iter()
        .map(|(area, files, added, deleted)| Area {
            area,
            files: files.len(),
            added,
            deleted,
        })
        .collect();
    rows.sort_by_key(|x| std::cmp::Reverse(x.added + x.deleted + x.files as u64));
    rows
}

/// 沒 commit 的一個檔。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DirtyFile {
    pub file: String,
    pub status: String,
    pub days: Option<f64>,
}

fn round1(x: f64) -> f64 {
    format!("{x:.1}").parse().unwrap_or(x)
}

/// 對應 `repocard.dirty_files`：放最久的在前。`now`＝Unix 秒（沒給＝現在）。
pub fn dirty_files(root: &Path, now: Option<f64>) -> Vec<DirtyFile> {
    let now = now.unwrap_or_else(now_secs);
    let (rc, out) = git(root, &["status", "--porcelain"], GIT_TIMEOUT);
    if rc != 0 {
        return Vec::new();
    }
    let mut rows = Vec::new();
    for ln in out.lines() {
        if ln.chars().count() < 4 {
            continue;
        }
        let status = take_chars(ln, 2).trim();
        let status = if status.is_empty() { "?" } else { status };
        let rest: String = ln.chars().skip(3).collect();
        let f = rest
            .rsplit(" -> ")
            .next()
            .unwrap_or(&rest)
            .trim_matches('"')
            .to_string();
        let days = mtime_secs(&root.join(&f))
            .ok()
            .map(|m| round1((now - m) / 86400.0));
        rows.push(DirtyFile {
            file: f,
            status: status.to_string(),
            days,
        });
    }
    rows.sort_by(|a, b| b.days.unwrap_or(0.0).total_cmp(&a.days.unwrap_or(0.0)));
    rows
}

/// `ps` 的一列：(pid, 啟動時間, 指令列)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsRow {
    pub pid: i64,
    pub start: NaiveDateTime,
    pub command: String,
}

/// Python `s.split(None, maxsplit)`。
fn split_ws(s: &str, maxsplit: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s.trim_start();
    while !rest.is_empty() {
        if out.len() == maxsplit {
            out.push(rest);
            break;
        }
        match rest.find(char::is_whitespace) {
            Some(i) => {
                out.push(&rest[..i]);
                rest = rest[i..].trim_start();
            }
            None => {
                out.push(rest);
                break;
            }
        }
    }
    out
}

/// 對應 `_ps_rows`：沒有 ps（Windows）回空。
pub fn ps_rows() -> Vec<PsRow> {
    let Ok(o) = Command::new("ps")
        .args(["-axo", "pid=,lstart=,command="])
        .output()
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&o.stdout);
    let mut rows = Vec::new();
    for ln in text.lines() {
        let p = split_ws(ln, 6);
        if p.len() < 7 || !p[0].chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        // lstart 例：Fri Oct  2 20:20:27 2026（日不補零）
        let stamp = format!("{} {} {:0>2} {} {}", p[1], p[2], p[3], p[4], p[5]);
        let Ok(start) = NaiveDateTime::parse_from_str(&stamp, "%a %b %d %H:%M:%S %Y") else {
            continue;
        };
        let Ok(pid) = p[0].parse() else { continue };
        rows.push(PsRow {
            pid,
            start,
            command: p[6].to_string(),
        });
    }
    rows
}

/// 對應 `_cwd_pids`：cwd 在 root 底下的程序 pid（lsof；沒有 lsof 回空集合）。
pub fn cwd_pids(root: &Path) -> HashSet<i64> {
    let Ok(o) = Command::new("lsof")
        .args(["-a", "-d", "cwd", "-Fpn"])
        .output()
    else {
        return HashSet::new();
    };
    let text = String::from_utf8_lossy(&o.stdout);
    let rs = root.to_string_lossy().into_owned();
    let sep = std::path::MAIN_SEPARATOR;
    let mut pids = HashSet::new();
    let mut pid: Option<i64> = None;
    for ln in text.lines() {
        if let Some(x) = ln.strip_prefix('p') {
            pid = x.parse().ok();
        } else if let Some(n) = ln.strip_prefix('n') {
            if let Some(p) = pid.filter(|p| *p != 0) {
                if n == rs || n.starts_with(&format!("{rs}{sep}")) {
                    pids.insert(p);
                }
            }
        }
    }
    pids
}

/// 跟 repo 有關的 dev 程序一筆。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Running {
    pub pid: i64,
    pub started: String,
    pub command: String,
    pub changed_since_start: usize,
    pub changed_files: Vec<String>,
}

fn local_ts(dt: NaiveDateTime) -> f64 {
    Local
        .from_local_datetime(&dt)
        .earliest()
        .map(|t| t.timestamp() as f64)
        .unwrap_or_else(|| dt.and_utc().timestamp() as f64)
}

/// 對應 `repocard.running`：跟這個 repo 有關的 dev 程序——指令列含 repo 路徑，或 cwd 在 repo 內且是常見 runtime。
/// 每個帶 started 與「啟動後改過的 tracked 檔」——回答「改了要不要重開」。
/// `ps`／`cwd` 沒給就現場偵測（測試注入假資料用）。
pub fn running(root: &Path, ps: Option<Vec<PsRow>>, cwd: Option<HashSet<i64>>) -> Vec<Running> {
    let root = py_resolve(root);
    let rows = ps.unwrap_or_else(ps_rows);
    let cwd = cwd.unwrap_or_else(|| cwd_pids(&root));
    let me = i64::from(std::process::id());
    let rs = root.to_string_lossy().into_owned();
    let sep = std::path::MAIN_SEPARATOR;
    let mut found: Vec<(Running, NaiveDateTime)> = Vec::new();
    for r in rows {
        let cmd = &r.command;
        let padded = format!(" {cmd} ");
        if r.pid == me || SELF_MARKERS.iter().any(|m| padded.contains(m)) {
            continue;
        }
        let first = cmd.split_whitespace().next().unwrap_or("");
        if first.contains("/node_modules/") {
            continue; // esbuild 之類的子程序：主程序（node …/vite）已代表它
        }
        let exe = Path::new(first)
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let by_cmd = cmd.contains(&format!("{rs}{sep}")) || cmd.ends_with(&rs);
        let by_cwd = cwd.contains(&r.pid)
            && (DEV_NAMES.contains(&exe.as_str())
                || DEV_NAMES.iter().any(|n| {
                    format!("{cmd} ").contains(&format!("/{n} "))
                        || cmd.starts_with(&format!("{n} "))
                }));
        if !(by_cmd || by_cwd) {
            continue;
        }
        found.push((
            Running {
                pid: r.pid,
                started: r.start.format("%Y-%m-%d %H:%M").to_string(),
                command: take_chars(cmd, 160).to_string(),
                changed_since_start: 0,
                changed_files: Vec::new(),
            },
            r.start,
        ));
    }
    if found.is_empty() {
        return Vec::new();
    }
    let (rc, out) = git(&root, &["ls-files", "-z"], GIT_TIMEOUT);
    let tracked: Vec<&str> = if rc == 0 {
        out.split('\0').filter(|f| !f.is_empty()).collect()
    } else {
        Vec::new()
    };
    let mtimes: Vec<(f64, &str)> = tracked
        .iter()
        .filter_map(|f| mtime_secs(&root.join(f)).ok().map(|m| (m, *f)))
        .collect();
    found
        .into_iter()
        .map(|(mut p, start)| {
            let ts = local_ts(start);
            let mut changed: Vec<(f64, &str)> =
                mtimes.iter().copied().filter(|(m, _)| *m > ts).collect();
            changed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(b.1)));
            p.changed_since_start = changed.len();
            p.changed_files = changed
                .iter()
                .rev()
                .take(5)
                .map(|(_, f)| f.to_string())
                .collect();
            p
        })
        .collect()
}

/// 最後一次交接。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Handoff {
    pub at: String,
    pub source: String,
    pub text: String,
}

/// 沒交接就結束的會話（hook 留的 suggestion）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unhanded {
    pub at: String,
    pub text: String,
}

fn last_agent(spine_dir: &Path, repo_id: &str) -> Result<(Option<Handoff>, Option<Unhanded>)> {
    let (mut handoff, mut unhanded) = (None, None);
    for ev in spine::iter_events(spine_dir)? {
        if ev.kv("repo") != Some(repo_id) {
            continue;
        }
        let first = ev.first_line().to_string();
        let at = ev.stamp();
        if ev.etype == spine::EventType::Decision && first.starts_with("交接") {
            handoff = Some(Handoff {
                at,
                source: ev.source.clone(),
                text: first.chars().skip(3).collect::<String>().trim().to_string(),
            });
        } else if ev.etype == spine::EventType::Suggestion && ev.source == "hook" {
            unhanded = Some(Unhanded { at, text: first });
        }
    }
    Ok((handoff, unhanded))
}

// ---------------------------------------------------------------- 組卡

/// 變化視窗的種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowKind {
    /// 上次看過之後
    Seen,
    /// 最近 5 則
    Recent,
    /// 近 N 天
    Days,
    /// 某個 commit 之後
    Commit,
}

/// 變化視窗。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Window {
    pub kind: WindowKind,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
}

/// 卡的明細段（repo 存在、是 git repo、不是 external 才有）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CardDetail {
    pub head: Option<String>,
    pub window: Window,
    pub new_commits: Vec<Commit>,
    pub areas: Vec<Area>,
    pub dirty: Vec<DirtyFile>,
    pub running: Vec<Running>,
    pub last_handoff: Option<Handoff>,
    pub last_unhanded: Option<Unhanded>,
    pub broken_refs: Vec<BrokenRef>,
    pub drift: Vec<Drift>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marked_seen: Option<String>,
}

/// 狀態卡（鍵名、順序同 Python dict）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Card {
    pub ok: bool,
    pub repo: String,
    pub path: String,
    pub tier: String,
    pub exists: bool,
    pub branch: Option<String>,
    pub note: String,
    pub ahead: Option<i64>,
    pub behind: Option<i64>,
    pub commits_7d: Option<u64>,
    pub commits_30d: Option<u64>,
    pub last_commit_days: Option<f64>,
    #[serde(flatten)]
    pub detail: Option<CardDetail>,
}

static DAYS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+d$").expect("寫死的 regex"));

/// 程序偵測的注入點（測試給假資料；None＝現場偵測）。
#[derive(Debug, Clone, Default)]
pub struct ProcProbe {
    pub ps_rows: Option<Vec<PsRow>>,
    pub cwd_pids: Option<HashSet<i64>>,
}

/// 對應 `repocard.build`：一個 repo 的狀態卡。since＝`seen`（預設：上次看過之後）｜`recent`（最近 5 則）｜
/// `7d` 之類｜commit hash。mark=true：看完把水位線推到目前 HEAD（人看卡才推；agent 不推）。
pub fn build(
    spine_dir: &Path,
    repo_id: &str,
    since: Option<&str>,
    mark: bool,
    now: Option<NaiveDateTime>,
    probe: ProcProbe,
) -> Result<Card> {
    let entry = registry::get_repo(spine_dir, repo_id)?;
    let st = collect::collect_repo(&RepoSpec::from(&entry));
    let root = expanduser(entry.path_str());
    let mut card = Card {
        ok: true,
        repo: repo_id.to_string(),
        path: root.to_string_lossy().into_owned(),
        tier: entry.tier_str().unwrap_or_else(|| "active".into()),
        exists: st.exists,
        branch: st.branch.clone(),
        note: st.note.clone(),
        ahead: st.ahead,
        behind: st.behind,
        commits_7d: st.commits_7d,
        commits_30d: st.commits_30d,
        last_commit_days: st.last_commit_days,
        detail: None,
    };
    if !st.exists || st.note == "非 git repo" || entry.kind().as_deref() == Some("external") {
        return Ok(card);
    }
    let (rc, head) = git(&root, &["rev-parse", "--short", "HEAD"], GIT_TIMEOUT);
    let head = (rc == 0).then_some(head);
    let seen = get_seen(spine_dir, repo_id);
    let since = since.filter(|s| !s.is_empty()).unwrap_or("seen");
    let recent5 = |root: &Path| commits(root, None, 5).unwrap_or_default();
    let (window, new_commits, rng) = match (
        since,
        seen.as_ref().and_then(|s| {
            s.head
                .clone()
                .filter(|h| !h.is_empty())
                .map(|h| (h, s.at.clone()))
        }),
    ) {
        ("seen", Some((sh, sat))) => {
            let range = format!("{sh}..HEAD");
            match commits(&root, Some(&range), 30) {
                // 水位線 commit 不見了（rebase）→ 退回最近 5 則
                None => (
                    Window {
                        kind: WindowKind::Recent,
                        label: "上次看過的 commit 已不在歷史裡；列最近 5 則".into(),
                        since: None,
                    },
                    recent5(&root),
                    vec!["-5".to_string()],
                ),
                Some(c) => (
                    Window {
                        kind: WindowKind::Seen,
                        label: format!("你上次看是 {sat}（{sh}）"),
                        since: Some(sat),
                    },
                    c,
                    vec![range],
                ),
            }
        }
        ("seen" | "recent", _) => (
            Window {
                kind: WindowKind::Recent,
                label: if since == "seen" {
                    "第一次看這個 repo；列最近 5 則".into()
                } else {
                    "最近 5 則".into()
                },
                since: None,
            },
            recent5(&root),
            vec!["-5".to_string()],
        ),
        (s, _) if DAYS_RE.is_match(s) => {
            let n = &s[..s.len() - 1];
            let arg = format!("--since={n}.days");
            (
                Window {
                    kind: WindowKind::Days,
                    label: format!("近 {n} 天"),
                    since: None,
                },
                commits(&root, Some(&arg), 30).unwrap_or_default(),
                vec![arg],
            )
        }
        (s, _) => {
            let range = format!("{s}..HEAD");
            let c = commits(&root, Some(&range), 30)
                .ok_or_else(|| Error::invalid(format!("找不到 commit：{s}")))?;
            (
                Window {
                    kind: WindowKind::Commit,
                    label: format!("{s} 之後"),
                    since: None,
                },
                c,
                vec![range],
            )
        }
    };
    let areas = if new_commits.is_empty() {
        Vec::new()
    } else {
        areas(&root, &rng)
    };
    let (last_handoff, last_unhanded) = last_agent(spine_dir, repo_id)?;
    let mut detail = CardDetail {
        head: head.clone(),
        window,
        new_commits,
        areas,
        dirty: dirty_files(&root, None),
        running: running(&root, probe.ps_rows, probe.cwd_pids),
        last_handoff,
        last_unhanded,
        broken_refs: crossref::lint_doc_refs(spine_dir, Some(&[repo_id.to_string()]))?,
        drift: crossref::drift(spine_dir, Some(repo_id), None)?,
        marked_seen: None,
    };
    if let Some(h) = head.filter(|_| mark) {
        mark_seen(spine_dir, repo_id, &h, now)?;
        detail.marked_seen = Some(h);
    }
    card.detail = Some(detail);
    Ok(card)
}

/// 對應 `repocard.render`：狀態卡 → 一屏內的人話。沒事的段落不印（無事不報）。
pub fn render(card: &Card) -> String {
    if !card.exists {
        return format!("# {}\n路徑不存在：{}", card.repo, card.path);
    }
    let mut out = vec![format!(
        "# {}（{}，{}）  {}",
        card.repo,
        card.branch
            .as_deref()
            .filter(|b| !b.is_empty())
            .unwrap_or("-"),
        card.tier,
        card.path
    )];
    if !card.note.is_empty() {
        out.push(card.note.clone());
    }
    let Some(d) = &card.detail else {
        return out.join("\n");
    };
    let nc = &d.new_commits;
    out.push(format!("\n## 變化：{}", d.window.label));
    if nc.is_empty() {
        out.push("沒有新 commit。".into());
    }
    for c in nc.iter().take(10) {
        out.push(format!("- {} {} {}", c.date, c.hash, c.subject));
    }
    if nc.len() > 10 {
        out.push(format!("- …另 {} 則", nc.len() - 10));
    }
    if !d.areas.is_empty() {
        out.push(format!(
            "改動集中在：{}",
            d.areas
                .iter()
                .take(5)
                .map(|a| format!("{} {} 檔 +{}/-{}", a.area, a.files, a.added, a.deleted))
                .collect::<Vec<_>>()
                .join("、")
        ));
    }
    if !d.dirty.is_empty() {
        let old = d.dirty.iter().any(|x| x.days.unwrap_or(0.0) >= 1.0);
        out.push(format!(
            "\n## 沒 commit 的改動：{} 個檔{}",
            d.dirty.len(),
            if old {
                format!("（放最久 {:.0} 天）", d.dirty[0].days.unwrap_or(0.0))
            } else {
                String::new()
            }
        ));
        for x in d.dirty.iter().take(6) {
            let age = x
                .days
                .map(|v| format!("{v:.1} 天"))
                .unwrap_or_else(|| "?".into());
            out.push(format!("- [{}] {}（{age}）", x.status, x.file));
        }
        if d.dirty.len() > 6 {
            out.push(format!("- …另 {} 個", d.dirty.len() - 6));
        }
    }
    if !d.running.is_empty() {
        out.push("\n## 正在跑".into());
        for p in &d.running {
            let mut line = format!("- pid {}，{} 啟動：{}", p.pid, p.started, p.command);
            if p.changed_since_start > 0 {
                line.push_str(&format!(
                    "\n  啟動後改過 {} 個檔（最近：{}）——不會自動重載的部分要重開",
                    p.changed_since_start,
                    p.changed_files
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("、")
                ));
            } else {
                line.push_str("\n  啟動後沒有 tracked 檔改過");
            }
            out.push(line);
        }
    }
    if let Some(a) = card.ahead.filter(|a| *a != 0) {
        out.push(format!("\n還沒 push：{a} 個 commit"));
    }
    let (lh, lu) = (&d.last_handoff, &d.last_unhanded);
    if lh.is_some() || lu.is_some() {
        out.push("\n## agent".into());
        if let Some(h) = lh {
            out.push(format!("- 最後交接 {}（{}）：{}", h.at, h.source, h.text));
        }
        if let Some(u) = lu {
            if lh.as_ref().is_none_or(|h| u.at > h.at) {
                out.push(format!("- {} {}", u.at, u.text));
            }
        }
    }
    if !d.broken_refs.is_empty() {
        out.push(format!(
            "\n## agent 文件裡失效的跨 repo 路徑：{} 處",
            d.broken_refs.len()
        ));
        for b in d.broken_refs.iter().take(5) {
            out.push(format!(
                "- {}:{} `{}` → {}",
                b.doc, b.line, b.reference, b.fix
            ));
        }
    }
    if !d.drift.is_empty() {
        out.push("\n## 引用過的上游有變".into());
        out.extend(crossref::drift_lines(&d.drift, 5));
    }
    if let Some(m) = &d.marked_seen {
        out.push(format!(
            "\n（已記下看到 {m}；下次只列之後的變化。--peek 不記）"
        ));
    }
    out.join("\n")
}
