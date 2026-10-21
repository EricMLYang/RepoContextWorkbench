//! 工作台的行動按鈕 → 引擎原語（每個 action 一個函式，[`ACTIONS`] 表分派）。對應 Python `webui/actions.py`。
//!
//! registry／組的寫入走操作表（`ops`）：參數驗證、留痕跟 CLI／MCP 同一份，這裡只轉回傳形狀。
//! 新增 action：寫一個 `a_<名稱>(spine, payload, terms)`，加進 [`ACTIONS`]。Rust 才有的較大一群放子模組：
//! `namecard`（名片）、`finder`（⌘K、變更與差異）、`briefs`（交給 agent 的任務文字）。
//!
//! 錯誤形狀：Python 回 `{"error": "..."}`；這裡回 `Err(Error::Invalid("..."))`，訊息逐字相同
//! （桌面殼把它轉成 IPC 的 `Err(String)`）。會話相關 action 透過 [`Terms`] 拿會話清冊——
//! 引擎不依賴 Tauri，殼把 `TermManager` 傳進來即可（測試可給假清冊）。

use std::path::{Path, PathBuf};

use chrono::{Duration, Local};
use serde_json::{json, Map, Value};

use super::state::{collision_index, normalize_dest, session_draft, term_value};
use crate::ops::{self, Via};
use crate::session::{self, SessionOpts};
use crate::spine::{self, EventType};
use crate::term::{AgentLaunch, TermInfo, TermManager};
use crate::{brief, collide, registry, route};
use crate::{Error, Result};

mod briefs;
mod files;
mod finder;
mod namecard;
mod sessions;

use briefs::{a_pick_brief, a_profile_draft_brief, a_review_brief};
use files::{
    a_backlinks, a_create_entry, a_read_file, a_rename_entry, a_repo_tree, a_save_image,
    a_write_file,
};
use finder::{a_file_diff, a_palette, a_repo_changes, a_scope_changes};
use namecard::{a_describe_repo, a_repo_class, a_repo_profile};
use sessions::{
    a_auto_continue, a_session_draft, a_session_result, a_session_seen, a_term_create, a_term_kill,
};

/// 會話清冊（`TermManager` 的那幾個操作）。殼給真的，測試給假的。
pub trait Terms {
    /// 對應 `TermManager.get(sid)` 的描述欄位；未知 sid 回 None。
    fn info(&self, sid: &str) -> Option<TermInfo>;
    /// 對應 `TermManager.create_shell`。回 (sid, title)。
    fn create_shell(&self, cwd: Option<&Path>, origin: Option<String>) -> Result<(String, String)>;
    /// 對應 `TermManager.create_agent`（指令已由 `session::build` 組好）。回 (sid, title)。
    fn create_agent(&self, launch: AgentLaunch) -> Result<(String, String)>;
    /// 對應 `TermManager.kill`：未知 sid 回 false。
    fn kill(&self, sid: &str) -> bool;
    /// 會話開在哪個目錄（Rust 才有；給「這個會話的 Terminal」沿用）。未知 sid 回 None。
    fn cwd(&self, _sid: &str) -> Option<PathBuf> {
        None
    }
}

impl Terms for TermManager {
    fn info(&self, sid: &str) -> Option<TermInfo> {
        self.get(sid).map(|s| s.info())
    }
    fn create_shell(&self, cwd: Option<&Path>, origin: Option<String>) -> Result<(String, String)> {
        let s = TermManager::create_shell(self, cwd, origin)?;
        Ok((s.sid().to_string(), s.title().to_string()))
    }
    fn create_agent(&self, launch: AgentLaunch) -> Result<(String, String)> {
        let s = TermManager::create_agent(self, launch)?;
        Ok((s.sid().to_string(), s.title().to_string()))
    }
    fn kill(&self, sid: &str) -> bool {
        TermManager::kill(self, sid)
    }
    fn cwd(&self, sid: &str) -> Option<PathBuf> {
        self.get(sid).and_then(|s| s.cwd().map(Path::to_path_buf))
    }
}

type ActionFn = fn(&Path, &Value, Option<&dyn Terms>) -> Result<Value>;

