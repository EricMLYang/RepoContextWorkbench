//! 安裝與掛載：Claude Code hook 入口、給 agent 的使用說明、ctx setup。對應 `repo_context/cli/setup.py`。

use std::io::Read;

use clap::{Args, ValueEnum};
use ctx_core::agentsetup::{self, SetupEnv};
use ctx_core::session_status::StatusHook;
use ctx_core::{config, fsutil, hooks};

use super::common::{fail, CmdResult, Ctx, Exit, EXIT_FINDINGS, EXIT_NO_SPINE};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum HookEvent {
    SessionStart,
    SessionEnd,
    /// Rust 才有：App 開的會話這一輪做完（Claude Code `Stop`）→ 會話狀態「等你」
    Stop,
    /// Rust 才有：要你允許工具或閒著等輸入（Claude Code `Notification`）→「等你」＋訊息
    Notification,
    /// Rust 才有：你送出訊息（Claude Code `UserPromptSubmit`）→「處理中」
    Prompt,
    /// Rust 才有：要你允許工具或回答（Notification 的 permission_prompt／elicitation_*／agent_needs_input）→「卡住了」
    Permission,
    /// Rust 才有（F-81）：要你允許哪個工具、做什麼（Claude Code `PermissionRequest`）→「卡住了：Bash：cargo test」；只記錄、不回決定
    PermissionRequest,
    /// Rust 才有（F-83）：Claude Code 的 statusLine 指令——把 rate_limits（5h／7d 額度）寫到 .state/usage.json；不印東西
    Statusline,
}

impl HookEvent {
    /// 開場／收尾 hook（`hooks::run_hook` 認的名字）。
    fn as_str(self) -> &'static str {
        match self {
            HookEvent::SessionStart => "session-start",
            HookEvent::SessionEnd => "session-end",
            other => other.status_hook().map(StatusHook::arg).unwrap_or_default(),
        }
    }

    /// App 會話的狀態／額度 hook（引擎的 [`StatusHook`]，字串在那邊）；開場／收尾 hook 回 None。
    fn status_hook(self) -> Option<StatusHook> {
        Some(match self {
            HookEvent::SessionStart | HookEvent::SessionEnd => return None,
            HookEvent::Stop => StatusHook::Stop,
            HookEvent::Notification => StatusHook::Notification,
            HookEvent::Prompt => StatusHook::Prompt,
            HookEvent::Permission => StatusHook::Permission,
            HookEvent::PermissionRequest => StatusHook::PermissionRequest,
            HookEvent::Statusline => StatusHook::Statusline,
        })
    }
}

/// Claude Code hook 入口：stdin 是 hook JSON。永遠 exit 0——hook 壞掉不能擋住開 agent。
pub fn cmd_hook(ctx: &Ctx, event: HookEvent) -> CmdResult {
    let mut buf = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut buf);
    let stdin = String::from_utf8_lossy(&buf);
    if let Some(h) = event.status_hook() {
        // App 開的會話才有：照環境變數給的路徑寫，不找 spine、不印任何東西（不是 App 開的會話就什麼都不做）
        let target = std::env::var(ctx_core::session_status::ENV).ok();
        match h {
            // 額度餘光（F-83）
            StatusHook::Statusline => {
                ctx_core::usage::record(&stdin, target.as_deref(), None);
            }
            // 會話狀態（做完／等你／處理中／要你允許）
            h => {
                ctx_core::session_status::record_hook(h, &stdin, target.as_deref(), None);
            }
        }
        return Ok(());
    }
    let Some(d) = config::find_spine(ctx.spine.as_deref()) else {
        return Ok(());
    };
    let out = hooks::run_hook(&d, event.as_str(), &stdin);
    if !out.is_empty() {
        println!("{out}");
    }
    Ok(())
}

/// 給 agent 的使用說明（skill 本體）——可貼進 AGENTS.md／GEMINI.md 給沒有 skill 機制的 agent。
pub fn cmd_instructions() -> CmdResult {
    println!("{}", agentsetup::skill_text(false));
    Ok(())
}

#[derive(Args, Debug)]
pub struct SetupArgs {
    /// spine repo 路徑
    #[arg(long = "spine")]
    pub spine_path: Option<String>,
    /// 註冊 Claude Code：MCP server、SessionStart/SessionEnd hooks、skill
    #[arg(long)]
    pub claude: bool,
    /// 只列出會做什麼
    #[arg(long)]
    pub dry_run: bool,
}

pub fn cmd_setup(ctx: &Ctx, a: &SetupArgs) -> CmdResult {
    let spine_arg = a
        .spine_path
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(ctx.spine.as_deref().filter(|s| !s.is_empty()));
    let d = match spine_arg {
        Some(s) => {
            let d = fsutil::resolve(&fsutil::expanduser(s));
            if !config::is_spine(&d) {
                return Err(fail(
                    "no_spine",
                    format!(
                        "{} 不是 spine repo（先跑 `ctx init {}`）",
                        d.display(),
                        d.display()
                    ),
                    None,
                    EXIT_NO_SPINE,
                ));
            }
            d
        }
        None => ctx.spine_dir()?,
    };
    let env = SetupEnv::detect(&d)?;
    let mut failed = false;
    for st in agentsetup::plan(a.claude) {
        println!(
            "{}{}",
            if a.dry_run { "（試跑）" } else { "" },
            st.desc(&env)
        );
        if !a.dry_run {
            match st.run(&env) {
                Ok(r) => {
                    if !r.message.is_empty() {
                        println!("  → {}", r.message);
                    }
                    failed = failed || !r.ok;
                }
                Err(e) => {
                    println!("  → 失敗：{e}");
                    failed = true;
                }
            }
        }
    }
    if a.dry_run {
        println!("加上不帶 --dry-run 再跑一次才會真的寫入。");
    }
    if failed {
        // 有一步沒成就不能回 0：呼叫的人（或 agent）會以為掛好了
        return Err(Exit::Code(EXIT_FINDINGS));
    }
    Ok(())
}
