//! 對講（2026-11-04，ideation F-121；Rust 才有）：不同 repo 的 agent 互傳訊息——真正的多 repo 協作。
//!
//! 以前跨 repo 只有「讀」（`read_from`）跟「問一次」（`ask_repo`，另起 `claude -p`、慢又花錢）；
//! 兩個 repo 各有一個 agent 在跑時，要靠人在兩個終端之間複製貼上。這裡讓 agent 直接留話給另一個 repo：
//!
//! - **送**：`send_message(to, text, re?)`——留痕是一筆 `suggestion [agent] id:<日期>-m<N> repo:<收件 repo>`，
//!   第一行 `〔對講〕A → B：…`（不新增事件型別：資料格式是合約，Python 對拍版也讀得懂）；收件匣自然有一張卡。
//! - **收**：送達狀態放 `.state/mailbox.json`（快取級，跟 spine 事件一一對應）。收件 repo 有 App 開的會話
//!   在跑時，下一次用完工具（`PostToolUse`）或你送出訊息（`UserPromptSubmit`）就由 hook 把訊息塞進它的脈絡；
//!   會話閒著時，**回覆**由 App 直接打進原本發問的那場會話（它是自己開口問的，打斷算掙得）；新的請求不自動打斷，
//!   收件匣卡片給「交給 X 的會話」「開會話處理」。都沒有會話：那邊下次開場注入「收到的訊息」，或 agent 呼叫 `messages`。
//! - **看得到彼此**：`messages` 也回同組現在在跑的 agent（repo、標題、在做什麼），要改共用的東西前先打聲招呼。
//!
//! 每則訊息只送達一次（第一個拿到的會話）；回覆指定回原本的會話（`to_sid`），那場還活著就只給它。

use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    agent_error, append, clean, event_dict, home_ids, next_id, resolve_path, where_am_i, Target,
};
use crate::status::AgentStatus;
use crate::{registry, spine, Result};

/// 對講事件第一行的前綴（收件匣、搜尋、開場都靠它認）。
pub const TAG: &str = "〔對講〕";
/// 一則最多幾個字元（要更多就寫成檔、傳路徑）。
const TEXT_MAX: usize = 2000;
/// 送達過的保留幾天；狀態檔最多幾則。
const KEEP_DAYS: i64 = 14;
const KEEP_MAX: usize = 300;
/// 一次塞進脈絡最多幾則、每則幾個字元（其餘說「另有 N 則，呼叫 messages」）。
const DELIVER_MAX: usize = 5;
const DELIVER_CHARS: usize = 600;

/// 一則訊息（`.state/mailbox.json` 的一項）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Msg {
    pub id: String,
    /// `YYYY-MM-DD HH:MM`
    pub at: String,
    pub from: String,
    pub to: String,
    /// 收件 repo 的根目錄（解析過的絕對路徑）：hook 只比路徑，不必每次讀 registry
    pub to_path: String,
    pub text: String,
    /// 回覆哪一則
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub re: Option<String>,
    /// 發話的 App 會話（回覆要送回它）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_sid: Option<String>,
    /// 指定送到哪場 App 會話（回覆＝原發問者）；那場已經不在就誰先拿到給誰
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_sid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered: Option<Delivered>,
}

/// 什麼時候、經由哪條路、交給哪場會話。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delivered {
    pub at: String,
    pub via: Via,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
}

/// 送達的路。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Via {
    /// 會話用完工具／你送出訊息時，hook 塞進脈絡
    Hook,
    /// 開場注入
    Start,
    /// agent 自己呼叫 `messages`
    Mcp,
    /// App 打進閒著的會話（回覆）或人按「交給 X 的會話」
    App,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Book {
    #[serde(default)]
    messages: Vec<Msg>,
}

fn book_path(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("mailbox.json")
}

