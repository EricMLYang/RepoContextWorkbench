//! P5 digest：mock provider——首輪產摘要落事件、二輪水位線擋住（一律增量）、失敗浮出。
//! 對應 tests/test_digest.py。

use std::path::Path;

use ctx_core::agents::AgentProvider;
use ctx_core::digest;
use ctx_core::spine::{self, EventType};
use ctx_core::testkit::spine_with_repos;
use ctx_core::{Error, Result};

fn digest_events(d: &Path) -> Vec<spine::Event> {
    spine::iter_events(d)
        .unwrap()
        .into_iter()
        .filter(|e| e.source == "digest")
        .collect()
}

#[test]
fn test_digest_then_incremental_watermark() {
    let (_t, d) = spine_with_repos();
    let res = digest::run(&d, Some("g1"), None, None, None).unwrap(); // config 預設 provider=mock
    let ok: Vec<_> = res.iter().filter(|r| r.note == "ok").collect();
    let ok_ids: Vec<&str> = ok.iter().map(|r| r.repo.as_str()).collect();
    assert!(ok_ids.contains(&"repo-a") && ok_ids.contains(&"repo-b")); // 兩個 repo 的 commit 都在 7 天內
    for r in &ok {
        assert!(r.file.as_deref().is_some_and(|f| f.ends_with(".md")));
    }
    let evs = digest_events(&d);
    assert_eq!(evs.len(), 2);
    assert!(evs.iter().all(|e| e.etype == EventType::Suggestion));
    // 二輪：水位線＝剛剛的事件時間，無新 commit → 不寫事件（無事不報）
    let res2 = digest::run(&d, Some("g1"), None, None, None).unwrap();
    assert!(res2.iter().all(|r| r.note == "無新 commit"));
    assert_eq!(digest_events(&d).len(), 2);
}

/// Python 用 monkeypatch 把 digest.run_text 換成丟 AgentUnsure；Rust 注入一個永遠沒把握的 provider。
struct Boom;

impl AgentProvider for Boom {
    fn name(&self) -> &'static str {
        "boom"
    }
    fn run_judgement(
        &self,
        _prompt: &str,
        _spine_dir: &Path,
        _model: Option<&str>,
    ) -> Result<serde_json::Value> {
        Err(Error::External("模擬 agent 掛掉".into()))
    }
    fn run_text(
        &self,
        _prompt: &str,
        _spine_dir: &Path,
        _model: Option<&str>,
        _cwd: Option<&Path>,
        _read_only: bool,
    ) -> Result<String> {
        Err(Error::External("模擬 agent 掛掉".into()))
    }
}

#[test]
fn test_digest_failure_floats_system_unsure() {
    let (_t, d) = spine_with_repos();
    let res = digest::run_with(&d, Some("g1"), None, &Boom, None).unwrap();
    assert!(res.iter().all(|r| r.note == "失敗（已浮出）"));
    let evs = digest_events(&d);
    assert!(!evs.is_empty() && evs.iter().all(|e| e.body.contains("system-unsure")));
}
