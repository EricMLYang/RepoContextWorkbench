//! F-123 提問欄位與決策佇列：ask 寫入與讀回、沒 ask 時 body 不變、分隔字元與建議的拒絕、舊交接解析；
//! decide list 的排序與已回答判定；decide answer 的編號轉換、超出範圍、busy 不留痕；
//! 回答留痕不進 retro／summary／ruminate／context_for／開場。

use std::path::{Path, PathBuf};

use chrono::{Duration, Local, NaiveDateTime};
use ctx_core::agentapi::{self as api, Target};
use ctx_core::ask::{Ask, AskInput, AskKind};
use ctx_core::decide::{self, FollowRec};
use ctx_core::registry::{add_group, add_repo, set_field, AddRepo};
use ctx_core::relay::{self, RelayScope, RelayStatus};
use ctx_core::session_status::{path_for, record_hook, StatusHook};
use ctx_core::spine::{self, EventType};
use ctx_core::testkit::{make_git_repo, spine};
use ctx_core::{hooks, ruminate, summary, Error};
use serde_json::json;

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

fn ago(days: i64, hours: i64) -> NaiveDateTime {
    now() - Duration::days(days) - Duration::hours(hours)
}

fn register(d: &Path, tmp: &Path, id: &str) -> PathBuf {
    let p = make_git_repo(tmp, id, 6, 0, false);
    add_repo(d, id, &p, AddRepo::default()).unwrap();
    p
}

/// 照 `agentapi::handoff_with_ask` 的格式在指定時間寫一筆交接（帶 id，ref＝decision:<id>）。
fn handoff_at(
    d: &Path,
    repo: &str,
    when: NaiveDateTime,
    id: &str,
    next: &str,
    ask: Option<&Ask>,
) -> String {
    let mut lines = vec![
        "交接：做完一段".to_string(),
        "完成：做完一段".to_string(),
        "剩下：無".to_string(),
        format!("下次從這裡接：{next}"),
    ];
    if let Some(a) = ask {
        lines.extend(a.body_lines());
    }
    spine::append_event(
        d,
        "decision",
        "agent",
        &[format!("id:{id}"), format!("repo:{repo}")],
        &lines.join("\n"),
        Some(when),
    )
    .unwrap();
    format!("decision:{id}")
}

fn ask3() -> AskInput {
    AskInput {
        question: "快取要用 LRU 還是 LFU？".into(),
        kind: Some("choose".into()),
        options: vec!["LRU".into(), "LFU".into(), "先不做快取".into()],
        recommend: Some("2".into()),
        reversible: None,
        evidence: vec!["README.md".into(), "docs/missing.md".into()],
    }
}

fn code(e: Error) -> (&'static str, String, String) {
    match e {
        Error::Rejected {
            code,
            message,
            hint,
        } => (code.as_str(), message, hint.unwrap_or_default()),
        other => panic!("應該是 Rejected：{other:?}"),
    }
}

fn event_count(d: &Path) -> usize {
    spine::iter_events(d).unwrap().len()
}

fn session(d: &Path, sid: &str, cwd: &Path, ev: StatusHook) {
    let st = path_for(d, sid);
    let payload =
        json!({"cwd": cwd.to_string_lossy(), "session_id": "11111111-2222-3333-4444-555555555555"});
    assert!(record_hook(ev, &payload.to_string(), st.to_str(), None));
}

// ---------------------------------------------------------------- A. 提問欄位

