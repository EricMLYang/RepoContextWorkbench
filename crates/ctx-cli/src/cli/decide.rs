//! `ctx decide`：要你決定（F-123／F-124，Rust 才有）——list 列出各 repo 在等你決定的卡，answer 回答一張並印出怎麼送給 agent；
//! request 是 agent 做到一半先問放行規則；trust／untrust／overturn／rules 是人管放行規則。
//! 判斷都在 `ctx_core::decide`／`ctx_core::policy`；這裡只轉參數、排版。文字輸出寫給不記得這個 repo 細節的人看。

use clap::Subcommand;
use ctx_core::decide::{
    self, AutoList, Card, Decided, FollowRec, LevelChanged, MyAnswers, Overturned, PastAnswers,
    Queue, RulesReport, TrustHint, Undo,
};
use ctx_core::policy::{Decision, Level};
use ctx_core::pystr::clip;

use super::agent::{agent_err, HandoffAskArgs, Scoped, WriteOpts};
use super::common::{fail, split_opt, CmdResult, Ctx, Exit, EXIT_REJECTED};
use super::relay::{render_answer, scope_of, RelayScopeArgs};
use ctx_core::RejectCode;

/// 卡片幾張以上時頁尾提示可以讓總管先整理（F-125）。
const STEWARD_HINT_CARDS: usize = 5;

/// 人看的卡上，問題最多幾個字（--json 是全文）。
const QUESTION_CHARS: usize = 300;

#[derive(Subcommand, Debug)]
pub enum DecideCmd {
    /// 列出在等你決定的卡：每個 repo 最後一筆交接還沒回答的（帶結構化提問的在前）
    List {
        #[command(flatten)]
        scope: RelayScopeArgs,
        /// 改列最近的自動放行（全部 repo；每筆附推翻指令）
        #[arg(long = "auto", conflicts_with = "group")]
        auto: bool,
        /// 改列你最近的回答（近 7 天、最多 10 筆；每筆附撤回指令）
        #[arg(long = "mine", conflicts_with_all = ["group", "auto"])]
        mine: bool,
        /// 跟 --auto 一起用：從什麼時候算起（"YYYY-MM-DD HH:MM"；預設近 7 天）
        #[arg(long, requires = "auto")]
        since: Option<String>,
    },
    /// 回答一張卡：選項編號或一句話。留一筆痕，印出怎麼把回答送給那個 repo 的 agent（只印，不開會話）
    Answer {
        /// 登記的 repo id
        repo: String,
        /// 選項編號，或一句話（可以不加引號）
        #[arg(required = true, num_args = 1..)]
        reply: Vec<String>,
    },
    /// agent 做到一半要人決定時先問（沒掛 MCP 時用；同 MCP request_decision）：放行規則允許就回答案，否則回 wait 與原因
    Request {
        #[command(flatten)]
        ask: HandoffAskArgs,
        #[command(flatten)]
        scope: Scoped,
        #[command(flatten)]
        w: WriteOpts,
    },
    /// 以後這類照建議自動放行（commit、content_review、config_change、choose 才行）
    Trust {
        repo: String,
        kind: String,
        /// 只在沒 commit 的檔全落在這些前綴底下時放行（逗號分隔，例 docs/,notes/）
        #[arg(long)]
        paths: Option<String>,
    },
    /// 降回每次都問
    Untrust { repo: String, kind: String },
    /// 推翻一筆自動放行（留痕、規則降回每次都問）；給人的回答的 ref＝撤回那筆回答（規則不動）。那筆交接回到佇列
    Overturn {
        /// 放行或回答的 ref（ctx decide list --auto／--mine 看得到）
        #[arg(value_name = "REF")]
        reference: String,
        /// 為什麼推翻（一句話）
        #[arg(long)]
        why: Option<String>,
    },
    /// 放行規則一覽：等級、since、paths、放行與推翻次數、升級建議、policy.yaml 的問題
    Rules,
    /// 在一張卡上留「總管建議」（選項編號或一句話）；使用者回答時看得到。不是回答，agent（總管）可以下
    Suggest {
        /// 登記的 repo id
        repo: String,
        /// 建議：選項編號，或一句話（可以不加引號）
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        /// 為什麼這樣建議（一句話）
        #[arg(long)]
        why: Option<String>,
    },
}

/// Claude Code 在它的 Bash 工具裡設的環境變數（值是 `1`）。
const AGENT_SHELL_ENV: &str = "CLAUDECODE";

