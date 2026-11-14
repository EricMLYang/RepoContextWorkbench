//! `ctx setup`（2026-09-25 Agent 友善輪）——讓「在任何 repo 打開 agent」就接得上。
//! 對應 Python `repo_context/agentsetup.py`。
//!
//! 三件事，全部冪等（已經掛過就跳過，不重複寫）：
//! 1. 使用者設定檔記住 spine 位置（之後 ctx／MCP／hook 都不用帶 --spine）。
//! 2. `--claude`：
//!    - MCP：`claude mcp add --scope user repo-context -- <ctx> mcp`
//!      （user scope＝每個專案都掛得上；server 以專案目錄為 cwd 推主場）
//!    - hooks：`~/.claude/settings.json` 加 SessionStart／SessionEnd（寫入前備份一份 .bak-ctx）
//!    - skill：`~/.claude/skills/repo-context/SKILL.md`（工具怎麼用、什麼時候用）
//!
//! 3. `--codex`（2026-11-14，Rust 才有）：同樣三件事掛到 Codex CLI，agent 走同一個介面（[`AgentCli`]）：
//!    - MCP：`codex mcp add repo-context -- <ctx> mcp`（寫進 `~/.codex/config.toml`）
//!    - hooks：`~/.codex/hooks.json` 加 SessionStart／SessionEnd（形狀跟 Claude Code 的 `hooks` 一樣）。
//!      Codex 的新 hook 要使用者在 `/hooks` 審核信任後才會跑——setup 不代簽，只在結果裡提醒。
//!    - skill：`~/.codex/skills/repo-context/SKILL.md`
//!
//! 沒有 `claude`／`codex` 指令時，MCP 那步改印出手動指令，不失敗；真的註冊失敗則 [`StepResult::ok`]＝false，
//! 入口據此回非 0（c231287：呼叫的人或 agent 會以為掛好了）。
//!
//! **與 Python 刻意的差異**：MCP／hooks 的指令指向 Rust 的 ctx 執行檔（`exe::ctx_exe`，
//! 由呼叫端傳入 `exe`），不是 `<python> -m repo_context`；也就不再帶 `-e PYTHONUTF8=1`。
//! 已存在的 hooks 認兩種形式（Python 的 `repo_context hook`、Rust 的 `ctx hook`），兩版不會重複掛。

use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;
use serde_json::{json, Map, Value};
use std::sync::LazyLock;

use crate::{config, Error, Result};

pub const MCP_NAME: &str = "repo-context";
/// Python 版 hook 指令的記號（`<python> -m repo_context hook <event>`）。
pub const HOOK_MARK: &str = "repo_context hook";

/// setup 認得的 agent CLI：MCP 怎麼註冊、hook 檔在哪、skill 放哪，各家不同的地方都從這裡查。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentCli {
    /// Claude Code（`claude`）
    Claude,
    /// OpenAI Codex CLI（`codex`）
    Codex,
}

impl AgentCli {
    pub const ALL: [AgentCli; 2] = [AgentCli::Claude, AgentCli::Codex];

