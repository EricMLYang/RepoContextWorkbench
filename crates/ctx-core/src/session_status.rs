//! 每個 App 會話「現在輪到誰」（2026-10-14，Rust 才有）：agent 在做、還是在等你。
//!
//! 多個 agent 會話同時跑時，最要緊的是一眼看出**哪個在等我**。MCP `report_status` 要 agent 自己記得回報，
//! 而且依範圍存（同範圍兩個會話會互蓋）；這裡改由 Claude Code 自己的 hook 自動寫、依會話存：
//! - `Stop`（這一輪做完）→ `finished`「這一輪做完了」（F-80：做完不等於等你——要你核准／回答才是；
//!   你看過那個會話後 App 呼叫 [`mark_seen`] 記 `seen`，「做完沒看」跟「看過了」才分得開；下一個 hook 事件重寫整份、`seen` 自然消失）
//! - `Notification`（依通知種類分開掛）：要你允許工具或回答（`permission_prompt`、`elicitation_*`、
//!   `agent_needs_input`）→ `blocked`（agent 停住了，App 視窗不在前面時發系統通知）；閒著等輸入
//!   （`idle_prompt`）→ `waiting`。都帶 Claude Code 給的訊息；其他種類（登入成功等）不動狀態
//! - `PermissionRequest`（F-81：要你允許哪個工具、做什麼）→ `blocked`「Bash：cargo test」——只記錄、不回決定，
//!   提示照常交回 TUI；同一分鐘內跟著來的 `Notification(permission_prompt)` 不蓋掉這句比較具體的
//! - `PreToolUse`／`PostToolUse`（F-82）→ `working`＋`tool: {label, since}`「處理中：Bash：cargo test」／工具結束就拿掉——
//!   App 看得出它在做什麼、做多久；超過 [`STALL_MINUTES`] 沒結束＝「可能卡住」（讀端算，見 `workbench::state::sessions`）
//! - `UserPromptSubmit`（你送出訊息）→ `working`
//!
//! 只有 App 開的 Claude Code 會話會寫：開會話時多帶 `--settings=<spine>/.state/session_hooks.json`
//! （只掛這三個 hook，不動使用者的 `~/.claude/settings.json`）與環境變數 [`ENV`]＝這個會話的狀態檔路徑。
//! hook 照路徑寫，不必找 spine。會話結束就刪；App 重開時全部清掉（sid 每次重開從頭編）。
//!
//! 檔案（`.state/session_status/<sid>.json`）：`{"status","text","at","event"}`，`at` 是 `YYYY-MM-DD HH:MM`。
//! 2026-10-16（ideation F-71）多兩個選填欄位，會話列一眼分得出誰是誰、做完說了什麼：
//! `title`＝Claude Code 自己取的對話標題（逐字稿的 `ai-title`，之後的事件沿用）、
//! `reply`＝這一輪最後的回覆（一行、截 [`REPLY_MAX`] 字；`Stop` 才寫，你送出新訊息就清掉）。
//! 都從 hook 給的 `transcript_path` 讀尾巴（只讀不寫）。
//! F-78：另記 `claude_session`（agent CLI 自己的會話 id；Codex 的也記在這個鍵，另有 `agent: "codex"`）與 `cwd`；程序結束時不刪檔，改寫成
//! `status: "exited"`（[`retire`]）——已結束的會話列據此給「接續」（`claude --resume`）。
//!
//! F-130（2026-11-18）狀態雙通道：除了 hook，agent 也可能自己在終端輸出裡報狀態（OSC 7501，見
//! [`crate::program_status`]）。App 的 PTY 讀到就寫**並排的另一個檔** `<sid>.program.json`（[`ProgramRecorder`]）——
//! hook 是另一個行程、每次重寫整份 `<sid>.json`，兩個來源寫同一個檔會互蓋，分開就各寫各的。
//! 程式回報的檔跟 hook 檔同形（`status`／`text`／`at`／`event`，讀端不必分兩套），多 `source: "program"`、
//! `ts`（毫秒）與 `program`（原始回報：state、kind、app、msg、progress、title）。hook 檔也多兩個欄位：
//! `source: "hook"`、`ts`（毫秒，比 `at` 精確，優先規則用）；舊檔沒有這兩個欄位照樣讀（`ts` 退回檔案修改時間）。
//! 哪一個算數由 [`pick_source`] 決定，讀端用 [`read_resolved`]。

use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime};
use serde_json::{json, Value};

use crate::program_status::{BlockedKind, ProgramReport, ProgramState};
use crate::status::AgentStatus;
use crate::Result;

/// App 開的 agent 會話帶的環境變數：這個會話的狀態檔絕對路徑。
pub const ENV: &str = "REPO_CONTEXT_SESSION_STATUS";

/// App 會話掛的 Claude Code hook（`ctx hook <arg>`）。settings 檔怎麼寫、CLI 怎麼認、狀態怎麼對應，都從這裡查，
/// 字串只出現一次（[`StatusHook::arg`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusHook {
    /// Claude Code `Stop`：這一輪做完 → finished
    Stop,
    /// `Notification` 的 `idle_prompt`：閒著等輸入 → waiting
    Notification,
    /// `UserPromptSubmit`：你送出 → working
    Prompt,
    /// `Notification` 的 permission_prompt／elicitation_*／agent_needs_input：要你允許或回答 → blocked
    Permission,
    /// `PermissionRequest`：要你允許哪個工具、做什麼 → blocked「Bash：…」（F-81）
    PermissionRequest,
    /// `statusLine`：額度餘光（F-83），不寫會話狀態，見 [`crate::usage`]
    Statusline,
    /// `PreToolUse`：開始用某個工具 → working＋`tool`（F-82）
    ToolStart,
    /// `PostToolUse`：工具結束 → working，拿掉 `tool`（F-82）
    ToolEnd,
    /// `StopFailure`：這一輪因 API 錯誤（額度、過載…）中斷 → waiting「這一輪出錯中斷：…」（F-99）
    StopFailure,
    /// `SubagentStart`：開了一個子 agent → 記一個標記檔，不動狀態（F-102）
    SubagentStart,
    /// `SubagentStop`：子 agent 結束 → 拿掉標記檔（F-102）
    SubagentStop,
}

impl StatusHook {
    /// `ctx hook <arg>` 的 arg，也是狀態檔 `event` 欄位的值。
    pub fn arg(self) -> &'static str {
        match self {
            StatusHook::Stop => "stop",
            StatusHook::Notification => "notification",
            StatusHook::Prompt => "prompt",
            StatusHook::Permission => "permission",
            StatusHook::PermissionRequest => "permission-request",
            StatusHook::Statusline => "statusline",
            StatusHook::ToolStart => "tool-start",
            StatusHook::ToolEnd => "tool-end",
            StatusHook::StopFailure => "stop-failure",
            StatusHook::SubagentStart => "subagent-start",
            StatusHook::SubagentStop => "subagent-stop",
        }
    }
}
/// 訊息最多留幾個字元（Notification 的訊息可能很長）。
const TEXT_MAX: usize = 120;
/// 一個工具跑超過幾分鐘沒結束就標「可能卡住」（F-82；cargo build 這種也會超過，所以只是提醒，不是錯誤）。
pub const STALL_MINUTES: i64 = 10;
/// 最後回覆預覽最多幾個字元。
const REPLY_MAX: usize = 160;

/// `.state/session_status`。
pub fn dir(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("session_status")
}

/// 這個會話的狀態檔。
pub fn path_for(spine_dir: &Path, sid: &str) -> PathBuf {
    dir(spine_dir).join(format!("{sid}.json"))
}

/// 讀這個會話的狀態；沒有或壞掉＝None。
pub fn read(spine_dir: &Path, sid: &str) -> Option<Value> {
    let s = std::fs::read_to_string(path_for(spine_dir, sid)).ok()?;
    serde_json::from_str::<Value>(&s)
        .ok()
        .filter(|v| v.get("status").and_then(Value::as_str).is_some())
}

