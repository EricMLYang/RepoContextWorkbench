//! Agent 介面（2026-09-25 Agent 友善輪）——agent 是主要使用者，不是被請進工作台的客人。
//! 對應 `repo_context/agentapi.py`。
//!
//! 之前的問題：agent 只有從工作台開、cwd 在 spine 時才拿得到脈絡；MCP 工具照規格編號切，不照
//! agent 的工作切；輸出是給人看的中文表格，沒有可引用的 id；範圍只是 prompt 裡的約定。
//!
//! 這一層把「agent 一次工作」需要的動作收成幾個函數，全部回 JSON（`serde_json::Value`，鍵序同 Python）：
//! where_am_i → context_for → next_work → log_decision／add_todo／close_todo → report_status → handoff；
//! 另有 search_knowledge、resolve_ref／read_from／ask_repo／repo_status（跨 repo 參考）。
//! MCP 工作層、CLI（--json）、Claude Code hooks 都呼叫這裡——同一份邏輯，三個入口。
//!
//! 範圍紀律（從約定變成機制）：沒指定範圍＝依 cwd 推出「主場」；查詢預設只回主場範圍；
//! 寫入主場以外的範圍會被拒（`Error::Rejected{code:"cross_scope"}`），要先問使用者、
//! 確認後帶 `cross_scope_ok=true` 才寫得進去。
//!
//! 錯誤：Python `AgentError(code, message, hint)` → [`Error::Rejected`]；
//! 入口用 [`error_dict`] 轉成 `{ok:false,error,message,hint}`。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::Local;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::crossref::{self, Drift, Neighbor, RunText, MAX_READ_CHARS};
use crate::knowledge;
use crate::pack::RepoRef;
use crate::pystr::{expanduser, repr, resolve as py_resolve, splitlines};
use crate::registry::{self, Repo};
use crate::repocard::{self, ProcProbe};
use crate::ruminate::{self, Link};
use crate::spine::{self, Event, EventType};
use crate::status::{self, AgentStatus, StatusRecord};
use crate::{brief, collect, config, context, summary, Error, Result};

pub use crate::status::{get_status, STATUSES};

/// 對應 `AgentError(code, message, hint)` 的建構。
pub fn agent_error(code: &str, message: impl Into<String>, hint: Option<&str>) -> Error {
    Error::rejected(code, message, hint.map(str::to_string))
}

/// 對應 `AgentError.to_dict`：`{ok:false, error, message, hint?}`。
/// 非 Rejected 的錯誤（Python 會冒成一般例外）以 `error` 依種類標示：Invalid／Validation＝bad_request，其餘＝internal。
pub fn error_dict(e: &Error) -> Value {
    let (code, message, hint) = match e {
        Error::Rejected {
            code,
            message,
            hint,
        } => (code.as_str(), message.clone(), hint.clone()),
        Error::Invalid(m) | Error::Validation(m) => ("bad_request", m.clone(), None),
        other => ("internal", other.to_string(), None),
    };
    let mut out = Map::new();
    out.insert("ok".into(), Value::Bool(false));
    out.insert("error".into(), Value::String(code.into()));
    out.insert("message".into(), Value::String(message));
    if let Some(h) = hint.filter(|h| !h.is_empty()) {
        out.insert("hint".into(), Value::String(h));
    }
    Value::Object(out)
}

/// 呼叫端指定的範圍：cwd（推主場）＋可選的 group／repos（明確範圍）。
/// 對應 Python 各函數的 `cwd=None, group=None, repos=None` 參數。
#[derive(Debug, Clone, Default)]
pub struct Target {
    /// None＝行程目前目錄。
    pub cwd: Option<PathBuf>,
    pub group: Option<String>,
    /// Python 也收逗號字串——呼叫端用 [`registry::split_ids`] 先拆；空字串項會被略過。
    pub repos: Option<Vec<String>>,
}

impl Target {
    /// 只給 cwd（主場範圍）。
    pub fn at(cwd: impl Into<PathBuf>) -> Self {
        Target {
            cwd: Some(cwd.into()),
            ..Default::default()
        }
    }
    pub fn with_group(mut self, g: impl Into<String>) -> Self {
        self.group = Some(g.into());
        self
    }
    pub fn with_repos<S: AsRef<str>>(mut self, ids: &[S]) -> Self {
        self.repos = Some(ids.iter().map(|s| s.as_ref().to_string()).collect());
        self
    }
}

// ---------------------------------------------------------------- 定位與範圍

fn resolve_path(p: &Path) -> PathBuf {
    py_resolve(&expanduser(p))
}

/// where_am_i 裡的 repo 摘要。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WhereRepo {
    pub id: String,
    /// registry 裡登記的原字串
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub tier: String,
}

/// 對應 `where_am_i` 回傳的 dict（鍵序同 Python）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Where {
    pub cwd: String,
    pub spine: String,
    pub registered: bool,
    pub in_spine: bool,
    pub repo: Option<WhereRepo>,
    pub groups: Vec<String>,
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Where {
    fn repo_id(&self) -> Option<&str> {
        self.repo.as_ref().map(|r| r.id.as_str())
    }
}