/// 對應 `actions.ACTIONS`（順序照抄）。
pub const ACTIONS: &[(&str, ActionFn)] = &[
    ("ack", a_ack),
    ("relate", a_relate),
    ("unrelate", a_unrelate),
    ("relation_dismiss", a_relation_dismiss),
    ("fresh_try", a_fresh_try),
    ("todo_brief", a_todo_brief),
    ("reopen_defer", a_reopen_defer),
    ("decision_outcome", a_decision_outcome),
    ("decision_supersede", a_decision_supersede),
    ("fresh_dismiss", a_fresh_dismiss),
    ("collide", a_collide),
    ("collide_rerun", a_collide_rerun),
    ("ignore", a_ignore),
    ("close_loop", a_close_loop),
    ("loop_defer", a_loop_defer),
    ("defer", a_defer),
    ("tier", a_tier),
    ("remove_repo", a_remove_repo),
    ("route_collision", a_route_collision),
    ("tag", a_tag),
    ("repo_profile", a_repo_profile),
    ("describe_repo", a_describe_repo),
    ("repo_class", a_repo_class),
    ("palette", a_palette),
    ("pick_brief", a_pick_brief),
    ("profile_draft_brief", a_profile_draft_brief),
    ("save_group", a_save_group),
    ("update_group", a_update_group),
    ("remove_group", a_remove_group),
    ("set_goal", a_set_goal),
    ("session_draft", a_session_draft),
    ("scan_dir", a_scan_dir),
    ("register_repos", a_register_repos),
    ("repo_tree", a_repo_tree),
    ("group_add_folder", a_group_add_folder),
    ("reorder", a_reorder),
    ("read_file", a_read_file),
    ("repo_changes", a_repo_changes),
    ("scope_changes", a_scope_changes),
    ("file_diff", a_file_diff),
    ("review_brief", a_review_brief),
    ("write_file", a_write_file),
    ("create_entry", a_create_entry),
    ("rename_entry", a_rename_entry),
    ("save_image", a_save_image),
    ("backlinks", a_backlinks),
    ("locate_path", files::a_locate_path),
    ("repo_worktrees", files::a_repo_worktrees),
    ("repo_log", files::a_repo_log),
    ("repo_github", files::a_repo_github),
    ("commit_diff", files::a_commit_diff),
    ("worktree_remove", files::a_worktree_remove),
    ("past_sessions", finder::a_past_sessions),
    ("session_transcript", finder::a_session_transcript),
    ("session_digest", finder::a_session_digest),
    ("agent_commands", finder::a_agent_commands),
    ("prompts", finder::a_prompts),
    ("prompt_save", finder::a_prompt_save),
    ("prompt_remove", finder::a_prompt_remove),
    ("diff_notes_brief", finder::a_diff_notes_brief),
    ("assist_plan", a_assist_plan),
    ("assist_log", a_assist_log),
    ("assist_recent", a_assist_recent),
    ("ack_scope", a_ack_scope),
    ("group_notes", a_group_notes),
    ("note_add", a_note_add),
    ("note_remove", a_note_remove),
    ("brief", a_brief),
    ("term_create", a_term_create),
    ("term_kill", a_term_kill),
    ("session_seen", a_session_seen),
    ("session_result", a_session_result),
    ("auto_continue", a_auto_continue),
];

/// 不寫入任何東西的 action：殼不發 `workbench://changed`（否則展開資料夾、開檔都會讓前端重抓整份 state）。
/// Rust 才有（2026-10-13，原本寫死在殼、漏了檔案樹與檢視器的讀取）。
const READ_ONLY: &[&str] = &[
    "session_draft",
    "scan_dir",
    "repo_tree",
    "read_file",
    "repo_changes",
    "scope_changes",
    "file_diff",
    "review_brief",
    "group_notes",
    "repo_profile",
    "palette",
    "pick_brief",
    "profile_draft_brief",
    "assist_plan",
    "assist_recent",
    "backlinks",
    "locate_path",
    "repo_worktrees",
    "repo_log",
    "repo_github",
    "commit_diff",
    "past_sessions",
    "session_transcript",
    "agent_commands",
    "prompts",
    "diff_notes_brief",
];

/// 這個 action 只讀（不改 spine、registry、會話清冊）。未知名稱＝false。
pub fn is_read_only(name: &str) -> bool {
    READ_ONLY.contains(&name)
}

/// 全部 action 名稱（給殼列舉）。
pub fn action_names() -> Vec<&'static str> {
    ACTIONS.iter().map(|(n, _)| *n).collect()
}

/// 對應 `actions.handle_action`：空範圍一律拒絕（不代表全部）；未知 action 回錯。
pub fn handle_action(
    spine_dir: impl AsRef<Path>,
    name: &str,
    payload: &Value,
    terms: Option<&dyn Terms>,
) -> Result<Value> {
    if payload
        .get("repos")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        return Err(Error::invalid("至少選擇一個 repo；空範圍不代表全部"));
    }
    let Some((_, f)) = ACTIONS.iter().find(|(n, _)| *n == name) else {
        return Err(Error::invalid(format!("未知 action: {name}")));
    };
    let mut out = f(spine_dir.as_ref(), payload, terms)?;
    if RESCAN_AFTER.contains(&name) {
        if let Some(o) = out.as_object_mut() {
            o.insert("rescan".into(), Value::Bool(true));
        }
    }
    Ok(out)
}

