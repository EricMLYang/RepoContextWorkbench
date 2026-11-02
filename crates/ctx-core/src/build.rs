//! 現在跑的是哪一版（2026-11-02，F-114）。
//!
//! commit、commit 時間、原始碼目錄是 build 時寫死的（`build.rs`）；[`info`] 執行時再到原始碼目錄問 git：
//! - `pending`：原始碼已經有、這版還沒有的 commit（「N 個改動沒裝」）——只有原始碼目錄還在這台機器上才有；
//! - `since`：給了上次看過的那版，列出從那版到這版多了哪些 commit（「已更新：多了什麼」），
//!   以及這段期間 `docs/TESTING.md` 新增或改過的驗收列（「可以測什麼」——使用者重開 App 後最常問的）。
//!
//! 只讀、不打網路；git 不在或原始碼不在就只回這版本身。
//!
//! 背景更新（2026-11-03，F-118；只有 macOS）：[`start_install`] 在背景跑原始碼裡的 `scripts/install-app.sh --wait --reopen`
//! ——App 開著也先 build，build 好了等使用者結束 App（⌘Q）才換、換好自動重開。腳本把階段寫進 [`install_status_path`]，
//! [`info`] 帶出來（`install`），App 狀態列顯示進度。App 不會自己結束（會結束正在跑的會話，要使用者自己按）。

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

/// `from..to` 之間 `docs/TESTING.md` 新增或改過、macOS 還沒打勾的表格列：`{id, step, expect, line}`（line＝新版裡第幾行）（同一個 id 只留一次，最多 [`LIST_MAX`]）。
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
    // 新檔裡的行號（-U0：只有 @@ 標頭與 +／- 行；+ 行一行一號）
    let mut at = 0usize;
    for line in out.lines() {
        if let Some(h) = line.strip_prefix("@@ ") {
            // @@ -a[,b] +c[,d] @@
            at = h
                .split(' ')
                .find_map(|t| t.strip_prefix('+'))
                .and_then(|t| t.split(',').next())
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            continue;
        }
        if line.starts_with("+++") || !line.starts_with('+') {
            continue;
        }
        let here = at;
        at += 1;
        let Some(row) = line.strip_prefix("+| ") else {
            continue;
        };
        let cols: Vec<&str> = row.trim_end_matches(" |").split(" | ").collect();
        // 驗收列的 id 長得像 8d-21、8c-14（表頭、分隔線不算）
        let [id, step, expect, rest @ ..] = &cols[..] else {
            continue;
        };
        // macOS 那欄已經打勾（[x]）＝驗過了；打勾本身也會讓那一列出現在差異裡，不能當成新的要驗
        if rest.first().is_some_and(|m| m.contains("[x]")) {
            continue;
        }
        let id = id.trim();
        let looks_like_id =
            id.contains('-') && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if looks_like_id && seen.insert(id.to_string()) {
            list.push(
                json!({"id": id, "step": step.trim(), "expect": expect.trim(), "line": here}),
            );
        }
        if list.len() >= LIST_MAX {
            break;
        }
    }
    list
}

/// 背景更新的狀態檔（腳本寫、這裡讀）。
pub fn install_status_path() -> std::path::PathBuf {
    crate::fsutil::expanduser("~/.cache/repo-context/install-status.json")
}

/// 背景更新的輸出紀錄。
pub fn install_log_path() -> std::path::PathBuf {
    crate::fsutil::expanduser("~/.cache/repo-context/install.log")
}