/// 對應 `agentapi.where_am_i`：cwd → 所在 repo、所屬組、主場範圍。認不出來也回結果（registered=false＋hint）。
pub fn where_am_i(spine_dir: &Path, cwd: Option<&Path>) -> Result<Where> {
    let here = match cwd {
        Some(c) => resolve_path(c),
        None => resolve_path(&std::env::current_dir()?),
    };
    let data = registry::load(spine_dir)?;
    let mut best: Option<(&Repo, usize)> = None;
    for r in &data.repos {
        let root = resolve_path(Path::new(&r.path_str()));
        if here.starts_with(&root) {
            let len = root.to_string_lossy().len();
            if best.is_none_or(|(_, l)| len > l) {
                best = Some((r, len));
            }
        }
    }
    let best = best.map(|(r, _)| r);
    let groups: Vec<String> = match best {
        Some(b) => {
            let id = b.id();
            data.groups
                .iter()
                .filter(|g| g.members().contains(&id))
                .map(|g| g.name())
                .collect()
        }
        None => Vec::new(),
    };
    let sp = resolve_path(spine_dir);
    let in_spine = here.starts_with(&sp);
    let mut out = Where {
        cwd: here.to_string_lossy().into_owned(),
        spine: sp.to_string_lossy().into_owned(),
        registered: best.is_some(),
        in_spine,
        repo: None,
        group: groups.first().cloned(),
        groups,
        note: None,
        hint: None,
    };
    if let Some(b) = best {
        out.repo = Some(WhereRepo {
            id: b.id(),
            path: b.path_str(),
            kind: b.kind().unwrap_or_else(|| "mine".into()),
            tier: b.tier_str().unwrap_or_else(|| "active".into()),
        });
        if out.groups.len() > 1 {
            out.note = Some(format!(
                "此 repo 屬於多個組（{}），預設用第一個；要換組就在呼叫時帶 group。",
                out.groups.join("、")
            ));
        } else if out.groups.is_empty() {
            out.note = Some("此 repo 還沒編進任何組，範圍只有它自己。".into());
        }
    } else if in_spine {
        out.note = Some("cwd 在 spine repo 裡：沒有主場範圍，查詢預設是全部 repo。".into());
    } else {
        out.hint = Some(format!(
            "這個目錄沒登記在 registry。可請使用者跑 `ctx registry add <id> {}`，或在工作台首頁掃描登記。",
            out.cwd
        ));
    }
    Ok(out)
}

/// 範圍（對應 Python `_scope_of` 回傳的 dict）。`repos=None`＝全部。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Scope {
    pub group: Option<String>,
    pub repos: Option<Vec<String>>,
    pub label: String,
    #[serde(skip)]
    pub explicit: bool,
}

impl Scope {
    fn ids(&self) -> &[String] {
        self.repos.as_deref().unwrap_or(&[])
    }
    /// 對應 Python `_in`：group 與 repos 都沒有＝全部。
    fn contains(&self, ev: &Event) -> bool {
        if self.group.is_none() && self.repos.is_none() {
            return true;
        }
        spine::in_scope(ev, self.group.as_deref(), self.ids())
    }
}

fn scope_of(spine_dir: &Path, w: &Where, t: &Target) -> Result<Scope> {
    let repos: Vec<String> = t
        .repos
        .iter()
        .flatten()
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .collect();
    if !repos.is_empty() {
        let (gname, entries) = registry::resolve_group(spine_dir, None, Some(&repos))?;
        return Ok(Scope {
            group: None,
            repos: Some(entries.iter().map(Repo::id).collect()),
            label: gname,
            explicit: true,
        });
    }
    if let Some(g) = t.group.as_deref().filter(|g| !g.is_empty()) {
        let (_, entries) = registry::resolve_group(spine_dir, Some(g), None)?;
        return Ok(Scope {
            group: Some(g.into()),
            repos: Some(entries.iter().map(Repo::id).collect()),
            label: g.into(),
            explicit: true,
        });
    }
    if let Some(g) = &w.group {
        let (_, entries) = registry::resolve_group(spine_dir, Some(g), None)?;
        return Ok(Scope {
            group: Some(g.clone()),
            repos: Some(entries.iter().map(Repo::id).collect()),
            label: g.clone(),
            explicit: false,
        });
    }
    if let Some(rid) = w.repo_id() {
        return Ok(Scope {
            group: None,
            repos: Some(vec![rid.to_string()]),
            label: rid.to_string(),
            explicit: false,
        });
    }
    Ok(Scope {
        group: None,
        repos: None,
        label: "全部".into(),
        explicit: false,
    })
}

/// 對應 `_home_ids`：主場涵蓋的 repo id（所在 repo 所屬全部組的成員；沒組＝它自己）。
fn home_ids(spine_dir: &Path, w: &Where) -> Result<Option<HashSet<String>>> {
    let Some(rid) = w.repo_id() else {
        return Ok(None);
    };
    let mut ids: HashSet<String> = HashSet::from([rid.to_string()]);
    for g in registry::load(spine_dir)?.groups {
        if w.groups.contains(&g.name()) {
            ids.extend(g.members());
        }
    }
    Ok(Some(ids))
}