/// 會改變 repo 採集結果的 action（tier、延後問句卡、移除 repo、類別——參考類不採集）：結果多帶 `rescan: true`，前端照它重新採集。
/// Rust 才有（2026-10-13，原本前端寫死這份清單）。
const RESCAN_AFTER: &[&str] = &[
    "tier",
    "defer",
    "remove_repo",
    "repo_class",
    "worktree_remove",
];

/// 把 payload 解析成型別化參數。**新 action 一律用這個**（缺欄位、型別不對都在這裡擋下）；
/// 下面那組 `raw`／`s`／`int` 只給照抄 Python 版的舊 action（`payload.get(k) or …` 的語意），合併回 main 後換掉。
fn parse<T: serde::de::DeserializeOwned>(action: &str, p: &Value) -> Result<T> {
    T::deserialize(p).map_err(|e| Error::invalid(format!("{action} 參數不合法：{e}")))
}

// ── payload 小工具（Python `payload.get(k) or …` 的語意）──

/// `payload.get(k)`：字串原樣；null／不存在＝None；其他型別以 JSON 文字表示。
fn raw(p: &Value, k: &str) -> Option<String> {
    match p.get(k)? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        v => Some(crate::pystr::py_str(v)),
    }
}

/// `payload.get(k) or None`：空字串也當沒給。
fn s(p: &Value, k: &str) -> Option<String> {
    raw(p, k).filter(|v| !v.is_empty())
}

/// 錯誤訊息裡的 `{payload.get(k)}`：沒給印 `None`（照 Python）。
fn shown(p: &Value, k: &str) -> String {
    raw(p, k).unwrap_or_else(|| "None".into())
}

/// `payload.get(k) or None` 的字串清單（空清單＝None）。
fn list(p: &Value, k: &str) -> Option<Vec<String>> {
    let a = p.get(k)?.as_array()?;
    let v: Vec<String> = a
        .iter()
        .map(|x| match x {
            Value::String(s) => s.clone(),
            other => crate::pystr::py_str(other),
        })
        .collect();
    (!v.is_empty()).then_some(v)
}

/// `int(payload.get(k) or default)`。
fn int(p: &Value, k: &str, default: i64) -> Result<i64> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Number(n)) => match n.as_i64() {
            Some(0) => Ok(default),
            Some(v) => Ok(v),
            None => Ok(n
                .as_f64()
                .map(|f| f as i64)
                .filter(|v| *v != 0)
                .unwrap_or(default)),
        },
        Some(Value::String(s)) if s.is_empty() => Ok(default),
        Some(Value::String(s)) => s
            .trim()
            .parse()
            .map_err(|_| Error::invalid(format!("invalid literal for int() with base 10: '{s}'"))),
        Some(Value::Bool(b)) => Ok(if *b { 1 } else { default }),
        Some(_) => Err(Error::invalid(format!("{k} 必須是整數"))),
    }
}

fn obj(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = Map::new();
    for (k, v) in pairs {
        m.insert(k.to_string(), v);
    }
    Value::Object(m)
}

/// 對應 `actions._via_op`：跑操作表的操作，回 `{ok: true, **shape(result)}`。
fn via_op(
    spine_dir: &Path,
    name: &str,
    args: Value,
    shape: impl FnOnce(Value) -> Value,
) -> Result<Value> {
    let (result, _) = ops::call(spine_dir, name, args, Via::Ui)?;
    let mut out = Map::new();
    out.insert("ok".into(), json!(true));
    if let Value::Object(m) = shape(result) {
        out.extend(m);
    }
    Ok(Value::Object(out))
}

fn ok() -> Value {
    json!({"ok": true})
}

fn valid_loop_num(num: &str) -> bool {
    num.strip_prefix('#')
        .is_some_and(|n| !n.is_empty() && n.chars().all(char::is_numeric))
}

fn scope_tokens(tokens: &[String]) -> Vec<String> {
    tokens
        .iter()
        .filter(|t| t.starts_with("repo:") || t.starts_with("group:"))
        .cloned()
        .collect()
}

