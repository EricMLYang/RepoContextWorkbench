//! 掃描與稽核：找未登記 repo、tier 漂移、路徑失效、關係斷鏈。對應 `registry/audit.py`。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_yaml::Value;

use super::repos::{add_repo, AddRepo};
use super::store::load;
use crate::fsutil::{expanduser, resolve};
use crate::pystr::py_str_yaml;
use crate::{Error, Result};

fn git(repo_path: &Path, args: &[&str]) -> (bool, String) {
    match Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .output()
    {
        Ok(o) => (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).trim().to_string(),
        ),
        Err(_) => (false, String::new()),
    }
}

/// 對應 `registry.last_commit_days`：最後一個 commit 距今幾天（非 git repo／無 commit＝None）。
pub fn last_commit_days(repo_path: impl AsRef<Path>) -> Option<f64> {
    let (ok, out) = git(repo_path.as_ref(), &["log", "-1", "--format=%ct"]);
    if !ok || out.is_empty() {
        return None;
    }
    let ct: i64 = out.parse().ok()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    Some((now - ct as f64) / 86400.0)
}

fn known_paths(repos: &[super::store::Repo]) -> HashSet<PathBuf> {
    repos
        .iter()
        .map(|r| {
            let p = r.path();
            if p.exists() {
                resolve(&p)
            } else {
                p
            }
        })
        .collect()
}

fn sorted_children(base: &Path) -> Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(base)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    v.sort();
    Ok(v)
}

/// 對應 `registry.scan_dir`：mani「init 自動掃描」課（拆機報告 §2.2）——找 base_dir 下含 .git 的資料夾，
/// 回傳未登記候選 [(path, 建議id)]。id 撞名時加序號（同名資料夾登在別處的情況）。
pub fn scan_dir(
    spine_dir: impl AsRef<Path>,
    base_dir: impl AsRef<Path>,
) -> Result<Vec<(PathBuf, String)>> {
    scan(spine_dir.as_ref(), base_dir.as_ref(), false)
}

/// Rust 才有（2026-10-11，App「加入 repo」）：同 [`scan_dir`]，但 base_dir 本身就是未登記的 git repo 時排第一個。
pub fn scan_dir_with_self(
    spine_dir: impl AsRef<Path>,
    base_dir: impl AsRef<Path>,
) -> Result<Vec<(PathBuf, String)>> {
    scan(spine_dir.as_ref(), base_dir.as_ref(), true)
}

fn scan(spine_dir: &Path, base_dir: &Path, with_self: bool) -> Result<Vec<(PathBuf, String)>> {
    let data = load(spine_dir)?;
    let known = known_paths(&data.repos);
    let mut used: HashSet<String> = data.repos.iter().map(|r| r.id()).collect();
    let base = expanduser(base_dir);
    if !base.is_dir() {
        return Err(Error::invalid(format!(
            "目錄不存在: {}",
            base_dir.display()
        )));
    }
    let mut out = Vec::new();
    let mut dirs = sorted_children(&base)?;
    if with_self {
        dirs.insert(0, base.clone());
    }
    for child in dirs {
        if !(child.is_dir() && child.join(".git").exists()) || known.contains(&resolve(&child)) {
            continue;
        }
        let name = child
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (mut cid, mut n) = (name.clone(), 2);
        while used.contains(&cid) {
            cid = format!("{name}-{n}");
            n += 1;
        }
        used.insert(cid.clone());
        out.push((child, cid));
    }
    Ok(out)
}

/// 對應 `registry.scan_register`：混合模式的「自動下半身」——掃描到的 repo 以預設值登記（mine/active/無 tag）；
/// 上半身（type/tier/tags/血統/關係）留人手寫——狀態欄位機器填，語意我填。回傳新登記的 id。
pub fn scan_register(
    spine_dir: impl AsRef<Path>,
    base_dir: impl AsRef<Path>,
) -> Result<Vec<String>> {
    let spine_dir = spine_dir.as_ref();
    let mut added = Vec::new();
    for (path, cid) in scan_dir(spine_dir, base_dir)? {
        add_repo(spine_dir, &cid, &path, AddRepo::default())?;
        added.push(cid);
    }
    Ok(added)
}

/// Rust 才有（2026-10-11，App「加入資料夾」，像 IDE 的 Add Folder to Workspace）：一個 git repo 資料夾 →
/// 已登記（同一實體路徑）就回它的 id，沒有就以資料夾名登記（撞名加序號）。回 (id, 這次新登記)。
pub fn add_folder(spine_dir: impl AsRef<Path>, folder: impl AsRef<Path>) -> Result<(String, bool)> {
    let spine_dir = spine_dir.as_ref();
    let path = expanduser(folder.as_ref());
    if !path.is_dir() {
        return Err(Error::invalid(format!(
            "資料夾不存在: {}",
            folder.as_ref().display()
        )));
    }
    let real = resolve(&path);
    let data = load(spine_dir)?;
    if let Some(r) = data.repos.iter().find(|r| resolve(&r.path()) == real) {
        return Ok((r.id(), false));
    }
    if !path.join(".git").exists() {
        let n = scan_dir(spine_dir, &path).map(|v| v.len()).unwrap_or(0);
        return Err(Error::invalid(if n > 0 {
            format!("這個資料夾不是 git repo；下一層有 {n} 個未登記的 repo，請改用「掃描資料夾」一次加入")
        } else {
            "這個資料夾不是 git repo".to_string()
        }));
    }
    let name = real
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    let (mut id, mut n) = (name.clone(), 2);
    while data.repo(&id).is_some() {
        id = format!("{name}-{n}");
        n += 1;
    }
    add_repo(spine_dir, &id, &path, AddRepo::default())?;
    Ok((id, true))
}