fn read_book(spine_dir: &Path) -> Book {
    std::fs::read_to_string(book_path(spine_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn stamp(now: NaiveDateTime) -> String {
    now.format("%Y-%m-%d %H:%M").to_string()
}

fn now_or(now: Option<NaiveDateTime>) -> NaiveDateTime {
    now.unwrap_or_else(|| Local::now().naive_local())
}

/// 舊的、送達過的收掉（沒送達的永遠留著，直到有人收）。
fn prune(b: &mut Book, now: NaiveDateTime) {
    let cutoff = stamp(now - chrono::Duration::days(KEEP_DAYS));
    b.messages
        .retain(|m| m.delivered.is_none() || m.at.as_str() >= cutoff.as_str());
    let extra = b.messages.len().saturating_sub(KEEP_MAX);
    if extra > 0 {
        // 先丟最舊的已送達
        let mut dropped = 0;
        b.messages.retain(|m| {
            if dropped < extra && m.delivered.is_some() {
                dropped += 1;
                false
            } else {
                true
            }
        });
    }
}

/// App 會話的 sid：從 [`crate::session_status::ENV`] 給的狀態檔路徑取檔名（`<sid>.json`）。
pub fn sid_from_env() -> Option<String> {
    let p = std::env::var(crate::session_status::ENV).ok()?;
    sid_of_status_path(Path::new(p.trim()))
}

pub fn sid_of_status_path(p: &Path) -> Option<String> {
    let stem = p.file_stem()?.to_string_lossy().into_owned();
    (!stem.is_empty()).then_some(stem)
}

/// 要送出的一則：收件 repo（回覆可省略）、內文、回覆哪一則、發話的 App 會話。
#[derive(Debug, Clone, Copy, Default)]
pub struct Outgoing<'a> {
    pub to: Option<&'a str>,
    pub text: &'a str,
    pub re: Option<&'a str>,
    pub from_sid: Option<&'a str>,
}

/// 送一則（`send_message`）。`to` 空＋帶 `re`＝回給那則的發話 repo。收件 repo 要在你的主場（同組），
/// 跨組要使用者同意（`cross_scope_ok`）。`from_sid`＝發話的 App 會話（MCP 從環境變數拿）。
pub fn send_message(
    spine_dir: &Path,
    out: &Outgoing,
    t: &Target,
    cross_scope_ok: bool,
    now: Option<NaiveDateTime>,
) -> Result<Value> {
    let Outgoing {
        to,
        text,
        re,
        from_sid,
    } = *out;
    let now = now_or(now);
    let text = clean(text, "text")?;
    if text.chars().count() > TEXT_MAX {
        return Err(agent_error(
            crate::RejectCode::BadRequest,
            format!("訊息超過 {TEXT_MAX} 字"),
            Some("長的內容寫成檔案（或指出 repo 裡的路徑），訊息裡只放摘要與路徑。"),
        ));
    }
    let w = where_am_i(spine_dir, t.cwd.as_deref())?;
    let Some(from) = w.repo.as_ref().map(|r| r.id.clone()) else {
        return Err(agent_error(
            crate::RejectCode::BadRequest,
            "你所在的目錄沒登記成 repo，對方不知道是誰在說話".to_string(),
            w.hint.as_deref(),
        ));
    };
    let book = read_book(spine_dir);
    let orig = re
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(|r| {
            book.messages
                .iter()
                .find(|m| m.id == r)
                .cloned()
                .ok_or_else(|| {
                    agent_error(
                        crate::RejectCode::NotFound,
                        format!("找不到訊息 {r}"),
                        Some("用 messages 看最近的訊息與編號。"),
                    )
                })
        })
        .transpose()?;
    let to = match (to.map(str::trim).filter(|s| !s.is_empty()), &orig) {
        (Some(x), _) => x.to_string(),
        (None, Some(o)) => o.from.clone(),
        (None, None) => {
            return Err(agent_error(
                crate::RejectCode::BadRequest,
                "要給哪個 repo？帶 to（repo id），回覆則帶 re".to_string(),
                Some("同組有哪些 repo：context_for 的 roster，或 messages 的 peers。"),
            ))
        }
    };
    let repo = registry::get_repo(spine_dir, &to).map_err(|_| {
        agent_error(
            crate::RejectCode::NotFound,
            format!("沒有登記叫 {to} 的 repo"),
            Some("repo id 看 context_for 的 roster（同組成員）。"),
        )
    })?;
    if !cross_scope_ok {
        if let Some(home) = home_ids(spine_dir, &w)? {
            if !home.contains(&to) {
                return Err(agent_error(
                    crate::RejectCode::CrossScope,
                    format!("{to} 不在你的主場（{from} 所屬的組）"),
                    Some("先問使用者要不要跨組傳話；同意後再帶 cross_scope_ok=true 重試。"),
                ));
            }
        }
    }
    // 收發兩邊共同的組（留痕歸在那組；沒有就只標 repo）
    let group = registry::load(spine_dir)?
        .groups
        .iter()
        .find(|g| w.groups.contains(&g.name()) && g.members().contains(&to))
        .map(|g| g.name());
    let events = spine::iter_events(spine_dir)?;
    let id = next_id(&events, &now.format("%Y-%m-%d").to_string(), "m");
    let mut toks = vec![format!("id:{id}")];
    if let Some(g) = &group {
        toks.push(format!("group:{g}"));
    }
    toks.push(format!("repo:{to}"));
    let re_note = orig
        .as_ref()
        .map(|o| format!("（回 {}）", o.id))
        .unwrap_or_default();
    let body = format!("{TAG}{from} → {to}{re_note}：{text}");
    let ev = append(spine_dir, "suggestion", "agent", &toks, &body)?;
    let to_path = resolve_path(&repo.path()).to_string_lossy().into_owned();
    let msg = Msg {
        id: id.clone(),
        at: stamp(now),
        from: from.clone(),
        to: to.clone(),
        to_path,
        text,
        re: orig.as_ref().map(|o| o.id.clone()),
        from_sid: from_sid.map(str::to_string),
        to_sid: orig.as_ref().and_then(|o| o.from_sid.clone()),
        delivered: None,
    };
    crate::locking::update_json_locked(spine_dir, &book_path(spine_dir), |b: &mut Book| {
        b.messages.push(msg.clone());
        prune(b, now);
    })?;
    let live: Vec<Value> = peers(spine_dir)
        .into_iter()
        .filter(|p| p["repo"] == json!(to))
        .collect();
    let delivery = if live.is_empty() && msg.re.is_some() {
        format!("回覆已留給 {to}：發問的會話不在了，那邊下次開場或呼叫 messages 時會看到。")
    } else if live.is_empty() {
        format!(
            "{to} 現在沒有 agent 會話在跑：使用者收件匣有一張卡（可一鍵開會話處理），那邊下次開場也會看到。不用等，繼續你的工作。"
        )
    } else if msg.to_sid.is_some() {
        "會送回發問的那場會話（它用完下一個工具時，或閒著時由 App 直接交給它）。".to_string()
    } else {
        format!(
            "{to} 有 {} 個會話在跑：它用完下一個工具、或使用者對它說話時就會看到。對方回覆會送回你這場會話。",
            live.len()
        )
    };
    Ok(json!({
        "ok": true,
        "id": id,
        "to": to,
        "re": msg.re,
        "delivery": delivery,
        "peers_in_target": live,
        "event": event_dict(&ev),
    }))
}

/// 收件 repo 的路徑包含 `cwd`？（會話可能開在 repo 的子目錄）
fn addressed_here(m: &Msg, cwd: &Path) -> bool {
    !m.to_path.is_empty() && cwd.starts_with(Path::new(&m.to_path))
}

/// 這場會話（`sid`，在 `cwd`）現在該收的訊息：還沒送達、收件 repo 包含 cwd、指定給別場而那場還在的不給。
/// 拿到就標成已送達（同一則只給一次）。先不鎖讀一次：大部分時候沒有信，hook 不必等鎖。
pub fn take_for(
    spine_dir: &Path,
    cwd: &Path,
    sid: Option<&str>,
    via: Via,
    now: Option<NaiveDateTime>,
) -> Vec<Msg> {
    let cwd = resolve_path(cwd);
    let alive = |s: &str| {
        crate::session_status::read(spine_dir, s).is_some_and(|v| v["status"] != "exited")
    };
    let wanted = |m: &Msg| {
        m.delivered.is_none()
            && addressed_here(m, &cwd)
            && match (&m.to_sid, sid) {
                (Some(t), Some(me)) if t == me => true,
                (Some(t), _) => !alive(t),
                (None, _) => true,
            }
    };
    if !read_book(spine_dir).messages.iter().any(wanted) {
        return Vec::new();
    }
    let at = stamp(now_or(now));
    crate::locking::update_json_locked(spine_dir, &book_path(spine_dir), |b: &mut Book| {
        let mut out = Vec::new();
        for m in b.messages.iter_mut().filter(|m| wanted(m)) {
            m.delivered = Some(Delivered {
                at: at.clone(),
                via,
                sid: sid.map(str::to_string),
            });
            out.push(m.clone());
        }
        out
    })
    .unwrap_or_default()
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!(
            "{}…（全文：messages）",
            s.chars().take(n).collect::<String>()
        )
    }
}

/// 塞進 agent 脈絡的那段文字（hook 的 additionalContext、開場注入共用）。
pub fn render(msgs: &[Msg]) -> String {
    if msgs.is_empty() {
        return String::new();
    }
    let mut t = String::from("## 其他 repo 的 agent 傳話給你（repo-context 對講）\n");
    for m in msgs.iter().take(DELIVER_MAX) {
        let re =
            m.re.as_deref()
                .map(|r| format!("，回你的 {r}"))
                .unwrap_or_default();
        t.push_str(&format!(
            "- [{}] {} → {}（{}{re}）：{}\n",
            m.id,
            m.from,
            m.to,
            m.at,
            clip(&m.text, DELIVER_CHARS)
        ));
    }
    if msgs.len() > DELIVER_MAX {
        t.push_str(&format!(
            "- 另有 {} 則：呼叫 messages 看全部\n",
            msgs.len() - DELIVER_MAX
        ));
    }
    t.push_str(
        "處理方式：是回覆就接著用；是請求就判斷合不合理、在不在你這個 repo 的範圍——要改檔或做大事先跟使用者確認，\
         做完（或做不了）用 repo-context 的 MCP 工具 send_message（不是內建的 SendMessage；沒掛 MCP 就 `ctx msg send --re <編號> \"…\"`）帶 re=<編號> 回覆對方，一句話講結果與 commit／檔案路徑。不要為了回覆中斷使用者正交代的事。",
    );
    t
}

/// hook 輸出（`PostToolUse`／`UserPromptSubmit`）：有信就回 Claude Code 的 JSON，沒信回空字串。
pub fn hook_output(event_name: &str, msgs: &[Msg]) -> String {
    if msgs.is_empty() {
        return String::new();
    }
    json!({"hookSpecificOutput": {"hookEventName": event_name, "additionalContext": render(msgs)}})
        .to_string()
}

/// App 會話的 hook（`ctx hook tool-end`／`prompt`）順手收信：`target`＝狀態檔路徑（[`crate::session_status::ENV`]），
/// spine 從它推（`<spine>/.state/session_status/<sid>.json`），cwd 從 hook 輸入拿。回傳要印的 JSON（沒信＝空）。
/// 永遠不失敗：hook 壞掉不能擋住 agent。
pub fn deliver_on_hook(event_name: &str, payload: &Value, target: Option<&str>) -> String {
    let Some(target) = target.map(str::trim).filter(|t| !t.is_empty()) else {
        return String::new();
    };
    let status = Path::new(target);
    let Some(spine_dir) = status
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
    else {
        return String::new();
    };
    if !book_path(spine_dir).exists() {
        return String::new();
    }
    let Some(cwd) = payload
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
    else {
        return String::new();
    };
    let sid = sid_of_status_path(status);
    let msgs = take_for(spine_dir, Path::new(cwd), sid.as_deref(), Via::Hook, None);
    hook_output(event_name, &msgs)
}

/// 現在在跑的 agent 會話（App 開的，看 `.state/session_status`）：repo、標題、狀態、在用的工具。
/// 沒登記的目錄略過。給 `messages` 與送信回報「對方有沒有人在」。
pub fn peers(spine_dir: &Path) -> Vec<Value> {
    let Ok(rd) = std::fs::read_dir(crate::session_status::dir(spine_dir)) else {
        return Vec::new();
    };
    let Ok(data) = registry::load(spine_dir) else {
        return Vec::new();
    };
    let roots: Vec<(String, PathBuf)> = data
        .repos
        .iter()
        .map(|r| (r.id(), resolve_path(&r.path())))
        .collect();
    let mut out = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // <sid>.json 才是狀態檔（.meter.json 是用量）
        if !name.ends_with(".json") || name.ends_with(".meter.json") {
            continue;
        }
        let Some(v) = std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        else {
            continue;
        };
        let status = v["status"].as_str().unwrap_or("");
        if status.is_empty() || status == "exited" {
            continue;
        }
        let Some(cwd) = v["cwd"].as_str().map(|c| resolve_path(Path::new(c))) else {
            continue;
        };
        let Some((repo, _)) = roots
            .iter()
            .filter(|(_, root)| cwd.starts_with(root))
            .max_by_key(|(_, root)| root.as_os_str().len())
        else {
            continue;
        };
        let label = match AgentStatus::parse(status) {
            Some(AgentStatus::Working) => "處理中",
            Some(AgentStatus::Blocked) => "要使用者允許／回答",
            Some(AgentStatus::Waiting) => "等使用者回應",
            Some(AgentStatus::Finished | AgentStatus::Done) => "這一輪做完了（閒著）",
            _ => status,
        };
        let mut peer = json!({
            "sid": sid_of_status_path(&p),
            "repo": repo,
            "status": status,
            "state": label,
            "at": v["at"],
        });
        for k in ["title", "text"] {
            if let Some(s) = v[k].as_str().filter(|s| !s.is_empty()) {
                peer[k] = json!(s);
            }
        }
        if let Some(tool) = v["tool"]["label"].as_str() {
            peer["tool"] = json!(tool);
        }
        out.push(peer);
    }
    out.sort_by(|a, b| a["repo"].as_str().cmp(&b["repo"].as_str()));
    out
}

