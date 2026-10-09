//! Agent 介面（2026-09-25 Agent 友善輪）：agentapi 的 CLI 入口，--json 回結構化結果。
//! 對應 `repo_context/cli/agent.py`。

use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, ValueEnum};
use ctx_core::agentapi::{self, SearchOpts, Target};
use ctx_core::repocard::{self, ProcProbe};
use ctx_core::ruminate::{self, Link};
use ctx_core::{backfill, crossref, pystr, Error};
use serde_json::{json, Map, Value};

use super::common::{fail, split_opt, CmdResult, Ctx, Exit, EXIT_FINDINGS, EXIT_REJECTED};
use crate::{py_float, truthy};

// ── 共用 ────────────────────────────────────────────────────────────────

#[derive(Args, Debug, Clone)]
pub struct Scoped {
    /// 用哪個目錄推主場（預設目前目錄）
    #[arg(long)]
    pub cwd: Option<String>,
    /// 組名（省略＝主場）
    #[arg(long)]
    pub group: Option<String>,
    /// 逗號分隔 repo id（臨時組合）
    #[arg(long)]
    pub repos: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct WriteOpts {
    /// 寫入主場以外（使用者同意後才用）
    #[arg(long = "cross-scope-ok")]
    pub cross_scope_ok: bool,
    /// 留痕來源（預設 agent；人手動記用 manual）
    #[arg(long, default_value = "agent")]
    pub source: String,
}

impl WriteOpts {
    /// `_agent_call` 會丟掉空字串參數 → 用函式預設值 "agent"。
    fn source(&self) -> &str {
        if self.source.is_empty() {
            "agent"
        } else {
            &self.source
        }
    }
}

/// Python `_agent_call` 只轉交非 None、非 "" 的參數。
fn nz(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|x| !x.is_empty())
}

fn cwd_path(s: &Option<String>) -> Option<PathBuf> {
    nz(s).map(PathBuf::from)
}

impl Scoped {
    fn target(&self) -> Target {
        Target {
            cwd: cwd_path(&self.cwd),
            group: nz(&self.group).map(str::to_string),
            repos: split_opt(nz(&self.repos)),
        }
    }
}

/// `_agent_call` 的錯誤出口：AgentError → (code, message, hint)；ValueError → bad_request。
fn agent_err(e: Error) -> Exit {
    match e {
        Error::Rejected {
            code,
            message,
            hint,
        } => fail(&code, message, hint, EXIT_REJECTED),
        Error::Invalid(m) | Error::Validation(m) => fail("bad_request", m, None, EXIT_REJECTED),
        other => Exit::Core(other),
    }
}

/// 對應 `_agent_call`：跑 agentapi，錯誤轉結構化，成功印人話或 JSON。
fn agent_call(
    ctx: &Ctx,
    f: impl FnOnce(&Path) -> ctx_core::Result<Value>,
    human: impl FnOnce(&Value) -> String,
) -> CmdResult {
    let d = ctx.spine_dir()?;
    let res = f(&d).map_err(agent_err)?;
    ctx.emit(&res, human);
    Ok(())
}

/// Value 欄位當字串（Python f-string 的 `{r['x']}`；缺＝空字串）。
fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        None => String::new(),
        Some(x) => pystr::py_str(x),
    }
}

/// `r.get(k)` 的真值。
fn has(v: &Value, k: &str) -> bool {
    v.get(k).is_some_and(truthy)
}

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// `a or b or c or ""`
fn first_truthy(v: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find(|k| has(v, k))
        .map(|k| s(v, k))
        .unwrap_or_default()
}

// ── 人話格式 ────────────────────────────────────────────────────────────

fn h_where(r: &Value) -> String {
    let mut out = if has(r, "repo") {
        let groups: Vec<String> = arr(r, "groups").iter().map(pystr::py_str).collect();
        let g = if groups.is_empty() {
            "（不在任何組）".to_string()
        } else {
            groups.join("、")
        };
        format!(
            "repo：{}（{}）\n組：{g}",
            s(&r["repo"], "id"),
            s(&r["repo"], "path")
        )
    } else {
        format!(
            "這個目錄沒有登記{}",
            if has(r, "in_spine") {
                "（在 spine 裡）"
            } else {
                ""
            }
        )
    };
    for k in ["note", "hint"] {
        if has(r, k) {
            out.push('\n');
            out.push_str(&s(r, k));
        }
    }
    out
}

fn h_next(r: &Value) -> String {
    let mut out = vec![format!("# 下一步（{}）", s(&r["scope"], "label"))];
    if has(r, "goal") {
        out.push(format!("目標：{}", s(r, "goal")));
    }
    for (i, it) in arr(r, "items").iter().enumerate() {
        let tag = first_truthy(it, &["id", "ref", "repo"]);
        out.push(
            format!("{}. [{}] {tag} {}", i + 1, s(it, "kind"), s(it, "text")).replace("  ", " "),
        );
        out.push(format!("   為什麼：{}", s(it, "why")));
    }
    if has(r, "note") {
        out.push(s(r, "note"));
    }
    out.join("\n")
}

fn snippet_lines(out: &mut Vec<String>, f: &Value) {
    for sn in arr(f, "snippets") {
        out.push(format!("    L{}: {}", s(sn, "line"), s(sn, "text")));
    }
}

