//! Claude Code hooks（2026-09-25 Agent 友善輪）——留痕從「prompt 拜託」變成機制。對應 `repo_context/hooks.py`。
//!
//! ROLE_PROMPT 寫「沒留痕＝沒回報」，但 agent 做完事常常忘了留；工作台的會話回報、交接草稿、
//! 「接續上次工作」就全空。這裡用 hooks 補兩端：
//!
//! - SessionStart：依 cwd 找出主場，把精簡版情境（目標／上次交接／下一步／未結／工具約定）注入
//!   開場脈絡；順手記下會話開始時間與 HEAD。cwd 沒登記＝什麼都不輸出（不打擾非本工具的 repo）。
//! - SessionEnd：會話期間若沒有交接，但有 commit／未提交檔／留痕，就落一筆 suggestion [hook]——
//!   「這次會話沒交接就結束了」，只陳述採得到的事實，不替 agent 編成果。
//!   （Rust 才有）之後補一次 spine 批次 commit（[`spine::autocommit`]），判斷不會只躺在工作目錄裡。
//!
//! hook 的輸入是 Claude Code 從 stdin 給的 JSON（session_id／cwd／hook_event_name…）。
//! 任何錯誤都吞掉並回空字串／None：hook 壞掉不能擋住使用者開 agent。

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{Local, NaiveDateTime};
use regex::Regex;
use serde_json::{json, Value};
use std::sync::LazyLock;

use crate::agentapi::{self, ContextFor, Target};
use crate::fsutil::expanduser;
use crate::pyjson::dumps as py_dumps;
use crate::pystr::py_str;
use crate::spine::{self, EventType};
use crate::{crossref, ruminate, Result};

/// 對應 `hooks.TOOLS_LINE`。
pub const TOOLS_LINE: &str = "context_for／next_work／log_decision／add_todo／close_todo／\
report_status／handoff／search_knowledge／read_from／ask_repo／repo_status";

/// 開場名冊最多列幾個成員。
const ROSTER_LINES: usize = 20;

/// 名冊一列：`- **id**（你在這）｜角色｜路徑 — 介紹（推測）｜N 個會話在跑`。
fn roster_line(r: &Value) -> String {
    let mut line = format!("- **{}**", s(r, "id"));
    if r["you"] == true {
        line.push_str("（你在這）");
    }
    if let Some(role) = r["role"].as_str() {
        line.push_str(&format!("｜{role}"));
    }
    line.push_str(&format!("｜{}", s(r, "path")));
    if let Some(d) = r["description"].as_str() {
        line.push_str(&format!(" — {d}"));
        if r["description_source"] == "readme" {
            line.push_str("（推測自 README）");
        }
    }
    let n = r["sessions"].as_u64().unwrap_or(0);
    if n > 0 && r["you"] != true {
        line.push_str(&format!("｜{n} 個會話在跑"));
    }
    line
}

fn marker_dir(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("hook_sessions")
}

/// Rust 才有（2026-10-13，組員名冊）：每個 repo 現在開著幾個 agent 會話——讀開場標記（SessionEnd 會刪、
/// 超過 [`STALE_MARKER_DAYS`] 天的開場時清掉）。讀不到就是空的。
pub fn open_sessions_by_repo(spine_dir: &Path) -> std::collections::HashMap<String, usize> {
    let mut out = std::collections::HashMap::new();
    let Ok(rd) = std::fs::read_dir(marker_dir(spine_dir)) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let repo = std::fs::read_to_string(&p)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v["repo"].as_str().map(String::from));
        if let Some(r) = repo {
            *out.entry(r).or_insert(0) += 1;
        }
    }
    out
}

static BACKFILL_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*[（(]回填自[^）)]*[）)]").expect("固定 regex"));

static SAFE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\w\-]").expect("固定 regex"));