/// `messages`：我這個 repo 收到的（沒送達的這次算送達）與送出的最近幾則，加上同組現在在跑的 agent。
pub fn messages(spine_dir: &Path, t: &Target, sid: Option<&str>, limit: usize) -> Result<Value> {
    let w = where_am_i(spine_dir, t.cwd.as_deref())?;
    let Some(me) = w.repo.as_ref().map(|r| r.id.clone()) else {
        return Err(agent_error(
            crate::RejectCode::BadRequest,
            "你所在的目錄沒登記成 repo".to_string(),
            w.hint.as_deref(),
        ));
    };
    let cwd = PathBuf::from(&w.cwd);
    let new = take_for(spine_dir, &cwd, sid, Via::Mcp, None);
    let new_ids: Vec<&str> = new.iter().map(|m| m.id.as_str()).collect();
    let book = read_book(spine_dir);
    let limit = limit.clamp(1, 50);
    let mine: Vec<Value> = book
        .messages
        .iter()
        .rev()
        .filter(|m| m.to == me || m.from == me)
        .take(limit)
        .map(|m| {
            let mut v = serde_json::to_value(m).unwrap_or(Value::Null);
            v["direction"] = json!(if m.to == me { "in" } else { "out" });
            v["new"] = json!(new_ids.contains(&m.id.as_str()));
            if let Some(o) = v.as_object_mut() {
                o.remove("to_path");
            }
            v
        })
        .collect();
    let home = home_ids(spine_dir, &w)?.unwrap_or_default();
    let peers: Vec<Value> = peers(spine_dir)
        .into_iter()
        .filter(|p| p["repo"].as_str().is_some_and(|r| home.contains(r)))
        .filter(|p| sid.is_none() || p["sid"].as_str() != sid)
        .collect();
    Ok(json!({
        "ok": true,
        "repo": me,
        "new": new.len(),
        "messages": mine,
        "peers": peers,
        "hint": "回覆：send_message 帶 re=<編號>；要請別的 repo 做事：send_message 帶 to=<repo id>。",
    }))
}

