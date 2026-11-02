//! 現在跑的是哪一版（2026-11-02，F-114）。
//!
//! commit、commit 時間、原始碼目錄是 build 時寫死的（`build.rs`）；[`info`] 執行時再到原始碼目錄問 git：
//! - `pending`：原始碼已經有、這版還沒有的 commit（「N 個改動沒裝」）——只有原始碼目錄還在這台機器上才有；
//! - `since`：給了上次看過的那版，列出從那版到這版多了哪些 commit（「已更新：多了什麼」），
//!   以及這段期間 `docs/TESTING.md` 新增或改過的驗收列（「可以測什麼」——使用者重開 App 後最常問的）。
//!
//! 只讀、不打網路；git 不在或原始碼不在就只回這版本身。

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

/// 這版的 commit（7 碼）；不在 git 裡 build 的＝空字串。
pub const COMMIT: &str = env!("CTX_BUILD_COMMIT");
/// 這版 commit 的時間（`YYYY-MM-DD HH:MM`，本地）。
pub const COMMIT_DATE: &str = env!("CTX_BUILD_DATE");
/// build 時的原始碼目錄（repo 根目錄）。
pub const SOURCE_DIR: &str = env!("CTX_SOURCE_DIR");
/// `ctx --version` 用：`0.2.0 (a37c4b5 2026-11-01 10:28)`。
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), env!("CTX_VERSION_SUFFIX"));

/// 每份清單最多列幾個 commit（其餘只給數量）。
pub const LIST_MAX: usize = 30;
const GIT_TIMEOUT: Duration = Duration::from_secs(3);

/// `a..b` 之間的 commit（新的在前）：(總數, 前 [`LIST_MAX`] 個 `{sha, subject, date}`)；git 失敗＝None。
fn range(src: &Path, from: &str, to: &str) -> Option<(usize, Vec<Value>)> {
    let spec = format!("{from}..{to}");
    let (code, n) = crate::crossref::git(src, &["rev-list", "--count", &spec], GIT_TIMEOUT);
    let n: usize = (code == 0).then(|| n.trim().parse().ok()).flatten()?;
    if n == 0 {
        return Some((0, vec![]));
    }
    let max = format!("-{LIST_MAX}");
    let (code, out) = crate::crossref::git(
        src,
        &[
            "log",
            &max,
            "--format=%h%x09%cd%x09%s",
            "--date=format:%m-%d %H:%M",
            &spec,
        ],
        GIT_TIMEOUT,
    );
    if code != 0 {
        return None;
    }
    let list = out
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            Some(json!({"sha": it.next()?, "date": it.next()?, "subject": it.next()?}))
        })
        .collect();
    Some((n, list))
}

/// 驗收清單（相對原始碼根目錄）。
const TESTING_MD: &str = "docs/TESTING.md";

/// `from..to` 之間 `docs/TESTING.md` 新增或改過的表格列：`{id, step, expect}`（同一個 id 只留一次，最多 [`LIST_MAX`]）。
fn tests_between(src: &Path, from: &str, to: &str) -> Vec<Value> {
    let spec = format!("{from}..{to}");
    let (code, out) = crate::crossref::git(
        src,
        &["diff", "-U0", "--no-color", &spec, "--", TESTING_MD],
        GIT_TIMEOUT,
    );
    if code != 0 {
        return vec![];
    }
    let mut seen = std::collections::HashSet::new();
    let mut list = Vec::new();
    for line in out.lines() {
        let Some(row) = line.strip_prefix("+| ") else {
            continue;
        };
        let cols: Vec<&str> = row.trim_end_matches(" |").split(" | ").collect();
        // 驗收列的 id 長得像 8d-21、8c-14（表頭、分隔線不算）
        let [id, step, expect, ..] = cols[..] else {
            continue;
        };
        let id = id.trim();
        let looks_like_id =
            id.contains('-') && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if looks_like_id && seen.insert(id.to_string()) {
            list.push(json!({"id": id, "step": step.trim(), "expect": expect.trim()}));
        }
        if list.len() >= LIST_MAX {
            break;
        }
    }
    list
}