#[test]
fn handoff_ask_written_and_read_back() {
    let (tmp, d) = spine();
    let a = register(&d, tmp.path(), "repo-a");
    let h = api::handoff_with_ask(
        &d,
        "寫完快取層",
        "等使用者選快取策略",
        &["壓測"],
        Some(&ask3()),
        &Target::at(&a),
        false,
        "agent",
    )
    .unwrap();
    let v = serde_json::to_value(&h).unwrap();
    assert_eq!(v["ask"]["recommend"], "LFU", "建議的編號換成選項文字");
    assert_eq!(v["ask"]["kind"], "choose");
    let ev = spine::iter_events(&d).unwrap().pop().unwrap();
    assert_eq!(
        ev.body,
        "交接：寫完快取層\n完成：寫完快取層\n剩下：壓測\n下次從這裡接：等使用者選快取策略\n\
         要你決定：快取要用 LRU 還是 LFU？\n類別：choose\n選項：LRU｜LFU｜先不做快取\n建議：LFU\n\
         可逆：是\n證據：README.md；docs/missing.md"
    );
    let p = api::parse_handoff(&ev);
    let ask = p.ask.expect("讀得回提問");
    assert_eq!(ask.question, "快取要用 LRU 還是 LFU？");
    assert_eq!(ask.kind, AskKind::Choose);
    assert_eq!(ask.options, ["LRU", "LFU", "先不做快取"]);
    assert_eq!(ask.recommend_index(), Some(2));
    assert!(ask.reversible);
    assert_eq!(ask.evidence, ["README.md", "docs/missing.md"]);
    // 原本的欄位照舊
    assert_eq!(p.next_step.as_deref(), Some("等使用者選快取策略"));
    assert_eq!(p.remaining.as_deref(), Some("壓測"));
}

#[test]
fn handoff_without_ask_is_byte_identical() {
    let (tmp, d) = spine();
    let a = register(&d, tmp.path(), "repo-a");
    let t = Target::at(&a);
    let old = api::handoff(
        &d,
        "寫完\n第二行",
        "補測試",
        &["x", "y"],
        &t,
        false,
        "agent",
    )
    .unwrap();
    let new = api::handoff_with_ask(
        &d,
        "寫完\n第二行",
        "補測試",
        &["x", "y"],
        None,
        &t,
        false,
        "agent",
    )
    .unwrap();
    let evs = spine::iter_events(&d).unwrap();
    let bodies: Vec<&str> = evs
        .iter()
        .filter(|e| api::is_handoff(e))
        .map(|e| e.body.as_str())
        .collect();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0], bodies[1]);
    assert_eq!(
        bodies[0],
        "交接：寫完\n完成：寫完 第二行\n剩下：x；y\n下次從這裡接：補測試"
    );
    // 沒帶 ask：JSON 不多出 ask 鍵（CLI／MCP 的回傳跟以前一樣）
    for h in [old, new] {
        let v = serde_json::to_value(&h).unwrap();
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["ok", "ref", "todos", "event"]);
    }
    assert!(api::parse_handoff(&evs[evs.len() - 1]).ask.is_none());
    let pj = serde_json::to_value(api::parse_handoff(&evs[evs.len() - 1])).unwrap();
    assert!(pj.get("ask").is_none());
}

#[test]
fn handoff_rejects_bad_ask_and_writes_nothing() {
    let (tmp, d) = spine();
    let a = register(&d, tmp.path(), "repo-a");
    let t = Target::at(&a);
    let before = event_count(&d);
    let try_ask = |ask: AskInput| {
        code(
            api::handoff_with_ask(
                &d,
                "做完",
                "等你",
                &[] as &[&str],
                Some(&ask),
                &t,
                false,
                "agent",
            )
            .unwrap_err(),
        )
    };
    let (c, m, h) = try_ask(AskInput {
        options: vec!["A｜B".into(), "C".into()],
        ..ask3()
    });
    assert_eq!(c, "bad_request");
    assert!(m.contains("｜") && !h.is_empty(), "{m} / {h}");
    let (c, m, h) = try_ask(AskInput {
        recommend: Some("D".into()),
        ..ask3()
    });
    assert_eq!(c, "bad_request");
    assert!(m.contains("D") && h.contains("1) LRU") && h.contains("3) 先不做快取"));
    let (c, _, _) = try_ask(AskInput {
        recommend: Some("4".into()),
        ..ask3()
    });
    assert_eq!(c, "bad_request");
    let (c, m, _) = try_ask(AskInput {
        evidence: vec!["a.md；b.md".into()],
        ..ask3()
    });
    assert_eq!(c, "bad_request");
    assert!(m.contains("；"));
    let (c, _, h) = try_ask(AskInput {
        kind: Some("approve".into()),
        ..ask3()
    });
    assert_eq!(c, "bad_request");
    assert!(h.contains("send_external"));
    assert_eq!(event_count(&d), before, "驗不過就什麼都不寫（也不改現況）");
}

