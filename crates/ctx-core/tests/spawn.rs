//! P9 生：升格建 repo＋出生登記（origin 血統）＋outcome 回連（命中率）＋存活率視圖。對應 tests/test_spawn.py。

use ctx_core::registry::{self, Survival};
use ctx_core::spawn::spawn;
use ctx_core::spine::{self, EventType};
use ctx_core::testkit::spine;

/// collide 尚未搬（另一波）：照 `collide.submit` 落的事件形狀直接寫一筆 opened 碰撞。
fn submit_collision(d: &std::path::Path, idea: &str) -> String {
    let cid = format!("{}-a", chrono::Local::now().format("%Y-%m-%d"));
    spine::append_event(
        d,
        "collision",
        "hotkey",
        &[format!("id:{cid}")],
        &format!("opened\n輸入：{idea}"),
        None,
    )
    .unwrap();
    cid
}

#[test]
fn test_spawn_from_collision_closes_the_loop() {
    let (tmp, d) = spine();
    let cid = submit_collision(&d, "值得長成 repo 的想法");
    let target = tmp.path().join("born-repo");
    let origin = format!("collision:{cid}");
    let p = spawn(&d, "born-repo", &target, None, Some(&origin), None).unwrap();
    assert!(p.join(".git").is_dir() && p.join("README.md").is_file());
    assert_eq!(
        registry::get_repo(&d, "born-repo").unwrap().origin(),
        Some(origin)
    );
    // decision 出生登記 ＋ outcome 回連
    let types: Vec<EventType> = spine::iter_events(&d)
        .unwrap()
        .into_iter()
        .map(|e| e.etype)
        .collect();
    assert!(types.contains(&EventType::Decision) && types.contains(&EventType::Outcome));
    let s = spine::stats(&d).unwrap();
    assert!(s.collisions == 1 && s.collisions_with_outcome == 1);
    assert_eq!(s.hit_rate, Some(1.0)); // M2 驗收③：命中率從真資料算得出來
    assert_eq!(
        registry::survival(&d).unwrap(),
        Survival {
            spawned: 1,
            alive: 1,
            survival_rate: Some(1.0)
        }
    );
}

#[test]
fn test_spawn_moves_incubator_material() {
    let (tmp, d) = spine();
    let inc = d.join("incubator");
    std::fs::write(inc.join("idea.md"), "種子內容\n").unwrap();
    let p = spawn(&d, "r2", tmp.path().join("r2"), Some("idea.md"), None, None).unwrap();
    assert!(!inc.join("idea.md").exists()); // 搬離 incubator＝升格完成
    assert_eq!(
        std::fs::read_to_string(p.join("idea.md")).unwrap(),
        "種子內容\n"
    );
}

#[test]
fn test_spawn_rejects_nonempty_target_and_missing_material() {
    let (tmp, d) = spine();
    let busy = tmp.path().join("busy");
    std::fs::create_dir(&busy).unwrap();
    std::fs::write(busy.join("x.txt"), "x").unwrap();
    assert!(spawn(&d, "r3", &busy, None, None, None)
        .unwrap_err()
        .to_string()
        .contains("非空"));
    let e = spawn(&d, "r4", tmp.path().join("r4"), Some("nope.md"), None, None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("素材不存在"));
    // 失敗的 spawn 不准弄髒 registry
    assert!(registry::load(&d)
        .unwrap()
        .repos
        .iter()
        .all(|r| r.id() != "r3" && r.id() != "r4"));
}
