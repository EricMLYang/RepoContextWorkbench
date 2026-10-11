//! Repo 檔案清單（2026-10-11；Rust 才有，Python 沒有對應模組）。
//!
//! App 的 repo 詳情像 IDE 側邊欄一樣逐層展開，每一列可以複製絕對或相對路徑：
//! - 一次只列一層（展開資料夾才讀下一層），大 repo 也不會一次讀完。
//! - 不列 `.git`；git repo 裡被 .gitignore 忽略的不列（交給 `git check-ignore`，規則跟 git 一致），
//!   不是 git 的資料夾全列。
//! - 只讀，不改任何檔；`dir` 必須在 repo 裡（擋 `..`、絕對路徑、符號連結跳出 repo）。

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use crate::{Error, Result};

/// 一層最多列幾項；超過的只回數量（`more`）。
pub const MAX_ENTRIES: usize = 2000;

/// 一列：檔案或資料夾。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// 相對 repo 根目錄（用系統的路徑分隔符）。
    pub rel: PathBuf,
    pub abs: PathBuf,
    /// 符號連結一律當檔案（不跟進去，免得跳出 repo）。
    pub dir: bool,
}

/// 一層的內容：資料夾在前、再依名稱（不分大小寫）排序。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// 超過 [`MAX_ENTRIES`] 沒列出的項數。
    pub more: usize,
}

/// `dir`（相對路徑，空字串＝根目錄）只能往下走。
fn checked_rel(dir: &str) -> Result<PathBuf> {
    let rel = PathBuf::from(dir.trim());
    for c in rel.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(Error::invalid(format!("資料夾必須在 repo 裡：{dir}"))),
        }
    }
    Ok(rel)
}

/// git repo 裡被忽略的那些（`rels` 是相對 root 的路徑）。不是 git、或 git 不能用＝都不忽略。
fn ignored(root: &Path, rels: &[PathBuf]) -> Vec<bool> {
    let mut out = vec![false; rels.len()];
    if rels.is_empty() || !root.join(".git").exists() {
        return out;
    }
    let Ok(mut child) = Command::new("git")
        .args(["check-ignore", "--stdin", "-z"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return out;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let mut buf = Vec::new();
        for r in rels {
            buf.extend_from_slice(r.to_string_lossy().as_bytes());
            buf.push(0);
        }
        let _ = stdin.write_all(&buf);
    }
    // 有忽略的 exit 0、全部沒忽略 exit 1；兩種都照 stdout 判斷
    let Ok(res) = child.wait_with_output() else {
        return out;
    };
    let hits: Vec<&[u8]> = res
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .collect();
    for (i, r) in rels.iter().enumerate() {
        let key = r.to_string_lossy();
        out[i] = hits.iter().any(|h| *h == key.as_bytes());
    }
    out
}

/// 列出 `root/dir` 這一層。
pub fn list_dir(root: &Path, dir: &str) -> Result<Listing> {
    let rel_dir = checked_rel(dir)?;
    let root_real = root
        .canonicalize()
        .map_err(|_| Error::invalid(format!("repo 路徑不存在：{}", root.display())))?;
    let target = root_real.join(&rel_dir);
    let target_real = target
        .canonicalize()
        .map_err(|_| Error::invalid(format!("資料夾不存在：{dir}")))?;
    if !target_real.starts_with(&root_real) {
        return Err(Error::invalid(format!("資料夾必須在 repo 裡：{dir}")));
    }
    if !target_real.is_dir() {
        return Err(Error::invalid(format!("不是資料夾：{dir}")));
    }

    let mut entries: Vec<Entry> = std::fs::read_dir(&target_real)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                return None;
            }
            let dir = e.file_type().ok().is_some_and(|t| t.is_dir());
            Some(Entry {
                rel: rel_dir.join(&name),
                abs: root.join(&rel_dir).join(&name),
                name,
                dir,
            })
        })
        .collect();
    let rels: Vec<PathBuf> = entries.iter().map(|e| e.rel.clone()).collect();
    let skip = ignored(&root_real, &rels);
    let mut i = 0;
    entries.retain(|_| {
        i += 1;
        !skip[i - 1]
    });
    entries.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    let more = entries.len().saturating_sub(MAX_ENTRIES);
    entries.truncate(MAX_ENTRIES);
    Ok(Listing { entries, more })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(l: &Listing) -> Vec<(String, bool)> {
        l.entries.iter().map(|e| (e.name.clone(), e.dir)).collect()
    }

    #[test]
    fn plain_folder_lists_dirs_first_case_insensitive() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        std::fs::create_dir(r.join("src")).unwrap();
        std::fs::write(r.join("b.md"), "").unwrap();
        std::fs::write(r.join("A.md"), "").unwrap();
        std::fs::write(r.join("src").join("main.rs"), "").unwrap();
        let l = list_dir(r, "").unwrap();
        assert_eq!(
            names(&l),
            vec![
                ("src".into(), true),
                ("A.md".into(), false),
                ("b.md".into(), false)
            ]
        );
        let sub = list_dir(r, "src").unwrap();
        assert_eq!(sub.entries[0].rel, PathBuf::from("src").join("main.rs"));
        assert_eq!(sub.entries[0].abs, r.join("src").join("main.rs"));
    }

    #[test]
    fn git_repo_hides_dot_git_and_ignored() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        let ok = Command::new("git")
            .args(["init", "-q"])
            .current_dir(r)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return; // 沒有 git 的環境跳過
        }
        std::fs::write(r.join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::create_dir(r.join("target")).unwrap();
        std::fs::write(r.join("run.log"), "").unwrap();
        std::fs::write(r.join("README.md"), "").unwrap();
        let l = list_dir(r, "").unwrap();
        assert_eq!(
            names(&l),
            vec![(".gitignore".into(), false), ("README.md".into(), false)]
        );
    }

    #[test]
    fn rejects_paths_outside_repo() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path().join("repo");
        std::fs::create_dir_all(r.join("a")).unwrap();
        for bad in ["..", "a/../..", "/etc"] {
            assert!(list_dir(&r, bad).is_err(), "{bad} 應被拒");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(t.path(), r.join("out")).unwrap();
            assert!(list_dir(&r, "out").is_err(), "符號連結跳出 repo 應被拒");
            let l = list_dir(&r, "").unwrap();
            let out = l.entries.iter().find(|e| e.name == "out").unwrap();
            assert!(!out.dir, "符號連結當檔案");
        }
        assert!(list_dir(&r, "nope").is_err());
    }
}
