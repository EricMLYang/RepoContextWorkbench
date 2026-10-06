//! 日常：採集、打包、簡報、脊椎事件、碰撞、落點、上游、digest、排程、通知。
//! 對應 `repo_context/cli/daily.py`。

use std::path::Path;
use std::sync::mpsc;

use chrono::Local;
use clap::{Args, ValueEnum};
use ctx_core::collect::{self, RepoSpec};
use ctx_core::pack::{self, RepoRef};
use ctx_core::spine::{self, EventType, Query};
use ctx_core::{
    agentapi, brief, collide, config, digest, notify, registry, route, spawn, timer, upstream,
};
use serde_json::Value;

use super::common::{pct0, split_opt, CmdResult, Ctx, Exit, EXIT_FINDINGS};
use crate::pyjson;

#[derive(Args, Debug)]
pub struct ScopeArgs {
    #[arg(long)]
    pub group: Option<String>,
    #[arg(long)]
    pub repos: Option<String>,
}

impl ScopeArgs {
    pub fn ids(&self) -> Option<Vec<String>> {
        split_opt(self.repos.as_deref())
    }
}

/// `_registry.resolve_group(d, args.group, args.repos)`。
fn resolve(d: &Path, s: &ScopeArgs) -> Result<(String, Vec<registry::Repo>), Exit> {
    Ok(registry::resolve_group(
        d,
        s.group.as_deref(),
        s.ids().as_deref(),
    )?)
}

pub fn cmd_lint(ctx: &Ctx) -> CmdResult {
    let d = ctx.spine_dir()?;
    let reds = spine::lint(&d, None)?;
    if !reds.is_empty() {
        for r in &reds {
            println!("[RED] {r}");
        }
        return Err(Exit::Code(EXIT_FINDINGS));
    }
    println!("脊椎 lint 零紅字 OK（衛生迴圈）");
    Ok(())
}

pub fn cmd_collect(ctx: &Ctx, s: &ScopeArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    let (_, entries) = resolve(&d, s)?;
    let specs: Vec<RepoSpec> = entries.iter().map(RepoSpec::from).collect();
    let states = collect::collect_group(&specs, Some(&d));
    ctx.emit(&states, |st| collect::format_table(st));
    Ok(())
}

#[derive(Args, Debug)]
pub struct PulseArgs {
    #[command(flatten)]
    pub scope: ScopeArgs,
    #[arg(long, default_value_t = 20)]
    pub limit: i64,
}

/// 活動脈動：組的監控專注「活動頻繁度＋近期 commit 內容」（2026-09-08）。
pub fn cmd_pulse(ctx: &Ctx, a: &PulseArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    let (gname, entries) = resolve(&d, &a.scope)?;
    let specs: Vec<RepoSpec> = entries.iter().map(RepoSpec::from).collect();
    let states = collect::collect_group(&specs, Some(&d));
    let pulse = collect::group_pulse(&states);
    println!("# 脈動 {gname}");
    println!("{}", collect::pulse_line(&pulse));
    println!();
    println!("{:<24} {:>3} {:>3}  最近 commit", "repo", "7d", "30d");
    for s in &states {
        if s.repo_type != "mine" || !s.exists {
            continue;
        }
        let head = match s.recent_commits.first() {
            Some(c) => format!("{} {}", c.date, c.subject),
            None => "（無）".into(),
        };
        println!(
            "{:<24} {:>3} {:>3}  {head}",
            s.id,
            s.commits_7d.unwrap_or(0),
            s.commits_30d.unwrap_or(0)
        );
    }
    println!();
    println!("# 近期 commit（新→舊，最多 {} 則）", a.limit);
    // Python 的 list[:limit]：負數＝去掉尾巴幾個
    let all = collect::recent_across(&states, usize::MAX);
    let n = py_slice_len(all.len(), a.limit);
    for c in all.iter().take(n) {
        println!("{} {:<24} {} {}", c.date, c.repo, c.hash, c.subject);
    }
    Ok(())
}

/// Python `xs[:limit]` 的長度。
fn py_slice_len(len: usize, limit: i64) -> usize {
    if limit >= 0 {
        (limit as usize).min(len)
    } else {
        len.saturating_sub(limit.unsigned_abs() as usize)
    }
}

#[derive(Args, Debug)]
pub struct PackArgs {
    #[command(flatten)]
    pub scope: ScopeArgs,
    #[arg(long)]
    pub budget: Option<i64>,
    /// 只算各 repo token 成本不打包（挑的預算函數）
    #[arg(long)]
    pub estimate: bool,
}

