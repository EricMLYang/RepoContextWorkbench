//! 工作台的行動按鈕 → 引擎原語（每個 action 一個函式，[`ACTIONS`] 表分派）。對應 Python `webui/actions.py`。
//!
//! registry／組的寫入走操作表（`ops`）：參數驗證、留痕跟 CLI／MCP 同一份，這裡只轉回傳形狀。
//! 新增 action：寫一個 `a_<名稱>(spine, payload, terms)`，加進 [`ACTIONS`]。
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
    ("write_file", a_write_file),
    ("group_notes", a_group_notes),
    ("note_add", a_note_add),
    ("note_remove", a_note_remove),
    ("brief", a_brief),
    ("term_create", a_term_create),
    ("term_kill", a_term_kill),
    ("session_result", a_session_result),
];

/// 不寫入任何東西的 action：殼不發 `workbench://changed`（否則展開資料夾、開檔都會讓前端重抓整份 state）。
/// Rust 才有（2026-10-13，原本寫死在殼、漏了檔案樹與檢視器的讀取）。
const READ_ONLY: &[&str] = &[
    "session_draft",
    "scan_dir",
    "repo_tree",
    "read_file",
    "group_notes",
    "repo_profile",
    "palette",
    "pick_brief",
    "profile_draft_brief",
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

/// 會改變 repo 採集結果的 action（tier、延後問句卡、移除 repo）：結果多帶 `rescan: true`，前端照它重新採集。
/// Rust 才有（2026-10-13，原本前端寫死這份清單）。
const RESCAN_AFTER: &[&str] = &["tier", "defer", "remove_repo"];

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

/// Rust 才有（F-57）：一個 repo 的名片（介紹、角色、關鍵字；沒填介紹時從 README 推一句，標 `readme`）。
#[derive(serde::Deserialize)]
struct ProfileArgs {
    id: String,
}

fn a_repo_profile(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let ProfileArgs { id } = parse("repo_profile", p)?;
    profile_json(&registry::get_repo(d, &id)?, None)
}

fn profile_json(r: &registry::Repo, changed: Option<bool>) -> Result<Value> {
    let mut v = serde_json::to_value(crate::profile::profile(r))?;
    v["ok"] = json!(true);
    if let Some(c) = changed {
        v["changed"] = json!(c);
    }
    Ok(v)
}

/// Rust 才有（F-57）：App 編輯名片。表單送三欄的完整內容，**只有跟 registry 不同的欄位**才寫
/// （走 `registry_describe`，同 CLI／MCP 的驗證與留痕）；都沒變就不寫、不記事件。
/// 關鍵字可用逗號、頓號、全形逗號或換行分隔，去重、保留順序。回傳更新後的名片＋`changed`。
#[derive(serde::Deserialize)]
struct DescribeArgs {
    id: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    keywords: Option<String>,
}

fn split_keywords(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for k in s.split([',', '，', '、', '\n']).map(str::trim) {
        if !k.is_empty() && !out.iter().any(|o| o == k) {
            out.push(k.to_string());
        }
    }
    out
}

fn a_describe_repo(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let a: DescribeArgs = parse("describe_repo", p)?;
    let cur = registry::get_repo(d, &a.id)?;
    let mut args = Map::new();
    for (k, new, old) in [
        ("description", &a.description, cur.description()),
        ("role", &a.role, cur.role()),
    ] {
        if let Some(n) = new.as_deref().map(str::trim) {
            if n != old.as_deref().map(str::trim).unwrap_or("") {
                args.insert(k.into(), json!(n));
            }
        }
    }
    if let Some(k) = a.keywords.as_deref().map(split_keywords) {
        if k != cur.keywords() {
            args.insert("keywords".into(), json!(k));
        }
    }
    if args.is_empty() {
        return profile_json(&cur, Some(false));
    }
    args.insert("id".into(), json!(a.id));
    ops::call(d, "registry_describe", Value::Object(args), Via::Ui)?;
    profile_json(&registry::get_repo(d, &a.id)?, Some(true))
}

/// Rust 才有（2026-10-13）：App 的 ⌘K——打字找 repo（名稱部分字串、縮寫、名片內容）與牌組（名稱、目標）。
/// 排名在 [`crate::profile::quick_find`]；前端只畫。repo 多帶它在哪些牌組。
#[derive(serde::Deserialize)]
struct PaletteArgs {
    #[serde(default)]
    query: String,
    /// 找檔案的範圍（畫面目前的範圍；都沒有＝所有沒封存的 repo）。repo 與牌組不受範圍限制。
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    repos: Option<Vec<String>>,
}

/// ⌘K 一次最多列幾個 repo（打字會再縮小範圍）。
const PALETTE_LIMIT: usize = 30;
/// ⌘K 一次最多列幾個檔案。
const PALETTE_FILES: usize = 20;
/// 打幾個字以上才找檔案（一個字會中一大堆）。
const PALETTE_FILE_MIN_CHARS: usize = 2;

fn a_palette(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let PaletteArgs {
        query,
        group,
        repos: scope_repos,
    } = parse("palette", p)?;
    let data = registry::load(d)?;
    let profiles: Vec<_> = data.repos.iter().map(crate::profile::profile).collect();
    let q = query.trim().to_lowercase();
    let repos: Vec<Value> = crate::profile::quick_find(&profiles, &q, PALETTE_LIMIT)
        .into_iter()
        .map(|h| {
            let groups: Vec<String> = data
                .groups
                .iter()
                .filter(|g| g.members().contains(&h.repo))
                .map(|g| g.name())
                .collect();
            let mut v = serde_json::to_value(&h)?;
            v["groups"] = json!(groups);
            Ok(v)
        })
        .collect::<Result<_>>()?;
    let groups: Vec<Value> = data
        .groups
        .iter()
        .filter(|g| {
            let goal = g.get_str("goal").unwrap_or_default();
            q.is_empty() || g.name().to_lowercase().contains(&q) || goal.to_lowercase().contains(&q)
        })
        .map(|g| {
            json!({"name": g.name(), "members": g.members().len(),
                   "goal": g.get_str("goal").unwrap_or_default()})
        })
        .collect();
    let files = if q.chars().count() >= PALETTE_FILE_MIN_CHARS {
        palette_files(&data, &q, group.as_deref(), scope_repos.as_deref())
    } else {
        Vec::new()
    };
    Ok(json!({"ok": true, "query": query, "repos": repos, "groups": groups, "files": files}))
}

/// ⌘K 的檔案結果：範圍內每個 repo 用檔名找（各 repo 平行跑 `git ls-files`），合起來照分數排。
fn palette_files(
    data: &registry::RegistryData,
    q: &str,
    group: Option<&str>,
    scope_repos: Option<&[String]>,
) -> Vec<Value> {
    let ids: Vec<String> = match (scope_repos, group) {
        (Some(r), _) => r.to_vec(),
        (None, Some(g)) => data
            .groups
            .iter()
            .find(|x| x.name() == g)
            .map(|x| x.members())
            .unwrap_or_default(),
        (None, None) => data
            .repos
            .iter()
            .filter(|r| r.tier() != Some(registry::Tier::Archived))
            .map(|r| r.id())
            .collect(),
    };
    let roots: Vec<(String, PathBuf)> = ids
        .iter()
        .filter_map(|id| data.repo(id).map(|r| (id.clone(), r.path())))
        .collect();
    let mut hits: Vec<(String, PathBuf, crate::filetree::FileHit)> = std::thread::scope(|sc| {
        let jobs: Vec<_> = roots
            .iter()
            .map(|(id, root)| {
                sc.spawn(move || {
                    crate::filetree::find_files(root, q, PALETTE_FILES)
                        .into_iter()
                        .map(|h| (id.clone(), root.clone(), h))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        jobs.into_iter()
            .flat_map(|j| j.join().unwrap_or_default())
            .collect()
    });
    hits.sort_by(|a, b| {
        b.2.score
            .cmp(&a.2.score)
            .then(a.2.rel.len().cmp(&b.2.rel.len()))
    });
    hits.truncate(PALETTE_FILES);
    hits.into_iter()
        .map(|(id, root, h)| {
            json!({"repo": id, "path": h.rel, "name": h.name,
                   "abs": root.join(&h.rel).to_string_lossy()})
        })
        .collect()
}

/// Rust 才有（2026-10-14，痛點 2「挑幾份文件丟給 agent」）：從檔案樹挑的檔案 → 會話任務文字＋`@路徑`。
/// 每個路徑都要在它的 repo 裡（同檢視器的檢查）；重複的只算一次、照挑的順序依 repo 分段，
/// 段落標題帶名片（角色、介紹），agent 不用再問「這是哪個 repo」。
#[derive(serde::Deserialize)]
struct PickBriefArgs {
    files: Vec<PickedFile>,
}

#[derive(serde::Deserialize)]
struct PickedFile {
    id: String,
    path: String,
}

/// 一次最多交幾個檔（再多就該用組情境卡或任務包，不是逐一挑）。
const PICK_MAX: usize = 50;

fn a_pick_brief(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let PickBriefArgs { files } = parse("pick_brief", p)?;
    if files.is_empty() {
        return Err(Error::invalid("還沒挑任何檔案"));
    }
    if files.len() > PICK_MAX {
        return Err(Error::invalid(format!(
            "一次最多 {PICK_MAX} 個檔案（挑了 {} 個）；要整組的脈絡請用組情境卡",
            files.len()
        )));
    }
    let data = registry::load(d)?;
    // repo → 挑到的絕對路徑（保留第一次出現的順序）
    let mut by_repo: Vec<(registry::Repo, Vec<String>)> = Vec::new();
    for f in &files {
        let repo = data
            .repo(&f.id)
            .ok_or_else(|| Error::invalid(format!("repo 不存在: {}", f.id)))?;
        let abs = crate::filetree::locate(&repo.path(), &f.path)?
            .to_string_lossy()
            .into_owned();
        match by_repo.iter_mut().find(|(r, _)| r.id() == f.id) {
            Some((_, list)) if list.contains(&abs) => {}
            Some((_, list)) => list.push(abs),
            None => by_repo.push((repo.clone(), vec![abs])),
        }
    }
    let mut text = String::from("先讀這幾份檔案（從工作台挑的），讀完再開始：\n");
    let mut mentions = Vec::new();
    for (repo, list) in &by_repo {
        let card = crate::profile::profile(repo);
        let mut head = format!("\n## {}", card.id);
        if let Some(role) = &card.role {
            head.push_str(&format!("（{role}）"));
        }
        if let Some(desc) = &card.description {
            head.push_str(&format!(" — {desc}"));
        }
        text.push_str(&head);
        text.push('\n');
        for abs in list {
            text.push_str(&format!("- {abs}\n"));
            mentions.push(format!("@{abs}"));
        }
    }
    let repos: Vec<String> = by_repo.iter().map(|(r, _)| r.id()).collect();
    Ok(
        json!({"ok": true, "text": text, "mentions": mentions.join("\n"),
              "repos": repos, "count": mentions.len()}),
    )
}

/// Rust 才有（F-57 P2）：請 agent 批次起草名片的任務文字。`ids` 沒給＝還沒寫介紹、沒封存的 repo（最多
/// [`DRAFT_MAX`] 個）。任務要求只讀 repo、先列草稿給人確認、確認後才用 `ctx registry describe` 寫——
/// 名片是人要背書的，agent 只起草。附上已用過的角色，讓新寫的跟既有的用詞一致。
#[derive(serde::Deserialize)]
struct DraftArgs {
    #[serde(default)]
    ids: Option<Vec<String>>,
}

/// 一次最多請 agent 起草幾張名片（多了人審不完）。
const DRAFT_MAX: usize = 10;

fn a_profile_draft_brief(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let DraftArgs { ids } = parse("profile_draft_brief", p)?;
    let data = registry::load(d)?;
    let picked: Vec<&registry::Repo> = match ids {
        Some(ids) => ids
            .iter()
            .map(|id| {
                data.repo(id)
                    .ok_or_else(|| Error::invalid(format!("repo 不存在: {id}")))
            })
            .collect::<Result<_>>()?,
        None => data
            .repos
            .iter()
            .filter(|r| r.tier() != Some(registry::Tier::Archived) && r.description().is_none())
            .collect(),
    };
    if picked.is_empty() {
        return Err(Error::invalid("沒有要起草的 repo（都寫了介紹）"));
    }
    let picked = &picked[..picked.len().min(DRAFT_MAX)];
    let mut roles: Vec<String> = Vec::new();
    for r in &data.repos {
        if let Some(role) = r.role() {
            if !roles.contains(&role) {
                roles.push(role);
            }
        }
    }
    let mut text = String::from(
        "替這幾個 repo 起草「名片」（一句話介紹、協作角色、搜尋關鍵字）——repo 多了以後靠它找到、\
         同組的 agent 開場靠它認得彼此：\n\n",
    );
    for r in picked {
        let card = crate::profile::profile(r);
        text.push_str(&format!("- {}：{}", card.id, card.path));
        if let (Some(desc), Some(crate::profile::DescSource::Readme)) =
            (&card.description, card.description_source)
        {
            text.push_str(&format!("（README 第一句：{desc}）"));
        }
        text.push('\n');
    }
    if !roles.is_empty() {
        text.push_str(&format!(
            "\n已經用過的角色（能沿用就沿用，用詞一致才好找）：{}\n",
            roles.join("、")
        ));
    }
    text.push_str(
        "\n做法：\n\
         1. 每個 repo 讀 README／AGENTS.md、目錄結構、最近的 commit。**只讀，不改 repo 裡任何檔。**\n\
         2. 先把草稿列成表給我看（id｜角色｜介紹｜關鍵字）：介紹一句話，說這個 repo 是什麼、別的 repo 來這裡找什麼；\
         角色一個短詞；關鍵字 3～6 個。等我確認或修改。\n\
         3. 我確認後，逐一用 `ctx registry describe <id> --description \"…\" --role \"…\" --keywords \"a,b,c\"` 寫入\
         （存在 spine 的 registry，不寫進 repo）。\n",
    );
    let repos: Vec<String> = picked.iter().map(|r| r.id()).collect();
    Ok(json!({"ok": true, "text": text, "repos": repos}))
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

fn a_session_draft(d: &Path, p: &Value, t: Option<&dyn Terms>) -> Result<Value> {
    let sid = raw(p, "sid").unwrap_or_default();
    let Some(info) = t.and_then(|t| t.info(&sid)) else {
        return Err(Error::invalid(format!("未知會話: {}", shown(p, "sid"))));
    };
    let mut out = Map::new();
    out.insert("ok".into(), json!(true));
    if let Value::Object(m) = session_draft(d, &term_value(&info))? {
        out.extend(m);
    }
    Ok(Value::Object(out))
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

/// Rust 才有（2026-10-11）：repo 詳情的檔案清單，一次一層（見 [`crate::filetree`]）。
#[derive(serde::Deserialize)]
struct RepoTreeArgs {
    id: String,
    /// 相對 repo 根目錄；空＝根。
    #[serde(default)]
    dir: String,
}

fn a_repo_tree(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let RepoTreeArgs { id, dir } = parse("repo_tree", p)?;
    let root = registry::get_repo(d, &id)?.path();
    let l = crate::filetree::list_dir(&root, &dir)?;
    let entries: Vec<Value> = l
        .entries
        .iter()
        .map(|e| {
            json!({"name": e.name, "path": e.rel.to_string_lossy(),
                   "abs": e.abs.to_string_lossy(), "dir": e.dir})
        })
        .collect();
    Ok(
        json!({"ok": true, "id": id, "root": root.to_string_lossy(), "dir": dir,
              "entries": entries, "more": l.more}),
    )
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

/// Rust 才有（2026-10-11）：唯讀檢視器讀一個檔（見 [`crate::filetree::read_file`]）。
#[derive(serde::Deserialize)]
struct ReadFileArgs {
    id: String,
    path: String,
}

fn a_read_file(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let ReadFileArgs { id, path } = parse("read_file", p)?;
    let root = registry::get_repo(d, &id)?.path();
    Ok(file_json(&id, crate::filetree::read_file(&root, &path)?))
}

fn file_json(id: &str, f: crate::filetree::FileRead) -> Value {
    let mut out = json!({"ok": true, "id": id, "path": f.rel.to_string_lossy(),
                         "abs": f.abs.to_string_lossy(), "size": f.size});
    let extra = match f.view {
        crate::filetree::FileView::Text {
            text,
            truncated,
            editable,
            version,
        } => {
            json!({"kind": "text", "text": text, "truncated": truncated,
                   "editable": editable, "version": version})
        }
        crate::filetree::FileView::Image { mime, base64 } => {
            json!({"kind": "image", "mime": mime, "base64": base64})
        }
        crate::filetree::FileView::Binary => json!({"kind": "binary"}),
        crate::filetree::FileView::TooLarge => json!({"kind": "too_large"}),
    };
    if let (Value::Object(o), Value::Object(e)) = (&mut out, extra) {
        o.extend(e);
    }
    out
}

/// Rust 才有（2026-10-13）：檢視器簡單編輯（見 [`crate::filetree::write_file`]）。
/// 不記 spine 事件——改檔的痕跡在 git。磁碟上已被改過回 `{ok: false, conflict: true}`。
#[derive(serde::Deserialize)]
struct WriteFileArgs {
    id: String,
    path: String,
    text: String,
    /// 讀檔時的 `version`。
    base: String,
}

fn a_write_file(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let WriteFileArgs {
        id,
        path,
        text,
        base,
    } = parse("write_file", p)?;
    let root = registry::get_repo(d, &id)?.path();
    Ok(
        match crate::filetree::write_file(&root, &path, &text, &base)? {
            crate::filetree::WriteOutcome::Saved(f) => file_json(&id, f),
            crate::filetree::WriteOutcome::Conflict => {
                json!({"ok": false, "conflict": true, "id": id, "path": path})
            }
        },
    )
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

/// 對單一 repo 開的會話屬於哪個牌組（只給畫面歸類，Rust 才有，2026-10-13 從前端搬來）：
/// `hint`（前端目前選的組）有這個 repo 就是它；否則只有一個組有它才算；都不成立就沒有。不是對單一 repo 開就照 `hint`。
fn home_group_for(d: &Path, hint: Option<String>, repo: Option<&str>) -> Result<Option<String>> {
    let Some(repo) = repo else {
        return Ok(hint);
    };
    let owners: Vec<String> = registry::load(d)?
        .groups
        .iter()
        .filter(|g| g.members().iter().any(|m| m == repo))
        .map(|g| g.name())
        .collect();
    Ok(match hint {
        Some(h) if owners.contains(&h) => Some(h),
        _ if owners.len() == 1 => owners.into_iter().next(),
        _ => None,
    })
}

fn a_term_create(d: &Path, p: &Value, t: Option<&dyn Terms>) -> Result<Value> {
    let Some(terms) = t else {
        return Err(Error::invalid("本 server 未啟用 terminal"));
    };
    let kind = raw(p, "kind").unwrap_or_else(|| "shell".into());
    let (sid, title) = if kind == "agent" {
        let group = s(p, "group");
        let repos = list(p, "repos");
        let repo = s(p, "repo");
        let task = s(p, "task");
        let agent = s(p, "agent").unwrap_or_else(|| "claude".into());
        let home = home_group_for(d, s(p, "home_group"), repo.as_deref())?;
        let plan = session::build(
            d,
            SessionOpts {
                group: group.as_deref(),
                repos: repos.as_deref(),
                repo: repo.as_deref(),
                task: task.as_deref(),
                agent: Some(&agent),
            },
        )?;
        // 會話狀態（Rust 才有）：Claude Code 多帶一份只有三個 hook 的設定（做完／要你允許／你送出），
        // 不動使用者的 ~/.claude/settings.json；寫不出設定檔就照舊開（只是看不到「等你」）
        let mut argv = plan.cmd;
        if plan.claude_code && !argv.is_empty() {
            let file =
                crate::exe::ctx_exe().and_then(|exe| crate::session_status::settings_file(d, &exe));
            if let Ok(f) = file {
                argv.insert(1, format!("--settings={}", f.display()));
            }
        }
        terms.create_agent(AgentLaunch {
            agent,
            argv,
            cwd: plan.cwd,
            env: Vec::new(),
            group,
            repos,
            repo,
            task,
            origin: s(p, "origin"),
            home_group: home,
        })?
    } else if let Some(of) = s(p, "of") {
        // 某個會話的 Terminal：開在同一個目錄，origin 記 `shell-of:<sid>`，畫面據此配對切換
        let cwd = terms
            .cwd(&of)
            .ok_or_else(|| Error::invalid(format!("未知會話: {of}")))?;
        terms.create_shell(Some(&cwd), Some(format!("shell-of:{of}")))?
    } else {
        let cwd = s(p, "cwd").map(PathBuf::from);
        terms.create_shell(cwd.as_deref(), s(p, "origin"))?
    };
    Ok(json!({"ok": true, "sid": sid, "title": title}))
}

fn a_term_kill(_d: &Path, p: &Value, t: Option<&dyn Terms>) -> Result<Value> {
    let sid = raw(p, "sid").unwrap_or_default();
    if !t.is_some_and(|t| t.kill(&sid)) {
        return Err(Error::invalid(format!("未知會話: {}", shown(p, "sid"))));
    }
    Ok(ok())
}

fn a_session_result(d: &Path, p: &Value, t: Option<&dyn Terms>) -> Result<Value> {
    let sid = raw(p, "sid").unwrap_or_default();
    let session = t.and_then(|t| t.info(&sid));
    let text = raw(p, "text").unwrap_or_default().trim().to_string();
    let (Some(session), false) = (session, text.is_empty()) else {
        return Err(Error::invalid("請選擇會話並填寫結果"));
    };
    let origin = session.origin.clone().unwrap_or_default();
    let mut tokens: Vec<String> = if origin.starts_with("collision:") {
        vec![format!("ref:{origin}")]
    } else {
        Vec::new()
    };
    let reg = registry::load(d)?;
    let scope = session.scope.clone().unwrap_or_default();
    if reg.group(&scope).is_some() {
        tokens.push(format!("group:{scope}"));
    } else if reg.repo(&scope).is_some() {
        tokens.push(format!("repo:{scope}"));
    } else if let Some(m) = session.scope_repos.as_ref().filter(|m| !m.is_empty()) {
        tokens.push(format!("group:臨時({})", m.join(",")));
    }
    // A session note is a decision record, not evidence of an adopted idea.
    let ev = spine::append_event(
        d,
        "decision",
        "monitor",
        &tokens,
        &format!("會話結果：{}\n{text}", session.title),
        None,
    )?;
    Ok(json!({"ok": true, "date": ev.date, "time": ev.time}))
}
