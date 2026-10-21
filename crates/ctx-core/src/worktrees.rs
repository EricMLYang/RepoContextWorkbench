//! 一個 repo 掛出去的 git worktree（2026-10-22，ideation F-107；Rust 才有，參考 BAT 的 worktree handler）。
//!
//! agent 平行工作（Claude Code `--worktree`、sub-agent 的隔離）會在 repo 底下留一堆 worktree，
//! 做完沒人收：2026-10-21 本 repo 的 `.claude/worktrees` 留了 9 個、共 18 GB（#129）。
//! 這裡列出每個 worktree 的分支、有沒有沒 commit 的改動、有沒有「只在這裡」的 commit，
//! 乾淨的才給移除——移除走 `git worktree remove`（不加 `--force`：有改動 git 自己會拒絕），分支留著。

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::{Error, Result};

/// 一個 worktree。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Worktree {
    pub path: String,
    /// 分支名（`refs/heads/` 之後）；detached＝None。
    pub branch: Option<String>,
    /// HEAD 前 8 碼。
    pub head: String,
    /// 主 worktree（repo 本身），不能移除。
    pub main: bool,
    /// 還沒 commit 的檔數（含沒追蹤的）；目錄不在了＝None。
    pub dirty: Option<usize>,
    /// 只在這裡的 commit 數：其他分支、遠端都沒有（移除後只剩 reflog 找得到）。
    pub unique: usize,
    /// git 標了 locked／prunable（目錄已不在）。
    pub locked: bool,
    pub prunable: bool,
}

impl Worktree {
    /// 可以安全移除：不是主 worktree、沒鎖、沒有沒 commit 的改動、沒有只在這裡的 commit。
    pub fn removable(&self) -> bool {
        !self.main && !self.locked && self.dirty.is_none_or(|d| d == 0) && self.unique == 0
    }
}

fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let o = Command::new("git").arg("-C").arg(cwd).args(args).output()?;
    if !o.status.success() {
        return Err(Error::External(format!(
            "git {} 失敗：{}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// `git worktree list --porcelain` 的一段 → (路徑, HEAD, 分支, locked, prunable)。
fn parse(porcelain: &str) -> Vec<(String, String, Option<String>, bool, bool)> {
    porcelain
        .split("\n\n")
        .filter_map(|block| {
            let mut path = None;
            let mut head = String::new();
            let mut branch = None;
            let (mut locked, mut prunable) = (false, false);
            for ln in block.lines() {
                if let Some(p) = ln.strip_prefix("worktree ") {
                    path = Some(p.to_string());
                } else if let Some(h) = ln.strip_prefix("HEAD ") {
                    head = h.chars().take(8).collect();
                } else if let Some(b) = ln.strip_prefix("branch ") {
                    branch = Some(b.trim_start_matches("refs/heads/").to_string());
                } else if ln == "locked" || ln.starts_with("locked ") {
                    locked = true;
                } else if ln == "prunable" || ln.starts_with("prunable ") {
                    prunable = true;
                }
            }
            path.map(|p| (p, head, branch, locked, prunable))
        })
        .collect()
}

/// 這個 HEAD 有幾個 commit 是別的分支（除了它自己的）和遠端都沒有的。
fn unique_commits(root: &Path, head: &str, branch: Option<&str>) -> usize {
    if head.is_empty() {
        return 0;
    }
    // 給 --branches 的樣式不能帶 refs/heads/（git rev-list 文件）
    let exclude = branch.map(|b| format!("--exclude={b}"));
    let mut args = vec!["rev-list", "--count", head, "--not"];
    if let Some(e) = &exclude {
        args.push(e);
    }
    args.extend(["--branches", "--remotes"]);
    git(root, &args)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// 列出 repo 的 worktree（主 worktree 在最前面）。
pub fn list(root: &Path) -> Result<Vec<Worktree>> {
    let out = git(root, &["worktree", "list", "--porcelain"])?;
    Ok(parse(&out)
        .into_iter()
        .enumerate()
        .map(|(i, (path, head, branch, locked, prunable))| {
            let dir = PathBuf::from(&path);
            let dirty = dir
                .is_dir()
                .then(|| git(&dir, &["status", "--porcelain"]).ok())
                .flatten()
                .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count());
            let unique = if i == 0 {
                0
            } else {
                unique_commits(root, &head, branch.as_deref())
            };
            Worktree {
                path,
                branch,
                head,
                main: i == 0,
                dirty,
                unique,
                locked,
                prunable,
            }
        })
        .collect())
}

/// 移除一個 worktree：要在 [`list`] 裡、而且 [`Worktree::removable`]；不加 `--force`，分支留著。
/// 目錄已經不在（prunable）就 `git worktree prune`。
pub fn remove(root: &Path, path: &str) -> Result<()> {
    let all = list(root)?;
    let Some(w) = all.iter().find(|w| w.path == path) else {
        return Err(Error::invalid(format!(
            "這個 repo 沒有這個 worktree：{path}"
        )));
    };
    if w.main {
        return Err(Error::invalid("主 worktree（repo 本身）不能移除"));
    }
    if w.locked {
        return Err(Error::invalid(
            "這個 worktree 被鎖住了（git worktree unlock 之後再移除）",
        ));
    }
    if let Some(n) = w.dirty.filter(|n| *n > 0) {
        return Err(Error::invalid(format!(
            "還有 {n} 個檔沒 commit，先處理再移除"
        )));
    }
    if w.unique > 0 {
        return Err(Error::invalid(format!(
            "有 {} 個 commit 只在這裡（別的分支與遠端都沒有），先合併或推上去再移除",
            w.unique
        )));
    }
    if w.prunable || !Path::new(path).is_dir() {
        git(root, &["worktree", "prune"])?;
    } else {
        git(root, &["worktree", "remove", path])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        let o = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }

    #[test]
    fn parse_reads_branch_detached_locked_prunable() {
        let got = parse(
            "worktree /r\nHEAD 0123456789abcdef\nbranch refs/heads/main\n\n\
             worktree /r/.claude/worktrees/a\nHEAD fedcba9876543210\ndetached\nlocked reason\n\n\
             worktree /gone\nHEAD aaaaaaaa11111111\nbranch refs/heads/x\nprunable gitdir file points to non-existent location\n",
        );
        assert_eq!(got.len(), 3);
        assert_eq!(
            got[0],
            (
                "/r".into(),
                "01234567".into(),
                Some("main".into()),
                false,
                false
            )
        );
        assert_eq!(got[1].2, None);
        assert!(got[1].3);
        assert!(got[2].4);
    }

    #[test]
    fn list_and_remove_only_clean_worktrees_without_unique_commits() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path().join("r");
        std::fs::create_dir_all(&r).unwrap();
        run(&r, &["init", "-q", "-b", "main"]);
        std::fs::write(r.join("a.txt"), "a").unwrap();
        run(&r, &["add", "."]);
        run(&r, &["commit", "-qm", "init"]);
        let clean = t.path().join("wt-clean");
        let busy = t.path().join("wt-busy");
        run(
            &r,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "clean",
                clean.to_str().unwrap(),
            ],
        );
        run(
            &r,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "busy",
                busy.to_str().unwrap(),
            ],
        );
        std::fs::write(busy.join("b.txt"), "b").unwrap();
        run(&busy, &["add", "."]);
        run(&busy, &["commit", "-qm", "only here"]);
        std::fs::write(busy.join("c.txt"), "c").unwrap();
        let all = list(&r).unwrap();
        assert_eq!(all.len(), 3);
        assert!(all[0].main && !all[0].removable());
        let c = all
            .iter()
            .find(|w| w.branch.as_deref() == Some("clean"))
            .unwrap();
        assert_eq!((c.dirty, c.unique), (Some(0), 0));
        assert!(c.removable());
        let b = all
            .iter()
            .find(|w| w.branch.as_deref() == Some("busy"))
            .unwrap();
        assert_eq!((b.dirty, b.unique), (Some(1), 1));
        assert!(!b.removable());
        let e = remove(&r, &b.path).unwrap_err().to_string();
        assert!(e.contains("沒 commit"), "{e}");
        std::fs::remove_file(busy.join("c.txt")).unwrap();
        let e = remove(&r, &b.path).unwrap_err().to_string();
        assert!(e.contains("只在這裡"), "{e}");
        assert!(remove(&r, &all[0].path).is_err());
        remove(&r, &c.path).unwrap();
        assert!(!clean.exists());
        assert_eq!(list(&r).unwrap().len(), 2);
        // 分支留著
        assert!(git(&r, &["branch", "--list", "clean"])
            .unwrap()
            .contains("clean"));
    }
}