/// 判定丟背景跑：找得到 `ctx`（自己、同目錄或 PATH，見 `exe::ctx_exe`）時起 detached 進程（關掉 App 也跑得完，同 Python）；
/// 否則（桌面 App 沒附 ctx）在本進程開執行緒跑。`REPOCTX_NO_BACKGROUND` 設了就不跑（測試用）。
fn judge_in_background(spine_dir: &Path, cid: &str) -> Result<()> {
    if crate::exe::background_disabled() {
        return Ok(());
    }
    let exe = crate::exe::ctx_exe()?;
    if crate::exe::is_ctx(&exe) {
        collide::spawn_detached_with(&exe, spine_dir, cid, None, None)?;
        return Ok(());
    }
    let (spine, cid) = (spine_dir.to_path_buf(), cid.to_string());
    std::thread::Builder::new()
        .name(format!("collide-{cid}"))
        .spawn(move || {
            // 失敗時 run_judgement 自己會落 system-unsure 事件浮出；這裡沒有人可回報
            let _ = collide::run_judgement(&spine, &cid, None, None, None, None);
        })?;
    Ok(())
}

// ── actions ──

fn a_ack(d: &Path, _p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    spine::ack_unread(d, None)?;
    Ok(ok())
}

/// Rust 才有（N-37）：只把目前範圍的一般未讀標為已讀（`.state/seen_scopes.json`），別組不受影響。
fn a_ack_scope(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let repos = list(p, "repos");
    let group = s(p, "group");
    let scope = super::state::StateScope::new(group.as_deref(), repos.as_deref());
    let at = super::state::ack_scope(d, &scope, None)?;
    Ok(json!({"ok": true, "at": at}))
}

fn a_relate(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    via_op(
        d,
        "registry_relate",
        json!({"a": raw(p, "a"), "b": raw(p, "b"), "kind": raw(p, "kind"), "note": s(p, "note")}),
        |mut r| {
            if let Some(o) = r.as_object_mut() {
                o.shift_remove("label");
            }
            json!({"relation": r})
        },
    )
}

/// Rust 才有（F-46）：「該重看」延後（人看過、先維持原判斷）。
fn a_reopen_defer(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let Some(r) = s(p, "ref").filter(|x| !x.is_empty()) else {
        return Err(Error::invalid("reopen_defer 需要 ref"));
    };
    let days = p.get("days").and_then(Value::as_i64).unwrap_or(14);
    let until = crate::agentapi::snooze_reopen(d, &r, days)?;
    Ok(json!({"ok": true, "until": until}))
}

/// Rust 才有（2026-10-20）：判斷回顧卡「記結果…」——人在 App 直接寫這個判斷後來怎樣
/// （同 agent 的 `log_decision` 帶 `outcome_of`；source=manual）。範圍照卡片給的組／repo。
/// 之前卡上只有「開會話回顧」與「延 30 天」，人明明十秒能寫卻沒地方寫，結果記錄率一直 0%。
fn a_decision_outcome(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    decision_from_app(d, p, "decision_outcome", "後來怎樣", |r| {
        crate::agentapi::DecisionExtra {
            outcome_of: Some(r),
            ..Default::default()
        }
    })
}

/// Rust 才有（2026-10-20）：該重看卡「改判斷…」——人在 App 直接寫新判斷，取代原判斷
/// （同 agent 的 `log_decision` 帶 `supersedes`；source=manual）。取代了，該重看卡就退場。
fn a_decision_supersede(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    decision_from_app(d, p, "decision_supersede", "新判斷", |r| {
        crate::agentapi::DecisionExtra {
            supersedes: Some(r),
            ..Default::default()
        }
    })
}

/// 收件匣卡片上人寫的判斷（記結果／改判斷）共用：`ref`＋`text` 必填，範圍照卡片給的組／repo。
fn decision_from_app(
    d: &Path,
    p: &Value,
    name: &str,
    what: &str,
    extra: impl FnOnce(String) -> crate::agentapi::DecisionExtra,
) -> Result<Value> {
    let Some(r) = s(p, "ref") else {
        return Err(Error::invalid(format!("{name} 需要 ref")));
    };
    let Some(text) = s(p, "text")
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
    else {
        return Err(Error::invalid(format!("{name} 需要 text（{what}）")));
    };
    let t = crate::agentapi::Target {
        cwd: None,
        group: s(p, "group"),
        repos: s(p, "repo").map(|x| vec![x]),
    };
    let logged = crate::agentapi::log_decision_ext(d, &text, &t, None, &extra(r), true, "manual")?;
    Ok(json!({"ok": true, "ref": logged.reference}))
}

