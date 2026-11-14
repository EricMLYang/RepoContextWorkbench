//! F-125 總管：registry changes 的合併與分段；簡報的上限、裁切順序、無事不報；spine 開場；總管會話的 SessionEnd；
//! decide suggest 的留痕不影響連續計數／抽查／已回答、卡片帶出建議、沒有卡時拒絕。

use std::path::{Path, PathBuf};

use chrono::{Duration, Local, NaiveDateTime};
use ctx_core::agentapi::{self as api, Target};
use ctx_core::ask::AskInput;
use ctx_core::decide::{self, AnsweredBy};
use ctx_core::ops::{self, Via};
use ctx_core::registry::{add_group, add_repo, AddRepo};
use ctx_core::registry_log::{self, Area};
use ctx_core::relay::RelayScope;
use ctx_core::steward::{self, GroupLine, BRIEF_MAX_CHARS};
use ctx_core::testkit::{make_git_repo, spine};
use ctx_core::{hooks, spine as sp, Error};
use serde_json::json;

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

fn stamp(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%d %H:%M").to_string()
}

fn register(d: &Path, tmp: &Path, id: &str) -> PathBuf {
    let p = make_git_repo(tmp, id, 6, 0, false);
    add_repo(d, id, &p, AddRepo::default()).unwrap();
    p
}

fn op(d: &Path, name: &str, args: serde_json::Value) {
    ops::call(d, name, args, Via::Cli).unwrap();
}

#[test]
fn registry_changes_are_merged_results() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let old = register(&d, t, "old"); // 登記在 since 之前
    register(&d, t, "z");
    let since = stamp(now() - Duration::minutes(1));
    let p = make_git_repo(t, "fresh", 1, 0, false);
    op(
        &d,
        "registry_add",
        json!({"id": "fresh", "path": p.to_string_lossy()}),
    );
    op(
        &d,
        "registry_describe",
        json!({"id": "fresh", "role": "實作"}),
    );
    let tmpx = make_git_repo(t, "tmpx", 1, 0, false);
    op(
        &d,
        "registry_add",
        json!({"id": "tmpx", "path": tmpx.to_string_lossy()}),
    );
    op(&d, "registry_remove", json!({"id": "tmpx"}));
    op(&d, "registry_remove", json!({"id": "old"}));
    for r in ["一", "二", "三"] {
        op(
            &d,
            "registry_describe",
            json!({"id": "z", "description": format!("第{r}版")}),
        );
    }
    op(&d, "registry_tier", json!({"id": "z", "tier": "paused"}));
    op(&d, "registry_tier", json!({"id": "z", "tier": "dormant"}));
    op(&d, "registry_tag", json!({"id": "z", "add": ["a", "b"]}));
    op(&d, "registry_tag", json!({"id": "z", "remove": ["a"]}));
    op(&d, "group_add", json!({"name": "g", "members": ["z"]}));
    op(&d, "group_goal", json!({"name": "g", "text": "做完 X"}));
    op(
        &d,
        "registry_relate",
        json!({"a": "z", "b": "fresh", "kind": "feeds"}),
    );
    let r = registry_log::changes(&d, &since, now()).unwrap();
    let names = |xs: &[registry_log::RegChange]| -> Vec<(Area, String)> {
        xs.iter().map(|c| (c.area, c.name.clone())).collect()
    };
    assert_eq!(
        names(&r.added),
        [
            (Area::Repo, "fresh".to_string()),
            (Area::Group, "g".to_string()),
            (Area::Relation, "z → fresh（feeds）".to_string()),
        ]
    );
    // 新增的 repo：期間的名片併進細節
    assert!(
        r.added[0].details.iter().any(|x| x.contains("角色＝實作")),
        "{:?}",
        r.added[0]
    );
    assert!(r.added[1]
        .details
        .iter()
        .any(|x| x.contains("目標：做完 X")));
    // 登記又移出的 tmpx 不列；old 列在移除
    assert_eq!(names(&r.removed), [(Area::Repo, "old".to_string())]);
    // z：同一面向只留最後的樣子
    let z = &r.changed[0];
    assert_eq!((z.area, z.name.as_str(), z.events), (Area::Repo, "z", 7));
    assert!(
        z.details.iter().any(|x| x.contains("介紹＝第三版")),
        "{:?}",
        z.details
    );
    assert!(!z.details.iter().any(|x| x.contains("第一版")));
    assert!(
        z.details.contains(&"tier active → dormant".to_string()),
        "{:?}",
        z.details
    );
    assert!(z.details.contains(&"tag +b".to_string()), "{:?}", z.details);
    assert_eq!(r.unrecognized, 0);
    // 現在的異常：路徑不存在
    std::fs::remove_dir_all(&old).ok();
    std::fs::remove_dir_all(t.join("z")).unwrap();
    let r = registry_log::changes(&d, &since, now()).unwrap();
    assert_eq!(
        r.missing_paths
            .iter()
            .map(|m| m.repo.as_str())
            .collect::<Vec<_>>(),
        ["z"]
    );
    // 留痕說移出、現在卻還在（登記回來那一步沒留痕）：列在變更並說明
    let tmpx2 = make_git_repo(t, "tmpy", 1, 0, false);
    op(
        &d,
        "registry_add",
        json!({"id": "tmpy", "path": tmpx2.to_string_lossy()}),
    );
    op(&d, "registry_remove", json!({"id": "tmpy"}));
    add_repo(&d, "tmpy", &tmpx2, AddRepo::default()).unwrap();
    let r = registry_log::changes(&d, &since, now()).unwrap();
    assert!(r.removed.iter().all(|c| c.name != "tmpy"));
    let back = r.added.iter().find(|c| c.name == "tmpy").unwrap();
    assert!(
        back.details.iter().any(|x| x.contains("又回到 registry")),
        "{:?}",
        back.details
    );
    // 移出（期間前就登記的）又無痕登記回來：列在變更
    op(&d, "registry_remove", json!({"id": "fresh"}));
    add_repo(&d, "fresh", &p, AddRepo::default()).unwrap();
    let since0 = stamp(now() - Duration::days(1));
    let r = registry_log::changes(&d, &since0, now()).unwrap();
    assert!(r.added.iter().any(|c| c.name == "fresh"));
}

