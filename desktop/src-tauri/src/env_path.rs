//! PATH 修補。
//!
//! 從 Finder／Dock 打開的 .app 只拿到 launchd 給的 PATH（`/usr/bin:/bin:/usr/sbin:/sbin`），
//! 找不到使用者裝在 Homebrew、~/.local/bin 的 `git`／`gh`／`claude`（AGENTS.md：從 Finder 打開也要找得到）。
//! 做法：啟動時跑一次使用者的 login＋interactive shell 印出 PATH（加逾時，壞掉的 rc 檔不會卡住啟動）；
//! 失敗就退回「目前 PATH＋常見安裝位置」。兩種結果都再補上常見位置中缺的，順序：shell 給的優先。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MARK: &str = "__RC_PATH__";

/// 常見安裝位置（shell 取不到時的退路）。
pub fn extra_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".local/bin"),
        home.join(".cargo/bin"),
    ]
}

/// 從 shell 輸出撈出夾在兩個標記中間的 PATH（rc 檔可能印歡迎詞、oh-my-zsh 訊息之類的雜訊）。
pub fn parse_marked(out: &str) -> Option<String> {
    let start = out.find(MARK)? + MARK.len();
    let rest = &out[start..];
    let end = rest.find(MARK)?;
    let p = rest[..end].trim();
    (!p.is_empty()).then(|| p.to_string())
}

/// 跑 `$SHELL -ilc 'printf …"$PATH"…'`，最多等 `timeout`；逾時就殺掉回 `None`。
pub fn login_shell_path(shell: &str, timeout: Duration) -> Option<String> {
    let script = format!("printf '%s%s%s' '{MARK}' \"$PATH\" '{MARK}'");
    let mut child = Command::new(shell)
        .args(["-ilc", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    parse_marked(&out)
}

/// 把 `extras` 中 `base` 還沒有的目錄接在後面（不重複、保留 base 順序）。
pub fn merge(base: &str, extras: &[PathBuf]) -> String {
    let mut dirs: Vec<PathBuf> = std::env::split_paths(base)
        .filter(|p| !p.as_os_str().is_empty())
        .collect();
    for e in extras {
        if !dirs.contains(e) {
            dirs.push(e.clone());
        }
    }
    std::env::join_paths(dirs)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| base.to_string())
}

/// 算出修補後的 PATH（不改環境，方便測試）。
pub fn resolve(shell: Option<&str>, current: &str, home: &Path, timeout: Duration) -> String {
    let from_shell = shell
        .filter(|s| !s.is_empty())
        .and_then(|s| login_shell_path(s, timeout));
    merge(from_shell.as_deref().unwrap_or(current), &extra_dirs(home))
}

/// 啟動時呼叫一次：修補本行程的 PATH（之後 spawn 的 git／gh／claude／PTY 都吃得到）。Windows 不動。
pub fn fix_path() {
    if cfg!(windows) {
        return;
    }
    let current = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let shell = std::env::var("SHELL").ok();
    let fixed = resolve(shell.as_deref(), &current, &home, Duration::from_secs(3));
    std::env::set_var("PATH", fixed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_marked_ignores_rc_noise() {
        let out = format!("Welcome!\n{MARK}/a/bin:/usr/bin{MARK}trailing");
        assert_eq!(parse_marked(&out).as_deref(), Some("/a/bin:/usr/bin"));
        assert_eq!(parse_marked("no marks"), None);
        assert_eq!(parse_marked(&format!("{MARK}{MARK}")), None);
    }

    #[test]
    fn merge_appends_missing_without_duplicates() {
        let home = PathBuf::from("/Users/x");
        let got = merge("/usr/bin:/opt/homebrew/bin", &extra_dirs(&home));
        assert_eq!(
            got,
            "/usr/bin:/opt/homebrew/bin:/usr/local/bin:/Users/x/.local/bin:/Users/x/.cargo/bin"
        );
    }

    #[cfg(unix)]
    #[test]
    fn login_shell_path_reads_path_from_sh() {
        let p = login_shell_path("/bin/sh", Duration::from_secs(10)).expect("sh 應印出 PATH");
        assert!(!p.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn resolve_falls_back_when_shell_missing_or_slow() {
        let home = PathBuf::from("/h");
        let got = resolve(
            Some("/nonexistent/shell"),
            "/usr/bin:/bin",
            &home,
            Duration::from_secs(1),
        );
        assert!(got.starts_with("/usr/bin:/bin:/opt/homebrew/bin"));
        assert!(got.ends_with("/h/.cargo/bin"));
        // 逾時：一個睡很久的「shell」要在逾時內被放棄
        let dir = tempfile::tempdir().unwrap();
        let slow = dir.path().join("slowsh");
        std::fs::write(&slow, "#!/bin/sh\nsleep 30\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&slow, std::fs::Permissions::from_mode(0o755)).unwrap();
        let t0 = Instant::now();
        assert_eq!(
            login_shell_path(slow.to_str().unwrap(), Duration::from_millis(300)),
            None
        );
        assert!(t0.elapsed() < Duration::from_secs(5));
    }
}