/// Rust 才有（F-42）：未結事項的任務包（前端拿 markdown 當新會話的任務）。
fn a_todo_brief(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let Some(num) = s(p, "num").filter(|x| !x.is_empty()) else {
        return Err(Error::invalid("todo_brief 需要 num"));
    };
    crate::agentapi::todo_brief(d, &num, &crate::agentapi::Target::default())
}

/// Rust 才有（F-26）：反芻推薦「拿來試」——在該組開帶出處的待辦（人在 App 按的＝manual）。
fn a_fresh_try(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let Some(id) = s(p, "id").filter(|x| !x.is_empty()) else {
        return Err(Error::invalid("fresh_try 需要 id"));
    };
    crate::agentapi::try_fresh(d, &id, "manual")
}

/// Rust 才有（F-48）：反芻推薦「不相關」（同 `ctx fresh --dismiss`）。
fn a_fresh_dismiss(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let Some(id) = s(p, "id").filter(|x| !x.is_empty()) else {
        return Err(Error::invalid("fresh_dismiss 需要 id"));
    };
    let l = crate::ruminate::dismiss(d, &id)?;
    Ok(json!({"ok": true, "dismissed": l.id}))
}

/// Rust 才有（F-47）：關係建議「不相關」——記住，之後不再建議。
fn a_relation_dismiss(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let (Some(a), Some(b)) = (
        s(p, "a").filter(|x| !x.is_empty()),
        s(p, "b").filter(|x| !x.is_empty()),
    ) else {
        return Err(Error::invalid("relation_dismiss 需要 a 與 b"));
    };
    crate::relsuggest::dismiss(d, &a, &b)?;
    Ok(json!({"ok": true, "dismissed": [a, b]}))
}

fn a_unrelate(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    via_op(
        d,
        "registry_unrelate",
        json!({"a": raw(p, "a"), "b": raw(p, "b"), "kind": s(p, "kind")}),
        |r| r,
    )
}

fn a_collide(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let idea = raw(p, "idea").unwrap_or_default().trim().to_string();
    if idea.is_empty() {
        return Err(Error::invalid("想法不可為空"));
    }
    let repos = list(p, "repos");
    let group = s(p, "group");
    let cid = collide::submit(
        d,
        &idea,
        group.as_deref(),
        repos.as_deref(),
        "monitor",
        None,
    )?;
    if p.get("wait").is_some_and(super::state::truthy) {
        let j = collide::run_judgement(d, &cid, None, None, None, None)?;
        return Ok(json!({"ok": true, "cid": cid, "judgement": j}));
    }
    judge_in_background(d, &cid)?;
    Ok(json!({"ok": true, "cid": cid}))
}

fn a_collide_rerun(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let Some(cid) = s(p, "cid") else {
        return Err(Error::invalid("缺 cid"));
    };
    judge_in_background(d, &cid)?;
    Ok(json!({"ok": true, "cid": cid}))
}

fn a_ignore(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let etype = shown(p, "type");
    if !spine::EVENT_TYPES.contains(&etype.as_str()) {
        return Err(Error::invalid(format!("未知事件型別: {etype}")));
    }
    let rf = match s(p, "cid") {
        Some(cid) => format!("collision:{cid}"),
        None => format!("{etype}:{}", shown(p, "time")),
    };
    let header = match p.get("header") {
        None => String::new(),
        Some(_) => shown(p, "header"),
    };
    spine::append_event(
        d,
        "chosen",
        "monitor",
        &[format!("ref:{rf}")],
        &format!("忽略並記錄：{header}"),
        None,
    )?;
    Ok(ok())
}

fn a_close_loop(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let num = raw(p, "num").unwrap_or_default();
    if !valid_loop_num(&num) {
        return Err(Error::invalid(format!("loop 編號不合法: {num}")));
    }
    let cur = spine::open_loops(d)?
        .into_iter()
        .find(|e| e.tokens.contains(&num));
    let mut tokens = vec![num.clone()];
    tokens.extend(cur.map(|c| scope_tokens(&c.tokens)).unwrap_or_default());
    spine::append_event(
        d,
        "open-loop",
        "monitor",
        &tokens,
        "closed → 從工作台關閉",
        None,
    )?;
    Ok(ok())
}

