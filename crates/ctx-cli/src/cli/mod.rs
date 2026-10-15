//! CLI——人的介面（同一組原語也經 MCP server 給 agent）。對應 `repo_context/cli/__init__.py`。
//!
//! 依領域拆模組（同 Python 2026-09-25 架構檢查）：
//!     common（spine 定位／錯誤出口／--json）、admin（init／registry／組，寫入走 ops）、
//!     daily（日常原語）、apps（會話／MCP／工作台）、agent（agentapi 入口）、setup（hook／安裝）。
//! 新增子命令：放進對應模組，再在 [`Cmd`] 加一項。要跨入口共用的管理寫入，先在 `ctx_core::ops` 加操作。
//! exit code 約定見 common.rs。

pub mod admin;
pub mod agent;
pub mod apps;
pub mod common;
pub mod daily;
pub mod setup;

use clap::{Parser, Subcommand};

use common::{finish, Ctx};

#[derive(Parser, Debug)]
#[command(
    name = "ctx",
    about = "repo-context 工作台：把一群 repo 分成組，每組組成一份 context",
    version,
    infer_long_args = true
)]
pub struct Cli {
    /// spine repo 路徑（預設 $REPOENGINE_SPINE → ctx setup 記住的 → cwd）
    #[arg(long)]
    pub spine: Option<String>,
    /// 結構化輸出（agent 用；錯誤也是 JSON，exit code 見 cli/common.rs）
    #[arg(long)]
    pub json: bool,
    #[command(subcommand)]
    pub cmd: Cmd,
}