#[test]
fn old_and_hand_edited_handoffs_parse() {
    let (tmp, d) = spine();
    register(&d, tmp.path(), "repo-a");
    // 舊交接（沒有提問欄位）
    spine::append_event(
        &d,
        "decision",
        "agent",
        &["repo:repo-a"],
        "交接：舊的\n完成：舊的\n剩下：無\n下次從這裡接：補文件",
        None,
    )
    .unwrap();
    // 人手改過：只剩問題、類別寫錯、選項有空格、建議不在選項裡、多了不認得的行
    spine::append_event(
        &d,
        "decision",
        "manual",
        &["repo:repo-a"],
        "交接：手改\n下次從這裡接：等你\n要你決定： 要不要上線？ \n類別：上線\n選項：｜要 ｜ 不要｜\n建議：再等等\n亂寫一行\n可逆",
        None,
    )
    .unwrap();
    // 只有第一行
    spine::append_event(
        &d,
        "decision",
        "agent",
        &["repo:repo-a"],
        "交接：只有標題",
        None,
    )
    .unwrap();
    let evs = spine::iter_events(&d).unwrap();
    let hs: Vec<_> = evs.iter().map(api::parse_handoff).collect();
    assert!(hs[0].ask.is_none());
    assert_eq!(hs[0].next_step.as_deref(), Some("補文件"));
    let a = hs[1].ask.as_ref().unwrap();
    assert_eq!(a.question, "要不要上線？");
    assert_eq!(a.kind, AskKind::Other);
    assert_eq!(a.options, ["要", "不要"]);
    assert_eq!(a.recommend.as_deref(), Some("再等等"));
    assert_eq!(a.recommend_index(), None);
    assert!(!a.reversible, "缺「可逆：」當不可逆");
    assert!(hs[2].ask.is_none() && hs[2].next_step.is_none());
}

// ---------------------------------------------------------------- B. 佇列

