//! 這波葉節點模組測試共用的 fixture。
//!
//! 對應 conftest `spine_with_repos`，但**不經 registry**：registry／spine 在另一波搬，
//! 這裡直接建兩個假 git repo（repo-a 乾淨、1 天前 commit；repo-b dirty 2 檔放 5 天、6 天前 commit），
//! 手組 `resolve_group(spine, "g1")` 會回的 entries（順序 repo-a、repo-b）。
//! spine 目錄只是 temp 裡的空資料夾——agentmark／knowledge 快取只需要它當根。
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use ctx_core::collect::RepoSpec;
use ctx_core::pack::RepoRef;
use ctx_core::testkit::make_git_repo;

pub struct World {
    pub _tmp: tempfile::TempDir,
    pub tmp: PathBuf,
    pub spine: PathBuf,
    pub repo_a: PathBuf,
    pub repo_b: PathBuf,
}

impl World {
    /// 組 g1 的選料 entries（pack／knowledge 用）。
    pub fn refs(&self) -> Vec<RepoRef> {
        vec![
            RepoRef::new("repo-a", &self.repo_a),
            RepoRef::new("repo-b", &self.repo_b),
        ]
    }
    /// 組 g1 的採集 entries（collect 用）。
    pub fn specs(&self) -> Vec<RepoSpec> {
        vec![
            RepoSpec::new("repo-a", &self.repo_a),
            RepoSpec::new("repo-b", &self.repo_b),
        ]
    }
}

/// 把檔案 mtime 設成 `days` 天前。
pub fn age_file(p: &Path, days: u64) {
    let when = SystemTime::now() - Duration::from_secs(days * 86400);
    let f = std::fs::File::options().write(true).open(p).unwrap();
    f.set_modified(when).unwrap();
}

/// 對應 conftest `spine_with_repos`（不含 registry 登記）。
pub fn spine_with_repos() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().to_path_buf();
    let spine = base.join("spine-repo");
    std::fs::create_dir_all(&spine).unwrap();
    let repo_a = make_git_repo(&base, "repo-a", 1, 0, true);
    let repo_b = make_git_repo(&base, "repo-b", 6, 2, true);
    for i in 0..2 {
        age_file(&repo_b.join(format!("wip{i}.md")), 5);
    }
    World {
        _tmp: tmp,
        tmp: base,
        spine,
        repo_a,
        repo_b,
    }
}