/// 子命令（順序＝`ctx --help` 列出的順序，同 Python：admin、daily、apps、agent、setup）。
#[derive(Subcommand, Debug)]
pub enum Cmd {
    // ── admin ──
    /// 建 spine repo scaffold
    Init(admin::InitArgs),
    /// P1 登記/掃描/稽核/tag/關係/export
    Registry {
        #[command(subcommand)]
        action: admin::RegistryCmd,
    },
    /// P2 組（add／add-member／remove-member／rename／remove；context＝組情境卡；summary＝工作摘要；goal＝一句話目標）
    Group {
        #[command(subcommand)]
        action: admin::GroupCmd,
    },
    // ── daily ──
    /// 活動脈動：7d/30d commit 數＋近期 commit 主旨（組的監控焦點）
    Pulse(daily::PulseArgs),
    /// P3 採集
    Collect(daily::ScopeArgs),
    /// P6 打包
    Pack(daily::PackArgs),
    /// 早晨簡報（樣貌A）
    Brief(daily::ScopeArgs),
    /// P10 寫事件
    Append(daily::AppendArgs),
    /// P11 查詢
    Query(daily::QueryArgs),
    /// 未結 open loops（P11 視圖；可依組過濾）
    Loops(daily::ScopeArgs),
    /// 脊椎衛生迴圈（ref 斷鏈/逾期 loop/碰撞無回程/dead-letter）
    Lint,
    /// P7 撞（兩段式）
    Collide(daily::CollideArgs),
    /// P8 落
    Route(daily::RouteArgs),
    /// 未讀事件（殼 poll 的同一來源）
    Unread {
        #[arg(long)]
        ack: bool,
    },
    /// git 單一提交者（批次 commit）
    Commit {
        #[arg(short, long, default_value = "spine: batch commit")]
        message: String,
    },
    /// P4 查上游 release/commit
    Upstream(daily::ScopeArgs),
    /// P5 增量 digest（便宜模型）
    Digest(daily::DigestArgs),
    /// P9 生（升格＋出生登記＋血統）
    Spawn(daily::SpawnArgs),
    /// P12 排程 tick（S6 引信；schedule 見 config.yaml）
    Timer(daily::TimerArgs),
    /// P13 通知內容（interrupt 白名單優先）
    Notify,
    // ── apps ──
    /// P16 帶料開 agent 終端視窗（含 MCP 掛載）
    Session(apps::SessionArgs),
    /// MCP server（stdio）；預設只開工作層工具，--admin 另開維護工具
    Mcp {
        /// 另開管理層工具（registry／組／打包／碰撞／升格）
        #[arg(long)]
        admin: bool,
    },
    /// 工作台桌面 App（Tauri；IDE 風格＋內嵌 terminal）
    App(apps::AppArgs),
    /// （已停用）瀏覽器模式——請改用 `ctx app`
    Ui(apps::UiArgs),
    // ── agent ──
    /// 我在哪：cwd 屬於哪個 repo／組（agent 開場用）
    Where {
        #[arg(long)]
        cwd: Option<String>,
    },
    /// 主場的完整脈絡（--json：目標／交接／下一步／未結／事件）
    Context(agent::Scoped),
    /// 接下來值得做什麼（每項附依據）
    Next {
        #[command(flatten)]
        scope: agent::Scoped,
        #[arg(long, default_value_t = 5)]
        limit: i64,
    },
    /// 過去的 agent 會話：範圍內 repo 的 Claude Code 互動會話（標題、最後回覆、接續指令；Rust 才有）
    Sessions {
        #[command(flatten)]
        scope: agent::Scoped,
        #[arg(long, default_value_t = 10)]
        limit: i64,
        /// 只列標題／第一句／最後回覆／repo／分支含這些詞的（空白分開，都要有）
        #[arg(long)]
        query: Option<String>,
    },
    /// 記一筆判斷（decision），回傳可引用的 ref
    Log(agent::LogArgs),
    /// 未結事項：add <文字> [--due] ／ close <#N> [--note] ／ review（體檢）／ brief <#N>（任務包）
    Todo(agent::TodoArgs),
    /// 回報 agent 階段：working／waiting／blocked／done
    Status(agent::StatusArgs),
    /// 收尾交接：--done 完成了什麼 --next 下次從哪接 [--remaining 新事項 ...]
    Handoff(agent::HandoffArgs),
    /// 知識 ↔ repo 相關度：這段文字跟哪些 repo 的哪些檔最相關
    Search(agent::SearchArgs),
    /// 反芻推薦：別組新進的知識／判斷中，跟主場手上工作強相關的
    Fresh(agent::FreshArgs),
    /// 判斷匯出：寫成 AgDR 格式的 md 到指定目錄（不掛 MCP 的 agent 也讀得到；Rust 才有）
    Decisions {
        #[command(subcommand)]
        cmd: agent::DecisionsCmd,
    },
    /// 助手：一句話改工作台設定（建組、組員、標籤、分層、目標）；先印計畫、確認才做（Rust 才有）
    Assist(agent::AssistArgs),
    /// 健康指標：交接率、推薦命中率、判斷結果記錄率、未結、工具用量（Rust 才有）
    Health {
        /// 看近幾天
        #[arg(long, default_value_t = 14)]
        days: i64,
    },
    /// 回填：從過去的 Claude Code 會話撈判斷／待辦／交接（scan／run／review／accept）
    Backfill(agent::BackfillArgs),
    /// 名字 → 路徑：<repo>、<repo>:<export>[/子路徑]、<repo>:<相對路徑>
    Resolve {
        #[arg(value_name = "REF")]
        reference: String,
    },
    /// 讀別的 repo 的檔案或目錄（附 commit；會記一筆引用）
    Read(agent::ReadArgs),
    /// 在另一個 repo 開唯讀 agent 回答問題（慢、花錢；回結論＋引用）
    Ask(agent::AskArgs),
    /// repo 狀態卡：上次看過後的變化、沒 commit 的檔、正在跑的程序、交接、失效路徑
    Repo(agent::RepoArgs),
    /// 跨 repo 參考：check（失效路徑＋上游變動＋建議 export）／drift／suggest／cite
    Refs(agent::RefsArgs),
    // ── setup ──
    /// Claude Code hook 入口（stdin 讀 hook JSON；ctx setup --claude 會掛好）
    Hook { event: setup::HookEvent },
    /// 印出給 agent 的使用說明（可貼進 AGENTS.md）
    Instructions,
    /// 記住 spine 位置；--claude 另外掛 MCP（user scope）＋hooks＋skill
    Setup(setup::SetupArgs),
}