/// 這版的資訊；`since`＝上次看過的那版 commit（跟這版一樣或認不得就不列 `since`）。
/// 回 `{commit, date, source, pending: {count, commits}|null, since: {from, count, commits, tests}|null}`。
pub fn info(since: Option<&str>) -> Value {
    let src = Path::new(SOURCE_DIR);
    let known = !COMMIT.is_empty() && !SOURCE_DIR.is_empty() && src.join(".git").exists();
    let pending = known
        .then(|| range(src, COMMIT, "HEAD"))
        .flatten()
        .map(|(n, c)| json!({"count": n, "commits": c}));
    let since = since
        .map(str::trim)
        .filter(|s| known && !s.is_empty() && *s != COMMIT)
        .filter(|s| s.chars().all(|c| c.is_ascii_hexdigit()))
        .and_then(|s| {
            range(src, s, COMMIT).map(|(n, c)| {
                json!({"from": s, "count": n, "commits": c, "tests": tests_between(src, s, COMMIT)})
            })
        });
    json!({
        "commit": (!COMMIT.is_empty()).then_some(COMMIT),
        "date": (!COMMIT_DATE.is_empty()).then_some(COMMIT_DATE),
        "source": known.then_some(SOURCE_DIR),
        "pending": pending,
        "since": since,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_build_knows_itself() {
        // 測試就是在 repo 裡 build 的：commit、原始碼都在；HEAD 就是這版或更新
        let v = info(None);
        assert_eq!(v["commit"], COMMIT);
        assert!(VERSION.starts_with(env!("CARGO_PKG_VERSION")));
        if v["source"].is_string() {
            assert!(v["pending"]["count"].is_u64());
        }
        assert!(v["since"].is_null());
        // 跟這版一樣、不是 commit 的字串：不列 since
        assert!(info(Some(COMMIT))["since"].is_null());
        assert!(info(Some("HEAD; rm -rf /"))["since"].is_null());
    }

    #[test]
    fn range_lists_commits_between() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        crate::testkit::git(r, &["init", "-q"], &[]);
        for i in 0..3 {
            std::fs::write(r.join("f"), i.to_string()).unwrap();
            crate::testkit::git(r, &["add", "f"], &[]);
            crate::testkit::git(r, &["commit", "-qm", &format!("第 {i} 個")], &[]);
        }
        let (n, list) = range(r, "HEAD~2", "HEAD").unwrap();
        assert_eq!(n, 2);
        assert_eq!(list[0]["subject"], "第 2 個");
        assert_eq!(list[1]["subject"], "第 1 個");
        assert_eq!(range(r, "HEAD", "HEAD").unwrap().0, 0);
        assert!(range(r, "nope", "HEAD").is_none());

        // 驗收列：新增與改過的都算，表頭與分隔線不算，同 id 只一次
        std::fs::create_dir_all(r.join("docs")).unwrap();
        let md = |rows: &str| {
            std::fs::write(
                r.join("docs/TESTING.md"),
                format!("| # | 步驟 | 預期 | macOS | Windows |\n|---|---|---|---|---|\n{rows}"),
            )
            .unwrap();
            crate::testkit::git(r, &["add", "docs"], &[]);
            crate::testkit::git(r, &["commit", "-qm", "testing"], &[]);
        };
        md("| 8d-1 | 舊的 | 舊 | [ ] | [ ] |\n");
        md("| 8d-1 | 舊的（改過） | 舊 | [ ] | [ ] |\n| 8d-2 | 按「分割」 | 左寫右看 | [ ] | 未測 |\n");
        let t = tests_between(r, "HEAD~1", "HEAD");
        assert_eq!(t.len(), 2);
        assert_eq!(t[0]["id"], "8d-1");
        assert_eq!(
            t[1],
            json!({"id": "8d-2", "step": "按「分割」", "expect": "左寫右看"})
        );
        assert!(tests_between(r, "HEAD", "HEAD").is_empty());
    }
}
