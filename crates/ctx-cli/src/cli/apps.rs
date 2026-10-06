//! 開窗：agent 會話、MCP server、工作台（桌面 App）。對應 `repo_context/cli/apps.py`。
//!
//! 與 Python 的刻意差異：
//! - `ctx app` 不再起 pywebview＋本機 HTTP，改成啟動 Tauri 桌面 App（`Repo Context.app`）。
//! - `ctx ui`（瀏覽器模式）不搬：印一行說明改用 `ctx app`，exit 2。

use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Args;
use ctx_core::session::{self, SessionOpts};

use super::common::{split_opt, CmdResult, Ctx, Exit, EXIT_REJECTED};

#[derive(Args, Debug)]
pub struct SessionArgs {
    #[arg(long)]
    pub group: Option<String>,
    #[arg(long)]
    pub repos: Option<String>,
    /// 會話 cwd 設為此 repo（改用 --mcp-config 掛載）
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long)]
    pub task: Option<String>,
    /// 開哪家 agent CLI（config agents: 註冊表；預設 claude）
    #[arg(long, default_value = "claude")]
    pub agent: String,
    /// 只印指令不開視窗
    #[arg(long = "print")]
    pub print_only: bool,
}

pub fn cmd_session(ctx: &Ctx, a: &SessionArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    let ids = split_opt(a.repos.as_deref());
    let plan = session::build(
        &d,
        SessionOpts {
            group: a.group.as_deref(),
            repos: ids.as_deref(),
            repo: a.repo.as_deref().filter(|s| !s.is_empty()),
            task: a.task.as_deref().filter(|s| !s.is_empty()),
            agent: Some(a.agent.as_str()).filter(|s| !s.is_empty()),
        },
    )?;
    println!("料：{}", plan.pack_path.display());
    if a.print_only {
        println!("cd {}", plan.cwd.display());
        println!("{}", session::list2cmdline(&plan.cmd));
        return Ok(());
    }
    session::open_terminal(&plan.cwd, &plan.cmd);
    Ok(())
}

pub fn cmd_mcp(ctx: &Ctx, admin: bool) -> CmdResult {
    let d = ctx.spine_dir()?;
    // --admin 是 store_true 但預設 None：沒給＝看 config `mcp.admin`
    crate::mcp::serve(&d, if admin { Some(true) } else { None })?;
    Ok(())
}

#[derive(Args, Debug)]
pub struct AppArgs {
    /// 相容 Python 版的旗標（桌面 App 不開 HTTP port，忽略）
    #[arg(long, default_value_t = 0)]
    pub port: i64,
}

const APP_NAME: &str = "Repo Context.app";

/// 找桌面 App：ctx 同目錄（含 `.app` 包內）→ /Applications → ~/Applications →
/// workspace 的 `target/{release,debug}/bundle/macos/`。
pub fn find_app() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        let exe = exe.canonicalize().unwrap_or(exe);
        if let Some(dir) = exe.parent() {
            cands.push(dir.join(APP_NAME));
            // ctx 被包進 .app（Contents/MacOS/ctx）時，往上找到 .app 本身
            for anc in dir.ancestors() {
                if anc.extension().is_some_and(|e| e == "app") {
                    cands.push(anc.to_path_buf());
                    break;
                }
            }
            // target/{debug,release}/ctx → target/{release,debug}/bundle/macos/
            if let Some(target) = dir.parent() {
                for prof in ["release", "debug"] {
                    cands.push(target.join(prof).join("bundle/macos").join(APP_NAME));
                }
            }
        }
    }
    cands.push(Path::new("/Applications").join(APP_NAME));
    if let Some(home) = std::env::var_os("HOME") {
        cands.push(PathBuf::from(home).join("Applications").join(APP_NAME));
    }
    // 從 repo 根目錄跑 `ctx app`（cargo run）時
    for prof in ["release", "debug"] {
        cands.push(
            PathBuf::from("target")
                .join(prof)
                .join("bundle/macos")
                .join(APP_NAME),
        );
    }
    cands.into_iter().find(|p| p.is_dir())
}

/// `ctx app`：啟動桌面 App（Tauri）。spine 找得到就用 `--spine` 帶過去。
pub fn cmd_app(ctx: &Ctx, _a: &AppArgs) -> CmdResult {
    // 明確給了錯的 --spine 照樣報錯；沒給又找不到＝讓 App 自己問
    let spine = match ctx.spine_dir() {
        Ok(d) => Some(d),
        Err(e) if ctx.spine.is_some() => return Err(e),
        Err(_) => None,
    };
    if !cfg!(target_os = "macos") {
        // Windows（VDI）版的打包與啟動方式還沒做；未測
        return Err(Exit::Msg(
            "錯誤：`ctx app` 目前只支援 macOS（Repo Context.app）".into(),
        ));
    }
    let Some(app) = find_app() else {
        return Err(Exit::Msg(format!(
            "錯誤：找不到桌面 App（{APP_NAME}）。\n\
             找過：ctx 同目錄、/Applications、~/Applications、target/{{release,debug}}/bundle/macos/\n\
             提示：在 repo 根目錄跑 `cd desktop && npm install && npm run bundle`，\
             產物在 target/release/bundle/macos/{APP_NAME}（拖進 /Applications 即可）"
        )));
    };
    let mut cmd = Command::new("open");
    cmd.arg(&app);
    if let Some(d) = &spine {
        cmd.arg("--args").arg("--spine").arg(d);
    }
    let st = cmd.status()?;
    if !st.success() {
        return Err(Exit::Msg(format!("錯誤：無法啟動 {}", app.display())));
    }
    println!("已開啟 {}", app.display());
    Ok(())
}

#[derive(Args, Debug)]
pub struct UiArgs {
    #[arg(long, default_value_t = 8765)]
    pub port: i64,
    #[arg(long)]
    pub no_browser: bool,
}

/// `ctx ui`：瀏覽器模式不搬（Rust 版沒有本機 HTTP 工作台），請改用 `ctx app`。
pub fn cmd_ui(_ctx: &Ctx, _a: &UiArgs) -> CmdResult {
    eprintln!("ctx ui（瀏覽器模式）已停用：請改用 `ctx app` 開桌面工作台");
    Err(Exit::Code(EXIT_REJECTED))
}
