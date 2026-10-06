//! L1：P1 關係層（2026-09-08 組為單位輪）——A 是 B 的 PM 這類 repo 間關係要能設、能查、能稽核。
//! 對應 tests/test_relations.py。

use std::collections::{BTreeSet, HashMap};

use ctx_core::registry::{self, AddRepo, Direction, RelationRow};
use ctx_core::testkit::{make_git_repo, spine_with_repos};

const NONE: &[&str] = &[];

fn err<T: std::fmt::Debug>(r: ctx_core::Result<T>) -> String {
    r.unwrap_err().to_string()
}

fn labels(d: &std::path::Path, id: &str) -> HashMap<(String, Direction), String> {
    registry::relations_of(d, id)
        .unwrap()
        .into_iter()
        .map(|x| ((x.peer, x.direction), x.label))
        .collect()
}

#[test]
fn test_relate_roundtrip_and_rejects() {
    let (_t, d) = spine_with_repos();
    let r = registry::relate(&d, "repo-a", "repo-b", "pm-of", Some("a 規劃 b"), NONE).unwrap();
    assert_eq!(
        serde_json::to_value(&r).unwrap(),
        serde_json::json!({"to": "repo-b", "kind": "pm-of", "note": "a 規劃 b"})
    );
    // 冪等：同 (a,b,kind) 再設一次不會變兩筆，note 以最新為準
    registry::relate(&d, "repo-a", "repo-b", "pm-of", Some("改註"), NONE).unwrap();
    assert_eq!(
        registry::relations(&d, None).unwrap(),
        vec![RelationRow {
            from: "repo-a".into(),
            to: "repo-b".into(),
            kind: "pm-of".into(),
            note: Some("改註".into()),
            exports: vec![]
        }]
    );
    // 不同 kind 可並存
    registry::relate(&d, "repo-a", "repo-b", "feeds", None, NONE).unwrap();
    assert_eq!(registry::relations(&d, None).unwrap().len(), 2);
    assert!(err(registry::relate(
        &d, "repo-a", "repo-a", "pm-of", None, NONE
    ))
    .contains("自己"));
    assert!(err(registry::relate(&d, "repo-a", "nope", "pm-of", None, NONE)).contains("不存在"));
    assert!(err(registry::relate(
        &d, "repo-a", "repo-b", "boss-of", None, NONE
    ))
    .contains("kind"));
    // unrelate：指定 kind 只刪那一筆；不給 kind 全刪
    registry::unrelate(&d, "repo-a", "repo-b", Some("feeds")).unwrap();
    let kinds: Vec<String> = registry::relations(&d, None)
        .unwrap()
        .into_iter()
        .map(|x| x.kind)
        .collect();
    assert_eq!(kinds, vec!["pm-of"]);
    // 寫入真的落 registry.yaml（資產是純文字，下一代工具直接讀得到）
    assert!(std::fs::read_to_string(d.join("registry.yaml"))
        .unwrap()
        .contains("pm-of"));
    registry::unrelate(&d, "repo-a", "repo-b", None).unwrap();
    assert!(registry::relations(&d, None).unwrap().is_empty());
}

#[test]
fn test_relations_of_labels_and_neighbors() {
    let (tmp, d) = spine_with_repos();
    registry::add_repo(
        &d,
        "repo-c",
        make_git_repo(tmp.path(), "repo-c", 0, 0, true),
        AddRepo::default(),
    )
    .unwrap();
    registry::relate(&d, "repo-a", "repo-b", "pm-of", None, NONE).unwrap();
    registry::relate(&d, "repo-c", "repo-a", "feeds", None, NONE).unwrap();
    registry::relate(&d, "repo-b", "repo-c", "sibling-topic", None, NONE).unwrap();
    let k = |p: &str, dir| (p.to_string(), dir);
    let at_a = labels(&d, "repo-a");
    assert_eq!(at_a[&k("repo-b", Direction::Out)], "規劃（PM）"); // a → b：a 是 b 的 PM
    assert_eq!(at_a[&k("repo-c", Direction::In)], "吃料自"); // c feeds a：a 吃料自 c
    let at_b = labels(&d, "repo-b");
    assert_eq!(at_b[&k("repo-a", Direction::In)], "PM 是"); // 反向讀法
    assert_eq!(at_b[&k("repo-c", Direction::Out)], "同主題"); // 對稱關係兩向同字
    let at_c = labels(&d, "repo-c");
    assert_eq!(at_c[&k("repo-b", Direction::In)], "同主題");
    // 一跳鄰居（兩向），臨時組〔＋相關 repo〕用
    let set = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
    assert_eq!(
        registry::related_ids(&d, &["repo-a"]).unwrap(),
        set(&["repo-b", "repo-c"])
    );
    assert_eq!(
        registry::related_ids(&d, &["repo-b"]).unwrap(),
        set(&["repo-a", "repo-c"])
    );
    // 人話行：組情境卡用
    let lines = registry::relation_lines(&d, &["repo-a", "repo-b"]).unwrap();
    assert!(lines
        .iter()
        .any(|ln| ln.contains("repo-a") && ln.contains("規劃（PM）") && ln.contains("repo-b")));
    assert!(lines
        .iter()
        .any(|ln| ln.contains("repo-c") && ln.contains("組外"))); // 指到組外要標明
}