/// 對應 `_check_write_scope`：寫入主場以外＝拒寫（除非使用者已確認）。主場不明時不擋。
fn check_write_scope(spine_dir: &Path, w: &Where, s: &Scope, cross_scope_ok: bool) -> Result<()> {
    if cross_scope_ok || !s.explicit {
        return Ok(());
    }
    let Some(home) = home_ids(spine_dir, w)? else {
        return Ok(());
    };
    if s.group.as_ref().is_some_and(|g| w.groups.contains(g)) {
        return Ok(());
    }
    if let Some(r) = &s.repos {
        if !r.is_empty() && r.iter().all(|x| home.contains(x)) {
            return Ok(());
        }
    }
    let groups = if w.groups.is_empty() {
        "無組".to_string()
    } else {
        w.groups.join("、")
    };
    Err(agent_error(
        "cross_scope",
        format!(
            "要寫入的範圍「{}」不在你的主場（{}／{groups}）。",
            s.label,
            w.repo_id().unwrap_or_default()
        ),
        Some("先問使用者是否要跨組留痕；使用者同意後再帶 cross_scope_ok=true 重試。"),
    ))
}

/// 對應 `_tokens_for`：寫入事件的範圍 token。
fn tokens_for(s: &Scope, repo_id: Option<&str>) -> Vec<String> {
    let mut toks = Vec::new();
    if let Some(g) = &s.group {
        toks.push(format!("group:{g}"));
    }
    match repo_id {
        Some(rid) if s.repos.is_none() || s.ids().iter().any(|x| x == rid) => {
            toks.push(format!("repo:{rid}"))
        }
        _ => {
            if let Some([only]) = s.repos.as_deref() {
                toks.push(format!("repo:{only}"));
            }
        }
    }
    toks
}

fn entries_of(spine_dir: &Path, s: &Scope) -> Result<Vec<Repo>> {
    match &s.repos {
        None => Ok(registry::load(spine_dir)?.repos),
        Some(ids) => ids
            .iter()
            .map(|i| registry::get_repo(spine_dir, i))
            .collect(),
    }
}

fn scope_json(s: &Scope) -> Value {
    json!({"group": s.group, "repos": s.repos, "label": s.label})
}

// ---------------------------------------------------------------- 事件的 JSON 形狀

/// 對應 `_first`：body 第一行；沒有 body＝tokens 以空白串。
fn first(ev: &Event) -> String {
    if ev.body.is_empty() {
        ev.tokens.join(" ")
    } else {
        splitlines(&ev.body)
            .first()
            .map(|s| s.to_string())
            .unwrap_or_default()
    }
}

fn kv_opt(ev: &Event, k: &str) -> Option<String> {
    ev.kv(k).map(str::to_string)
}

/// 對應 `agentapi.event_ref`：可放進 `ref:` token 的鍵——有 id 用 id，沒有用 HH:MM。
pub fn event_ref(ev: &Event) -> String {
    let key = ev.kv("id").filter(|s| !s.is_empty()).unwrap_or(&ev.time);
    format!("{}:{key}", ev.etype.as_str())
}

/// 對應 `agentapi.event_dict`。
pub fn event_dict(ev: &Event) -> Value {
    json!({
        "ref": event_ref(ev), "date": ev.date, "time": ev.time,
        "type": ev.etype.as_str(), "source": ev.source, "group": kv_opt(ev, "group"),
        "repo": kv_opt(ev, "repo"), "text": first(ev), "body": ev.body,
    })
}

fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// 對應 `agentapi.loop_dict`。`today` 預設今天。
pub fn loop_dict(ev: &Event, today_s: Option<&str>) -> Value {
    let t = today_s.map(str::to_string).unwrap_or_else(today);
    let due = ev.kv("due").filter(|d| !d.is_empty());
    json!({
        "id": ev.num(), "text": first(ev), "due": due,
        "overdue": due.is_some_and(|d| d < t.as_str()), "group": kv_opt(ev, "group"),
        "repo": kv_opt(ev, "repo"), "opened": format!("{} {}", ev.date, ev.time),
        "source": ev.source,
    })
}

fn open_loops(spine_dir: &Path, s: &Scope) -> Result<Vec<Event>> {
    Ok(spine::open_loops(spine_dir)?
        .into_iter()
        .filter(|ev| s.contains(ev))
        .collect())
}

fn is_handoff(ev: &Event) -> bool {
    ev.etype == EventType::Decision && ev.body.starts_with("交接")
}

/// 對應 `_last_handoff`：範圍內最後一筆「交接：…」decision 的欄位。
fn last_handoff(events: &[Event], s: &Scope) -> Option<Value> {
    let best = events.iter().rfind(|ev| is_handoff(ev) && s.contains(ev))?;
    let mut fields: Vec<(String, String)> = Vec::new();
    for ln in splitlines(&best.body).into_iter().skip(1) {
        if let Some((k, v)) = ln.split_once('：') {
            let k = k.trim().to_string();
            let v = v.trim().to_string();
            match fields.iter_mut().find(|(kk, _)| *kk == k) {
                Some(slot) => slot.1 = v,
                None => fields.push((k, v)),
            }
        }
    }
    let field = |k: &str| {
        fields
            .iter()
            .find(|(kk, _)| kk == k)
            .map(|(_, v)| v.clone())
    };
    let summary: String = first(best).chars().skip(3).collect();
    Some(json!({
        "ref": event_ref(best), "at": format!("{} {}", best.date, best.time),
        "source": best.source, "summary": summary.trim(),
        "done": field("完成"), "remaining": field("剩下"),
        "next_step": field("下次從這裡接"),
    }))
}

