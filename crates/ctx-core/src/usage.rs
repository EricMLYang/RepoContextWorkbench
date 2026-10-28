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
//! 檔案（`.state/usage.json`）：`{"at","five_hour":{"used","resets","resets_at"}|null,"seven_day":…,"context":n|null,"model","warn"}`，
//! `used` 是 0–100 的整數、`resets` 是排好的本地時間（今天「HH:MM」、不是今天「MM-DD HH:MM」）、
//! `resets_at` 是 epoch 秒（2026-10-28 加，舊檔沒有）、`warn`＝任一視窗 ≥ [`WARN_AT`]。
//! 讀出來時（[`read`]）再補兩個隨時間變的欄位：每個視窗的 `resets_in_min`（還有幾分鐘重置）、整份的 `age_min`（幾分鐘前更新）。
//!
//! 百分比是整個訂閱帳號的用量（claude.ai、其他終端用掉的也算在內），但只有 App 的會話跑完一輪時才更新；
//! App 背景的 `claude -p`（碰撞判定、ask_repo、助手）用訂閱登入時也吃同一份額度。
//!
//! **Codex**（2026-10-28，[`codex`]）：不用另起程序或打網路（Orca 走 app-server RPC＋後端 API），
//! 讀 Codex 自己寫的會話紀錄 `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl`（預設 `~/.codex`）——
//! 每次 `token_count` 事件都帶 `rate_limits.primary／secondary`（`used_percent`、`window_minutes`、`resets_at`）。
//! 這台機器上任何 Codex 會話都會寫，所以不限 App 開的；形狀跟 Claude 的一樣（`five_hour`／`seven_day`）。

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

/// 讀目前的 Claude 額度餘光（補上 `resets_in_min`、`age_min`）；沒有或壞掉＝None。
pub fn read(spine_dir: &Path) -> Option<Value> {
    let s = std::fs::read_to_string(path(spine_dir)).ok()?;
    let mut v = serde_json::from_str::<Value>(&s)
        .ok()
        .filter(|v| v.get("at").and_then(Value::as_str).is_some())?;
    enrich(&mut v, Local::now());
    Some(v)
}

/// 補上隨時間變的欄位：視窗的 `resets_in_min`（有 `resets_at` 才有）、整份的 `age_min`（`at` 認得出才有）。
fn enrich(v: &mut Value, now: DateTime<Local>) {
    for k in ["five_hour", "seven_day"] {
        let Some(w) = v.get_mut(k).and_then(Value::as_object_mut) else {
            continue;
        };
        if let Some(t) = w.get("resets_at").and_then(Value::as_i64) {
            w.insert(
                "resets_in_min".into(),
                json!(((t - now.timestamp()) / 60).max(0)),
            );
        }
    }
    let at = v.get("at").and_then(Value::as_str).and_then(|a| {
        NaiveDateTime::parse_from_str(a, "%Y-%m-%d %H:%M")
            .ok()
            .and_then(|t| Local.from_local_datetime(&t).single())
    });
    if let Some(at) = at {
        v["age_min"] = json!((now - at).num_minutes().max(0));
    }
}

/// 重置時間的 epoch 秒（epoch 數字或 ISO 字串）；認不出＝None。
fn resets_epoch(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => Some(n.as_f64()? as i64),
        Value::String(s) => DateTime::parse_from_rfc3339(s.trim())
            .ok()
            .map(|t| t.timestamp()),
        _ => None,
    }
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
    Some(
        json!({"used": used, "resets": resets_text(w.get("resets_at"), now),
                "resets_at": resets_epoch(w.get("resets_at"))}),
    )
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

// ── Codex ──────────────────────────────────────────────────────────────

/// 會話紀錄只讀尾巴這麼多位元組（`token_count` 事件每輪都有，最後一筆一定在尾巴附近）。
const CODEX_TAIL: u64 = 512 * 1024;

/// `$CODEX_HOME/sessions`（沒設＝`~/.codex/sessions`）。
fn codex_sessions_dir() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::fsutil::expanduser("~/.codex"))
        .join("sessions")
}

/// 名字由大到小的子目錄（`YYYY`／`MM`／`DD` 都是補零的數字，字串排序＝時間排序）。
fn dirs_desc(p: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

/// 最近改過的會話紀錄（新到舊，最多 5 份）：看最新的兩天（昨天開、今天還在跑的會話寫在昨天的目錄）。
/// 剛開的會話還沒有 `token_count`，所以不只看最新那份。
fn newest_rollouts(sessions: &Path) -> Vec<(PathBuf, std::fs::Metadata)> {
    let days = dirs_desc(sessions)
        .into_iter()
        .flat_map(|y| dirs_desc(&y))
        .flat_map(|m| dirs_desc(&m))
        .take(2);
    let mut files = days
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "jsonl")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("rollout-"))
        })
        .filter_map(|p| std::fs::metadata(&p).ok().map(|m| (p, m)))
        .collect::<Vec<_>>();
    files.sort_by_key(|(_, m)| std::cmp::Reverse(m.modified().ok()));
    files.truncate(5);
    files
}

/// 讀檔案尾巴（最後 [`CODEX_TAIL`] 位元組；第一行可能被切一半，丟掉）。
fn read_tail(p: &Path, len: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(p).ok()?;
    let start = len.saturating_sub(CODEX_TAIL);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    Some(if start > 0 {
        text.split_once('\n')
            .map(|(_, r)| r.to_string())
            .unwrap_or_default()
    } else {
        text
    })
}

