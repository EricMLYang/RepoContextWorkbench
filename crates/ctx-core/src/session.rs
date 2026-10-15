//! P16 agent 會話——帶料開原生終端視窗；MCP 自動掛載 v1 只保證 Claude Code。對應 `repo_context/session.py`。
//!
//! 流程：P6 打包選料 → 確保 spine repo 有 `.mcp.json`（掛本引擎的 MCP server）→
//! 組出 claude 啟動指令（cwd=spine 時靠 cwd `.mcp.json` 自動掛載；cwd=某 repo 時用 `--mcp-config`）。
//! 開視窗是 best-effort（Windows start／macOS Terminal／其他印指令）；指令組裝是純函數可測。
//!
//! **與 Python 版刻意的差異**：`.mcp.json` 的 MCP server 指令指向 Rust 的 `ctx` 執行檔
//! （[`ctx_exe`]，找法見 `exe.rs`）＋`--spine <spine> mcp`，
//! 不再是 `python -m repo_context ... mcp`，也不帶 `PYTHONUTF8`。
//! 已存在且含 `repo_context` 的 `.mcp.json` 照舊不動（搬遷期間不搶 Python 版寫好的設定）。

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Local;
use serde_json::json;

use crate::context;
use crate::exe::ctx_exe;
use crate::pack::{self, RepoRef};
use crate::registry;
use crate::{config, spine, Error, Result};

/// 對應 `session.DEFAULT_TASK`。
pub const DEFAULT_TASK: &str =
    "先讀情境卡，用 3 行說明：你拿到哪些脈絡、這次預計產出什麼、哪些動作需要我先確認，再問我要做什麼";

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
    let spine_abs = crate::fsutil::resolve(spine_dir);
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
    /// 單一 repo 開會話時，呼叫端判定它屬於哪個登記的組（App 的 `term_create` 由 `home_group_for` 算）。
    /// 見 [`scope_with_home`]。Rust 才有。
    pub home_group: Option<&'a str>,
}

/// 組好的會話：在哪跑、跑什麼、會話料寫在哪。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPlan {
    pub cwd: PathBuf,
    pub cmd: Vec<String>,
    pub pack_path: PathBuf,
    /// 這個 agent 是 Claude Code（config 的 `mcp: mcp-config`）：吃 `--settings`，App 據此掛會話狀態 hook。
    pub claude_code: bool,
    /// `cmd` 裡 agent 指令本身佔幾個元素（config 的 `cmd`，例 `["claude"]` 或 `["npx", "claude"]`）：
    /// 要另外加旗標就插在這個位置，不會插進包裝指令中間。
    pub flags_at: usize,
}

/// 單一 repo 開會話、而且它屬於某個登記的組（`home_group`）時的範圍——開場注入（hook 從 cwd 推組）
/// 與情境卡才會是同一個組（2026-10-14 實測：建了只有一個 repo 的 MyProjects，情境卡卻叫「臨時(…)」、
/// 落 `groups/adhoc/`，事件只標 `repo:`，按組回查漏掉）：
/// - 組裡只有這個 repo＝就是那個組：卡名、資料夾、事件標頭都用組。
/// - 組裡還有別的 repo：卡維持只講這個 repo（不把別人拉進範圍），資料夾與事件標頭掛那個組。
///
/// 回傳 (group, repos, 另掛的組)。`home_group` 只是提示：已經給了組、不是單一 repo、
/// 組不存在或 repo 不在組裡，就照原本的範圍。
fn scope_with_home<'a>(
    spine_dir: &Path,
    opts: &SessionOpts<'a>,
) -> (Option<&'a str>, Option<&'a [String]>, Option<&'a str>) {
    let group = opts.group.filter(|g| !g.is_empty());
    let home = opts.home_group.filter(|g| !g.is_empty());
    let (None, Some(h), Some([only])) = (group, home, opts.repos) else {
        return (group, opts.repos, None);
    };
    let members: Vec<String> = match registry::resolve_group(spine_dir, Some(h), None) {
        Ok((_, entries)) => entries.iter().map(registry::Repo::id).collect(),
        Err(_) => return (group, opts.repos, None),
    };
    match members.as_slice() {
        [m] if m == only => (Some(h), None, None),
        ms if ms.contains(only) => (None, opts.repos, Some(h)),
        _ => (group, opts.repos, None),
    }
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
    let (group, repos, home) = scope_with_home(spine_dir, &opts);
    let (gname, entries) = registry::resolve_group(spine_dir, group, repos)?;
    let now = Local::now().naive_local();
    // 會話料＝組情境卡（成員角色／脈動／未結／最近事件／組管家角色）＋文件地圖
    // （L4 回饋「料太長」：地圖不是全文，agent 自己會讀檔，先挑再細讀）
    let refs: Vec<RepoRef> = entries.iter().map(RepoRef::from).collect();
    let text = format!(
        "{}\n{}",
        context::build_session_card(spine_dir, group, repos, Some(now))?,
        pack::pack_index(&refs, 80)
    );
    // 資料夾用登記的原名（只給組時 resolve_group 回的就是原名；給的可能是換寫過的 PM_Group）
    let by_group_only = repos.is_none_or(<[String]>::is_empty);
    let dir = match group {
        Some(_) if by_group_only => gname.as_str(),
        Some(g) => g,
        None => home.unwrap_or("adhoc"),
    };
    let out = spine_dir
        .join("groups")
        .join(dir)
        .join("materials")
        .join(format!("session-{}.md", now.format("%Y%m%d-%H%M%S")));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &text)?;
    let task = opts.task.filter(|t| !t.is_empty());
    // 教練規格：開會話也是出手，留 presented（自己的留痕，不進收件匣）
    let mut toks: Vec<String> = Vec::new();
    if let Some(g) = group.or(home) {
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
            format!("；任務：{}", crate::pystr::take_chars(first, 60))
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
    // 範圍只有一個 repo：不講「組」與成員角色（#73）
    let (card, parts) = if entries.len() == 1 {
        ("情境卡", "repo 現況")
    } else {
        ("組情境卡", "成員角色")
    };
    let prompt = format!(
        "{card}＋文件地圖已備好：{}——先讀情境卡（{parts}、脈動、未結、你的角色），\
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
        claude_code: spec.mcp == "mcp-config",
        flags_at: spec.cmd.len(),
    })
}