    /// PATH 上的指令名，也是 `ctx setup --<名>` 的旗標名。
    pub fn program(self) -> &'static str {
        match self {
            AgentCli::Claude => "claude",
            AgentCli::Codex => "codex",
        }
    }

    /// 給人看的名字。
    pub fn label(self) -> &'static str {
        match self {
            AgentCli::Claude => "Claude Code",
            AgentCli::Codex => "Codex",
        }
    }

    /// 設定根目錄：`$CLAUDE_CONFIG_DIR`／`~/.claude`、`$CODEX_HOME`／`~/.codex`。
    pub fn home(self) -> PathBuf {
        let (var, default) = match self {
            AgentCli::Claude => ("CLAUDE_CONFIG_DIR", "~/.claude"),
            AgentCli::Codex => ("CODEX_HOME", "~/.codex"),
        };
        std::env::var_os(var)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::fsutil::expanduser(default))
    }

    /// hook 寫在哪個檔（兩家都是頂層 `{"hooks": {事件: [...]}}`）。
    pub fn hooks_file(self, home: &Path) -> PathBuf {
        home.join(match self {
            AgentCli::Claude => "settings.json",
            AgentCli::Codex => "hooks.json",
        })
    }

    /// skill 檔。
    pub fn skill_file(self, home: &Path) -> PathBuf {
        home.join("skills").join(MCP_NAME).join("SKILL.md")
    }

    /// 註冊 MCP 的指令。Claude：名稱一定在 `--` 與任何 `-e` 之前——`-e` 吃多值，名稱放後面會被當成
    /// 環境變數吞掉（2026-09-25 真機 setup 踩到，c231287）。
    pub fn mcp_add_command(self, exe: &Path) -> Vec<String> {
        let exe = exe.to_string_lossy().into_owned();
        let parts: Vec<&str> = match self {
            AgentCli::Claude => vec![
                "claude", "mcp", "add", "--scope", "user", MCP_NAME, "--", &exe, "mcp",
            ],
            AgentCli::Codex => vec!["codex", "mcp", "add", MCP_NAME, "--", &exe, "mcp"],
        };
        parts.into_iter().map(String::from).collect()
    }

    fn mcp_get_command(self) -> Vec<String> {
        [self.program(), "mcp", "get", MCP_NAME]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// 已註冊時提示怎麼重設。
    fn mcp_remove_hint(self) -> &'static str {
        match self {
            AgentCli::Claude => "claude mcp remove repo-context -s user",
            AgentCli::Codex => "codex mcp remove repo-context",
        }
    }

    /// 開場 hook 那一項（`hooks[]` 裡的一個 handler）。Codex 預設把 hook 給的脈絡截到約 2500 token
    /// （超過只給頭尾＋暫存檔路徑），開場注入會被切掉；`additionalContextLimit: 0`＝照給
    /// （長度由我們自己的 [`crate::hooks::START_BUDGET`] 管）。
    fn hook_handler(self, exe: &Path, event: &str, arg: &str) -> Value {
        let mut h = json!({"type": "command", "command": hook_command(exe, arg)});
        if self == AgentCli::Codex && event == "SessionStart" {
            h["additionalContextLimit"] = json!(0);
        }
        h
    }

    /// hook 裝好之後要不要多講一句（Codex：沒信任前不會跑）。
    fn hooks_note(self) -> &'static str {
        match self {
            AgentCli::Claude => "",
            AgentCli::Codex => {
                "；Codex 的新 hook 要你信任才會跑：開 `codex` 輸入 `/hooks`，把 ctx 的幾項設為信任"
            }
        }
    }
}

/// Rust 版 hook 指令（`…/ctx hook <event>`，路徑可能帶引號）。
static CTX_HOOK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:^|[/\\'"\s])ctx(?:\.exe)?['"]?\s+hook(?:\s|$)"#).expect("固定 regex")
});

/// skill 內容。與 Python `repo_context/assets/SKILL.md` 只差「判斷的沿革、回顧與交辦（Rust 版）」一段（2026-10-10，ideation N-23；對拍時拿掉這段再比）；冪等比對靠它逐字相同。
pub const SKILL_MD: &str = include_str!("assets/SKILL.md");

/// 一步的結果：給人看的一行＋是否成功（失敗要讓入口回非 0）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepResult {
    pub message: String,
    pub ok: bool,
}

impl StepResult {
    fn ok(message: impl Into<String>) -> Self {
        StepResult {
            message: message.into(),
            ok: true,
        }
    }
    fn failed(message: impl Into<String>) -> Self {
        StepResult {
            message: message.into(),
            ok: false,
        }
    }
}

