//! `ctx decide`：要你決定（F-123，Rust 才有）——list 列出各 repo 在等你決定的卡，answer 回答一張並印出怎麼送給 agent。
//! 判斷都在 `ctx_core::decide`；這裡只轉參數、排版。文字輸出寫給不記得這個 repo 細節的人看。

use clap::Subcommand;
use ctx_core::decide::{self, Card, Decided, FollowRec, Queue};
use ctx_core::pystr::clip;

use super::agent::agent_err;
use super::common::{CmdResult, Ctx};
use super::relay::{render_answer, scope_of, RelayScopeArgs};

/// 人看的卡上，問題最多幾個字（--json 是全文）。
const QUESTION_CHARS: usize = 300;

#[derive(Subcommand, Debug)]
pub enum DecideCmd {
    /// 列出在等你決定的卡：每個 repo 最後一筆交接還沒回答的（帶結構化提問的在前）
    List {
        #[command(flatten)]
        scope: RelayScopeArgs,
    },
    /// 回答一張卡：選項編號或一句話。留一筆痕，印出怎麼把回答送給那個 repo 的 agent（只印，不開會話）
    Answer {
        /// 登記的 repo id
        repo: String,
        /// 選項編號，或一句話（可以不加引號）
        #[arg(required = true, num_args = 1..)]
        reply: Vec<String>,
    },
}

pub fn cmd_decide(ctx: &Ctx, cmd: &DecideCmd) -> CmdResult {
    let d = ctx.spine_dir()?;
    match cmd {
        DecideCmd::List { scope } => {
            let sc = scope_of(&d, scope, None)?;
            let q = decide::list(&d, &sc).map_err(agent_err)?;
            ctx.emit(&q, render_list);
        }
        DecideCmd::Answer { repo, reply } => {
            let r = decide::answer(&d, repo, &reply.join(" ")).map_err(agent_err)?;
            ctx.emit(&r, render_decided);
        }
    }
    Ok(())
}

fn render_list(q: &Queue) -> String {
    if q.cards.is_empty() {
        return format!("沒有要你決定的事（{}，{}）。", q.scope, q.at);
    }
    let mut out = vec![
        format!("# 要你決定：{}（{}）", q.scope, q.at),
        format!("共 {} 件，{} 件有結構化提問", q.cards.len(), q.structured),
    ];
    for (i, c) in q.cards.iter().enumerate() {
        out.push(String::new());
        out.extend(card_lines(i + 1, c));
    }
    out.join("\n")
}

fn card_lines(n: usize, c: &Card) -> Vec<String> {
    let mut head = format!("{n}. {}", c.repo);
    if let Some(g) = &c.group {
        head.push_str(&format!("〔{g}〕"));
    }
    match (&c.about, &c.role) {
        (Some(a), Some(r)) => head.push_str(&format!("：{a}（{r}）")),
        (Some(a), None) => head.push_str(&format!("：{a}")),
        (None, Some(r)) => head.push_str(&format!("（{r}）　{}", c.path)),
        (None, None) => head.push_str(&format!("　{}", c.path)),
    }
    let mut out = vec![head];
    if c.structured {
        out.push(format!("   問：{}", clip(&c.question, QUESTION_CHARS)));
    } else {
        // 交接沒帶提問：問題就是交接的下一步（接力從文字看出在等你）
        out.push(format!(
            "   問（交接的下一步）：{}",
            clip(&c.question, QUESTION_CHARS)
        ));
    }
    for o in &c.options {
        let mark = if o.recommended { "　← 建議" } else { "" };
        out.push(format!("     {}) {}{mark}", o.n, o.text));
    }
    if c.options.is_empty() {
        if let Some(r) = &c.recommend {
            out.push(format!("   建議：{r}"));
        }
    }
    if let (Some(k), Some(rev)) = (c.kind, c.reversible) {
        out.push(format!(
            "   類別：{}；{}",
            k.label(),
            if rev { "可逆" } else { "做了收不回來" }
        ));
    }
    if !c.evidence.is_empty() {
        let ev: Vec<String> = c
            .evidence
            .iter()
            .map(|e| {
                if e.exists {
                    e.reference.clone()
                } else {
                    format!("{}（不存在）", e.reference)
                }
            })
            .collect();
        out.push(format!("   證據：{}", ev.join("、")));
    }
    let age = if c.days > 0 {
        format!("放了 {} 天", c.days)
    } else if c.hours > 0 {
        format!("放了 {} 小時", c.hours)
    } else {
        "剛交接".to_string()
    };
    // 接力的狀態（交接太舊、會話開著…）在備註裡已經講了，不另印標籤
    let mut when = format!("   {age}（handoff {}）", c.handoff_at);
    if !c.notes.is_empty() {
        when.push_str(&format!("；{}", c.notes.join("；")));
    }
    out.push(when);
    let reply = c
        .options
        .iter()
        .find(|o| o.recommended)
        .map(|o| o.n.to_string())
        .unwrap_or_else(|| "\"…\"".to_string());
    out.push(format!(
        "   → ctx decide answer {} {reply}",
        ctx_core::term::shlex_quote(&c.repo)
    ));
    out
}

fn render_decided(r: &Decided) -> String {
    let a = &r.answered;
    let mut how = Vec::new();
    if let Some(n) = r.option {
        how.push(format!("第 {n} 個選項"));
    }
    if a.follow != FollowRec::NoRecommendation {
        how.push(a.follow.label().to_string());
    }
    let how = if how.is_empty() {
        String::new()
    } else {
        format!("（{}）", how.join("，"))
    };
    let mut out = vec![
        format!(
            "已記下：{}「{}」→ {}{how}　{}",
            a.repo,
            clip(&a.question, 80),
            a.answer,
            a.reference
        ),
        String::new(),
    ];
    out.push(render_answer(&r.relay));
    out.join("\n")
}