/// 會話結束：刪掉它的狀態檔、用量檔、子 agent 標記（沒有就算了）。
pub fn clear(spine_dir: &Path, sid: &str) {
    let p = path_for(spine_dir, sid);
    let _ = std::fs::remove_file(&p);
    drop_extras(&p);
    crate::auto_continue::off(spine_dir, sid);
}

/// 狀態檔旁邊的兩樣東西：用量檔與子 agent 標記資料夾（程序結束就沒意義了）。
fn drop_extras(status: &Path) {
    let _ = std::fs::remove_file(meter_path(status));
    remove_program(status);
    let _ = std::fs::remove_dir_all(subagents_dir(status));
}

/// 每場會話的用量（F-101）：statusLine 給的 context 用量 % 與模型，`<sid>.meter.json`（跟狀態檔分開寫，不互蓋）。
pub fn meter_path(status: &Path) -> PathBuf {
    status.with_extension("meter.json")
}

/// 子 agent 標記（F-102）：`<sid>.sub/<agent_id>`，一個子 agent 一個空檔——
/// 同時開好幾個也不會互相覆蓋計數（不做讀改寫）。
pub fn subagents_dir(status: &Path) -> PathBuf {
    status.with_extension("sub")
}

/// 子 agent 標記超過幾小時就不算（沒收到 SubagentStop：中斷、當掉）。
const SUBAGENT_STALE_HOURS: u64 = 2;

/// 這場會話現在有幾個子 agent 在跑。
pub fn subagent_count(spine_dir: &Path, sid: &str) -> usize {
    let Ok(rd) = std::fs::read_dir(subagents_dir(&path_for(spine_dir, sid))) else {
        return 0;
    };
    let fresh = |e: &std::fs::DirEntry| {
        e.metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() < SUBAGENT_STALE_HOURS * 3600)
    };
    rd.flatten().filter(fresh).count()
}

/// 這場會話的用量檔（context %、模型）；沒有＝None。
pub fn meter(spine_dir: &Path, sid: &str) -> Option<Value> {
    let s = std::fs::read_to_string(meter_path(&path_for(spine_dir, sid))).ok()?;
    serde_json::from_str(&s).ok()
}

/// statusLine 每輪都來：context 用量 % 或模型變了才寫（整數 %，一輪寫一次），回傳有沒有寫。
pub fn record_meter(status: &Path, context: Option<u64>, model: Option<&str>) -> bool {
    if context.is_none() && model.is_none() {
        return false;
    }
    let p = meter_path(status);
    let v = json!({"context": context, "model": model});
    let same = std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .is_some_and(|prev| prev == v);
    !same && crate::fsutil::write_atomic(&p, v.to_string().as_bytes()).is_ok()
}

/// SubagentStart／SubagentStop：加或拿掉一個標記檔。agent_id 只收英數與 `-_`（拼進路徑）。
fn record_subagent(status: &Path, start: bool, payload: &Value) -> bool {
    let Some(id) = payload
        .get("agent_id")
        .and_then(Value::as_str)
        .filter(|id| {
            (1..=80).contains(&id.len())
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
    else {
        return false;
    };
    let dir = subagents_dir(status);
    if start {
        std::fs::create_dir_all(&dir).is_ok() && std::fs::write(dir.join(id), b"").is_ok()
    } else {
        std::fs::remove_file(dir.join(id)).is_ok()
    }
}

/// 程序結束（F-78）：記得 Claude Code 會話 id 的就留一份 `exited`（標題、id、目錄）給「接續」用；
/// 沒有 id（hook 從沒觸發）就照舊刪掉。App 重開時 [`clear_all`] 一起清。
pub fn retire(spine_dir: &Path, sid: &str) {
    drop_extras(&path_for(spine_dir, sid));
    crate::auto_continue::off(spine_dir, sid);
    let keep = read(spine_dir, sid).filter(|v| {
        v.get("claude_session")
            .and_then(Value::as_str)
            .is_some_and(crate::history::valid_session_id)
    });
    let Some(mut v) = keep else {
        clear(spine_dir, sid);
        return;
    };
    v["status"] = json!(AgentStatus::Exited.as_str());
    v["text"] = json!("程序已結束");
    v["event"] = json!("exit");
    v["at"] = json!(Local::now()
        .naive_local()
        .format("%Y-%m-%d %H:%M")
        .to_string());
    if let Some(o) = v.as_object_mut() {
        o.remove("reply");
    }
    let _ = crate::fsutil::write_atomic(&path_for(spine_dir, sid), v.to_string().as_bytes());
}

/// 你看過這個會話了（F-80）：狀態檔記 `seen: true`。沒有狀態檔＝沒事可記（回 false）；已經記過＝不重寫。
/// 之後任何 hook 事件都會重寫整份、`seen` 跟著消失——下一輪做完又是「沒看」。
/// F-130：程式回報的檔也記（沒有 hook 的 Codex 會話，「做完沒看」才消得掉）；兩個檔有一個改了就回 true。
pub fn mark_seen(spine_dir: &Path, sid: &str) -> bool {
    let status = path_for(spine_dir, sid);
    let hook = match read(spine_dir, sid) {
        Some(mut v) if v.get("seen") != Some(&Value::Bool(true)) => {
            v["seen"] = json!(true);
            crate::fsutil::write_atomic(&status, v.to_string().as_bytes()).is_ok()
        }
        _ => false,
    };
    let program = {
        let _g = program_lock();
        let p = program_path(&status);
        match read_json(&p) {
            Some(mut v) if v.get("seen") != Some(&Value::Bool(true)) => {
                v["seen"] = json!(true);
                crate::fsutil::write_atomic(&p, v.to_string().as_bytes()).is_ok()
            }
            _ => false,
        }
    };
    hook || program
}

/// App 重開：前一次的會話都不在了（sid 會重新從頭編），全部清掉，只刪 `*.json`。
pub fn clear_all(spine_dir: &Path) {
    crate::auto_continue::clear_all(spine_dir);
    let Ok(rd) = std::fs::read_dir(dir(spine_dir)) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        match p.extension().and_then(|x| x.to_str()) {
            Some("json") => {
                let _ = std::fs::remove_file(p);
            }
            Some("sub") if p.is_dir() => {
                let _ = std::fs::remove_dir_all(p);
            }
            _ => {}
        }
    }
}

/// 同 [`clear_all`]，但 `keep` 裡的會話（F-128：herdr-core 後端接回來的、App 關著期間還在跑的）留著：
/// 它們的狀態檔、程式回報、用量、子 agent 標記、自動繼續都還算數。
pub fn clear_all_except(spine_dir: &Path, keep: &[String]) {
    if keep.is_empty() {
        clear_all(spine_dir);
        return;
    }
    // `<sid>.json`、`<sid>.program.json`、`<sid>.meter.json`、`<sid>.sub/` 的 sid 都是第一個點之前
    let kept = |p: &Path| {
        p.file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.split('.').next())
            .is_some_and(|sid| keep.iter().any(|k| k == sid))
    };
    for d in [dir(spine_dir), crate::auto_continue::dir(spine_dir)] {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if kept(&p) {
                continue;
            }
            match p.extension().and_then(|x| x.to_str()) {
                Some("json") => {
                    let _ = std::fs::remove_file(p);
                }
                Some("sub") if p.is_dir() => {
                    let _ = std::fs::remove_dir_all(p);
                }
                _ => {}
            }
        }
    }
}