/// 這個 pid 還活著嗎（Unix：kill 0；其他平台當作活著）。
fn alive(pid: i64) -> bool {
    #[cfg(unix)]
    {
        // SAFETY：kill(pid, 0) 只檢查存在與權限，不送訊號
        pid > 0 && unsafe { libc::kill(pid as libc::pid_t, 0) } == 0
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// 進行中的階段（腳本還在跑）。
const IN_PROGRESS: &[&str] = &["building", "waiting", "installing"];

/// 背景更新的狀態：`{phase, at, pid, commit, detail, log, current, age_min}`；沒有就 None。
/// 狀態檔說進行中但那個程序已經不在了（被砍、機器重開）＝`failed`、detail「中途停了」。
/// `current`＝那次更新的 commit 就是現在跑的這版（App 不必再提）。
pub fn install_status() -> Option<Value> {
    read_status(&install_status_path(), chrono::Local::now())
}

fn read_status(path: &Path, now: chrono::DateTime<chrono::Local>) -> Option<Value> {
    let s = std::fs::read_to_string(path).ok()?;
    let mut v: Value = serde_json::from_str(s.trim()).ok()?;
    let phase = v.get("phase")?.as_str()?.to_string();
    if IN_PROGRESS.contains(&phase.as_str()) && !alive(v["pid"].as_i64().unwrap_or(0)) {
        v["phase"] = json!("failed");
        v["detail"] = json!("中途停了（程序不在了）");
    }
    let commit = v["commit"].as_str().unwrap_or("");
    v["current"] = json!(!COMMIT.is_empty() && commit == COMMIT);
    let at = v["at"].as_str().and_then(|a| {
        chrono::NaiveDateTime::parse_from_str(a, "%Y-%m-%d %H:%M")
            .ok()
            .and_then(|t| chrono::TimeZone::from_local_datetime(&chrono::Local, &t).single())
    });
    if let Some(at) = at {
        v["age_min"] = json!((now - at).num_minutes().max(0));
    }
    v["log"] = json!(install_log_path().to_string_lossy());
    Some(v)
}

/// 開始背景更新（F-118）：已經在跑就拒絕；只有 macOS、原始碼目錄還在才行。回剛寫的狀態。
pub fn start_install() -> crate::Result<Value> {
    use crate::Error;
    if !cfg!(target_os = "macos") {
        return Err(Error::invalid(
            "背景更新只有 macOS 有（Windows 請照 README 手動裝）",
        ));
    }
    if let Some(st) = install_status() {
        if IN_PROGRESS.contains(&st["phase"].as_str().unwrap_or("")) {
            return Err(Error::invalid(format!(
                "已經在更新了（{}，{} 開始）",
                st["phase"].as_str().unwrap_or(""),
                st["at"].as_str().unwrap_or("")
            )));
        }
    }
    let src = Path::new(SOURCE_DIR);
    let script = src.join("scripts").join("install-app.sh");
    if SOURCE_DIR.is_empty() || !script.is_file() {
        return Err(Error::invalid(format!(
            "找不到原始碼裡的安裝腳本：{}",
            script.display()
        )));
    }
    let status = install_status_path();
    let log = install_log_path();
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let out = std::fs::File::create(&log)?;
    let mut cmd = std::process::Command::new("/bin/bash");
    cmd.arg(&script)
        .args(["--wait", "--reopen"])
        .current_dir(src)
        .env("CTX_INSTALL_STATUS", &status)
        .stdin(std::process::Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(out);
    crate::exe::detach(&mut cmd);
    // 起腳本之前先寫一份 building（pid 先用 App 自己的）：App 馬上問也看得到、不會看到上一次的 done；
    // 腳本一開始就會用自己的 pid 蓋掉——先寫才不會反過來把腳本很早寫的 failed（例：有沒 commit 的改動）蓋掉
    let at = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let write = |v: &Value| crate::fsutil::write_atomic(&status, format!("{v}\n").as_bytes());
    let v = json!({"phase": "building", "at": at, "pid": std::process::id(), "commit": "", "detail": ""});
    write(&v)?;
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let failed = json!({"phase": "failed", "at": at, "pid": 0, "commit": "", "detail": format!("起不了安裝腳本：{e}")});
            write(&failed)?;
            return Err(e.into());
        }
    };
    // 腳本還沒寫自己的（狀態檔還是 App 的 pid）：換成腳本的 pid——腳本萬一一開始就死掉，才看得出「中途停了」
    let still_ours = std::fs::read_to_string(&status)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(t.trim()).ok())
        .is_some_and(|cur| cur["pid"].as_u64() == Some(u64::from(std::process::id())));
    if still_ours {
        let mut v2 = v.clone();
        v2["pid"] = json!(child.id());
        write(&v2)?;
    }
    Ok(install_status().unwrap_or(v))
}

/// install-app.sh 裝完寫的紀錄：`{commit, previous, at}`。
pub fn installed_record_path() -> std::path::PathBuf {
    crate::fsutil::expanduser("~/.cache/repo-context/installed.json")
}

/// 紀錄裡裝的就是這版時，換掉的那版 commit；不是這版、沒有紀錄、沒記到＝None。
fn previous_install(path: &Path) -> Option<String> {
    let v: Value = serde_json::from_str(std::fs::read_to_string(path).ok()?.trim()).ok()?;
    let commit = v["commit"].as_str()?;
    let prev = v["previous"].as_str()?.trim();
    (!COMMIT.is_empty() && commit == COMMIT && !prev.is_empty() && prev != COMMIT)
        .then(|| prev.to_string())
}

/// 這版的資訊；`since`＝上次看過的那版 commit（跟這版一樣或認不得就不列 `since`）。
/// 回 `{commit, date, source, pending: {count, commits}|null, since: {from, count, commits, tests}|null}`。
pub fn info(since: Option<&str>) -> Value {
    info_with(since, &installed_record_path())
}