// ---------------------------------------------------------------- 讀

/// [`context_for_full`] 的結果：JSON（給 MCP／CLI）＋開場注入要用的型別化片段。
#[derive(Debug, Clone)]
pub struct ContextFor {
    pub value: Value,
    pub neighbors: Vec<Neighbor>,
    pub drift: Vec<Drift>,
    pub fresh: Vec<Link>,
}

/// 對應 `agentapi.context_for`：開場一次拿齊——我在哪、範圍、目標、上次交接、下一步、未結、
/// 最近事件、agent 狀態。
pub fn context_for(spine_dir: &Path, t: &Target, card: bool) -> Result<Value> {
    Ok(context_for_full(spine_dir, t, card)?.value)
}

/// 同 [`context_for`]，另外回傳型別化的 neighbors／drift／fresh（hooks 渲染用）。
pub fn context_for_full(spine_dir: &Path, t: &Target, card: bool) -> Result<ContextFor> {
    let w = where_am_i(spine_dir, t.cwd.as_deref())?;
    let s = scope_of(spine_dir, &w, t)?;
    let entries = entries_of(spine_dir, &s)?;
    let specs: Vec<collect::RepoSpec> = entries.iter().map(collect::RepoSpec::from).collect();
    let states = collect::collect_group(&specs, Some(spine_dir));
    let snoozed = brief::snoozed(spine_dir, None)?;
    let th = config::load(spine_dir)?.thresholds();
    let cards: Vec<brief::QuestionCard> = brief::build_question_cards(&states, &th)
        .into_iter()
        .filter(|c| !snoozed.contains(&(c.repo.clone(), c.qkind.as_str().to_string())))
        .collect();
    let summ = summary::build_summary(
        spine_dir,
        s.group.as_deref(),
        if s.group.is_some() {
            None
        } else {
            s.repos.as_deref()
        },
        Some(states.clone()),
        Some(cards.clone()),
        None,
    )?;
    let all = spine::iter_events(spine_dir)?;
    let evs: Vec<&Event> = all
        .iter()
        .filter(|ev| ev.etype != EventType::Presented && s.contains(ev))
        .collect();
    let recent: Vec<Value> = evs.iter().rev().take(8).map(|e| event_dict(e)).collect();
    let ids: Vec<String> = entries.iter().map(Repo::id).collect();
    let rid = w.repo_id().map(str::to_string);
    let neighbors = match &rid {
        Some(r) => crossref::neighbors(spine_dir, r)?,
        None => Vec::new(),
    };
    let drift = match &rid {
        Some(r) => crossref::drift(spine_dir, Some(r), None)?,
        None => Vec::new(),
    };
    let fresh: Vec<Link> = match &s.group {
        Some(g) => ruminate::fresh(spine_dir, g, false)?
            .into_iter()
            .take(5)
            .collect(),
        None => Vec::new(),
    };
    let attention: Vec<Value> = cards
        .iter()
        .map(|c| json!({"repo": c.repo, "kind": c.qkind.name(), "text": c.title}))
        .collect();
    let mut out = json!({
        "ok": true,
        "where": w,
        "scope": scope_json(&s),
        "goal": summ.goal, "goal_gap": summ.goal_gap,
        "last_progress": summ.progress,
        "last_handoff": last_handoff(&all, &s),
        "next": summ.next, "next_gap": summ.next_gap,
        "changes": summ.changes,
        "open_loops": open_loops(spine_dir, &s)?.iter().map(|e| loop_dict(e, None)).collect::<Vec<_>>(),
        "recent_events": recent,
        "relations": registry::relation_lines(spine_dir, &ids)?,
        "pulse": collect::pulse_line(&collect::group_pulse(&states)),
        "agent_status": status::get_status(spine_dir, s.group.as_deref(), s.repos.as_deref()),
        "attention": attention,
        "neighbors": neighbors,
        "drift": drift,
        "fresh": fresh,
    });
    if card {
        let text = context::build_context(
            spine_dir,
            s.group.as_deref(),
            if s.group.is_some() {
                None
            } else {
                s.repos.as_deref()
            },
            Some(states),
            None,
        )?;
        out["card"] = Value::String(text);
    }
    Ok(ContextFor {
        value: out,
        neighbors,
        drift,
        fresh,
    })
}