/// 收件匣／App 用：某一則訊息（找不到＝None）。
pub fn get(spine_dir: &Path, id: &str) -> Option<Msg> {
    read_book(spine_dir)
        .messages
        .into_iter()
        .find(|m| m.id == id)
}

/// 人按「交給 X 的會話」或 App 打進閒著的會話之後：標成已送達。
pub fn mark_delivered(spine_dir: &Path, id: &str, sid: Option<&str>, via: Via) -> Result<bool> {
    let at = stamp(Local::now().naive_local());
    crate::locking::update_json_locked(spine_dir, &book_path(spine_dir), |b: &mut Book| {
        match b.messages.iter_mut().find(|m| m.id == id) {
            Some(m) if m.delivered.is_none() => {
                m.delivered = Some(Delivered {
                    at,
                    via,
                    sid: sid.map(str::to_string),
                });
                true
            }
            _ => false,
        }
    })
}

/// 交進終端的那句（App 打進閒著的會話、人按「交給 X 的會話」共用）：單行，送出後 agent 自己會讀。
pub fn as_prompt(m: &Msg) -> String {
    let kind = if m.re.is_some() { "回覆" } else { "傳話" };
    let one_line = m.text.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(
        "〔對講〕{} 的 agent {kind}（{}）：{} ——處理完用 repo-context 的 send_message（MCP；或 `ctx msg send --re {} \"…\"`）帶 re 回覆它。",
        m.from,
        m.id,
        clip(&one_line, DELIVER_CHARS),
        m.id
    )
}