fn h_search(r: &Value) -> String {
    let q: String = s(r, "query").chars().take(40).collect();
    let indexed = r
        .get("indexed_files")
        .map(pystr::py_str)
        .unwrap_or_else(|| "0".into());
    let mut out = vec![format!(
        "# 相關度（{}，索引 {indexed} 檔）：{q}",
        s(r, "scope")
    )];
    if has(r, "note") {
        out.push(s(r, "note"));
    }
    let repos = arr(r, "repos");
    if !repos.is_empty() {
        let xs: Vec<String> = repos
            .iter()
            .take(6)
            .map(|x| format!("{}({})", s(x, "repo"), s(x, "score")))
            .collect();
        out.push(format!("repo 排名：{}", xs.join("、")));
    }
    for f in arr(r, "files") {
        let why: Vec<String> = arr(f, "why").iter().map(pystr::py_str).collect();
        let why = if has(f, "why") {
            format!("（{}）", why.join("；"))
        } else {
            String::new()
        };
        out.push(format!(
            "- {}/{}  {}{why}",
            s(f, "repo"),
            s(f, "file"),
            s(f, "score")
        ));
        snippet_lines(&mut out, f);
    }
    let decisions = arr(r, "decisions");
    if !decisions.is_empty() {
        out.push("過去的判斷：".into());
        for d in decisions {
            let mut tail = String::new();
            if has(d, "superseded_by") {
                tail.push_str(&format!("（已被 {} 取代）", s(d, "superseded_by")));
            }
            for o in arr(d, "outcomes") {
                tail.push_str(&format!("（結果 {}：{}）", s(o, "at"), s(o, "text")));
            }
            out.push(format!(
                "- {} {} {}{tail}",
                s(d, "at"),
                s(d, "ref"),
                s(d, "text")
            ));
            if has(d, "hit") {
                out.push(format!("    {}", s(d, "hit")));
            }
        }
    }
    out.join("\n")
}

fn h_search_full(r: &Value) -> String {
    let mut out = vec![h_search(r)];
    if has(r, "code") {
        out.push("程式碼：".into());
        for c in arr(r, "code") {
            let why: Vec<String> = arr(c, "why").iter().map(pystr::py_str).collect();
            out.push(format!(
                "- {}/{}  （{}）",
                s(c, "repo"),
                s(c, "file"),
                why.join("；")
            ));
            snippet_lines(&mut out, c);
        }
    }
    out.join("\n")
}

fn h_resolve(r: &Value) -> String {
    let mut out = format!(
        "{}{}",
        s(r, "path"),
        if has(r, "exists") {
            ""
        } else {
            "（不存在）"
        }
    );
    if has(r, "export") {
        out.push_str(&format!("\n= {}:{}", s(r, "repo"), s(r, "export")));
        if has(r, "desc") {
            out.push_str(&format!("（{}）", s(r, "desc")));
        }
    }
    if has(r, "hint") {
        out.push_str(&format!("\n{}", s(r, "hint")));
    }
    out
}

fn h_read(r: &Value) -> String {
    let commit = {
        let c = first_truthy(r, &["file_commit", "commit"]);
        if c.is_empty() {
            "-".to_string()
        } else {
            c
        }
    };
    let head = format!("# {}:{}（commit {commit}）", s(r, "repo"), s(r, "rel"));
    if s(r, "kind") == "dir" {
        let mut lines = vec![head];
        let entries = arr(r, "entries");
        for e in entries {
            let files = if has(e, "files") {
                format!("（{} 檔）", s(e, "files"))
            } else {
                String::new()
            };
            lines.push(format!("- {}{files}", s(e, "name")));
        }
        let total = r.get("total_entries").and_then(Value::as_u64).unwrap_or(0);
        if total as usize > entries.len() {
            lines.push(format!("- …共 {total} 項"));
        }
        return lines.join("\n");
    }
    let tail = if has(r, "hint") {
        format!("\n\n（{}）", s(r, "hint"))
    } else {
        String::new()
    };
    format!("{head}\n{}{tail}", s(r, "content"))
}

fn h_ask(r: &Value) -> String {
    let commit = if has(r, "commit") {
        s(r, "commit")
    } else {
        "-".into()
    };
    let mut out = vec![
        format!(
            "# {} 的回答（{}，commit {commit}）",
            s(r, "repo"),
            s(r, "provider")
        ),
        s(r, "answer"),
    ];
    if has(r, "citations") {
        out.push("\n引用：".into());
        for c in arr(r, "citations") {
            let mut line = format!("- {}:{}", s(r, "repo"), s(c, "file"));
            if has(c, "lines") {
                line.push_str(&format!(" L{}", s(c, "lines")));
            }
            if has(c, "why") {
                line.push_str(&format!("——{}", s(c, "why")));
            }
            out.push(line);
        }
    }
    if has(r, "note") {
        out.push(format!("\n（{}）", s(r, "note")));
    }
    out.join("\n")
}

