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
//! F-78：另記 `claude_session`（Claude Code 自己的會話 id）與 `cwd`；程序結束時不刪檔，改寫成
//! `status: "exited"`（[`retire`]）——已結束的會話列據此給「接續」（`claude --resume`）。

use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime};
use serde_json::{json, Value};

use crate::Result;

/// App 開的 agent 會話帶的環境變數：這個會話的狀態檔絕對路徑。
pub const ENV: &str = "REPO_CONTEXT_SESSION_STATUS";
/// 訊息最多留幾個字元（Notification 的訊息可能很長）。
const TEXT_MAX: usize = 120;
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

/// 會話結束：刪掉它的狀態檔（沒有就算了）。
pub fn clear(spine_dir: &Path, sid: &str) {
    let _ = std::fs::remove_file(path_for(spine_dir, sid));
}

/// 程序結束（F-78）：記得 Claude Code 會話 id 的就留一份 `exited`（標題、id、目錄）給「接續」用；
/// 沒有 id（hook 從沒觸發）就照舊刪掉。App 重開時 [`clear_all`] 一起清。
pub fn retire(spine_dir: &Path, sid: &str) {
    let keep = read(spine_dir, sid).filter(|v| {
        v.get("claude_session")
            .and_then(Value::as_str)
            .is_some_and(crate::history::valid_session_id)
    });
    let Some(mut v) = keep else {
        clear(spine_dir, sid);
        return;
    };
    v["status"] = json!("exited");
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
pub fn mark_seen(spine_dir: &Path, sid: &str) -> bool {
    let Some(mut v) = read(spine_dir, sid) else {
        return false;
    };
    if v.get("seen") == Some(&Value::Bool(true)) {
        return false;
    }
    v["seen"] = json!(true);
    crate::fsutil::write_atomic(&path_for(spine_dir, sid), v.to_string().as_bytes()).is_ok()
}

/// App 重開：前一次的會話都不在了（sid 會重新從頭編），全部清掉，只刪 `*.json`。
pub fn clear_all(spine_dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir(spine_dir)) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("json") {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// 寫 App 會話用的 Claude Code 設定檔（只有三個 hook），回傳路徑。每次開會話都重寫（`ctx` 位置可能變了）。
pub fn settings_file(spine_dir: &Path, ctx: &Path) -> Result<PathBuf> {
    let p = spine_dir.join(".state").join("session_hooks.json");
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let cmd = |arg: &str| json!([{"type": "command", "command": crate::agentsetup::hook_command(ctx, arg)}]);
    // Notification 依通知種類（matcher）分開掛：要你允許／要你回答 → permission（blocked）；閒著等輸入 →
    // notification（waiting）；其他種類（登入成功、額度…）不動會話狀態。一個 matcher 一筆，不靠 regex 的「|」
    let mut notification: Vec<Value> = PERMISSION_MATCHERS
        .iter()
        .map(|m| json!({"matcher": m, "hooks": cmd("permission")}))
        .collect();
    notification.push(json!({"matcher": "idle_prompt", "hooks": cmd("notification")}));
    let v = json!({"hooks": {
        "Stop": [{"hooks": cmd("stop")}],
        "Notification": notification,
        "PermissionRequest": [{"hooks": cmd("permission-request")}],
        "UserPromptSubmit": [{"hooks": cmd("prompt")}],
    }});
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

/// hook 事件 → (status, text)。不認得＝None。
fn status_of(event: &str, payload: &Value) -> Option<(&'static str, String)> {
    match event {
        "stop" => Some(("finished", "這一輪做完了".to_string())),
        "notification" => {
            let text = message(payload, "需要你回應");
            let msg = text.as_str();
            // 照 matcher 掛的是 idle_prompt；萬一舊設定沒分 matcher，從欄位或訊息再判斷一次是不是要你允許
            let permission = payload.get("notification_type").and_then(Value::as_str)
                == Some("permission_prompt")
                || msg.to_lowercase().contains("permission");
            Some(if permission {
                ("blocked", text)
            } else {
                ("waiting", text)
            })
        }
        "permission" => Some(("blocked", message(payload, "要你允許或回答"))),
        "permission-request" => Some(("blocked", pending_action(payload))),
        "prompt" => Some(("working", "處理中".to_string())),
        _ => None,
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
    let Some(tool) = payload
        .get("tool_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
    else {
        return "要你允許工具".to_string();
    };
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
        None => format!("{tool}：要你允許"),
    };
    crate::pystr::take_chars(&text, TEXT_MAX).to_string()
}

/// 環境變數給的路徑像不像我們的狀態檔：`…/.state/session_status/<名稱>.json`。不像就不寫（不往任意路徑寫檔）。
fn valid_target(p: &Path) -> bool {
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
    event: &str,
    stdin: &str,
    target: Option<&str>,
    now: Option<NaiveDateTime>,
) -> bool {
    let Some(target) = target.map(str::trim).filter(|t| !t.is_empty()) else {
        return false;
    };
    let path = PathBuf::from(target);
    if !valid_target(&path) {
        return false;
    }
    let payload = crate::hooks::parse_payload(stdin);
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
        (Some(p), "notification" | "permission", "blocked")
            if p.get("event").and_then(Value::as_str) == Some("permission-request")
                && p.get("status").and_then(Value::as_str) == Some("blocked")
                && p.get("at").and_then(Value::as_str) == Some(at.as_str()) =>
        {
            p.get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or(text)
        }
        _ => text,
    };
    let mut rec = json!({"status": status, "text": text, "at": at, "event": event});
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
fn transcript_bits(event: &str, payload: &Value, path: &Path) -> (Option<String>, Option<String>) {
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
        "stop" => payload
            .get("last_assistant_message")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
            .or(tail.reply)
            .map(|r| crate::transcript::one_line(&r, REPLY_MAX)),
        "prompt" => None,
        _ => prev_of("reply"),
    };
    (title, reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_writes_only_to_our_status_files() {
        let t = tempfile::tempdir().unwrap();
        let sp = t.path();
        let target = path_for(sp, "t3");
        let at = chrono::NaiveDate::from_ymd_opt(2026, 10, 14)
            .unwrap()
            .and_hms_opt(1, 2, 0);
        // 不是 App 開的會話：不寫
        assert!(!record_hook("stop", "{}", None, at));
        assert!(!record_hook("stop", "{}", Some(""), at));
        // 路徑不像我們的狀態檔：不寫
        let evil = sp.join("registry.yaml");
        assert!(!record_hook("stop", "{}", Some(evil.to_str().unwrap()), at));
        assert!(!evil.exists());
        assert!(!record_hook(
            "stop",
            "{}",
            Some("rel/.state/session_status/x.json"),
            at
        ));

        let tp = target.to_str().unwrap();
        assert!(record_hook("prompt", "", Some(tp), at));
        assert_eq!(read(sp, "t3").unwrap()["status"], "working");
        let msg = r#"{"message":"Claude needs your permission to use Bash"}"#;
        assert!(record_hook("notification", msg, Some(tp), at));
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
        assert!(record_hook("notification", idle, Some(tp), at));
        assert_eq!(read(sp, "t3").unwrap()["status"], "waiting");
        let typed = r#"{"message":"需要確認","notification_type":"permission_prompt"}"#;
        assert!(record_hook("notification", typed, Some(tp), at));
        assert_eq!(read(sp, "t3").unwrap()["status"], "blocked");
        assert!(record_hook("stop", "not json", Some(tp), at));
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
        assert!(record_hook("stop", "{}", Some(tp), at));
        assert!(read(sp, "t3").unwrap().get("seen").is_none());
        assert!(record_hook(
            "permission",
            r#"{"message":"Allow Edit?"}"#,
            Some(tp),
            at
        ));
        let v = read(sp, "t3").unwrap();
        assert_eq!(
            (v["status"].as_str(), v["text"].as_str()),
            (Some("blocked"), Some("Allow Edit?"))
        );
        assert!(!record_hook("bogus", "{}", Some(tp), at));

        clear(sp, "t3");
        assert!(read(sp, "t3").is_none());
        record_hook("stop", "{}", Some(tp), at);
        std::fs::write(dir(sp).join("keep.txt"), "x").unwrap();
        clear_all(sp);
        assert!(read(sp, "t3").is_none());
        assert!(dir(sp).join("keep.txt").exists());
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
        assert!(record_hook("stop", &payload, Some(tps), None));
        let v = read(sp, "t1").unwrap();
        assert_eq!(v["title"], "修登入流程");
        assert_eq!(v["reply"], "改好了。 測試都過，要 commit 嗎？");
        // 等你允許：標題、回覆都沿用
        assert!(record_hook(
            "permission",
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
        assert!(record_hook("prompt", "{}", Some(tps), None));
        let v = read(sp, "t1").unwrap();
        assert_eq!(v["title"], "修登入流程");
        assert!(v.get("reply").is_none());
        // Claude Code 直接給最後訊息就用它；相對路徑的逐字稿不讀
        let p2 = json!({"transcript_path": "rel/x.jsonl", "last_assistant_message": "直接給的"})
            .to_string();
        assert!(record_hook("stop", &p2, Some(tps), None));
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
        assert!(record_hook("stop", &p, Some(tps), None));
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
            "prompt",
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
            "stop",
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
        assert!(record_hook("permission-request", req, Some(tp), at));
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
        assert!(record_hook("permission", note, Some(tp), at));
        let v = read(sp, "t9").unwrap();
        assert_eq!(v["text"], "Bash：cargo test -q --workspace");
        assert_eq!(v["event"], "permission");
        // 不同分鐘＝另一次請求，照通知寫
        let later = at.map(|a| a + chrono::Duration::minutes(1));
        assert!(record_hook("permission", note, Some(tp), later));
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
            "AskUserQuestion：要你允許"
        );
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
    }
}
