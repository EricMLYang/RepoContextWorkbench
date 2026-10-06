//! 對應 `tests/test_agent_claude.py`——L3：真 agent 冒煙（人觸發）。需要已登入的 claude CLI。
//!
//! 跑法：`REPOENGINE_RUN_AGENT_TESTS=1 cargo test -p ctx-core --test agent_claude -- --ignored`
//! 預設 `#[ignore]`：不在 L1/L2 消耗額度（Python 版用 `pytest -m agent`＋skipif）。

mod common;

use ctx_core::{agents, pack};

/// Python 版走 `collide.submit`＋`collide.run_judgement(provider="claude")`，再查 spine 的 collision 事件。
/// collide／spine 還沒搬（等第一波 spine/registry），這裡先只驗 claude provider 這一段：
/// 組內容用 pack 打包、真 claude 判定、五欄位驗證、raw log 落檔。spine 事件斷言待 collide 搬完補上。
#[test]
#[ignore = "需 REPOENGINE_RUN_AGENT_TESTS=1 且 claude CLI 在 PATH（真 agent，耗額度）"]
fn test_real_collide_smoke() {
    if std::env::var_os("REPOENGINE_RUN_AGENT_TESTS").is_none() {
        eprintln!("略過：沒設 REPOENGINE_RUN_AGENT_TESTS");
        return;
    }
    let w = common::spine_with_repos();
    let packed = pack::pack_group(&w.refs(), 60000);
    let prompt = format!(
        "你是「碰撞判定器」。判斷【想法】相對【組內容】是：已知、衝突、還是真增量。\n\
         只輸出一個 JSON 物件：{{\"判定\": \"已知|衝突|真增量\", \"理由\": \"…\", \"證據\": \"…\", \
         \"落點建議\": \"…\", \"下一步\": \"…\"}}\n\n【想法】\n想把快取策略改成 LRU（組內筆記已有相同結論）\n\n【組內容】\n{}",
        packed.text
    );
    let j = agents::judge("claude", &prompt, &w.spine, None)
        .expect("真 agent 判定失敗——看 .agent_logs/ 最新一筆 raw log");
    assert!(["已知", "衝突", "真增量"].contains(&j.verdict.as_str()));
    let logs = std::fs::read_dir(w.spine.join(".agent_logs"))
        .unwrap()
        .count();
    assert!(logs > 0);
}