/// 寫 App 會話用的 Claude Code 設定檔（會話狀態 hook＋額度餘光的 statusLine），回傳路徑。每次開會話都重寫（`ctx` 位置可能變了）。
pub fn settings_file(spine_dir: &Path, ctx: &Path) -> Result<PathBuf> {
    let p = spine_dir.join(".state").join("session_hooks.json");
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let cmd = |h: StatusHook| json!([{"type": "command", "command": crate::agentsetup::hook_command(ctx, h.arg())}]);
    // Notification 依通知種類（matcher）分開掛：要你允許／要你回答 → permission（blocked）；閒著等輸入 →
    // notification（waiting）；其他種類（登入成功、額度…）不動會話狀態。一個 matcher 一筆，不靠 regex 的「|」
    let mut notification: Vec<Value> = PERMISSION_MATCHERS
        .iter()
        .map(|m| json!({"matcher": m, "hooks": cmd(StatusHook::Permission)}))
        .collect();
    notification.push(json!({"matcher": "idle_prompt", "hooks": cmd(StatusHook::Notification)}));
    let v = json!({
        "hooks": {
            "Stop": [{"hooks": cmd(StatusHook::Stop)}],
            "Notification": notification,
            "PermissionRequest": [{"hooks": cmd(StatusHook::PermissionRequest)}],
            "PreToolUse": [{"hooks": cmd(StatusHook::ToolStart)}],
            "PostToolUse": [{"hooks": cmd(StatusHook::ToolEnd)}],
            "UserPromptSubmit": [{"hooks": cmd(StatusHook::Prompt)}],
            "StopFailure": [{"hooks": cmd(StatusHook::StopFailure)}],
            "SubagentStart": [{"hooks": cmd(StatusHook::SubagentStart)}],
            "SubagentStop": [{"hooks": cmd(StatusHook::SubagentStop)}],
        },
        // F-83：額度餘光——statusLine 每輪給的 rate_limits 寫到 .state/usage.json；這條不印東西，
        // 那場會話的終端就沒有 statusline（使用者 2026-10-16 接受）
        "statusLine": {"type": "command", "command": crate::agentsetup::hook_command(ctx, StatusHook::Statusline.arg())},
    });
    crate::fsutil::write_atomic(&p, serde_json::to_string_pretty(&v)?.as_bytes())?;
    Ok(p)
}

/// 這些通知種類＝agent 停住、要你做決定（Claude Code hooks 文件的 Notification matcher）。
const PERMISSION_MATCHERS: [&str; 4] = [
    "permission_prompt",
    "elicitation_dialog",
    "elicitation_url_dialog",
    "agent_needs_input",
];

/// hook 事件 → (階段, 文字)。statusline 不寫會話狀態＝None。
fn status_of(event: StatusHook, payload: &Value) -> Option<(AgentStatus, String)> {
    match event {
        StatusHook::Stop => Some((AgentStatus::Finished, "這一輪做完了".to_string())),
        StatusHook::Notification => {
            let text = message(payload, "需要你回應");
            let msg = text.as_str();
            // 照 matcher 掛的是 idle_prompt；萬一舊設定沒分 matcher，從欄位或訊息再判斷一次是不是要你允許
            let permission = payload.get("notification_type").and_then(Value::as_str)
                == Some("permission_prompt")
                || msg.to_lowercase().contains("permission");
            Some(if permission {
                (AgentStatus::Blocked, text)
            } else {
                (AgentStatus::Waiting, text)
            })
        }
        StatusHook::Permission => Some((AgentStatus::Blocked, message(payload, "要你允許或回答"))),
        StatusHook::PermissionRequest => Some((AgentStatus::Blocked, pending_action(payload))),
        StatusHook::Prompt | StatusHook::ToolEnd => {
            Some((AgentStatus::Working, "處理中".to_string()))
        }
        StatusHook::ToolStart => Some((
            AgentStatus::Working,
            match tool_label(payload) {
                Some(l) => format!("處理中：{l}"),
                None => "處理中".to_string(),
            },
        )),
        StatusHook::StopFailure => {
            let why = ["error_details", "error"]
                .iter()
                .find_map(|k| {
                    payload
                        .get(*k)
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                })
                .unwrap_or("API 錯誤");
            Some((
                AgentStatus::Waiting,
                crate::pystr::take_chars(&format!("這一輪出錯中斷：{why}"), TEXT_MAX).to_string(),
            ))
        }
        StatusHook::Statusline | StatusHook::SubagentStart | StatusHook::SubagentStop => None,
    }
}

/// 通知的訊息（截到 [`TEXT_MAX`]）；沒有就用 `fallback`。
fn message(payload: &Value, fallback: &str) -> String {
    let msg = payload
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(fallback);
    crate::pystr::take_chars(msg, TEXT_MAX).to_string()
}

/// `PermissionRequest` 的一句話（F-81，抄 BAT `describePendingAction`）：工具名＋人看得懂的那個欄位
/// （Bash 的指令、改檔的路徑、問題的標題…），一行、截到 [`TEXT_MAX`]。沒有工具名就只說要你允許。
fn pending_action(payload: &Value) -> String {
    tool_label(payload).unwrap_or_else(|| "要你允許工具".to_string())
}

/// 「哪個工具、做什麼」：`tool_name`＋`tool_input` 裡人看得懂的那個欄位（`PreToolUse`／`PostToolUse`／
/// `PermissionRequest` 的 payload 形狀相同）。沒有工具名＝None。
fn tool_label(payload: &Value) -> Option<String> {
    let tool = payload
        .get("tool_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())?;
    const KEYS: [&str; 8] = [
        "command",
        "file_path",
        "path",
        "notebook_path",
        "url",
        "pattern",
        "description",
        "prompt",
    ];
    let detail = payload
        .get("tool_input")
        .and_then(Value::as_object)
        .and_then(|input| {
            KEYS.iter().find_map(|k| {
                input
                    .get(*k)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
            })
        });
    let text = match detail {
        Some(d) => format!("{tool}：{}", crate::transcript::one_line(d, TEXT_MAX)),
        None => tool.to_string(),
    };
    Some(crate::pystr::take_chars(&text, TEXT_MAX).to_string())
}

/// 環境變數給的路徑像不像我們的狀態檔：`…/.state/session_status/<名稱>.json`。不像就不寫（不往任意路徑寫檔）。
pub(crate) fn valid_target(p: &Path) -> bool {
    let name_ok = p
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".json") && !n.starts_with('.'));
    let parent = p.parent();
    let dir_ok = parent
        .and_then(|d| d.file_name())
        .is_some_and(|n| n == "session_status");
    let state_ok = parent
        .and_then(Path::parent)
        .and_then(|d| d.file_name())
        .is_some_and(|n| n == ".state");
    p.is_absolute() && name_ok && dir_ok && state_ok
}

/// `ctx hook stop|notification|prompt` 的本體：`target`＝環境變數 [`ENV`] 的值（沒有＝不是 App 開的會話，什麼都不做）。
/// 回傳有沒有寫。永遠不失敗（hook 壞掉不能擋住 agent）。
pub fn record_hook(
    event: StatusHook,
    stdin: &str,
    target: Option<&str>,
    now: Option<NaiveDateTime>,
) -> bool {
    record_hook_for(event, stdin, target, now, None)
}

