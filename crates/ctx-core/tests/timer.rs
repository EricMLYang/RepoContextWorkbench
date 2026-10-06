//! P12 timer：到期判定純函數（當日補課、同日不重跑、跨日重來）＋tick 真跑任務＋失敗浮出。
//! 對應 tests/test_timer.py；另含 tests/test_ruminate.py::test_timer_task。

use std::sync::mpsc;
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDate, NaiveDateTime};
use ctx_core::agentapi::{self as api, Target};
use ctx_core::config::ScheduleEntry;
use ctx_core::registry::{self, AddRepo};
use ctx_core::testkit::{make_git_repo, spine, spine_with_repos};
use ctx_core::{ruminate, spine as spine_mod, timer};
use serde_json::{json, Map, Value};

fn entry(task: &str, at: &str, group: Option<&str>) -> ScheduleEntry {
    ScheduleEntry {
        task: task.into(),
        at: at.into(),
        group: group.map(str::to_string),
    }
}

fn dt(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, mo, d)
        .unwrap()
        .and_hms_opt(h, mi, 0)
        .unwrap()
}

fn state(pairs: &[(&str, &str)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect()
}

const KEY: &str = "brief@07:30@g1";

#[test]
fn test_due_entries_rules() {
    let sched = vec![entry("brief", "07:30", Some("g1"))];
    let due = |h: u32, m: u32, st: &Map<String, Value>| {
        timer::due_entries(&sched, st, dt(2026, 9, 1, h, m))
    };
    let empty = Map::new();
    assert!(due(7, 0, &empty).is_empty()); // 還沒到點
    assert_eq!(due(7, 30, &empty), sched); // 到點
    assert_eq!(due(23, 0, &empty), sched); // 補課：當日內晚到也跑
    assert!(due(23, 0, &state(&[(KEY, "2026-09-01")])).is_empty()); // 同日跑過不重跑
    assert_eq!(
        timer::due_entries(&sched, &state(&[(KEY, "2026-09-01")]), dt(2026, 9, 2, 8, 0)),
        sched
    ); // 跨日重來
       // 跨日不補昨天的：state 空、新的一天早於排程時刻 → 不跑
    assert!(timer::due_entries(&sched, &empty, dt(2026, 9, 2, 6, 0)).is_empty());
}

#[test]
fn test_tick_runs_brief_once_per_day() {
    let (_t, sp) = spine_with_repos();
    let sched = vec![entry("brief", "00:00", Some("g1"))];
    let ran = timer::tick(&sp, None, Some(&sched)).unwrap();
    assert_eq!(ran, vec![("brief@00:00@g1".to_string(), true)]);
    assert!(spine_mod::iter_events(&sp)
        .unwrap()
        .iter()
        .any(|e| e.etype == spine_mod::EventType::Presented));
    let today = Local::now().format("%Y-%m-%d").to_string();
    assert!(sp
        .join("groups/g1/briefs")
        .join(format!("{today}.md"))
        .exists());
    let st: Value =
        serde_json::from_str(&std::fs::read_to_string(sp.join(".state/timer.json")).unwrap())
            .unwrap();
    assert_eq!(st["brief@00:00@g1"], json!(today));
    assert!(timer::tick(&sp, None, Some(&sched)).unwrap().is_empty()); // 同日第二輪不重跑
}

#[test]
fn test_tick_failure_floats_and_does_not_retry_bomb() {
    let (_t, sp) = spine();
    let sched = vec![entry("brief", "00:00", Some("no-such-group"))];
    let ran = timer::tick(&sp, None, Some(&sched)).unwrap();
    assert_eq!(ran, vec![("brief@00:00@no-such-group".to_string(), false)]);
    let evs: Vec<_> = spine_mod::iter_events(&sp)
        .unwrap()
        .into_iter()
        .filter(|e| e.source == "timer")
        .collect();
    assert!(evs.len() == 1 && evs[0].body.contains("system-unsure")); // 失敗必須浮出
    assert!(timer::tick(&sp, None, Some(&sched)).unwrap().is_empty()); // 失敗也記今天跑過——不轟炸
}

const CARD: &str = "# 向量索引的冷啟動與增量重建

增量重建要記水位線，冷啟動時先載入快照再補增量。
倒排索引的分段合併策略會影響查詢延遲；合併太頻繁會拖垮寫入吞吐。
";

/// tests/test_ruminate.py::test_timer_task
#[test]
fn test_timer_task() {
    let (t, sp) = spine();
    let impl_ = make_git_repo(t.path(), "impl", 0, 0, true);
    let cards = make_git_repo(t.path(), "cards", 0, 0, true);
    registry::add_repo(&sp, "impl", &impl_, AddRepo::default()).unwrap();
    registry::add_repo(&sp, "cards", &cards, AddRepo::default()).unwrap();
    registry::add_group(&sp, "產品", &["impl"]).unwrap();
    registry::add_group(&sp, "知識", &["cards"]).unwrap();
    registry::set_group_field(&sp, "產品", "goal", "把搜尋服務的向量索引改成增量重建").unwrap();
    let ti = Target::at(&impl_);
    api::add_todo(
        &sp,
        "冷啟動太慢：載入快照後補增量，量測查詢延遲",
        None,
        &ti,
        false,
        "agent",
    )
    .unwrap();
    api::log_decision(
        &sp,
        "倒排索引分段合併改成每小時一次，避免拖垮寫入吞吐",
        &ti,
        None,
        false,
        "agent",
    )
    .unwrap();
    std::fs::write(cards.join("索引重建.md"), CARD).unwrap();
    let now = Local::now()
        .naive_local()
        .date()
        .and_hms_opt(23, 0, 0)
        .unwrap();
    let ran = timer::tick(&sp, Some(now), Some(&[entry("ruminate", "06:00", None)])).unwrap();
    assert_eq!(ran, vec![("ruminate@06:00@".to_string(), true)]);
    assert!(!ruminate::fresh(&sp, "產品", false).unwrap().is_empty());
}

// ---- Rust 版補測（非 Python 一對一）----

/// 下一個醒來點：今天稍後的排程；今天都跑過＝明天最早一項；已到點未跑＝現在。
#[test]
fn next_wake_points() {
    let sched = vec![
        entry("brief", "07:30", Some("g1")),
        entry("commit", "18:00", None),
    ];
    let empty = Map::new();
    assert_eq!(
        timer::next_wake(&sched, &empty, dt(2026, 9, 1, 6, 0)),
        Some(dt(2026, 9, 1, 7, 30))
    );
    let ran = state(&[(KEY, "2026-09-01")]);
    assert_eq!(
        timer::next_wake(&sched, &ran, dt(2026, 9, 1, 8, 0)),
        Some(dt(2026, 9, 1, 18, 0))
    );
    let all = state(&[(KEY, "2026-09-01"), ("commit@18:00@", "2026-09-01")]);
    assert_eq!(
        timer::next_wake(&sched, &all, dt(2026, 9, 1, 19, 0)),
        Some(dt(2026, 9, 2, 7, 30))
    );
    assert_eq!(
        timer::next_wake(&sched, &empty, dt(2026, 9, 1, 8, 0)),
        Some(dt(2026, 9, 1, 8, 0))
    );
    assert_eq!(timer::next_wake(&[], &empty, dt(2026, 9, 1, 8, 0)), None);
}

/// run_loop 可停：丟掉 sender 就立刻結束（不會睡滿 MAX_SLEEP）。
#[test]
fn run_loop_stops_on_signal() {
    let (_t, sp) = spine();
    let (tx, rx) = mpsc::channel::<()>();
    let start = Instant::now();
    let h = std::thread::spawn(move || timer::run_loop(&sp, &rx));
    std::thread::sleep(Duration::from_millis(200));
    tx.send(()).unwrap();
    h.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(10));
}