fn ask_input() -> AskInput {
    AskInput {
        question: "要 commit 嗎？".into(),
        kind: Some("commit".into()),
        options: vec!["要".into(), "不要".into()],
        recommend: Some("1".into()),
        reversible: None,
        evidence: vec![],
    }
}

#[test]
fn brief_is_capped_and_quiet_when_nothing_is_wrong() {
    let (tmp, d) = spine();
    register(&d, tmp.path(), "repo-a");
    let b = steward::brief(&d, now()).unwrap();
    assert!(b.text.starts_with("# 總管簡報"));
    assert!(b.text.contains("## 要你決定：沒有 → ctx decide list --all"));
    assert!(!b.text.contains("## 異常"), "沒有異常就不印：{}", b.text);
    assert!(!b.text.contains("## 自動放行"));
    assert!(!b.text.contains("## 放行規則"));
    assert_eq!(b.cut, 0);
    // 塞爆：各組 200 行＋異常——從最後一段（異常）的最後一項砍起，再砍各組
    let mut big = b.clone();
    big.groups = (0..200)
        .map(|i| GroupLine {
            group: Some(format!("組{i:03}")),
            repos: 1,
            waiting: 0,
            running: 0,
            ready: 0,
            last_handoff: None,
            idle_days: None,
        })
        .collect();
    big.missing_paths = vec![ctx_core::registry_log::MissingPath {
        repo: "gone".into(),
        path: "/nope".into(),
    }];
    let (text, cut) = steward::render_text(&big);
    assert!(
        text.chars().count() <= BRIEF_MAX_CHARS,
        "{}",
        text.chars().count()
    );
    assert!(cut > 0);
    assert!(!text.contains("## 異常"), "最後一段先砍");
    assert!(text.contains("## 各組"));
    assert!(text.contains("組000"), "同一段從後面砍");
    assert!(!text.contains("組199"));
    let last = text.lines().last().unwrap();
    assert!(
        last.starts_with(&format!("還有 {cut} 項：")) && last.contains("ctx relay plan"),
        "{last}"
    );
    // 第一段永遠在
    assert!(text.contains("## 要你決定"));
}

