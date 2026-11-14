//! `ctx relay`：接力（F-122，Rust 才有）——plan 看哪些 repo 能直接從上次交接接著做，report 看事後各 repo 做了什麼。
//! 判斷都在 `ctx_core::relay`；這裡只轉參數、排版。

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use ctx_core::agentapi;
use ctx_core::pystr::clip;
use ctx_core::relay::{self, Answer, Delivery, Plan, RelayScope, RelayStatus, Report, Verdict};

use super::agent::agent_err;
use super::common::{fail, split_opt, CmdResult, Ctx, Exit, EXIT_REJECTED};

/// 人看的表裡「下一步」「完成」最多幾個字（--json 是全文）。
const SHOWN_CHARS: usize = 120;

#[derive(Args, Debug, Clone)]
pub struct RelayScopeArgs {
    /// 組名（省略＝目前目錄所屬的組）
    #[arg(long, conflicts_with = "all")]
    pub group: Option<String>,
    /// 全部登記的 repo（參考類、暫停、封存的不列）
    #[arg(long)]
    pub all: bool,
    /// 用哪個目錄推所屬的組（預設目前目錄）
    #[arg(long)]
    pub cwd: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum RelayCmd {
    /// 接力前：每個 repo 能不能直接從上次交接的下一步接著做（可接／等你／會話開著／先補交接／交接太舊／會話處理中／沒有交接／路徑不存在）
    Plan {
        #[command(flatten)]
        scope: RelayScopeArgs,
    },
    /// 接力後：每個 repo 在 --since 之後的 handoff、commit、沒 commit 的檔、會話狀態
    Report {
        /// 從什麼時候算起："YYYY-MM-DD HH:MM"
        #[arg(long)]
        since: String,
        #[command(flatten)]
        scope: RelayScopeArgs,
        /// 逗號分隔 repo id（指定時照列，不略過參考類）
        #[arg(long, conflicts_with_all = ["group", "all"])]
        repos: Option<String>,
    },
    /// 回覆一個在等你的 repo：組出帶著上次交接與你這句回覆的任務，印出怎麼送（開著的會話／claude --resume／新會話）。只印，不開會話
    Answer {
        /// 登記的 repo id
        repo: String,
        /// 你的回覆（一句話）
        reply: String,
    },
}

/// 參數 → 範圍。沒給＝目前目錄所屬的組（不在組裡但登記過＝只看這個 repo）。
fn scope_of(d: &Path, a: &RelayScopeArgs, repos: Option<&str>) -> Result<RelayScope, Exit> {
    if let Some(ids) = split_opt(repos) {
        return Ok(RelayScope::Repos(ids));
    }
    if a.all {
        return Ok(RelayScope::All);
    }
    if let Some(g) = a.group.as_deref().filter(|g| !g.is_empty()) {
        return Ok(RelayScope::Group(g.to_string()));
    }
    let cwd = a.cwd.as_ref().map(PathBuf::from);
    let w = agentapi::where_am_i(d, cwd.as_deref()).map_err(agent_err)?;
    if let Some(g) = w.group {
        return Ok(RelayScope::Group(g));
    }
    if let Some(r) = w.repo {
        return Ok(RelayScope::Repos(vec![r.id]));
    }
    Err(fail(
        "no_scope",
        format!("{} 不在任何登記的 repo 或組裡", w.cwd),
        Some("用 --group <組> 或 --all 指定範圍".into()),
        EXIT_REJECTED,
    ))
}

pub fn cmd_relay(ctx: &Ctx, cmd: &RelayCmd) -> CmdResult {
    let d = ctx.spine_dir()?;
    match cmd {
        RelayCmd::Plan { scope } => {
            let sc = scope_of(&d, scope, None)?;
            let plan = relay::plan(&d, &sc).map_err(agent_err)?;
            ctx.emit(&plan, render_plan);
        }
        RelayCmd::Report {
            since,
            scope,
            repos,
        } => {
            let sc = scope_of(&d, scope, repos.as_deref())?;
            let rep = relay::report(&d, &sc, since).map_err(agent_err)?;
            ctx.emit(&rep, render_report);
        }
        RelayCmd::Answer { repo, reply } => {
            let a = relay::answer(&d, repo, reply).map_err(agent_err)?;
            ctx.emit(&a, render_answer);
        }
    }
    Ok(())
}

fn render_plan(p: &Plan) -> String {
    let mut out = vec![format!("# 接力 plan：{}（{}）", p.scope, p.at)];
    let counts: Vec<String> = p
        .counts
        .iter()
        .map(|c| format!("{} {}", c.status.label(), c.count))
        .collect();
    let mut head = if counts.is_empty() {
        "（範圍內沒有 repo）".to_string()
    } else {
        counts.join("、")
    };
    if p.skipped > 0 {
        head.push_str(&format!("；略過 {} 個（參考類、暫停、封存）", p.skipped));
    }
    out.push(head);
    out.push(format!("等你回覆的共 {} 件", p.waiting_on_you));
    for st in RelayStatus::ORDER {
        let rows: Vec<_> = p.rows.iter().filter(|r| r.status == st).collect();
        if rows.is_empty() {
            continue;
        }
        out.push(String::new());
        out.push(format!("## {}（{}）", st.label(), rows.len()));
        for r in rows {
            let mut line = format!("- {}", r.repo);
            if let Some(g) = &r.group {
                line.push_str(&format!("〔{g}〕"));
            }
            if let (Some(at), Some(rf)) = (&r.handoff_at, &r.handoff_ref) {
                line.push_str(&format!(" handoff {at}（{rf}）"));
            }
            out.push(line);
            if let Some(n) = &r.next_step {
                out.push(format!("  下一步：{}", clip(n, SHOWN_CHARS)));
            }
            // 可接的任務就是「接續上次交接：<下一步>」，不重印；其他狀態的任務跟下一步不同才印
            if st != RelayStatus::Ready {
                if let Some(pr) = &r.prompt {
                    out.push(format!("  任務：{}", clip(pr, SHOWN_CHARS)));
                }
            }
            if !r.notes.is_empty() {
                out.push(format!("  備註：{}", r.notes.join("；")));
            }
            if r.waits_on_you || st == RelayStatus::SessionOpen {
                out.push(format!("  回覆：ctx relay answer {} \"…\"", r.repo));
            }
        }
    }
    out.push(String::new());
    out.push("（--json 看完整的下一步與要送給 agent 的任務）".into());
    out.join("\n")
}

fn render_report(r: &Report) -> String {
    let mut out = vec![
        format!("# 接力 report：{}（自 {}）", r.scope, r.since),
        r.total.line.clone(),
    ];
    if r.skipped > 0 {
        out.push(format!("（略過 {} 個：參考類、暫停、封存）", r.skipped));
    }
    for v in Verdict::ORDER {
        let rows: Vec<_> = r.rows.iter().filter(|x| x.verdict == v).collect();
        if rows.is_empty() {
            continue;
        }
        out.push(String::new());
        out.push(format!("## {}（{}）", v.label(), rows.len()));
        for x in rows {
            let mut bits = Vec::new();
            if !x.exists {
                bits.push("路徑不存在".to_string());
            }
            if x.commits > 0 {
                let subjects: Vec<&str> = x
                    .recent_commits
                    .iter()
                    .map(|c| c.subject.as_str())
                    .collect();
                let more = if x.commits > subjects.len() {
                    "…"
                } else {
                    ""
                };
                bits.push(format!(
                    "{} 個 commit（{}{more}）",
                    x.commits,
                    subjects.join("；")
                ));
            }
            if x.dirty > 0 {
                bits.push(format!("{} 個檔沒 commit", x.dirty));
            }
            if x.session != relay::SessionNow::None {
                bits.push(format!("會話{}", x.session.label()));
            }
            let mut line = format!("- {}", x.repo);
            if let Some(g) = &x.group {
                line.push_str(&format!("〔{g}〕"));
            }
            if !bits.is_empty() {
                line.push_str(&format!("：{}", bits.join("、")));
            }
            out.push(line);
            for h in &x.handoffs {
                out.push(format!(
                    "  handoff {}：{}",
                    h.at,
                    clip(h.done.as_deref().unwrap_or("（沒寫完成）"), SHOWN_CHARS)
                ));
                if let Some(n) = &h.next_step {
                    out.push(format!("    下次從這裡接：{}", clip(n, SHOWN_CHARS)));
                }
            }
        }
    }
    out.join("\n")
}

fn render_answer(a: &Answer) -> String {
    let mut out = vec![
        format!(
            "# 接力 answer：{}（handoff {}，{}）",
            a.repo, a.handoff_at, a.handoff_ref
        ),
        String::new(),
        "## 任務文字".to_string(),
        a.task.clone(),
        String::new(),
        "## 送達方式".to_string(),
    ];
    match &a.delivery {
        Delivery::ExistingSession { sid, state } => {
            out.push(format!(
                "App 裡開著的會話 {sid}（{}）：在 App 的那場會話貼上上面的任務文字",
                state.label()
            ));
        }
        Delivery::Resume {
            claude_session_id,
            cwd,
        } => {
            out.push(format!(
                "接回留下這筆 handoff 的 Claude Code 會話 {claude_session_id}（目錄 {cwd}）"
            ));
        }
        Delivery::NewSession { cwd } => {
            out.push(format!("開一場新會話（目錄 {cwd}）"));
        }
    }
    if !a.commands.is_empty() {
        out.push(String::new());
        out.push("## 指令（貼到終端機）".to_string());
        out.extend(a.commands.iter().cloned());
    }
    if a.redacted > 0 {
        out.push(String::new());
        out.push(format!("（回覆裡有 {} 處像憑證的字，已遮掉）", a.redacted));
    }
    out.join("\n")
}