/// 對應 `registry.audit(spine, scan_dirs)`（active_max_days=30、dormant_min_days=7）。
pub fn audit<P: AsRef<Path>>(spine_dir: impl AsRef<Path>, scan_dirs: &[P]) -> Result<Vec<String>> {
    audit_with(spine_dir, scan_dirs, 30.0, 7.0)
}

fn fmt0(days: f64) -> String {
    format!("{days:.0}")
}

/// 對應 `registry.audit`：紅字清單。scan_dirs：額外掃「資料夾在但 registry 沒有」的洞。
/// 反向漂移（2026-09-08 真資料實錘：MI 線 PM repo 掛 active 沉默 66 天、code repo 掛 dormant
/// 卻 7 天 4 commits）：dormant 但 dormant_min_days 內有 commit 也算紅字——活動才是真相。
pub fn audit_with<P: AsRef<Path>>(
    spine_dir: impl AsRef<Path>,
    scan_dirs: &[P],
    active_max_days: f64,
    dormant_min_days: f64,
) -> Result<Vec<String>> {
    let sd = spine_dir.as_ref();
    let data = load(sd)?;
    let mut reds = Vec::new();
    // Rust 才有（2026-10-21）：App 開會話用的 .mcp.json 指到的 ctx 不在（10-18 重裝後指到被刪的 bundle，
    // 每場會話 MCP 都連不上、兩天沒人發現）——放第一條，App 側欄「登記檢查」就看得到
    let mcp = crate::session::mcp_json_status(sd);
    if mcp["status"] == "command_missing" {
        reds.push(format!(
            "[App 會話 MCP] .mcp.json 指到的 ctx 不在（{}）：App 開的會話連不上 MCP、agent 只能退回 CLI；下次開會話會自動重寫",
            mcp["command"].as_str().unwrap_or("")
        ));
    }
    let known = known_paths(&data.repos);
    for r in &data.repos {
        let (id, p) = (r.id(), r.path());
        let tier = r.tier_str();
        let mine = r.kind().as_deref() == Some("mine");
        if !p.is_dir() {
            reds.push(format!("[路徑失效] {id}: {} 不存在", r.path_str()));
            continue;
        }
        if tier.as_deref() == Some("paused") && !r.has("resume_when") {
            reds.push(format!(
                "[paused 無 resume_when] {id}：說不出復工條件就不准掛 paused"
            ));
        }
        if tier.as_deref() == Some("active") && mine {
            match last_commit_days(&p) {
                None => reds.push(format!("[非 git repo] {id}: {}", r.path_str())),
                Some(d) if d > active_max_days => reds.push(format!(
                    "[tier 漂移] {id}: active 但 {} 天無 commit",
                    fmt0(d)
                )),
                _ => {}
            }
        }
        if tier.as_deref() == Some("dormant") && mine {
            if let Some(d) = last_commit_days(&p).filter(|d| *d <= dormant_min_days) {
                reds.push(format!(
                    "[tier 漂移] {id}: dormant 但 {} 天前有 commit（該升回 active？）",
                    fmt0(d)
                ));
            }
        }
        if r.kind().as_deref() == Some("external") && !r.has("upstream") {
            reds.push(format!("[external 無 upstream] {id}：P4 無從查起"));
        }
    }
    let ids: HashSet<String> = data.repos.iter().map(|r| r.id()).collect();
    for r in &data.repos {
        let Some(Value::Sequence(rels)) = r.get("relations") else {
            continue;
        };
        for x in rels {
            let to = x
                .get("to")
                .map(py_str_yaml)
                .unwrap_or_else(|| "None".into());
            if x.get("to")
                .and_then(Value::as_str)
                .is_none_or(|t| !ids.contains(t))
            {
                let kind = x
                    .get("kind")
                    .map(py_str_yaml)
                    .unwrap_or_else(|| "None".into());
                reds.push(format!("[關係斷鏈] {} → {to} ({kind})：目標未登記", r.id()));
            }
        }
    }
    for d in scan_dirs {
        let base = expanduser(d.as_ref());
        if !base.is_dir() {
            continue;
        }
        for child in sorted_children(&base)? {
            if child.is_dir() && child.join(".git").exists() && !known.contains(&resolve(&child)) {
                reds.push(format!(
                    "[未登記] {}: 是 git repo 但 registry 沒有",
                    child.display()
                ));
            }
        }
    }
    Ok(reds)
}
