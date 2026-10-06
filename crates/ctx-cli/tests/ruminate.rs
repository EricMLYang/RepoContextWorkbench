//! 反芻輪的 CLI 入口。對應 tests/test_ruminate.py 的 test_cli_fresh（ctx-core tests/ruminate.rs 標「等 ctx-cli」）。

mod common;

use common::*;
use ctx_core::agentapi::{self as api, Target};
use ctx_core::registry::{self, AddRepo};

const CARD: &str = "# 向量索引的冷啟動與增量重建

增量重建要記水位線，冷啟動時先載入快照再補增量。
倒排索引的分段合併策略會影響查詢延遲；合併太頻繁會拖垮寫入吞吐。
";

#[test]
fn test_cli_fresh() {
    // 兩組：產品（impl，手上在做索引重建）、知識（cards，卡片 repo）
    let (t, sp) = spine();
    let impl_ = make_git_repo(t.path(), "impl", 0, 0, true);
    let cards = make_git_repo(t.path(), "cards", 0, 0, true);
    registry::add_repo(&sp, "impl", &impl_, AddRepo::default()).unwrap();
    registry::add_repo(&sp, "cards", &cards, AddRepo::default()).unwrap();
    registry::add_group(&sp, "產品", &["impl"]).unwrap();
    registry::add_group(&sp, "知識", &["cards"]).unwrap();
    registry::set_group_field(&sp, "產品", "goal", "把搜尋服務的向量索引改成增量重建").unwrap();
    api::add_todo(
        &sp,
        "冷啟動太慢：載入快照後補增量，量測查詢延遲",
        None,
        &Target::at(&impl_),
        false,
        "agent",
    )
    .unwrap();
    api::log_decision(
        &sp,
        "倒排索引分段合併改成每小時一次，避免拖垮寫入吞吐",
        &Target::at(&impl_),
        None,
        false,
        "agent",
    )
    .unwrap();

    std::fs::write(cards.join("索引重建.md"), CARD).unwrap();
    let c = ctx(t.path()).spine(&sp);
    let p = c.run(&["fresh", "--run"]);
    assert!(
        p.code == 0 && p.stdout.contains("1 筆新推薦"),
        "{}",
        p.stdout
    );
    let p = c.run(&["fresh", "--json", "--cwd", s(&impl_)]);
    let data = p.json();
    assert_eq!(data["links"][0]["ref"], "cards:索引重建.md");
    let id = data["links"][0]["id"].as_str().unwrap().to_string();
    let p = c.run(&["fresh", "--dismiss", &id]);
    assert_eq!(p.code, 0);
    let p = c.run(&["fresh", "--stats"]);
    assert!(p.stdout.contains("略過 1"));
}
