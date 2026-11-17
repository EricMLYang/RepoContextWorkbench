//! 找 `hc`、找 core 的 socket、core 沒在跑就用 `hc server start` 叫起來。
//!
//! socket 在哪照 herdr-core `docs/security.md`「State and endpoints」：
//! - 狀態目錄＝`$XDG_STATE_HOME`（預設 `~/.local/state`）`/herdr-core/<session>/`
//! - socket＝`$XDG_RUNTIME_DIR/herdr-core/<session>.sock`；沒設就 `/run/user/<uid>/herdr-core/`（存在的話）；再不然放在狀態目錄裡
//! - config 的 `socket` 可以改它；`HC_SOCKET` 也可以——**我們不認 `HC_SOCKET`**（App 可能是從某個 hc 終端機裡被打開的，
//!   那個變數指的是別人的 core），叫 `hc` 時也把它拿掉。
//! - core 起來之後會把真正在聽的 socket 寫進狀態目錄的 `core.lock`——先信它，再用上面算出來的。
//!
//! 自動啟動不自己複製 herdr-core 的那套（macOS 登入階段、launchd、鎖）：直接跑 `hc server start`，它自己會處理。
//! `HC_SESSION` 一律帶我們自己的名字（預設 `ctx`），不碰使用者自己的 Herdr。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::client::RpcError;

/// 呼叫 `hc` 時不往下傳的變數：別的 core 的位置、我們在哪個 hc 終端機裡（App 可能是從那裡打開的）。
const FOREIGN_ENV: &[&str] = &[
    "HC_SOCKET",
    "HC_TERMINAL_ID",
    "HC_TERMINAL_SOCKET",
    "HC_SESSION",
];

/// 怎麼找、怎麼叫 herdr-core。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HcSetup {
    /// `hc`（或 `herdr-core`）執行檔
    pub exe: PathBuf,
    /// `HC_SESSION`：我們自己的 core（預設 `ctx`）
    pub session: String,
    /// 叫 `hc` 時另外帶的環境變數（測試拿來隔離 `XDG_*`；正式執行是空的）。也用來算 socket 位置。
    pub env: Vec<(String, String)>,
}

impl HcSetup {
    fn var(&self, k: &str) -> Option<String> {
        self.env
            .iter()
            .rev()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
            .or_else(|| std::env::var(k).ok())
            .filter(|v| !v.is_empty())
    }

    fn home(&self) -> PathBuf {
        PathBuf::from(self.var("HOME").unwrap_or_else(|| "/".into()))
    }

    /// 這個 session 的狀態目錄。
    pub fn state_dir(&self) -> PathBuf {
        let base = self
            .var("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home().join(".local/state"));
        base.join("herdr-core").join(&self.session)
    }

    fn config_file(&self) -> PathBuf {
        let base = self
            .var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home().join(".config"));
        base.join("herdr-core").join("config.toml")
    }

    /// 照規則算出來的 socket（core 還沒起來時用）。
    pub fn default_socket(&self) -> PathBuf {
        if let Some(s) = config_socket(&self.config_file()) {
            return s;
        }
        let run = self.var("XDG_RUNTIME_DIR").map(PathBuf::from).or_else(|| {
            #[cfg(unix)]
            {
                // SAFETY: getuid 沒有失敗的情況，也不碰記憶體
                let uid = unsafe { libc::getuid() };
                Some(PathBuf::from(format!("/run/user/{uid}"))).filter(|p| p.is_dir())
            }
            #[cfg(not(unix))]
            {
                None
            }
        });
        let dir = match run {
            Some(r) => r.join("herdr-core"),
            None => self.state_dir(),
        };
        dir.join(format!("{}.sock", self.session))
    }

    /// 要試的 socket：`core.lock` 記的（core 在跑時最準）、再來是算出來的。
    pub fn socket_candidates(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Ok(s) = std::fs::read_to_string(self.state_dir().join("core.lock")) {
            let s = s.trim();
            if !s.is_empty() {
                out.push(PathBuf::from(s));
            }
        }
        let d = self.default_socket();
        if !out.contains(&d) {
            out.push(d);
        }
        out
    }

    /// 一個 `hc` 指令（帶好隔離用的環境）。
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.exe);
        for k in FOREIGN_ENV {
            cmd.env_remove(k);
        }
        // core 是我們叫起來的：上一層 Claude Code 會話的身分不能被 core（和它開的每個會話）繼承
        for k in crate::term::CLAUDE_SESSION_ENV {
            cmd.env_remove(k);
        }
        cmd.env("HC_SESSION", &self.session);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::null());
        cmd
    }

    /// `hc --version` 的版本號（`hc 0.1.21` → `0.1.21`）。不碰 core。
    pub fn version(&self) -> Option<String> {
        let out = self
            .command()
            .arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        let v = s.split_whitespace().last()?.to_string();
        out.status.success().then_some(v)
    }

    /// `hc server start`：叫起 core（已經在跑就什麼都不做）。最多等 `timeout`。
    pub fn start_core(&self, timeout: Duration) -> std::result::Result<(), RpcError> {
        let mut child = self
            .command()
            .args(["server", "start"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| RpcError::unavailable(format!("跑不起 {}：{e}", self.exe.display())))?;
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(st)) if st.success() => return Ok(()),
                Ok(Some(st)) => {
                    let mut err = String::new();
                    if let Some(mut e) = child.stderr.take() {
                        use std::io::Read;
                        let _ = e.read_to_string(&mut err);
                    }
                    return Err(RpcError::unavailable(format!(
                        "hc server start 失敗（{st}）：{}",
                        err.trim()
                    )));
                }
                Ok(None) if start.elapsed() > timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(RpcError::unavailable(format!(
                        "hc server start 沒在 {} 秒內結束",
                        timeout.as_secs()
                    )));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => return Err(RpcError::unavailable(format!("等 hc server start：{e}"))),
            }
        }
    }
}