/// 同 [`record_hook`]，`agent`＝hook 指令帶的 `--agent`（哪一家的 CLI；None＝沒帶，沿用上一筆記的）。
/// 記進狀態檔的 `agent`：程序結束後「接續」才知道要用 `claude --resume` 還是 `codex resume`。
pub fn record_hook_for(
    event: StatusHook,
    stdin: &str,
    target: Option<&str>,
    now: Option<NaiveDateTime>,
    agent: Option<crate::config::AgentKind>,
) -> bool {
    let Some(target) = target.map(str::trim).filter(|t| !t.is_empty()) else {
        return false;
    };
    let path = PathBuf::from(target);
    if !valid_target(&path) {
        return false;
    }
    let payload = crate::hooks::parse_payload(stdin);
    if matches!(event, StatusHook::SubagentStart | StatusHook::SubagentStop) {
        return record_subagent(&path, event == StatusHook::SubagentStart, &payload);
    }
    let Some((status, text)) = status_of(event, &payload) else {
        return false;
    };
    let at = now
        .unwrap_or_else(|| Local::now().naive_local())
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let prev = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    // F-81：PermissionRequest 已經寫了「Bash：cargo test」，同一分鐘跟著來的 Notification 只會說
    // 「Claude needs your permission」——留具體的那句
    let text = match (&prev, event, status) {
        (Some(p), StatusHook::Notification | StatusHook::Permission, AgentStatus::Blocked)
            if p.get("event").and_then(Value::as_str)
                == Some(StatusHook::PermissionRequest.arg())
                && p.get("status").and_then(Value::as_str)
                    == Some(AgentStatus::Blocked.as_str())
                && p.get("at").and_then(Value::as_str) == Some(at.as_str()) =>
        {
            p.get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or(text)
        }
        _ => text,
    };
    let mut rec = json!({"status": status.as_str(), "text": text, "at": at, "event": event.arg(),
                         "source": StatusSource::Hook.as_str(), "ts": millis_of(now)});
    // F-82：正在用哪個工具、從幾點幾分幾秒開始（讀端算分鐘數與「可能卡住」）；工具結束或任何別的事件就沒有這個鍵
    if event == StatusHook::ToolStart {
        rec["tool"] = json!({
            "label": tool_label(&payload).unwrap_or_else(|| "（工具）".to_string()),
            "since": now
                .unwrap_or_else(|| Local::now().naive_local())
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        });
    }
    // F-98：每一次 Stop 一個 id——自動繼續靠它分辨「同一次做完」（看過了、重寫狀態都不會變）
    if event == StatusHook::Stop {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        rec["stop_id"] = json!(nanos.to_string());
    }
    // F-98：你自己送出的話（不是自動繼續送的那句）→ 自動繼續的計數歸零
    if event == StatusHook::Prompt {
        let said = payload.get("prompt").and_then(Value::as_str).unwrap_or("");
        crate::auto_continue::on_prompt(&path, said);
    }
    let (title, reply) = transcript_bits(event, &payload, &path);
    // F-78：接續用的 Claude Code 會話 id 與目錄（每個 hook 都帶；這次沒帶就沿用上一筆）
    // (hook 欄位, 狀態檔欄位, 合格條件)
    let pick = |from: &str, kept: &str, ok: fn(&str) -> bool| {
        payload
            .get(from)
            .and_then(Value::as_str)
            .filter(|v| ok(v))
            .or_else(|| prev.as_ref()?.get(kept)?.as_str().filter(|v| ok(v)))
            .map(str::to_string)
    };
    if let Some(id) = pick(
        "session_id",
        "claude_session",
        crate::history::valid_session_id,
    ) {
        rec["claude_session"] = json!(id);
    }
    if let Some(c) = pick("cwd", "cwd", |c| !c.is_empty()) {
        rec["cwd"] = json!(c);
    }
    let agent = agent
        .map(|a| a.name().to_string())
        .or_else(|| prev.as_ref()?.get("agent")?.as_str().map(str::to_string));
    if let Some(a) = agent {
        rec["agent"] = json!(a);
    }
    if let Some(t) = title {
        rec["title"] = json!(t);
    }
    if let Some(r) = reply {
        rec["reply"] = json!(r);
    }
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    crate::fsutil::write_atomic(&path, rec.to_string().as_bytes()).is_ok()
}

/// 標題與最後回覆：`Stop` 讀逐字稿尾巴（Claude Code 有給 `last_assistant_message` 就先用它）；
/// 其他事件沿用上一筆的標題，回覆只留到你送出下一句（`prompt`）之前。
fn transcript_bits(
    event: StatusHook,
    payload: &Value,
    path: &Path,
) -> (Option<String>, Option<String>) {
    let prev = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let prev_of = |k: &str| {
        prev.as_ref()
            .and_then(|v| v.get(k))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let tail = payload
        .get("transcript_path")
        .and_then(Value::as_str)
        .map(Path::new)
        .filter(|p| p.is_absolute() && p.extension().is_some_and(|x| x == "jsonl"))
        .map(crate::transcript::tail)
        .unwrap_or_default();
    let title = tail.title.or_else(|| prev_of("title"));
    let reply = match event {
        StatusHook::Stop => payload
            .get("last_assistant_message")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
            .or(tail.reply)
            .map(|r| crate::transcript::one_line(&r, REPLY_MAX)),
        StatusHook::Prompt => None,
        _ => prev_of("reply"),
    };
    (title, reply)
}

// ── F-130：程式回報（OSC 7501）與優先規則 ──────────────────────────────────

/// 會話狀態從哪裡來。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusSource {
    /// agent CLI 的 hook（`ctx hook …`，另一個行程寫 `<sid>.json`）
    Hook,
    /// agent 自己在終端輸出裡報的（OSC 7501，App 的 PTY 讀到後寫 `<sid>.program.json`）
    Program,
}

impl StatusSource {
    /// 狀態檔與 IPC 的 `source` 欄位。
    pub fn as_str(self) -> &'static str {
        match self {
            StatusSource::Hook => "hook",
            StatusSource::Program => "program",
        }
    }
}

/// 程式回報的檔：`<sid>.program.json`（跟 hook 寫的 `<sid>.json` 並排，不互蓋）。
pub fn program_path(status: &Path) -> PathBuf {
    status.with_extension("program.json")
}

/// 目錄裡這個檔名是不是 hook 的會話狀態檔（`<sid>.json`；用量檔、程式回報檔不算）。
pub fn is_status_file_name(name: &str) -> bool {
    name.ends_with(".json") && !name.ends_with(".meter.json") && !name.ends_with(".program.json")
}

/// 同一個行程裡寫程式回報檔的有兩處（PTY 讀取執行緒、App 的「看過了」），讀改寫要排隊。
/// 只有 App 這個行程會寫這個檔（hook 不碰它），所以行程內的鎖就夠。
static PROGRAM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn program_lock() -> std::sync::MutexGuard<'static, ()> {
    PROGRAM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn remove_program(status: &Path) {
    let _g = program_lock();
    let _ = std::fs::remove_file(program_path(status));
}

fn read_json(p: &Path) -> Option<Value> {
    let s = std::fs::read_to_string(p).ok()?;
    serde_json::from_str::<Value>(&s)
        .ok()
        .filter(|v| v.get("status").and_then(Value::as_str).is_some())
}

/// 這個會話的程式回報（沒有或壞掉＝None）。
pub fn read_program(spine_dir: &Path, sid: &str) -> Option<Value> {
    read_json(&program_path(&path_for(spine_dir, sid)))
}

/// `now`（測試給的本地時間）→ 毫秒；沒給就是現在。
fn millis_of(now: Option<NaiveDateTime>) -> i64 {
    now.and_then(|n| n.and_local_timezone(Local).earliest())
        .map(|d| d.timestamp_millis())
        .unwrap_or_else(|| Local::now().timestamp_millis())
}

