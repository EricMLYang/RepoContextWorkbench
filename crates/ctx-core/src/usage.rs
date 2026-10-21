//! Claude Code 額度餘光（2026-10-16，ideation F-83；Rust 才有）：從 statusLine 拿 5h／7d 用量，寫 `.state/usage.json`。
//!
//! Claude Code 每輪（串流時約每秒數次）會把一份 JSON 餵給 `statusLine` 指令，裡面有 `rate_limits`
//! （`five_hour`／`seven_day` 的 `used_percentage`、`resets_at`）、`context_window`、`model`——
//! 這是 Messages API 回應順便帶的，讀它不花任何額度（Orca 也是這樣拿；OAuth 的 usage 端點會 429）。
//!
//! - 只有 App 開的會話會掛（`session_status::settings_file` 的 `statusLine` → `ctx hook statusline`），
//!   跟會話狀態 hook 同一個環境變數 [`crate::session_status::ENV`] 找到 `.state`；不是 App 開的會話什麼都不做。
//! - 這條指令不印任何東西：那場會話的終端就沒有 statusline（使用者 2026-10-16 接受；Orca 的作法相同）。
//! - 內容沒變就不重寫（statusline 一秒數次；`.state/usage.json` 不在殼的監看清單，App 每 5 秒重抓 state 時帶出）。
//! - 另留最後一份原始 payload 在 `.state/usage_raw.json`（只在內容變時寫），欄位名漂了好查。
//!
//! 檔案（`.state/usage.json`）：`{"at","five_hour":{"used","resets"}|null,"seven_day":…,"context":n|null,"model","warn"}`，
//! `used` 是 0–100 的整數、`resets` 是排好的本地時間（今天「HH:MM」、不是今天「MM-DD HH:MM」）、`warn`＝任一視窗 ≥ [`WARN_AT`]。

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use serde_json::{json, Value};

/// 任一視窗用到這個百分比就標 `warn`（Orca 也是 80%）。
pub const WARN_AT: u64 = 80;

/// `.state/usage.json`。
pub fn path(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("usage.json")
}

fn raw_path(state_dir: &Path) -> PathBuf {
    state_dir.join("usage_raw.json")
}

/// 讀目前的額度餘光；沒有或壞掉＝None。
pub fn read(spine_dir: &Path) -> Option<Value> {
    let s = std::fs::read_to_string(path(spine_dir)).ok()?;
    serde_json::from_str::<Value>(&s)
        .ok()
        .filter(|v| v.get("at").and_then(Value::as_str).is_some())
}

/// 重置時間：epoch 秒或 ISO 字串 → 本地時間；今天只寫時分。認不出＝None。
fn resets_text(v: Option<&Value>, now: NaiveDateTime) -> Option<String> {
    let t: DateTime<Local> = match v? {
        Value::Number(n) => {
            let secs = n.as_f64()? as i64;
            Local.timestamp_opt(secs, 0).single()?
        }
        Value::String(s) => DateTime::parse_from_rfc3339(s.trim())
            .ok()?
            .with_timezone(&Local),
        _ => return None,
    };
    let t = t.naive_local();
    Some(if t.date() == now.date() {
        t.format("%H:%M").to_string()
    } else {
        t.format("%m-%d %H:%M").to_string()
    })
}

/// 一個視窗：`used_percentage`（Orca 也認 OAuth 形狀的 `utilization`）→ 0–100 整數；沒有數字＝None。
fn window(v: Option<&Value>, now: NaiveDateTime) -> Option<Value> {
    let w = v?.as_object()?;
    let used = w
        .get("used_percentage")
        .or_else(|| w.get("utilization"))
        .and_then(Value::as_f64)?;
    let used = used.round().clamp(0.0, 100.0) as u64;
    Some(json!({"used": used, "resets": resets_text(w.get("resets_at"), now)}))
}

/// statusLine 的 payload → 我們的形狀。沒有任何視窗也沒有 context／model＝None（舊版 CLI 沒給，不寫空檔）。
pub fn summarize(payload: &Value, now: NaiveDateTime) -> Option<Value> {
    let rl = payload.get("rate_limits");
    let five = window(rl.and_then(|r| r.get("five_hour")), now);
    let seven = window(rl.and_then(|r| r.get("seven_day")), now);
    let context = payload
        .get("context_window")
        .and_then(|c| c.get("used_percentage"))
        .and_then(Value::as_f64)
        .map(|p| p.round().clamp(0.0, 100.0) as u64);
    let model = payload
        .get("model")
        .and_then(|m| m.get("display_name").or_else(|| m.get("id")))
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty());
    if five.is_none() && seven.is_none() && context.is_none() && model.is_none() {
        return None;
    }
    let warn = [&five, &seven].iter().any(|w| {
        w.as_ref()
            .and_then(|w| w["used"].as_u64())
            .is_some_and(|u| u >= WARN_AT)
    });
    Some(json!({
        "at": now.format("%Y-%m-%d %H:%M").to_string(),
        "five_hour": five, "seven_day": seven,
        "context": context, "model": model, "warn": warn,
    }))
}