/// App 定時（spine 有變動就跑）：**回覆**指定給某場 App 會話、那場閒著（做完／等你）→ 回傳 (sid, 要打的字)，
/// 並標成已送達。新請求不自動打斷（收件匣卡片給人決定）；要你允許／處理中的不打（處理中會由 hook 送到）。
pub fn idle_replies(spine_dir: &Path, live_sids: &[String]) -> Vec<(String, String)> {
    let book = read_book(spine_dir);
    let mut out = Vec::new();
    for m in book
        .messages
        .iter()
        .filter(|m| m.delivered.is_none() && m.re.is_some())
    {
        let Some(sid) = m.to_sid.as_deref() else {
            continue;
        };
        if !live_sids.iter().any(|s| s == sid) {
            continue;
        }
        let idle = crate::session_status::read(spine_dir, sid)
            .and_then(|v| v["status"].as_str().map(str::to_string))
            .is_some_and(|s| {
                s == AgentStatus::Finished.as_str() || s == AgentStatus::Waiting.as_str()
            });
        if !idle {
            continue;
        }
        if mark_delivered(spine_dir, &m.id, Some(sid), Via::App).unwrap_or(false) {
            out.push((sid.to_string(), as_prompt(m)));
        }
    }
    out
}

/// App 定時跑（跟自動繼續同一個時機）：把回覆打進閒著的發問會話。回傳打了幾則。
pub fn tick(spine_dir: &Path, terms: &crate::term::TermManager) -> usize {
    if !book_path(spine_dir).exists() {
        return 0;
    }
    let live: Vec<String> = terms
        .list()
        .into_iter()
        .filter(|i| i.alive && i.kind == crate::term::TermKind::Agent)
        .map(|i| i.sid)
        .collect();
    idle_replies(spine_dir, &live)
        .into_iter()
        .filter(|(sid, text)| crate::auto_continue::send_line(terms, sid, text))
        .count()
}