/// shell 可直接貼上的一行指令（POSIX：`shlex.join`；Windows：`subprocess.list2cmdline`）。
pub fn cmdline<S: AsRef<str>>(parts: &[S]) -> String {
    parts
        .iter()
        .map(|p| {
            if cfg!(windows) {
                win_quote(p.as_ref())
            } else {
                sh_quote(p.as_ref())
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Python `shlex.quote`。
fn sh_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    let safe = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }
}

/// Python `subprocess.list2cmdline` 的單一參數規則。
fn win_quote(s: &str) -> String {
    if !s.is_empty() && !s.contains([' ', '\t']) && !s.contains('"') {
        return s.to_string();
    }
    let needq = s.is_empty() || s.contains([' ', '\t']);
    let mut out = String::new();
    if needq {
        out.push('"');
    }
    let mut bs = 0usize;
    for c in s.chars() {
        match c {
            '\\' => bs += 1,
            '"' => {
                out.push_str(&"\\".repeat(bs * 2 + 1));
                out.push('"');
                bs = 0;
            }
            c => {
                out.push_str(&"\\".repeat(bs));
                bs = 0;
                out.push(c);
            }
        }
    }
    if needq {
        out.push_str(&"\\".repeat(bs * 2));
        out.push('"');
    } else {
        out.push_str(&"\\".repeat(bs));
    }
    out
}

/// 對應 `agentsetup.hook_command`：`<ctx> hook <event>`（Python：`<python> -m repo_context hook <event>`）。
pub fn hook_command(exe: &Path, event: &str) -> String {
    cmdline(&[exe.to_string_lossy().as_ref(), "hook", event])
}

/// settings.json 裡這條 command 是不是我們掛的 hook（Python 版或 Rust 版都算）。
pub fn is_our_hook(command: &str) -> bool {
    command.contains(HOOK_MARK) || CTX_HOOK_RE.is_match(command)
}

/// 對應 `agentsetup.mcp_add_command`（Claude Code 的；其他家見 [`AgentCli::mcp_add_command`]）。
pub fn mcp_add_command(exe: &Path) -> Vec<String> {
    AgentCli::Claude.mcp_add_command(exe)
}

/// 對應 `agentsetup.claude_home`：`$CLAUDE_CONFIG_DIR` 或 `~/.claude`。
pub fn claude_home() -> PathBuf {
    AgentCli::Claude.home()
}

/// `$CODEX_HOME` 或 `~/.codex`。
pub fn codex_home() -> PathBuf {
    AgentCli::Codex.home()
}

/// 對應 `agentsetup.skill_text`：`frontmatter=false` 去掉開頭 `---…---`（`ctx instructions` 用）。
pub fn skill_text(frontmatter: bool) -> String {
    let text = SKILL_MD;
    if !frontmatter && text.starts_with("---") {
        // Python text.split("---", 2)[2].lstrip()
        let mut it = text.splitn(3, "---");
        it.next();
        it.next();
        if let Some(rest) = it.next() {
            return rest.trim_start().to_string();
        }
    }
    text.to_string()
}

// ---- 各步驟（回傳一行結果說明）----

/// 對應 `agentsetup.save_spine`（寫使用者設定檔 `~/.config/repo-context/config.json`）。
pub fn save_spine(spine_dir: impl AsRef<Path>) -> Result<StepResult> {
    save_spine_to(&config::user_config_path(), spine_dir)
}

/// 同 [`save_spine`]，設定檔路徑由呼叫端給（測試用）。
pub fn save_spine_to(user_config: &Path, spine_dir: impl AsRef<Path>) -> Result<StepResult> {
    let mut data = config::load_user_config_from(user_config);
    if !data.is_object() {
        data = json!({});
    }
    let resolved = crate::fsutil::resolve(spine_dir.as_ref());
    if let Some(m) = data.as_object_mut() {
        m.insert(
            "spine".into(),
            Value::String(resolved.to_string_lossy().into_owned()),
        );
    }
    let p = config::save_user_config_to(user_config, &data)?;
    Ok(StepResult::ok(format!("已寫入 {}", p.display())))
}

/// 對應 `agentsetup.install_hooks`：settings.json 加 SessionStart／SessionEnd（冪等；寫入前備份 .bak-ctx）。
/// 檔案不是合法 JSON 就不動它（回一行說明，不算失敗——同 Python）。
pub fn install_hooks(settings_path: &Path, exe: &Path) -> Result<StepResult> {
    install_hooks_for(AgentCli::Claude, settings_path, exe)
}

/// 同 [`install_hooks`]，哪一家由呼叫端給（Codex 的檔是 `hooks.json`，形狀一樣）。
pub fn install_hooks_for(cli: AgentCli, hooks_path: &Path, exe: &Path) -> Result<StepResult> {
    let p = hooks_path;
    let mut data: Value = if p.exists() {
        match serde_json::from_str::<Value>(&std::fs::read_to_string(p)?) {
            Ok(v @ Value::Object(_)) => v,
            _ => {
                return Ok(StepResult::ok(format!(
                    "{} 不是合法 JSON，沒有動它；請手動加 hooks（見 `ctx setup --{} --dry-run`）",
                    p.display(),
                    cli.program()
                )))
            }
        }
    } else {
        json!({})
    };
    let root = data
        .as_object_mut()
        .ok_or_else(|| Error::invalid(format!("{} 頂層不是物件", p.display())))?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| Error::invalid(format!("{} 的 hooks 不是物件", p.display())))?;
    let mut added = Vec::new();
    for (event, arg) in [
        ("SessionStart", "session-start"),
        ("SessionEnd", "session-end"),
    ] {
        let entries = hooks
            .entry(event)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| Error::invalid(format!("{} 的 hooks.{event} 不是陣列", p.display())))?;
        let exists = entries.iter().any(|e| {
            e.get("hooks").and_then(Value::as_array).is_some_and(|hs| {
                hs.iter().any(|h| {
                    h.get("command")
                        .and_then(Value::as_str)
                        .is_some_and(is_our_hook)
                })
            })
        });
        if exists {
            continue;
        }
        entries.push(json!({"hooks": [cli.hook_handler(exe, event, arg)]}));
        added.push(event);
    }
    if added.is_empty() {
        return Ok(StepResult::ok("hooks 已存在，略過"));
    }
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if p.exists() {
        let mut bak = p.as_os_str().to_owned();
        bak.push(".bak-ctx");
        std::fs::copy(p, PathBuf::from(bak))?;
    }
    // json.dumps(ensure_ascii=False, indent=2) + "\n"
    let mut text = serde_json::to_string_pretty(&data)?;
    text.push('\n');
    std::fs::write(p, text)?;
    Ok(StepResult::ok(format!(
        "已加入 {} → {}{}",
        added.join(", "),
        p.display(),
        cli.hooks_note()
    )))
}

