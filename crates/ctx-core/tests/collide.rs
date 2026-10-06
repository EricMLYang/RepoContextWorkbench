//! 對應 `tests/test_pack_route_collide.py` 裡 **collide** 相關的測試（pack 在 tests/pack.rs、route 在 tests/route.rs）。
//!
//! Python 用 `REPOENGINE_MOCK_FAIL` 環境變數＋`MockProvider._calls` 類別變數控制 mock 失敗次數；
//! Rust 測試平行跑、不碰全域環境變數，改用 `run_judgement_with` 注入 `MockProvider::new(n)`。

use std::path::Path;

use ctx_core::agents::{MockProvider, Verdict};
use ctx_core::collide;
use ctx_core::spine::{self, Event, EventType, Query};
use ctx_core::testkit::spine_with_repos;

fn collisions(d: &Path) -> Vec<Event> {
    spine::query(d, &Query::of_type(EventType::Collision)).unwrap()
}

fn submit_g1(d: &Path, idea: &str) -> String {
    collide::submit(d, idea, Some("g1"), None, "hotkey", None).unwrap()
}

#[test]
fn test_collide_two_stage_mock() {
    let (_t, d) = spine_with_repos();
    // 段一：想法即刻落脊椎
    let cid = submit_g1(&d, "把快取改成 LFU 如何？（已知 的變形）");
    let opened: Vec<Event> = collisions(&d)
        .into_iter()
        .filter(|e| e.body.starts_with("opened"))
        .collect();
    assert_eq!(opened.len(), 1);
    assert!(opened[0].body.contains("LFU"));
    assert_eq!(opened[0].kv("id"), Some(cid.as_str()));
    // 段二：mock 判定落脊椎，五欄位齊
    let j = collide::run_judgement(&d, &cid, None, None, Some("mock"), None)
        .unwrap()
        .unwrap();
    assert!(matches!(
        j.verdict,
        Verdict::Known | Verdict::Conflict | Verdict::Novel
    ));
    let r = format!("collision:{cid}");
    let results: Vec<Event> = collisions(&d)
        .into_iter()
        .filter(|e| e.kv("ref") == Some(r.as_str()))
        .collect();
    assert_eq!(results.len(), 1);
    for field in ["判定：", "理由：", "證據：", "落點建議：", "下一步："] {
        assert!(results[0].body.contains(field));
    }
    // 判定結果是未讀（回程通知的資料來源）
    assert!(spine::get_unread(&d)
        .unwrap()
        .iter()
        .any(|e| e.kv("ref") == Some(r.as_str())));
}

/// 內層驗證失敗重試一次：第 1 次壞、第 2 次好 → 成功。
#[test]
fn test_collide_inner_validation_retry() {
    let (_t, d) = spine_with_repos();
    let cid = submit_g1(&d, "重試路徑測試");
    let mock = MockProvider::new(1);
    let j = collide::run_judgement_with(&d, &cid, None, None, &mock, None).unwrap();
    assert!(j.is_some());
    assert_eq!(mock.calls(), 2);
}

/// 兩次都壞 → 不准沉默：落 system-unsure 事件、回傳 None。
#[test]
fn test_collide_system_unsure_surfaces() {
    let (_t, d) = spine_with_repos();
    let cid = submit_g1(&d, "必敗測試");
    let j = collide::run_judgement_with(&d, &cid, None, None, &MockProvider::new(5), None).unwrap();
    assert!(j.is_none());
    let fails: Vec<Event> = collisions(&d)
        .into_iter()
        .filter(|e| e.body.contains("system-unsure"))
        .collect();
    assert_eq!(fails.len(), 1);
}

#[test]
fn test_collision_id_sequence() {
    let (_t, d) = spine_with_repos();
    let a = submit_g1(&d, "想法一");
    let b = submit_g1(&d, "想法二");
    assert!(a.ends_with("-a") && b.ends_with("-b"));
}

#[test]
fn test_agent_raw_log_written() {
    let (_t, d) = spine_with_repos();
    let cid = submit_g1(&d, "raw log 測試");
    collide::run_judgement_with(&d, &cid, None, None, &MockProvider::new(0), None).unwrap();
    let logs = std::fs::read_dir(d.join(".agent_logs"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .count();
    assert!(logs > 0, "每次 agent 呼叫 raw request/response 必落檔");
}

/// 回歸：prompt 模板含三個判定詞，mock 只准看【想法】段。
#[test]
fn test_mock_verdict_follows_idea_not_template() {
    let (_t, d) = spine_with_repos();
    let cid = submit_g1(&d, "一個完全嶄新的方向");
    let j = collide::run_judgement_with(&d, &cid, None, None, &MockProvider::new(0), None)
        .unwrap()
        .unwrap();
    assert_eq!(j.verdict, Verdict::Novel);
    let cid2 = submit_g1(&d, "這與既有結論衝突吧");
    let j2 = collide::run_judgement_with(&d, &cid2, None, None, &MockProvider::new(0), None)
        .unwrap()
        .unwrap();
    assert_eq!(j2.verdict, Verdict::Conflict);
}

/// P2 臨時組合免建組：submit(repos=...) 落 group:臨時(...) token，
/// run_judgement 不帶參數也能從事件解回選料範圍（detached 進程同路徑）。
#[test]
fn test_collide_adhoc_repos_token_roundtrip() {
    let (_t, d) = spine_with_repos();
    let ids = vec!["repo-a".to_string()];
    let cid = collide::submit(&d, "臨時組合的想法", None, Some(&ids), "hotkey", None).unwrap();
    let opened = collisions(&d)
        .into_iter()
        .find(|e| e.kv("id") == Some(cid.as_str()))
        .unwrap();
    assert_eq!(opened.kv("group"), Some("臨時(repo-a)"));
    let j = collide::run_judgement_with(&d, &cid, None, None, &MockProvider::new(0), None).unwrap(); // 不給 group/repos
    assert!(j.is_some());
}

/// detached 進程的參數＝`ctx --spine <d> collide run <cid> [--group] [--provider]`（同 Python 子命令）；
/// 設了 REPOCTX_NO_BACKGROUND（testkit::spine 會設）就不起進程。Python 沒有對應測試。
#[test]
fn detached_args_and_no_background() {
    let (_t, d) = spine_with_repos();
    let args: Vec<String> = collide::detached_args(&d, "2026-10-06-a", Some("g1"), Some("mock"))
        .into_iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        args,
        vec![
            "--spine".to_string(),
            d.to_string_lossy().into_owned(),
            "collide".into(),
            "run".into(),
            "2026-10-06-a".into(),
            "--group".into(),
            "g1".into(),
            "--provider".into(),
            "mock".into(),
        ]
    );
    assert!(!collide::spawn_detached(&d, "2026-10-06-a", None, None).unwrap());
}

/// 找不到 opened 事件＝Invalid（Python ValueError）。
#[test]
fn run_judgement_unknown_cid_is_invalid() {
    let (_t, d) = spine_with_repos();
    assert!(matches!(
        collide::run_judgement(&d, "1999-01-01-a", None, None, None, None),
        Err(ctx_core::Error::Invalid(_))
    ));
}
