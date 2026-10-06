//! P12 排程——timer 住殼內（S6）：殼（桌面 App／`ctx timer`）按時呼叫引擎，不用 OS 排程器。
//! 對應 `repo_context/timer.py`。
//!
//! - config.yaml `schedule:` 每項 {task: brief|digest|upstream|ruminate|commit, at: "HH:MM", group: <組名|省略>}
//! - 補課規則（檢討④）：錯過排程（睡眠／登出／殼未開）→ 醒來後**當日內補跑一次，跨日不補**
//!   （實作＝到點後當天只要沒跑過就跑；狀態記 `.state/timer.json` 的最後執行日）
//! - 任務失敗必須浮出：落 system-unsure 事件（准打斷類），不准沉默。
//!
//! 與 Python 的差異：`run_loop` 不是固定每 30 秒 tick，而是睡到下一個排程點（最多睡
//! [`MAX_SLEEP`]，好接住設定變更與系統睡眠後的時鐘跳動），並可由 channel 停止——桌面殼閒置時不吃 CPU。

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use chrono::{Local, NaiveDateTime, NaiveTime};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::config::{self, ScheduleEntry};
use crate::{brief, digest, ruminate, spine, upstream, Error, Result};

/// run_loop 單次最長睡多久（之後重讀 config、重算下一個排程點）。
pub const MAX_SLEEP: Duration = Duration::from_secs(300);
const RETRY: Duration = Duration::from_secs(30);

fn state_path(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("timer.json")
}

/// 對應 `timer._load_state`：沒檔＝空；壞檔＝錯誤（同 Python json.loads 例外）。
pub fn load_state(spine_dir: &Path) -> Result<Map<String, Value>> {
    let p = state_path(spine_dir);
    if !p.exists() {
        return Ok(Map::new());
    }
    match serde_json::from_str(&std::fs::read_to_string(p)?)? {
        Value::Object(m) => Ok(m),
        _ => Err(Error::invalid("timer.json 不是物件")),
    }
}

fn save_state(spine_dir: &Path, state: &Map<String, Value>) -> Result<()> {
    let p = state_path(spine_dir);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    // 同 Python json.dumps(ensure_ascii=False, indent=1)
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    state.serialize(&mut ser)?;
    std::fs::write(p, buf)?;
    Ok(())
}

/// 對應 `timer._key`：`task@at@group`。
pub fn key(e: &ScheduleEntry) -> String {
    format!("{}@{}@{}", e.task, e.at, e.group.as_deref().unwrap_or(""))
}

/// Python `e.get("at", "99:99")`：沒寫 at＝永不到點。
fn at_of(e: &ScheduleEntry) -> &str {
    if e.at.is_empty() {
        "99:99"
    } else {
        &e.at
    }
}

/// 對應 `timer.due_entries`（純函數）：到點且今天沒跑過的任務。跨日不補：只比對「今天」。
pub fn due_entries(
    schedule: &[ScheduleEntry],
    state: &Map<String, Value>,
    now: NaiveDateTime,
) -> Vec<ScheduleEntry> {
    let today = now.format("%Y-%m-%d").to_string();
    let cur = now.format("%H:%M").to_string();
    schedule
        .iter()
        .filter(|e| {
            at_of(e) <= cur.as_str() && state.get(&key(e)).and_then(Value::as_str) != Some(&today)
        })
        .cloned()
        .collect()
}

/// 對應 `timer.run_task`：跑一項排程任務。
pub fn run_task(spine_dir: &Path, e: &ScheduleEntry) -> Result<()> {
    let group = e.group.as_deref().filter(|g| !g.is_empty());
    match e.task.as_str() {
        "brief" => brief::run(spine_dir, group, None, None).map(|_| ()),
        "digest" => digest::run(spine_dir, group, None, None, None).map(|_| ()),
        "upstream" => upstream::check(spine_dir, group, None, None).map(|_| ()),
        "ruminate" => ruminate::run(spine_dir, None, None).map(|_| ()),
        "commit" => spine::batch_commit(spine_dir, "spine: scheduled batch commit").map(|_| ()),
        other => Err(Error::invalid(format!(
            "未知排程任務: {other}（brief|digest|upstream|ruminate|commit）"
        ))),
    }
}