#[test]
fn list_orders_and_filters() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let ids = [
        "ask-old", "ask-new", "text-new", "ready", "gone", "busy", "covered",
    ];
    for id in ids {
        register(&d, t, id);
    }
    add_group(&d, "g", &ids).unwrap();
    set_field(&d, "ask-old", "description", "舊的提問 repo").unwrap();
    set_field(&d, "ask-old", "role", "後端").unwrap();
    let ask = ask3().validate().unwrap();
    // 下一步的文字看不出在等人，但帶了提問 → 照樣是卡
    handoff_at(
        &d,
        "ask-old",
        ago(3, 0),
        "2026-01-01-d1",
        "繼續做 B",
        Some(&ask),
    );
    handoff_at(
        &d,
        "ask-new",
        ago(1, 0),
        "2026-01-01-d2",
        "繼續做 C",
        Some(&ask),
    );
    handoff_at(
        &d,
        "text-new",
        ago(0, 2),
        "2026-01-01-d3",
        "等使用者挑一個方案",
        None,
    );
    handoff_at(&d, "ready", ago(0, 2), "2026-01-01-d4", "接著寫 D", None);
    handoff_at(&d, "gone", ago(1, 0), "2026-01-01-d5", "等你", Some(&ask));
    std::fs::remove_dir_all(t.join("gone")).unwrap();
    handoff_at(&d, "busy", ago(1, 0), "2026-01-01-d6", "等你", Some(&ask));
    session(&d, "t1", &t.join("busy"), StatusHook::ToolStart);
    // 新的交接蓋掉舊的提問
    handoff_at(
        &d,
        "covered",
        ago(2, 0),
        "2026-01-01-d7",
        "等你",
        Some(&ask),
    );
    handoff_at(&d, "covered", ago(0, 1), "2026-01-01-d8", "接著寫 E", None);

    let before = event_count(&d);
    let q = decide::list(&d, &RelayScope::Group("g".into())).unwrap();
    assert_eq!(event_count(&d), before, "list 純讀取");
    let order: Vec<&str> = q.cards.iter().map(|c| c.repo.as_str()).collect();
    assert_eq!(order, ["ask-new", "ask-old", "text-new"]);
    assert_eq!(q.structured, 2);
    let c = &q.cards[1];
    assert!(c.structured);
    assert_eq!(c.about.as_deref(), Some("舊的提問 repo"));
    assert_eq!(c.role.as_deref(), Some("後端"));
    assert_eq!(c.handoff_ref, "decision:2026-01-01-d1");
    assert_eq!(c.days, 3);
    assert_eq!(c.kind, Some(AskKind::Choose));
    assert_eq!(c.reversible, Some(true));
    let opts: Vec<(usize, &str, bool)> = c
        .options
        .iter()
        .map(|o| (o.n, o.text.as_str(), o.recommended))
        .collect();
    assert_eq!(
        opts,
        [
            (1, "LRU", false),
            (2, "LFU", true),
            (3, "先不做快取", false)
        ]
    );
    let ev: Vec<(&str, bool)> = c
        .evidence
        .iter()
        .map(|e| (e.reference.as_str(), e.exists))
        .collect();
    assert_eq!(ev, [("README.md", true), ("docs/missing.md", false)]);
    let plain = &q.cards[2];
    assert!(!plain.structured);
    assert_eq!(plain.question, "等使用者挑一個方案");
    assert!(plain.kind.is_none() && plain.options.is_empty() && plain.reversible.is_none());
    assert!(
        !plain.notes.iter().any(|n| n == relay::NOTE_WAITS),
        "卡片不重印「下一步在等你」"
    );
    // JSON：structured 與數字欄位
    let v = serde_json::to_value(&q).unwrap();
    assert_eq!(v["cards"][0]["structured"], true);
    assert_eq!(v["cards"][0]["options"][1]["recommended"], true);
    assert_eq!(v["cards"][2]["kind"], serde_json::Value::Null);
}

// ---------------------------------------------------------------- B／C. 回答