/// 會話紀錄的文字 → 我們的形狀（跟 Claude 的一樣）。用最後一筆帶 `rate_limits` 的事件；視窗依長度分（300 分＝5h、10080 分＝7 天）。
pub fn summarize_codex(text: &str, now: NaiveDateTime) -> Option<Value> {
    let (line, rl) = text.lines().rev().find_map(|l| {
        if !l.contains("\"rate_limits\"") {
            return None;
        }
        let v: Value = serde_json::from_str(l).ok()?;
        let rl = v.get("payload")?.get("rate_limits")?.clone();
        rl.is_object().then_some((v, rl))
    })?;
    let mut five = None;
    let mut seven = None;
    for k in ["primary", "secondary"] {
        let Some(w) = rl.get(k).and_then(Value::as_object) else {
            continue;
        };
        let Some(used) = w.get("used_percent").and_then(Value::as_f64) else {
            continue;
        };
        let out = json!({
            "used": used.round().clamp(0.0, 100.0) as u64,
            "resets": resets_text(w.get("resets_at"), now),
            "resets_at": resets_epoch(w.get("resets_at")),
        });
        match w.get("window_minutes").and_then(Value::as_u64) {
            Some(m) if m >= 24 * 60 => seven = Some(out),
            Some(_) => five = Some(out),
            None if k == "primary" => five = Some(out),
            None => seven = Some(out),
        }
    }
    if five.is_none() && seven.is_none() {
        return None;
    }
    let at = line
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Local).naive_local())
        .unwrap_or(now);
    let warn = [&five, &seven].iter().any(|w| {
        w.as_ref()
            .and_then(|w| w["used"].as_u64())
            .is_some_and(|u| u >= WARN_AT)
    });
    Some(json!({
        "at": at.format("%Y-%m-%d %H:%M").to_string(),
        "five_hour": five, "seven_day": seven,
        "plan": rl.get("plan_type").and_then(Value::as_str),
        "warn": warn,
    }))
}

/// 上次讀到的（最新那份的檔案、大小、修改時間）→ 結果：App 每 5 秒重抓 state，檔案沒動就不重讀。
type CodexCache = Option<(PathBuf, u64, Option<std::time::SystemTime>, Option<Value>)>;
static CODEX_CACHE: std::sync::Mutex<CodexCache> = std::sync::Mutex::new(None);

/// Codex 的額度餘光（補上 `resets_in_min`、`age_min`）；沒裝 Codex、沒有會話紀錄、紀錄裡沒有額度＝None。
pub fn codex() -> Option<Value> {
    codex_in(&codex_sessions_dir())
}

fn codex_in(sessions: &Path) -> Option<Value> {
    let files = newest_rollouts(sessions);
    let (p, meta) = files.first()?;
    let (p, len, mtime) = (p.clone(), meta.len(), meta.modified().ok());
    let mut cache = CODEX_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let hit = cache
        .as_ref()
        .filter(|(cp, cl, cm, _)| *cp == p && *cl == len && *cm == mtime)
        .map(|(_, _, _, v)| v.clone());
    let v = match hit {
        Some(v) => v,
        None => {
            let now = Local::now().naive_local();
            let v = files
                .iter()
                .find_map(|(f, m)| read_tail(f, m.len()).and_then(|t| summarize_codex(&t, now)));
            *cache = Some((p, len, mtime, v.clone()));
            v
        }
    };
    drop(cache);
    let mut v = v?;
    enrich(&mut v, Local::now());
    Some(v)
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

    #[test]
    fn codex_reads_last_rate_limits_from_rollout_tail() {
        let now = at(20, 50);
        let line = |pct: f64| {
            json!({"timestamp": "2026-10-16T12:40:00.000Z", "type": "event_msg",
                   "payload": {"type": "token_count", "rate_limits": {
                       "limit_id": "codex", "plan_type": "plus",
                       "primary": {"used_percent": pct, "window_minutes": 300, "resets_at": 1793188617},
                       "secondary": {"used_percent": 81.0, "window_minutes": 10080, "resets_at": 1793775417}}}})
            .to_string()
        };
        let text = [
            line(2.0),
            r#"{"type":"response_item","payload":{}}"#.into(),
            line(3.4),
        ]
        .join("\n");
        let v = summarize_codex(&text, now).unwrap();
        assert_eq!(v["five_hour"]["used"], 3);
        assert_eq!(v["five_hour"]["resets_at"], 1793188617);
        assert_eq!(v["seven_day"]["used"], 81);
        assert_eq!(v["plan"], "plus");
        assert_eq!(v["warn"], true);
        assert!(summarize_codex(r#"{"type":"session_meta"}"#, now).is_none());

        // 目錄：最新那份還沒有額度（剛開）→ 往前找；補上 resets_in_min／age_min
        let t = tempfile::tempdir().unwrap();
        let day = t.path().join("2026").join("10").join("16");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("rollout-a.jsonl"), line(5.0) + "\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(day.join("rollout-b.jsonl"), r#"{"type":"session_meta"}"#).unwrap();
        std::fs::write(day.join("other.jsonl"), line(99.0)).unwrap();
        let v = codex_in(t.path()).unwrap();
        assert_eq!(v["five_hour"]["used"], 5);
        assert!(v["five_hour"]["resets_in_min"].is_i64());
        assert!(v["age_min"].is_i64());
        assert!(codex_in(&t.path().join("nope")).is_none());
    }

    #[test]
    fn enrich_adds_minutes_to_reset_and_age() {
        let now = Local.from_local_datetime(&at(20, 50)).single().unwrap();
        let mut v = json!({"at": "2026-10-16 20:20",
                           "five_hour": {"used": 1, "resets_at": now.timestamp() + 150 * 60},
                           "seven_day": {"used": 2, "resets": "10-20 01:00"}});
        enrich(&mut v, now);
        assert_eq!(v["five_hour"]["resets_in_min"], 150);
        assert!(v["seven_day"].get("resets_in_min").is_none());
        assert_eq!(v["age_min"], 30);
    }
}
