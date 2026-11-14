//! repo 最近的 commit 與單一 commit 的差異（2026-10-22，ideation F-109；Rust 才有，參考 BAT 的 Git 檢視）。
//!
//! 「變更」頁只看得到還沒 commit 的；agent 做完常常已經 commit 了，人要回頭看「剛剛那幾個 commit 改了什麼」。
//! 只讀：`git log` 與 `git show`，都有逾時；差異跟檔案差異同一個上限（[`crate::filetree::DIFF_MAX`]）。

use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::{Error, Result};

const GIT_TIMEOUT: Duration = Duration::from_secs(5);
/// 一次最多列幾個 commit。
pub const LOG_MAX: usize = 100;

/// 一個 commit。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Commit {
    pub sha: String,
    pub short: String,
    pub subject: String,
    pub author: String,
    /// commit 時間（ISO 8601，帶時區）。
    pub at: String,
}

/// 最近 `limit` 個 commit（目前分支，新的在前）。不是 git repo、還沒有 commit＝空的。
pub fn log(root: &Path, limit: usize) -> Vec<Commit> {
    let n = format!("-n{}", limit.clamp(1, LOG_MAX));
    let (code, out) = crate::crossref::git(
        root,
        &[
            "log",
            &n,
            "--no-color",
            "--format=%H%x1f%h%x1f%s%x1f%an%x1f%cI%x1e",
        ],
        GIT_TIMEOUT,
    );
    if code != 0 {
        return Vec::new();
    }
    parse_log(&out)
}

/// `since`（`YYYY-MM-DD HH:MM`，本機時間）之後的 commit：(總數, 最新的 `limit` 個)。
/// 不是 git repo、還沒有 commit＝(0, 空)。只讀（`git log`／`rev-list` 不拿 index 鎖，仍帶 `--no-optional-locks`）。
/// Rust 才有（2026-11-14，接力 `relay report`）。
pub fn since(root: &Path, since: &str, limit: usize) -> (usize, Vec<Commit>) {
    let after = format!("--since={since}");
    let (code, count) = crate::crossref::git(
        root,
        &["--no-optional-locks", "rev-list", "--count", &after, "HEAD"],
        GIT_TIMEOUT,
    );
    let total = if code == 0 {
        count.trim().parse().unwrap_or(0)
    } else {
        0
    };
    if total == 0 {
        return (0, Vec::new());
    }
    let n = format!("-n{}", limit.clamp(1, LOG_MAX));
    let (code, out) = crate::crossref::git(
        root,
        &[
            "--no-optional-locks",
            "log",
            &n,
            &after,
            "--no-color",
            "--format=%H%x1f%h%x1f%s%x1f%an%x1f%cI%x1e",
        ],
        GIT_TIMEOUT,
    );
    let commits = if code == 0 {
        parse_log(&out)
    } else {
        Vec::new()
    };
    (total, commits)
}

fn parse_log(out: &str) -> Vec<Commit> {
    out.split('\u{1e}')
        .filter_map(|rec| {
            let f: Vec<&str> = rec.trim_matches('\n').split('\u{1f}').collect();
            match f.as_slice() {
                [sha, short, subject, author, at] => Some(Commit {
                    sha: sha.to_string(),
                    short: short.to_string(),
                    subject: subject.to_string(),
                    author: author.to_string(),
                    at: at.to_string(),
                }),
                _ => None,
            }
        })
        .collect()
}

/// 一個 commit 的說明＋統計＋差異（`git show`）。sha 只收 4～64 個十六進位字（不拼別的參數）。
/// 回傳 (文字, 有沒有截掉)。
pub fn show(root: &Path, sha: &str) -> Result<(String, bool)> {
    if !(4..=64).contains(&sha.len()) || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::invalid(format!("commit 不對：{sha}")));
    }
    let (code, mut text) = crate::crossref::git(
        root,
        &[
            "show",
            "--no-color",
            "--no-ext-diff",
            "--stat",
            "--patch",
            "--format=commit %H%nAuthor: %an <%ae>%nDate:   %cI%n%n%B",
            sha,
        ],
        GIT_TIMEOUT,
    );
    if code != 0 {
        return Err(Error::invalid(format!("找不到這個 commit：{sha}")));
    }
    let max = crate::filetree::DIFF_MAX;
    let truncated = text.len() > max;
    if truncated {
        let mut cut = max;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    Ok((text, truncated))
}

/// 沒 commit 的檔（含未追蹤；改名的新舊路徑都列），repo 內相對路徑、`/` 分隔。
/// 放行規則（`policy` 的 `paths`）用：引擎自己查，不信 agent 說的。不是 git repo 或 git 失敗＝錯誤。
/// 只讀（`--no-optional-locks`）。Rust 才有（2026-11-14，F-124）。
pub fn dirty_files(root: &Path) -> Result<Vec<String>> {
    let (code, out) = crate::crossref::git(
        root,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "-z",
            "--untracked-files=all",
        ],
        GIT_TIMEOUT,
    );
    if code != 0 {
        return Err(Error::External(format!(
            "{} 查不到 git 狀態（不是 git repo 或 git 失敗）",
            root.display()
        )));
    }
    let mut files = Vec::new();
    let mut it = out.split('\0').filter(|x| !x.is_empty());
    while let Some(entry) = it.next() {
        let (xy, path) = (entry.get(..2).unwrap_or(""), entry.get(3..).unwrap_or(""));
        files.push(path.to_string());
        // 改名／複製：下一段是原本的路徑
        if xy.starts_with(['R', 'C']) {
            if let Some(orig) = it.next() {
                files.push(orig.to_string());
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn run(dir: &Path, args: &[&str]) {
        let o = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "小明")
            .env("GIT_AUTHOR_EMAIL", "m@t")
            .env("GIT_COMMITTER_NAME", "小明")
            .env("GIT_COMMITTER_EMAIL", "m@t")
            .output()
            .unwrap();
        assert!(o.status.success());
    }

    #[test]
    fn log_lists_newest_first_and_show_gives_the_patch() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        assert!(log(r, 10).is_empty(), "不是 git repo");
        run(r, &["init", "-q", "-b", "main"]);
        assert!(log(r, 10).is_empty(), "還沒有 commit");
        std::fs::write(r.join("a.txt"), "a\n").unwrap();
        run(r, &["add", "."]);
        run(r, &["commit", "-qm", "第一個"]);
        std::fs::write(r.join("a.txt"), "a\nb\n").unwrap();
        run(r, &["commit", "-qam", "加一行 | 有分隔符號"]);
        let l = log(r, 10);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].subject, "加一行 | 有分隔符號");
        assert_eq!(l[0].author, "小明");
        assert_eq!(l[1].subject, "第一個");
        assert_eq!(log(r, 1).len(), 1);
        let (text, cut) = show(r, &l[0].short).unwrap();
        assert!(!cut);
        assert!(text.contains("加一行"), "{text}");
        assert!(text.contains("+b"), "{text}");
        assert!(text.contains("a.txt | 1 +"), "{text}");
        assert!(show(r, "--output=/tmp/x").is_err());
        assert!(show(r, "ffffffff").is_err());
    }
}
