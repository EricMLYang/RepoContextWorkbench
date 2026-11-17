//! `ctx home sync [--check]`（F-132，Rust 才有）：spine 的靜態入口——由 registry 生成 `AGENTS.md`、`CLAUDE.md`、
//! `groups/<組>/REPOS.md`。邏輯在 `ctx_core::home`，這裡只轉參數與輸出。
//!
//! exit code：0＝都是最新（或寫完了）｜1＝`--check` 有過期的檔，或有手寫檔擋住（衝突）。

use clap::Subcommand;
use ctx_core::home::{self, Outcome, Pending};
use serde_json::json;

use super::common::{CmdResult, Ctx, Exit, EXIT_FINDINGS};

#[derive(Subcommand, Debug)]
pub enum HomeCmd {
    /// 照 registry 重寫 spine 的 AGENTS.md、CLAUDE.md、groups/<組>/REPOS.md（手寫的同名檔不覆蓋；Rust 才有）
    Sync {
        /// 不寫，只報告哪些檔過期或衝突（有就 exit 1）
        #[arg(long)]
        check: bool,
    },
}

pub fn cmd_home(ctx: &Ctx, cmd: &HomeCmd) -> CmdResult {
    let d = ctx.spine_dir()?;
    match cmd {
        HomeCmd::Sync { check: true } => {
            let c = home::check(&d)?;
            let stale = c.stale();
            let conflicts = c.conflicts();
            let up_to_date = stale.is_empty() && conflicts.is_empty();
            let out = json!({
                "ok": true, "check": true, "up_to_date": up_to_date,
                "stale": stale, "conflicts": conflicts, "files": c.files,
            });
            ctx.emit(&out, |_| {
                let mut lines: Vec<String> = c
                    .files
                    .iter()
                    .map(|f| format!("{}  {}", check_label(f.status), f.path))
                    .collect();
                lines.push(if up_to_date {
                    "spine 入口檔都是最新的。".into()
                } else if stale.is_empty() {
                    "有手寫的同名檔擋住（不會覆蓋）：要改用生成的就先把它移走，再跑 `ctx home sync`。".into()
                } else {
                    "跑 `ctx home sync` 更新。".into()
                });
                lines.join("\n")
            });
            if up_to_date {
                Ok(())
            } else {
                Err(Exit::Code(EXIT_FINDINGS))
            }
        }
        HomeCmd::Sync { check: false } => {
            let s = home::sync(&d)?;
            let conflicts = s.conflicts();
            let out = json!({"ok": true, "conflicts": conflicts, "files": s.files});
            ctx.emit(&out, |_| {
                let mut lines: Vec<String> = s
                    .files
                    .iter()
                    .map(|f| format!("{}  {}", sync_label(f.status), f.path))
                    .collect();
                if !conflicts.is_empty() {
                    lines.push(
                        "衝突＝同名檔是手寫的（第一行沒有生成標記），沒有覆蓋：要改用生成的就先把它移走再跑一次。"
                            .into(),
                    );
                }
                lines.join("\n")
            });
            if conflicts.is_empty() {
                Ok(())
            } else {
                Err(Exit::Code(EXIT_FINDINGS))
            }
        }
    }
}

fn check_label(p: Pending) -> &'static str {
    match p {
        Pending::Create => "過期（會新建）",
        Pending::Update => "過期（會更新）",
        Pending::Remove => "過期（會刪掉）",
        Pending::Unchanged => "最新",
        Pending::Conflict => "衝突（手寫檔）",
    }
}

fn sync_label(o: Outcome) -> &'static str {
    match o {
        Outcome::Written => "寫了",
        Outcome::Unchanged => "沒變",
        Outcome::Removed => "刪了",
        Outcome::Conflict => "衝突（手寫檔，沒覆蓋）",
    }
}
