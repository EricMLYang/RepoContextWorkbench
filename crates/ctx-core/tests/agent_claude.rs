//! 對應 `tests/test_agent_claude.py`——L3：真 agent 冒煙（人觸發）。需要已登入的 claude CLI。
//!
//! 跑法：`REPOENGINE_RUN_AGENT_TESTS=1 cargo test -p ctx-core --test agent_claude -- --ignored`
//! 預設 `#[ignore]`：不在 L1/L2 消耗額度（Python 版用 `pytest -m agent`＋skipif）。

use ctx_core::collide;
use ctx_core::spine::{self, EventType, Query};
use ctx_core::testkit::spine_with_repos;

#[test]
#[ignore = "需 REPOENGINE_RUN_AGENT_TESTS=1 且 claude CLI 在 PATH（真 agent，耗額度）"]
fn test_real_collide_smoke() {
    if std::env::var_os("REPOENGINE_RUN_AGENT_TESTS").is_none() {
        eprintln!("略過：沒設 REPOENGINE_RUN_AGENT_TESTS");
        return;
    }
    let (_t, d) = spine_with_repos();
    let cid = collide::submit(
        &d,
        "想把快取策略改成 LRU（組內筆記已有相同結論）",
        Some("g1"),
        None,
        "hotkey",
        None,
    )
    .unwrap();
    let j = collide::run_judgement(&d, &cid, None, None, Some("claude"), None)
        .unwrap()
        .expect("真 agent 判定失敗——看 .agent_logs/ 最新一筆 raw log");
    assert!(["已知", "衝突", "真增量"].contains(&j.verdict.as_str()));
    assert!(std::fs::read_dir(d.join(".agent_logs")).unwrap().count() > 0);
    let results: Vec<_> = spine::query(&d, &Query::of_type(EventType::Collision))
        .unwrap()
        .into_iter()
        .filter(|e| e.kv("ref") == Some(format!("collision:{cid}").as_str()))
        .collect();
    assert_eq!(results.len(), 1);
}