/// 對應 `agentsetup.install_skill`：寫 `<home>/skills/repo-context/SKILL.md`（內容相同就略過）；
/// `home`＝那一家的設定根目錄（`~/.claude`、`~/.codex`）。
pub fn install_skill(claude_home: &Path) -> Result<StepResult> {
    let p = claude_home.join("skills").join(MCP_NAME).join("SKILL.md");
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let new = skill_text(true);
    if p.exists() && std::fs::read_to_string(&p).ok().as_deref() == Some(new.as_str()) {
        return Ok(StepResult::ok("skill 已是最新，略過"));
    }
    std::fs::write(&p, new)?;
    Ok(StepResult::ok(format!("已寫入 {}", p.display())))
}

/// 已裝的 skill 跟這個執行檔內建的一不一樣（F-74，Rust 才有）：`current`／`stale`（重裝 App 後沒重跑
/// setup，agent 讀到的是舊說明）／`missing`。參考 Orca 的「skill 有新版」提示。
pub fn skill_status(claude_home: &Path) -> &'static str {
    let p = claude_home.join("skills").join(MCP_NAME).join("SKILL.md");
    match std::fs::read_to_string(&p) {
        Err(_) => "missing",
        Ok(t) if t == skill_text(true) => "current",
        Ok(_) => "stale",
    }
}

