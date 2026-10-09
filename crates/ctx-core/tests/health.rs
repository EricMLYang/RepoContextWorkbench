//! Rust 才有（F-36／N-07）：健康指標。

use chrono::{Duration, Local};
use ctx_core::health::{health, record_call};
use ctx_core::spine::append_event;
use ctx_core::testkit::spine;

#[test]
fn health_counts_handoffs_loops_outcomes_and_tools() {
    let (_t, d) = spine();
    let now = Local::now().naive_local();
    let ago = |h: i64| Some(now - Duration::hours(h));
    append_event(
        &d,
        "decision",
        "agent",
        &["id:2026-01-01-d1"],
        "交接：做完 A\n完成：A",
        ago(5),
    )
    .unwrap();
    append_event(
        &d,
        "suggestion",
        "hook",
        &[] as &[&str],
        "agent 會話結束但沒有交接（…）",
        ago(4),
    )
    .unwrap();
    append_event(
        &d,
        "open-loop",
        "agent",
        &["#1", "due:2000-01-01"],
        "opened → 補測試",
        ago(3),
    )
    .unwrap();
    append_event(&d, "open-loop", "agent", &["#2"], "opened → 寫文件", ago(3)).unwrap();
    append_event(&d, "open-loop", "agent", &["#2"], "closed → 寫完了", ago(2)).unwrap();
    // 很久以前的判斷：滿回顧天數、記了結果
    append_event(
        &d,
        "decision",
        "agent",
        &["id:2026-01-01-d2"],
        "快取用 LRU",
        Some(now - Duration::days(30)),
    )
    .unwrap();
    append_event(
        &d,
        "outcome",
        "agent",
        &["ref:decision:2026-01-01-d2"],
        "命中率 92%",
        ago(1),
    )
    .unwrap();
    record_call(&d, "context_for", 120, true);
    record_call(&d, "context_for", 80, true);
    record_call(&d, "close_todo", 1, false);
    let h = health(&d, 14, None).unwrap();
    assert_eq!(h["handoff"]["handoffs"], 1);
    assert_eq!(h["handoff"]["missed"], 1);
    assert_eq!(h["handoff"]["rate"], 0.5);
    assert_eq!(h["loops"]["opened"], 2);
    assert_eq!(h["loops"]["closed"], 1);
    assert_eq!(h["loops"]["open_now"], 1);
    assert_eq!(h["loops"]["overdue"], 1);
    assert_eq!(h["decisions"]["with_outcome"], 1);
    let tools = h["tools"].as_array().unwrap();
    let cf = tools.iter().find(|t| t["tool"] == "context_for").unwrap();
    assert_eq!(
        (cf["calls"].as_u64(), cf["avg_ms"].as_u64()),
        (Some(2), Some(100))
    );
    assert_eq!(
        tools.iter().find(|t| t["tool"] == "close_todo").unwrap()["errors"],
        1
    );
}