/// 使用者本人才做的子命令（放寬規則、替使用者回答）：在 agent 的 shell 裡跑就拒絕。
///
/// 只擋「順手就做」——agent 照著輸出裡的 `ctx decide trust …` 就複製貼上、或自己幫使用者答卡；不是防惡意：
/// agent 硬要 unset 環境變數擋不住，但做了就有留痕（policy.yaml 跟著 git、回答進 spine）。沒有繞過的旗標。
/// 只放在 CLI：App 的 IPC 直接呼叫引擎，那是人在按。
fn refuse_agent_shell(what: &str) -> Result<(), Exit> {
    if std::env::var_os(AGENT_SHELL_ENV).is_some_and(|v| !v.is_empty() && v != "0") {
        return Err(fail(
            RejectCode::HumanOnly.as_str(),
            format!("{what}是使用者本人的決定，agent 不能代做：這要使用者自己在終端或 App 下"),
            Some(
                "agent 要問使用者就用 ctx decide request（或 MCP request_decision），沒放行就收尾 handoff 帶同一個提問、等使用者回答"
                    .into(),
            ),
            EXIT_REJECTED,
        ));
    }
    Ok(())
}

pub fn cmd_decide(ctx: &Ctx, cmd: &DecideCmd) -> CmdResult {
    match cmd {
        DecideCmd::Answer { .. } => refuse_agent_shell("回答決策卡")?,
        DecideCmd::Trust { .. } => refuse_agent_shell("把一類提問改成自動放行")?,
        _ => {}
    }
    let d = ctx.spine_dir()?;
    match cmd {
        DecideCmd::List { mine: true, .. } => {
            let l =
                decide::my_answers(&d, chrono::Local::now().naive_local()).map_err(agent_err)?;
            ctx.emit(&l, render_mine);
        }
        DecideCmd::List {
            auto: true, since, ..
        } => {
            let l = decide::auto_list(&d, since.as_deref()).map_err(agent_err)?;
            ctx.emit(&l, render_auto);
        }
        DecideCmd::List { scope, .. } => {
            let sc = scope_of(&d, scope, None)?;
            let q = decide::list(&d, &sc).map_err(agent_err)?;
            ctx.emit(&q, render_list);
        }
        DecideCmd::Answer { repo, reply } => {
            let r = decide::answer(&d, repo, &reply.join(" ")).map_err(agent_err)?;
            ctx.emit(&r, render_decided);
        }
        DecideCmd::Request { ask, scope, w } => {
            let Some(input) = ask.input() else {
                return Err(fail(
                    "bad_request",
                    "要用 --ask 給問題",
                    Some("例：ctx decide request --ask \"要 commit 嗎？\" --kind commit --option 要 --option 不要 --recommend 1".into()),
                    EXIT_REJECTED,
                ));
            };
            let r = decide::request(&d, &scope.target(), &input, w.cross_scope_ok)
                .map_err(agent_err)?;
            ctx.emit(&r, render_request);
        }
        DecideCmd::Trust { repo, kind, paths } => {
            let paths = paths
                .as_ref()
                .map(|p| split_opt(Some(p)).unwrap_or_default());
            let r = decide::trust(&d, repo, kind, paths).map_err(agent_err)?;
            ctx.emit(&r, |x| render_level("以後照建議自動放行", x));
        }
        DecideCmd::Untrust { repo, kind } => {
            let r = decide::untrust(&d, repo, kind).map_err(agent_err)?;
            ctx.emit(&r, |x| render_level("降回每次都問", x));
        }
        DecideCmd::Overturn { reference, why } => {
            let r = decide::overturn(&d, reference, why.as_deref()).map_err(agent_err)?;
            ctx.emit(&r, render_overturned);
        }
        DecideCmd::Suggest { repo, text, why } => {
            let cwd = std::env::current_dir().ok();
            let r = decide::suggest(&d, repo, &text.join(" "), why.as_deref(), cwd.as_deref())
                .map_err(agent_err)?;
            ctx.emit(&r, |r| {
                let a = &r.advice;
                let mut line = format!(
                    "已在 {} 的卡上留{}：{}{}　{}",
                    a.repo,
                    a.from.advice_label(),
                    a.option.map(|n| format!("{n}) ")).unwrap_or_default(),
                    a.text,
                    a.reference
                );
                if let Some(w) = &a.why {
                    line.push_str(&format!("\n理由：{w}"));
                }
                line
            });
        }
        DecideCmd::Rules => {
            let r = decide::rules_report(&d).map_err(agent_err)?;
            ctx.emit(&r, render_rules);
        }
    }
    Ok(())
}