/// 對應 `agentapi.next_work`：接下來值得做的事，依序：逾期未結 → 上次交接的下一步 → 有 due 的未結 →
/// 其他未結 → 需要注意的 repo。每項都帶 why。Python 預設 `limit=5`。
pub fn next_work(spine_dir: &Path, t: &Target, limit: usize) -> Result<Value> {
    let w = where_am_i(spine_dir, t.cwd.as_deref())?;
    let s = scope_of(spine_dir, &w, t)?;
    let td = today();
    let loops: Vec<Value> = open_loops(spine_dir, &s)?
        .iter()
        .map(|e| loop_dict(e, Some(&td)))
        .collect();
    let due_of = |lp: &Value| lp["due"].as_str().map(str::to_string);
    let todo_item = |lp: &Value, why: String| json!({"kind": "todo", "id": lp["id"], "text": lp["text"], "due": lp["due"], "why": why});
    let mut items: Vec<Value> = Vec::new();
    let mut overdue: Vec<&Value> = loops.iter().filter(|lp| lp["overdue"] == true).collect();
    overdue.sort_by_key(|lp| due_of(lp));
    for lp in overdue {
        let why = format!("未結事項已逾期（due {}）", due_of(lp).unwrap_or_default());
        items.push(todo_item(lp, why));
    }
    let all = spine::iter_events(spine_dir)?;
    if let Some(ho) = last_handoff(&all, &s) {
        if let Some(ns) = ho["next_step"].as_str().filter(|x| !x.is_empty()) {
            items.push(json!({
                "kind": "handoff", "ref": ho["ref"], "text": ns,
                "why": format!("上次交接（{}，{}）留下的下一步",
                               ho["at"].as_str().unwrap_or(""), ho["source"].as_str().unwrap_or("")),
            }));
        }
    }
    let mut rest: Vec<&Value> = loops.iter().filter(|lp| lp["overdue"] != true).collect();
    rest.sort_by_key(|lp| {
        let d = due_of(lp);
        (d.is_none(), d.unwrap_or_default())
    });
    for lp in rest {
        let why = match due_of(lp) {
            Some(d) => format!("未結事項，due {d}"),
            None => "未結事項（沒有 due）".to_string(),
        };
        items.push(todo_item(lp, why));
    }
    if items.len() < limit {
        let specs: Vec<collect::RepoSpec> = entries_of(spine_dir, &s)?
            .iter()
            .map(collect::RepoSpec::from)
            .collect();
        let states = collect::collect_group(&specs, Some(spine_dir));
        let snoozed = brief::snoozed(spine_dir, None)?;
        let th = config::load(spine_dir)?.thresholds();
        for c in brief::build_question_cards(&states, &th) {
            if snoozed.contains(&(c.repo.clone(), c.qkind.as_str().to_string())) {
                continue;
            }
            items.push(json!({
                "kind": "attention", "repo": c.repo, "text": c.title,
                "why": format!("採集閾值：{}", c.qkind.name()),
            }));
        }
    }
    let goal = match &s.group {
        Some(g) => registry::get_group(spine_dir, g)
            .ok()
            .and_then(|gr| gr.goal()),
        None => None,
    };
    let empty = items.is_empty();
    items.truncate(limit);
    let mut out = json!({"ok": true, "scope": scope_json(&s), "goal": goal, "items": items});
    if empty {
        out["note"] = "沒有逾期或未結事項，也沒有觸發提醒的 repo；下一步由使用者決定。".into();
    }
    Ok(out)
}

/// 對應 `agentapi.fresh_links`：反芻推薦——上次反芻後別組新進的知識／判斷，跟主場手上工作強相關的。
pub fn fresh_links(spine_dir: &Path, t: &Target, include_seen: bool) -> Result<Value> {
    let w = where_am_i(spine_dir, t.cwd.as_deref())?;
    let s = scope_of(spine_dir, &w, t)?;
    let Some(g) = &s.group else {
        return Ok(json!({
            "ok": true, "scope": s.label, "links": [],
            "note": "反芻以組為單位；所在 repo 不屬於任何組（或是臨時範圍）。",
        }));
    };
    Ok(json!({
        "ok": true, "scope": s.label,
        "links": ruminate::fresh(spine_dir, g, include_seen)?,
        "last_run": ruminate::load_state(spine_dir).last_run,
    }))
}

/// [`search_knowledge`] 的選項（Python 預設 everywhere=true, limit=8, exclude_paths=(), code=true）。
#[derive(Debug, Clone)]
pub struct SearchOpts {
    pub everywhere: bool,
    pub limit: usize,
    pub exclude_paths: Vec<PathBuf>,
    pub code: bool,
}

impl Default for SearchOpts {
    fn default() -> Self {
        SearchOpts {
            everywhere: true,
            limit: 8,
            exclude_paths: Vec::new(),
            code: true,
        }
    }
}

/// 對應 `agentapi.search_knowledge`：知識 ↔ repo 相關度（L1 定位）。預設搜全部已登記 repo；
/// everywhere=false 只搜主場。md 走 BM25，程式碼走 git grep（code=false 可關）。結果都附 ref。
pub fn search_knowledge(
    spine_dir: &Path,
    query: &str,
    t: &Target,
    opts: &SearchOpts,
) -> Result<Value> {
    if query.trim().is_empty() {
        return Err(agent_error(
            "bad_request",
            "query 不能是空的",
            Some("給一段文字、一個想法或一段卡片內容。"),
        ));
    }
    let w = where_am_i(spine_dir, t.cwd.as_deref())?;
    let explicit = t.group.as_deref().is_some_and(|g| !g.is_empty())
        || t.repos.as_ref().is_some_and(|r| !r.is_empty());
    let (label, entries) = if opts.everywhere && !explicit {
        ("全部".to_string(), registry::load(spine_dir)?.repos)
    } else {
        let s = scope_of(spine_dir, &w, t)?;
        let e = entries_of(spine_dir, &s)?;
        (s.label, e)
    };
    let refs: Vec<RepoRef> = entries.iter().map(RepoRef::from).collect();
    let boosts = crossref::search_boosts(spine_dir, w.repo_id())?;
    let res = knowledge::search(
        &refs,
        query,
        opts.limit,
        Some(spine_dir),
        &opts.exclude_paths,
        &boosts,
    );
    let mut out = Map::new();
    out.insert("ok".into(), Value::Bool(true));
    out.insert("scope".into(), Value::String(label));
    if let Value::Object(m) = serde_json::to_value(&res)? {
        out.extend(m);
    }
    add_refs(out.get_mut("files"));
    if opts.code {
        let hits = crossref::code_search(&entries, query, (opts.limit / 2).max(3), 40);
        let mut v = serde_json::to_value(hits)?;
        add_refs(Some(&mut v));
        out.insert("code".into(), v);
    }
    Ok(Value::Object(out))
}