/// config.toml 頂層的 `socket = "…"`（只認這一種最簡單的寫法；其他寫法就當沒設，退回預設位置）。
fn config_socket(p: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(p).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            break; // 進到別的 table 了
        }
        let Some(rest) = line.strip_prefix("socket") else {
            continue;
        };
        let Some(v) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let v = v.trim();
        let v = v.split(" #").next().unwrap_or(v).trim();
        let v = v
            .strip_prefix('"')
            .and_then(|x| x.strip_suffix('"'))
            .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))?;
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }
    None
}

/// 找 `hc`：config `session.hc_path`（有設就只用它——指錯了要讓人看到，不默默換一個）→ PATH 上的 `hc`／`herdr-core`
/// → `~/.local/bin/herdr-core`。
pub fn find_hc(configured: Option<&Path>) -> std::result::Result<PathBuf, String> {
    if let Some(p) = configured {
        let p = crate::fsutil::expanduser(p);
        return if crate::exe::is_executable(&p) {
            Ok(p)
        } else {
            Err(format!(
                "config session.hc_path 指到的 hc 不存在或不能執行（{}）",
                p.display()
            ))
        };
    }
    for name in ["hc", "herdr-core"] {
        if let Some(p) = crate::exe::which(name) {
            return Ok(p);
        }
    }
    let local = crate::fsutil::expanduser("~/.local/bin/herdr-core");
    if crate::exe::is_executable(&local) {
        return Ok(local);
    }
    Err(
        "找不到 herdr-core（hc）：PATH 上沒有 hc／herdr-core，~/.local/bin/herdr-core 也沒有"
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(env: &[(&str, &str)]) -> HcSetup {
        HcSetup {
            exe: PathBuf::from("/nonexistent/hc"),
            session: "ctx".into(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn socket_follows_runtime_dir_then_lock_file() {
        let t = tempfile::tempdir().unwrap();
        let state = t.path().join("state");
        let s = setup(&[
            ("XDG_RUNTIME_DIR", "/tmp/rt"),
            ("XDG_STATE_HOME", state.to_str().unwrap()),
            ("XDG_CONFIG_HOME", t.path().join("cfg").to_str().unwrap()),
        ]);
        assert_eq!(s.state_dir(), state.join("herdr-core/ctx"));
        assert_eq!(
            s.default_socket(),
            PathBuf::from("/tmp/rt/herdr-core/ctx.sock")
        );
        assert_eq!(
            s.socket_candidates(),
            vec![PathBuf::from("/tmp/rt/herdr-core/ctx.sock")]
        );
        // core 在跑時 core.lock 記的優先
        std::fs::create_dir_all(s.state_dir()).unwrap();
        std::fs::write(s.state_dir().join("core.lock"), "/tmp/other/ctx.sock").unwrap();
        assert_eq!(
            s.socket_candidates(),
            vec![
                PathBuf::from("/tmp/other/ctx.sock"),
                PathBuf::from("/tmp/rt/herdr-core/ctx.sock")
            ]
        );
        // config 的 socket
        std::fs::create_dir_all(t.path().join("cfg/herdr-core")).unwrap();
        std::fs::write(
            t.path().join("cfg/herdr-core/config.toml"),
            "notes = false\nsocket = \"/tmp/x.sock\" # mine\n[templates]\nsocket = \"/no\"\n",
        )
        .unwrap();
        assert_eq!(s.default_socket(), PathBuf::from("/tmp/x.sock"));
    }

    #[test]
    fn find_hc_honours_configured_path_only() {
        let e = find_hc(Some(Path::new("/nonexistent/dir/hc"))).unwrap_err();
        assert!(e.contains("session.hc_path"), "{e}");
    }

    #[test]
    fn command_drops_foreign_core_and_claude_identity() {
        let s = setup(&[("XDG_STATE_HOME", "/tmp/s")]);
        let cmd = s.command();
        let envs: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(envs.contains(&("HC_SESSION".into(), Some("ctx".into()))));
        assert!(envs.contains(&("HC_SOCKET".into(), None)));
        assert!(envs.contains(&("CLAUDECODE".into(), None)));
        assert!(envs.contains(&("XDG_STATE_HOME".into(), Some("/tmp/s".into()))));
    }
}
