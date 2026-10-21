//! repo 在 GitHub 上開著的 PR 與 issue（2026-10-22，ideation F-112；Rust 才有，參考 BAT 的 GitHub 面板）。
//!
//! agent 開了 PR、別人回了 issue，人要切到瀏覽器才看得到。這裡用使用者自己的 `gh`（已登入的那個）列出來，
//! 只讀：`gh pr list`／`gh issue list --json`，有逾時。沒裝 `gh`、沒登入、不是 GitHub repo 都照 `gh` 的話說明。

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::{Error, Result};

/// `gh` 每次最多等多久（要連網）。
const GH_TIMEOUT: Duration = Duration::from_secs(12);
/// 各列幾筆。
const LIMIT: &str = "20";

/// 跑一個指令，回傳 (exit code, stdout, stderr)；逾時就殺掉回錯。
fn run(cwd: &Path, prog: &str, args: &[&str], timeout: Duration) -> Result<(i32, String, String)> {
    let mut child = Command::new(prog)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::invalid("沒有安裝 GitHub CLI（gh）：brew install gh，再 gh auth login")
            } else {
                Error::External(format!("{prog} 跑不起來：{e}"))
            }
        })?;
    let pipe = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut buf);
            }
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let out = pipe(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let err = pipe(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::External(format!(
                "{prog} 超過 {} 秒沒回應（網路？）",
                timeout.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    Ok((
        status.code().unwrap_or(1),
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
    ))
}

fn gh_json(cwd: &Path, args: &[&str]) -> Result<Value> {
    let (code, out, err) = run(cwd, "gh", args, GH_TIMEOUT)?;
    if code != 0 {
        let msg = err.trim();
        return Err(Error::invalid(if msg.is_empty() {
            format!("gh {} 失敗", args.join(" "))
        } else {
            crate::pystr::clip(msg, 300)
        }));
    }
    serde_json::from_str(&out).map_err(|e| Error::External(format!("gh 的輸出看不懂：{e}")))
}

/// 一筆 PR／issue → 畫面要的形狀（作者只留登入名、標籤只留名字）。
fn slim(v: &Value) -> Value {
    let author = v
        .get("author")
        .and_then(|a| a.get("login"))
        .and_then(Value::as_str);
    let labels: Vec<&str> = v
        .get("labels")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(|x| x.get("name").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    json!({
        "number": v.get("number"),
        "title": v.get("title"),
        "url": v.get("url"),
        "author": author,
        "updated": v.get("updatedAt"),
        "branch": v.get("headRefName"),
        "draft": v.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        "review": v.get("reviewDecision").and_then(Value::as_str).filter(|s| !s.is_empty()),
        "labels": labels,
    })
}

/// 開著的 PR 與 issue（各最多 20 筆，最近更新的在前）。
pub fn open_items(root: &Path) -> Result<Value> {
    let prs = gh_json(
        root,
        &[
            "pr",
            "list",
            "--state",
            "open",
            "--limit",
            LIMIT,
            "--json",
            "number,title,url,author,updatedAt,headRefName,isDraft,reviewDecision",
        ],
    )?;
    let issues = gh_json(
        root,
        &[
            "issue",
            "list",
            "--state",
            "open",
            "--limit",
            LIMIT,
            "--json",
            "number,title,url,author,updatedAt,labels",
        ],
    )?;
    let list = |v: &Value| -> Vec<Value> { v.as_array().into_iter().flatten().map(slim).collect() };
    Ok(json!({"prs": list(&prs), "issues": list(&issues)}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slim_keeps_what_the_list_shows() {
        let v = json!({"number": 12, "title": "修登入", "url": "https://github.com/o/r/pull/12",
            "author": {"login": "me", "name": "Me"}, "updatedAt": "2026-10-21T10:00:00Z",
            "headRefName": "fix-login", "isDraft": true, "reviewDecision": "",
            "labels": [{"name": "bug", "color": "red"}]});
        let s = slim(&v);
        assert_eq!(s["author"], "me");
        assert_eq!(s["branch"], "fix-login");
        assert_eq!(s["draft"], true);
        assert_eq!(s["review"], Value::Null);
        assert_eq!(s["labels"], json!(["bug"]));
    }

    #[test]
    fn missing_program_and_timeout_are_explained() {
        let t = tempfile::tempdir().unwrap();
        let e = run(t.path(), "definitely-not-a-program-xyz", &[], GH_TIMEOUT).unwrap_err();
        assert!(e.to_string().contains("沒有安裝"), "{e}");
        let e = run(t.path(), "sleep", &["5"], Duration::from_millis(100)).unwrap_err();
        assert!(e.to_string().contains("沒回應"), "{e}");
        let (code, out, _) = run(t.path(), "echo", &["hi"], GH_TIMEOUT).unwrap();
        assert_eq!((code, out.trim()), (0, "hi"));
    }
}
