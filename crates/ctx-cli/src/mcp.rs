//! MCP server——兩層工具（2026-09-25 Agent 友善輪）。對應 `repo_context/mcpserver.py`。
//!
//! **工作層（預設）**：照 agent 一次工作的節奏切——where_am_i／context_for／next_work／
//! log_decision／add_todo／close_todo／report_status／handoff／search_knowledge，
//! 加上跨 repo 參考輪的 read_from／ask_repo／repo_status。
//! 依 server 的 cwd（或參數 cwd）推出主場範圍；回傳 JSON（帶可引用的 ref／id）；
//! 錯誤是結構化的 {ok:false, error, message, hint}；寫入主場以外會被拒（cross_scope）。
//! 邏輯全在 `ctx_core::agentapi`，CLI `--json` 與 hooks 同源。
//!
//! **管理層（`ctx mcp --admin` 或 config `mcp.admin: true` 才開）**：registry／組／採集／
//! 打包／碰撞／升格等維護工具——給人（或人明確授權的 agent）用，預設不塞給 agent。
//!
//! stdio 傳輸（newline-delimited JSON-RPC 2.0），只用 serde_json 手寫、零額外依賴（耗材原則）。
//! 拒寫去向（P10 檢討④）：validator 錯誤**原樣**回給 agent（isError tool result）供自我修正；
//! dead-letter 是人的入口（hotkey）專屬，MCP 不走。

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use chrono::Local;
use ctx_core::agentapi::{self, SearchOpts, Target};
use ctx_core::collect::{self, RepoSpec};
use ctx_core::ops::{self, Via};
use ctx_core::pack::{self, RepoRef};
use ctx_core::{
    brief, collide, config, context, crossref, digest, notify, pystr, registry, route, spawn,
    spine, upstream, Error, Result,
};
use once_cell::sync::Lazy;
use serde_json::{json, Map, Value};

use crate::cli::common::pct0;
use crate::{pyjson, truthy};

pub const PROTOCOL_VERSION: &str = "2024-11-05";

type Args = Map<String, Value>;

// ---------------------------------------------------------------- 參數小工具

/// 字串參數（非字串以 Python `str()` 轉；null／缺＝None）。
fn arg_s(a: &Args, k: &str) -> Option<String> {
    match a.get(k) {
        None | Some(Value::Null) => None,
        Some(v) => Some(pystr::py_str(v)),
    }
}

/// `a.get(k)` 的真值。
fn arg_flag(a: &Args, k: &str) -> bool {
    a.get(k).is_some_and(truthy)
}

/// 字串清單參數（陣列；單一字串視為一項）。
fn arg_list(a: &Args, k: &str) -> Vec<String> {
    match a.get(k) {
        Some(Value::Array(xs)) => xs.iter().map(pystr::py_str).collect(),
        Some(Value::Null) | None => Vec::new(),
        Some(v) => vec![pystr::py_str(v)],
    }
}

/// `repos` 參數：逗號字串或陣列。空＝None。
fn arg_repos(a: &Args) -> Option<Vec<String>> {
    match a.get("repos") {
        None | Some(Value::Null) => None,
        Some(Value::Array(xs)) if xs.is_empty() => None,
        Some(Value::Array(xs)) => Some(xs.iter().map(pystr::py_str).collect()),
        Some(Value::String(s)) if s.is_empty() => None,
        Some(v) => Some(registry::split_ids(&pystr::py_str(v))),
    }
}