pub fn cmd_pack(ctx: &Ctx, a: &PackArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    let (_, entries) = resolve(&d, &a.scope)?;
    let refs: Vec<RepoRef> = entries.iter().map(RepoRef::from).collect();
    let cfg = config::load(&d)?;
    let budget = match a.budget {
        Some(b) if b != 0 => b,
        _ => cfg.pack_token_budget(),
    };
    if a.estimate {
        // 挑的預算函數：先算再挑，粒度由數字反推（Repomix 課）
        let mut rows = pack::estimate(&refs);
        let total: usize = rows.iter().map(|r| r.tokens).sum();
        rows.sort_by_key(|r| std::cmp::Reverse(r.tokens));
        for r in &rows {
            println!("{:<24} {:>4} 檔 ~{:>7} tokens", r.id, r.files, r.tokens);
        }
        let fit = if (total as i64) <= budget {
            "裝得下".to_string()
        } else {
            "超預算（挑細一點或加 --budget）".to_string()
        };
        println!(
            "{:<24} {:>6} ~{:>7} tokens／預算 {budget} → {fit}",
            "合計", "", total
        );
        return Ok(());
    }
    let p = pack::pack_group(&refs, budget.max(0) as usize);
    let group = a
        .scope
        .group
        .as_deref()
        .filter(|g| !g.is_empty())
        .unwrap_or("adhoc");
    let out = d
        .join("groups")
        .join(group)
        .join("materials")
        .join(format!("pack-{}.md", Local::now().format("%Y%m%d-%H%M%S")));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &p.text)?;
    let mut msg = format!(
        "打包完成：{}（收錄 {}、未納入 {}",
        out.display(),
        p.included.len(),
        p.excluded.len()
    );
    if p.flagged.is_empty() {
        msg.push('）');
    } else {
        msg.push_str(&format!("、疑似機密擋下 {}）", p.flagged.len()));
    }
    println!("{msg}");
    Ok(())
}

pub fn cmd_brief(ctx: &Ctx, s: &ScopeArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    let ids = s.ids();
    let (out, text) = brief::run(&d, s.group.as_deref(), ids.as_deref(), None)?;
    println!("{text}");
    println!("→ 已寫入 {} 並落 presented 事件", out.display());
    Ok(())
}

#[derive(Args, Debug)]
pub struct AppendArgs {
    #[arg(long = "type", required = true)]
    pub etype: String,
    #[arg(long, required = true)]
    pub source: String,
    #[arg(long)]
    pub body: Option<String>,
    #[arg(long)]
    pub kv: Vec<String>,
    /// 人的入口：拒寫時原文落 dead-letter 不丟
    #[arg(long)]
    pub dead_letter: bool,
}

pub fn cmd_append(ctx: &Ctx, a: &AppendArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    let body = a.body.clone().unwrap_or_default();
    let mut tokens = a.kv.clone();
    if a.etype == "open-loop" {
        let due_days = config::load(&d)?.thresholds().openloop_default_due_days;
        tokens = spine::default_due_tokens(&tokens, &body, due_days, None);
    }
    let ev = if a.dead_letter {
        spine::append_event_or_dead_letter(&d, &a.etype, &a.source, &tokens, &body, None)?
    } else {
        Some(spine::append_event(
            &d, &a.etype, &a.source, &tokens, &body, None,
        )?)
    };
    match ev {
        None => {
            println!("validator 拒寫 → 原文已落 spine/dead-letter/（想法不丟）");
            Err(Exit::Code(1))
        }
        Some(ev) => {
            println!("已寫入：{}", ev.header());
            Ok(())
        }
    }
}

#[derive(Args, Debug)]
pub struct QueryArgs {
    #[arg(long = "type")]
    pub etype: Option<String>,
    #[arg(long)]
    pub date: Option<String>,
    #[arg(long)]
    pub group: Option<String>,
    #[arg(long)]
    pub today: bool,
    #[arg(long)]
    pub stats: bool,
    #[arg(short, long)]
    pub verbose: bool,
}

pub fn cmd_query(ctx: &Ctx, a: &QueryArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    if a.stats {
        let s = spine::stats(&d)?;
        let sv = registry::survival(&d)?;
        let rate = s
            .hit_rate
            .map(pct0)
            .unwrap_or_else(|| "n/a（無碰撞）".into());
        let srate = sv.survival_rate.map(pct0).unwrap_or_else(|| "n/a".into());
        println!(
            "碰撞 {} 次｜有 outcome 回連 {} 次｜靈感命中率 {rate}",
            s.collisions, s.collisions_with_outcome
        );
        println!(
            "生出 repo {} 個｜存活 {}｜存活率 {srate}",
            sv.spawned, sv.alive
        );
        return Ok(());
    }
    let date = if a.today {
        Some(Local::now().format("%Y-%m-%d").to_string())
    } else {
        a.date.clone()
    };
    let evs = query_events(&d, a.etype.as_deref(), date, a.group.clone())?;
    if ctx.json {
        let rows: Vec<Value> = evs.iter().map(agentapi::event_dict).collect();
        ctx.emit_json(&rows);
        return Ok(());
    }
    for ev in &evs {
        println!("{} {}", ev.date, ev.header());
        if a.verbose && !ev.body.is_empty() {
            println!("   {}", ev.body.replace('\n', "\n   "));
        }
    }
    Ok(())
}

