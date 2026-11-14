//! F-127 App 的決策 action：get_state 的小數字與去重、decide_queue／answer／auto／rules／trust／untrust／overturn、
//! steward_brief；App 進程帶著 CLAUDECODE 也不會被擋（檢查只在 CLI）。

use std::path::{Path, PathBuf};

use chrono::{Duration, Local, NaiveDateTime};
use ctx_core::agentapi::{self as api, Target};
use ctx_core::ask::AskInput;
use ctx_core::registry::{add_group, add_repo, AddRepo};
use ctx_core::spine;
use ctx_core::testkit::{make_git_repo, spine};
use ctx_core::workbench::{build_state, handle_action, is_read_only, StateScope};
use serde_json::{json, Value};

fn register(d: &Path, tmp: &Path, id: &str) -> PathBuf {
    let p = make_git_repo(tmp, id, 6, 0, false);
    add_repo(d, id, &p, AddRepo::default()).unwrap();
    p
}

fn ask() -> AskInput {
    AskInput {
        question: "要 commit 嗎？".into(),
        kind: Some("commit".into()),
        options: vec!["要".into(), "不要".into()],
        recommend: Some("1".into()),
        reversible: None,
        evidence: vec!["README.md".into(), "docs/none.md".into()],
    }
}

fn act(d: &Path, name: &str, p: Value) -> Value {
    handle_action(d, name, &p, None).unwrap_or_else(|e| panic!("{name}：{e}"))
}

fn ago(h: i64) -> NaiveDateTime {
    Local::now().naive_local() - Duration::hours(h)
}

#[test]
fn state_numbers_queue_and_dedupe() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let a = register(&d, t, "repo-a");
    let b = register(&d, t, "repo-b");
    register(&d, t, "repo-c");
    add_group(&d, "g", &["repo-a", "repo-b"]).unwrap();
    // repo-a：先回報等你，再交接帶提問（同一件事）
    api::report_status(&d, "waiting", "要不要 commit", &Target::at(&a)).unwrap();
    api::handoff_with_ask(
        &d,
        "寫完",
        "等使用者決定",
        &[] as &[&str],
        Some(&ask()),
        &Target::at(&a),
        false,
        "agent",
    )
    .unwrap();
    // repo-b：舊式交接，下一步在等人
    spine::append_event(
        &d,
        "decision",
        "agent",
        &["repo:repo-b"],
        "交接：草稿\n完成：草稿\n剩下：無\n下次從這裡接：等使用者決定要不要 push",
        Some(ago(2)),
    )
    .unwrap();
    let _ = b;
    let st = build_state(&d, &StateScope::new(Some("g"), None)).unwrap();
    let dec = &st["decide"];
    assert_eq!(dec["count"], 2, "{dec}");
    assert_eq!(dec["all"], 2);
    assert_eq!(dec["auto_recent"], 0);
    assert!(dec["key"].as_str().unwrap().contains("repo-a="));
    // 同一件事只出現一次：交接那則未讀、之前的「agent 等你回應」都收掉
    let titles: Vec<&str> = st["cards"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["title"].as_str())
        .collect();
    assert!(
        !titles
            .iter()
            .any(|x| x.starts_with("交接：寫完") || x.starts_with("agent 等你回應")),
        "{titles:?}"
    );
    // 清單
    let q = act(&d, "decide_queue", json!({"group": "g"}));
    let cards = q["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[0]["repo"], "repo-a");
    assert_eq!(cards[0]["done"], "寫完");
    let ev = &cards[0]["evidence"];
    assert_eq!(ev[0]["repo"], "repo-a");
    assert_eq!(ev[0]["rel"], "README.md");
    assert_eq!(ev[1]["exists"], false);
    assert!(is_read_only("decide_queue") && !is_read_only("decide_answer"));
    // 範圍是全部時 all＝count
    let st = build_state(&d, &StateScope::default()).unwrap();
    assert_eq!(st["decide"]["count"], st["decide"]["all"]);
}

#[test]
fn answer_trust_overturn_and_brief_from_app() {
    // App 進程可能帶著 CLAUDECODE（例如從 agent 的終端開 App）：引擎不看它
    std::env::set_var("CLAUDECODE", "1");
    let (tmp, d) = spine();
    let t = tmp.path();
    let a = register(&d, t, "repo-a");
    for i in 0..3 {
        spine::append_event(
            &d,
            "decision",
            "agent",
            &[format!("id:2026-01-01-d{i}"), "repo:repo-a".into()],
            &format!(
                "交接：第 {i} 段\n完成：第 {i} 段\n剩下：無\n下次從這裡接：等你\n{}",
                ask().validate().unwrap().body_lines().join("\n")
            ),
            Some(ago(3 - i)),
        )
        .unwrap();
        let r = act(&d, "decide_answer", json!({"repo": "repo-a", "reply": "1"}));
        assert_eq!(r["follow"], "followed");
        assert_eq!(r["relay"]["delivery"]["kind"], "new_session");
        if i == 2 {
            assert_eq!(r["suggestion"]["streak"], 3, "{r}");
        }
    }
    let r = act(
        &d,
        "decide_trust",
        json!({"repo": "repo-a", "kind": "commit"}),
    );
    assert_eq!(r["rule"]["level"], "auto");
    let dec = ctx_core::decide::request(&d, &Target::at(&a), &ask(), false).unwrap();
    let reference = serde_json::to_value(&dec).unwrap()["ref"]
        .as_str()
        .unwrap()
        .to_string();
    let l = act(&d, "decide_auto", json!({}));
    assert_eq!(l["items"].as_array().unwrap().len(), 1);
    let st = build_state(&d, &StateScope::default()).unwrap();
    assert_eq!(st["decide"]["auto_recent"], 1);
    let rules = act(&d, "decide_rules", json!({}));
    assert_eq!(rules["rules"][0]["approvals"], 1);
    let o = act(
        &d,
        "decide_overturn",
        json!({"ref": reference, "why": "不該自動"}),
    );
    assert_eq!(o["rule"]["level"], "ask");
    assert_eq!(o["what"], "overturn");
    // 你最近的回答（新的在前）→ 撤回最後一筆：卡回到佇列、規則不動
    assert!(is_read_only("decide_mine"));
    let mine = act(&d, "decide_mine", json!({}));
    assert_eq!(mine["items"].as_array().unwrap().len(), 3);
    let last = mine["items"][0]["ref"].as_str().unwrap().to_string();
    let w = act(&d, "decide_overturn", json!({"ref": last}));
    assert_eq!(w["what"], "withdraw");
    assert!(w["rule"].is_null());
    let q = act(&d, "decide_queue", json!({}));
    assert_eq!(q["cards"].as_array().unwrap().len(), 1);
    assert_eq!(q["cards"][0]["handoff_ref"], "decision:2026-01-01-d2");
    let mine = act(&d, "decide_mine", json!({}));
    assert!(mine["items"][0]["command"].is_null());
    assert!(mine["items"][0]["overturned_by"].is_string());
    let u = act(
        &d,
        "decide_untrust",
        json!({"repo": "repo-a", "kind": "choose"}),
    );
    assert_eq!(u["rule"]["level"], "ask");
    // 推翻兩次＝錯誤（App 顯示訊息）
    assert!(handle_action(&d, "decide_overturn", &json!({"ref": reference}), None).is_err());
    let b = act(&d, "steward_brief", json!({}));
    assert!(b["text"].as_str().unwrap().starts_with("# 總管簡報"));
    std::env::remove_var("CLAUDECODE");
}