/// 接續一場過去的 Claude Code 會話（F-72，Rust 才有）：config 裡這個 agent 的指令＋`--resume <id>`。
/// 回傳 (argv, flags_at)。只有 Claude Code（`mcp: mcp-config`）會接續；id 要先用
/// [`crate::history::valid_session_id`] 驗過。不寫情境卡、不留 presented——是回到舊對話，不是開新工作。
pub fn resume_argv(
    spine_dir: &Path,
    agent: &str,
    id: &str,
    prompt: Option<&str>,
) -> Result<(Vec<String>, usize)> {
    if !crate::history::valid_session_id(id) {
        return Err(Error::invalid(format!("會話 id 不對：{id}")));
    }
    let cfg = config::load(spine_dir)?;
    let agents = cfg.agents();
    let spec = agents
        .iter()
        .find(|(k, _)| k == agent)
        .map(|(_, s)| s)
        .ok_or_else(|| Error::invalid(format!("未知 agent: {agent}")))?;
    if spec.mcp != "mcp-config" {
        return Err(Error::invalid(format!(
            "{agent} 不是 Claude Code，不能接續舊會話"
        )));
    }
    let mut cmd = spec.cmd.clone();
    cmd.extend(["--resume".to_string(), id.to_string()]);
    // 接回時順便說第一句（例：請它補交接）；`claude --resume <id> "<話>"`
    if let Some(p) = prompt.map(str::trim).filter(|p| !p.is_empty()) {
        cmd.push(p.to_string());
    }
    Ok((cmd, spec.cmd.len()))
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

/// macOS Terminal 的 AppleScript：先把 cwd 和每個參數用 shell 規則跳脫（含單引號），
/// 整串再跳脫成 AppleScript 字串。task 文字、repo 路徑裡有 `'`、`"`、`$`、`;` 都不會跳出引號。
fn osascript_for(cwd: &Path, cmd: &[String]) -> String {
    use crate::term::shlex_quote;
    let line = format!(
        "cd {} && {}",
        shlex_quote(&cwd.to_string_lossy()),
        cmd.iter()
            .map(|c| shlex_quote(c))
            .collect::<Vec<_>>()
            .join(" ")
    );
    format!(
        "tell app \"Terminal\" to do script \"{}\"",
        line.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

/// Windows `cmd /k` 會自己解析 `& | < > ^ % !`，`list2cmdline` 的引號擋不住（`\"` 會讓 cmd 的引號狀態錯位）。
/// 有這些字元就不開視窗、改印出指令讓人自己跑。
fn cmd_unsafe(cmd: &[String]) -> bool {
    cmd.iter()
        .any(|a| a.contains(['&', '|', '<', '>', '^', '%', '!', '"', '\n', '\r']))
}

/// 對應 `session.open_terminal`：開原生終端視窗跑 cmd（best-effort；打不開就把指令印出來讓人自己跑）。
/// 刻意差異（2026-10-13）：Python 版沒跳脫參數裡的引號，task 文字可以注入指令；這裡修掉。
pub fn open_terminal(cwd: &Path, cmd: &[String]) -> bool {
    let spawned = if cfg!(windows) {
        !cmd_unsafe(cmd)
            && Command::new("cmd")
                .args(["/c", "start", "cmd", "/k", &list2cmdline(cmd)])
                .current_dir(cwd)
                .spawn()
                .is_ok()
    } else if cfg!(target_os = "macos") {
        Command::new("osascript")
            .args(["-e", &osascript_for(cwd, cmd)])
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osascript_quotes_cannot_break_out() {
        let cmd = vec![
            "claude".to_string(),
            "it's'; rm -rf ~; echo '\"$(x)\"".to_string(),
        ];
        let s = osascript_for(Path::new("/tmp/a'b"), &cmd);
        // AppleScript 字串只有頭尾兩個未跳脫的 "
        let inner = s
            .strip_prefix("tell app \"Terminal\" to do script \"")
            .and_then(|x| x.strip_suffix('"'))
            .unwrap();
        let unescaped = inner.replace("\\\"", "\"").replace("\\\\", "\\");
        assert!(!inner.replace("\\\"", "").contains('"'));
        // 還原成 shell 那一行後，每個參數都是完整的單引號字串
        assert_eq!(
            unescaped,
            "cd '/tmp/a'\"'\"'b' && claude 'it'\"'\"'s'\"'\"'; rm -rf ~; echo '\"'\"'\"$(x)\"'"
        );
    }

    #[test]
    fn cmd_metachars_fall_back_to_printing() {
        assert!(cmd_unsafe(&["claude".into(), "a & del x".into()]));
        assert!(cmd_unsafe(&["claude".into(), "say \"hi\"".into()]));
        assert!(!cmd_unsafe(&["claude".into(), "整理 README 的段落".into()]));
    }
}
