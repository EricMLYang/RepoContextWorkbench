//! P16 agent 會話——帶料開原生終端視窗；MCP 自動掛載 v1 只保證 Claude Code。對應 `repo_context/session.py`。
//!
//! 流程：P6 打包選料 → 確保 spine repo 有 `.mcp.json`（掛本引擎的 MCP server）→
//! 組出 claude 啟動指令（cwd=spine 時靠 cwd `.mcp.json` 自動掛載；cwd=某 repo 時用 `--mcp-config`）。
//! 開視窗是 best-effort（Windows start／macOS Terminal／其他印指令）；指令組裝是純函數可測。
//!
//! **與 Python 版刻意的差異**：`.mcp.json` 的 MCP server 指令指向 Rust 的 `ctx` 執行檔
//! （[`ctx_exe`]：`std::env::current_exe()`，或與它同目錄的 `ctx`）＋`--spine <spine> mcp`，
//! 不再是 `python -m repo_context ... mcp`，也不帶 `PYTHONUTF8`。
//! 已存在且含 `repo_context` 的 `.mcp.json` 照舊不動（搬遷期間不搶 Python 版寫好的設定）。

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Local;
use serde_json::json;

use crate::context;
use crate::pack::{self, RepoRef};
use crate::registry;
use crate::{config, spine, Error, Result};

/// 對應 `session.DEFAULT_TASK`。
pub const DEFAULT_TASK: &str =
    "先讀情境卡，用 3 行說明：你拿到哪些脈絡、這次預計產出什麼、哪些動作需要我先確認，再問我要做什麼";

/// MCP server 要跑的 `ctx` 執行檔：自己就是 `ctx` 就用自己；否則（例：桌面 App）找同目錄的 `ctx`；
/// 都沒有就退回自己。
pub fn ctx_exe() -> Result<PathBuf> {
    let me = std::env::current_exe()?;
    if me.file_stem().is_some_and(|s| s == "ctx") {
        return Ok(me);
    }
    let sibling = me.with_file_name(format!("ctx{}", std::env::consts::EXE_SUFFIX));
    Ok(if sibling.is_file() { sibling } else { me })
}

/// 對應 `session.ensure_mcp_json`：寫 spine repo 的 `.mcp.json`（已存在且含 repo_context 就不動）。回傳路徑。
pub fn ensure_mcp_json(spine_dir: &Path) -> Result<PathBuf> {
    ensure_mcp_json_with(spine_dir, &ctx_exe()?)
}

/// 同 [`ensure_mcp_json`]，`ctx` 執行檔由呼叫端給。
pub fn ensure_mcp_json_with(spine_dir: &Path, ctx: &Path) -> Result<PathBuf> {
    let p = spine_dir.join(".mcp.json");
    if let Ok(text) = std::fs::read_to_string(&p) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            if v.get("mcpServers")
                .and_then(|m| m.get("repo_context"))
                .is_some()
            {
                return Ok(p);
            }
        }
    }
    let spine_abs = crate::pystr::resolve(spine_dir);
    let data = json!({"mcpServers": {"repo_context": {
        "command": ctx.to_string_lossy(),
        "args": ["--spine", spine_abs.to_string_lossy(), "mcp"],
    }}});
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    serde::Serialize::serialize(&data, &mut ser)?;
    std::fs::write(&p, buf)?;
    Ok(p)
}

/// [`build`] 的參數（對應 Python 的關鍵字參數）。`agent` 空＝claude。
#[derive(Debug, Clone, Default)]
pub struct SessionOpts<'a> {
    pub group: Option<&'a str>,
    pub repos: Option<&'a [String]>,
    pub repo: Option<&'a str>,
    pub task: Option<&'a str>,
    pub agent: Option<&'a str>,
}

/// 組好的會話：在哪跑、跑什麼、會話料寫在哪。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPlan {
    pub cwd: PathBuf,
    pub cmd: Vec<String>,
    pub pack_path: PathBuf,
}

/// 對應 `session.build`：組會話——打包＋掛載＋指令。
///
/// agent＝config `agents:` 註冊表的 key（預設 claude）——terminal 對 agent 無關，spawn 哪家 CLI
/// 只是查表；MCP 掛載是各家不同的膠水，由表上 `mcp` 欄決定（v1 只保證 Claude Code 的自動掛載）。
pub fn build(spine_dir: &Path, opts: SessionOpts<'_>) -> Result<SessionPlan> {
    build_with(spine_dir, opts, &ctx_exe()?)
}