fn a_loop_defer(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let num = raw(p, "num").unwrap_or_default();
    if !valid_loop_num(&num) {
        return Err(Error::invalid(format!("loop 編號不合法: {num}")));
    }
    let days = int(p, "days", 7)?;
    let Some(cur) = spine::open_loops(d)?
        .into_iter()
        .find(|e| e.tokens.contains(&num))
    else {
        return Err(Error::invalid(format!("loop 不存在或已關: {num}")));
    };
    let due = (Local::now().date_naive() + Duration::days(days))
        .format("%Y-%m-%d")
        .to_string();
    let first = if cur.body.is_empty() {
        ""
    } else {
        cur.first_line()
    };
    let mut tokens = vec![num.clone(), format!("due:{due}")];
    tokens.extend(scope_tokens(&cur.tokens));
    spine::append_event(
        d,
        "open-loop",
        "monitor",
        &tokens,
        &format!("{first}\n延期 {days} 天 → 從工作台"),
        None,
    )?;
    Ok(json!({"ok": true, "due": due}))
}

fn a_defer(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let rid = s(p, "id").unwrap_or_default();
    let qkind = s(p, "qkind").unwrap_or_else(|| "question".into());
    let days = int(p, "days", 7)?;
    if rid.is_empty() {
        return Err(Error::invalid("缺 repo id"));
    }
    let until = (Local::now().date_naive() + Duration::days(days))
        .format("%Y-%m-%d")
        .to_string();
    // 按鈕說「七天後再提醒」，做的就是延後提醒——留痕第一行也要這樣寫，
    // 使用者翻活動記錄時才說得出系統究竟做了什麼（2026-09-12 檢討 §6.1）
    let text = match p.get("text") {
        None => String::new(),
        Some(_) => shown(p, "text"),
    };
    let body = format!(
        "{days} 天後再提醒（{rid} 的「{}」提醒延到 {until}；沒有安排任何進度）\nsnooze:{qkind} until:{until}\n{text}",
        brief::qkind_name(&qkind)
    );
    spine::append_event(
        d,
        "chosen",
        "monitor",
        &[format!("repo:{rid}")],
        body.trim_end(),
        None,
    )?;
    Ok(json!({"ok": true, "until": until, "days": days}))
}

fn a_tier(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    via_op(
        d,
        "registry_tier",
        json!({"id": raw(p, "id"), "tier": raw(p, "tier"), "resume_when": s(p, "resume_when")}),
        |r| obj(vec![("id", r["id"].clone()), ("tier", r["tier"].clone())]),
    )
}

fn a_remove_repo(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    via_op(d, "registry_remove", json!({"id": raw(p, "id")}), |r| r)
}

fn a_route_collision(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let cid = s(p, "cid").unwrap_or_default();
    let dest = normalize_dest(s(p, "dest").as_deref());
    let (false, Some(dest)) = (cid.is_empty(), dest) else {
        return Err(Error::invalid(
            "dest 不合法（repo:<id> | group:<g> | incubator）",
        ));
    };
    let events = spine::iter_events(d)?;
    let idx = collision_index(&events);
    // Python 版 opened 不在時會炸成 500；這裡一樣當「找不到碰撞」
    let Some((rec, opened)) = idx
        .get(&cid)
        .and_then(|r| r.opened.as_ref().map(|o| (r, o)))
    else {
        return Err(Error::invalid(format!("找不到碰撞: {cid}")));
    };
    let judged = events.iter().find(|e| {
        e.etype == EventType::Collision
            && e.kv("id") == Some(cid.as_str())
            && e.body.starts_with("判定：")
    });
    let text = format!(
        "# 碰撞 {cid}\n\n## 想法\n{}\n\n## 判定\n{}\n\n（來源：{} {}｜範圍 {}）\n",
        rec.idea,
        judged.map(|e| e.body.as_str()).unwrap_or("（尚無判定）"),
        opened.date,
        opened.time,
        rec.group
            .as_deref()
            .filter(|g| !g.is_empty())
            .unwrap_or("全部")
    );
    let target = route::route(d, &text, &dest, &format!("collision-{cid}"), None)?;
    let fname = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    spine::append_event(
        d,
        "chosen",
        "monitor",
        &[format!("ref:collision:{cid}")],
        &format!("落點：{dest} → {fname}"),
        None,
    )?;
    Ok(json!({"ok": true, "cid": cid, "dest": dest, "file": target.to_string_lossy()}))
}

fn a_tag(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    via_op(
        d,
        "registry_tag",
        json!({"id": raw(p, "id"), "add": list(p, "add"), "remove": list(p, "remove")}),
        |r| r,
    )
}

/// 只帶 repo id 的 action 參數（名片、變更、審查共用；子模組在 `namecard`／`finder`／`briefs`）。
#[derive(serde::Deserialize)]
struct ProfileArgs {
    id: String,
}

fn a_save_group(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let Some(repos) = list(p, "repos") else {
        return Err(Error::invalid("至少勾選一個 repo"));
    };
    via_op(
        d,
        "group_add",
        json!({"name": raw(p, "name").unwrap_or_default(), "members": repos}),
        |r| obj(vec![("name", r["name"].clone())]),
    )
}