#[test]
fn test_custom_kind_from_config() {
    let (_t, d) = spine_with_repos();
    std::fs::write(
        d.join("config.yaml"),
        "relations:\n  kinds:\n    tests: {forward: 驗證, inverse: 被驗證於}\n",
    )
    .unwrap();
    let kinds = registry::relation_kinds(&d).unwrap();
    assert!(kinds.contains("tests") && kinds.contains("pm-of"));
    registry::relate(&d, "repo-b", "repo-a", "tests", None, NONE).unwrap();
    let at_a: HashMap<String, String> = registry::relations_of(&d, "repo-a")
        .unwrap()
        .into_iter()
        .map(|x| (x.peer, x.label))
        .collect();
    assert_eq!(at_a["repo-b"], "被驗證於");
}

#[test]
fn test_remove_repo_strips_relations_and_audit_dangling() {
    let (_t, d) = spine_with_repos();
    registry::relate(&d, "repo-a", "repo-b", "pm-of", None, NONE).unwrap();
    registry::remove_repo(&d, "repo-b").unwrap();
    assert!(registry::relations(&d, None).unwrap().is_empty()); // 指向被移除 repo 的關係一併清掉
                                                                // 手改 yaml 留下斷鏈 → audit 紅字
    let mut data = registry::load(&d).unwrap();
    let rel: serde_yaml::Value = serde_yaml::from_str("[{to: ghost, kind: pm-of}]").unwrap();
    data.repos[0].set("relations", rel);
    registry::save(&d, &data).unwrap();
    let reds = registry::audit(&d, NONE).unwrap();
    assert!(reds
        .iter()
        .any(|r| r.contains("關係斷鏈") && r.contains("ghost")));
}

// ── exports（registry_export 背後）─────────────────────────────────────

#[test]
fn exports_set_relate_and_remove() {
    let (_t, d) = spine_with_repos();
    let r = registry::set_export(&d, "repo-a", "筆記", "notes", Some("計劃書")).unwrap();
    assert_eq!(
        (r.path.as_str(), r.desc.as_deref()),
        ("notes/", Some("計劃書"))
    );
    assert!(err(registry::set_export(&d, "repo-a", "x", "../repo-b", None)).contains("跳出 repo"));
    assert!(err(registry::set_export(&d, "repo-a", "x", "nope", None)).contains("不存在"));
    assert!(err(registry::set_export(&d, "repo-a", "a:b", "notes", None)).contains("不能空白"));
    assert!(err(registry::relate(
        &d,
        "repo-a",
        "repo-b",
        "feeds",
        None,
        &["書摘"]
    ))
    .contains("沒有 export"));
    let rel = registry::relate(&d, "repo-a", "repo-b", "feeds", None, &["筆記"]).unwrap();
    assert_eq!(rel.exports, vec!["筆記"]);
    assert!(registry::remove_export(&d, "repo-a", "筆記").unwrap());
    assert!(!registry::remove_export(&d, "repo-a", "筆記").unwrap());
    let repo = registry::get_repo(&d, "repo-a").unwrap();
    assert!(repo.exports().is_empty() && repo.get("exports").is_none());
    assert!(repo.relations()[0].exports.is_empty()); // 引用它的關係也清掉
}