fn hint_lines(hints: &[TrustHint]) -> Vec<String> {
    if hints.is_empty() {
        return Vec::new();
    }
    let mut out = vec![String::new(), "## 可以升級成自動放行".to_string()];
    out.extend(hints.iter().map(|h| format!("- {}", h.line())));
    out
}

fn render_list(q: &Queue) -> String {
    if q.cards.is_empty() {
        let mut out = vec![format!("沒有要你決定的事（{}，{}）。", q.scope, q.at)];
        out.extend(hint_lines(&q.suggestions));
        return out.join("\n");
    }
    let mut out = vec![
        format!("# 要你決定：{}（{}）", q.scope, q.at),
        format!("共 {} 件，{} 件有結構化提問", q.cards.len(), q.structured),
    ];
    for (i, c) in q.cards.iter().enumerate() {
        out.push(String::new());
        out.extend(card_lines(i + 1, c));
    }
    out.extend(hint_lines(&q.suggestions));
    if q.cards.len() >= STEWARD_HINT_CARDS {
        out.push(String::new());
        out.push("讓總管先整理：ctx session --steward".into());
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
    if let Some(n) = &c.advice {
        let mut line = format!(
            "   {}：{}{}",
            n.from.advice_label(),
            n.option.map(|k| format!("{k}) ")).unwrap_or_default(),
            n.text
        );
        if let Some(w) = &n.why {
            line.push_str(&format!("（{w}）"));
        }
        out.push(line);
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
        format!("答錯了要撤回：ctx decide overturn {}", a.reference),
        String::new(),
    ];
    if let Some(adv) = &r.advice_differs {
        out.push(format!(
            "{}的是「{}」，你選的是「{}」",
            adv.from.advice_label(),
            adv.text,
            a.answer
        ));
        out.push(String::new());
    }
    if let Some(rule) = &r.downgraded {
        out.push(format!(
            "這類原本自動放行，這次跟建議不同 → {}/{} 降回每次都問（since {}）",
            rule.repo,
            rule.kind.as_str(),
            rule.since
        ));
        out.push(String::new());
    }
    out.push(render_answer(&r.relay));
    if let Some(h) = &r.suggestion {
        out.push(String::new());
        out.push(h.line());
    }
    out.join("\n")
}

fn render_request(d: &Decision) -> String {
    match d {
        Decision::Auto(a) => format!(
            "auto：照「{}」接著做（規則 {}，留痕 {}；使用者要推翻：ctx decide overturn {}）",
            a.answer, a.rule, a.reference, a.reference
        ),
        Decision::Wait { reason } => format!(
            "wait（{}）：{}\n能做的先做完，收尾 handoff 帶同一個提問（--ask …），使用者會在 ctx decide list 看到",
            reason.code, reason.message
        ),
    }
}

fn past_line(p: &PastAnswers) -> String {
    if p.answers == 0 {
        "這類還沒有人回答過的紀錄".to_string()
    } else {
        format!(
            "這類過去人回答 {} 次：照建議 {}、跟建議不同 {}；目前連續照建議 {} 次",
            p.answers, p.followed, p.differed, p.streak
        )
    }
}

fn render_level(what: &str, x: &LevelChanged) -> String {
    let prev = match x.previous {
        Some(Level::Auto) => "原本是自動放行",
        Some(Level::Ask) => "原本是每次都問",
        None => "原本沒有規則",
    };
    let mut out = vec![
        format!(
            "{}/{}：{what}（{prev}；since {}）",
            x.rule.repo,
            x.rule.kind.as_str(),
            x.rule.since
        ),
        past_line(&x.past),
    ];
    if !x.rule.paths.is_empty() {
        out.push(format!(
            "只在沒 commit 的檔全在這些路徑底下時放行：{}",
            x.rule.paths.join("、")
        ));
    }
    out.join("\n")
}

fn render_overturned(o: &Overturned) -> String {
    let a = &o.approval;
    if o.what == Undo::Withdraw {
        let mut out = vec![format!(
            "已撤回：{}「{}」→ {}（{}）　留痕 {}",
            a.repo,
            clip(&a.question, 80),
            a.answer,
            a.reference,
            o.reference
        )];
        out.push("這筆回答不再算數（不算連續照建議、不算抽查）；放行規則沒有動".into());
        if a.handoff_ref.is_some() {
            out.push("那張卡回到 ctx decide list，可以重新回答".into());
        }
        out.push("如果已經把回答送給 agent，要另外跟它說".into());
        return out.join("\n");
    }
    let mut out = vec![format!(
        "已推翻：{}「{}」→ {}（{}，{}）　留痕 {}",
        a.repo,
        clip(&a.question, 80),
        a.answer,
        a.rule.as_deref().unwrap_or("—"),
        a.reference,
        o.reference
    )];
    match (&o.rule, &o.note) {
        (Some(r), _) => out.push(format!(
            "{}/{} 降回每次都問（since {}），連續計數從現在算",
            r.repo,
            r.kind.as_str(),
            r.since
        )),
        (None, Some(n)) => out.push(n.clone()),
        (None, None) => {}
    }
    if a.handoff_ref.is_some() {
        out.push("那筆交接回到 ctx decide list；已經照放行的答案動手的 agent，要另外跟它說".into());
    } else {
        out.push("這筆是 agent 做到一半問的：它可能已經照答案做了，要另外跟它說".into());
    }
    out.join("\n")
}

fn render_rules(r: &RulesReport) -> String {
    let mut out = vec![format!(
        "# 放行規則（{}{}）",
        r.file,
        if r.exists {
            ""
        } else {
            "，還沒有這個檔"
        }
    )];
    if r.rules.is_empty() {
        out.push("沒有規則：每個提問都問人。".into());
    }
    for x in &r.rules {
        let level = match x.rule.level {
            Level::Auto => "自動放行",
            Level::Ask => "每次都問",
        };
        let mut line = format!(
            "- {}/{}：{level}（since {}）",
            x.rule.repo,
            x.rule.kind.as_str(),
            x.rule.since
        );
        if !x.rule.paths.is_empty() {
            line.push_str(&format!("；paths {}", x.rule.paths.join("、")));
        }
        // since 只到分鐘：ask 時「since 之後放行 N」數到的是降級前同一分鐘那筆，會誤導，只講怎麼回到 ask 的
        match x.rule.level {
            Level::Auto => line.push_str(&format!(
                "；since 之後放行 {}、被推翻 {}",
                x.approvals, x.overturned
            )),
            Level::Ask if x.overturned > 0 => line.push_str(&format!(
                "；被推翻 {} 次後降回（自 since 起每次都問）",
                x.overturned
            )),
            Level::Ask => {}
        }
        out.push(line);
    }
    out.extend(hint_lines(&r.suggestions));
    if !r.problems.is_empty() {
        out.push(String::new());
        out.push("## policy.yaml 的問題（這些條目不生效＝照樣問人）".into());
        out.extend(r.problems.iter().map(|p| format!("- {p}")));
    }
    out.join("\n")
}

fn render_mine(l: &MyAnswers) -> String {
    if l.items.is_empty() {
        return format!("{} 之後你沒有回答過卡。", l.since);
    }
    let mut out = vec![format!(
        "# 你最近的回答（{} 之後，{} 筆）",
        l.since,
        l.items.len()
    )];
    for it in &l.items {
        let a = &it.approval;
        out.push(String::new());
        out.push(format!(
            "- {} {}（{}）{}",
            a.at,
            a.repo,
            a.follow.label(),
            if a.overturned_by.is_some() {
                "　已撤回"
            } else {
                ""
            }
        ));
        out.push(format!("  問：{}", clip(&a.question, QUESTION_CHARS)));
        out.push(format!("  答：{}", a.answer));
        if let Some(c) = &it.command {
            out.push(format!("  → 撤回：{c}"));
        }
    }
    out.join("\n")
}

fn render_auto(l: &AutoList) -> String {
    if l.items.is_empty() {
        return format!("{} 之後沒有自動放行。", l.since);
    }
    let mut out = vec![format!(
        "# 自動放行（{} 之後，{} 筆）",
        l.since,
        l.items.len()
    )];
    for it in &l.items {
        let a = &it.approval;
        out.push(String::new());
        out.push(format!(
            "- {} {}（{}）{}",
            a.at,
            a.repo,
            a.rule.as_deref().unwrap_or("—"),
            if a.overturned_by.is_some() {
                "　已推翻"
            } else {
                ""
            }
        ));
        out.push(format!("  問：{}", clip(&a.question, QUESTION_CHARS)));
        out.push(format!("  答：{}", a.answer));
        if let Some(c) = &it.command {
            out.push(format!("  → {c}"));
        }
    }
    out.join("\n")
}
