//! 工作台的行動按鈕 → 引擎原語（每個 action 一個函式，[`ACTIONS`] 表分派）。對應 Python `webui/actions.py`。
//!
//! registry／組的寫入走操作表（`ops`）：參數驗證、留痕跟 CLI／MCP 同一份，這裡只轉回傳形狀。
//! 新增 action：寫一個 `a_<名稱>(spine, payload, terms)`，加進 [`ACTIONS`]。
//!
//! 錯誤形狀：Python 回 `{"error": "..."}`；這裡回 `Err(Error::Invalid("..."))`，訊息逐字相同
//! （桌面殼把它轉成 IPC 的 `Err(String)`）。會話相關 action 透過 [`Terms`] 拿會話清冊——
//! 引擎不依賴 Tauri，殼把 `TermManager` 傳進來即可（測試可給假清冊）。

use std::path::Path;

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
}

type ActionFn = fn(&Path, &Value, Option<&dyn Terms>) -> Result<Value>;

/// 對應 `actions.ACTIONS`（順序照抄）。
pub const ACTIONS: &[(&str, ActionFn)] = &[
    ("ack", a_ack),
    ("relate", a_relate),
    ("unrelate", a_unrelate),
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
    ("save_group", a_save_group),
    ("update_group", a_update_group),
    ("remove_group", a_remove_group),
    ("set_goal", a_set_goal),
    ("session_draft", a_session_draft),
    ("scan_dir", a_scan_dir),
    ("register_repos", a_register_repos),
    ("brief", a_brief),
    ("term_create", a_term_create),
    ("term_kill", a_term_kill),
    ("session_result", a_session_result),
];

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
    f(spine_dir.as_ref(), payload, terms)
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
    let found = registry::scan_dir(d, &base)?;
    let candidates: Vec<Value> = found
        .into_iter()
        .map(|(path, id)| json!({"path": path.to_string_lossy(), "id": id}))
        .collect();
    Ok(json!({"ok": true, "dir": base, "candidates": candidates}))
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
        terms.create_agent(AgentLaunch {
            agent,
            argv: plan.cmd,
            cwd: plan.cwd,
            env: Vec::new(),
            group,
            repos,
            repo,
            task,
            origin: s(p, "origin"),
        })?
    } else {
        let cwd = s(p, "cwd").map(std::path::PathBuf::from);
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