/// 人在收件匣按「交給在跑的會話」：挑收件 repo 裡一場還活著的 agent 會話（閒著的優先），把訊息打進去、標成已送達。
/// `live`＝App 現在活著的 agent 會話 sid。回傳交給了哪場（沒有可交的＝None）。
pub fn pick_session(spine_dir: &Path, m: &Msg, live: &[String]) -> Option<String> {
    let mut cands: Vec<Value> = peers(spine_dir)
        .into_iter()
        .filter(|p| p["repo"] == json!(m.to))
        .filter(|p| {
            p["sid"]
                .as_str()
                .is_some_and(|s| live.iter().any(|l| l == s))
        })
        .filter(|p| p["status"] != "blocked")
        .collect();
    // 閒著（做完／等你）在前，處理中在後
    cands.sort_by_key(|p| if p["status"] == "working" { 1 } else { 0 });
    cands
        .first()
        .and_then(|p| p["sid"].as_str().map(str::to_string))
}

/// 會話列用（F-121）：這場 App 會話（`sid`，`started` 之後）送出與收到的對講——`label` 給一個標籤
/// （「等 rb 回覆」優先，否則「對講 N 則」），`lines` 給滑過去看的往來（新的在前，最多 5 行）。沒有就 None。
pub fn session_talk(spine_dir: &Path, sid: &str, started: &str) -> Option<Value> {
    let book = read_book(spine_dir);
    let mine = |m: &Msg| {
        m.at.as_str() >= started
            && (m.from_sid.as_deref() == Some(sid)
                || m.to_sid.as_deref() == Some(sid)
                || m.delivered.as_ref().and_then(|d| d.sid.as_deref()) == Some(sid))
    };
    let msgs: Vec<&Msg> = book.messages.iter().filter(|m| mine(m)).collect();
    if msgs.is_empty() {
        return None;
    }
    let answered = |id: &str| book.messages.iter().any(|m| m.re.as_deref() == Some(id));
    let waiting: Vec<&str> = msgs
        .iter()
        .filter(|m| m.from_sid.as_deref() == Some(sid) && m.re.is_none() && !answered(&m.id))
        .map(|m| m.to.as_str())
        .collect();
    let label = match waiting.as_slice() {
        [] => format!("對講 {} 則", msgs.len()),
        [one] => format!("等 {one} 回覆"),
        many => format!("等 {} 個 repo 回覆", many.len()),
    };
    let lines: Vec<String> = msgs
        .iter()
        .rev()
        .take(5)
        .map(|m| {
            let short: String = m.text.chars().take(60).collect();
            let hm = m.at.get(11..).unwrap_or(&m.at);
            if m.from_sid.as_deref() == Some(sid) {
                let st = if m.delivered.is_some() {
                    "已送到"
                } else {
                    "還沒送到"
                };
                format!("→ {}（{hm}，{st}）：{short}", m.to)
            } else {
                format!("← {}（{hm}）：{short}", m.from)
            }
        })
        .collect();
    Some(json!({"label": label, "waiting": !waiting.is_empty(), "lines": lines}))
}