fn a_update_group(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    // 編輯牌組：勾選結果＝新名單；new_name 不同才改名——一個 transaction，失敗不留半套
    let Some(repos) = list(p, "repos") else {
        return Err(Error::invalid("至少勾選一個 repo（要整組拿掉請用刪除）"));
    };
    let rename = raw(p, "new_name")
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    via_op(
        d,
        "group_update",
        json!({"name": raw(p, "name").unwrap_or_default().trim(), "members": repos, "rename": rename}),
        |r| {
            obj(vec![
                ("name", r["group"]["name"].clone()),
                ("members", r["group"]["members"].clone()),
            ])
        },
    )
}

fn a_remove_group(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    via_op(
        d,
        "group_remove",
        json!({"name": raw(p, "name").unwrap_or_default().trim()}),
        |r| r,
    )
}

fn a_set_goal(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    if raw(p, "group").unwrap_or_default().trim().is_empty() {
        return Err(Error::invalid(
            "目標只能記在已儲存的牌組上；先把這個範圍存成牌組",
        ));
    }
    via_op(
        d,
        "group_goal",
        json!({"name": raw(p, "group"), "text": raw(p, "text").unwrap_or_default()}),
        |r| r,
    )
}

fn a_scan_dir(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let base = raw(p, "dir").unwrap_or_default().trim().to_string();
    if base.is_empty() {
        return Err(Error::invalid("請填要掃描的資料夾路徑"));
    }
    // Rust 才有：選到的資料夾本身就是 repo 時也列進候選（App「加入 repo」可以直接選 repo 資料夾）
    let found = registry::scan_dir_with_self(d, &base)?;
    let candidates: Vec<Value> = found
        .into_iter()
        .map(|(path, id)| json!({"path": path.to_string_lossy(), "id": id}))
        .collect();
    Ok(json!({"ok": true, "dir": base, "candidates": candidates}))
}

/// Rust 才有（2026-10-11）：側欄拖曳排序。`kind`＝groups（`order`＝組名）｜members（`group`＋repo id）｜
/// repos（「所有 repo」＝registry 順序）。只換順序、不記 spine 事件。
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum ReorderArgs {
    Groups { order: Vec<String> },
    Members { group: String, order: Vec<String> },
    Repos { order: Vec<String> },
}

fn a_reorder(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    match parse("reorder", p)? {
        ReorderArgs::Groups { order } => registry::reorder_groups(d, &order)?,
        ReorderArgs::Members { group, order } => registry::reorder_members(d, &group, &order)?,
        ReorderArgs::Repos { order } => registry::reorder_repos(d, &order)?,
    }
    Ok(json!({"ok": true}))
}

/// Rust 才有（F-63）：App 助手——一句話 → 驗證過的計畫（不執行；App 照計畫呼叫原本的 action）。
fn a_assist_plan(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let text = raw(p, "text").unwrap_or_default();
    let scope = crate::assist::Scope {
        group: s(p, "group"),
        repos: list(p, "repos"),
    };
    let plan = crate::assist::plan(d, &text, &scope)?;
    let mut v = serde_json::to_value(&plan)?;
    if let Some(o) = v.as_object_mut() {
        o.insert("ok".into(), json!(true));
        o.insert("text".into(), json!(text.trim()));
    }
    Ok(v)
}

/// Rust 才有（F-68）：最近用過、真的做了事的助手指令（⌘K「?」空白時列出來，一鍵再跑）。
fn a_assist_recent(d: &Path, _p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    Ok(json!({"ok": true, "texts": crate::assist::recent(d, 5)?}))
}

/// Rust 才有（F-63）：助手執行完落一筆 presented 事件（指令、做了哪些、略過哪些）。
/// 各步驟本身的留痕由原本的 action 負責，這裡只記「誰用哪句話做了這些」。
fn a_assist_log(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let text = raw(p, "text").unwrap_or_default();
    let done = list(p, "done").unwrap_or_default();
    let skipped = list(p, "skipped").unwrap_or_default();
    let failed = s(p, "failed");
    let mut toks = Vec::new();
    if let Some(g) = s(p, "group") {
        toks.push(format!("group:{g}"));
    }
    let mut body = vec![format!("助手：{}", text.trim())];
    if !done.is_empty() {
        body.push(format!("做了：{}", done.join("；")));
    }
    if !skipped.is_empty() {
        body.push(format!("略過：{}", skipped.join("；")));
    }
    if let Some(f) = failed {
        body.push(format!("失敗停下：{f}"));
    }
    spine::append_event(d, "presented", "assist", &toks, &body.join("\n"), None)?;
    Ok(json!({"ok": true}))
}