fn h_written(r: &Value) -> String {
    let ev = r.get("event").cloned().unwrap_or(Value::Null);
    let mut bits = vec![first_truthy(r, &["ref", "id"])];
    if has(r, "todos") {
        let todos: Vec<String> = arr(r, "todos").iter().map(pystr::py_str).collect();
        bits.push(format!("新未結 {}", todos.join(" ")));
    }
    if has(&ev, "text") {
        bits.push(s(&ev, "text"));
    }
    let bits: Vec<String> = bits.into_iter().filter(|b| !b.is_empty()).collect();
    format!("已寫入：{}", bits.join("｜"))
}

// ── 子命令 ──────────────────────────────────────────────────────────────

pub fn cmd_where(ctx: &Ctx, cwd: &Option<String>) -> CmdResult {
    let c = cwd_path(cwd);
    agent_call(
        ctx,
        |d| {
            Ok(serde_json::to_value(agentapi::where_am_i(
                d,
                c.as_deref(),
            )?)?)
        },
        h_where,
    )
}

pub fn cmd_context(ctx: &Ctx, sc: &Scoped) -> CmdResult {
    let t = sc.target();
    agent_call(
        ctx,
        |d| agentapi::context_for(d, &t, true),
        |r| {
            r.get("card")
                .filter(|c| truthy(c))
                .map(pystr::py_str)
                .unwrap_or_default()
        },
    )
}

pub fn cmd_next(ctx: &Ctx, sc: &Scoped, limit: i64) -> CmdResult {
    let t = sc.target();
    agent_call(
        ctx,
        |d| agentapi::next_work(d, &t, limit.max(0) as usize),
        h_next,
    )
}

#[derive(Args, Debug)]
pub struct LogArgs {
    pub text: String,
    /// 相關事件 ref，例 decision:2026-09-25-d1
    #[arg(long = "ref")]
    pub reference: Option<String>,
    /// 這筆取代哪一筆判斷（decision:<id>；Rust 才有）
    #[arg(long)]
    pub supersedes: Option<String>,
    /// 什麼情況要重看（寫日期 YYYY-MM-DD 會在到期時提醒；Rust 才有）
    #[arg(long)]
    pub reopen_when: Option<String>,
    /// 這段文字是哪一筆判斷後來的結果（decision:<id>；寫成 outcome；Rust 才有）
    #[arg(long)]
    pub outcome_of: Option<String>,
    #[command(flatten)]
    pub scope: Scoped,
    #[command(flatten)]
    pub w: WriteOpts,
}