#[test]
fn answer_by_number_records_and_clears_card() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let root = t.join("claude-projects");
    register(&d, t, "repo-a");
    register(&d, t, "repo-b");
    add_group(&d, "g", &["repo-a", "repo-b"]).unwrap();
    let ask = ask3().validate().unwrap();
    let rf = handoff_at(&d, "repo-a", ago(1, 0), "2026-01-01-d1", "繼續", Some(&ask));
    handoff_at(
        &d,
        "repo-b",
        ago(1, 0),
        "2026-01-01-d2",
        "等你決定要不要 commit",
        None,
    );

    // 超出範圍：拒絕、列出選項、不留痕
    let before = event_count(&d);
    let (c, m, h) = code(decide::answer_with(&d, "repo-a", "4", Some(&root)).unwrap_err());
    assert_eq!(c, "bad_request");
    assert!(m.contains("4"));
    assert!(h.contains("1) LRU") && h.contains("2) LFU") && h.contains("3) 先不做快取"));
    let (c, _, _) = code(decide::answer_with(&d, "repo-a", "0", Some(&root)).unwrap_err());
    assert_eq!(c, "bad_request");
    assert_eq!(event_count(&d), before);

    // 編號 → 選項文字；照建議
    let r = decide::answer_with(&d, "repo-a", "2", Some(&root)).unwrap();
    assert_eq!(r.answered.answer, "LFU");
    assert_eq!(r.option, Some(2));
    assert_eq!(r.answered.follow, FollowRec::Followed);
    assert_eq!(r.answered.kind, Some(AskKind::Choose));
    assert_eq!(r.answered.handoff_ref.as_deref(), Some(rf.as_str()));
    assert_eq!(r.relay.handoff_ref, rf);
    assert!(
        r.relay
            .task
            .contains("使用者對「快取要用 LRU 還是 LFU？」的回答是：LFU"),
        "{}",
        r.relay.task
    );
    assert_eq!(event_count(&d), before + 1);
    let ev = spine::iter_events(&d).unwrap().pop().unwrap();
    assert_eq!(ev.etype, EventType::Chosen);
    assert_eq!(ev.source, "cli");
    assert!(ev.body.starts_with(spine::DECIDE_TAG));
    assert_eq!(ev.kv("ref"), Some(rf.as_str()));
    assert_eq!(ev.kv("repo"), Some("repo-a"));
    assert!(ev
        .body
        .lines()
        .any(|l| l == "decide kind:choose follow:followed by:human rev:yes"));
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["ref"], r.answered.reference);
    assert_eq!(v["follow"], "followed");
    assert_eq!(v["relay"]["delivery"]["kind"], "new_session");

    // 卡消失；再答一次＝沒有卡
    let q = decide::list(&d, &RelayScope::Group("g".into())).unwrap();
    assert_eq!(
        q.cards.iter().map(|c| c.repo.as_str()).collect::<Vec<_>>(),
        ["repo-b"]
    );
    let (c, _, _) = code(decide::answer_with(&d, "repo-a", "1", Some(&root)).unwrap_err());
    assert_eq!(c, "not_found");

    // 沒提問的卡：數字也是自由文字；沒有建議
    let b = decide::answer_with(&d, "repo-b", "  先 commit\n再說 ", Some(&root)).unwrap();
    assert_eq!(b.answered.answer, "先 commit 再說");
    assert_eq!(b.answered.follow, FollowRec::NoRecommendation);
    assert_eq!(b.answered.kind, None);
    assert!(b.relay.task.contains("使用者的回覆：先 commit 再說"));

    // 同一個 repo 又交接一筆新的提問 → 新卡；自由文字跟建議不同
    handoff_at(&d, "repo-a", ago(0, 0), "2026-01-01-d9", "繼續", Some(&ask));
    let r2 = decide::answer_with(&d, "repo-a", "改用 ARC", Some(&root)).unwrap();
    assert_eq!(r2.option, None);
    assert_eq!(r2.answered.follow, FollowRec::Differed);
    // 回覆就是選項文字也算那個選項
    handoff_at(
        &d,
        "repo-a",
        ago(0, 0),
        "2026-01-01-d10",
        "繼續",
        Some(&ask),
    );
    let r3 = decide::answer_with(&d, "repo-a", "LRU", Some(&root)).unwrap();
    assert_eq!(r3.option, Some(1));
    assert_eq!(r3.answered.follow, FollowRec::Differed);

    // history：舊到新、不用解析自由文字
    let hist = decide::history(&d).unwrap();
    let got: Vec<(&str, Option<AskKind>, FollowRec)> = hist
        .iter()
        .map(|a| (a.repo.as_str(), a.kind, a.follow))
        .collect();
    assert_eq!(
        got,
        [
            ("repo-a", Some(AskKind::Choose), FollowRec::Followed),
            ("repo-b", None, FollowRec::NoRecommendation),
            ("repo-a", Some(AskKind::Choose), FollowRec::Differed),
            ("repo-a", Some(AskKind::Choose), FollowRec::Differed),
        ]
    );
    // 每筆的 id 都不一樣
    let mut refs: Vec<&str> = hist.iter().map(|a| a.reference.as_str()).collect();
    refs.dedup();
    assert_eq!(refs.len(), 4);
}