/// 收件匣只留**還沒送到**的請求（無事不報）：回覆是給發問的 agent 的、已送到的請求對方 agent 已經在處理——
/// 人要看往來去會話列（`session_talk`）或 `ctx msg list`。信箱裡找不到的（太舊被收掉）照常留著。
pub fn settle_unread(spine_dir: &Path, events: &mut Vec<spine::Event>) {
    if !book_path(spine_dir).exists() {
        return;
    }
    let book = read_book(spine_dir);
    events.retain(|ev| {
        if ev.source != "agent" || !ev.body.starts_with(TAG) {
            return true;
        }
        let Some(id) = ev.kv("id") else {
            return true;
        };
        match book.messages.iter().find(|m| m.id == id) {
            Some(m) => m.re.is_none() && m.delivered.is_none(),
            None => true,
        }
    });
}

/// `ctx health` 用：期間內（`at >= since`）的對講數字——請求幾則、有回覆的幾則、還沒送到幾則、各條送達路各幾則。
/// 看的是「agent 會不會主動用」與「送不送得到」（R39 留的問題）。信箱只留 14 天的已送達，期間更長時會少算。
pub fn stats(spine_dir: &Path, since: &str) -> Value {
    let book = read_book(spine_dir);
    let ms: Vec<&Msg> = book
        .messages
        .iter()
        .filter(|m| m.at.as_str() >= since)
        .collect();
    let requests: Vec<&&Msg> = ms.iter().filter(|m| m.re.is_none()).collect();
    let answered = requests
        .iter()
        .filter(|r| {
            book.messages
                .iter()
                .any(|m| m.re.as_deref() == Some(r.id.as_str()))
        })
        .count();
    let mut via = serde_json::Map::new();
    for m in &ms {
        if let Some(d) = &m.delivered {
            let k = serde_json::to_value(d.via)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let n = via.get(&k).and_then(Value::as_u64).unwrap_or(0);
            via.insert(k, json!(n + 1));
        }
    }
    json!({
        "requests": requests.len(),
        "replies": ms.len() - requests.len(),
        "answered": answered,
        "undelivered": ms.iter().filter(|m| m.delivered.is_none()).count(),
        "via": via,
    })
}