/// 同 [`build`]，`.mcp.json` 裡的 `ctx` 執行檔由呼叫端給。
pub fn build_with(spine_dir: &Path, opts: SessionOpts<'_>, ctx: &Path) -> Result<SessionPlan> {
    let agent = opts.agent.filter(|a| !a.is_empty()).unwrap_or("claude");
    let cfg = config::load(spine_dir)?;
    let agents = cfg.agents();
    let Some((_, spec)) = agents.iter().find(|(k, _)| k == agent) else {
        return Err(Error::invalid(format!(
            "未知 agent: {agent}（config agents: 目前有 {}）",
            agents
                .iter()
                .map(|(k, _)| k.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    };
    let (gname, entries) = registry::resolve_group(spine_dir, opts.group, opts.repos)?;
    let now = Local::now().naive_local();
    // 會話料＝組情境卡（成員角色／脈動／未結／最近事件／組管家角色）＋文件地圖
    // （L4 回饋「料太長」：地圖不是全文，agent 自己會讀檔，先挑再細讀）
    let refs: Vec<RepoRef> = entries.iter().map(RepoRef::from).collect();
    let text = format!(
        "{}\n{}",
        context::build_context(spine_dir, opts.group, opts.repos, None, Some(now))?,
        pack::pack_index(&refs, 80)
    );
    let group = opts.group.filter(|g| !g.is_empty());
    let out = spine_dir
        .join("groups")
        .join(group.unwrap_or("adhoc"))
        .join("materials")
        .join(format!("session-{}.md", now.format("%Y%m%d-%H%M%S")));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &text)?;
    let task = opts.task.filter(|t| !t.is_empty());
    // 教練規格：開會話也是出手，留 presented（自己的留痕，不進收件匣）
    let mut toks: Vec<String> = Vec::new();
    if let Some(g) = group {
        toks.push(format!("group:{g}"));
    }
    if let Some(r) = opts.repo.filter(|r| !r.is_empty()) {
        toks.push(format!("repo:{r}"));
    }
    let out_name = out
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let task_note = task
        .map(|t| {
            let first = crate::pystr::splitlines(t).first().copied().unwrap_or("");
            format!("；任務：{}", crate::fsutil::take_chars(first, 60))
        })
        .unwrap_or_default();
    spine::append_event(
        spine_dir,
        "presented",
        "session",
        &toks,
        &format!(
            "組情境卡＋文件地圖：{gname}（{} repo）→ {out_name}{task_note}",
            entries.len()
        ),
        Some(now),
    )?;
    let mcp_json = ensure_mcp_json_with(spine_dir, ctx)?;
    let repo = opts.repo.filter(|r| !r.is_empty());
    let cwd = match repo {
        Some(r) => registry::get_repo(spine_dir, r)?.path(),
        None => spine_dir.to_path_buf(),
    };
    let prompt = format!(
        "組情境卡＋文件地圖已備好：{}——先讀情境卡（成員角色、脈動、未結、你的角色），\
         需要細節再照地圖挑檔細讀，不要整包讀完。任務：{}",
        out.display(),
        task.unwrap_or(DEFAULT_TASK)
    );
    let mut cmd = spec.cmd.clone();
    if repo.is_some() && spec.mcp == "mcp-config" {
        // 等號形式必須：claude 的 --mcp-config 吃多值，空格形式會把後面的
        // 帶料 prompt 也吞成設定檔路徑（L4 實錘：Invalid MCP configuration）
        cmd.push(format!("--mcp-config={}", mcp_json.display()));
    }
    cmd.extend(spec.prompt_flag.iter().cloned());
    cmd.push(prompt);
    Ok(SessionPlan {
        cwd,
        cmd,
        pack_path: out,
    })
}

/// Python `subprocess.list2cmdline`（Windows 命令列引號規則）。
pub fn list2cmdline(args: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    for a in args {
        let needs = a.is_empty() || a.contains([' ', '\t']);
        let mut s = String::new();
        if needs {
            s.push('"');
        }
        let mut backslashes = 0usize;
        for c in a.chars() {
            match c {
                '\\' => backslashes += 1,
                '"' => {
                    s.push_str(&"\\".repeat(backslashes * 2 + 1));
                    s.push('"');
                    backslashes = 0;
                }
                _ => {
                    s.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                    s.push(c);
                }
            }
        }
        if needs {
            s.push_str(&"\\".repeat(backslashes * 2));
            s.push('"');
        } else {
            s.push_str(&"\\".repeat(backslashes));
        }
        out.push(s);
    }
    out.join(" ")
}

/// 對應 `session.open_terminal`：開原生終端視窗跑 cmd（best-effort；打不開就把指令印出來讓人自己跑）。
pub fn open_terminal(cwd: &Path, cmd: &[String]) -> bool {
    let spawned = if cfg!(windows) {
        Command::new("cmd")
            .args(["/c", "start", "cmd", "/k", &list2cmdline(cmd)])
            .current_dir(cwd)
            .spawn()
            .is_ok()
    } else if cfg!(target_os = "macos") {
        let quoted = cmd
            .iter()
            .map(|c| format!("'{c}'"))
            .collect::<Vec<_>>()
            .join(" ")
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let script = format!(
            "tell app \"Terminal\" to do script \"cd '{}' && {quoted}\"",
            cwd.display()
        );
        Command::new("osascript")
            .args(["-e", &script])
            .spawn()
            .is_ok()
    } else {
        false
    };
    if !spawned {
        println!(
            "開不了終端視窗——自己跑：\ncd {}\n{}",
            cwd.display(),
            list2cmdline(cmd)
        );
    }
    spawned
}