fn notes_json(d: &Path, group: &str, notes: &[crate::notes::Note]) -> Value {
    let items: Vec<Value> = notes
        .iter()
        .map(|n| json!({"at": n.at, "text": n.text}))
        .collect();
    json!({"ok": true, "group": group,
           "path": crate::notes::path(d, group).to_string_lossy(), "notes": items})
}

/// Rust 才有（2026-10-11）：牌組筆記（`groups/<組>/notes.md`，見 [`crate::notes`]）。
/// 牌組筆記的參數（空的牌組名由 `notes` 模組報錯）。
#[derive(serde::Deserialize)]
struct NoteArgs {
    #[serde(default)]
    group: String,
    #[serde(default)]
    text: String,
    /// 要刪的那則的時間（`note_remove`）。
    #[serde(default)]
    at: String,
}

fn notes_of(d: &Path, group: &str) -> Result<Value> {
    Ok(notes_json(d, group, &crate::notes::list(d, group)?))
}

fn a_group_notes(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let a: NoteArgs = parse("group_notes", p)?;
    notes_of(d, a.group.trim())
}

fn a_note_add(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let a: NoteArgs = parse("note_add", p)?;
    crate::notes::add(d, a.group.trim(), &a.text, None)?;
    notes_of(d, a.group.trim())
}

fn a_note_remove(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let a: NoteArgs = parse("note_remove", p)?;
    crate::notes::remove(d, a.group.trim(), &a.at, &a.text)?;
    notes_of(d, a.group.trim())
}

/// Rust 才有（2026-10-11）：像 IDE 的 Add Folder to Workspace——一個 repo 資料夾加進牌組
/// （沒登記就先登記；`group` 空＝只登記，給臨時組用）。
fn a_group_add_folder(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let Some(path) = s(p, "path")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    else {
        return Err(Error::invalid("請選一個資料夾"));
    };
    let group = s(p, "group")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let (id, registered) = registry::add_folder(d, &path)?;
    if let Some(g) = &group {
        registry::add_group_members(d, g, &[id.as_str()])?;
    }
    if registered || group.is_some() {
        let tokens: Vec<String> = group.iter().map(|g| format!("group:{g}")).collect();
        let body = match (&group, registered) {
            (Some(g), true) => format!("登記 repo {id} 並加入牌組「{g}」（從工作台）"),
            (Some(g), false) => format!("把 {id} 加入牌組「{g}」（從工作台）"),
            (None, _) => format!("登記 repo {id}（從工作台）"),
        };
        spine::append_event(d, "decision", "monitor", &tokens, &body, None)?;
    }
    Ok(json!({"ok": true, "id": id, "registered": registered, "group": group}))
}

fn a_register_repos(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let items = p
        .get("repos")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        return Err(Error::invalid("至少選一個 repo 才建得起工作範圍"));
    }
    let name = raw(p, "group").unwrap_or_default().trim().to_string();
    let mut added: Vec<String> = Vec::new();
    for it in &items {
        let rid = raw(it, "id").unwrap_or_default().trim().to_string();
        let path = raw(it, "path").unwrap_or_default().trim().to_string();
        if rid.is_empty() || path.is_empty() {
            // Python 另帶 added（部分成功）；IPC 只回錯誤訊息（IPC.md）
            return Err(Error::invalid("每個 repo 都需要 id 與路徑"));
        }
        registry::add_repo(d, &rid, &path, registry::AddRepo::default())?;
        added.push(rid);
    }
    if !name.is_empty() {
        registry::add_group(d, &name, &added)?;
    }
    let tokens: Vec<String> = if name.is_empty() {
        Vec::new()
    } else {
        vec![format!("group:{name}")]
    };
    let body = format!(
        "登記 {} 個 repo：{}{}（從工作台）",
        added.len(),
        added.join("、"),
        if name.is_empty() {
            String::new()
        } else {
            format!("，建立牌組「{name}」")
        }
    );
    spine::append_event(d, "decision", "monitor", &tokens, &body, None)?;
    Ok(json!({"ok": true, "added": added, "group": (!name.is_empty()).then_some(name)}))
}

fn a_brief(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let (out, text) = brief::run(
        d,
        s(p, "group").as_deref(),
        list(p, "repos").as_deref(),
        None,
    )?;
    Ok(json!({"ok": true, "text": text, "file": out.to_string_lossy()}))
}