/// 程式回報 → App 既有的階段、`event`、文字。`event` 沿用 hook 的字（讀端已經認得：
/// `permission`＝要你在終端允許、`stop-failure`＝這一輪出錯），一般的進度用 `program`。
/// idle／clear 不是顯示狀態＝None（見 [`ProgramRecorder::record`]）。
pub fn program_view(r: &ProgramReport) -> Option<(AgentStatus, &'static str, String)> {
    let msg = |fallback: &str| {
        let m = r.msg.as_deref().map(str::trim).filter(|m| !m.is_empty());
        crate::pystr::take_chars(m.unwrap_or(fallback), TEXT_MAX).to_string()
    };
    Some(match (r.state, r.kind) {
        (ProgramState::Working, _) => (AgentStatus::Working, "program", msg("處理中")),
        (ProgramState::Done, _) => (AgentStatus::Finished, "program", msg("這一輪做完了")),
        (ProgramState::Blocked, Some(BlockedKind::Permission)) => (
            AgentStatus::Blocked,
            StatusHook::Permission.arg(),
            msg("要你允許"),
        ),
        // 要你回答、要你登入、沒說原因：都是「等你」
        (ProgramState::Blocked, _) => (
            AgentStatus::Waiting,
            StatusHook::Notification.arg(),
            msg("等你回應"),
        ),
        (ProgramState::Error, _) => (
            AgentStatus::Waiting,
            StatusHook::StopFailure.arg(),
            match r.msg.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
                Some(m) => {
                    crate::pystr::take_chars(&format!("這一輪出錯中斷：{m}"), TEXT_MAX).to_string()
                }
                None => "這一輪出錯中斷".to_string(),
            },
        ),
        (ProgramState::Idle | ProgramState::Clear, _) => return None,
    })
}

#[derive(serde::Serialize)]
struct ProgramRecord<'a> {
    status: &'static str,
    text: String,
    at: String,
    event: &'static str,
    source: StatusSource,
    ts: i64,
    program: ProgramFields<'a>,
}

/// 原始回報（只留根紀錄要的欄位；沒有的不輸出）。
#[derive(serde::Serialize)]
struct ProgramFields<'a> {
    state: ProgramState,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<BlockedKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    app: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    msg: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    progress: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
}

/// 一個會話的程式回報寫入器（PTY 讀取執行緒持有）。記得上一次寫了什麼：沒變就不寫檔（節流）。
#[derive(Debug)]
pub struct ProgramRecorder {
    status: PathBuf,
    last: Option<ProgramReport>,
}

impl ProgramRecorder {
    /// `status`＝這個會話 hook 用的狀態檔路徑（[`path_for`]）；程式回報寫在它旁邊。
    pub fn new(status: PathBuf) -> Self {
        ProgramRecorder { status, last: None }
    }

    /// 記一筆回報，回傳檔案有沒有變。只記根紀錄（子紀錄＝子 agent，hook 那邊另有計數）。
    /// - 跟上一筆一樣：不寫
    /// - `clear`：撤掉紀錄（刪檔）
    /// - `idle`：上一筆是做完／出錯就留著（你還沒看結果）；否則撤掉——閒著不是一個要顯示的狀態
    ///   （剛開的 Codex 一開始就送 idle，不該亮「等你」）
    pub fn record(&mut self, report: ProgramReport, now: Option<NaiveDateTime>) -> bool {
        if report.id.is_some() || self.last.as_ref() == Some(&report) {
            return false;
        }
        let prev = self.last.replace(report.clone()).map(|r| r.state);
        let Some((status, event, text)) = program_view(&report) else {
            let keep = report.state == ProgramState::Idle
                && matches!(prev, Some(ProgramState::Done | ProgramState::Error));
            if keep {
                return false;
            }
            let _g = program_lock();
            return std::fs::remove_file(program_path(&self.status)).is_ok();
        };
        let at = now
            .unwrap_or_else(|| Local::now().naive_local())
            .format("%Y-%m-%d %H:%M")
            .to_string();
        let rec = ProgramRecord {
            status: status.as_str(),
            text,
            at,
            event,
            source: StatusSource::Program,
            ts: millis_of(now),
            program: ProgramFields {
                state: report.state,
                kind: report.kind,
                app: report.app.as_deref(),
                msg: report.msg.as_deref(),
                progress: report.progress,
                title: report.title.as_deref(),
            },
        };
        let Ok(bytes) = serde_json::to_vec(&rec) else {
            return false;
        };
        let p = program_path(&self.status);
        if let Some(parent) = p.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return false;
            }
        }
        let _g = program_lock();
        crate::fsutil::write_atomic(&p, &bytes).is_ok()
    }
}

/// 一個來源最後一次說了什麼、什麼時候（毫秒）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observed {
    pub status: AgentStatus,
    pub at_ms: i64,
}

/// 優先規則（F-130，照 Orca `hook-precedence` 的想法）：
/// - 只有一邊有紀錄就用那一邊；都沒有＝None
/// - 都有：hook 優先。例外是 hook 已經靜默超過 `takeover_ms`、而程式回報**在那之後**換到了不同的階段——
///   hook 漏了事件（例：沒等到 Stop），程式自己說已經離開那個狀態了，就信程式的
/// - 下一個 hook 事件一來（時間比程式回報新）就還給 hook
/// - hook 記的是 `exited`（程序已結束、留給「接續」）一律用 hook
pub fn pick_source(
    hook: Option<Observed>,
    program: Option<Observed>,
    now_ms: i64,
    takeover_ms: i64,
) -> Option<StatusSource> {
    match (hook, program) {
        (None, None) => None,
        (Some(_), None) => Some(StatusSource::Hook),
        (None, Some(_)) => Some(StatusSource::Program),
        (Some(h), Some(p)) => {
            let yields = h.status != AgentStatus::Exited
                && p.status != h.status
                && p.at_ms > h.at_ms
                && now_ms - h.at_ms >= takeover_ms;
            Some(if yields {
                StatusSource::Program
            } else {
                StatusSource::Hook
            })
        }
    }
}

/// 狀態檔的時間：`ts`（毫秒）；舊檔沒有就用檔案修改時間。
fn observed(v: &Value, p: &Path) -> Option<Observed> {
    let status = v
        .get("status")
        .and_then(Value::as_str)
        .and_then(AgentStatus::parse)?;
    let at_ms = v.get("ts").and_then(Value::as_i64).or_else(|| {
        let t = std::fs::metadata(p).and_then(|m| m.modified()).ok()?;
        let d = t.duration_since(std::time::UNIX_EPOCH).ok()?;
        i64::try_from(d.as_millis()).ok()
    })?;
    Some(Observed { status, at_ms })
}