/// 所有掛過的 agent CLI 合起來的 skill 狀態（`ctx health`、App 的「更新 skill」看這個）：
/// 有一家是舊的＝`stale`；都沒裝＝`missing`；其餘＝`current`。
pub fn skill_status_installed() -> &'static str {
    skill_status_of(
        &installed_clis()
            .iter()
            .map(|c| c.home())
            .collect::<Vec<_>>(),
    )
}

/// 同 [`skill_status_installed`]，各家的設定根目錄由呼叫端給（測試用）。
pub fn skill_status_of(homes: &[PathBuf]) -> &'static str {
    let all: Vec<&str> = homes.iter().map(|h| skill_status(h)).collect();
    if all.contains(&"stale") {
        "stale"
    } else if all.contains(&"current") {
        "current"
    } else {
        "missing"
    }
}

/// 把每一家已裝的 skill 換成這版內建的；回各家的結果（一家都沒裝＝裝到 Claude Code，跟原本一樣）。
pub fn update_installed_skills() -> Result<Vec<(AgentCli, StepResult)>> {
    let mut clis = installed_clis();
    if clis.is_empty() {
        clis.push(AgentCli::Claude);
    }
    clis.into_iter()
        .map(|c| Ok((c, install_skill(&c.home())?)))
        .collect()
}

/// 外部指令的結果（[`install_mcp_with`] 的注入點用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

fn run_cmd(argv: &[String]) -> std::io::Result<CmdOutput> {
    let (prog, rest) = argv
        .split_first()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "空指令"))?;
    let o = Command::new(prog).args(rest).output()?;
    Ok(CmdOutput {
        success: o.status.success(),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    })
}

pub use crate::exe::which;

/// 對應 `agentsetup.install_mcp`：`claude mcp get` 已有就略過，否則 `claude mcp add`。
pub fn install_mcp(exe: &Path) -> StepResult {
    install_mcp_for(AgentCli::Claude, exe)
}

/// 同 [`install_mcp`]，哪一家由呼叫端給。
pub fn install_mcp_for(cli: AgentCli, exe: &Path) -> StepResult {
    install_mcp_cli_with(cli, exe, which(cli.program()).is_some(), &mut run_cmd)
}

/// `install_mcp` 的本體：`claude_found`＝PATH 上有沒有 claude、`run`＝跑外部指令（測試注入）。
pub fn install_mcp_with(
    exe: &Path,
    claude_found: bool,
    run: &mut dyn FnMut(&[String]) -> std::io::Result<CmdOutput>,
) -> StepResult {
    install_mcp_cli_with(AgentCli::Claude, exe, claude_found, run)
}

/// 同 [`install_mcp_with`]，哪一家由呼叫端給：`<cli> mcp get` 已有就略過，否則 `<cli> mcp add`。
pub fn install_mcp_cli_with(
    cli: AgentCli,
    exe: &Path,
    found: bool,
    run: &mut dyn FnMut(&[String]) -> std::io::Result<CmdOutput>,
) -> StepResult {
    let add = cli.mcp_add_command(exe);
    if !found {
        return StepResult::ok(format!(
            "找不到 {} 指令；請手動執行：{}",
            cli.program(),
            cmdline(&add)
        ));
    }
    if run(&cli.mcp_get_command()).is_ok_and(|o| o.success) {
        return StepResult::ok(format!(
            "MCP 已註冊，略過（要重設先 `{}`）",
            cli.mcp_remove_hint()
        ));
    }
    match run(&add) {
        Ok(o) if o.success => StepResult::ok(match cli {
            AgentCli::Claude => "已註冊（user scope）",
            AgentCli::Codex => "已註冊（Codex 的 config.toml）",
        }),
        Ok(o) => {
            let msg = if o.stderr.is_empty() {
                o.stdout
            } else {
                o.stderr
            };
            StepResult::failed(format!(
                "註冊失敗：{}",
                crate::pystr::take_chars(msg.trim(), 200)
            ))
        }
        Err(e) => StepResult::failed(format!(
            "註冊失敗：{}",
            crate::pystr::take_chars(e.to_string().trim(), 200)
        )),
    }
}

