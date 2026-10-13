//! L1：P8 路由。對應 tests/test_pack_route_collide.py 裡 route 相關的測試
//! （pack／collide 的測試由各自模組的搬遷處理）。

use ctx_core::route::route;
use ctx_core::spine::{self, EventType, Query};
use ctx_core::testkit::spine_with_repos;

#[test]
fn test_route_to_repo_inbox() {
    let (_t, d) = spine_with_repos();
    let p = route(&d, "碰撞結論內容", "repo:repo-a", "cache-idea", None).unwrap();
    assert!(p.exists() && p.to_string_lossy().contains("01_inbox"));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "碰撞結論內容");
    let evs = spine::query(&d, &Query::of_type(EventType::Decision)).unwrap();
    assert!(evs.len() == 1 && evs[0].header().contains("repo:repo-a"));
}

#[test]
fn test_route_to_incubator_and_group() {
    let (_t, d) = spine_with_repos();
    let p1 = route(&d, "x", "incubator", "idea", None).unwrap();
    let p2 = route(&d, "y", "group:g1", "idea", None).unwrap();
    assert!(
        p1.to_string_lossy().contains("incubator") && p2.to_string_lossy().contains("materials")
    );
    assert!(route(&d, "z", "nowhere:abc", "idea", None).is_err());
}

/// 組名有空白（2026-10-14）：落點寫成換寫過的 group:PM_Group 時，檔案放進登記原名的組資料夾。
#[test]
fn test_route_to_group_with_space_uses_registered_folder() {
    let (_t, d) = spine_with_repos();
    ctx_core::registry::add_group(&d, "PM Group", &["repo-a"]).unwrap();
    let p = route(&d, "結論", "group:PM_Group", "idea", None).unwrap();
    assert!(
        p.starts_with(d.join("groups").join("PM Group").join("materials")),
        "{p:?}"
    );
}