/// `spine.query(d, type=..., date=..., group=...)`：Python 拿字串比對，未知型別＝查不到東西。
pub fn query_events(
    d: &Path,
    etype: Option<&str>,
    date: Option<String>,
    group: Option<String>,
) -> ctx_core::Result<Vec<spine::Event>> {
    let et = match etype.filter(|t| !t.is_empty()) {
        None => None,
        Some(t) => match t.parse::<EventType>() {
            Ok(t) => Some(t),
            Err(_) => return Ok(Vec::new()),
        },
    };
    spine::query(
        d,
        &Query {
            etype: et,
            date,
            group,
            since: None,
        },
    )
}

pub fn cmd_loops(ctx: &Ctx, s: &ScopeArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    let mut loops = spine::open_loops(&d)?;
    let given = |x: &Option<String>| x.as_deref().is_some_and(|v| !v.is_empty());
    if given(&s.group) || given(&s.repos) {
        let (_, entries) = resolve(&d, s)?;
        let ids: Vec<String> = entries.iter().map(|e| e.id()).collect();
        loops.retain(|ev| spine::in_scope(ev, s.group.as_deref(), &ids));
    }
    if ctx.json {
        let rows: Vec<Value> = loops
            .iter()
            .map(|ev| agentapi::loop_dict(ev, None))
            .collect();
        ctx.emit_json(&rows);
        return Ok(());
    }
    if loops.is_empty() {
        println!("無未結 open loops");
    }
    for ev in &loops {
        let num = ev.num().unwrap_or("#?");
        println!(
            "{num} due:{} {}",
            ev.kv("due").filter(|x| !x.is_empty()).unwrap_or("-"),
            ev.first_line()
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum CollideAction {
    Submit,
    Run,
}

#[derive(Args, Debug)]
pub struct CollideArgs {
    pub action: CollideAction,
    pub idea: Option<String>,
    #[arg(long)]
    pub cid: Option<String>,
    #[command(flatten)]
    pub scope: ScopeArgs,
    #[arg(long)]
    pub provider: Option<String>,
    /// 同步跑段二
    #[arg(long)]
    pub wait: bool,
    /// 只落想法不判定
    #[arg(long)]
    pub no_run: bool,
}

pub fn cmd_collide(ctx: &Ctx, a: &CollideArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    let ids = a.scope.ids();
    let group = a.scope.group.as_deref();
    let provider = a.provider.as_deref();
    match a.action {
        CollideAction::Submit => {
            let Some(idea) = a.idea.as_deref() else {
                return Err(Exit::Msg("錯誤：collide submit 需要想法文字".into()));
            };
            let cid = collide::submit(&d, idea, group, ids.as_deref(), "hotkey", None)?;
            println!("已丟進碰撞：{cid}（想法已落脊椎）");
            if a.wait {
                let j = collide::run_judgement(&d, &cid, group, ids.as_deref(), provider, None)?;
                match j {
                    Some(j) => println!("{}", pyjson::dumps_indent(&j)),
                    None => println!("判定失敗（system-unsure）——已落脊椎浮出"),
                }
            } else if !a.no_run {
                collide::spawn_detached(&d, &cid, group, provider)?;
                println!("判定已丟 detached 進程，結果走脊椎未讀回程");
            }
            Ok(())
        }
        CollideAction::Run => {
            // 段二；detached 進程的入口（collide run <cid> 位置參數）
            let cid = a
                .cid
                .clone()
                .filter(|c| !c.is_empty())
                .or_else(|| a.idea.clone())
                .unwrap_or_default();
            match collide::run_judgement(&d, &cid, group, ids.as_deref(), provider, None)? {
                None => Err(Exit::Code(1)),
                Some(j) => {
                    println!("{}", pyjson::dumps_indent(&j));
                    Ok(())
                }
            }
        }
    }
}

#[derive(Args, Debug)]
pub struct RouteArgs {
    #[arg(long)]
    pub text: Option<String>,
    #[arg(long)]
    pub file: Option<String>,
    #[arg(long, required = true)]
    pub dest: String,
    #[arg(long, default_value = "idea")]
    pub title: String,
}

pub fn cmd_route(ctx: &Ctx, a: &RouteArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    let text = match (&a.file, &a.text) {
        (Some(f), _) if !f.is_empty() => std::fs::read_to_string(f)?,
        (_, Some(t)) => t.clone(),
        _ => return Err(Exit::Msg("錯誤：route 需要 --text 或 --file".into())),
    };
    let p = route::route(&d, &text, &a.dest, &a.title, None)?;
    println!("已落：{}", p.display());
    Ok(())
}

pub fn cmd_unread(ctx: &Ctx, ack: bool) -> CmdResult {
    let d = ctx.spine_dir()?;
    let evs = spine::get_unread(&d)?;
    println!("未讀 {} 則", evs.len());
    for ev in &evs {
        println!("{} {}", ev.date, ev.header());
    }
    if ack {
        spine::ack_unread(&d, None)?;
        println!("已標記全部已讀");
    }
    Ok(())
}

pub fn cmd_commit(ctx: &Ctx, message: &str) -> CmdResult {
    let d = ctx.spine_dir()?;
    println!(
        "{}",
        if spine::batch_commit(&d, message)? {
            "已 commit"
        } else {
            "無變更可 commit"
        }
    );
    Ok(())
}

pub fn cmd_upstream(ctx: &Ctx, s: &ScopeArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    let ids = s.ids();
    let (n, findings) = upstream::check(&d, s.group.as_deref(), ids.as_deref(), None)?;
    if n == 0 {
        println!("無 external repo 可查（registry 的 external 需帶 upstream 欄位）");
        return Ok(());
    }
    if findings.is_empty() {
        println!("查了 {n} 個 external，上游無新事（無事不報）");
        return Ok(());
    }
    for f in &findings {
        println!(
            "{}: {} {} {}（已落 suggestion 事件）",
            f.repo, f.kind, f.id, f.title
        );
    }
    Ok(())
}

#[derive(Args, Debug)]
pub struct DigestArgs {
    #[command(flatten)]
    pub scope: ScopeArgs,
    #[arg(long)]
    pub provider: Option<String>,
}

pub fn cmd_digest(ctx: &Ctx, a: &DigestArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    let ids = a.scope.ids();
    for r in digest::run(
        &d,
        a.scope.group.as_deref(),
        ids.as_deref(),
        a.provider.as_deref(),
        None,
    )? {
        match r.file.as_deref().filter(|f| !f.is_empty()) {
            Some(f) => println!("{}: {} → {f}", r.repo, r.note),
            None => println!("{}: {}", r.repo, r.note),
        }
    }
    Ok(())
}

#[derive(Args, Debug)]
pub struct SpawnArgs {
    pub id: String,
    pub path: String,
    /// incubator 裡的素材檔名（會搬進新 repo）
    #[arg(long = "from-incubator")]
    pub from_incubator: Option<String>,
    /// 血統，如 collision:2026-09-01-a（會落 outcome 回連）
    #[arg(long)]
    pub origin: Option<String>,
}

pub fn cmd_spawn(ctx: &Ctx, a: &SpawnArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    let p = spawn::spawn(
        &d,
        &a.id,
        &a.path,
        a.from_incubator.as_deref(),
        a.origin.as_deref(),
        None,
    )?;
    println!(
        "已升格：{} → {}（出生登記＋血統已落脊椎）",
        a.id,
        p.display()
    );
    Ok(())
}

#[derive(Args, Debug)]
pub struct TimerArgs {
    /// 跑一輪到期任務就退出
    #[arg(long)]
    pub once: bool,
    /// 相容 Python 版的旗標；Rust 版睡到下一個排程點（最多 300 秒重看一次），不再固定間隔 tick
    #[arg(long, default_value_t = 30)]
    pub interval: i64,
}

pub fn cmd_timer(ctx: &Ctx, a: &TimerArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    if a.once {
        let ran = timer::tick(&d, None, None)?;
        if ran.is_empty() {
            println!("無到期任務");
        }
        for (key, ok) in &ran {
            println!(
                "{} {key}",
                if *ok {
                    "✔"
                } else {
                    "✘（已浮出 system-unsure）"
                }
            );
        }
        return Ok(());
    }
    println!("timer 啟動（睡到下一個排程點，最多 300s 重看一次；Ctrl+C 結束）");
    let (tx, rx) = mpsc::channel::<()>();
    // Ctrl+C → 送停止訊號；run_loop 在 recv_timeout 醒來就結束（不在任務跑到一半時硬殺）
    ctrlc::set_handler(move || {
        let _ = tx.send(());
    })
    .map_err(|e| Exit::Msg(format!("錯誤：無法攔 Ctrl+C：{e}")))?;
    timer::run_loop(&d, &rx);
    println!("\ntimer 已停");
    Ok(())
}

pub fn cmd_notify(ctx: &Ctx) -> CmdResult {
    let d = ctx.spine_dir()?;
    println!("{}", notify::render(&notify::pending(&d)?));
    Ok(())
}
