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

/// 名片覆蓋：只算沒封存的 repo；沒寫介紹的列出來（README 推測不算寫了）。
#[test]
fn health_reports_profile_coverage() {
    use ctx_core::registry::{describe_repo, set_field, Describe};
    let (_t, d) = ctx_core::testkit::spine_with_repos();
    let h = health(&d, 14, None).unwrap();
    assert_eq!(h["profiles"]["repos"], 2);
    assert_eq!(h["profiles"]["described"], 0);
    assert_eq!(
        h["profiles"]["undescribed"],
        serde_json::json!(["repo-a", "repo-b"])
    );
    describe_repo(
        &d,
        "repo-a",
        &Describe {
            description: Some("讀書筆記".into()),
            role: Some("知識庫".into()),
            keywords: None,
        },
    )
    .unwrap();
    set_field(&d, "repo-b", "tier", "archived").unwrap();
    let h = health(&d, 14, None).unwrap();
    let p = &h["profiles"];
    assert_eq!(
        (p["repos"].as_u64(), p["described"].as_u64()),
        (Some(1), Some(1))
    );
    assert_eq!(p["with_role"], 1);
    assert_eq!(p["undescribed"], serde_json::json!([]));
    assert_eq!(p["undescribed_more"], 0);
}

/// F-63 助手用量：App（source assist）與 agent（workbench_apply）都算；有做事、失敗、復原分開數。
#[test]
fn health_counts_assistant_use() {
    let (_t, d) = spine();
    let at = |m: i64| Some(Local::now().naive_local() - Duration::minutes(m));
    let ev = |src: &str, body: &str, m: i64| {
        append_event(&d, "presented", src, &[] as &[&str], body, at(m)).unwrap();
    };
    ev("assist", "助手：切到 g1\n做了：切到牌組「g1」", 30);
    ev(
        "assist",
        "助手：a 加標籤 x\n做了：a：加標籤「x」\n失敗停下：…",
        20,
    );
    ev("assist", "助手：復原：a 加標籤 x\n做了：a：加標籤「x」", 10);
    ev(
        "agent",
        "助手（agent）：改工作台設定\n略過：切到牌組「g1」",
        5,
    );
    ev("session", "組情境卡＋文件地圖：…", 5);
    let r = health(&d, 14, None).unwrap();
    assert_eq!(
        r["assist"],
        serde_json::json!({"runs": 3, "from_agent": 1, "did_something": 2, "failed": 1, "undone": 1})
    );
}
