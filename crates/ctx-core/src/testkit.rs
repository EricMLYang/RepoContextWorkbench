//! 測試共用 fixture（對應 `tests/conftest.py`）。只在 feature `testkit` 下編譯。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 假身分＋不碰使用者 git 設定的 git 指令。
pub fn git(cwd: &Path, args: &[&str], extra_env: &[(&str, &str)]) {
    let mut c = Command::new("git");
    c.arg("-C").arg(cwd).args(args);
    c.env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t");
    for (k, v) in extra_env {
        c.env(k, v);
    }
    let out = c.output().expect("git 執行失敗");
    assert!(
        out.status.success(),
        "git {:?} 失敗：{}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// 對應 conftest.make_git_repo：README＋筆記，最後 commit 在 `days_old` 天前，`dirty_files` 個未提交檔。
pub fn make_git_repo(
    base: &Path,
    name: &str,
    days_old: i64,
    dirty_files: usize,
    notes: bool,
) -> PathBuf {
    let p = base.join(name);
    std::fs::create_dir_all(&p).unwrap();
    git(&p, &["init", "-q"], &[]);
    std::fs::write(
        p.join("README.md"),
        format!("# {name}\n這個 repo 談 widget 快取設計。\n"),
    )
    .unwrap();
    if notes {
        std::fs::create_dir_all(p.join("notes")).unwrap();
        std::fs::write(
            p.join("notes").join("plan.md"),
            "## 計劃\n已知結論：快取要用 LRU。\n",
        )
        .unwrap();
    }
    let when = (chrono::Local::now() - chrono::Duration::days(days_old))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    git(&p, &["add", "-A"], &[]);
    git(
        &p,
        &["commit", "-q", "-m", "init"],
        &[("GIT_AUTHOR_DATE", &when), ("GIT_COMMITTER_DATE", &when)],
    );
    for i in 0..dirty_files {
        std::fs::write(p.join(format!("wip{i}.md")), "未提交草稿\n").unwrap();
    }
    p
}

/// 對應 conftest `spine` fixture：temp 目錄裡 init 好的 spine repo。回傳 (TempDir 守衛, spine 路徑)。
/// TempDir 的路徑＝conftest 的 `tmp_path`（其他假 repo 建在這底下）。
pub fn spine() -> (tempfile::TempDir, PathBuf) {
    std::env::set_var("REPOCTX_NO_BACKGROUND", "1");
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path().join("spine-repo");
    crate::init::init_spine(&d).unwrap();
    (tmp, d)
}

/// 對應 conftest `spine_with_repos`：spine ＋ 兩個 mine repo（repo-a 1 天前乾淨、repo-b 6 天前
/// 且 2 個 dirty 檔、dirty 檔 mtime 設成 5 天前）＋ 組 g1。
pub fn spine_with_repos() -> (tempfile::TempDir, PathBuf) {
    let (tmp, d) = spine();
    let ra = make_git_repo(tmp.path(), "repo-a", 1, 0, true);
    let rb = make_git_repo(tmp.path(), "repo-b", 6, 2, true);
    // 讓 dirty 檔看起來已放了 5 天
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(5 * 86400);
    for i in 0..2 {
        let f = std::fs::File::options()
            .write(true)
            .open(rb.join(format!("wip{i}.md")))
            .unwrap();
        f.set_times(
            std::fs::FileTimes::new()
                .set_accessed(old)
                .set_modified(old),
        )
        .unwrap();
    }
    use crate::registry::{add_group, add_repo, AddRepo};
    add_repo(&d, "repo-a", &ra, AddRepo::default()).unwrap();
    add_repo(&d, "repo-b", &rb, AddRepo::default()).unwrap();
    add_group(&d, "g1", &["repo-a", "repo-b"]).unwrap();
    (tmp, d)
}

/// 測試用時間：今天 h:m（對應 test_spine 的 `_t`）。
pub fn today_at(h: u32, m: u32) -> chrono::NaiveDateTime {
    chrono::Local::now()
        .date_naive()
        .and_hms_opt(h, m, 0)
        .unwrap()
}