/// `ctx hook statusline` 的本體：`target`＝環境變數 [`crate::session_status::ENV`]（會話狀態檔路徑），
/// 由它找到 `.state`；不是 App 開的會話（沒有環境變數）＝什麼都不做。回傳有沒有寫。永遠不失敗、不印東西。
pub fn record(stdin: &str, target: Option<&str>, now: Option<NaiveDateTime>) -> bool {
    let Some(target) = target.map(str::trim).filter(|t| !t.is_empty()) else {
        return false;
    };
    let status_path = PathBuf::from(target);
    if !crate::session_status::valid_target(&status_path) {
        return false;
    }
    let Some(state_dir) = status_path.parent().and_then(Path::parent) else {
        return false;
    };
    let payload = crate::hooks::parse_payload(stdin);
    let now = now.unwrap_or_else(|| Local::now().naive_local());
    let Some(mut out) = summarize(&payload, now) else {
        return false;
    };
    // F-101：這場會話自己的 context 用量 %、模型（usage.json 是全部會話共用一份，看不出是誰的）
    crate::session_status::record_meter(
        &status_path,
        out["context"].as_u64(),
        out["model"].as_str(),
    );
    let p = state_dir.join("usage.json");
    // 內容沒變就不寫（忽略 at：一分鐘內 statusline 會來幾十次）
    if let Some(prev) = std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    {
        let mut a = prev.clone();
        let mut b = out.clone();
        a["at"] = Value::Null;
        b["at"] = Value::Null;
        if a == b {
            return false;
        }
    }
    if std::fs::create_dir_all(state_dir).is_err() {
        return false;
    }
    out["at"] = json!(now.format("%Y-%m-%d %H:%M").to_string());
    let _ = crate::fsutil::write_atomic(&raw_path(state_dir), payload.to_string().as_bytes());
    crate::fsutil::write_atomic(&p, out.to_string().as_bytes()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(h: u32, m: u32) -> NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 10, 16)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap()
    }

    #[test]
    fn summarize_reads_rate_limits_context_and_model() {
        let now = at(20, 50);
        // 今天 23:10 重置（本地）
        let reset_today = Local
            .from_local_datetime(&at(23, 10))
            .single()
            .unwrap()
            .timestamp();
        let payload = json!({
            "model": {"id": "claude-opus-5-5", "display_name": "Opus"},
            "context_window": {"used_percentage": 41.4, "context_window_size": 200000},
            "rate_limits": {
                "five_hour": {"used_percentage": 62.3, "resets_at": reset_today},
                "seven_day": {"utilization": 90, "resets_at": "2026-10-20T01:00:00Z"}
            }
        });
        let v = summarize(&payload, now).unwrap();
        assert_eq!(v["five_hour"]["used"], 62);
        assert_eq!(v["five_hour"]["resets"], "23:10");
        assert_eq!(v["seven_day"]["used"], 90);
        assert!(v["seven_day"]["resets"]
            .as_str()
            .unwrap()
            .starts_with("10-20 "));
        assert_eq!(v["context"], 41);
        assert_eq!(v["model"], "Opus");
        assert_eq!(v["warn"], true);
        assert_eq!(v["at"], "2026-10-16 20:50");
        // 沒有任何可用欄位＝None
        assert!(summarize(&json!({"workspace": {"current_dir": "/x"}}), now).is_none());
        // 只有一個視窗、沒有重置時間
        let v = summarize(
            &json!({"rate_limits": {"five_hour": {"used_percentage": 12}}}),
            now,
        )
        .unwrap();
        assert_eq!(v["five_hour"]["resets"], Value::Null);
        assert_eq!(v["seven_day"], Value::Null);
        assert_eq!(v["warn"], false);
    }

    #[test]
    fn record_writes_only_for_app_sessions_and_only_when_changed() {
        let t = tempfile::tempdir().unwrap();
        let sp = t.path();
        let target = crate::session_status::path_for(sp, "t1");
        let target = target.to_str().unwrap();
        let body = json!({"rate_limits": {"five_hour": {"used_percentage": 10}}}).to_string();
        assert!(!record(&body, None, Some(at(1, 0))));
        assert!(!record(
            &body,
            Some(sp.join("x.json").to_str().unwrap()),
            Some(at(1, 0))
        ));
        assert!(record(&body, Some(target), Some(at(1, 0))));
        let v = read(sp).unwrap();
        assert_eq!(
            (v["five_hour"]["used"].as_u64(), v["at"].as_str()),
            (Some(10), Some("2026-10-16 01:00"))
        );
        assert!(sp.join(".state").join("usage_raw.json").exists());
        // 同內容、不同分鐘：不重寫
        assert!(!record(&body, Some(target), Some(at(1, 1))));
        assert_eq!(read(sp).unwrap()["at"], "2026-10-16 01:00");
        // 數字變了才寫
        let body2 = json!({"rate_limits": {"five_hour": {"used_percentage": 11}}}).to_string();
        assert!(record(&body2, Some(target), Some(at(1, 2))));
        assert_eq!(read(sp).unwrap()["at"], "2026-10-16 01:02");
        // 不是 JSON、沒欄位：不寫
        assert!(!record("not json", Some(target), Some(at(1, 3))));
    }
}
