//! 每個 App 會話「現在輪到誰」（2026-10-14，Rust 才有）：agent 在做、還是在等你。
//!
//! 多個 agent 會話同時跑時，最要緊的是一眼看出**哪個在等我**。MCP `report_status` 要 agent 自己記得回報，
//! 而且依範圍存（同範圍兩個會話會互蓋）；這裡改由 Claude Code 自己的 hook 自動寫、依會話存：
//! - `Stop`（這一輪做完）→ `waiting`「這一輪做完了，等你回覆」
//! - `Notification`（要你允許工具、或閒著等輸入）→ `waiting`＋Claude Code 給的訊息
//! - `UserPromptSubmit`（你送出訊息）→ `working`
//!
//! 只有 App 開的 Claude Code 會話會寫：開會話時多帶 `--settings=<spine>/.state/session_hooks.json`
//! （只掛這三個 hook，不動使用者的 `~/.claude/settings.json`）與環境變數 [`ENV`]＝這個會話的狀態檔路徑。
//! hook 照路徑寫，不必找 spine。會話結束就刪；App 重開時全部清掉（sid 每次重開從頭編）。
//!
//! 檔案（`.state/session_status/<sid>.json`）：`{"status","text","at","event"}`，`at` 是 `YYYY-MM-DD HH:MM`。

use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime};
use serde_json::{json, Value};

use crate::Result;

/// App 開的 agent 會話帶的環境變數：這個會話的狀態檔絕對路徑。
pub const ENV: &str = "REPO_CONTEXT_SESSION_STATUS";
/// 訊息最多留幾個字元（Notification 的訊息可能很長）。
const TEXT_MAX: usize = 120;

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
    let hook = |arg: &str| {
        json!([{"hooks": [{"type": "command",
                            "command": crate::agentsetup::hook_command(ctx, arg)}]}])
    };
    let v = json!({"hooks": {
        "Stop": hook("stop"),
        "Notification": hook("notification"),
        "UserPromptSubmit": hook("prompt"),
    }});
    crate::fsutil::write_atomic(&p, serde_json::to_string_pretty(&v)?.as_bytes())?;
    Ok(p)
}

/// hook 事件 → (status, text)。不認得＝None。
fn status_of(event: &str, payload: &Value) -> Option<(&'static str, String)> {
    match event {
        "stop" => Some(("waiting", "這一輪做完了，等你回覆".to_string())),
        "notification" => {
            let msg = payload
                .get("message")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .unwrap_or("需要你回應");
            Some((
                "waiting",
                crate::pystr::take_chars(msg, TEXT_MAX).to_string(),
            ))
        }
        "prompt" => Some(("working", "處理中".to_string())),
        _ => None,
    }
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
    let rec = json!({"status": status, "text": text, "at": at, "event": event});
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    crate::fsutil::write_atomic(&path, rec.to_string().as_bytes()).is_ok()
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
                Some("waiting"),
                Some("Claude needs your permission to use Bash"),
                Some("2026-10-14 01:02")
            )
        );
        assert!(record_hook("stop", "not json", Some(tp), at));
        assert_eq!(read(sp, "t3").unwrap()["text"], "這一輪做完了，等你回覆");
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
    fn settings_file_has_three_hooks_pointing_at_ctx() {
        let t = tempfile::tempdir().unwrap();
        let p = settings_file(t.path(), Path::new("/opt/ctx")).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        for (ev, arg) in [
            ("Stop", "stop"),
            ("Notification", "notification"),
            ("UserPromptSubmit", "prompt"),
        ] {
            let cmd = v["hooks"][ev][0]["hooks"][0]["command"].as_str().unwrap();
            assert!(
                cmd.contains("/opt/ctx") && cmd.ends_with(&format!("hook {arg}")),
                "{cmd}"
            );
        }
        assert!(v["hooks"].get("SessionStart").is_none());
    }
}