#[test]
fn spine_opening_and_steward_session_end() {
    let (tmp, d) = spine();
    let a = register(&d, tmp.path(), "repo-a");
    // spine 裡開的會話：簡報＋作業規則；有 session_id 才記「看過了」
    let out = hooks::session_start(&d, &json!({"cwd": d.to_string_lossy()}), None);
    let ctx: serde_json::Value = serde_json::from_str(&out).unwrap();
    let text = ctx["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        text.starts_with("# 總管簡報") && text.contains("## 總管的作業規則"),
        "{text}"
    );
    assert!(text.contains("ctx decide suggest"));
    assert!(steward::last_seen(&d).is_none(), "手動試跑不記");
    let out = hooks::session_start(
        &d,
        &json!({"session_id": "st1", "cwd": d.to_string_lossy()}),
        None,
    );
    assert!(!out.is_empty());
    assert!(steward::last_seen(&d).is_some());
    assert!(
        !d.join(".state/hook_sessions/st1.json").exists(),
        "總管會話不記 marker"
    );
    // 總管會話期間別的 repo 有事、總管結束：不記「沒交接就結束」
    api::log_decision(
        &d,
        "別的 repo 的判斷",
        &Target::at(&a),
        None,
        false,
        "agent",
    )
    .unwrap();
    let before = sp::iter_events(&d).unwrap().len();
    assert!(hooks::session_end(&d, &json!({"session_id": "st1"}), None).is_none());
    assert_eq!(sp::iter_events(&d).unwrap().len(), before);
    // 未登記的目錄仍然 0 位元組；一般 repo 的開場不帶總管的東西
    assert_eq!(
        hooks::session_start(&d, &json!({"cwd": tmp.path().to_string_lossy()}), None),
        ""
    );
    let repo_out = hooks::session_start(&d, &json!({"cwd": a.to_string_lossy()}), None);
    assert!(
        repo_out.contains("你在 repo-a") && !repo_out.contains("總管"),
        "{repo_out}"
    );
}

#[test]
fn suggest_is_not_an_answer() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let a = register(&d, t, "repo-a");
    register(&d, t, "quiet");
    add_group(&d, "g", &["repo-a", "quiet"]).unwrap();
    // 沒有卡：拒絕
    match decide::suggest(&d, "quiet", "1", None).unwrap_err() {
        Error::Rejected { code, .. } => assert_eq!(code.as_str(), "not_found"),
        other => panic!("{other:?}"),
    }
    api::handoff_with_ask(
        &d,
        "寫完",
        "等使用者決定",
        &[] as &[&str],
        Some(&ask_input()),
        &Target::at(&a),
        false,
        "agent",
    )
    .unwrap();
    let h0 = decide::history(&d).unwrap().len();
    let s = decide::suggest(&d, "repo-a", "2", Some("測試還沒跑完")).unwrap();
    assert_eq!((s.note.text.as_str(), s.note.option), ("不要", Some(2)));
    let ev = sp::iter_events(&d).unwrap().pop().unwrap();
    assert_eq!(ev.source, "steward");
    assert!(sp::is_decide_record(&ev));
    // 不是回答：卡還在、history 沒多、不算連續與抽查（都只看 history）
    assert_eq!(decide::history(&d).unwrap().len(), h0);
    let q = decide::list(&d, &RelayScope::All).unwrap();
    assert_eq!(q.cards.len(), 1);
    let note = q.cards[0].steward.as_ref().expect("卡上帶總管建議");
    assert_eq!(
        (note.text.as_str(), note.why.as_deref()),
        ("不要", Some("測試還沒跑完"))
    );
    // 不進 agent 的最近事件
    let ctx = api::context_for(&d, &Target::at(&a), true).unwrap();
    assert!(!serde_json::to_string(&ctx["recent_events"])
        .unwrap()
        .contains("總管建議"));
    // 人答得跟總管不同：輸出說一句；一樣就不說
    let r = decide::answer_with(&d, "repo-a", "1", Some(&t.join("cp"))).unwrap();
    assert_eq!(r.steward_differs.as_deref(), Some("不要"));
    assert_eq!(r.answered.by, AnsweredBy::Human);
    // 新的交接：舊建議失效
    api::handoff_with_ask(
        &d,
        "又寫完",
        "等使用者決定",
        &[] as &[&str],
        Some(&ask_input()),
        &Target::at(&a),
        false,
        "agent",
    )
    .unwrap();
    let q = decide::list(&d, &RelayScope::All).unwrap();
    assert!(q.cards[0].steward.is_none());
    decide::suggest(&d, "repo-a", "要", None).unwrap();
    let r = decide::answer_with(&d, "repo-a", "1", Some(&t.join("cp"))).unwrap();
    assert!(r.steward_differs.is_none());
    // 已回答：不能再留
    assert!(decide::suggest(&d, "repo-a", "要", None).is_err());
}