/// 這個會話現在算數的狀態（hook 或程式回報，依 [`pick_source`]），多一個 `source` 欄位。
/// `takeover`＝hook 靜默多久之後讓給程式回報（config `thresholds.program_status_takeover_seconds`）。
pub fn read_resolved(spine_dir: &Path, sid: &str, takeover: std::time::Duration) -> Option<Value> {
    let hp = path_for(spine_dir, sid);
    let pp = program_path(&hp);
    let hook = read_json(&hp);
    let program = read_json(&pp);
    let source = pick_source(
        hook.as_ref().and_then(|v| observed(v, &hp)),
        program.as_ref().and_then(|v| observed(v, &pp)),
        Local::now().timestamp_millis(),
        i64::try_from(takeover.as_millis()).unwrap_or(i64::MAX),
    )
    // 狀態字串認不得（壞檔）：照舊用 hook 那份
    .or_else(|| hook.is_some().then_some(StatusSource::Hook))?;
    let mut v = match source {
        StatusSource::Hook => hook?,
        StatusSource::Program => program?,
    };
    v["source"] = json!(source.as_str());
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_failure_waits_with_the_reason_and_subagents_are_counted_by_marker_files() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        let target = path_for(d, "t1");
        let tgt = target.to_str();
        assert!(record_hook(
            StatusHook::StopFailure,
            r#"{"error":"rate_limit","error_details":"5-hour limit reached"}"#,
            tgt,
            None
        ));
        let v = read(d, "t1").unwrap();
        assert_eq!(v["status"], "waiting");
        assert_eq!(v["event"], "stop-failure");
        assert_eq!(v["text"], "這一輪出錯中斷：5-hour limit reached");
        // 子 agent：一個一個標記，不動狀態；壞的 id 不寫
        for id in ["a1", "a2"] {
            let body = format!(r#"{{"agent_id":"{id}","agent_type":"Explore"}}"#);
            assert!(record_hook(StatusHook::SubagentStart, &body, tgt, None));
        }
        assert!(!record_hook(
            StatusHook::SubagentStart,
            r#"{"agent_id":"../x"}"#,
            tgt,
            None
        ));
        assert_eq!(subagent_count(d, "t1"), 2);
        assert!(record_hook(
            StatusHook::SubagentStop,
            r#"{"agent_id":"a1"}"#,
            tgt,
            None
        ));
        assert_eq!(subagent_count(d, "t1"), 1);
        assert_eq!(read(d, "t1").unwrap()["event"], "stop-failure");
        // 用量：變了才寫
        assert!(record_meter(&target, Some(41), Some("Opus")));
        assert!(!record_meter(&target, Some(41), Some("Opus")));
        assert!(record_meter(&target, Some(42), Some("Opus")));
        assert_eq!(meter(d, "t1").unwrap()["context"], 42);
        // 程序結束：標記與用量一起清
        clear(d, "t1");
        assert_eq!(subagent_count(d, "t1"), 0);
        assert!(meter(d, "t1").is_none());
    }

    #[test]
    fn hook_writes_only_to_our_status_files() {
        let t = tempfile::tempdir().unwrap();
        let sp = t.path();
        let target = path_for(sp, "t3");
        let at = chrono::NaiveDate::from_ymd_opt(2026, 10, 14)
            .unwrap()
            .and_hms_opt(1, 2, 0);
        // 不是 App 開的會話：不寫
        assert!(!record_hook(StatusHook::Stop, "{}", None, at));
        assert!(!record_hook(StatusHook::Stop, "{}", Some(""), at));
        // 路徑不像我們的狀態檔：不寫
        let evil = sp.join("registry.yaml");
        assert!(!record_hook(
            StatusHook::Stop,
            "{}",
            Some(evil.to_str().unwrap()),
            at
        ));
        assert!(!evil.exists());
        assert!(!record_hook(
            StatusHook::Stop,
            "{}",
            Some("rel/.state/session_status/x.json"),
            at
        ));

        let tp = target.to_str().unwrap();
        assert!(record_hook(StatusHook::Prompt, "", Some(tp), at));
        assert_eq!(read(sp, "t3").unwrap()["status"], "working");
        let msg = r#"{"message":"Claude needs your permission to use Bash"}"#;
        assert!(record_hook(StatusHook::Notification, msg, Some(tp), at));
        let v = read(sp, "t3").unwrap();
        assert_eq!(
            (v["status"].as_str(), v["text"].as_str(), v["at"].as_str()),
            (
                Some("blocked"),
                Some("Claude needs your permission to use Bash"),
                Some("2026-10-14 01:02")
            )
        );
        let idle =
            r#"{"message":"Claude is waiting for your input","notification_type":"idle_prompt"}"#;
        assert!(record_hook(StatusHook::Notification, idle, Some(tp), at));
        assert_eq!(read(sp, "t3").unwrap()["status"], "waiting");
        let typed = r#"{"message":"需要確認","notification_type":"permission_prompt"}"#;
        assert!(record_hook(StatusHook::Notification, typed, Some(tp), at));
        assert_eq!(read(sp, "t3").unwrap()["status"], "blocked");
        assert!(record_hook(StatusHook::Stop, "not json", Some(tp), at));
        let v = read(sp, "t3").unwrap();
        assert_eq!(
            (v["status"].as_str(), v["text"].as_str(), v.get("seen")),
            (Some("finished"), Some("這一輪做完了"), None)
        );
        // F-80：看過了記 seen；再記一次不重寫；下一個 hook 事件就把 seen 清掉
        assert!(mark_seen(sp, "t3"));
        assert!(!mark_seen(sp, "t3"));
        assert_eq!(read(sp, "t3").unwrap()["seen"], true);
        assert!(!mark_seen(sp, "nobody"));
        assert!(record_hook(StatusHook::Stop, "{}", Some(tp), at));
        assert!(read(sp, "t3").unwrap().get("seen").is_none());
        assert!(record_hook(
            StatusHook::Permission,
            r#"{"message":"Allow Edit?"}"#,
            Some(tp),
            at
        ));
        let v = read(sp, "t3").unwrap();
        assert_eq!(
            (v["status"].as_str(), v["text"].as_str()),
            (Some("blocked"), Some("Allow Edit?"))
        );
        assert!(!record_hook(StatusHook::Statusline, "{}", Some(tp), at));

        clear(sp, "t3");
        assert!(read(sp, "t3").is_none());
        record_hook(StatusHook::Stop, "{}", Some(tp), at);
        std::fs::write(dir(sp).join("keep.txt"), "x").unwrap();
        clear_all(sp);
        assert!(read(sp, "t3").is_none());
        assert!(dir(sp).join("keep.txt").exists());
    }

    #[test]
    fn clear_all_except_keeps_adopted_sessions() {
        let t = tempfile::tempdir().unwrap();
        let sp = t.path();
        std::fs::create_dir_all(dir(sp)).unwrap();
        for f in [
            "t1.json",
            "t1.program.json",
            "t1.meter.json",
            "t2.json",
            "t12.json",
        ] {
            std::fs::write(dir(sp).join(f), "{}").unwrap();
        }
        std::fs::create_dir_all(dir(sp).join("t1.sub")).unwrap();
        std::fs::create_dir_all(dir(sp).join("t2.sub")).unwrap();
        std::fs::create_dir_all(crate::auto_continue::dir(sp)).unwrap();
        std::fs::write(crate::auto_continue::dir(sp).join("t1.json"), "{}").unwrap();
        std::fs::write(crate::auto_continue::dir(sp).join("t2.json"), "{}").unwrap();
        clear_all_except(sp, &["t1".to_string()]);
        let left = |d: PathBuf| {
            let mut v: Vec<String> = std::fs::read_dir(d)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            left(dir(sp)),
            ["t1.json", "t1.meter.json", "t1.program.json", "t1.sub"]
        );
        assert_eq!(left(crate::auto_continue::dir(sp)), ["t1.json"]);
    }

    #[test]
    fn stop_keeps_title_and_last_reply_from_the_transcript() {
        let t = tempfile::tempdir().unwrap();
        let sp = t.path();
        let tr = sp.join("abc.jsonl");
        std::fs::write(
            &tr,
            [
                r#"{"type":"ai-title","aiTitle":"修登入流程"}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"改好了。\n\n測試都過，要 commit 嗎？"}]}}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        let tp = path_for(sp, "t1");
        let tps = tp.to_str().unwrap();
        let payload = json!({"transcript_path": tr}).to_string();
        assert!(record_hook(StatusHook::Stop, &payload, Some(tps), None));
        let v = read(sp, "t1").unwrap();
        assert_eq!(v["title"], "修登入流程");
        assert_eq!(v["reply"], "改好了。 測試都過，要 commit 嗎？");
        // 等你允許：標題、回覆都沿用
        assert!(record_hook(
            StatusHook::Permission,
            r#"{"message":"Allow Bash?"}"#,
            Some(tps),
            None
        ));
        let v = read(sp, "t1").unwrap();
        assert_eq!(
            (v["title"].as_str(), v["reply"].as_str()),
            (
                Some("修登入流程"),
                Some("改好了。 測試都過，要 commit 嗎？")
            )
        );
        // 你送出下一句：回覆清掉、標題留著
        assert!(record_hook(StatusHook::Prompt, "{}", Some(tps), None));
        let v = read(sp, "t1").unwrap();
        assert_eq!(v["title"], "修登入流程");
        assert!(v.get("reply").is_none());
        // Claude Code 直接給最後訊息就用它；相對路徑的逐字稿不讀
        let p2 = json!({"transcript_path": "rel/x.jsonl", "last_assistant_message": "直接給的"})
            .to_string();
        assert!(record_hook(StatusHook::Stop, &p2, Some(tps), None));
        assert_eq!(read(sp, "t1").unwrap()["reply"], "直接給的");
    }

    #[test]
    fn exit_keeps_what_resume_needs_only_when_the_session_id_is_known() {
        let t = tempfile::tempdir().unwrap();
        let sp = t.path();
        let tp = path_for(sp, "t1");
        let tps = tp.to_str().unwrap();
        let p = json!({"session_id": "aaaaaaaa-3333", "cwd": "/r",
                       "last_assistant_message": "好了"})
        .to_string();
        assert!(record_hook(StatusHook::Stop, &p, Some(tps), None));
        let v = read(sp, "t1").unwrap();
        assert_eq!(
            (v["claude_session"].as_str(), v["cwd"].as_str()),
            (Some("aaaaaaaa-3333"), Some("/r"))
        );
        retire(sp, "t1");
        let v = read(sp, "t1").unwrap();
        assert_eq!(v["status"], "exited");
        assert_eq!(v["claude_session"], "aaaaaaaa-3333");
        assert!(v.get("reply").is_none());
        // 之後的事件沒帶 id／cwd：沿用
        let tp1 = path_for(sp, "t1");
        assert!(record_hook(
            StatusHook::Prompt,
            "{}",
            Some(tp1.to_str().unwrap()),
            None
        ));
        let v = read(sp, "t1").unwrap();
        assert_eq!(
            (v["claude_session"].as_str(), v["cwd"].as_str()),
            (Some("aaaaaaaa-3333"), Some("/r"))
        );
        // 沒記到 id：照舊刪掉
        assert!(record_hook(
            StatusHook::Stop,
            r#"{"session_id":"bad id"}"#,
            Some(path_for(sp, "t2").to_str().unwrap()),
            None
        ));
        retire(sp, "t2");
        assert!(read(sp, "t2").is_none());
    }

    #[test]
    fn permission_request_says_which_tool_and_what_and_is_not_overwritten_by_the_notification() {
        let t = tempfile::tempdir().unwrap();
        let sp = t.path();
        let tp = path_for(sp, "t9");
        let tp = tp.to_str().unwrap();
        let at = chrono::NaiveDate::from_ymd_opt(2026, 10, 16)
            .unwrap()
            .and_hms_opt(21, 0, 0);
        let req = r#"{"tool_name":"Bash","tool_input":{"command":"cargo test -q\n  --workspace","description":"跑測試"}}"#;
        assert!(record_hook(
            StatusHook::PermissionRequest,
            req,
            Some(tp),
            at
        ));
        let v = read(sp, "t9").unwrap();
        assert_eq!(
            (
                v["status"].as_str(),
                v["text"].as_str(),
                v["event"].as_str()
            ),
            (
                Some("blocked"),
                Some("Bash：cargo test -q --workspace"),
                Some("permission-request")
            )
        );
        // 同一分鐘跟著來的 Notification 只會說「needs your permission」：留具體那句
        let note = r#"{"message":"Claude needs your permission to use Bash","notification_type":"permission_prompt"}"#;
        assert!(record_hook(StatusHook::Permission, note, Some(tp), at));
        let v = read(sp, "t9").unwrap();
        assert_eq!(v["text"], "Bash：cargo test -q --workspace");
        assert_eq!(v["event"], "permission");
        // 不同分鐘＝另一次請求，照通知寫
        let later = at.map(|a| a + chrono::Duration::minutes(1));
        assert!(record_hook(StatusHook::Permission, note, Some(tp), later));
        assert_eq!(
            read(sp, "t9").unwrap()["text"],
            "Claude needs your permission to use Bash"
        );
        // 改檔用路徑；沒有工具名就只說要你允許；沒認得的欄位就只有工具名
        assert_eq!(
            pending_action(
                &json!({"tool_name":"Edit","tool_input":{"file_path":"src/x.rs","old_string":"a"}})
            ),
            "Edit：src/x.rs"
        );
        assert_eq!(pending_action(&json!({})), "要你允許工具");
        assert_eq!(
            pending_action(&json!({"tool_name":"AskUserQuestion","tool_input":{"questions":[]}})),
            "AskUserQuestion"
        );
    }

    #[test]
    fn tool_start_and_end_keep_working_but_show_the_tool() {
        let t = tempfile::tempdir().unwrap();
        let sp = t.path();
        let tp = path_for(sp, "t8");
        let tp = tp.to_str().unwrap();
        let at = chrono::NaiveDate::from_ymd_opt(2026, 10, 16)
            .unwrap()
            .and_hms_opt(22, 0, 7);
        let start = r#"{"tool_name":"Bash","tool_input":{"command":"cargo test -q"}}"#;
        assert!(record_hook(StatusHook::ToolStart, start, Some(tp), at));
        let v = read(sp, "t8").unwrap();
        assert_eq!(
            (v["status"].as_str(), v["text"].as_str()),
            (Some("working"), Some("處理中：Bash：cargo test -q"))
        );
        assert_eq!(v["tool"]["label"], "Bash：cargo test -q");
        assert_eq!(v["tool"]["since"], "2026-10-16 22:00:07");
        // 工具結束：回到單純的處理中，tool 鍵消失
        assert!(record_hook(StatusHook::ToolEnd, start, Some(tp), at));
        let v = read(sp, "t8").unwrap();
        assert_eq!(v["text"], "處理中");
        assert!(v.get("tool").is_none());
        // 核准後 Claude Code 會先 PreToolUse：blocked 自然回到 working
        assert!(record_hook(
            StatusHook::PermissionRequest,
            start,
            Some(tp),
            at
        ));
        assert_eq!(read(sp, "t8").unwrap()["status"], "blocked");
        assert!(record_hook(StatusHook::ToolStart, start, Some(tp), at));
        assert_eq!(read(sp, "t8").unwrap()["status"], "working");
    }

    #[test]
    fn settings_file_hooks_point_at_ctx_by_notification_matcher() {
        let t = tempfile::tempdir().unwrap();
        let p = settings_file(t.path(), Path::new("/opt/ctx")).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        for (ev, arg) in [
            ("Stop", "stop"),
            ("UserPromptSubmit", "prompt"),
            ("PermissionRequest", "permission-request"),
        ] {
            let cmd = v["hooks"][ev][0]["hooks"][0]["command"].as_str().unwrap();
            assert!(
                cmd.contains("/opt/ctx") && cmd.ends_with(&format!("hook {arg}")),
                "{cmd}"
            );
        }
        // Notification：一個 matcher 一筆
        let by: Vec<(&str, &str)> = v["hooks"]["Notification"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                let cmd = e["hooks"][0]["command"].as_str().unwrap();
                (
                    e["matcher"].as_str().unwrap(),
                    cmd.rsplit(' ').next().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            by,
            [
                ("permission_prompt", "permission"),
                ("elicitation_dialog", "permission"),
                ("elicitation_url_dialog", "permission"),
                ("agent_needs_input", "permission"),
                ("idle_prompt", "notification"),
            ]
        );
        assert!(v["hooks"].get("SessionStart").is_none());
        // F-83：statusLine 也指到 ctx
        let sl = v["statusLine"]["command"].as_str().unwrap();
        assert!(
            sl.contains("/opt/ctx") && sl.ends_with("hook statusline"),
            "{sl}"
        );
    }

    // ── F-130 ──

    fn obs(status: AgentStatus, at_ms: i64) -> Option<Observed> {
        Some(Observed { status, at_ms })
    }

    #[test]
    fn hook_wins_unless_it_went_silent_on_a_state_the_program_has_left() {
        use AgentStatus::*;
        let two_min = 120_000;
        // 四種組合
        assert_eq!(pick_source(None, None, 0, two_min), None);
        assert_eq!(
            pick_source(obs(Working, 0), None, 999_999, two_min),
            Some(StatusSource::Hook)
        );
        assert_eq!(
            pick_source(None, obs(Working, 0), 0, two_min),
            Some(StatusSource::Program)
        );
        assert_eq!(
            pick_source(obs(Working, 0), obs(Finished, 1_000), 1_000, two_min),
            Some(StatusSource::Hook)
        );
        // hook 停在處理中超過 2 分鐘、程式之後說做完了 → 程式
        assert_eq!(
            pick_source(obs(Working, 0), obs(Finished, 1_000), two_min, two_min),
            Some(StatusSource::Program)
        );
        // 還沒滿門檻：仍是 hook
        assert_eq!(
            pick_source(obs(Working, 0), obs(Finished, 1_000), two_min - 1, two_min),
            Some(StatusSource::Hook)
        );
        // 程式說的跟 hook 一樣、或程式的回報比 hook 舊：不讓
        assert_eq!(
            pick_source(obs(Working, 0), obs(Working, 1_000), 9 * two_min, two_min),
            Some(StatusSource::Hook)
        );
        assert_eq!(
            pick_source(
                obs(Working, 5_000),
                obs(Finished, 1_000),
                9 * two_min,
                two_min
            ),
            Some(StatusSource::Hook)
        );
        // 下一個 hook 事件來了（比程式回報新）→ 還給 hook
        assert_eq!(
            pick_source(
                obs(Waiting, 2_000),
                obs(Finished, 1_000),
                9 * two_min,
                two_min
            ),
            Some(StatusSource::Hook)
        );
        // 程序結束留給「接續」的那筆一律用 hook
        assert_eq!(
            pick_source(obs(Exited, 0), obs(Working, 1_000), 9 * two_min, two_min),
            Some(StatusSource::Hook)
        );
    }

    fn report(state: ProgramState, kind: Option<BlockedKind>) -> ProgramReport {
        ProgramReport {
            id: None,
            state,
            kind,
            progress: None,
            app: Some("codex".into()),
            msg: None,
            title: None,
        }
    }

    #[test]
    fn program_reports_go_to_their_own_file_and_unchanged_ones_are_not_rewritten() {
        let t = tempfile::tempdir().unwrap();
        let sp = t.path();
        let mut rec = ProgramRecorder::new(path_for(sp, "t1"));
        // 剛開的 Codex 先送 idle：不是要顯示的狀態，不寫
        assert!(!rec.record(report(ProgramState::Idle, None), None));
        assert!(read_program(sp, "t1").is_none());
        assert!(rec.record(report(ProgramState::Working, None), None));
        let pp = program_path(&path_for(sp, "t1"));
        let first = std::fs::read_to_string(&pp).unwrap();
        let v: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(
            (
                v["status"].as_str(),
                v["source"].as_str(),
                v["event"].as_str()
            ),
            (Some("working"), Some("program"), Some("program"))
        );
        assert_eq!(v["program"]["app"], "codex");
        assert!(v["ts"].as_i64().is_some());
        // 一樣的回報：不重寫（內容、時間都不動）
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(!rec.record(report(ProgramState::Working, None), None));
        assert_eq!(std::fs::read_to_string(&pp).unwrap(), first);
        // hook 的檔沒被碰
        assert!(read(sp, "t1").is_none());
        // 沒有 hook：讀端用程式回報
        assert_eq!(
            read_resolved(sp, "t1", std::time::Duration::from_secs(120)).unwrap()["source"],
            "program"
        );
        // 子紀錄不記
        let mut child = report(ProgramState::Blocked, None);
        child.id = Some("sub/a1".into());
        assert!(!rec.record(child, None));
        // 做完 → 看過了 → 接著的 idle 不把結果洗掉
        assert!(rec.record(report(ProgramState::Done, None), None));
        assert_eq!(read_program(sp, "t1").unwrap()["status"], "finished");
        assert!(mark_seen(sp, "t1"));
        assert!(!mark_seen(sp, "t1"));
        assert!(!rec.record(report(ProgramState::Idle, None), None));
        assert_eq!(read_program(sp, "t1").unwrap()["seen"], true);
        // clear：撤掉
        assert!(rec.record(report(ProgramState::Clear, None), None));
        assert!(read_program(sp, "t1").is_none());
        // 程序結束一起清；App 重開也清
        assert!(rec.record(report(ProgramState::Working, None), None));
        retire(sp, "t1");
        assert!(read_program(sp, "t1").is_none());
        assert!(rec.record(report(ProgramState::Done, None), None));
        clear_all(sp);
        assert!(read_program(sp, "t1").is_none());
        // 程式回報檔不算 hook 的狀態檔（目錄列舉用）
        assert!(is_status_file_name("t1.json"));
        assert!(!is_status_file_name("t1.program.json"));
        assert!(!is_status_file_name("t1.meter.json"));
    }

    #[test]
    fn program_states_map_onto_the_existing_ones() {
        let view = |s, k| program_view(&report(s, k)).map(|(st, ev, _)| (st, ev));
        assert_eq!(
            view(ProgramState::Working, None),
            Some((AgentStatus::Working, "program"))
        );
        assert_eq!(
            view(ProgramState::Done, None),
            Some((AgentStatus::Finished, "program"))
        );
        assert_eq!(
            view(ProgramState::Blocked, Some(BlockedKind::Permission)),
            Some((AgentStatus::Blocked, "permission"))
        );
        for k in [Some(BlockedKind::Question), Some(BlockedKind::Auth), None] {
            assert_eq!(
                view(ProgramState::Blocked, k),
                Some((AgentStatus::Waiting, "notification"))
            );
        }
        assert_eq!(
            view(ProgramState::Error, None),
            Some((AgentStatus::Waiting, "stop-failure"))
        );
        assert_eq!(view(ProgramState::Idle, None), None);
        let mut r = report(ProgramState::Blocked, Some(BlockedKind::Permission));
        r.msg = Some("approve Write: a.rs".into());
        assert_eq!(program_view(&r).unwrap().2, "approve Write: a.rs");
    }

    #[test]
    fn old_hook_files_without_source_or_ts_still_read_and_win() {
        let t = tempfile::tempdir().unwrap();
        let sp = t.path();
        std::fs::create_dir_all(dir(sp)).unwrap();
        // 2026-10 的格式：沒有 source、沒有 ts
        std::fs::write(
            path_for(sp, "t1"),
            r#"{"status":"working","text":"處理中","at":"2026-10-14 01:02","event":"prompt"}"#,
        )
        .unwrap();
        let v = read_resolved(sp, "t1", std::time::Duration::from_secs(120)).unwrap();
        assert_eq!(
            (v["status"].as_str(), v["source"].as_str()),
            (Some("working"), Some("hook"))
        );
        assert_eq!(read(sp, "t1").unwrap()["event"], "prompt");
        // 新寫的 hook 紀錄多 source、ts
        assert!(record_hook(
            StatusHook::Stop,
            "{}",
            path_for(sp, "t1").to_str(),
            None
        ));
        let v = read(sp, "t1").unwrap();
        assert_eq!(v["source"], "hook");
        assert!(v["ts"].as_i64().is_some());
        // 程式回報較新、但 hook 剛寫：仍是 hook；門檻 0 秒＝立刻讓
        let mut rec = ProgramRecorder::new(path_for(sp, "t1"));
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(rec.record(report(ProgramState::Working, None), None));
        let long = std::time::Duration::from_secs(120);
        assert_eq!(read_resolved(sp, "t1", long).unwrap()["source"], "hook");
        let now = std::time::Duration::ZERO;
        let v = read_resolved(sp, "t1", now).unwrap();
        assert_eq!(
            (v["status"].as_str(), v["source"].as_str()),
            (Some("working"), Some("program"))
        );
        // 下一個 hook 事件：收回
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(record_hook(
            StatusHook::Stop,
            "{}",
            path_for(sp, "t1").to_str(),
            None
        ));
        assert_eq!(read_resolved(sp, "t1", now).unwrap()["source"], "hook");
    }
}