#[test]
fn answer_blocked_writes_nothing() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let root = t.join("claude-projects");
    let busy = register(&d, t, "busy");
    register(&d, t, "gone");
    register(&d, t, "ready");
    let ask = ask3().validate().unwrap();
    handoff_at(&d, "busy", ago(1, 0), "2026-01-01-d1", "等你", Some(&ask));
    session(&d, "t1", &busy, StatusHook::ToolStart);
    handoff_at(&d, "gone", ago(1, 0), "2026-01-01-d2", "等你", Some(&ask));
    std::fs::remove_dir_all(t.join("gone")).unwrap();
    handoff_at(&d, "ready", ago(1, 0), "2026-01-01-d3", "接著寫 D", None);

    let before = event_count(&d);
    let (c, m, _) = code(decide::answer_with(&d, "busy", "2", Some(&root)).unwrap_err());
    assert_eq!(c, "busy", "{m}");
    let (c, m, _) = code(decide::answer_with(&d, "gone", "2", Some(&root)).unwrap_err());
    assert_eq!(c, "not_found");
    assert!(m.contains("路徑不存在"), "{m}");
    let (c, m, _) = code(decide::answer_with(&d, "ready", "好", Some(&root)).unwrap_err());
    assert_eq!(c, "not_found");
    assert!(m.contains("沒有在等你決定"), "{m}");
    let (c, _, _) = code(decide::answer_with(&d, "nope", "好", Some(&root)).unwrap_err());
    assert_eq!(c, "not_found");
    let (c, _, _) = code(decide::answer_with(&d, "ready", "  ", Some(&root)).unwrap_err());
    assert_eq!(c, "bad_request");
    assert_eq!(event_count(&d), before, "送不出去就不留痕");
    assert!(decide::history(&d).unwrap().is_empty());
}

#[test]
fn answered_requires_matching_ref_and_time() {
    let (tmp, d) = spine();
    let t = tmp.path();
    register(&d, t, "repo-a");
    // 沒有 id 的交接 ref 只有時間：前一天同一分鐘的回答不算答到今天這筆
    let when = ago(0, 1);
    let hhmm = when.format("%H:%M").to_string();
    spine::append_event(
        &d,
        "chosen",
        "cli",
        &["repo:repo-a".to_string(), format!("ref:decision:{hhmm}")],
        &format!(
            "{}repo-a：舊問題 → 好\ndecide kind:none follow:no_recommendation",
            spine::DECIDE_TAG
        ),
        Some(when - Duration::days(1)),
    )
    .unwrap();
    spine::append_event(
        &d,
        "decision",
        "agent",
        &["repo:repo-a"],
        "交接：x\n下次從這裡接：等你決定要不要 push",
        Some(when),
    )
    .unwrap();
    let q = decide::list(&d, &RelayScope::All).unwrap();
    assert_eq!(q.cards.len(), 1);
    assert_eq!(q.cards[0].handoff_ref, format!("decision:{hhmm}"));
}

// ---------------------------------------------------------------- D. 接力

#[test]
fn relay_plan_treats_ask_as_needs_you() {
    let (tmp, d) = spine();
    let t = tmp.path();
    register(&d, t, "asky");
    register(&d, t, "plain");
    add_group(&d, "g", &["asky", "plain"]).unwrap();
    let ask = ask3().validate().unwrap();
    handoff_at(
        &d,
        "asky",
        ago(1, 0),
        "2026-01-01-d1",
        "接著寫 B",
        Some(&ask),
    );
    handoff_at(&d, "plain", ago(1, 0), "2026-01-01-d2", "接著寫 B", None);
    let p = relay::plan(&d, &RelayScope::Group("g".into())).unwrap();
    let row = |id: &str| p.rows.iter().find(|r| r.repo == id).unwrap();
    assert_eq!(row("asky").status, RelayStatus::NeedsYou);
    assert!(row("asky").waits_on_you);
    assert!(row("asky").prompt.is_none());
    assert_eq!(
        row("asky").ask.as_ref().map(|a| a.question.as_str()),
        Some("快取要用 LRU 還是 LFU？")
    );
    assert_eq!(row("plain").status, RelayStatus::Ready);
    let v = serde_json::to_value(row("plain")).unwrap();
    assert!(v.get("ask").is_none(), "沒提問的列 JSON 不多鍵");
    let v = serde_json::to_value(row("asky")).unwrap();
    assert_eq!(v["ask"]["options"][2], "先不做快取");
}