fn add_refs(v: Option<&mut Value>) {
    if let Some(Value::Array(a)) = v {
        for f in a {
            let r = format!(
                "{}:{}",
                f["repo"].as_str().unwrap_or(""),
                f["file"].as_str().unwrap_or("")
            );
            f["ref"] = Value::String(r);
        }
    }
}

/// 對應 `_ref_call`：RefError 原樣（已是 Rejected）、其他 ValueError → bad_request。
fn ref_call<T: Serialize>(r: Result<T>) -> Result<Value> {
    match r {
        Ok(v) => Ok(serde_json::to_value(v)?),
        Err(e @ Error::Rejected { .. }) => Err(e),
        Err(Error::Invalid(m)) | Err(Error::Validation(m)) => {
            Err(agent_error("bad_request", m, None))
        }
        Err(e) => Err(e),
    }
}

/// 對應 `agentapi.resolve_ref`：名字 → 實際路徑。不記引用。
pub fn resolve_ref(spine_dir: &Path, reference: &str) -> Result<Value> {
    ref_call(crossref::resolve(spine_dir, reference))
}

/// 對應 `agentapi.read_from`：讀別的 repo 的檔案或目錄（L2），讀了會記一筆引用（主場 repo → 對方）。
pub fn read_from(
    spine_dir: &Path,
    reference: &str,
    lines: Option<&str>,
    cwd: Option<&Path>,
) -> Result<Value> {
    let w = where_am_i(spine_dir, cwd)?;
    ref_call(crossref::read(
        spine_dir,
        reference,
        w.repo_id(),
        lines,
        MAX_READ_CHARS,
    ))
}

/// 對應 `agentapi.ask_repo`：在對方 repo 開一個唯讀 agent 回答（L3）。provider＝agent provider 名。
pub fn ask_repo(
    spine_dir: &Path,
    repo: &str,
    question: &str,
    cwd: Option<&Path>,
    provider: Option<&str>,
) -> Result<Value> {
    ask_repo_with(
        spine_dir,
        repo,
        question,
        cwd,
        provider,
        &crate::agents::run_text,
    )
}

/// [`ask_repo`] 的本體，文字 agent 由呼叫端注入（測試不打真網路）。
pub fn ask_repo_with(
    spine_dir: &Path,
    repo: &str,
    question: &str,
    cwd: Option<&Path>,
    provider: Option<&str>,
    run_text: &RunText<'_>,
) -> Result<Value> {
    let w = where_am_i(spine_dir, cwd)?;
    ref_call(crossref::ask_repo_with(
        spine_dir,
        repo,
        question,
        w.repo_id(),
        provider,
        None,
        run_text,
    ))
}

/// 對應 `agentapi.repo_status`：repo 狀態卡（agent 版：不推水位線）。repo 省略＝cwd 所在 repo。
pub fn repo_status(
    spine_dir: &Path,
    repo: Option<&str>,
    cwd: Option<&Path>,
    since: Option<&str>,
) -> Result<Value> {
    let repo = match repo.filter(|r| !r.is_empty()) {
        Some(r) => r.to_string(),
        None => {
            let w = where_am_i(spine_dir, cwd)?;
            match w.repo_id() {
                Some(r) => r.to_string(),
                None => {
                    return Err(Error::rejected(
                        "bad_request",
                        "cwd 不在已登記的 repo 裡，請指定 repo",
                        w.hint.or(w.note),
                    ))
                }
            }
        }
    };
    let since = since.filter(|s| !s.is_empty()).unwrap_or("recent");
    ref_call(repocard::build(
        spine_dir,
        &repo,
        Some(since),
        false,
        None,
        ProcProbe::default(),
    ))
}

// ---------------------------------------------------------------- 寫

fn next_id(events: &[Event], date_str: &str, prefix: &str) -> String {
    let used: HashSet<&str> = events.iter().filter_map(|e| e.kv("id")).collect();
    let mut n = 1;
    while used.contains(format!("{date_str}-{prefix}{n}").as_str()) {
        n += 1;
    }
    format!("{date_str}-{prefix}{n}")
}

static LOOP_NUM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^#\d+$").expect("固定 regex"));
static REF_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-z-]+:\S+$").expect("固定 regex"));
static DATE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").expect("固定 regex"));