pub fn cmd_log(ctx: &Ctx, a: &LogArgs) -> CmdResult {
    let t = a.scope.target();
    let extra = agentapi::DecisionExtra {
        supersedes: a.supersedes.clone(),
        reopen_when: a.reopen_when.clone(),
        outcome_of: a.outcome_of.clone(),
    };
    agent_call(
        ctx,
        |d| {
            agentapi::log_decision_ext(
                d,
                &a.text,
                &t,
                nz(&a.reference),
                &extra,
                a.w.cross_scope_ok,
                a.w.source(),
            )
        },
        h_written,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum TodoAction {
    Add,
    Close,
    /// 體檢：哪些可能已處理（附證據）、哪些擱置太久（Rust 才有）
    Review,
    /// 任務包：把一項未結組成給全新會話的 markdown（目標／背景／輸入／限制／完成條件；Rust 才有）
    Brief,
}

#[derive(Args, Debug)]
pub struct TodoArgs {
    pub action: TodoAction,
    /// add：要做的事；close／brief：編號（#3 或 3）；review：不用
    #[arg(default_value = "")]
    pub text: String,
    /// YYYY-MM-DD
    #[arg(long)]
    pub due: Option<String>,
    /// close 時：怎麼結的
    #[arg(long)]
    pub note: Option<String>,
    #[command(flatten)]
    pub scope: Scoped,
    #[command(flatten)]
    pub w: WriteOpts,
}

pub fn cmd_todo(ctx: &Ctx, a: &TodoArgs) -> CmdResult {
    if a.text.is_empty() && !matches!(a.action, TodoAction::Review) {
        return Err(fail(
            "bad_request",
            "todo add／close／brief 需要 <TEXT>（add：要做的事；close／brief：編號）",
            None,
            2,
        ));
    }
    match a.action {
        TodoAction::Review => {
            let t = a.scope.target();
            agent_call(ctx, |d| agentapi::review_todos(d, &t), h_review)
        }
        TodoAction::Brief => {
            let t = a.scope.target();
            agent_call(
                ctx,
                |d| agentapi::todo_brief(d, &a.text, &t),
                |r| s(r, "markdown"),
            )
        }
        TodoAction::Add => {
            let t = a.scope.target();
            agent_call(
                ctx,
                |d| {
                    agentapi::add_todo(d, &a.text, nz(&a.due), &t, a.w.cross_scope_ok, a.w.source())
                },
                h_written,
            )
        }
        TodoAction::Close => {
            let c = cwd_path(&a.scope.cwd);
            agent_call(
                ctx,
                |d| {
                    agentapi::close_todo(
                        d,
                        &a.text,
                        nz(&a.note),
                        c.as_deref(),
                        a.w.cross_scope_ok,
                        a.w.source(),
                    )
                },
                |r| format!("已關閉 {}：{}", s(r, "id"), s(r, "closed")),
            )
        }
    }
}

/// `todo review` 的人話：可能已處理（附證據）→ 擱置太久 → 其餘只算數量。
fn h_review(r: &Value) -> String {
    let c = &r["counts"];
    let mut out = vec![format!(
        "未結體檢（{}）：可能已處理 {}、擱置太久 {}、進行中 {}",
        s(&r["scope"], "label"),
        s(c, "maybe_done"),
        s(c, "stale"),
        s(c, "active")
    )];
    for it in arr(r, "items") {
        let tag = match it["verdict"].as_str() {
            Some("maybe_done") => "可能已處理",
            Some("stale") => "擱置太久",
            _ => continue,
        };
        let bf = if it["backfilled"] == true {
            "（回填）"
        } else {
            ""
        };
        out.push(format!("- {} [{tag}] {}{bf}", s(it, "id"), s(it, "text")));
        out.push(format!("    {}", s(it, "why")));
        for e in arr(it, "evidence") {
            let shared: Vec<String> = arr(e, "shared").iter().map(pystr::py_str).collect();
            out.push(format!(
                "    · {} {} {}——蓋住 {}%：{}",
                s(e, "at"),
                s(e, "ref"),
                pystr::take_chars(&s(e, "text"), 60),
                (e["cover"].as_f64().unwrap_or(0.0) * 100.0).round(),
                shared.join("、")
            ));
        }
    }
    if out.len() == 1 {
        out.push("沒有看起來已處理或擱置太久的項目。".into());
    } else {
        out.push("確認後：ctx todo close <#N> --note \"怎麼結的\"；還要做就 ctx todo add 重開一筆新的 due。".into());
    }
    out.join("\n")
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    #[arg(value_parser = agentapi::STATUSES)]
    pub status: String,
    #[arg(default_value = "")]
    pub text: String,
    #[command(flatten)]
    pub scope: Scoped,
}

pub fn cmd_status(ctx: &Ctx, a: &StatusArgs) -> CmdResult {
    let t = a.scope.target();
    agent_call(
        ctx,
        |d| agentapi::report_status(d, &a.status, &a.text, &t),
        |r| {
            format!(
                "狀態：{}（{}）",
                s(&r["status"], "status"),
                s(&r["status"], "scope")
            )
        },
    )
}

#[derive(Args, Debug)]
pub struct HandoffArgs {
    #[arg(long, required = true)]
    pub done: String,
    #[arg(long = "next", required = true)]
    pub next_step: String,
    /// 還沒做完的新事項（可重複；各開一個未結）
    #[arg(long)]
    pub remaining: Vec<String>,
    #[command(flatten)]
    pub scope: Scoped,
    #[command(flatten)]
    pub w: WriteOpts,
}

pub fn cmd_handoff(ctx: &Ctx, a: &HandoffArgs) -> CmdResult {
    let t = a.scope.target();
    agent_call(
        ctx,
        |d| {
            agentapi::handoff(
                d,
                &a.done,
                &a.next_step,
                &a.remaining,
                &t,
                a.w.cross_scope_ok,
                a.w.source(),
            )
        },
        h_written,
    )
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    pub query: Vec<String>,
    /// 拿一個檔的內容當查詢（例：一張新卡片）
    #[arg(long)]
    pub file: Option<String>,
    /// 只搜主場範圍（預設搜全部 repo）
    #[arg(long)]
    pub here: bool,
    /// 不搜程式碼（只搜 md）
    #[arg(long = "no-code")]
    pub no_code: bool,
    #[arg(long, default_value_t = 8)]
    pub limit: i64,
    #[command(flatten)]
    pub scope: Scoped,
}

pub fn cmd_search(ctx: &Ctx, a: &SearchArgs) -> CmdResult {
    let query = if !a.query.is_empty() {
        a.query.join(" ")
    } else if let Some(f) = nz(&a.file) {
        std::fs::read_to_string(f)?
    } else {
        String::new()
    };
    let t = a.scope.target();
    let opts = SearchOpts {
        everywhere: !a.here,
        limit: a.limit.max(0) as usize,
        exclude_paths: nz(&a.file).map(PathBuf::from).into_iter().collect(),
        code: !a.no_code,
    };
    agent_call(
        ctx,
        |d| agentapi::search_knowledge(d, &query, &t, &opts),
        h_search_full,
    )
}

#[derive(Args, Debug)]
pub struct FreshArgs {
    #[command(flatten)]
    pub scope: Scoped,
    /// 現在跑一輪反芻（排程任務 ruminate 做同一件事）
    #[arg(long)]
    pub run: bool,
    /// 配 --run：回看幾天（預設＝上次反芻之後）
    #[arg(long)]
    pub days: Option<i64>,
    /// 連用過／略過的也列
    #[arg(long)]
    pub all: bool,
    /// 略過一筆推薦，例 r3
    #[arg(long, value_name = "ID")]
    pub dismiss: Option<String>,
    /// 拿來試：在推薦所屬的組開一個帶出處的待辦，推薦標成用上了（Rust 才有）
    #[arg(long = "try", value_name = "ID")]
    pub try_id: Option<String>,
    /// 命中率：推薦後來有多少被讀了
    #[arg(long)]
    pub stats: bool,
}

fn h_fresh(r: &Value) -> String {
    if has(r, "note") {
        return s(r, "note");
    }
    let last = if has(r, "last_run") {
        s(r, "last_run")
    } else {
        "從未".into()
    };
    let head = format!("# 反芻推薦（{}；上次反芻 {last}）", s(r, "scope"));
    let links = arr(r, "links");
    if links.is_empty() {
        return format!(
            "{head}\n目前沒有新推薦。{}",
            if has(r, "last_run") {
                ""
            } else {
                "先跑 `ctx fresh --run`。"
            }
        );
    }
    let mut out = vec![head];
    for lv in links {
        let status = s(lv, "status");
        let tag = match status.as_str() {
            "new" => String::new(),
            "used" => "〔用上了〕".into(),
            "dismissed" => "〔已略過〕".into(),
            other => format!("〔{other}〕"),
        };
        let id = s(lv, "id");
        let lines = match serde_json::from_value::<Link>(lv.clone()) {
            Ok(l) => ruminate::link_lines(&[l], 1),
            Err(_) => Vec::new(),
        };
        for (i, ln) in lines.into_iter().enumerate() {
            out.push(if i == 0 {
                ln.replacen(&format!("[{id}]"), &format!("[{id}]{tag}"), 1)
            } else {
                ln
            });
        }
        let basis: Vec<String> = arr(lv, "basis").iter().map(pystr::py_str).collect();
        out.push(format!(
            "    依據：{}；分數 {}",
            basis.join("、"),
            s(lv, "score")
        ));
    }
    out.join("\n")
}

pub fn cmd_fresh(ctx: &Ctx, a: &FreshArgs) -> CmdResult {
    if let Some(id) = a.try_id.as_deref() {
        let source = if ctx.json { "agent" } else { "manual" };
        return agent_call(
            ctx,
            |d| agentapi::try_fresh(d, id, source),
            |r| {
                format!(
                    "已開待辦 {}（{}）：{}",
                    s(r, "todo"),
                    s(r, "group"),
                    s(r, "text")
                )
            },
        );
    }
    let d = ctx.spine_dir()?;
    if a.run {
        let res = ruminate::run(&d, None, a.days)?;
        let n: usize = res.groups.iter().map(|(_, v)| v.len()).sum();
        ctx.emit(&res, |r| {
            let mut out = format!(
                "反芻完成：自 {} 起 {} 個新／改過的檔，{n} 筆新推薦",
                r.since, r.candidates
            );
            for (g, v) in &r.groups {
                out.push_str(&format!("\n  {g}：{}", v.len()));
            }
            out
        });
        return Ok(());
    }
    if a.stats {
        let st = ruminate::stats(&d)?;
        ctx.emit(&st, |s| {
            format!(
                "推薦 {} 筆：未處理 {}、用上了 {}、略過 {}；命中率 {}（上次反芻 {}）",
                s.total,
                s.new,
                s.used,
                s.dismissed,
                s.hit_rate.map(py_float).unwrap_or_else(|| "—".into()),
                s.last_run
                    .as_deref()
                    .filter(|x| !x.is_empty())
                    .unwrap_or("從未")
            )
        });
        return Ok(());
    }
    if let Some(id) = nz(&a.dismiss) {
        let l = ruminate::dismiss(&d, id).map_err(|e| match e {
            Error::Invalid(m) | Error::Validation(m) => fail("not_found", m, None, EXIT_REJECTED),
            other => Exit::Core(other),
        })?;
        let out = json!({"ok": true, "dismissed": l});
        ctx.emit(&out, |_| {
            format!(
                "已略過 {} {}",
                l.id.clone().unwrap_or_else(|| "None".into()),
                l.reference
            )
        });
        return Ok(());
    }
    let t = a.scope.target();
    agent_call(ctx, |d| agentapi::fresh_links(d, &t, a.all), h_fresh)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum BackfillAction {
    Scan,
    Run,
    Review,
    Accept,
}

#[derive(Args, Debug)]
pub struct BackfillArgs {
    pub action: BackfillAction,
    /// accept：<repo> 後接候選編號（d1 t2 h）或 all
    pub ids: Vec<String>,
    /// 只處理這個 repo
    #[arg(long)]
    pub repo: Option<String>,
    /// run：這次最多抽幾場
    #[arg(long)]
    pub limit: Option<i64>,
    /// run：claude 模型（預設 sonnet）
    #[arg(long, default_value = "sonnet")]
    pub model: String,
    /// run：同時跑幾個模型呼叫（預設 4）
    #[arg(long, default_value_t = 4)]
    pub jobs: i64,
}

/// Python `Counter(...).most_common()`：次數多的在前，同次數依第一次出現的順序。
fn most_common<'a>(items: impl Iterator<Item = &'a str>) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for x in items {
        match out.iter_mut().find(|(k, _)| k == x) {
            Some((_, n)) => *n += 1,
            None => out.push((x.to_string(), 1)),
        }
    }
    // 穩定排序：同次數保持第一次出現的順序
    out.sort_by_key(|x| std::cmp::Reverse(x.1));
    out
}

pub fn cmd_backfill(ctx: &Ctx, a: &BackfillArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    match a.action {
        BackfillAction::Scan => {
            let recs = backfill::scan(&d, None, None)?;
            // dict(Counter)：第一次出現的順序
            let mut counts = Map::new();
            for r in &recs {
                let n = counts.get(&r.status).and_then(Value::as_u64).unwrap_or(0);
                counts.insert(r.status.clone(), json!(n + 1));
            }
            let c = most_common(recs.iter().map(|r| r.status.as_str()));
            let per = most_common(
                recs.iter()
                    .filter(|r| r.status == "pending" || r.status == "done")
                    .map(|r| r.repo.as_deref().unwrap_or("None")),
            );
            let mut unmapped: Vec<String> = recs
                .iter()
                .filter(|r| r.status == "unmapped")
                .map(|r| {
                    r.cwd
                        .clone()
                        .filter(|c| !c.is_empty())
                        .unwrap_or_else(|| "?".into())
                })
                .collect();
            unmapped.sort();
            unmapped.dedup();
            let out = json!({"sessions": recs, "counts": counts});
            ctx.emit(&out, |_| {
                let mut lines = vec![format!(
                    "會話 {} 場：{}",
                    recs.len(),
                    c.iter()
                        .map(|(k, v)| format!("{k} {v}"))
                        .collect::<Vec<_>>()
                        .join("、")
                )];
                lines.extend(per.iter().map(|(rid, n)| format!("  {rid}：{n} 場")));
                lines.extend(
                    unmapped
                        .iter()
                        .map(|cwd| format!("對不到 repo 的 cwd：{cwd}")),
                );
                lines.join("\n")
            });
            Ok(())
        }
        BackfillAction::Run => {
            let call = backfill::claude_caller(&d, &a.model);
            let opts = backfill::RunOpts {
                root: None,
                limit: a.limit.filter(|x| *x > 0).map(|x| x as usize),
                repo: nz(&a.repo).map(str::to_string),
                jobs: a.jobs.max(1) as usize,
            };
            let quiet = ctx.json;
            let log = move |m: &str| {
                if !quiet {
                    println!("{m}");
                }
            };
            let res = backfill::run(&d, &call, &opts, &log)?;
            ctx.emit(&res, |r| {
                format!(
                    "抽取 {} 場、彙整 {} 個 repo、失敗 {}；看結果：ctx backfill review",
                    r.extracted,
                    r.consolidated.len(),
                    r.failed.len()
                )
            });
            if !res.failed.is_empty() {
                return Err(Exit::Code(EXIT_FINDINGS));
            }
            Ok(())
        }
        BackfillAction::Review => {
            let repo = nz(&a.repo);
            let cands = backfill::candidates(&d, repo)?;
            if ctx.json {
                ctx.emit_json(&cands);
            } else {
                let text = backfill::review_text(&d, repo)?;
                if !text.is_empty() {
                    println!("{text}");
                }
            }
            Ok(())
        }
        BackfillAction::Accept => {
            let mut repo = nz(&a.repo).map(str::to_string);
            let mut ids = a.ids.clone();
            if repo.is_none() && !ids.is_empty() {
                // ctx backfill accept <repo> d1 t2 h（--repo 寫在最後也可以）
                repo = Some(ids.remove(0));
            }
            let (Some(repo), false) = (repo.filter(|r| !r.is_empty()), ids.is_empty()) else {
                return Err(fail(
                    "bad_request",
                    "用法：ctx backfill accept <repo> <d1 t2 h …|all>",
                    None,
                    EXIT_REJECTED,
                ));
            };
            let refs = backfill::accept(&d, &repo, &ids).map_err(|e| match e {
                Error::Invalid(m) | Error::Validation(m) => {
                    fail("bad_request", m, None, EXIT_REJECTED)
                }
                Error::Rejected { message, .. } => {
                    fail("bad_request", message, None, EXIT_REJECTED)
                }
                other => Exit::Core(other),
            })?;
            let out = json!({"ok": true, "written": refs});
            ctx.emit(&out, |_| {
                if refs.is_empty() {
                    "都收過了".into()
                } else {
                    format!("寫進 spine {} 筆：{}", refs.len(), refs.join(" "))
                }
            });
            Ok(())
        }
    }
}

pub fn cmd_resolve(ctx: &Ctx, reference: &str) -> CmdResult {
    agent_call(ctx, |d| agentapi::resolve_ref(d, reference), h_resolve)
}

#[derive(Args, Debug)]
pub struct ReadArgs {
    #[arg(value_name = "REF")]
    pub reference: String,
    /// 只讀某段，例 120-180
    #[arg(long)]
    pub lines: Option<String>,
    /// 引用方目錄（預設目前目錄）
    #[arg(long)]
    pub cwd: Option<String>,
}

pub fn cmd_read(ctx: &Ctx, a: &ReadArgs) -> CmdResult {
    let c = cwd_path(&a.cwd);
    agent_call(
        ctx,
        |d| agentapi::read_from(d, &a.reference, nz(&a.lines), c.as_deref()),
        h_read,
    )
}

#[derive(Args, Debug)]
pub struct AskArgs {
    pub repo: String,
    #[arg(required = true, num_args = 1..)]
    pub question: Vec<String>,
    /// claude（預設看 config provider.ask）或 mock
    #[arg(long)]
    pub provider: Option<String>,
    #[arg(long)]
    pub cwd: Option<String>,
}

pub fn cmd_ask(ctx: &Ctx, a: &AskArgs) -> CmdResult {
    let c = cwd_path(&a.cwd);
    let q = a.question.join(" ");
    agent_call(
        ctx,
        |d| agentapi::ask_repo(d, &a.repo, &q, c.as_deref(), nz(&a.provider)),
        h_ask,
    )
}

#[derive(Args, Debug)]
pub struct RepoArgs {
    /// repo id（省略＝目前目錄所在 repo）
    pub id: Option<String>,
    /// 7d／30d／commit hash；省略＝你上次看過之後
    #[arg(long)]
    pub since: Option<String>,
    /// 看但不記已讀（下次仍從同一點列）
    #[arg(long)]
    pub peek: bool,
    #[arg(long)]
    pub cwd: Option<String>,
}

/// 狀態卡。人看（不帶 --peek、不是 --json）才推已讀水位線。
pub fn cmd_repo(ctx: &Ctx, a: &RepoArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    let rid = match nz(&a.id) {
        Some(r) => r.to_string(),
        None => {
            let c = cwd_path(&a.cwd);
            let w = agentapi::where_am_i(&d, c.as_deref()).map_err(agent_err)?;
            match w.repo {
                Some(r) => r.id,
                None => {
                    let hint = w.hint.clone().or(w.note.clone());
                    return Err(fail(
                        "bad_request",
                        "目前目錄不在已登記的 repo 裡，請指定 repo id",
                        hint,
                        EXIT_REJECTED,
                    ));
                }
            }
        }
    };
    let card = repocard::build(
        &d,
        &rid,
        a.since.as_deref(),
        !(a.peek || ctx.json),
        None,
        ProcProbe::default(),
    )
    .map_err(|e| match e {
        Error::Invalid(m) | Error::Validation(m) => fail("bad_request", m, None, EXIT_REJECTED),
        Error::Rejected { message, .. } => fail("bad_request", message, None, EXIT_REJECTED),
        other => Exit::Core(other),
    })?;
    ctx.emit(&card, repocard::render);
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum RefsAction {
    Check,
    Drift,
    Suggest,
    Cite,
}

#[derive(Args, Debug)]
pub struct RefsArgs {
    pub action: RefsAction,
    /// cite：被引用的 <repo>:<路徑>
    #[arg(value_name = "REF")]
    pub reference: Option<String>,
    /// 只看這個 repo（check／drift）；cite 時＝引用方
    #[arg(long)]
    pub repo: Option<String>,
    /// cite：說明（例：複製進 20_reference/）
    #[arg(long)]
    pub note: Option<String>,
    #[arg(long)]
    pub cwd: Option<String>,
    /// suggest：被引用幾次以上才建議
    #[arg(long, default_value_t = 3)]
    pub min: i64,
}

pub fn cmd_refs(ctx: &Ctx, a: &RefsArgs) -> CmdResult {
    let d = ctx.spine_dir()?;
    let repo = nz(&a.repo);
    let min = a.min.max(0) as usize;
    match a.action {
        RefsAction::Check => {
            let only: Option<Vec<String>> = repo.map(|r| vec![r.to_string()]);
            let broken = crossref::lint_doc_refs(&d, only.as_deref())?;
            let dr = crossref::drift(&d, repo, None)?;
            let sug = crossref::suggest_exports(&d, min, 1)?;
            let res = json!({"ok": true, "broken": broken, "drift": dr, "suggest": sug});
            ctx.emit(&res, |_| {
                let mut out: Vec<String> = Vec::new();
                if !broken.is_empty() {
                    out.push(format!(
                        "## agent 文件裡失效的跨 repo 路徑（{}）",
                        broken.len()
                    ));
                    out.extend(broken.iter().map(|b| {
                        format!(
                            "- {}/{}:{} `{}`\n  → {}",
                            b.repo, b.doc, b.line, b.reference, b.fix
                        )
                    }));
                }
                if !dr.is_empty() {
                    out.push(format!("## 引用過的上游有變（{}）", dr.len()));
                    out.extend(crossref::drift_lines(&dr, 50));
                }
                if !sug.is_empty() {
                    out.push("## 常被引用、建議宣告成 export".into());
                    out.extend(sug.iter().map(|x| {
                        format!(
                            "- {}:{} 被引用 {} 次（{}）\n  → {}",
                            x.repo,
                            x.path,
                            x.cites,
                            x.consumers.join(", "),
                            x.command
                        )
                    }));
                }
                if out.is_empty() {
                    "跨 repo 參考零紅字 OK".into()
                } else {
                    out.join("\n")
                }
            });
            if (!broken.is_empty() || !dr.is_empty()) && !ctx.json {
                return Err(Exit::Code(EXIT_FINDINGS));
            }
            Ok(())
        }
        RefsAction::Drift => {
            let dr = crossref::drift(&d, repo, None)?;
            ctx.emit(&dr, |x| {
                let lines = crossref::drift_lines(x, 100);
                if lines.is_empty() {
                    "引用過的上游都沒變".into()
                } else {
                    lines.join("\n")
                }
            });
            Ok(())
        }
        RefsAction::Suggest => {
            let sug = crossref::suggest_exports(&d, min, 1)?;
            ctx.emit(&sug, |xs| {
                if xs.is_empty() {
                    format!("沒有被引用 ≥{} 次卻沒宣告的目錄", a.min)
                } else {
                    xs.iter()
                        .map(|x| {
                            format!(
                                "{}:{} 被引用 {} 次 → {}",
                                x.repo, x.path, x.cites, x.command
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            });
            Ok(())
        }
        RefsAction::Cite => {
            // 手動記一筆引用：例如把上游檔複製進本 repo 時，記下當時的版本（之後上游改了會提醒）
            let Some(reference) = nz(&a.reference) else {
                return Err(Exit::Msg(
                    "用法：refs cite <repo>:<路徑> [--note 說明] [--cwd 引用方 repo 目錄]".into(),
                ));
            };
            let r = crossref::resolve(&d, reference).map_err(agent_err)?;
            let c = cwd_path(&a.cwd);
            let w = agentapi::where_am_i(&d, c.as_deref()).map_err(agent_err)?;
            let consumer = repo
                .map(str::to_string)
                .or_else(|| w.repo.as_ref().map(|x| x.id.clone()));
            let Some(consumer) = consumer else {
                return Err(fail(
                    "bad_request",
                    "認不出引用方 repo：在該 repo 目錄下執行，或帶 --repo",
                    None,
                    EXIT_REJECTED,
                ));
            };
            let commit = Command::new("git")
                .args(["-C", &r.root, "rev-parse", "--short", "HEAD"])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|s| !s.is_empty());
            let rec = crossref::record_cite(
                &d,
                Some(&consumer),
                &r.repo,
                &r.rel,
                commit.as_deref(),
                "manual",
                nz(&a.note),
                None,
            )?;
            let out = json!({"ok": true, "cite": rec});
            ctx.emit(&out, |x| {
                if truthy(&x["cite"]) {
                    format!(
                        "已記引用：{consumer} ← {}:{}（{}）",
                        r.repo,
                        r.rel,
                        s(&x["cite"], "commit")
                    )
                } else {
                    "今天已經記過這筆引用".into()
                }
            });
            Ok(())
        }
    }
}

/// 健康指標（Rust 才有，F-36／N-07）：真用一陣子後，用數字看行為有沒有改變。
pub fn cmd_health(ctx: &Ctx, days: i64) -> CmdResult {
    let d = ctx.spine_dir()?;
    let v = ctx_core::health::health(&d, days, None)?;
    ctx.emit(&v, |r| {
        let pct = |x: &Value| {
            x.as_f64()
                .map(|f| format!("{:.0}%", f * 100.0))
                .unwrap_or_else(|| "—".into())
        };
        let mut out = vec![format!(
            "# 健康指標（近 {} 天，自 {}）",
            s(r, "days"),
            s(r, "since")
        )];
        let h = &r["handoff"];
        out.push(format!(
            "- 交接率 {}：交接 {}、沒交接就結束 {}",
            pct(&h["rate"]),
            s(h, "handoffs"),
            s(h, "missed")
        ));
        let f = &r["fresh"];
        out.push(format!(
            "- 推薦命中率 {}：推了 {}、用上 {}、略過 {}",
            pct(&f["hit_rate"]),
            s(f, "found"),
            s(f, "used"),
            s(f, "dismissed")
        ));
        let dd = &r["decisions"];
        out.push(format!(
            "- 判斷結果記錄率 {}：有結果 {}、待回顧 {}",
            pct(&dd["outcome_rate"]),
            s(dd, "with_outcome"),
            s(dd, "awaiting_retro")
        ));
        let l = &r["loops"];
        out.push(format!(
            "- 未結：期間開 {}、關 {}；現在 {} 項，逾期 {}",
            s(l, "opened"),
            s(l, "closed"),
            s(l, "open_now"),
            s(l, "overdue")
        ));
        let tools = arr(r, "tools");
        if tools.is_empty() {
            out.push("- 工具用量：還沒有紀錄（MCP 呼叫才會記，`.state/metrics.jsonl`）".into());
        } else {
            out.push("- 工具用量：".into());
            for t in tools {
                out.push(format!(
                    "    {}：{} 次，平均 {}ms，失敗 {}",
                    s(t, "tool"),
                    s(t, "calls"),
                    s(t, "avg_ms"),
                    s(t, "errors")
                ));
            }
        }
        out.join("\n")
    });
    Ok(())
}

#[derive(clap::Subcommand, Debug)]
pub enum DecisionsCmd {
    /// 寫成 AgDR md：<dir>/AgDR-<id>.md（被取代的標 superseded；帶何時重看與結果；沒變的不重寫）
    Export {
        /// 輸出目錄（例：docs/agdr）
        dir: String,
        #[command(flatten)]
        scope: Scoped,
    },
}

/// 判斷匯出（Rust 才有，F-40）。
pub fn cmd_decisions(ctx: &Ctx, cmd: &DecisionsCmd) -> CmdResult {
    match cmd {
        DecisionsCmd::Export { dir, scope } => {
            let t = scope.target();
            let dir = PathBuf::from(dir);
            agent_call(
                ctx,
                |d| agentapi::export_agdr(d, &t, &dir),
                |r| {
                    format!(
                        "匯出到 {}：寫了 {} 份、沒變 {} 份（範圍 {}）",
                        s(r, "dir"),
                        arr(r, "written").len(),
                        s(r, "unchanged"),
                        s(&r["scope"], "label")
                    )
                },
            )
        }
    }
}