/// 收件匣卡片用：沒送達的請求（不含回覆）——卡片給「交給 X 的會話／開會話處理」。
pub fn undelivered(spine_dir: &Path) -> Vec<Msg> {
    read_book(spine_dir)
        .messages
        .into_iter()
        .filter(|m| m.delivered.is_none())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, to_path: &str) -> Msg {
        Msg {
            id: id.into(),
            at: "2026-11-04 23:00".into(),
            from: "a".into(),
            to: "b".into(),
            to_path: to_path.into(),
            text: "請幫忙".into(),
            re: None,
            from_sid: None,
            to_sid: None,
            delivered: None,
        }
    }

    #[test]
    fn render_lists_and_caps() {
        let ms: Vec<Msg> = (0..7).map(|i| msg(&format!("x-m{i}"), "/r")).collect();
        let t = render(&ms);
        assert!(t.contains("[x-m0] a → b"));
        assert!(t.contains("另有 2 則"));
        assert!(t.contains("re=<編號>"));
        assert_eq!(render(&[]), "");
        assert_eq!(hook_output("PostToolUse", &[]), "");
        let h: Value = serde_json::from_str(&hook_output("PostToolUse", &ms[..1])).unwrap();
        assert_eq!(h["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    }

    #[test]
    fn addressed_by_path_prefix() {
        let m = msg("x", "/w/b");
        assert!(addressed_here(&m, Path::new("/w/b")));
        assert!(addressed_here(&m, Path::new("/w/b/sub")));
        assert!(!addressed_here(&m, Path::new("/w/bb")));
        assert!(!addressed_here(&m, Path::new("/w")));
    }

    #[test]
    fn prune_keeps_undelivered() {
        let mut b = Book::default();
        let mut old = msg("old", "/r");
        old.at = "2026-01-01 00:00".into();
        let mut old_done = old.clone();
        old_done.id = "old-done".into();
        old_done.delivered = Some(Delivered {
            at: "2026-01-01 00:01".into(),
            via: Via::Hook,
            sid: None,
        });
        b.messages = vec![old, old_done];
        let now = NaiveDateTime::parse_from_str("2026-11-04 23:00", "%Y-%m-%d %H:%M").unwrap();
        prune(&mut b, now);
        assert_eq!(b.messages.len(), 1);
        assert_eq!(b.messages[0].id, "old");
    }

    #[test]
    fn prompt_is_one_line() {
        let mut m = msg("2026-11-04-m1", "/r");
        m.text = "第一行\n第二行".into();
        let p = as_prompt(&m);
        assert!(!p.contains('\n'));
        assert!(p.contains("--re 2026-11-04-m1"));
    }
}