fn info_with(since: Option<&str>, record: &Path) -> Value {
    // 沒有「上次看過」（這台第一次用有 F-114 的版本）：用 install-app.sh 記的「換掉的是哪版」，
    // 第一次打開新版也看得到「已更新：多了什麼、可以驗收什麼」
    let fallback = since.is_none().then(|| previous_install(record)).flatten();
    let since = since.or(fallback.as_deref());
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
        "install": install_status(),
        // 重裝之後 agent 讀的 skill 常常還是舊版（setup 沒重跑）：current／stale／missing
        "skill": crate::agentsetup::skill_status(&crate::agentsetup::claude_home()),
        // 「可以驗收的」點了開哪份（相對原始碼根目錄）
        "testing": TESTING_MD,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_build_knows_itself() {
        // 測試就是在 repo 裡 build 的：commit、原始碼都在；HEAD 就是這版或更新
        let none = std::path::Path::new("/nonexistent/installed.json");
        let v = info_with(None, none);
        assert_eq!(v["commit"], COMMIT);
        assert!(VERSION.starts_with(env!("CARGO_PKG_VERSION")));
        if v["source"].is_string() {
            assert!(v["pending"]["count"].is_u64());
        }
        assert!(v["since"].is_null());
        // 跟這版一樣、不是 commit 的字串：不列 since
        assert!(info_with(Some(COMMIT), none)["since"].is_null());
        assert!(info_with(Some("HEAD; rm -rf /"), none)["since"].is_null());
    }

    #[test]
    fn previous_install_only_for_this_build() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("installed.json");
        assert_eq!(previous_install(&p), None);
        std::fs::write(
            &p,
            format!(r#"{{"commit":"{COMMIT}","previous":"9a41433","at":"x"}}"#),
        )
        .unwrap();
        assert_eq!(
            previous_install(&p),
            (!COMMIT.is_empty()).then(|| "9a41433".to_string())
        );
        // 裝的不是這版（之後又 build 了別版在跑）、沒記到舊版：不用
        std::fs::write(&p, r#"{"commit":"zzzzzzz","previous":"9a41433","at":"x"}"#).unwrap();
        assert_eq!(previous_install(&p), None);
        std::fs::write(
            &p,
            format!(r#"{{"commit":"{COMMIT}","previous":"","at":"x"}}"#),
        )
        .unwrap();
        assert_eq!(previous_install(&p), None);
    }

    #[test]
    fn install_status_marks_dead_runs_failed() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("st.json");
        let now = chrono::Local::now();
        let at = now.format("%Y-%m-%d %H:%M").to_string();
        assert!(read_status(&p, now).is_none());
        // 自己這個程序還活著：照寫
        let me = std::process::id();
        std::fs::write(
            &p,
            format!(r#"{{"phase":"waiting","at":"{at}","pid":{me},"commit":"x"}}"#),
        )
        .unwrap();
        let v = read_status(&p, now).unwrap();
        assert_eq!(v["phase"], "waiting");
        assert_eq!(v["current"], false);
        assert_eq!(v["age_min"], 0);
        // 不在的 pid：進行中改成 failed
        std::fs::write(
            &p,
            format!(r#"{{"phase":"building","at":"{at}","pid":999999,"commit":"x"}}"#),
        )
        .unwrap();
        assert_eq!(read_status(&p, now).unwrap()["phase"], "failed");
        // done 不看 pid；commit 就是這版＝current
        std::fs::write(
            &p,
            format!(r#"{{"phase":"done","at":"{at}","pid":1,"commit":"{COMMIT}"}}"#),
        )
        .unwrap();
        let v = read_status(&p, now).unwrap();
        assert_eq!(v["phase"], "done");
        assert_eq!(v["current"], !COMMIT.is_empty());
        std::fs::write(&p, "壞掉").unwrap();
        assert!(read_status(&p, now).is_none());
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
        md("| 8d-1 | 舊的（改過） | 舊 | [ ] | [ ] |\n| 8d-2 | 按「分割」 | 左寫右看 | [ ] | 未測 |\n| 8d-0 | 驗過的 | 好 | [x] | [ ] |\n");
        let t = tests_between(r, "HEAD~1", "HEAD");
        assert_eq!(t.len(), 2);
        assert_eq!(t[0]["id"], "8d-1");
        // 第 1、2 行是表頭與分隔線
        assert_eq!(t[0]["line"], 3);
        assert_eq!(
            t[1],
            json!({"id": "8d-2", "step": "按「分割」", "expect": "左寫右看", "line": 4})
        );
        assert!(tests_between(r, "HEAD", "HEAD").is_empty());
    }
}