/// 整數參數（數字或數字字串）。
fn arg_int(a: &Args, k: &str) -> Option<i64> {
    match a.get(k)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn ev_lines(events: &[spine::Event], verbose: bool) -> String {
    let mut out = Vec::new();
    for ev in events {
        out.push(format!("{} {}", ev.date, ev.header()));
        if verbose && !ev.body.is_empty() {
            out.push(format!("   {}", ev.body.replace('\n', "\n   ")));
        }
    }
    if out.is_empty() {
        "（無）".into()
    } else {
        out.join("\n")
    }
}

fn scope_ids(a: &Args) -> (Option<String>, Option<Vec<String>>) {
    (arg_s(a, "group"), arg_repos(a))
}

fn resolve(d: &Path, a: &Args) -> Result<(String, Vec<registry::Repo>)> {
    let (g, ids) = scope_ids(a);
    registry::resolve_group(d, g.as_deref(), ids.as_deref())
}

// ---------------------------------------------------------------- 管理層 handlers（回字串；Err＝isError）

fn t_registry_list(d: &Path, _a: &Args) -> Result<String> {
    let rows: Vec<String> = registry::load(d)?
        .repos
        .iter()
        .map(|r| {
            format!(
                "{}\t{}\t{}\t{}",
                r.id(),
                r.kind().unwrap_or_else(|| "mine".into()),
                r.tier_str().unwrap_or_else(|| "?".into()),
                r.path_str()
            )
        })
        .collect();
    Ok(if rows.is_empty() {
        "（registry 空）".into()
    } else {
        rows.join("\n")
    })
}

fn t_registry_audit(d: &Path, a: &Args) -> Result<String> {
    let reds = registry::audit(d, &arg_list(a, "scan_dirs"))?;
    Ok(if reds.is_empty() {
        "audit 零紅字 OK".into()
    } else {
        reds.iter()
            .map(|r| format!("[RED] {r}"))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

fn t_registry_scan(d: &Path, a: &Args) -> Result<String> {
    let dir = arg_s(a, "dir").ok_or_else(|| Error::invalid("'dir'"))?;
    if arg_flag(a, "apply") {
        let added = registry::scan_register(d, &dir)?;
        return Ok(if added.is_empty() {
            "無新 repo 可登記".into()
        } else {
            format!(
                "已登記：{}（上半身 type/tier/tags 待人補）",
                added.join(", ")
            )
        });
    }
    let cands = registry::scan_dir(d, &dir)?;
    Ok(if cands.is_empty() {
        "無未登記 repo".into()
    } else {
        cands
            .iter()
            .map(|(p, cid)| format!("[候選] {cid}: {}", p.display()))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

fn t_spine_lint(d: &Path, _a: &Args) -> Result<String> {
    let reds = spine::lint(d, None)?;
    Ok(if reds.is_empty() {
        "脊椎 lint 零紅字 OK（衛生迴圈）".into()
    } else {
        reds.iter()
            .map(|r| format!("[RED] {r}"))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

fn t_pack_estimate(d: &Path, a: &Args) -> Result<String> {
    let (_, entries) = resolve(d, a)?;
    let refs: Vec<RepoRef> = entries.iter().map(RepoRef::from).collect();
    let mut rows = pack::estimate(&refs);
    let total: usize = rows.iter().map(|r| r.tokens).sum();
    let budget = config::load(d)?.pack_token_budget();
    rows.sort_by_key(|r| std::cmp::Reverse(r.tokens));
    let mut lines: Vec<String> = rows
        .iter()
        .map(|r| format!("{}\t{} 檔\t~{} tokens", r.id, r.files, r.tokens))
        .collect();
    lines.push(format!(
        "合計 ~{total} tokens／預算 {budget} → {}",
        if (total as i64) <= budget {
            "裝得下"
        } else {
            "超預算（挑細一點）"
        }
    ));
    Ok(lines.join("\n"))
}

fn note_tail(note: &Option<String>) -> String {
    match note {
        Some(n) if !n.is_empty() => format!("  {n}"),
        _ => String::new(),
    }
}

fn t_registry_relations(d: &Path, a: &Args) -> Result<String> {
    let out: Vec<String> = match arg_s(a, "id").filter(|s| !s.is_empty()) {
        Some(id) => registry::relations_of(d, &id)?
            .iter()
            .map(|r| {
                let head = if r.direction == registry::Direction::Out {
                    format!("{id} {}→ {}", r.label, r.peer)
                } else {
                    format!("{id} 的{} {}", r.label, r.peer)
                };
                format!("{head}（{}）{}", r.kind, note_tail(&r.note))
            })
            .collect(),
        None => {
            let kinds = registry::relation_kinds(d)?;
            registry::relations(d, None)?
                .iter()
                .map(|r| {
                    let fwd = kinds
                        .get(&r.kind)
                        .map(|k| k.forward.clone())
                        .unwrap_or_else(|| r.kind.clone());
                    format!(
                        "{} {fwd}→ {}（{}）{}",
                        r.from,
                        r.to,
                        r.kind,
                        note_tail(&r.note)
                    )
                })
                .collect()
        }
    };
    if out.is_empty() {
        let kinds = registry::relation_kinds(d)?;
        let names: Vec<&str> = kinds.0.iter().map(|(k, _)| k.as_str()).collect();
        return Ok(format!("（無關係）可用 kind：{}", names.join(", ")));
    }
    Ok(out.join("\n"))
}

fn t_refs_check(d: &Path, _a: &Args) -> Result<String> {
    let broken = crossref::lint_doc_refs(d, None)?;
    let dr = crossref::drift(d, None, None)?;
    let sug = crossref::suggest_exports(d, 3, 1)?;
    let mut out: Vec<String> = broken
        .iter()
        .map(|b| {
            format!(
                "[失效路徑] {}/{}:{} `{}` → {}",
                b.repo, b.doc, b.line, b.reference, b.fix
            )
        })
        .collect();
    // Python `f"[上游有變]{ln[1:]}"`：去掉 drift 行開頭的一個字元
    out.extend(crossref::drift_lines(&dr, 50).iter().map(|ln| {
        let mut cs = ln.chars();
        cs.next();
        format!("[上游有變]{}", cs.as_str())
    }));
    out.extend(sug.iter().map(|s| {
        format!(
            "[建議 export] {}:{} 被引用 {} 次（{}）→ {}",
            s.repo,
            s.path,
            s.cites,
            s.consumers.join(", "),
            s.command
        )
    }));
    Ok(if out.is_empty() {
        "跨 repo 參考零紅字 OK".into()
    } else {
        out.join("\n")
    })
}

fn t_group_context(d: &Path, a: &Args) -> Result<String> {
    let (g, ids) = scope_ids(a);
    context::build_context(d, g.as_deref(), ids.as_deref(), None, None)
}

fn t_group_list(d: &Path, _a: &Args) -> Result<String> {
    let rows: Vec<String> = registry::load(d)?
        .groups
        .iter()
        .map(|g| format!("{}\t{}", g.name(), g.members().join(",")))
        .collect();
    Ok(if rows.is_empty() {
        "（無組）".into()
    } else {
        rows.join("\n")
    })
}

fn t_collect(d: &Path, a: &Args) -> Result<String> {
    let (_, entries) = resolve(d, a)?;
    let specs: Vec<RepoSpec> = entries.iter().map(RepoSpec::from).collect();
    Ok(collect::format_table(&collect::collect_group(
        &specs,
        Some(d),
    )))
}

fn t_pack(d: &Path, a: &Args) -> Result<String> {
    let (_, entries) = resolve(d, a)?;
    let refs: Vec<RepoRef> = entries.iter().map(RepoRef::from).collect();
    let budget = match arg_int(a, "budget") {
        Some(b) if b != 0 => b,
        _ => config::load(d)?.pack_token_budget(),
    };
    let p = pack::pack_group(&refs, budget.max(0) as usize);
    Ok(format!(
        "<!-- 收錄 {}、未納入 {}、疑似機密擋下 {} -->\n{}",
        p.included.len(),
        p.excluded.len(),
        p.flagged.len(),
        p.text
    ))
}

fn t_brief(d: &Path, a: &Args) -> Result<String> {
    let (g, ids) = scope_ids(a);
    Ok(brief::run(d, g.as_deref(), ids.as_deref(), None)?.1)
}

fn t_spine_append(d: &Path, a: &Args) -> Result<String> {
    let etype = arg_s(a, "type").ok_or_else(|| Error::invalid("'type'"))?;
    let source = arg_s(a, "source").ok_or_else(|| Error::invalid("'source'"))?;
    let body = arg_s(a, "body").unwrap_or_default();
    let mut tokens = arg_list(a, "kv");
    if etype == "open-loop" {
        let due_days = config::load(d)?.thresholds().openloop_default_due_days;
        tokens = spine::default_due_tokens(&tokens, &body, due_days, None);
    }
    let ev = spine::append_event(d, &etype, &source, &tokens, &body, None)?;
    Ok(format!("已寫入：{}", ev.header()))
}

fn t_spine_query(d: &Path, a: &Args) -> Result<String> {
    let date = if arg_flag(a, "today") {
        Some(Local::now().format("%Y-%m-%d").to_string())
    } else {
        arg_s(a, "date")
    };
    let evs =
        crate::cli::daily::query_events(d, arg_s(a, "type").as_deref(), date, arg_s(a, "group"))?;
    Ok(ev_lines(&evs, arg_flag(a, "verbose")))
}

fn t_spine_stats(d: &Path, _a: &Args) -> Result<String> {
    let s = spine::stats(d)?;
    let sv = registry::survival(d)?;
    let rate = s.hit_rate.map(pct0).unwrap_or_else(|| "n/a".into());
    let srate = sv.survival_rate.map(pct0).unwrap_or_else(|| "n/a".into());
    Ok(format!(
        "碰撞 {} 次｜outcome 回連 {} 次｜靈感命中率 {rate}｜生出 repo {} 個｜存活 {}｜存活率 {srate}",
        s.collisions, s.collisions_with_outcome, sv.spawned, sv.alive
    ))
}

fn t_open_loops(d: &Path, a: &Args) -> Result<String> {
    // 範圍跟簡報／情境卡同一份規則（以前這裡回全部組的未結）
    let mut evs = spine::open_loops(d)?;
    let (g, ids) = scope_ids(a);
    if g.as_deref().is_some_and(|x| !x.is_empty()) || ids.is_some() {
        let (_, entries) = resolve(d, a)?;
        let ids: Vec<String> = entries.iter().map(|e| e.id()).collect();
        evs.retain(|ev| spine::in_scope(ev, g.as_deref(), &ids));
    }
    Ok(ev_lines(&evs, true))
}

fn t_unread(d: &Path, a: &Args) -> Result<String> {
    let p = notify::pending(d)?;
    let all: Vec<spine::Event> = p.interrupt.iter().chain(p.normal.iter()).cloned().collect();
    let mut text = format!("{}\n{}", notify::render(&p), ev_lines(&all, false));
    if arg_flag(a, "ack") {
        spine::ack_unread(d, None)?;
        text.push_str("\n（已標記全部已讀）");
    }
    Ok(text)
}

fn t_collide_submit(d: &Path, a: &Args) -> Result<String> {
    let idea = arg_s(a, "idea").ok_or_else(|| Error::invalid("'idea'"))?;
    let (g, ids) = scope_ids(a);
    let provider = arg_s(a, "provider");
    let cid = collide::submit(d, &idea, g.as_deref(), ids.as_deref(), "agent", None)?;
    if arg_flag(a, "wait") {
        let j = collide::run_judgement(d, &cid, None, None, provider.as_deref(), None)?;
        return Ok(format!(
            "cid={cid}\n{}",
            match j {
                Some(j) => pyjson::dumps_indent(&j),
                None => "判定失敗（system-unsure）——已落脊椎浮出".into(),
            }
        ));
    }
    collide::spawn_detached(d, &cid, None, provider.as_deref())?;
    Ok(format!("cid={cid}（判定走 detached，結果看 unread）"))
}

fn t_route(d: &Path, a: &Args) -> Result<String> {
    let text = arg_s(a, "text").ok_or_else(|| Error::invalid("'text'"))?;
    let dest = arg_s(a, "dest").ok_or_else(|| Error::invalid("'dest'"))?;
    let title = arg_s(a, "title").unwrap_or_else(|| "idea".into());
    let p = route::route(d, &text, &dest, &title, None)?;
    Ok(format!("已落：{}", p.display()))
}

fn t_upstream_check(d: &Path, a: &Args) -> Result<String> {
    let (g, ids) = scope_ids(a);
    let (n, findings) = upstream::check(d, g.as_deref(), ids.as_deref(), None)?;
    if n == 0 {
        return Ok("無 external repo 可查（registry 的 external 需帶 upstream 欄位）".into());
    }
    Ok(if findings.is_empty() {
        format!("查了 {n} 個 external，上游無新事")
    } else {
        findings
            .iter()
            .map(|f| format!("{}: {} {} {}", f.repo, f.kind, f.id, f.title))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

fn t_digest_run(d: &Path, a: &Args) -> Result<String> {
    let (g, ids) = scope_ids(a);
    let results = digest::run(
        d,
        g.as_deref(),
        ids.as_deref(),
        arg_s(a, "provider").as_deref(),
        None,
    )?;
    let lines: Vec<String> = results
        .iter()
        .map(|r| match r.file.as_deref().filter(|f| !f.is_empty()) {
            Some(f) => format!("{}: {} → {f}", r.repo, r.note),
            None => format!("{}: {}", r.repo, r.note),
        })
        .collect();
    Ok(if lines.is_empty() {
        "（組內無 mine repo）".into()
    } else {
        lines.join("\n")
    })
}

fn t_spawn(d: &Path, a: &Args) -> Result<String> {
    let id = arg_s(a, "id").ok_or_else(|| Error::invalid("'id'"))?;
    let path = arg_s(a, "path").ok_or_else(|| Error::invalid("'path'"))?;
    let p = spawn::spawn(
        d,
        &id,
        &path,
        arg_s(a, "from_incubator").as_deref(),
        arg_s(a, "origin").as_deref(),
        None,
    )?;
    Ok(format!("已升格：{id} → {}", p.display()))
}

// ---------------------------------------------------------------- inputSchema 簡寫

fn str_t() -> Value {
    json!({"type": "string"})
}
fn bool_t() -> Value {
    json!({"type": "boolean"})
}
fn int_t() -> Value {
    json!({"type": "integer"})
}
fn arr_t() -> Value {
    json!({"type": "array", "items": {"type": "string"}})
}

/// Python `_d(base, desc, **extra)`：`{**base, "description": desc, **extra}`。
fn d_(base: Value, desc: &str) -> Value {
    let mut m = match base {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    m.insert("description".into(), json!(desc));
    Value::Object(m)
}

/// Python `_s(**props)`：`{"type":"object","properties":{...},"required":[...]}`。
fn s_(props: Vec<(&str, Value)>, req: &[&str]) -> Value {
    let mut p = Map::new();
    for (k, v) in props {
        p.insert(k.into(), v);
    }
    json!({"type": "object", "properties": p, "required": req})
}

fn scope_props() -> Vec<(&'static str, Value)> {
    vec![
        ("group", d_(str_t(), "組名；省略＝依 cwd 推出的主場")),
        (
            "repos",
            d_(str_t(), "逗號分隔 repo id（臨時組合，優先於 group）"),
        ),
    ]
}
fn cwd_props() -> Vec<(&'static str, Value)> {
    vec![(
        "cwd",
        d_(
            str_t(),
            "用哪個目錄推主場；省略＝server 啟動目錄（通常就是你的專案）",
        ),
    )]
}
fn cross_props() -> Vec<(&'static str, Value)> {
    vec![(
        "cross_scope_ok",
        d_(
            bool_t(),
            "寫入主場以外的範圍。只有在使用者明確同意後才設 true",
        ),
    )]
}

fn cat(parts: Vec<Vec<(&'static str, Value)>>) -> Vec<(&'static str, Value)> {
    parts.into_iter().flatten().collect()
}

// ---------------------------------------------------------------- 工作層

type WorkFn = fn(&Path, &Args, &Path) -> Result<Value>;
type AdminFn = Box<dyn Fn(&Path, &Args) -> Result<String> + Send + Sync>;

/// 一個工具：名稱、說明、inputSchema、實作。
pub struct Tool {
    pub name: String,
    pub description: String,
    pub schema: Value,
    kind: ToolKind,
}

enum ToolKind {
    Work(WorkFn),
    Admin(AdminFn),
}

/// Python `_w` 轉交的參數：缺必填就照 Python TypeError 的訊息報錯。
fn need(fname: &str, a: &Args, keys: &[&str]) -> Result<()> {
    let missing: Vec<&str> = keys
        .iter()
        .copied()
        .filter(|k| match a.get(*k) {
            None | Some(Value::Null) => true,
            Some(Value::String(s)) => s.is_empty(),
            _ => false,
        })
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let quoted: Vec<String> = missing.iter().map(|k| format!("'{k}'")).collect();
    let list = match quoted.len() {
        1 => quoted[0].clone(),
        2 => format!("{} and {}", quoted[0], quoted[1]),
        n => format!("{}, and {}", quoted[..n - 1].join(", "), quoted[n - 1]),
    };
    Err(Error::External(format!(
        "{fname}() missing {} required positional argument{}: {list}",
        missing.len(),
        if missing.len() == 1 { "" } else { "s" }
    )))
}

/// `_w` 的 cwd 規則：有給就用，沒給用 server 的。
fn cwd_of(a: &Args, here: &Path) -> PathBuf {
    arg_s(a, "cwd")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| here.to_path_buf())
}

fn target(a: &Args, here: &Path) -> Target {
    Target {
        cwd: Some(cwd_of(a, here)),
        group: arg_s(a, "group").filter(|s| !s.is_empty()),
        repos: arg_repos(a),
    }
}

fn nz(a: &Args, k: &str) -> Option<String> {
    arg_s(a, k).filter(|s| !s.is_empty())
}

fn w_where(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    Ok(serde_json::to_value(agentapi::where_am_i(
        d,
        Some(&cwd_of(a, here)),
    )?)?)
}
fn w_context(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    let card = a
        .get("card")
        .filter(|v| !v.is_null())
        .map(truthy)
        .unwrap_or(true);
    agentapi::context_for(d, &target(a, here), card)
}
fn w_next(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    let limit = arg_int(a, "limit").unwrap_or(5).max(0) as usize;
    agentapi::next_work(d, &target(a, here), limit)
}
fn w_log(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("log_decision", a, &["text"])?;
    agentapi::log_decision(
        d,
        &arg_s(a, "text").unwrap_or_default(),
        &target(a, here),
        nz(a, "ref").as_deref(),
        arg_flag(a, "cross_scope_ok"),
        "agent",
    )
}
fn w_add_todo(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("add_todo", a, &["text"])?;
    agentapi::add_todo(
        d,
        &arg_s(a, "text").unwrap_or_default(),
        nz(a, "due").as_deref(),
        &target(a, here),
        arg_flag(a, "cross_scope_ok"),
        "agent",
    )
}
fn w_close_todo(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("close_todo", a, &["id"])?;
    agentapi::close_todo(
        d,
        &arg_s(a, "id").unwrap_or_default(),
        nz(a, "note").as_deref(),
        Some(&cwd_of(a, here)),
        arg_flag(a, "cross_scope_ok"),
        "agent",
    )
}
fn w_status(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("report_status", a, &["status"])?;
    agentapi::report_status(
        d,
        &arg_s(a, "status").unwrap_or_default(),
        &nz(a, "text").unwrap_or_default(),
        &target(a, here),
    )
}
fn w_handoff(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("handoff", a, &["done", "next_step"])?;
    agentapi::handoff(
        d,
        &arg_s(a, "done").unwrap_or_default(),
        &arg_s(a, "next_step").unwrap_or_default(),
        &arg_list(a, "remaining"),
        &target(a, here),
        arg_flag(a, "cross_scope_ok"),
        "agent",
    )
}
fn w_search(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("search_knowledge", a, &["query"])?;
    let mut opts = SearchOpts::default();
    if let Some(v) = a.get("everywhere").filter(|v| !v.is_null()) {
        opts.everywhere = truthy(v);
    }
    if let Some(v) = a.get("code").filter(|v| !v.is_null()) {
        opts.code = truthy(v);
    }
    if let Some(l) = arg_int(a, "limit") {
        opts.limit = l.max(0) as usize;
    }
    agentapi::search_knowledge(
        d,
        &arg_s(a, "query").unwrap_or_default(),
        &target(a, here),
        &opts,
    )
}
fn w_read(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("read_from", a, &["ref"])?;
    agentapi::read_from(
        d,
        &arg_s(a, "ref").unwrap_or_default(),
        nz(a, "lines").as_deref(),
        Some(&cwd_of(a, here)),
    )
}
fn w_ask(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("ask_repo", a, &["repo", "question"])?;
    agentapi::ask_repo(
        d,
        &arg_s(a, "repo").unwrap_or_default(),
        &arg_s(a, "question").unwrap_or_default(),
        Some(&cwd_of(a, here)),
        nz(a, "provider").as_deref(),
    )
}
fn w_repo_status(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    agentapi::repo_status(
        d,
        nz(a, "repo").as_deref(),
        Some(&cwd_of(a, here)),
        nz(a, "since").as_deref(),
    )
}

fn work(name: &str, desc: &str, schema: Value, f: WorkFn) -> Tool {
    Tool {
        name: name.into(),
        description: desc.into(),
        schema,
        kind: ToolKind::Work(f),
    }
}

fn admin(
    name: &str,
    desc: &str,
    schema: Value,
    f: impl Fn(&Path, &Args) -> Result<String> + Send + Sync + 'static,
) -> Tool {
    Tool {
        name: name.into(),
        description: desc.into(),
        schema,
        kind: ToolKind::Admin(Box::new(f)),
    }
}

/// 對應 `WORK_TOOLS`（宣告順序＝tools/list 順序）。
pub static WORK_TOOLS: Lazy<Vec<Tool>> = Lazy::new(|| {
    vec![
        work(
            "where_am_i",
            "我在哪：cwd 屬於哪個已登記 repo、哪些組、預設工作範圍是什麼。沒登記會告訴你怎麼登記。",
            s_(cwd_props(), &[]),
            w_where,
        ),
        work(
            "context_for",
            "開場一次拿齊這個範圍的脈絡：目標、上次交接、建議下一步、未結事項（帶 id）、\
             最近事件（帶 ref）、關係、需要注意的 repo、agent 狀態；card＝可直接閱讀的情境卡。\
             開工前先呼叫一次。",
            s_(
                cat(vec![
                    cwd_props(),
                    scope_props(),
                    vec![("card", d_(bool_t(), "是否附 markdown 情境卡（預設 true）"))],
                ]),
                &[],
            ),
            w_context,
        ),
        work(
            "next_work",
            "接下來值得做什麼（依序：逾期未結 → 上次交接的下一步 → 其他未結 → 需要注意的 repo），\
             每項都附 why。",
            s_(
                cat(vec![
                    cwd_props(),
                    scope_props(),
                    vec![("limit", d_(int_t(), "最多幾項，預設 5"))],
                ]),
                &[],
            ),
            w_next,
        ),
        work(
            "log_decision",
            "記下一個判斷或結論（為什麼這樣做、依據哪份檔）。回傳 ref，之後可引用。",
            s_(
                cat(vec![
                    vec![
                        ("text", d_(str_t(), "判斷內容；第一行寫結論，後面寫依據")),
                        (
                            "ref",
                            d_(
                                str_t(),
                                "相關事件的 ref（例 decision:2026-09-25-d1、collision:2026-09-25-a）",
                            ),
                        ),
                    ],
                    cwd_props(),
                    scope_props(),
                    cross_props(),
                ]),
                &["text"],
            ),
            w_log,
        ),
        work(
            "add_todo",
            "開一個未結事項（之後要有人處理的事）。編號自動配，沒給 due 用預設天數。",
            s_(
                cat(vec![
                    vec![
                        ("text", d_(str_t(), "要做的事，一句話")),
                        ("due", d_(str_t(), "YYYY-MM-DD，可省略")),
                    ],
                    cwd_props(),
                    scope_props(),
                    cross_props(),
                ]),
                &["text"],
            ),
            w_add_todo,
        ),
        work(
            "close_todo",
            "關掉一個未結事項（用 context_for／next_work 回傳的 id，例 #3）。",
            s_(
                cat(vec![
                    vec![
                        ("id", d_(str_t(), "未結事項編號，例 #3")),
                        ("note", d_(str_t(), "怎麼結的，一句話")),
                    ],
                    cwd_props(),
                    cross_props(),
                ]),
                &["id"],
            ),
            w_close_todo,
        ),
        work(
            "report_status",
            "回報你現在的階段，讓使用者在工作台一眼看出輪到誰：working／waiting（等使用者回應）／\
             blocked（卡住）／done。waiting 與 blocked 會進使用者的收件匣。",
            s_(
                cat(vec![
                    vec![
                        ("status", {
                            let mut v = d_(str_t(), "目前階段");
                            v["enum"] = json!(agentapi::STATUSES);
                            v
                        }),
                        ("text", d_(str_t(), "在做什麼／在等什麼／卡在哪")),
                    ],
                    cwd_props(),
                    scope_props(),
                ]),
                &["status"],
            ),
            w_status,
        ),
        work(
            "handoff",
            "收尾交接（每次會話結束前必呼叫）：完成了什麼、下次從哪接；remaining 每項各開一個未結事項\
             （只放新的，已經有 id 的別重複）。下一個 agent 的 context_for／next_work 會讀到它。",
            s_(
                cat(vec![
                    vec![
                        ("done", d_(str_t(), "這次完成了什麼（成果、改了哪些檔）")),
                        ("next_step", d_(str_t(), "下次從哪裡接，一句話")),
                        ("remaining", d_(arr_t(), "還沒做完、需要追蹤的新事項")),
                    ],
                    cwd_props(),
                    scope_props(),
                    cross_props(),
                ]),
                &["done", "next_step"],
            ),
            w_handoff,
        ),
        work(
            "search_knowledge",
            "找其他 repo 的相關內容（跨 repo 參考的第一步）：給一段文字（問題、想法、卡片內容），\
             回傳 files（md，附行號片段、why、可直接給 read_from 的 ref）、code（git grep 命中的程式碼）\
             與 repo 排名。跟你所在 repo 有關係的 repo、以及它們宣告的 exports 會加分。預設搜全部已登記 repo。",
            s_(
                cat(vec![
                    vec![
                        ("query", d_(str_t(), "要比對的文字")),
                        (
                            "everywhere",
                            d_(bool_t(), "true（預設）＝搜全部 repo；false＝只搜主場範圍"),
                        ),
                        ("code", d_(bool_t(), "是否也搜程式碼（預設 true）")),
                        ("limit", d_(int_t(), "最多回幾個檔，預設 8")),
                    ],
                    cwd_props(),
                    scope_props(),
                ]),
                &["query"],
            ),
            w_search,
        ),
        work(
            "read_from",
            "讀別的 repo 的檔案或目錄。ref 用名字不用路徑：`<repo>:<export>/子路徑`（export 見開場的鄰居地圖）、\
             `<repo>:<相對路徑>`，或 search_knowledge 回傳的 ref。回傳內容＋commit；目錄回清單。\
             讀了會記一筆引用（之後上游改了會提醒）。讀組外 repo 不用先問，寫入才要。",
            s_(
                cat(vec![
                    vec![
                        (
                            "ref",
                            d_(
                                str_t(),
                                "例 notes:書摘/Chapter03.md、impl:src/app.py、impl（只給 repo＝看根目錄）",
                            ),
                        ),
                        ("lines", d_(str_t(), "只讀某段，例 120-180")),
                    ],
                    cwd_props(),
                ]),
                &["ref"],
            ),
            w_read,
        ),
        work(
            "ask_repo",
            "在另一個 repo 開一個唯讀 agent 回答問題（它會照那個 repo 自己的 AGENTS.md 慣例找資料），\
             只回結論＋引用，不把檔案塞進你的 context。慢（幾十秒）而且花錢——需要整理歸納時才用；\
             找單一檔案用 search_knowledge＋read_from。",
            s_(
                cat(vec![
                    vec![
                        ("repo", d_(str_t(), "repo id")),
                        ("question", d_(str_t(), "要問的問題，寫清楚要什麼")),
                        (
                            "provider",
                            d_(str_t(), "claude（預設看 config provider.ask）或 mock"),
                        ),
                    ],
                    cwd_props(),
                ]),
                &["repo", "question"],
            ),
            w_ask,
        ),
        work(
            "repo_status",
            "repo 狀態卡：最近變化與改動集中的目錄、沒 commit 的檔與放了幾天、正在跑的 dev server\
             （啟動後有沒有檔案又改過＝要不要重開）、最後交接、agent 文件裡失效的跨 repo 路徑、上游變動。",
            s_(
                cat(vec![
                    vec![
                        ("repo", d_(str_t(), "repo id；省略＝你所在的 repo")),
                        (
                            "since",
                            d_(str_t(), "7d／30d 或 commit hash；省略＝最近 5 則"),
                        ),
                    ],
                    cwd_props(),
                ]),
                &[],
            ),
            w_repo_status,
        ),
    ]
});

/// 對應 `ADMIN_TOOLS`：手寫的維護工具＋操作表（`ops::ops()`）自動變成的寫入工具。
pub static ADMIN_TOOLS: Lazy<Vec<Tool>> = Lazy::new(|| {
    let mut v = vec![
        admin(
            "registry_list",
            "列出所有已登記 repo（id／類型／tier／路徑）",
            s_(vec![], &[]),
            t_registry_list,
        ),
        admin(
            "registry_audit",
            "稽核 registry：tier 與實際活動不符、路徑失效、目錄裡有未登記 repo",
            s_(vec![("scan_dirs", arr_t())], &[]),
            t_registry_audit,
        ),
        admin(
            "registry_scan",
            "掃一個目錄找未登記的 git repo（apply=true 才真的登記）",
            s_(vec![("dir", str_t()), ("apply", bool_t())], &["dir"]),
            t_registry_scan,
        ),
        admin(
            "registry_relations",
            "讀 repo 間關係（給 id＝站在該 repo 兩向讀；省略＝全部）",
            s_(vec![("id", str_t())], &[]),
            t_registry_relations,
        ),
        admin(
            "refs_check",
            "跨 repo 參考檢查：agent 文件裡失效的跨 repo 路徑、引用過的上游有變、建議宣告的 export",
            s_(vec![], &[]),
            t_refs_check,
        ),
        admin(
            "group_context",
            "組情境卡（markdown）：成員與角色、脈動、本組未結、最近事件、角色說明",
            s_(scope_props(), &[]),
            t_group_context,
        ),
        admin(
            "group_list",
            "列出所有組與成員",
            s_(vec![], &[]),
            t_group_list,
        ),
        admin(
            "collect",
            "採集 git 狀態（dirty、末次 commit、ahead/behind、worktree）",
            s_(scope_props(), &[]),
            t_collect,
        ),
        admin(
            "pack",
            "把範圍內的 md 文件打包成一份全文（含洩密過濾）",
            s_(cat(vec![scope_props(), vec![("budget", int_t())]]), &[]),
            t_pack,
        ),
        admin(
            "pack_estimate",
            "只估算範圍內 md 文件的 token 成本，不打包",
            s_(scope_props(), &[]),
            t_pack_estimate,
        ),
        admin(
            "brief",
            "產生早晨簡報並存檔（會落一筆 presented 事件）",
            s_(scope_props(), &[]),
            t_brief,
        ),
        admin(
            "spine_append",
            "直接寫一筆原始脊椎事件（進階；一般請用 log_decision／add_todo）。\
             type: presented/chosen/decision/suggestion/collision/open-loop/outcome；\
             kv: #編號 或 group:/repo:/ref:/due:/id: token",
            s_(
                vec![
                    ("type", str_t()),
                    ("source", str_t()),
                    ("kv", arr_t()),
                    ("body", str_t()),
                ],
                &["type", "source"],
            ),
            t_spine_append,
        ),
        admin(
            "spine_query",
            "查脊椎事件（依型別／日期／組）",
            s_(
                vec![
                    ("type", str_t()),
                    ("date", str_t()),
                    ("today", bool_t()),
                    ("group", str_t()),
                    ("verbose", bool_t()),
                ],
                &[],
            ),
            t_spine_query,
        ),
        admin(
            "spine_stats",
            "量測：靈感命中率＋repo 存活率",
            s_(vec![], &[]),
            t_spine_stats,
        ),
        admin(
            "open_loops",
            "未結事項（可依 group／repos 過濾）",
            s_(scope_props(), &[]),
            t_open_loops,
        ),
        admin(
            "spine_lint",
            "脊椎衛生檢查：ref 斷鏈、逾期未結、碰撞沒回程、dead-letter 積壓",
            s_(vec![], &[]),
            t_spine_lint,
        ),
        admin(
            "unread",
            "使用者的未讀事件（ack=true 會把使用者的收件匣全部標已讀，別隨便用）",
            s_(vec![("ack", bool_t())], &[]),
            t_unread,
        ),
        admin(
            "collide_submit",
            "把一個想法丟去跟範圍內的 repo 比對（wait=true 同步等判定）",
            s_(
                cat(vec![
                    vec![("idea", str_t())],
                    scope_props(),
                    vec![("wait", bool_t()), ("provider", str_t())],
                ]),
                &["idea"],
            ),
            t_collide_submit,
        ),
        admin(
            "route",
            "把一段文字落檔（dest: group:<g> | repo:<id> | incubator）",
            s_(
                vec![("text", str_t()), ("dest", str_t()), ("title", str_t())],
                &["text", "dest"],
            ),
            t_route,
        ),
        admin(
            "upstream_check",
            "查 external repo 的上游新 release／commit",
            s_(scope_props(), &[]),
            t_upstream_check,
        ),
        admin(
            "digest_run",
            "跑增量摘要（便宜模型）",
            s_(cat(vec![scope_props(), vec![("provider", str_t())]]), &[]),
            t_digest_run,
        ),
        admin(
            "spawn",
            "把 incubator 的想法升格成新 repo（登記＋血統）",
            s_(
                vec![
                    ("id", str_t()),
                    ("path", str_t()),
                    ("from_incubator", str_t()),
                    ("origin", str_t()),
                ],
                &["id", "path"],
            ),
            t_spawn,
        ),
    ];
    // 管理類寫入動作一律來自操作表：參數、留痕、錯誤訊息跟 CLI／工作台同一份
    for o in ops::ops() {
        let name = o.name;
        let tool = admin(name, o.summary, o.schema(), move |d, a| {
            Ok(ops::call(d, name, Value::Object(a.clone()), Via::Mcp)?.1)
        });
        match v.iter_mut().find(|t| t.name == name) {
            Some(slot) => *slot = tool,
            None => v.push(tool),
        }
    }
    v
});

/// 對應 `_admin_enabled`：admin=None＝看 config `mcp.admin`。
fn admin_enabled(spine_dir: &Path, admin: Option<bool>) -> bool {
    if let Some(a) = admin {
        return a;
    }
    config::load(spine_dir)
        .ok()
        .and_then(|c| c.get("mcp.admin").map(truthy))
        .unwrap_or(false)
}

/// 對應 `tool_table`：工作層＋（管理層開時）管理層。
pub fn tool_table(spine_dir: &Path, admin: Option<bool>) -> Vec<&'static Tool> {
    let mut out: Vec<&'static Tool> = WORK_TOOLS.iter().collect();
    if admin_enabled(spine_dir, admin) {
        out.extend(ADMIN_TOOLS.iter());
    }
    out
}

fn text_of(v: &Value) -> String {
    pyjson::dumps_indent(v)
}

/// 工作層錯誤 → `{ok:false,error,message,hint?}`：AgentError（Rejected）照原 code；
/// 其他例外（Python 的 `except Exception`）一律 `failed`。
fn work_error(e: &Error) -> Value {
    match e {
        Error::Rejected { .. } => agentapi::error_dict(e),
        other => json!({"ok": false, "error": "failed", "message": other.to_string()}),
    }
}

/// 對應 `handle_message`：單則 JSON-RPC → 回應（notification 回 None）。
/// admin=None＝看 config `mcp.admin`；cwd＝推主場用的目錄（預設 `$CTX_CWD` 或 server 行程的 cwd）。
pub fn handle_message(
    spine_dir: &Path,
    msg: &Value,
    admin: Option<bool>,
    cwd: Option<&Path>,
) -> Option<Value> {
    let mid = msg.get("id").cloned().unwrap_or(Value::Null);
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    if method.starts_with("notifications/") {
        return None;
    }
    let ok = |result: Value| json!({"jsonrpc": "2.0", "id": mid, "result": result});
    if method == "initialize" {
        return Some(ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "repo_context", "version": "0.2.0"},
            "instructions": "repo-context：跨 repo 的工作脈絡與留痕。開工先 context_for，\
                做了判斷 log_decision、發現待辦 add_todo、等人或卡住 report_status，\
                收尾一定 handoff。範圍依 cwd 推出，寫到別組會被拒，要先問使用者。",
        })));
    }
    if method == "ping" {
        return Some(ok(json!({})));
    }
    let table = tool_table(spine_dir, admin);
    if method == "tools/list" {
        let tools: Vec<Value> = table
            .iter()
            .map(|t| json!({"name": t.name, "description": t.description, "inputSchema": t.schema}))
            .collect();
        return Some(ok(json!({ "tools": tools })));
    }
    if method == "tools/call" {
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let args: Args = match params.get("arguments") {
            Some(Value::Object(m)) => m.clone(),
            _ => Map::new(),
        };
        let Some(tool) = table.iter().find(|t| t.name == name) else {
            let shown = params
                .get("name")
                .map(pystr::py_str)
                .unwrap_or_else(|| "None".into());
            let hint = if ADMIN_TOOLS.iter().any(|t| t.name == name) {
                "（這是管理層工具，需用 `ctx mcp --admin` 啟動）"
            } else {
                ""
            };
            return Some(json!({"jsonrpc": "2.0", "id": mid,
                "error": {"code": -32602, "message": format!("未知 tool: {shown}{hint}")}}));
        };
        let here: PathBuf = match cwd {
            Some(c) => c.to_path_buf(),
            None => std::env::var_os("CTX_CWD")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from(".")),
        };
        let (text, is_error) = match &tool.kind {
            ToolKind::Work(f) => match f(spine_dir, &args, &here) {
                Ok(v) => (text_of(&v), false),
                Err(e) => (text_of(&work_error(&e)), true),
            },
            // 錯誤原樣回 agent 自我修正（含 validator 拒寫）
            ToolKind::Admin(f) => match f(spine_dir, &args) {
                Ok(s) => (s, false),
                Err(e) => (e.to_string(), true),
            },
        };
        return Some(ok(json!({
            "content": [{"type": "text", "text": text}],
            "isError": is_error,
        })));
    }
    Some(json!({"jsonrpc": "2.0", "id": mid,
        "error": {"code": -32601, "message": format!("未知 method: {method}")}}))
}

/// 對應 `serve`：stdio loop，一行一則 JSON-RPC。
pub fn serve(spine_dir: &Path, admin: Option<bool>) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if !msg.is_object() {
            continue;
        }
        if let Some(resp) = handle_message(spine_dir, &msg, admin, None) {
            let mut out = stdout.lock();
            writeln!(out, "{}", pyjson::dumps(&resp))?;
            out.flush()?;
        }
    }
    Ok(())
}