/// 對應 `timer.tick`：跑一輪到期任務。回傳 [(key, ok)]。成功失敗都記「今天跑過」——失敗浮出而非重試轟炸。
/// `schedule=None`＝讀 config `schedule`。
pub fn tick(
    spine_dir: &Path,
    now: Option<NaiveDateTime>,
    schedule: Option<&[ScheduleEntry]>,
) -> Result<Vec<(String, bool)>> {
    let now = now.unwrap_or_else(|| Local::now().naive_local());
    let owned;
    let schedule = match schedule {
        Some(s) => s,
        None => {
            owned = config::load(spine_dir)?.schedule();
            &owned
        }
    };
    let mut state = load_state(spine_dir)?;
    let mut ran = Vec::new();
    for e in due_entries(schedule, &state, now) {
        let k = key(&e);
        match run_task(spine_dir, &e) {
            Ok(()) => ran.push((k.clone(), true)),
            Err(err) => {
                // 失敗必須浮出，不准沉默
                spine::append_event(
                    spine_dir,
                    "suggestion",
                    "timer",
                    spine::NO_TOKENS,
                    &format!("system-unsure：排程任務 {k} 失敗：{err}"),
                    Some(now),
                )?;
                ran.push((k.clone(), false));
            }
        }
        state.insert(k, Value::String(now.format("%Y-%m-%d").to_string()));
    }
    if !ran.is_empty() {
        save_state(spine_dir, &state)?;
    }
    Ok(ran)
}

/// 下一個需要醒來的時間點（純函數）：今天還沒跑、排程時刻在 now 之後的最早一項；
/// 今天的都處理過了＝明天最早的一項。沒有排程（或 at 都不合法）＝None。
/// 已到點但今天沒跑的（due）＝now（立刻醒）。
pub fn next_wake(
    schedule: &[ScheduleEntry],
    state: &Map<String, Value>,
    now: NaiveDateTime,
) -> Option<NaiveDateTime> {
    if !due_entries(schedule, state, now).is_empty() {
        return Some(now);
    }
    let today = now.date();
    let times: Vec<NaiveTime> = schedule
        .iter()
        .filter_map(|e| NaiveTime::parse_from_str(&e.at, "%H:%M").ok())
        .collect();
    let later_today = times
        .iter()
        .map(|t| today.and_time(*t))
        .filter(|t| *t > now)
        .min();
    later_today.or_else(|| {
        let tomorrow = today.succ_opt()?;
        times.iter().map(|t| tomorrow.and_time(*t)).min()
    })
}

/// 對應 `timer.run_loop`：殼的常駐引信。睡到下一個排程點（最多 [`MAX_SLEEP`]）再 tick；
/// `stop` 收到訊息或 sender 全被丟掉就結束。tick 失敗（壞 config 等）吞掉，只保 loop 不死。
pub fn run_loop(spine_dir: &Path, stop: &Receiver<()>) {
    loop {
        let _ = tick(spine_dir, None, None);
        let now = Local::now().naive_local();
        let wait = config::load(spine_dir)
            .ok()
            .map(|c| c.schedule())
            .and_then(|sch| {
                let st = load_state(spine_dir).unwrap_or_default();
                next_wake(&sch, &st, now)
            })
            // 已到點卻沒跑掉（tick 本身失敗）＝隔 30 秒再試（同 Python interval），不忙等
            .map(|t| (t - now).to_std().unwrap_or(RETRY))
            .map(|d| if d.is_zero() { RETRY } else { d })
            .unwrap_or(MAX_SLEEP)
            .min(MAX_SLEEP);
        match stop.recv_timeout(wait) {
            Err(RecvTimeoutError::Timeout) => continue,
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}
