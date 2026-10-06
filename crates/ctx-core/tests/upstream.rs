//! P4 上游：fake fetch 注入——新 release 落事件、同一筆不重報（無事不報）、kinds/prerelease 過濾。
//! 對應 tests/test_upstream.py。

use std::path::Path;

use ctx_core::registry::{self, AddRepo};
use ctx_core::spine::{self, EventType};
use ctx_core::testkit::{spine, spine_with_repos};
use ctx_core::upstream::{self, Latest};
use ctx_core::Result;

fn reg_external(spine_dir: &Path, tmp: &Path) -> String {
    let rid = "ext-lib";
    let p = tmp.join(rid);
    std::fs::create_dir_all(&p).unwrap();
    registry::add_repo(
        spine_dir,
        rid,
        &p,
        AddRepo {
            kind: "external".into(),
            upstream: Some("owner/lib".into()),
            ..Default::default()
        },
    )
    .unwrap();
    rid.into()
}

fn upstream_events(d: &Path) -> Vec<spine::Event> {
    spine::iter_events(d)
        .unwrap()
        .into_iter()
        .filter(|e| e.source == "upstream")
        .collect()
}

#[test]
fn test_normalize_tolerates_url_forms() {
    assert_eq!(upstream::normalize("owner/lib"), "owner/lib");
    assert_eq!(
        upstream::normalize("https://github.com/owner/lib.git"),
        "owner/lib"
    );
    assert_eq!(upstream::normalize("github.com/owner/lib/"), "owner/lib");
}

#[test]
fn test_new_release_event_then_no_repeat() {
    let (t, d) = spine();
    let rid = reg_external(&d, t.path());
    let mut calls: Vec<(String, String, bool)> = Vec::new();
    let mut fetch = |up: &str, kind: &str, pre: bool| -> Result<Option<Latest>> {
        calls.push((up.into(), kind.into(), pre));
        Ok(Some(Latest {
            kind: "release".into(),
            id: "v1.2.0".into(),
            title: "big".into(),
            url: "http://x".into(),
        }))
    };
    let (n, findings) = upstream::check_with(&d, None, None, &mut fetch, None).unwrap();
    assert_eq!(n, 1);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].repo, rid);
    let evs = upstream_events(&d);
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].etype, EventType::Suggestion);
    assert!(evs[0].body.contains("v1.2.0"));
    assert_eq!(evs[0].kv("repo"), Some(rid.as_str()));
    // 同一筆不重報（洪流不進未讀）
    let (n2, findings2) = upstream::check_with(&d, None, None, &mut fetch, None).unwrap();
    assert_eq!(n2, 1);
    assert!(findings2.is_empty());
    assert_eq!(upstream_events(&d).len(), 1);
    // 預設 kinds=[release]、無 prerelease
    assert_eq!(
        calls[0],
        ("owner/lib".to_string(), "release".to_string(), false)
    );
    assert_eq!(calls.len(), 2);
    // 合約：.state/upstream.json 的形狀
    let state = std::fs::read_to_string(d.join(".state/upstream.json")).unwrap();
    assert_eq!(
        state,
        "{\n \"ext-lib\": {\n  \"release\": \"v1.2.0\"\n }\n}"
    );
}

#[test]
fn test_kinds_config_drives_fetch() {
    let (t, d) = spine();
    reg_external(&d, t.path());
    std::fs::write(
        d.join("config.yaml"),
        "thresholds:\n  upstream:\n    kinds: [release, commit]\n    prerelease: true\n",
    )
    .unwrap();
    let mut seen_kinds: Vec<(String, bool)> = Vec::new();
    let mut fetch = |_up: &str, kind: &str, pre: bool| -> Result<Option<Latest>> {
        seen_kinds.push((kind.into(), pre));
        Ok(None) // 查無 → 不落事件
    };
    let (_, findings) = upstream::check_with(&d, None, None, &mut fetch, None).unwrap();
    assert_eq!(
        seen_kinds,
        vec![("release".to_string(), true), ("commit".to_string(), true)]
    );
    assert!(findings.is_empty());
}

#[test]
fn test_no_external_repos() {
    let (_t, d) = spine_with_repos();
    let (n, findings) = upstream::check(&d, None, None, None).unwrap(); // g 內全是 mine
    assert_eq!(n, 0);
    assert!(findings.is_empty());
}
