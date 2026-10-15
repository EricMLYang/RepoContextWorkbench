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
//! 沒有 `claude` 指令時，MCP 那步改印出手動指令，不失敗；真的註冊失敗則 [`StepResult::ok`]＝false，
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

/// 對應 `agentsetup.mcp_add_command`。名稱一定在 `--` 與任何 `-e` 之前：
/// `-e` 吃多值，名稱放後面會被當成環境變數吞掉（2026-09-25 真機 setup 踩到，c231287）。
pub fn mcp_add_command(exe: &Path) -> Vec<String> {
    vec![
        "claude".into(),
        "mcp".into(),
        "add".into(),
        "--scope".into(),
        "user".into(),
        MCP_NAME.into(),
        "--".into(),
        exe.to_string_lossy().into_owned(),
        "mcp".into(),
    ]
}

/// 對應 `agentsetup.claude_home`：`$CLAUDE_CONFIG_DIR` 或 `~/.claude`。
pub fn claude_home() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::fsutil::expanduser("~/.claude"))
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
    let p = settings_path;
    let mut data: Value = if p.exists() {
        match serde_json::from_str::<Value>(&std::fs::read_to_string(p)?) {
            Ok(v @ Value::Object(_)) => v,
            _ => {
                return Ok(StepResult::ok(format!(
                "{} 不是合法 JSON，沒有動它；請手動加 hooks（見 `ctx setup --claude --dry-run`）",
                p.display()
            )))
            }
        }
    } else {
        json!({})
    };
    let root = data
        .as_object_mut()
        .ok_or_else(|| Error::invalid("settings.json 頂層不是物件"))?;
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
        entries.push(json!({"hooks": [{"type": "command", "command": hook_command(exe, arg)}]}));
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
        "已加入 {} → {}",
        added.join(", "),
        p.display()
    )))
}

/// 對應 `agentsetup.install_skill`：寫 `<claude_home>/skills/repo-context/SKILL.md`（內容相同就略過）。
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
    install_mcp_with(exe, which("claude").is_some(), &mut run_cmd)
}

/// `install_mcp` 的本體：`claude_found`＝PATH 上有沒有 claude、`run`＝跑外部指令（測試注入）。
pub fn install_mcp_with(
    exe: &Path,
    claude_found: bool,
    run: &mut dyn FnMut(&[String]) -> std::io::Result<CmdOutput>,
) -> StepResult {
    let add = mcp_add_command(exe);
    if !claude_found {
        return StepResult::ok(format!("找不到 claude 指令；請手動執行：{}", cmdline(&add)));
    }
    let get: Vec<String> = ["claude", "mcp", "get", MCP_NAME]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if run(&get).is_ok_and(|o| o.success) {
        return StepResult::ok(
            "MCP 已註冊，略過（要重設先 `claude mcp remove repo-context -s user`）",
        );
    }
    match run(&add) {
        Ok(o) if o.success => StepResult::ok("已註冊（user scope）"),
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

/// setup 的環境：spine、ctx 執行檔、claude 設定根目錄、使用者設定檔。測試可整組自己給。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupEnv {
    pub spine_dir: PathBuf,
    pub exe: PathBuf,
    pub claude_home: PathBuf,
    pub user_config: PathBuf,
}

impl SetupEnv {
    /// 從行程環境推（`exe::ctx_exe`、`CLAUDE_CONFIG_DIR`、`XDG_CONFIG_HOME`）。
    pub fn detect(spine_dir: impl AsRef<Path>) -> Result<Self> {
        Ok(SetupEnv {
            spine_dir: spine_dir.as_ref().to_path_buf(),
            exe: crate::exe::ctx_exe()?,
            claude_home: claude_home(),
            user_config: config::user_config_path(),
        })
    }
    pub fn settings_path(&self) -> PathBuf {
        self.claude_home.join("settings.json")
    }
}

/// setup 的一步（對應 Python `plan()` 回的 {desc, run}）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    SaveSpine,
    Mcp,
    Hooks,
    Skill,
}

impl Step {
    /// 這步要做什麼（`--dry-run` 印這行）。
    pub fn desc(self, env: &SetupEnv) -> String {
        match self {
            Step::SaveSpine => format!(
                "記住 spine 位置：{}",
                crate::fsutil::resolve(&env.spine_dir).display()
            ),
            Step::Mcp => format!("註冊 MCP server：{}", cmdline(&mcp_add_command(&env.exe))),
            Step::Hooks => format!(
                "掛 hooks（SessionStart／SessionEnd）到 {}：{}",
                env.settings_path().display(),
                hook_command(&env.exe, "session-start")
            ),
            Step::Skill => format!(
                "安裝 skill 到 {}",
                env.claude_home
                    .join("skills")
                    .join(MCP_NAME)
                    .join("SKILL.md")
                    .display()
            ),
        }
    }

    /// 執行這步。
    pub fn run(self, env: &SetupEnv) -> Result<StepResult> {
        match self {
            Step::SaveSpine => save_spine_to(&env.user_config, &env.spine_dir),
            Step::Mcp => Ok(install_mcp(&env.exe)),
            Step::Hooks => install_hooks(&env.settings_path(), &env.exe),
            Step::Skill => install_skill(&env.claude_home),
        }
    }
}

/// 對應 `agentsetup.plan`：記住 spine；`claude` 再加 MCP／hooks／skill。
pub fn plan(claude: bool) -> Vec<Step> {
    let mut steps = vec![Step::SaveSpine];
    if claude {
        steps.extend([Step::Mcp, Step::Hooks, Step::Skill]);
    }
    steps
}