fn next_loop_num(events: &[Event]) -> String {
    let max = events
        .iter()
        .filter(|e| e.etype == EventType::OpenLoop)
        .flat_map(|e| e.tokens.iter())
        .filter(|t| LOOP_NUM_RE.is_match(t))
        .filter_map(|t| t[1..].parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("#{}", max + 1)
}

/// 對應 `_clean`：去頭尾空白，空的拒；內文裡的 `## ` 標題降一級（事件邊界是 `## ` 開頭行）。
fn clean(text: &str, field: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(agent_error(
            "bad_request",
            format!("{field} 不能是空的"),
            None,
        ));
    }
    Ok(splitlines(text)
        .into_iter()
        .map(|ln| {
            if ln.starts_with("## ") {
                format!("#{ln}")
            } else {
                ln.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn append(
    spine_dir: &Path,
    etype: &str,
    source: &str,
    tokens: &[String],
    body: &str,
) -> Result<Event> {
    spine::append_event(spine_dir, etype, source, tokens, body, None).map_err(|e| match e {
        Error::Validation(m) => agent_error("invalid_event", m, Some("修正內容後重試。")),
        other => other,
    })
}

/// 對應 `agentapi.log_decision`：記一筆判斷（decision）。回傳 ref，之後可以用它引用這筆。
/// Python 預設 `source="agent"`。
pub fn log_decision(
    spine_dir: &Path,
    text: &str,
    t: &Target,
    reference: Option<&str>,
    cross_scope_ok: bool,
    source: &str,
) -> Result<Value> {
    let w = where_am_i(spine_dir, t.cwd.as_deref())?;
    let s = scope_of(spine_dir, &w, t)?;
    check_write_scope(spine_dir, &w, &s, cross_scope_ok)?;
    let events = spine::iter_events(spine_dir)?;
    let eid = next_id(&events, &today(), "d");
    let mut toks = vec![format!("id:{eid}")];
    toks.extend(tokens_for(&s, w.repo_id()));
    if let Some(r) = reference.filter(|r| !r.is_empty()) {
        if !REF_RE.is_match(r) {
            return Err(agent_error(
                "bad_request",
                format!("ref 格式不對：{}", repr(r)),
                Some(
                    "用其他工具回傳的 ref，例如 decision:2026-09-25-d1 或 collision:2026-09-25-a。",
                ),
            ));
        }
        toks.push(format!("ref:{r}"));
    }
    let ev = append(spine_dir, "decision", source, &toks, &clean(text, "text")?)?;
    Ok(json!({"ok": true, "ref": event_ref(&ev), "event": event_dict(&ev)}))
}

/// 對應 `agentapi.add_todo`：開一個未結事項（open-loop）。編號自動配；沒給 due 用 config 的預設天數。
pub fn add_todo(
    spine_dir: &Path,
    text: &str,
    due: Option<&str>,
    t: &Target,
    cross_scope_ok: bool,
    source: &str,
) -> Result<Value> {
    let w = where_am_i(spine_dir, t.cwd.as_deref())?;
    let s = scope_of(spine_dir, &w, t)?;
    check_write_scope(spine_dir, &w, &s, cross_scope_ok)?;
    let due = due.filter(|d| !d.is_empty());
    if let Some(d) = due {
        if !DATE_RE.is_match(d) {
            return Err(agent_error(
                "bad_request",
                format!("due 需為 YYYY-MM-DD：{}", repr(d)),
                None,
            ));
        }
    }
    let num = next_loop_num(&spine::iter_events(spine_dir)?);
    let mut toks = vec![num.clone()];
    if let Some(d) = due {
        toks.push(format!("due:{d}"));
    }
    toks.extend(tokens_for(&s, w.repo_id()));
    let body = clean(text, "text")?;
    let days = config::load(spine_dir)?
        .thresholds()
        .openloop_default_due_days;
    let toks = spine::default_due_tokens(&toks, &body, days, None);
    let ev = append(spine_dir, "open-loop", source, &toks, &body)?;
    Ok(json!({"ok": true, "id": num, "todo": loop_dict(&ev, None)}))
}

/// 對應 `agentapi.close_todo`：關掉一個未結事項。只能關主場範圍內的（跨範圍要使用者確認）。
/// `id` 可帶或不帶 `#`。
pub fn close_todo(
    spine_dir: &Path,
    id: &str,
    note: Option<&str>,
    cwd: Option<&Path>,
    cross_scope_ok: bool,
    source: &str,
) -> Result<Value> {
    let id = if id.starts_with('#') {
        id.to_string()
    } else {
        format!("#{id}")
    };
    let Some(cur) = spine::open_loops(spine_dir)?
        .into_iter()
        .find(|ev| ev.tokens.contains(&id))
    else {
        return Err(agent_error(
            "not_found",
            format!("找不到未結事項 {id}（不存在或已關）"),
            Some("用 next_work 或 context_for 看目前的未結清單。"),
        ));
    };
    let w = where_am_i(spine_dir, cwd)?;
    if let Some(home) = home_ids(spine_dir, &w)? {
        if !cross_scope_ok {
            let g = cur.kv("group").filter(|x| !x.is_empty());
            let r = cur.kv("repo").filter(|x| !x.is_empty());
            let ok = g.is_some_and(|g| w.groups.iter().any(|x| x == g))
                || r.is_some_and(|r| home.contains(r));
            if !ok {
                return Err(agent_error(
                    "cross_scope",
                    format!("{id} 屬於「{}」，不在你的主場。", g.or(r).unwrap_or("全域")),
                    Some("先問使用者；同意後帶 cross_scope_ok=true 重試。"),
                ));
            }
        }
    }
    let mut toks = vec![id.clone()];
    toks.extend(
        cur.tokens
            .iter()
            .filter(|t| t.starts_with("repo:") || t.starts_with("group:"))
            .cloned(),
    );
    let body = match note.filter(|n| !n.is_empty()) {
        Some(n) => format!("closed → {}", clean(n, "note")?),
        None => "closed → 完成".to_string(),
    };
    let ev = append(spine_dir, "open-loop", source, &toks, &body)?;
    Ok(json!({"ok": true, "id": id, "closed": first(&cur), "event": event_dict(&ev)}))
}

/// 對應 `agentapi.report_status`：回報目前階段（working／waiting／blocked／done）。
/// 現況檔一直被覆寫（見 [`crate::status`]）；waiting 與 blocked 另留一筆 suggestion 讓使用者在收件匣看到。
pub fn report_status(spine_dir: &Path, status_s: &str, text: &str, t: &Target) -> Result<Value> {
    let st = status::parse_status(status_s)?;
    let w = where_am_i(spine_dir, t.cwd.as_deref())?;
    let s = scope_of(spine_dir, &w, t)?;
    let rec = StatusRecord {
        status: st.as_str().into(),
        text: text.trim().into(),
        at: Local::now().format("%Y-%m-%d %H:%M").to_string(),
        scope: s.label.clone(),
        group: s.group.clone(),
        repos: s.repos.clone(),
        repo: w.repo_id().map(str::to_string),
    };
    status::save_status(spine_dir, &rec)?;
    let mut out = json!({"ok": true, "status": rec});
    if matches!(st, AgentStatus::Waiting | AgentStatus::Blocked) {
        let label = if st == AgentStatus::Waiting {
            "等你回應"
        } else {
            "卡住了"
        };
        let msg = if text.is_empty() { label } else { text };
        let ev = append(
            spine_dir,
            "suggestion",
            "agent",
            &tokens_for(&s, w.repo_id()),
            &format!("agent {label}：{}", clean(msg, "text")?),
        )?;
        out["event"] = event_dict(&ev);
    }
    Ok(out)
}

/// 對應 `agentapi.handoff`：收尾交接——完成了什麼、下次從哪接；remaining 每一項各開一個未結事項。
/// 落一筆 decision（第一行「交接：…」），並把現況設成 done。
pub fn handoff<S: AsRef<str>>(
    spine_dir: &Path,
    done: &str,
    next_step: &str,
    remaining: &[S],
    t: &Target,
    cross_scope_ok: bool,
    source: &str,
) -> Result<Value> {
    let w = where_am_i(spine_dir, t.cwd.as_deref())?;
    let s = scope_of(spine_dir, &w, t)?;
    check_write_scope(spine_dir, &w, &s, cross_scope_ok)?;
    let done = clean(done, "done")?;
    let next_step = clean(next_step, "next_step")?;
    let remaining: Vec<&str> = remaining
        .iter()
        .map(AsRef::as_ref)
        .filter(|r| !r.trim().is_empty())
        .collect();
    let mut todos: Vec<String> = Vec::new();
    for r in &remaining {
        let v = add_todo(spine_dir, r, None, t, true, source)?;
        todos.push(v["id"].as_str().unwrap_or_default().to_string());
    }
    let eid = next_id(&spine::iter_events(spine_dir)?, &today(), "d");
    let done_lines = splitlines(&done);
    let head: String = done_lines
        .first()
        .copied()
        .unwrap_or("")
        .chars()
        .take(80)
        .collect();
    let rem = if remaining.is_empty() {
        "無".to_string()
    } else {
        todos
            .iter()
            .zip(&remaining)
            .map(|(n, r)| format!("{n} {}", r.trim()))
            .collect::<Vec<_>>()
            .join("；")
    };
    let body = [
        format!("交接：{head}"),
        format!("完成：{}", done_lines.join(" ")),
        format!("剩下：{rem}"),
        format!("下次從這裡接：{}", splitlines(&next_step).join(" ")),
    ]
    .join("\n");
    let mut toks = vec![format!("id:{eid}")];
    toks.extend(tokens_for(&s, w.repo_id()));
    let ev = append(spine_dir, "decision", source, &toks, &body)?;
    report_status(spine_dir, "done", &head, t)?;
    Ok(json!({"ok": true, "ref": event_ref(&ev), "todos": todos, "event": event_dict(&ev)}))
}

/// 對應 `agentapi.all_statuses`。
pub fn all_statuses(spine_dir: &Path) -> Vec<Value> {
    status::all_statuses(spine_dir)
}

/// hooks 用：範圍判定（對應 Python hooks 直接呼叫 `_api._in`）。
pub(crate) fn in_scope_of(ev: &Event, group: Option<&str>, repos: Option<&[String]>) -> bool {
    let s = Scope {
        group: group.map(str::to_string),
        repos: repos.map(<[String]>::to_vec),
        label: String::new(),
        explicit: false,
    };
    s.contains(ev)
}

/// hooks 用：寫入事件的範圍 token（對應 Python hooks 直接呼叫 `_api._tokens_for`）。
pub(crate) fn tokens_of(
    group: Option<&str>,
    repos: Option<&[String]>,
    repo_id: Option<&str>,
) -> Vec<String> {
    let s = Scope {
        group: group.map(str::to_string),
        repos: repos.map(<[String]>::to_vec),
        label: String::new(),
        explicit: false,
    };
    tokens_for(&s, repo_id)
}