/// setup 的環境：spine、ctx 執行檔、各家 agent 的設定根目錄、使用者設定檔。測試可整組自己給。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupEnv {
    pub spine_dir: PathBuf,
    pub exe: PathBuf,
    pub claude_home: PathBuf,
    pub codex_home: PathBuf,
    pub user_config: PathBuf,
}

impl SetupEnv {
    /// 從行程環境推（`exe::ctx_exe`、`CLAUDE_CONFIG_DIR`、`CODEX_HOME`、`XDG_CONFIG_HOME`）。
    pub fn detect(spine_dir: impl AsRef<Path>) -> Result<Self> {
        Ok(SetupEnv {
            spine_dir: spine_dir.as_ref().to_path_buf(),
            exe: crate::exe::ctx_exe()?,
            claude_home: claude_home(),
            codex_home: codex_home(),
            user_config: config::user_config_path(),
        })
    }
    /// 這一家的設定根目錄。
    pub fn home(&self, cli: AgentCli) -> &Path {
        match cli {
            AgentCli::Claude => &self.claude_home,
            AgentCli::Codex => &self.codex_home,
        }
    }
    /// Claude Code 的 settings.json。
    pub fn settings_path(&self) -> PathBuf {
        self.hooks_path(AgentCli::Claude)
    }
    /// 這一家的 hook 檔。
    pub fn hooks_path(&self, cli: AgentCli) -> PathBuf {
        cli.hooks_file(self.home(cli))
    }
}

/// setup 的一步（對應 Python `plan()` 回的 {desc, run}）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    SaveSpine,
    Mcp(AgentCli),
    Hooks(AgentCli),
    Skill(AgentCli),
}

impl Step {
    /// 這步要做什麼（`--dry-run` 印這行）。
    pub fn desc(self, env: &SetupEnv) -> String {
        match self {
            Step::SaveSpine => format!(
                "記住 spine 位置：{}",
                crate::fsutil::resolve(&env.spine_dir).display()
            ),
            Step::Mcp(cli) => format!(
                "註冊 MCP server：{}",
                cmdline(&cli.mcp_add_command(&env.exe))
            ),
            Step::Hooks(cli) => format!(
                "掛 hooks（SessionStart／SessionEnd）到 {}：{}",
                env.hooks_path(cli).display(),
                hook_command(&env.exe, "session-start")
            ),
            Step::Skill(cli) => {
                format!("安裝 skill 到 {}", cli.skill_file(env.home(cli)).display())
            }
        }
    }

    /// 執行這步。
    pub fn run(self, env: &SetupEnv) -> Result<StepResult> {
        match self {
            Step::SaveSpine => save_spine_to(&env.user_config, &env.spine_dir),
            Step::Mcp(cli) => Ok(install_mcp_for(cli, &env.exe)),
            Step::Hooks(cli) => install_hooks_for(cli, &env.hooks_path(cli), &env.exe),
            Step::Skill(cli) => install_skill(env.home(cli)),
        }
    }
}

/// 對應 `agentsetup.plan`：記住 spine；每一家再加 MCP／hooks／skill。
pub fn plan(clis: &[AgentCli]) -> Vec<Step> {
    let mut steps = vec![Step::SaveSpine];
    for &cli in clis {
        steps.extend([Step::Mcp(cli), Step::Hooks(cli), Step::Skill(cli)]);
    }
    steps
}

/// 這台機器上哪幾家已經掛過 repo-context（skill 檔在）——重裝後更新 skill、`ctx health` 檢查版本都看這個。
pub fn installed_clis() -> Vec<AgentCli> {
    AgentCli::ALL
        .into_iter()
        .filter(|c| c.skill_file(&c.home()).exists())
        .collect()
}
