//! ctx-cli 測試共用：跑真 binary（對應 conftest.run_cli／test_agent_cli_e2e._ctx）。
//!
//! 環境變數只設在子進程上，不改測試進程的全域：temp `XDG_CONFIG_HOME`（不碰使用者真的
//! `~/.config/repo-context`）、`REPOCTX_NO_BACKGROUND=1`、清掉 `REPOENGINE_SPINE`、假 git 身分。
#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[allow(unused_imports)]
pub use ctx_core::testkit::{git, make_git_repo, spine, spine_with_repos};

pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout 不是 JSON（{e}）：{}", self.stdout))
    }
    pub fn all(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// 一次 CLI 呼叫的設定。
#[derive(Clone)]
pub struct Ctx {
    xdg: PathBuf,
    spine: Option<PathBuf>,
    cwd: Option<PathBuf>,
    home: Option<PathBuf>,
    env: Vec<(String, String)>,
    stdin: Option<String>,
}

/// `tmp`＝這個測試的 temp 根目錄（XDG_CONFIG_HOME 放在底下）。
pub fn ctx(tmp: &Path) -> Ctx {
    Ctx {
        xdg: tmp.join(".xdg"),
        spine: None,
        cwd: None,
        home: None,
        env: Vec::new(),
        stdin: None,
    }
}

impl Ctx {
    pub fn spine(mut self, p: &Path) -> Self {
        self.spine = Some(p.to_path_buf());
        self
    }
    pub fn cwd(mut self, p: &Path) -> Self {
        self.cwd = Some(p.to_path_buf());
        self
    }
    /// 對應 `_ctx(home=...)`：HOME／XDG／CLAUDE_CONFIG_DIR 指到 home，PATH 只剩系統目錄。
    pub fn home(mut self, p: &Path) -> Self {
        self.home = Some(p.to_path_buf());
        self
    }
    pub fn env(mut self, k: &str, v: impl AsRef<std::ffi::OsStr>) -> Self {
        self.env
            .push((k.to_string(), v.as_ref().to_string_lossy().into_owned()));
        self
    }
    pub fn stdin(mut self, s: &str) -> Self {
        self.stdin = Some(s.to_string());
        self
    }

    pub fn run(&self, args: &[&str]) -> Out {
        let mut c = Command::new(env!("CARGO_BIN_EXE_ctx"));
        if let Some(sp) = &self.spine {
            c.arg("--spine").arg(sp);
        }
        c.args(args);
        c.env_remove("REPOENGINE_SPINE")
            .env_remove("CTX_CWD")
            .env("REPOCTX_NO_BACKGROUND", "1")
            .env("XDG_CONFIG_HOME", &self.xdg)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t");
        if let Some(h) = &self.home {
            c.env("HOME", h)
                .env("XDG_CONFIG_HOME", h.join(".config"))
                .env("CLAUDE_CONFIG_DIR", h.join(".claude"))
                .env("PATH", "/usr/bin:/bin");
        }
        for (k, v) in &self.env {
            c.env(k, v);
        }
        let cwd = self
            .cwd
            .clone()
            .unwrap_or_else(|| self.xdg.parent().unwrap().to_path_buf());
        c.current_dir(cwd);
        c.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn().expect("跑 ctx 失敗");
        {
            let mut si = child.stdin.take().unwrap();
            if let Some(s) = &self.stdin {
                si.write_all(s.as_bytes()).unwrap();
            }
        }
        let o = child.wait_with_output().unwrap();
        Out {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    }

    /// `run_cli(..., check=True)`。
    pub fn ok(&self, args: &[&str]) -> Out {
        let o = self.run(args);
        assert_eq!(
            o.code, 0,
            "CLI 失敗 rc={}\ncmd={args:?}\nstdout={}\nstderr={}",
            o.code, o.stdout, o.stderr
        );
        o
    }
}

/// 把檔案 mtime 設成 `days` 天前。
pub fn age(p: &Path, days: u64) {
    let t = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 86400);
    std::fs::File::options()
        .write(true)
        .open(p)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_accessed(t).set_modified(t))
        .unwrap();
}

/// 今天的事件檔。
pub fn today_file(spine_dir: &Path) -> PathBuf {
    spine_dir
        .join("spine")
        .join("events")
        .join(format!("{}.md", today()))
}

pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}