// ---------------------------------------------------------------- C. 留痕不進別的地方

#[test]
fn decide_records_stay_out_of_agent_context_and_reviews() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let root = t.join("claude-projects");
    let a = register(&d, t, "repo-a");
    register(&d, t, "repo-b");
    add_group(&d, "g", &["repo-a", "repo-b"]).unwrap();
    api::handoff_with_ask(
        &d,
        "寫完快取層",
        "等使用者選快取策略",
        &[] as &[&str],
        Some(&ask3()),
        &Target::at(&a),
        false,
        "agent",
    )
    .unwrap();
    let r = decide::answer_with(&d, "repo-a", "2", Some(&root)).unwrap();
    let marker = spine::DECIDE_TAG;

    // context_for 的最近事件與上次進度
    let ctx = api::context_for(&d, &Target::at(&a), true).unwrap();
    let recent = serde_json::to_string(&ctx["recent_events"]).unwrap();
    assert!(!recent.contains(marker), "{recent}");
    assert!(!ctx["card"].as_str().unwrap().contains(marker));
    assert!(!serde_json::to_string(&ctx["last_progress"])
        .unwrap()
        .contains(marker));
    assert_eq!(ctx["last_handoff"]["next_step"], "等使用者選快取策略");

    // 開場注入
    let out = hooks::session_start(
        &d,
        &json!({"session_id": "s1", "cwd": a.to_string_lossy()}),
        None,
    );
    assert!(!out.contains(marker) && !out.contains("回答：LFU"), "{out}");

    // 工作摘要的上次進度
    let s = summary::build_summary(&d, Some("g"), None, None, None, None).unwrap();
    assert!(!s
        .progress
        .map(|p| p.text)
        .unwrap_or_default()
        .contains(marker));

    // 判斷回顧：滿 14 天也不算要回顧的判斷
    let events = spine::iter_events(&d).unwrap();
    let later = now() + Duration::days(30);
    let due = api::retro_candidates(&d, &events, None, later);
    assert!(
        due.iter()
            .all(|x| !x["ref"].as_str().unwrap().starts_with("chosen:")),
        "{due:?}"
    );

    // 反芻的「手上工作」
    let (focus, _) = ruminate::focus(&d, "g", &events, &[]).unwrap();
    assert!(!focus.contains(marker), "{focus}");

    // 不是交接
    let rec = events
        .iter()
        .find(|e| api::event_ref(e) == r.answered.reference)
        .unwrap();
    assert!(!api::is_handoff(rec));
    // 留痕查得到
    assert!(spine::is_decide_record(rec));
}

#[test]
fn relay_plan_after_answer_is_ready_with_answer() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let root = t.join("claude-projects");
    register(&d, t, "asky");
    let ask = ask3().validate().unwrap();
    handoff_at(&d, "asky", ago(1, 0), "2026-01-01-d1", "等你挑", Some(&ask));
    let scope = RelayScope::Repos(vec!["asky".into()]);
    let p = relay::plan(&d, &scope).unwrap();
    assert_eq!(p.rows[0].status, RelayStatus::NeedsYou);
    assert_eq!(p.waiting_on_you, 1);
    let r = decide::answer_with(&d, "asky", "1", Some(&root)).unwrap();
    let p = relay::plan(&d, &scope).unwrap();
    let row = &p.rows[0];
    assert_eq!(row.status, RelayStatus::Ready, "答過就能接");
    assert!(!row.waits_on_you);
    assert_eq!(p.waiting_on_you, 0);
    let task = row.prompt.as_deref().unwrap();
    assert!(
        task.contains("使用者對「快取要用 LRU 還是 LFU？」的回答是：LRU"),
        "{task}"
    );
    assert!(row
        .notes
        .iter()
        .any(|n| n.contains(&r.answered.reference) && n.contains("LRU")));
}