/// 對應 `hooks._safe`：session_id → 檔名（非字元換 `_`，最多 80 字）。空／缺＝`unknown`。
fn safe(sid: Option<&Value>) -> String {
    let raw = match sid {
        None | Some(Value::Null) | Some(Value::Bool(false)) => "unknown".to_string(),
        Some(Value::String(s)) if s.is_empty() => "unknown".to_string(),
        Some(Value::Number(n)) if n.as_f64() == Some(0.0) => "unknown".to_string(),
        Some(v) => py_str(v),
    };
    SAFE_RE.replace_all(&raw, "_").chars().take(80).collect()
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// 對應 `hooks.render_start`：context_for 的結果 → 開場注入文字（精簡，一屏內）。
pub fn render_start(c: &ContextFor) -> String {
    let ctx = &c.value;
    let w = &ctx["where"];
    let sc = &ctx["scope"];
    let where_ = w["repo"]
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("（未登記的目錄）");
    let label = s(sc, "label");
    let repos: Vec<&str> = sc["repos"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut head = format!("# repo-context：你在 {where_}，工作範圍「{label}」");
    if !repos.is_empty() {
        head.push_str(&format!(
            "（{} 個 repo：{}）",
            repos.len(),
            repos.join("、")
        ));
    }
    let mut out = vec![head];
    if let Some(n) = w
        .get("note")
        .and_then(Value::as_str)
        .filter(|x| !x.is_empty())
    {
        out.push(format!("註：{n}"));
    }
    out.push(match ctx["goal"].get("text").and_then(Value::as_str) {
        Some(g) => format!("目標：{g}"),
        None => format!("目標：（{}）", py_str(&ctx["goal_gap"])),
    });
    let ho = &ctx["last_handoff"];
    let lp = &ctx["last_progress"];
    if ho.is_object() {
        out.push(format!(
            "上次交接（{}，{}）：{}",
            s(ho, "at"),
            s(ho, "source"),
            s(ho, "summary")
        ));
        if let Some(ns) = ho["next_step"].as_str().filter(|x| !x.is_empty()) {
            out.push(format!("  下次從這裡接：{ns}"));
        }
    } else if lp.is_object() {
        out.push(format!(
            "上次進度（{}，{}）：{}",
            py_str(&lp["at"]),
            s(lp, "source"),
            s(lp, "text")
        ));
    }
    if ctx["next"].is_object() {
        out.push(format!(
            "建議下一步：{}（依據：{}）",
            BACKFILL_TAIL.replace_all(s(&ctx["next"], "text"), ""),
            s(&ctx["next"], "source")
        ));
    }
    let loops = ctx["open_loops"].as_array().cloned().unwrap_or_default();
    if !loops.is_empty() {
        // 未結被設上限時（N-26）總數用 open_loops_more.total
        let total = ctx["open_loops_more"]["total"]
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or(loops.len());
        out.push(format!("未結（{total}）："));
        for lp in loops.iter().take(6) {
            let flag = if lp["overdue"] == true {
                "，已逾期"
            } else {
                ""
            };
            let due = match lp["due"].as_str().filter(|d| !d.is_empty()) {
                Some(d) => format!("（due {d}{flag}）"),
                None => String::new(),
            };
            // Rust 才有（N-22）：回填來源的尾巴（「（回填自 … 會話 …）」）對 agent 沒用，開場不印（資料不動）
            let text = BACKFILL_TAIL.replace_all(s(lp, "text"), "");
            out.push(format!("- {} {}{due}", py_str(&lp["id"]), text.trim_end()));
        }
        if total > 6 {
            out.push(format!(
                "- …另 {} 項（用 next_work 看完整清單）",
                total - 6.min(loops.len())
            ));
        }
    }
    // Rust 才有（N-22）：要處理的提醒集中一段（跟 MCP context_for 的 hints 同一份）
    let hints: Vec<&Value> = ctx["hints"].as_array().into_iter().flatten().collect();
    if !hints.is_empty() {
        out.push(String::new());
        out.push("## 提醒（要處理的；先問使用者再動手）".into());
        out.extend(hints.iter().map(|h| format!("- {}", s(h, "text"))));
        out.push(String::new());
    }
    for a in ctx["attention"].as_array().into_iter().flatten().take(3) {
        out.push(format!("注意：{}", s(a, "text")));
    }
    let st = &ctx["agent_status"];
    if matches!(st["status"].as_str(), Some("waiting") | Some("blocked")) {
        out.push(format!(
            "上一個 agent 的狀態：{}（{}）{}",
            s(st, "status"),
            py_str(&st["at"]),
            st.get("text").map(py_str).unwrap_or_default()
        ));
    }
    // Rust 才有（2026-10-13）：組員名冊（context_for 的 roster）
    let roster: Vec<&Value> = ctx["roster"].as_array().into_iter().flatten().collect();
    if !roster.is_empty() {
        out.push(String::new());
        out.push(format!(
            "## 同組成員（{} 個；路徑可直接讀，要留引用就用 read_from）",
            roster.len()
        ));
        out.extend(roster.iter().take(ROSTER_LINES).map(|r| roster_line(r)));
        if roster.len() > ROSTER_LINES {
            out.push(format!(
                "- …另 {} 個（context_for 的 roster 有完整清單）",
                roster.len() - ROSTER_LINES
            ));
        }
        let you_missing = roster
            .iter()
            .any(|r| r["you"] == true && r["description_source"] != "registry");
        if you_missing {
            out.push(
                "你所在的 repo 還沒寫介紹：手上的事做完，可用 describe_repo 補一句介紹與角色（存在 spine，不動 repo）。"
                    .into(),
            );
        }
    }
    if !c.neighbors.is_empty() {
        out.push(String::new());
        out.push(
            "## 鄰居 repo（有關係的；要它們的東西用 read_from `<repo>:<export>/子路徑`，不要寫死路徑）"
                .into(),
        );
        out.extend(crossref::neighbor_lines(&c.neighbors, 4));
    }
    if !c.drift.is_empty() {
        out.push("你之前引用過的上游檔之後又改了（本地的副本或結論可能過期）：".into());
        out.extend(crossref::drift_lines(&c.drift, 3));
    }
    if !c.fresh.is_empty() {
        out.push(String::new());
        out.push("## 自上次以來跟你手上工作相關的新料（反芻；別組新進的知識或判斷）".into());
        out.extend(ruminate::link_lines(&c.fresh, 3));
        out.push(
            "用得上就 read_from 那個 ref；不相關可略過（使用者可 `ctx fresh --dismiss <id>`）。"
                .into(),
        );
    }
    out.extend([
        String::new(),
        "## 工作約定（repo-context）".into(),
        format!("- 範圍＝「{label}」。寫入其他組會被工具拒絕；要跨組先問使用者。"),
        format!(
            "- 工具（MCP repo-context）：{TOOLS_LINE}。沒掛 MCP 就用 `ctx <where|context|next|log|todo|status|handoff|search> --json`。"
        ),
        "- 做了判斷→log_decision；發現待辦→add_todo；等使用者或卡住→report_status。".into(),
        "- 改掉舊判斷→log_decision 帶 supersedes；判斷後來怎樣→log_decision 帶 outcome_of；把一項待辦交給新會話→`ctx todo brief #N`。".into(),
        "- 要別的 repo 的資料：search_knowledge 找 → read_from 讀（會記引用）；要對方整理歸納才用 ask_repo（慢、花錢）。"
            .into(),
        "- 收尾一定呼叫 handoff（完成了什麼、剩什麼、下次從哪接）；沒交接的會話會被記成未交接。".into(),
    ]);
    out.join("\n")
}

/// 對應 `hooks._git_lines`：`git -C <path> ...` 的非空行；失敗＝空。
fn git_lines(path: &str, args: &[&str]) -> Vec<String> {
    let p = expanduser(path);
    let (rc, out) = crossref::git(&p, args, Duration::from_secs(10));
    if rc != 0 {
        return Vec::new();
    }
    out.lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// 對應 `hooks.session_start`：回傳要印到 stdout 的字串（SessionStart hook 的 JSON 輸出）；不適用就回空字串。
pub fn session_start(spine_dir: &Path, payload: &Value, now: Option<NaiveDateTime>) -> String {
    session_start_inner(spine_dir, payload, now).unwrap_or_default()
}

fn session_start_inner(
    spine_dir: &Path,
    payload: &Value,
    now: Option<NaiveDateTime>,
) -> Result<String> {
    let cwd = match payload
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
    {
        Some(c) => c.to_string(),
        None => std::env::current_dir()?.to_string_lossy().into_owned(),
    };
    let cwd_p = PathBuf::from(&cwd);
    let w = agentapi::where_am_i(spine_dir, Some(&cwd_p))?;
    let Some(repo) = w.repo.clone() else {
        return Ok(String::new());
    };
    let ctx = agentapi::context_for_full(spine_dir, &Target::at(&cwd_p), false)?;
    if ruminate::is_stale(spine_dir, now) {
        // 背景補跑；這次開場用上一輪的結果
        let _ = ruminate::spawn_detached(spine_dir);
    }
    let now = now.unwrap_or_else(|| Local::now().naive_local());
    let output = py_dumps(&json!({"hookSpecificOutput": {
        "hookEventName": "SessionStart",
        "additionalContext": render_start(&ctx),
    }}));
    if !has_session_id(payload) {
        // 刻意差異（N-14）：沒有 session_id 的不是 Claude Code 開的會話（手動試跑、量測、誤觸）——
        // 照樣輸出注入內容，但不記 marker：Python 會寫成 `unknown.json`，之後被當成一場會話、
        // 下一個同樣沒 id 的 session-end 還會把它記成「未交接」。
        return Ok(output);
    }
    let d = marker_dir(spine_dir);
    std::fs::create_dir_all(&d)?;
    let head = git_lines(&repo.path, &["rev-parse", "HEAD"])
        .into_iter()
        .next();
    let marker = json!({
        "session_id": payload.get("session_id").cloned().unwrap_or(Value::Null),
        "cwd": cwd,
        "started": now.format("%Y-%m-%d %H:%M").to_string(),
        "started_iso": now.format("%Y-%m-%dT%H:%M:%S").to_string(),
        "repo": repo.id, "repo_path": repo.path,
        // 記下開場時的 HEAD：結束時只算這之後的 commit（用時間窗會把既有 commit 算進來）
        "head": head,
        "group": ctx.value["scope"]["group"], "repos": ctx.value["scope"]["repos"],
    });
    std::fs::write(
        d.join(format!("{}.json", safe(payload.get("session_id")))),
        py_dumps(&marker),
    )?;
    let _ = sweep_stale_markers(spine_dir, now);
    Ok(output)
}

/// Rust 才有（N-16）：會話被直接關掉（沒走 SessionEnd）時 marker 會一直留著。開場時把開了超過
/// [`STALE_MARKER_DAYS`] 天的當成已結束：照 SessionEnd 的規則判斷（有事發生又沒交接才記一筆），然後清掉。
/// 門檻保守——會話可能一開好幾天。
pub const STALE_MARKER_DAYS: i64 = 7;

fn sweep_stale_markers(spine_dir: &Path, now: NaiveDateTime) -> Result<usize> {
    let mut n = 0;
    for ent in std::fs::read_dir(marker_dir(spine_dir))?.flatten() {
        let p = ent.path();
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(m) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let started = m["started_iso"]
            .as_str()
            .and_then(|t| NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S").ok());
        if !started.is_some_and(|t| now - t > chrono::Duration::days(STALE_MARKER_DAYS)) {
            continue;
        }
        let sid = p
            .file_stem()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_default();
        let payload = json!({
            "session_id": sid,
            "reason": format!("超過 {STALE_MARKER_DAYS} 天沒有收到結束訊號，會話可能被直接關掉"),
        });
        session_end(spine_dir, &payload, Some(now));
        let _ = std::fs::remove_file(&p); // session_end 沒處理到（例如壞檔）也要清
        n += 1;
    }
    Ok(n)
}

/// hook 輸入有沒有可用的 session_id（[`safe`] 會換成 `unknown` 的那些都算沒有）。
fn has_session_id(payload: &Value) -> bool {
    safe(payload.get("session_id")) != "unknown"
}

/// 對應 `hooks.session_end`：會話結束——沒交接但有事發生 → suggestion [hook]。回傳寫入的事件 dict 或 None。
pub fn session_end(spine_dir: &Path, payload: &Value, now: Option<NaiveDateTime>) -> Option<Value> {
    session_end_inner(spine_dir, payload, now).ok().flatten()
}

fn session_end_inner(
    spine_dir: &Path,
    payload: &Value,
    now: Option<NaiveDateTime>,
) -> Result<Option<Value>> {
    if !has_session_id(payload) {
        return Ok(None); // 同 session_start：沒 id 的不是一場會話
    }
    let p = marker_dir(spine_dir).join(format!("{}.json", safe(payload.get("session_id"))));
    if !p.exists() {
        return Ok(None);
    }
    let m: Value = serde_json::from_str(&std::fs::read_to_string(&p)?)?;
    std::fs::remove_file(&p)?;
    let group = m["group"]
        .as_str()
        .filter(|g| !g.is_empty())
        .map(str::to_string);
    let repos: Option<Vec<String>> = m["repos"].as_array().map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    });
    let started = m["started"]
        .as_str()
        .ok_or_else(|| crate::Error::invalid("marker 缺 started"))?
        .to_string();
    let (mut traces, mut handed_off) = (0usize, false);
    for ev in spine::iter_events(spine_dir)? {
        if ev.stamp() < started || ev.source == "hook" || ev.source == "monitor" {
            continue;
        }
        if !agentapi::in_scope_of(&ev, group.as_deref(), repos.as_deref()) {
            continue;
        }
        traces += 1;
        if ev.etype == EventType::Decision && ev.body.starts_with("交接") {
            handed_off = true;
        }
    }
    if handed_off {
        return Ok(None);
    }
    let repo_path = s(&m, "repo_path");
    let commits = match m["head"].as_str().filter(|h| !h.is_empty()) {
        Some(h) => git_lines(repo_path, &["log", &format!("{h}..HEAD"), "--format=%h %s"]),
        None => git_lines(
            repo_path,
            &[
                "log",
                &format!("--since={}", s(&m, "started_iso")),
                "--format=%h %s",
            ],
        ),
    };
    let dirty = git_lines(repo_path, &["status", "--porcelain"]);
    if commits.is_empty() && dirty.is_empty() && traces == 0 {
        return Ok(None); // 無事不報：什麼都沒發生的會話不留痕
    }
    let mut first_fact = format!("期間 {} 個 commit", commits.len());
    if !commits.is_empty() {
        let subjects: Vec<&str> = commits
            .iter()
            .take(3)
            .map(|c| c.split_once(' ').map(|(_, s)| s).unwrap_or(c))
            .collect();
        first_fact.push_str(&format!(
            "（{}{}）",
            subjects.join("；"),
            if commits.len() > 3 { "…" } else { "" }
        ));
    }
    let facts = [
        first_fact,
        format!("{} 目前未提交 {} 個檔", s(&m, "repo"), dirty.len()),
        format!("範圍內留痕 {traces} 筆"),
    ];
    let toks = agentapi::tokens_of(group.as_deref(), repos.as_deref(), Some(s(&m, "repo")));
    let reason = payload
        .get("reason")
        .filter(|r| !r.is_null() && r.as_str() != Some(""))
        .map(py_str)
        .unwrap_or_else(|| "結束".into());
    let body = format!(
        "agent 會話結束但沒有交接（{started} 開始，{reason}）\n{}\n下次開場先補交接：做到哪、剩什麼、下次從哪接。",
        facts.join("；")
    );
    let ev = spine::append_event(spine_dir, "suggestion", "hook", &toks, &body, now)?;
    Ok(Some(agentapi::event_dict(&ev)))
}

/// 對應 `ctx hook <event>` 的引擎部分（Python `cli/setup.cmd_hook`）：stdin 是 hook JSON（壞的或空的＝`{}`），
/// event＝`session-start`｜`session-end`。回傳要印到 stdout 的字串（空＝不印）。永遠不失敗。
pub fn run_hook(spine_dir: &Path, event: &str, stdin: &str) -> String {
    let payload = parse_payload(stdin);
    if event == "session-start" {
        session_start(spine_dir, &payload, None)
    } else {
        session_end(spine_dir, &payload, None);
        // Rust 才有（N-01）：App 沒開時 timer 的排程 commit 不會跑——會話結束順手補一次
        let _ = spine::autocommit(spine_dir, None);
        String::new()
    }
}

/// hook stdin → JSON（Python `json.loads(sys.stdin.read() or "{}")`，壞的＝`{}`）。
pub fn parse_payload(stdin: &str) -> Value {
    let text = if stdin.is_empty() { "{}" } else { stdin };
    serde_json::from_str(text).unwrap_or_else(|_| json!({}))
}
