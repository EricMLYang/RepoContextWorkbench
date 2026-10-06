//! P9 生 spawn——兩段式升格＋出生登記（含血統）。對應 Python `repo_context/spawn.py`。
//!
//! 段一＝素材落 incubator（P8 route dest=incubator，已有）；
//! 段二＝本模組：incubator 素材升格成獨立 git repo → registry 出生登記（origin 血統）
//! → 脊椎 `decision [spawn]` 事件；origin 是 collision 時另落 `outcome` 回連
//! （碰撞→產出的反向追溯，靈感命中率的分子，v2 §3.2）。
//! 新 repo 自己的 git init/commit 不受「spine 單一提交者」約束——那條守的是 spine repo 的 git。

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{Local, NaiveDateTime};

use crate::registry::{self, AddRepo};
use crate::{spine, Error, Result};

fn run_git(cwd: &Path, args: &[&str]) -> Result<()> {
    let mut c = Command::new("git");
    c.arg("-C").arg(cwd).args(args);
    // 沒設身分的機器也要 commit 得出去（環境已設就用環境的）
    for (k, v) in [
        ("GIT_AUTHOR_NAME", "repo_context"),
        ("GIT_COMMITTER_NAME", "repo_context"),
        ("GIT_AUTHOR_EMAIL", "repo_context@local"),
        ("GIT_COMMITTER_EMAIL", "repo_context@local"),
    ] {
        if std::env::var_os(k).is_none() {
            c.env(k, v);
        }
    }
    let o = c.output()?;
    if !o.status.success() {
        return Err(Error::External(format!(
            "git {} 失敗：{}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        )));
    }
    Ok(())
}

fn move_file(src: &Path, dst: &Path) -> Result<()> {
    if std::fs::rename(src, dst).is_err() {
        // 跨檔案系統：複製後刪除（同 shutil.move）
        std::fs::copy(src, dst)?;
        std::fs::remove_file(src)?;
    }
    Ok(())
}

/// 對應 `spawn.spawn`：升格——建 repo、搬素材、登記血統、落事件。回傳新 repo 路徑。
pub fn spawn(
    spine_dir: impl AsRef<Path>,
    new_id: &str,
    target_path: impl AsRef<Path>,
    from_incubator: Option<&str>,
    origin: Option<&str>,
    when: Option<NaiveDateTime>,
) -> Result<PathBuf> {
    let spine_dir = spine_dir.as_ref();
    let now = when.unwrap_or_else(|| Local::now().naive_local());
    let target = crate::pystr::expanduser(target_path.as_ref());
    if target.exists() && std::fs::read_dir(&target)?.next().is_some() {
        return Err(Error::invalid(format!(
            "目標已存在且非空: {}",
            target.display()
        )));
    }
    let from_incubator = from_incubator.filter(|s| !s.is_empty());
    let origin = origin.filter(|s| !s.is_empty());
    let mut src = None;
    if let Some(f) = from_incubator {
        let p = spine_dir.join("incubator").join(f);
        if !p.is_file() {
            return Err(Error::invalid(format!("incubator 素材不存在: {f}")));
        }
        src = Some(p);
    }
    std::fs::create_dir_all(&target)?;
    let lineage = match (origin, from_incubator) {
        (Some(o), _) => o.to_string(),
        (None, Some(f)) => format!("incubator/{f}"),
        (None, None) => "manual".to_string(),
    };
    std::fs::write(
        target.join("README.md"),
        format!(
            "# {new_id}\n\n出生：{}｜血統：{lineage}\n",
            now.format("%Y-%m-%d")
        ),
    )?;
    if let Some(src) = &src {
        if let Some(name) = src.file_name() {
            move_file(src, &target.join(name))?; // 搬離 incubator＝升格完成
        }
    }
    let st = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&target)
        .output()?;
    if !st.status.success() {
        return Err(Error::External(format!(
            "git init 失敗：{}",
            target.display()
        )));
    }
    run_git(&target, &["add", "-A"])?;
    run_git(
        &target,
        &["commit", "-q", "-m", &format!("birth: {lineage}")],
    )?;
    registry::add_repo(
        spine_dir,
        new_id,
        &target,
        AddRepo {
            origin: origin.map(str::to_string),
            ..Default::default()
        },
    )?;
    spine::append_event(
        spine_dir,
        "decision",
        "spawn",
        &[format!("repo:{new_id}")],
        &format!("出生登記：{new_id} ← {lineage} → {}", target.display()),
        Some(now),
    )?;
    if let Some(o) = origin.filter(|o| o.starts_with("collision:")) {
        spine::append_event(
            spine_dir,
            "outcome",
            "spawn",
            &[format!("ref:{o}"), format!("repo:{new_id}")],
            &format!("長成了 repo:{new_id}"),
            Some(now),
        )?;
    }
    Ok(target)
}