/// 跑一個已解析的子命令。
pub fn dispatch(ctx: &Ctx, cmd: &Cmd) -> common::CmdResult {
    use Cmd::*;
    match cmd {
        Init(a) => admin::cmd_init(a),
        Registry { action } => admin::cmd_registry(ctx, action),
        Group { action } => admin::cmd_group(ctx, action),
        Pulse(a) => daily::cmd_pulse(ctx, a),
        Collect(a) => daily::cmd_collect(ctx, a),
        Pack(a) => daily::cmd_pack(ctx, a),
        Brief(a) => daily::cmd_brief(ctx, a),
        Append(a) => daily::cmd_append(ctx, a),
        Query(a) => daily::cmd_query(ctx, a),
        Loops(a) => daily::cmd_loops(ctx, a),
        Lint => daily::cmd_lint(ctx),
        Collide(a) => daily::cmd_collide(ctx, a),
        Route(a) => daily::cmd_route(ctx, a),
        Unread { ack } => daily::cmd_unread(ctx, *ack),
        Commit { message } => daily::cmd_commit(ctx, message),
        Upstream(a) => daily::cmd_upstream(ctx, a),
        Digest(a) => daily::cmd_digest(ctx, a),
        Spawn(a) => daily::cmd_spawn(ctx, a),
        Timer(a) => daily::cmd_timer(ctx, a),
        Notify => daily::cmd_notify(ctx),
        Session(a) => apps::cmd_session(ctx, a),
        Mcp { admin } => apps::cmd_mcp(ctx, *admin),
        App(a) => apps::cmd_app(ctx, a),
        Ui(a) => apps::cmd_ui(ctx, a),
        Where { cwd } => agent::cmd_where(ctx, cwd),
        Context(s) => agent::cmd_context(ctx, s),
        Next { scope, limit } => agent::cmd_next(ctx, scope, *limit),
        Sessions {
            scope,
            limit,
            query,
        } => agent::cmd_sessions(ctx, scope, *limit, query.as_deref()),
        Log(a) => agent::cmd_log(ctx, a),
        Todo(a) => agent::cmd_todo(ctx, a),
        Status(a) => agent::cmd_status(ctx, a),
        Handoff(a) => agent::cmd_handoff(ctx, a),
        Search(a) => agent::cmd_search(ctx, a),
        Fresh(a) => agent::cmd_fresh(ctx, a),
        Health { days } => agent::cmd_health(ctx, *days),
        Assist(a) => agent::cmd_assist(ctx, a),
        Decisions { cmd } => agent::cmd_decisions(ctx, cmd),
        Backfill(a) => agent::cmd_backfill(ctx, a),
        Resolve { reference } => agent::cmd_resolve(ctx, reference),
        Read(a) => agent::cmd_read(ctx, a),
        Ask(a) => agent::cmd_ask(ctx, a),
        Repo(a) => agent::cmd_repo(ctx, a),
        Refs(a) => agent::cmd_refs(ctx, a),
        Hook { event } => setup::cmd_hook(ctx, *event),
        Instructions => setup::cmd_instructions(),
        Setup(a) => setup::cmd_setup(ctx, a),
    }
}

/// 對應 `cli.main`：解析 argv、跑子命令，回傳 exit code。
/// `--json` 放在子命令前後都認（agent 自然會寫 `ctx where --json`）——先從 argv 抽掉再解析。
pub fn run<I, S>(argv: I) -> i32
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString>,
{
    let mut args: Vec<std::ffi::OsString> = argv.into_iter().map(Into::into).collect();
    let prog = if args.is_empty() {
        "ctx".into()
    } else {
        args.remove(0)
    };
    let as_json = args.iter().any(|a| a == "--json");
    args.retain(|a| a != "--json");
    let cli = match Cli::try_parse_from(std::iter::once(prog).chain(args)) {
        Ok(c) => c,
        Err(e) => {
            // 用法錯誤＝exit 2（同 argparse）；--help／--version＝0
            let _ = e.print();
            return e.exit_code();
        }
    };
    let ctx = Ctx {
        spine: cli.spine.clone(),
        json: as_json || cli.json,
    };
    let r = dispatch(&ctx, &cli.cmd);
    finish(&ctx, r)
}

pub use common::Exit;
