//! 工作台的畫面資料與行動按鈕。對應 `tests/test_webui.py`（L1 狀態函數＋原本經 HTTP 打的 action）。
//!
//! Python 版的 L2 測試起真 HTTP server 用 urllib 打 `/api/*`；桌面版沒有 HTTP（走 Tauri IPC），
//! 這裡改直接呼叫 `workbench::{build_state, build_scan, handle_action}`——HTTP 回 400 的情況＝這裡回 `Err`。
//! 傳輸層（token、只綁 127.0.0.1、404）在 `desktop/src-tauri/src/workbench.rs` 的單元測試另測 IPC 層。

use std::path::Path;

use chrono::{Duration, Local, NaiveDateTime};
use ctx_core::collide;
use ctx_core::registry;
use ctx_core::spine::{self, Event, EventType, Query};
use ctx_core::term::{AgentLaunch, TermInfo, TermKind};
use ctx_core::testkit::{make_git_repo, spine_with_repos, today_at};
use ctx_core::workbench::{
    annotate_terms, build_scan, build_state, handle_action, is_read_only, session_draft,
    StateScope, Terms, ACTIONS,
};
use serde_json::{json, Value};

fn st(d: &Path) -> Value {
    build_state(d, &StateScope::default()).unwrap()
}

fn st_in(d: &Path, group: Option<&str>, repos: Option<&[String]>) -> Value {
    build_state(d, &StateScope::new(group, repos)).unwrap()
}

fn act(d: &Path, name: &str, payload: Value) -> ctx_core::Result<Value> {
    handle_action(d, name, &payload, None)
}

fn err(d: &Path, name: &str, payload: Value) -> String {
    act(d, name, payload).unwrap_err().to_string()
}

fn of_type(d: &Path, t: EventType) -> Vec<Event> {
    spine::query(d, &Query::of_type(t)).unwrap()
}

fn ago(minutes: i64) -> NaiveDateTime {
    Local::now().naive_local() - Duration::minutes(minutes)
}

fn ev(d: &Path, t: &str, src: &str, tokens: &[&str], body: &str, when: NaiveDateTime) -> Event {
    spine::append_event(d, t, src, tokens, body, Some(when)).unwrap()
}

fn cards_of<'a>(st: &'a Value, kind: &str) -> Vec<&'a Value> {
    st["cards"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == kind)
        .collect()
}

fn card_keys(st: &Value) -> Vec<String> {
    st["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["key"].as_str().unwrap().to_string())
        .collect()
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

// ── L1：狀態函數 ─────────────────────────────────────────────

#[test]
fn test_build_state_fields() {
    let (_t, d) = spine_with_repos();
    ev(
        &d,
        "open-loop",
        "hotkey",
        &["#1"],
        "opened →「試」",
        today_at(9, 0),
    );
    let st = st(&d);
    assert_eq!(st["groups"][0]["name"], "g1");
    assert_eq!(st["groups"].as_array().unwrap().len(), 1);
    assert_eq!(strs(&st["groups"][0]["members"]), vec!["repo-a", "repo-b"]);
    let mut repos = strs(&st["repos"]);
    repos.sort();
    assert_eq!(repos, vec!["repo-a", "repo-b"]);
    assert_eq!(st["unread"].as_array().unwrap().len(), 1);
    assert_eq!(st["unread"][0]["type"], "open-loop");
    assert!(st["unread"][0].get("interrupt").is_none()); // 一般事件不帶打斷標記
    assert_eq!(st["loops"].as_array().unwrap().len(), 1);
    let mut keys: Vec<&String> = st["stats"].as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "alive",
            "collisions",
            "collisions_with_outcome",
            "hit_rate",
            "spawned",
            "survival_rate"
        ]
    );
}

#[test]
fn test_build_state_interrupt_first() {
    let (_t, d) = spine_with_repos();
    ev(&d, "decision", "manual", &[], "一般", today_at(9, 0));
    ev(
        &d,
        "suggestion",
        "timer",
        &[],
        "system-unsure：任務掛了",
        today_at(9, 1),
    );
    let st = st(&d);
    assert_eq!(st["unread"][0]["interrupt"], true); // interrupt 置頂
    assert_eq!(st["unread"][0]["source"], "timer");
    assert!(st["unread"][1].get("interrupt").is_none());
}

// 未搬：test_app_without_pywebview_exits_with_hint——pywebview 視窗模式是 Python 版的殼；
// 桌面版換成 Tauri（desktop/src-tauri），沒有「缺 pywebview」這種退場。

#[test]
fn test_build_scan_fields() {
    let (_t, d) = spine_with_repos();
    let sc = build_scan(&d, Some("g1"), None).unwrap();
    assert_eq!(sc["group"], "g1");
    let mut ids: Vec<String> = sc["states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["repo-a", "repo-b"]);
    assert!(strs(&sc["questions"]).iter().any(|q| q.contains("repo-b"))); // dirty 5 天 → 問句
    let all = build_scan(&d, None, None).unwrap(); // 無組＝全部
    assert_eq!(all["group"], "全部");
    assert_eq!(all["states"].as_array().unwrap().len(), 2);
}

// ── L2：原本經 HTTP 的 action ───────────────────────────────

// 未搬：test_inv_binds_localhost_only、test_inv_requires_token——桌面版無 HTTP（改測 IPC 層：
// desktop/src-tauri/src/workbench.rs 的 `ipc_has_no_network_surface`）。

/// 原測試另驗 `/` 的 HTML 文案與 /static 資產——桌面版的頁面是 React（desktop/src，前端測試另驗）；
/// 這裡只驗 state／scan 的內容（`api_state` 補 terms＝[]、agents）。
#[test]
fn test_page_and_state() {
    let (_t, d) = spine_with_repos();
    let st = ctx_core::workbench::api_state(&d, &StateScope::default(), &[]).unwrap();
    assert_eq!(st["groups"][0]["name"], "g1");
    assert_eq!(st["terms"], json!([])); // 終端清冊隨 state 輪詢
    assert_eq!(st["agents"], json!(["claude"]));
    assert_eq!(build_scan(&d, Some("g1"), None).unwrap()["group"], "g1");
}

#[test]
fn test_brief_via_api() {
    let (_t, d) = spine_with_repos();
    let r = act(&d, "brief", json!({"group": "g1"})).unwrap();
    let text = r["text"].as_str().unwrap();
    assert!(text.contains("早晨簡報") && text.contains("沉默摘要"));
    assert!(!of_type(&d, EventType::Presented).is_empty()); // presented 有留痕
}

#[test]
fn test_collide_via_api() {
    let (_t, d) = spine_with_repos();
    let r = act(
        &d,
        "collide",
        json!({"idea": "從監控台丟的想法", "group": "g1", "wait": true}),
    )
    .unwrap();
    assert_eq!(r["ok"], true);
    assert!(!r["judgement"]["判定"].as_str().unwrap().is_empty());
    let evs = of_type(&d, EventType::Collision);
    assert_eq!(evs.len(), 2); // opened + 判定結果
    assert!(evs[0].body.contains("從監控台丟的想法"));
    assert_eq!(err(&d, "collide", json!({"idea": "  "})), "想法不可為空"); // 空想法拒收
}

#[test]
fn test_ignore_writes_chosen() {
    let (_t, d) = spine_with_repos();
    ev(
        &d,
        "suggestion",
        "engine",
        &[],
        "upstream 有新 release",
        today_at(8, 0),
    );
    let r = act(
        &d,
        "ignore",
        json!({"type": "suggestion", "time": "08:00", "header": "## 08:00 suggestion [engine]"}),
    )
    .unwrap();
    assert_eq!(r["ok"], true);
    let chosen = of_type(&d, EventType::Chosen);
    assert_eq!(chosen.len(), 1);
    assert_eq!(chosen[0].kv("ref"), Some("suggestion:08:00"));
    assert!(chosen[0].body.contains("忽略並記錄")); // 忽略＝chosen 的負形，有留痕
}

#[test]
fn test_close_loop_and_ack() {
    let (_t, d) = spine_with_repos();
    // 事件時間要早於 ack 時刻，否則 ack 不掉
    ev(
        &d,
        "open-loop",
        "hotkey",
        &["#7"],
        "opened →「待辦」",
        ago(1),
    );
    assert_eq!(spine::open_loops(&d).unwrap().len(), 1);
    act(&d, "close_loop", json!({"num": "#7"})).unwrap();
    assert!(spine::open_loops(&d).unwrap().is_empty());
    assert!(err(&d, "close_loop", json!({"num": "７；rm -rf"})).contains("loop 編號不合法")); // 拒收
    act(&d, "ack", json!({})).unwrap();
    assert!(spine::get_unread(&d).unwrap().is_empty());
}

/// 原測試另驗 `GET /api/nope` 回 404——桌面版無 HTTP 路由；IPC 只有 run_action。
#[test]
fn test_unknown_routes() {
    let (_t, d) = spine_with_repos();
    assert!(err(&d, "nope", json!({})).contains("未知 action"));
}

#[test]
fn test_scan_with_repos_subset() {
    // 勾選＝臨時組合：scan 只回選取的 repo，問句同步過濾
    let (_t, d) = spine_with_repos();
    let sc = build_scan(&d, None, Some(&["repo-a".to_string()])).unwrap();
    let ids: Vec<&str> = sc["states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["repo-a"]);
    assert!(sc["group"].as_str().unwrap().starts_with("臨時("));
    assert!(!strs(&sc["questions"]).iter().any(|q| q.contains("repo-b"))); // dirty 的 repo-b 被排除
}

#[test]
fn test_collide_with_repos_adhoc() {
    let (_t, d) = spine_with_repos();
    let r = act(
        &d,
        "collide",
        json!({"idea": "只撞 repo-a 的想法", "repos": ["repo-a"], "wait": true}),
    )
    .unwrap();
    assert_eq!(r["ok"], true);
    assert!(r["judgement"].is_object());
    let opened: Vec<Event> = of_type(&d, EventType::Collision)
        .into_iter()
        .filter(|e| e.body.starts_with("opened"))
        .collect();
    assert_eq!(opened.last().unwrap().kv("group"), Some("臨時(repo-a)"));
}

#[test]
fn test_tag_api() {
    // tag：加/移除落 registry；state 帶 tags map 與詞彙（suggestions ∪ 既有）
    let (_t, d) = spine_with_repos();
    let r = act(&d, "tag", json!({"id": "repo-a", "add": ["PM", "開發"]})).unwrap();
    assert_eq!(r["tags"], json!(["PM", "開發"]));
    assert_eq!(
        registry::get_repo(&d, "repo-a").unwrap().tags(),
        vec!["PM", "開發"]
    );
    let st = st(&d);
    assert_eq!(st["tags"]["repo-a"], json!(["PM", "開發"]));
    assert_eq!(st["tags"]["repo-b"], json!([]));
    assert!(strs(&st["tag_vocab"]).contains(&"PM".to_string()));
    let r = act(&d, "tag", json!({"id": "repo-a", "remove": ["開發"]})).unwrap();
    assert_eq!(r["tags"], json!(["PM"]));
    // 使用者錯誤＝帶訊息的錯（走操作表）
    assert!(err(&d, "tag", json!({"id": "ghost", "add": ["x"]})).contains("repo 不存在"));
}

#[test]
fn test_save_group_api() {
    let (_t, d) = spine_with_repos();
    let r = act(&d, "save_group", json!({"name": "g2", "repos": ["repo-b"]})).unwrap();
    assert_eq!(r["name"], "g2");
    let (_, entries) = registry::resolve_group(&d, Some("g2"), None).unwrap();
    assert_eq!(
        entries.iter().map(|e| e.id()).collect::<Vec<_>>(),
        vec!["repo-b"]
    );
    assert!(act(&d, "save_group", json!({"name": "", "repos": ["repo-a"]})).is_err());
    assert!(act(&d, "save_group", json!({"name": "g3", "repos": []})).is_err());
}

// ── UI 第三輪（2026-09-02 檢討：出口變按鈕／統一卡片／會話有主）────────────

#[test]
fn test_cards_collision_pending_then_judged() {
    // 自己丟的想法＝「處理中」卡（無出口）；判定回來→原地換成判定卡，帶真出口
    let (_t, d) = spine_with_repos();
    let cid = collide::submit(&d, "測試想法", Some("g1"), None, "hotkey", Some(ago(2))).unwrap();
    let s = st(&d);
    let pend = cards_of(&s, "pending");
    assert_eq!(pend.len(), 1);
    assert!(pend[0]["title"].as_str().unwrap().contains("測試想法"));
    assert_eq!(pend[0]["actions"], json!([]));
    // opened 事件不會再以「一則可忽略的未讀」出現在收件匣
    assert!(!cards_of(&s, "event")
        .iter()
        .any(|c| c["title"].as_str().unwrap().starts_with("opened")));
    collide::run_judgement(&d, &cid, None, None, Some("mock"), Some(ago(1))).unwrap();
    let s = st(&d);
    assert!(cards_of(&s, "pending").is_empty());
    let judged = cards_of(&s, "collision");
    assert_eq!(judged.len(), 1);
    let c = judged[0];
    assert_eq!(c["cid"], cid.as_str());
    assert!(c["title"].as_str().unwrap().contains("測試想法"));
    assert!(!c["judgement"]["判定"].as_str().unwrap().is_empty());
    let actions = c["actions"].as_array().unwrap();
    let names: Vec<&str> = actions
        .iter()
        .map(|a| a["action"].as_str().unwrap())
        .collect();
    for n in ["route_collision", "term_create", "ignore"] {
        assert!(names.contains(&n), "{n}");
    }
    let labels: Vec<&str> = actions
        .iter()
        .map(|a| a["label"].as_str().unwrap())
        .collect();
    assert!(labels.iter().any(|l| l.starts_with("照建議落")));
    assert!(labels.contains(&"與 Agent 深入討論") && labels.contains(&"丟棄並記錄"));
    // 建議＝incubator 時不重複出「升格」
    assert_eq!(labels.iter().filter(|l| l.contains("incubator")).count(), 1);
}

#[test]
fn test_cards_hide_own_records_and_handled() {
    // presented/chosen 是自己出手的留痕，不進收件匣；被 chosen ref 到的事件＝已處理
    let (_t, d) = spine_with_repos();
    let t0 = ago(3);
    ev(&d, "presented", "morning-brief", &[], "呈現了", t0);
    let e = ev(
        &d,
        "suggestion",
        "engine",
        &[],
        "upstream 有新 release",
        t0 + Duration::minutes(1),
    );
    assert_eq!(card_keys(&st(&d)), vec![format!("suggestion:{}", e.time)]);
    act(
        &d,
        "ignore",
        json!({"type": "suggestion", "time": e.time, "header": e.header()}),
    )
    .unwrap();
    assert!(card_keys(&st(&d)).is_empty()); // chosen 本身也不進收件匣
}

#[test]
fn test_cards_interrupt_first_and_loops() {
    let (_t, d) = spine_with_repos();
    let t0 = ago(3);
    ev(
        &d,
        "open-loop",
        "hotkey",
        &["#1", "due:2030-01-01"],
        "opened →「試」",
        t0,
    );
    ev(
        &d,
        "collision",
        "engine",
        &["id:x1"],
        "判定失敗（system-unsure，准打斷）：boom",
        t0 + Duration::minutes(1),
    );
    let s = st(&d);
    assert_eq!(s["cards"][0]["kind"], "interrupt");
    let lp = cards_of(&s, "loop")[0];
    assert_eq!(lp["num"], "#1");
    assert_eq!(lp["due"], "2030-01-01");
    let mut names: Vec<&str> = lp["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["action"].as_str().unwrap())
        .collect();
    names.sort();
    // brief_session＝Rust 才有（F-42 用任務包開會話）；其餘三個同 Python
    assert_eq!(
        names,
        vec![
            "brief_session",
            "close_loop",
            "collide_prefill",
            "loop_defer"
        ]
    );
    // open-loop 事件只以 loop 卡呈現，不再重複成一則「其他未讀」
    assert!(!cards_of(&s, "event")
        .iter()
        .any(|c| c["etype"] == "open-loop"));
}

#[test]
fn test_scan_question_cards_and_defer() {
    // 問句是卡：有 kind/repo/actions；defer 後同一問句 7 天內不再浮出（chosen 留痕）
    let (_t, d) = spine_with_repos();
    let sc = build_scan(&d, Some("g1"), None).unwrap();
    let qc: Vec<&Value> = sc["question_cards"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["repo"] == "repo-b")
        .collect();
    assert_eq!(qc.len(), 1);
    assert_eq!(qc[0]["kind"], "question");
    let mut names: Vec<&str> = qc[0]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["action"].as_str().unwrap())
        .collect();
    names.sort();
    names.dedup();
    assert_eq!(names, vec!["defer", "term_create"]);
    act(
        &d,
        "defer",
        json!({"id": "repo-b", "qkind": qc[0]["qkind"], "days": 7, "text": qc[0]["title"]}),
    )
    .unwrap();
    let sc = build_scan(&d, Some("g1"), None).unwrap();
    assert!(!sc["question_cards"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["repo"] == "repo-b"));
    assert!(!strs(&sc["questions"]).iter().any(|q| q.contains("repo-b")));
    let ch = of_type(&d, EventType::Chosen);
    assert_eq!(ch.len(), 1);
    assert_eq!(ch[0].kv("repo"), Some("repo-b"));
    assert!(ch[0].body.contains("snooze"));
}

#[test]
fn test_tier_action() {
    let (_t, d) = spine_with_repos();
    let r = act(&d, "tier", json!({"id": "repo-a", "tier": "dormant"})).unwrap();
    assert_eq!(r["tier"], "dormant");
    assert_eq!(
        registry::get_repo(&d, "repo-a")
            .unwrap()
            .tier_str()
            .as_deref(),
        Some("dormant")
    );
    let dec = of_type(&d, EventType::Decision);
    let last = dec.last().unwrap();
    assert_eq!(last.kv("repo"), Some("repo-a"));
    assert!(last.body.contains("dormant"));
    assert!(act(&d, "tier", json!({"id": "repo-a", "tier": "bogus"})).is_err());
}

#[test]
fn test_route_collision_action() {
    let (_t, d) = spine_with_repos();
    let r = act(
        &d,
        "collide",
        json!({"idea": "落到 incubator 的想法", "group": "g1", "wait": true}),
    )
    .unwrap();
    let cid = r["cid"].as_str().unwrap().to_string();
    let r = act(
        &d,
        "route_collision",
        json!({"cid": cid, "dest": "incubator"}),
    )
    .unwrap();
    assert_eq!(r["ok"], true);
    let files: Vec<_> = std::fs::read_dir(d.join("incubator"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    assert_eq!(files.len(), 1);
    assert!(std::fs::read_to_string(&files[0])
        .unwrap()
        .contains("落到 incubator 的想法"));
    let ch = of_type(&d, EventType::Chosen);
    assert_eq!(
        ch.last().unwrap().kv("ref"),
        Some(format!("collision:{cid}").as_str())
    );
    // 落完就從收件匣消失
    assert!(!st(&d)["cards"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["cid"] == cid.as_str()));
    assert!(act(&d, "route_collision", json!({"cid": cid, "dest": "mars"})).is_err());
}

#[test]
fn test_loop_defer_action() {
    let (_t, d) = spine_with_repos();
    let today = Local::now().date_naive();
    let due0 = format!("due:{}", today.format("%Y-%m-%d"));
    ev(
        &d,
        "open-loop",
        "hotkey",
        &["#9", &due0],
        "opened →「明天再說」",
        ago(1),
    );
    act(&d, "loop_defer", json!({"num": "#9", "days": 7})).unwrap();
    let loops = spine::open_loops(&d).unwrap();
    assert_eq!(loops.len(), 1);
    let want = (today + Duration::days(7)).format("%Y-%m-%d").to_string();
    assert_eq!(loops[0].kv("due"), Some(want.as_str()));
    assert!(act(&d, "loop_defer", json!({"num": "9", "days": 7})).is_err());
}

#[test]
fn test_agent_session_title_has_task() {
    // 會話有主：title 用任務摘要，不再只有 claude:全部
    use ctx_core::term::agent_title;
    assert_eq!(agent_title("claude", "MI_PM", None, 40), "claude · MI_PM");
    let t = agent_title(
        "claude",
        "全部",
        Some("把 §7 動詞鏈對到 MCP 工具清單，順便檢查 README"),
        40,
    );
    assert!(t.starts_with("claude · 把 §7 動詞鏈對到") && t.chars().count() <= 40);
}

#[test]
fn test_relations_api_and_pulse() {
    // 組為單位輪：state 帶關係與詞彙、relate/unrelate action、scan 帶脈動
    let (_t, d) = spine_with_repos();
    let r = act(
        &d,
        "relate",
        json!({"a": "repo-a", "b": "repo-b", "kind": "pm-of", "note": "n"}),
    )
    .unwrap();
    assert_eq!(r["ok"], true);
    assert!(act(
        &d,
        "relate",
        json!({"a": "repo-a", "b": "repo-b", "kind": "boss"})
    )
    .is_err());
    let s = st(&d);
    let rel = &s["relations"][0];
    assert_eq!(s["relations"].as_array().unwrap().len(), 1);
    for (k, v) in [
        ("from", "repo-a"),
        ("to", "repo-b"),
        ("kind", "pm-of"),
        ("note", "n"),
    ] {
        assert_eq!(rel[k], v);
    }
    // 2026-10-06：任務文案與範圍歸屬由引擎給
    assert_eq!(rel["in_scope"], true);
    assert!(rel["task"]
        .as_str()
        .unwrap()
        .contains("repo-a 是 repo-b 的 PM"));
    assert_eq!(s["rel_kinds"]["pm-of"]["forward"], "規劃（PM）");
    let sc = build_scan(&d, Some("g1"), None).unwrap();
    assert_eq!(sc["pulse"]["commits_7d"], 2);
    assert!(sc["pulse"]["most_active"].is_string());
    assert!(!sc["states"][0]["commits_7d"].is_null());
    assert!(!sc["states"][0]["recent_commits"]
        .as_array()
        .unwrap()
        .is_empty());
    let r0 = sc["recent_commits"][0]["repo"].as_str().unwrap();
    assert!(r0 == "repo-a" || r0 == "repo-b");
    act(&d, "unrelate", json!({"a": "repo-a", "b": "repo-b"})).unwrap();
    assert_eq!(st(&d)["relations"], json!([]));
}

// ── 範圍歸屬、復原合約、會話記錄 ─────────────────────────────

#[test]
fn test_event_scope_inherits_collision_and_keeps_unknown_unclassified() {
    let (_t, d) = spine_with_repos();
    registry::add_group(&d, "g2", &["repo-b"]).unwrap();
    let t0 = ago(5);
    ev(
        &d,
        "collision",
        "hotkey",
        &["id:scope-a", "group:g1"],
        "opened\n輸入：A 組工作",
        t0,
    );
    ev(
        &d,
        "collision",
        "engine",
        &["id:scope-a"],
        "判定：真增量\n理由：有新內容\n落點建議：incubator",
        t0 + Duration::minutes(1),
    );
    ev(
        &d,
        "suggestion",
        "engine",
        &["repo:repo-b"],
        "B 的更新",
        t0 + Duration::minutes(2),
    );
    ev(
        &d,
        "suggestion",
        "timer",
        &[],
        "system-unsure：全域排程失敗",
        t0 + Duration::minutes(3),
    );
    let s = st(&d);
    let cards = s["cards"].as_array().unwrap();
    let reply = cards.iter().find(|c| c["cid"] == "scope-a").unwrap();
    assert_eq!(reply["scope_label"], "g1");
    let mut sr = strs(&reply["scope_repos"]);
    sr.sort();
    assert_eq!(sr, vec!["repo-a", "repo-b"]);
    let repo_card = cards.iter().find(|c| c["title"] == "B 的更新").unwrap();
    assert_eq!(repo_card["scope_repos"], json!(["repo-b"]));
    let warning = cards.iter().find(|c| c["kind"] == "interrupt").unwrap();
    assert_eq!(warning["scope_kind"], "unclassified");
    assert_eq!(warning["scope_repos"], json!([]));
    act(
        &d,
        "route_collision",
        json!({"cid": "scope-a", "dest": "incubator"}),
    )
    .unwrap();
    let s = st(&d);
    let record = s["recent"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == "chosen")
        .unwrap();
    assert_eq!(record["scope_label"], "g1");
}

#[test]
fn test_adhoc_scope_and_global_collision() {
    let (_t, d) = spine_with_repos();
    let a = collide::submit(
        &d,
        "子集",
        None,
        Some(&["repo-a".to_string()]),
        "hotkey",
        None,
    )
    .unwrap();
    let b = collide::submit(&d, "全域", None, None, "hotkey", None).unwrap();
    let s = st(&d);
    let card = |cid: &str| {
        s["cards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["cid"] == cid)
            .cloned()
            .unwrap()
    };
    assert_eq!(card(&a)["scope_repos"], json!(["repo-a"]));
    assert_eq!(card(&b)["scope_kind"], "global");
}

#[test]
fn test_loop_scope_survives_defer_and_close() {
    let (_t, d) = spine_with_repos();
    spine::append_event(
        &d,
        "open-loop",
        "hotkey",
        &["#42", "repo:repo-a"],
        "opened →「跟進」",
        None,
    )
    .unwrap();
    act(&d, "loop_defer", json!({"num": "#42", "days": 7})).unwrap();
    let s = st(&d);
    let card = cards_of(&s, "loop")[0];
    assert_eq!(card["scope_repos"], json!(["repo-a"]));
    act(&d, "close_loop", json!({"num": "#42"})).unwrap();
    let s = st(&d);
    let record = s["recent"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["body"].as_str().unwrap().starts_with("closed"))
        .unwrap();
    assert_eq!(record["scope_repos"], json!(["repo-a"]));
}

#[test]
fn test_recent_sorts_by_event_time_not_append_order() {
    let (_t, d) = spine_with_repos();
    ev(&d, "decision", "manual", &[], "newer", today_at(15, 0));
    ev(&d, "decision", "manual", &[], "backfilled", today_at(8, 0));
    let bodies: Vec<String> = st(&d)["recent"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["body"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(bodies, vec!["newer", "backfilled"]);
}

#[test]
fn test_empty_explicit_scope_never_expands_to_everything() {
    let (_t, d) = spine_with_repos();
    for action in ["collide", "brief", "term_create"] {
        let e = err(
            &d,
            action,
            json!({"repos": [], "idea": "test", "kind": "agent"}),
        );
        assert!(e.contains("空範圍"), "{action}: {e}");
    }
    assert!(of_type(&d, EventType::Collision).is_empty());
    let e = build_scan(&d, None, Some(&[])).unwrap_err().to_string();
    assert!(e.contains("空範圍"));
}

/// 假會話清冊（Python 測試用 SimpleNamespace）。
struct FakeTerms(Vec<TermInfo>);

impl Terms for FakeTerms {
    fn info(&self, sid: &str) -> Option<TermInfo> {
        self.0.iter().find(|t| t.sid == sid).cloned()
    }
    fn create_shell(
        &self,
        _: Option<&Path>,
        _: Option<String>,
    ) -> ctx_core::Result<(String, String)> {
        Ok(("t9".into(), "shell".into()))
    }
    fn create_agent(&self, l: AgentLaunch) -> ctx_core::Result<(String, String)> {
        Ok((
            "t8".into(),
            format!("{} · {}", l.agent, l.task.unwrap_or_default()),
        ))
    }
    fn kill(&self, sid: &str) -> bool {
        self.0.iter().any(|t| t.sid == sid)
    }
}

fn term_info(sid: &str, title: &str, scope: Option<&str>, origin: Option<&str>) -> TermInfo {
    TermInfo {
        sid: sid.into(),
        title: title.into(),
        alive: true,
        kind: TermKind::Agent,
        origin: origin.map(str::to_string),
        scope: scope.map(str::to_string),
        scope_repos: None,
        started: String::new(),
        task: None,
        home_group: None,
    }
}

#[test]
fn test_session_note_records_result_without_claiming_adoption() {
    let (_t, d) = spine_with_repos();
    let terms = FakeTerms(vec![term_info(
        "t1",
        "Agent 任務",
        Some("g1"),
        Some("group:g1"),
    )]);
    let run = |p: Value| handle_action(&d, "session_result", &p, Some(&terms));
    assert!(run(json!({"sid": "unknown", "text": "結果"})).is_err());
    assert!(run(json!({"sid": "t1", "text": " "})).is_err());
    let r = run(json!({"sid": "t1", "text": "完成檢查，下一步確認部署條件"})).unwrap();
    assert_eq!(r["ok"], true);
    let evs = of_type(&d, EventType::Decision);
    let last = evs.last().unwrap();
    assert_eq!(last.kv("group"), Some("g1"));
    assert!(last.body.contains("完成檢查"));
    assert!(of_type(&d, EventType::Outcome).is_empty());
}

// ── 2026-09-12 UX 檢討落地 ────────────────────────────────

#[test]
fn test_defer_trace_says_what_it_actually_did() {
    // §6.1：按鈕文字、執行效果、成功回傳與活動紀錄要講同一件事
    let (_t, d) = spine_with_repos();
    let r = act(
        &d,
        "defer",
        json!({"id": "repo-b", "qkind": "dirty", "days": 7}),
    )
    .unwrap();
    assert_eq!(r["ok"], true);
    assert!(r["until"].is_string());
    assert_eq!(r["days"], 7);
    let evs = of_type(&d, EventType::Chosen);
    let e = evs.last().unwrap();
    let first = e.body.lines().next().unwrap();
    assert!(first.contains("7 天後再提醒") && first.contains("沒有安排任何進度"));
    assert!(!e.body.contains("排進度"));
    // 人話在前，機器仍讀得到 snooze token（同一問句 7 天內不再浮出）
    let sc = build_scan(&d, Some("g1"), None).unwrap();
    assert!(!sc["question_cards"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["repo"] == "repo-b"));
}

#[test]
fn test_scan_carries_work_summary() {
    // §3：首屏要能回答『這組在做什麼、上次做到哪、接下來一步』，每格帶依據
    let (_t, d) = spine_with_repos();
    let s = build_scan(&d, Some("g1"), None).unwrap()["summary"].clone();
    assert_eq!(s["group"], "g1");
    for k in ["goal", "progress", "changes", "next", "gaps", "resume_task"] {
        assert!(s.get(k).is_some(), "{k}");
    }
    assert!(s["goal"].is_null() && s["goal_gap"].is_string()); // 沒登記目標就直說
    act(
        &d,
        "set_goal",
        json!({"group": "g1", "text": "把兩個 repo 的說法對齊"}),
    )
    .unwrap();
    let s2 = build_scan(&d, Some("g1"), None).unwrap()["summary"].clone();
    assert_eq!(s2["goal"]["text"], "把兩個 repo 的說法對齊");
    assert!(s2["goal"]["at"].is_string());
    assert_eq!(
        of_type(&d, EventType::Decision).last().unwrap().kv("group"),
        Some("g1")
    );
    // 臨時範圍沒有牌組可掛目標 → 講清楚，不靜默失敗
    assert!(act(&d, "set_goal", json!({"group": "", "text": "x"})).is_err());
}

#[test]
fn test_session_report_needs_a_trace_not_a_live_process() {
    // §5：程序存活不代表工作在推進——沒有留痕就是『尚無回報』
    let (_t, d) = spine_with_repos();
    let started = format!("{} 09:00", Local::now().format("%Y-%m-%d"));
    let term = json!({"sid": "t1", "title": "claude · 對齊驗收", "kind": "agent",
                      "scope": "g1", "scope_repos": ["repo-a", "repo-b"],
                      "started": started, "alive": true});
    let rows = annotate_terms(&d, std::slice::from_ref(&term)).unwrap();
    assert!(rows[0]["report"].is_null());
    assert!(!rows[0]["report_basis"].as_str().unwrap().is_empty());
    let report = || annotate_terms(&d, std::slice::from_ref(&term)).unwrap()[0]["report"].clone();
    // 會話開始前的留痕不算
    ev(
        &d,
        "decision",
        "agent",
        &["group:g1"],
        "開會話前就寫過的判斷",
        today_at(8, 0),
    );
    assert!(report().is_null());
    // 別組的留痕也不算
    ev(
        &d,
        "decision",
        "agent",
        &["group:other"],
        "別組的判斷",
        today_at(10, 0),
    );
    assert!(report().is_null());
    // 會話期間、本組的留痕才是回報，而且看得出是誰寫的
    ev(
        &d,
        "decision",
        "agent",
        &["group:g1"],
        "比對完兩邊驗收條件",
        today_at(11, 0),
    );
    let rep = report();
    assert_eq!(rep["text"], "比對完兩邊驗收條件");
    assert_eq!(rep["source"], "agent");
    assert_eq!(rep["type"], "decision");
    assert!(rep["at"].as_str().unwrap().ends_with("11:00"));
}

#[test]
fn test_session_draft_only_assembles_existing_traces() {
    // §5：交接草稿由既有留痕組成——沒有留痕就直說沒有，不編造成果
    let (_t, d) = spine_with_repos();
    let started = format!("{} 09:00", Local::now().format("%Y-%m-%d"));
    let term = json!({"sid": "t1", "title": "claude · 對齊驗收", "kind": "agent",
                      "scope": "g1", "scope_repos": ["repo-a", "repo-b"], "started": started});
    let dr = session_draft(&d, &term).unwrap();
    assert_eq!(dr["traces"], 0);
    let text = dr["text"].as_str().unwrap();
    assert!(text.contains("沒有留痕") && text.contains("下次從哪裡接回"));
    assert!(dr["note"].as_str().unwrap().contains("尚未儲存"));
    ev(
        &d,
        "decision",
        "agent",
        &["group:g1"],
        "比對完兩邊驗收條件",
        today_at(11, 0),
    );
    ev(
        &d,
        "open-loop",
        "agent",
        &["#3", "group:g1", "due:2099-01-01"],
        "補齊缺的驗收條件",
        today_at(11, 5),
    );
    let d2 = session_draft(&d, &term).unwrap();
    assert_eq!(d2["traces"], 2);
    let text = d2["text"].as_str().unwrap();
    assert!(text.contains("比對完兩邊驗收條件") && text.contains("#3"));
    assert_eq!(d2["loops"], 1);
}

#[test]
fn test_first_run_scan_and_register() {
    // §7：首次使用可以在介面完成——選資料夾 → 預覽 → 建第一組
    let (tmp, d) = spine_with_repos();
    let base = tmp.path().join("workspace");
    make_git_repo(&base, "fresh-one", 1, 0, true);
    make_git_repo(&base, "fresh-two", 1, 0, true);
    std::fs::create_dir_all(base.join("not-a-repo")).unwrap();
    let r = act(&d, "scan_dir", json!({"dir": base.to_string_lossy()})).unwrap();
    let mut ids: Vec<String> = r["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["fresh-one", "fresh-two"]);
    let bad = err(
        &d,
        "scan_dir",
        json!({"dir": tmp.path().join("nope").to_string_lossy()}),
    );
    assert!(bad.contains("目錄不存在"));
    let r = act(
        &d,
        "register_repos",
        json!({"repos": r["candidates"], "group": "第一組"}),
    )
    .unwrap();
    assert_eq!(r["added"], json!(["fresh-one", "fresh-two"]));
    assert_eq!(r["group"], "第一組");
    assert_eq!(
        registry::get_group(&d, "第一組").unwrap().members(),
        vec!["fresh-one", "fresh-two"]
    );
    assert!(of_type(&d, EventType::Decision)
        .last()
        .unwrap()
        .body
        .contains("登記 2 個 repo"));
    assert!(act(&d, "register_repos", json!({"repos": []})).is_err());
}

// ── 從其他 Python 測試檔搬來的工作台段落 ─────────────────────

/// 對應 tests/test_agentapi.py::test_workbench_session_shows_agent_status（agentapi 另一波在搬；
/// 這裡用 `status::save_status` 直接寫現況檔，驗 annotate_terms 只認會話開始後的回報）。
#[test]
fn test_workbench_session_shows_agent_status() {
    use ctx_core::status::{save_status, StatusRecord};
    let (_t, d) = spine_with_repos();
    let started = (Local::now() - Duration::minutes(1))
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let term = json!({"sid": "t1", "kind": "agent", "scope": "g1", "started": started});
    assert!(annotate_terms(&d, std::slice::from_ref(&term)).unwrap()[0]["agent_status"].is_null());
    let rec = |at: String| StatusRecord {
        status: "blocked".into(),
        text: "缺 API key".into(),
        at,
        scope: "g1".into(),
        group: Some("g1".into()),
        repos: None,
        repo: Some("repo-a".into()),
    };
    // 會話開始前的舊回報不算
    save_status(&d, &rec("2000-01-01 00:00".into())).unwrap();
    assert!(annotate_terms(&d, std::slice::from_ref(&term)).unwrap()[0]["agent_status"].is_null());
    save_status(&d, &rec(Local::now().format("%Y-%m-%d %H:%M").to_string())).unwrap();
    let row = annotate_terms(&d, std::slice::from_ref(&term)).unwrap()[0].clone();
    assert_eq!(row["agent_status"]["status"], "blocked");
}

// ── 2026-10-06：範圍篩選、任務文案移進引擎（前端不放業務邏輯）──────────

/// 卡片／事件帶 lane：範圍內＝scope、全域與未分類＝global、別組＝other；範圍外的打斷仍進 global。
#[test]
fn test_state_lanes_follow_selected_scope() {
    let (_t, d) = spine_with_repos();
    registry::add_group(&d, "g2", &["repo-b"]).unwrap();
    let t0 = ago(5);
    ev(&d, "suggestion", "engine", &["repo:repo-a"], "A 的更新", t0);
    ev(
        &d,
        "suggestion",
        "engine",
        &["repo:repo-b"],
        "B 的更新",
        t0 + Duration::minutes(1),
    );
    ev(
        &d,
        "suggestion",
        "timer",
        &[],
        "全域的事",
        t0 + Duration::minutes(2),
    );
    ev(
        &d,
        "suggestion",
        "engine",
        &["repo:repo-a"],
        "system-unsure：A 掛了",
        t0 + Duration::minutes(3),
    );
    let lane_of = |s: &Value, title: &str| -> String {
        s["cards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["title"] == title)
            .map(|c| c["lane"].as_str().unwrap().to_string())
            .unwrap()
    };
    // 全部：有範圍的都是 scope，沒範圍的是 global
    let all = st(&d);
    assert_eq!(all["scope"]["members"], json!(["repo-a", "repo-b"]));
    assert_eq!(lane_of(&all, "A 的更新"), "scope");
    assert_eq!(lane_of(&all, "全域的事"), "global");
    // g2（只有 repo-b）：A 的事不顯示，A 的打斷仍以全域警示出現
    let g2 = st_in(&d, Some("g2"), None);
    assert_eq!(g2["scope"]["group"], "g2");
    assert_eq!(g2["scope"]["members"], json!(["repo-b"]));
    assert_eq!(lane_of(&g2, "A 的更新"), "other");
    assert_eq!(lane_of(&g2, "B 的更新"), "scope");
    assert_eq!(lane_of(&g2, "全域的事"), "global");
    assert_eq!(lane_of(&g2, "system-unsure：A 掛了"), "global");
    // 臨時組合（只留登記過的）＋事件也帶 lane
    let adhoc = st_in(&d, None, Some(&["repo-a".into(), "ghost".into()]));
    assert_eq!(adhoc["scope"]["members"], json!(["repo-a"]));
    let recent = adhoc["recent"].as_array().unwrap();
    let rl = |body: &str| {
        recent.iter().find(|e| e["body"] == body).unwrap()["lane"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(rl("A 的更新"), "scope");
    assert_eq!(rl("B 的更新"), "other");
    assert_eq!(rl("全域的事"), "global");
}

/// 關係帶 in_scope／task；scan 帶 session_tasks（接續上次工作／整理本組現況／組內第一條關係）與問句卡 provenance。
#[test]
fn test_relation_tasks_and_session_tasks_come_from_engine() {
    let (tmp, d) = spine_with_repos();
    registry::add_repo(
        &d,
        "repo-c",
        make_git_repo(tmp.path(), "repo-c", 0, 0, true),
        registry::AddRepo::default(),
    )
    .unwrap();
    act(
        &d,
        "relate",
        json!({"a": "repo-a", "b": "repo-b", "kind": "feeds"}),
    )
    .unwrap();
    act(
        &d,
        "relate",
        json!({"a": "repo-c", "b": "repo-c", "kind": "custom-x"}),
    )
    .ok();
    let s = st_in(&d, Some("g1"), None);
    let rels = s["relations"].as_array().unwrap();
    let feeds = rels.iter().find(|r| r["kind"] == "feeds").unwrap();
    assert_eq!(feeds["in_scope"], true);
    assert!(feeds["task"]
        .as_str()
        .unwrap()
        .starts_with("檢查供料是否還新：repo-a 供料給 repo-b。"));
    let sc = build_scan(&d, Some("g1"), None).unwrap();
    let tasks = sc["session_tasks"].as_array().unwrap();
    let labels: Vec<&str> = tasks.iter().map(|t| t["label"].as_str().unwrap()).collect();
    assert_eq!(
        labels,
        vec!["接續上次工作", "整理本組現況", "供料給：repo-a → repo-b"]
    );
    assert_eq!(tasks[0]["text"], sc["summary"]["resume_task"]);
    assert_eq!(tasks[1]["text"], ctx_core::workbench::STATUS_PROMPT);
    let qc = &sc["question_cards"][0];
    assert_eq!(qc["scope_kind"], "repo");
    assert_eq!(qc["scope_repos"], json!([qc["repo"].clone()]));
    // 只有 repo-c 的臨時範圍：組內沒有關係 → 只有兩個起手任務
    let sc_c = build_scan(&d, None, Some(&["repo-c".into()])).unwrap();
    assert_eq!(sc_c["session_tasks"].as_array().unwrap().len(), 2);
}

/// term_create（agent）：session::build 組好指令後交給清冊；shell 照 cwd 開。
#[test]
fn test_term_create_agent_goes_through_session_build() {
    let (_t, d) = spine_with_repos();
    let terms = FakeTerms(vec![]);
    let r = handle_action(
        &d,
        "term_create",
        &json!({"kind": "agent", "group": "g1", "task": "對齊"}),
        Some(&terms),
    )
    .unwrap();
    assert_eq!(
        r,
        json!({"ok": true, "sid": "t8", "title": "claude · 對齊"})
    );
    // session::build 留了 presented 痕、寫了會話料
    assert!(of_type(&d, EventType::Presented)
        .iter()
        .any(|e| e.source == "session"));
    let r = handle_action(&d, "term_create", &json!({"kind": "shell"}), Some(&terms)).unwrap();
    assert_eq!(r["sid"], "t9");
    assert_eq!(
        act(&d, "term_create", json!({"kind": "shell"}))
            .unwrap_err()
            .to_string(),
        "本 server 未啟用 terminal"
    );
    assert_eq!(err(&d, "term_kill", json!({})), "未知會話: None");
    assert_eq!(
        err(&d, "session_draft", json!({"sid": "t5"})),
        "未知會話: t5"
    );
}

// Rust 才有（F-46）：收件匣的未結卡帶上 agent 看到的同一份體檢結果
#[test]
fn loop_card_shows_review_verdict_after_scan() {
    let (_t, d) = spine_with_repos();
    let yesterday = (Local::now() - Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    ev(
        &d,
        "open-loop",
        "agent",
        &["#1", "group:g1", &format!("due:{yesterday}")],
        "補 Windows 測試",
        ago(60 * 24 * 3),
    );
    ev(
        &d,
        "open-loop",
        "agent",
        &["#2", "group:g1"],
        "剛開的事",
        ago(5),
    );
    // scan 前：收件匣只做純檔案讀，沒有體檢結果
    let before = st(&d);
    assert!(cards_of(&before, "loop")
        .iter()
        .all(|c| c.get("verdict").is_none()));
    build_scan(&d, Some("g1"), None).unwrap();
    let after = st(&d);
    let loops = cards_of(&after, "loop");
    let c1 = loops.iter().find(|c| c["num"] == "#1").unwrap();
    assert_eq!(c1["verdict"], "擱置沒動靜");
    assert!(c1["lines"][0].as_str().unwrap().starts_with("體檢"));
    let c2 = loops.iter().find(|c| c["num"] == "#2").unwrap();
    assert!(c2.get("verdict").is_none()); // 進行中的不加註
}

// Rust 才有（F-47）：關係建議進 App 的 state；打掉後消失、設了關係後消失
#[test]
fn relation_suggestions_in_state_and_dismiss() {
    let (_t, d) = spine_with_repos();
    let a = registry::get_repo(&d, "repo-a").unwrap().path();
    for i in 0..3 {
        std::fs::write(
            a.join(format!("plan{i}.md")),
            format!("# 計畫\nrepo-b 的進度 {i}\n"),
        )
        .unwrap();
    }
    ctx_core::relsuggest::compute(&d).unwrap();
    let s = st(&d);
    let sug = s["relation_suggestions"].as_array().expect("要有建議");
    assert_eq!(
        (sug[0]["from"].as_str(), sug[0]["to"].as_str()),
        (Some("repo-a"), Some("repo-b"))
    );
    act(
        &d,
        "relation_dismiss",
        json!({"a": "repo-b", "b": "repo-a"}),
    )
    .unwrap();
    assert!(st(&d).get("relation_suggestions").is_none());
    // 重新算也不再出現
    assert!(ctx_core::relsuggest::compute(&d).unwrap().is_empty());
}

#[test]
fn relation_suggestion_disappears_after_relate() {
    let (_t, d) = spine_with_repos();
    let a = registry::get_repo(&d, "repo-a").unwrap().path();
    for i in 0..3 {
        std::fs::write(a.join(format!("plan{i}.md")), format!("repo-b 週報 {i}\n")).unwrap();
    }
    ctx_core::relsuggest::compute(&d).unwrap();
    act(
        &d,
        "relate",
        json!({"a": "repo-a", "b": "repo-b", "kind": "pm-of"}),
    )
    .unwrap();
    assert!(st(&d).get("relation_suggestions").is_none());
}

// Rust 才有（F-48）：反芻推薦成收件匣卡，出口＝拿來試／不相關
#[test]
fn fresh_cards_in_inbox_and_dismiss() {
    let (_t, d) = spine_with_repos();
    let state_p = d.join(".state/ruminate.json");
    std::fs::create_dir_all(state_p.parent().unwrap()).unwrap();
    let found = Local::now().format("%Y-%m-%d %H:%M").to_string();
    let link = |id: &str, rules: Option<u32>| {
        let mut l = json!({"id": id, "group": "g1", "kind": "file", "repo": "x", "ref": format!("x:{id}.md"),
            "title": format!("{id}.md"), "stamp": 1, "score": 9.0, "terms": ["向量", "索引"], "basis": [],
            "found": found, "status": "new"});
        if let Some(r) = rules {
            l["rules"] = json!(r);
        }
        l
    };
    std::fs::write(
        &state_p,
        json!({"last_run": null, "links": [link("r1", Some(ctx_core::ruminate::RULES_VERSION)), link("r2", None)]})
            .to_string(),
    )
    .unwrap();
    let s = st(&d);
    let fresh = cards_of(&s, "fresh");
    assert_eq!(fresh.len(), 1); // 舊規則的 r2 不成卡
    assert_eq!(fresh[0]["scope_label"], "g1");
    assert_eq!(fresh[0]["actions"][0]["action"], "fresh_try");
    act(&d, "fresh_dismiss", json!({"id": "r1"})).unwrap();
    assert!(cards_of(&st(&d), "fresh").is_empty());
}

// Rust 才有（F-46）：重看條件到期的判斷成卡；延後之後卡片與 agent 開場一起消失
#[test]
fn reopen_cards_due_and_defer() {
    let (_t, d) = spine_with_repos();
    let yesterday = (Local::now() - Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    ev(
        &d,
        "decision",
        "agent",
        &["id:2026-01-01-d1", "group:g1"],
        &format!("先不做 Windows\n何時重看：{yesterday} 或 VDI 使用者回報"),
        ago(60),
    );
    ev(
        &d,
        "decision",
        "agent",
        &["id:2026-01-01-d2", "group:g1"],
        "索引用 JSON\n何時重看：repo 超過 50 個",
        ago(50),
    );
    let s = st(&d);
    let rc = cards_of(&s, "reopen");
    assert_eq!(rc.len(), 1); // 沒日期的條件不成卡（agent 開場仍列為「有條件的判斷」）
    assert_eq!(rc[0]["title"], "該重看：先不做 Windows");
    assert_eq!(rc[0]["actions"][0]["action"], "term_create");
    assert!(rc[0]["actions"][0]["payload"]["task"]
        .as_str()
        .unwrap()
        .contains("supersedes decision:2026-01-01-d1"));
    let r = act(
        &d,
        "reopen_defer",
        json!({"ref": "decision:2026-01-01-d1", "days": 14}),
    )
    .unwrap();
    assert!(r["until"].as_str().is_some());
    assert!(cards_of(&st(&d), "reopen").is_empty());
}

// Rust 才有（F-23）：判斷回顧卡；延 30 天後消失
#[test]
fn retro_cards_and_defer() {
    let (_t, d) = spine_with_repos();
    ev(
        &d,
        "decision",
        "agent",
        &["id:2026-01-01-d1", "group:g1"],
        "快取用 LRU",
        ago(60 * 24 * 20),
    );
    let s = st(&d);
    let rc = cards_of(&s, "retro");
    assert_eq!(rc.len(), 1);
    assert_eq!(rc[0]["title"], "判斷回顧：快取用 LRU");
    assert!(rc[0]["actions"][0]["payload"]["task"]
        .as_str()
        .unwrap()
        .contains("outcome_of=decision:2026-01-01-d1"));
    act(
        &d,
        "reopen_defer",
        json!({"ref": "decision:2026-01-01-d1", "days": 30}),
    )
    .unwrap();
    assert!(cards_of(&st(&d), "retro").is_empty());
}

// Rust 才有（F-45）：延期間隔遞增 7 → 14 → 30，延 3 次以上提醒
#[test]
fn loop_defer_escalates() {
    let (_t, d) = spine_with_repos();
    ev(
        &d,
        "open-loop",
        "agent",
        &["#1", "group:g1"],
        "opened → 補測試",
        ago(60),
    );
    let defer_of = |s: &Value| {
        let c = cards_of(s, "loop")
            .into_iter()
            .find(|c| c["num"] == "#1")
            .unwrap()
            .clone();
        let a = c["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["action"] == "loop_defer")
            .unwrap()
            .clone();
        (
            a["label"].as_str().unwrap().to_string(),
            a["payload"]["days"].as_i64().unwrap(),
            c,
        )
    };
    assert_eq!(defer_of(&st(&d)).0, "延 7 天"); // 第一次同 Python
    for (label, days) in [
        ("之後再說（14 天）", 14),
        ("之後再說（30 天）", 30),
        ("之後再說（30 天）", 30),
    ] {
        let (_, cur, _) = defer_of(&st(&d));
        act(&d, "loop_defer", json!({"num": "#1", "days": cur})).unwrap();
        let (l, n, _) = defer_of(&st(&d));
        assert_eq!((l.as_str(), n), (label, days));
    }
    let (_, _, card) = defer_of(&st(&d));
    assert!(card["lines"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l.as_str().unwrap().contains("已延 3 次")));
}

// Rust 才有（F-50）：state 帶健康指標（同 ctx health）
#[test]
fn state_carries_health() {
    let (_t, d) = spine_with_repos();
    ev(
        &d,
        "open-loop",
        "agent",
        &["#1", "group:g1"],
        "opened → 補測試",
        ago(60),
    );
    let s = st(&d);
    assert_eq!(s["health"]["days"], 14);
    assert_eq!(s["health"]["loops"]["opened"], 1);
}

// ── 2026-10-11：加入資料夾＋檔案清單（Rust 才有）──

/// spine 底下每個檔的（相對路徑, 內容）——比對「唯讀 action 沒寫任何東西」。
fn spine_files(d: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                out.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(d, d, &mut out);
    out.sort();
    out
}

/// 標成唯讀的 action（殼據此不發 changed）真的不寫 spine；名稱都是存在的 action。
#[test]
fn read_only_actions_leave_spine_untouched() {
    let (_t, d) = spine_with_repos();
    act(&d, "note_add", json!({"group": "g1", "text": "先記一筆"})).unwrap();
    let names: Vec<&str> = ACTIONS.iter().map(|(n, _)| *n).collect();
    let calls = [
        ("repo_tree", json!({"id": "repo-a"})),
        ("read_file", json!({"id": "repo-a", "path": "README.md"})),
        ("group_notes", json!({"group": "g1"})),
        ("repo_profile", json!({"id": "repo-a"})),
        ("palette", json!({"query": "rep"})),
        ("repo_changes", json!({"id": "repo-b"})),
        ("scope_changes", json!({"group": "g1"})),
        ("review_brief", json!({"id": "repo-b"})),
        ("file_diff", json!({"id": "repo-b", "path": "wip0.md"})),
        (
            "pick_brief",
            json!({"files": [{"id": "repo-a", "path": "README.md"}]}),
        ),
        ("scan_dir", json!({"dir": d.parent().unwrap()})),
        // 本機規則對得上：不問 provider、不落事件
        ("assist_plan", json!({"text": "切到 g1"})),
        ("assist_recent", json!({})),
        ("backlinks", json!({"id": "repo-a", "path": "README.md"})),
    ];
    let before = spine_files(&d);
    for (name, payload) in calls {
        assert!(is_read_only(name), "{name}");
        assert!(names.contains(&name), "{name}");
        act(&d, name, payload).unwrap();
    }
    assert_eq!(spine_files(&d), before);
    assert!(!is_read_only("note_add") && !is_read_only("write_file") && !is_read_only("nope"));
}

#[test]
fn repo_tree_lists_one_level_by_repo_id() {
    let (_t, d) = spine_with_repos();
    let r = act(&d, "repo_tree", json!({"id": "repo-a"})).unwrap();
    let names: Vec<(&str, bool)> = r["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["name"].as_str().unwrap(), e["dir"].as_bool().unwrap()))
        .collect();
    assert_eq!(names, vec![("notes", true), ("README.md", false)]);
    let sub = act(&d, "repo_tree", json!({"id": "repo-a", "dir": "notes"})).unwrap();
    let e = &sub["entries"][0];
    assert_eq!(
        e["path"],
        json!(Path::new("notes").join("plan.md").to_string_lossy())
    );
    let root = registry::get_repo(&d, "repo-a").unwrap().path();
    assert_eq!(
        e["abs"],
        json!(root.join("notes").join("plan.md").to_string_lossy())
    );
    assert!(err(&d, "repo_tree", json!({"id": "repo-a", "dir": ".."})).contains("repo 裡"));
    assert!(act(&d, "repo_tree", json!({"id": "nope"})).is_err());
}

#[test]
fn scan_dir_offers_the_folder_itself_when_it_is_a_repo() {
    let (t, d) = spine_with_repos();
    let solo = make_git_repo(t.path(), "solo", 1, 0, false);
    let r = act(&d, "scan_dir", json!({"dir": solo.to_string_lossy()})).unwrap();
    assert_eq!(r["candidates"][0]["id"], json!("solo"));
    // 已登記的本身不再列
    let a = registry::get_repo(&d, "repo-a").unwrap().path();
    let r = act(&d, "scan_dir", json!({"dir": a.to_string_lossy()})).unwrap();
    assert_eq!(r["candidates"], json!([]));
}

#[test]
fn group_add_folder_registers_once_and_joins_group() {
    let (t, d) = spine_with_repos();
    let solo = make_git_repo(t.path(), "solo", 1, 0, false);
    let path = solo.to_string_lossy().to_string();
    let r = act(&d, "group_add_folder", json!({"path": path, "group": "g1"})).unwrap();
    assert_eq!(
        (r["id"].as_str(), r["registered"].as_bool()),
        (Some("solo"), Some(true))
    );
    assert!(registry::get_group(&d, "g1")
        .unwrap()
        .members()
        .contains(&"solo".to_string()));
    // 再加一次：不重登記、不重複成員
    let again = act(&d, "group_add_folder", json!({"path": path, "group": "g1"})).unwrap();
    assert_eq!(again["registered"], json!(false));
    let members = registry::get_group(&d, "g1").unwrap().members();
    assert_eq!(members.iter().filter(|m| *m == "solo").count(), 1);
    // 不是 git 的資料夾：下一層有 repo 時指向掃描
    let msg = err(
        &d,
        "group_add_folder",
        json!({"path": t.path().to_string_lossy()}),
    );
    assert!(msg.contains("掃描資料夾"), "{msg}");
}

#[test]
fn remove_repo_unregisters_but_keeps_files() {
    let (_t, d) = spine_with_repos();
    let a = registry::get_repo(&d, "repo-a").unwrap().path();
    act(&d, "remove_repo", json!({"id": "repo-a"})).unwrap();
    assert!(registry::get_repo(&d, "repo-a").is_err());
    assert!(!registry::get_group(&d, "g1")
        .unwrap()
        .members()
        .contains(&"repo-a".to_string()));
    assert!(a.join("README.md").exists(), "移出工作台不刪檔");
}

#[test]
fn reorder_groups_members_and_repos_only_permutes() {
    let (_t, d) = spine_with_repos();
    registry::add_group(&d, "g2", &["repo-b"]).unwrap();
    act(
        &d,
        "reorder",
        json!({"kind": "groups", "order": ["g2", "g1"]}),
    )
    .unwrap();
    let names: Vec<String> = registry::load(&d)
        .unwrap()
        .groups
        .iter()
        .map(|g| g.name())
        .collect();
    assert_eq!(names, vec!["g2", "g1"]);
    // state 的牌組順序跟著變（側欄就是照這個畫）
    let s = st(&d);
    assert_eq!(s["groups"][0]["name"], json!("g2"));

    act(
        &d,
        "reorder",
        json!({"kind": "members", "group": "g1", "order": ["repo-b", "repo-a"]}),
    )
    .unwrap();
    assert_eq!(
        registry::get_group(&d, "g1").unwrap().members(),
        vec!["repo-b", "repo-a"]
    );
    act(
        &d,
        "reorder",
        json!({"kind": "repos", "order": ["repo-b", "repo-a"]}),
    )
    .unwrap();
    assert_eq!(st(&d)["repos"], json!(["repo-b", "repo-a"]));

    // 不是同一批（少一個或多一個）就拒絕，不改
    assert!(err(
        &d,
        "reorder",
        json!({"kind": "members", "group": "g1", "order": ["repo-a"]})
    )
    .contains("變動"));
    assert!(err(
        &d,
        "reorder",
        json!({"kind": "groups", "order": ["g1", "g2", "g3"]})
    )
    .contains("變動"));
    assert_eq!(
        registry::get_group(&d, "g1").unwrap().members(),
        vec!["repo-b", "repo-a"]
    );
    // 排序不記 spine 事件
    assert!(of_type(&d, EventType::Decision)
        .iter()
        .all(|e| !e.body.contains("排序")));
}

#[test]
fn read_file_action_returns_text_and_guards_paths() {
    let (_t, d) = spine_with_repos();
    let r = act(
        &d,
        "read_file",
        json!({"id": "repo-a", "path": "notes/plan.md"}),
    )
    .unwrap();
    assert_eq!(r["kind"], json!("text"));
    assert!(r["text"].as_str().unwrap().contains("LRU"));
    assert_eq!(r["truncated"], json!(false));
    assert!(err(
        &d,
        "read_file",
        json!({"id": "repo-a", "path": "../repo-b/README.md"})
    )
    .contains("repo 裡"));
    assert!(act(&d, "read_file", json!({"id": "repo-a"})).is_err());
}

#[test]
fn write_file_action_saves_and_reports_conflict() {
    let (_t, d) = spine_with_repos();
    let read = |d: &std::path::Path| {
        act(
            d,
            "read_file",
            json!({"id": "repo-a", "path": "notes/plan.md"}),
        )
        .unwrap()
    };
    let r = read(&d);
    assert_eq!(r["editable"], json!(true));
    let v1 = r["version"].clone();
    let w = act(
        &d,
        "write_file",
        json!({"id": "repo-a", "path": "notes/plan.md", "text": "改過了\n", "base": v1}),
    )
    .unwrap();
    assert_eq!(w["ok"], json!(true));
    assert_eq!(w["text"], json!("改過了\n"));
    assert_ne!(w["version"], v1);
    // 拿舊 version 再存＝別人改過了：不寫、回 conflict（不是錯誤）
    let c = act(
        &d,
        "write_file",
        json!({"id": "repo-a", "path": "notes/plan.md", "text": "蓋掉\n", "base": v1}),
    )
    .unwrap();
    assert_eq!(c["conflict"], json!(true));
    assert_eq!(read(&d)["text"], json!("改過了\n"));
    // 不記 spine 事件
    assert!(!spine::iter_events(&d)
        .unwrap()
        .iter()
        .any(|e| e.body.contains("plan.md")));
    assert!(act(
        &d,
        "write_file",
        json!({"id": "repo-a", "path": "notes/plan.md"})
    )
    .is_err());
}

// ── 2026-10-12：牌組筆記（Rust 才有）──

#[test]
fn group_notes_add_list_remove_and_follow_rename() {
    let (_t, d) = spine_with_repos();
    let r = act(&d, "group_notes", json!({"group": "g1"})).unwrap();
    assert_eq!(r["notes"], json!([]));
    let r = act(
        &d,
        "note_add",
        json!({"group": "g1", "text": "問設計 *規格*"}),
    )
    .unwrap();
    let n = r["notes"][0].clone();
    assert_eq!(n["text"], json!("問設計 *規格*"));
    let file = d.join("groups").join("g1").join("notes.md");
    assert_eq!(r["path"], json!(file.to_string_lossy()));
    assert!(std::fs::read_to_string(&file).unwrap().contains("問設計"));
    // 不是脊椎事件：不進收件匣
    assert!(!spine::iter_events(&d)
        .unwrap()
        .iter()
        .any(|e| e.body.contains("問設計")));

    // 改組名：筆記跟著組走
    act(
        &d,
        "update_group",
        json!({"name": "g1", "new_name": "g2", "repos": ["repo-a", "repo-b"]}),
    )
    .unwrap();
    let r = act(&d, "group_notes", json!({"group": "g2"})).unwrap();
    assert_eq!(r["notes"][0]["text"], json!("問設計 *規格*"));

    assert!(err(
        &d,
        "note_remove",
        json!({"group": "g2", "at": n["at"], "text": "改過了"})
    )
    .contains("不在了"));
    let r = act(
        &d,
        "note_remove",
        json!({"group": "g2", "at": n["at"], "text": n["text"]}),
    )
    .unwrap();
    assert_eq!(r["notes"], json!([]));
    assert!(err(&d, "note_add", json!({"group": "", "text": "x"})).contains("牌組"));
}

// ── 2026-10-13：原本前端算的判斷搬回引擎 ──

/// 記下 create_agent 收到的 home_group。
struct HomeTerms(std::sync::Mutex<Vec<Option<String>>>);

impl Terms for HomeTerms {
    fn info(&self, _: &str) -> Option<TermInfo> {
        None
    }
    fn create_shell(
        &self,
        _: Option<&Path>,
        _: Option<String>,
    ) -> ctx_core::Result<(String, String)> {
        Ok(("t9".into(), "shell".into()))
    }
    fn create_agent(&self, l: AgentLaunch) -> ctx_core::Result<(String, String)> {
        self.0.lock().unwrap().push(l.home_group);
        Ok(("t8".into(), "agent".into()))
    }
    fn kill(&self, _: &str) -> bool {
        false
    }
}

#[test]
fn term_create_infers_home_group_for_single_repo() {
    let (_t, d) = spine_with_repos();
    let terms = HomeTerms(Default::default());
    let open = |p: Value| handle_action(&d, "term_create", &p, Some(&terms)).unwrap();
    // 只有 g1 有 repo-a：沒給提示也歸 g1
    open(json!({"kind": "agent", "repo": "repo-a"}));
    // 提示的組沒有這個 repo：不採用，照唯一擁有者
    open(json!({"kind": "agent", "repo": "repo-a", "home_group": "沒有這組"}));
    ctx_core::registry::add_group(&d, "g2", &["repo-a"]).unwrap();
    // 兩組都有：照提示；沒提示就不歸類
    open(json!({"kind": "agent", "repo": "repo-a", "home_group": "g2"}));
    open(json!({"kind": "agent", "repo": "repo-a"}));
    assert_eq!(
        *terms.0.lock().unwrap(),
        vec![
            Some("g1".into()),
            Some("g1".into()),
            Some("g2".into()),
            None
        ]
    );
}

/// 記下 create_shell 收到的 (cwd, origin)；只有 t1 知道目錄。
struct CwdTerms(std::sync::Mutex<Vec<(Option<std::path::PathBuf>, Option<String>)>>);

impl Terms for CwdTerms {
    fn info(&self, _: &str) -> Option<TermInfo> {
        None
    }
    fn create_shell(
        &self,
        cwd: Option<&Path>,
        origin: Option<String>,
    ) -> ctx_core::Result<(String, String)> {
        self.0
            .lock()
            .unwrap()
            .push((cwd.map(Path::to_path_buf), origin));
        Ok(("t9".into(), "shell".into()))
    }
    fn create_agent(&self, _: AgentLaunch) -> ctx_core::Result<(String, String)> {
        Ok(("t8".into(), "agent".into()))
    }
    fn kill(&self, _: &str) -> bool {
        false
    }
    fn cwd(&self, sid: &str) -> Option<std::path::PathBuf> {
        (sid == "t1").then(|| "/work/repo-a".into())
    }
}

/// 某個會話的 Terminal：沿用它的目錄、origin 記 `shell-of:<sid>`；未知會話拒絕。
#[test]
fn term_create_shell_of_session_uses_its_cwd() {
    let (_t, d) = spine_with_repos();
    let terms = CwdTerms(Default::default());
    let r = handle_action(
        &d,
        "term_create",
        &json!({"kind": "shell", "of": "t1"}),
        Some(&terms),
    )
    .unwrap();
    assert_eq!(r["sid"], "t9");
    assert_eq!(
        *terms.0.lock().unwrap(),
        vec![(Some("/work/repo-a".into()), Some("shell-of:t1".into()))]
    );
    let e = handle_action(
        &d,
        "term_create",
        &json!({"kind": "shell", "of": "t7"}),
        Some(&terms),
    )
    .unwrap_err()
    .to_string();
    assert_eq!(e, "未知會話: t7");
}

#[test]
fn state_marks_primary_action_and_gone_scope() {
    let (_t, d) = spine_with_repos();
    ev(
        &d,
        "open-loop",
        "agent",
        &["#1", "group:g1"],
        "確認部署條件",
        ago(5),
    );
    ev(
        &d,
        "collision",
        "engine",
        &["id:x1"],
        "判定失敗（system-unsure，准打斷）：boom",
        ago(4),
    );
    let s = st(&d);
    assert!(s["cards"].as_array().unwrap().len() >= 2);
    for c in s["cards"].as_array().unwrap() {
        let acts = c["actions"].as_array().unwrap();
        let n = acts.iter().filter(|a| a["primary"] == json!(true)).count();
        assert!(n <= 1, "一張卡最多一個主要動作：{c}");
        if acts.iter().any(|a| {
            !["ignore", "defer", "route_pick"].contains(&a["action"].as_str().unwrap_or(""))
        }) {
            assert_eq!(n, 1, "{c}");
        }
    }
    assert_eq!(st_in(&d, Some("g1"), None)["scope"]["gone"], json!(false));
    assert_eq!(
        st_in(&d, Some("刪掉的組"), None)["scope"]["gone"],
        json!(true)
    );
    let gone = ["不存在".to_string()];
    assert_eq!(st_in(&d, None, Some(&gone))["scope"]["gone"], json!(true));
}

#[test]
fn actions_that_change_scan_say_rescan() {
    let (_t, d) = spine_with_repos();
    let r = act(&d, "tier", json!({"id": "repo-a", "tier": "dormant"})).unwrap();
    assert_eq!(r["rescan"], json!(true));
    let r = act(&d, "group_notes", json!({"group": "g1"})).unwrap();
    assert!(r.get("rescan").is_none());
}

// ── F-57：App 看與改名片（Rust 才有）──

#[test]
fn repo_profile_falls_back_to_readme_and_describe_writes_only_changes() {
    let (_t, d) = spine_with_repos();
    let root = registry::get_repo(&d, "repo-a").unwrap().path();
    std::fs::write(root.join("README.md"), "# A\n\n讀書筆記與卡片的知識庫。\n").unwrap();
    let p = act(&d, "repo_profile", json!({"id": "repo-a"})).unwrap();
    assert_eq!(p["description"], "讀書筆記與卡片的知識庫。");
    assert_eq!(p["description_source"], "readme");
    assert!(p.get("role").is_none());
    assert!(act(&d, "repo_profile", json!({"id": "nope"})).is_err());

    let audits = || of_type(&d, EventType::Decision).len();
    let n0 = audits();
    let r = act(
        &d,
        "describe_repo",
        json!({"id": "repo-a", "description": " 知識庫：書摘與卡片 ", "role": "知識庫",
               "keywords": "書摘、卡片，閱讀, 書摘\n筆記"}),
    )
    .unwrap();
    assert_eq!(r["changed"], true);
    assert_eq!(r["description"], "知識庫：書摘與卡片");
    assert_eq!(r["description_source"], "registry");
    assert_eq!(r["role"], "知識庫");
    assert_eq!(r["keywords"], json!(["書摘", "卡片", "閱讀", "筆記"]));
    let n1 = audits();
    assert_eq!(n1, n0 + 1, "名片改動要留痕");

    // 原樣再存＝不寫、不記
    let same = act(
        &d,
        "describe_repo",
        json!({"id": "repo-a", "description": "知識庫：書摘與卡片", "role": "知識庫",
               "keywords": "書摘, 卡片, 閱讀, 筆記"}),
    )
    .unwrap();
    assert_eq!(same["changed"], false);
    assert_eq!(audits(), n1);

    // 只改角色：事件只寫角色；清空關鍵字
    act(
        &d,
        "describe_repo",
        json!({"id": "repo-a", "description": "知識庫：書摘與卡片", "role": "規格來源", "keywords": ""}),
    )
    .unwrap();
    let last = of_type(&d, EventType::Decision).pop().unwrap();
    assert!(last.body.contains("角色＝規格來源"), "{}", last.body);
    assert!(!last.body.contains("介紹＝"), "{}", last.body);
    assert!(registry::get_repo(&d, "repo-a")
        .unwrap()
        .keywords()
        .is_empty());

    // 清掉介紹 → 回到 README 推測
    let back = act(
        &d,
        "describe_repo",
        json!({"id": "repo-a", "description": ""}),
    )
    .unwrap();
    assert_eq!(back["description_source"], "readme");
    assert!(err(&d, "describe_repo", json!({"id": "nope", "role": "x"})).contains("nope"));
}

#[test]
fn palette_finds_repos_by_partial_name_and_profile_and_groups() {
    let (_t, d) = spine_with_repos();
    act(
        &d,
        "describe_repo",
        json!({"id": "repo-b", "description": "書摘與卡片", "role": "知識庫", "keywords": ""}),
    )
    .unwrap();
    act(&d, "set_goal", json!({"group": "g1", "text": "把登入做完"})).unwrap();
    let ids = |q: &str| -> Vec<String> {
        act(&d, "palette", json!({"query": q})).unwrap()["repos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["repo"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(ids(""), ["repo-a", "repo-b"]);
    assert_eq!(ids("rep"), ["repo-a", "repo-b"]);
    assert_eq!(ids("知識"), ["repo-b"]);
    assert_eq!(ids("-b"), ["repo-b"]);
    let r = act(&d, "palette", json!({"query": "知識"})).unwrap();
    assert_eq!(r["repos"][0]["groups"], json!(["g1"]));
    assert_eq!(r["repos"][0]["role"], "知識庫");
    assert!(r["groups"].as_array().unwrap().is_empty());
    let g = act(&d, "palette", json!({"query": "登入"})).unwrap();
    assert_eq!(g["groups"][0]["name"], "g1");
    assert_eq!(g["groups"][0]["members"], 2);
    assert_eq!(
        act(&d, "palette", json!({})).unwrap()["groups"][0]["name"],
        "g1"
    );

    // 檔案：兩個字以上才找；照範圍
    let f = act(&d, "palette", json!({"query": "plan"})).unwrap();
    let files = f["files"].as_array().unwrap();
    // 兩個 repo 都有 notes/plan.md：同分照 registry 順序
    assert_eq!(files.len(), 2, "{files:?}");
    assert_eq!(files[0]["repo"], "repo-a");
    assert_eq!(files[1]["repo"], "repo-b");
    assert_eq!(files[0]["path"], "notes/plan.md");
    let root = registry::get_repo(&d, "repo-a").unwrap().path();
    assert_eq!(
        files[0]["abs"],
        json!(root.join("notes/plan.md").to_string_lossy())
    );
    assert!(act(&d, "palette", json!({"query": "p"})).unwrap()["files"]
        .as_array()
        .unwrap()
        .is_empty());
    let scoped = act(
        &d,
        "palette",
        json!({"query": "readme", "repos": ["repo-b"]}),
    )
    .unwrap();
    let repos: Vec<&str> = scoped["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["repo"].as_str().unwrap())
        .collect();
    assert_eq!(repos, ["repo-b"]);
    let by_group = act(&d, "palette", json!({"query": "readme", "group": "g1"})).unwrap();
    assert_eq!(by_group["files"].as_array().unwrap().len(), 2);
}

// ── 挑幾份檔案交給 agent（痛點 2，Rust 才有）──

#[test]
fn pick_brief_groups_by_repo_with_profile_and_rejects_escapes() {
    let (_t, d) = spine_with_repos();
    act(
        &d,
        "describe_repo",
        json!({"id": "repo-a", "description": "讀書筆記", "role": "知識庫"}),
    )
    .unwrap();
    let ra = registry::get_repo(&d, "repo-a").unwrap().path();
    let rb = registry::get_repo(&d, "repo-b").unwrap().path();
    let plan = Path::new("notes").join("plan.md");
    let r = act(
        &d,
        "pick_brief",
        json!({"files": [
            {"id": "repo-a", "path": "README.md"},
            {"id": "repo-b", "path": "wip0.md"},
            {"id": "repo-a", "path": plan},
            {"id": "repo-a", "path": "README.md"},
        ]}),
    )
    .unwrap();
    assert_eq!(r["repos"], json!(["repo-a", "repo-b"]));
    assert_eq!(r["count"], 3);
    let text = r["text"].as_str().unwrap();
    let a_readme = ra.join("README.md").to_string_lossy().into_owned();
    let a_plan = ra.join(&plan).to_string_lossy().into_owned();
    let b_wip = rb.join("wip0.md").to_string_lossy().into_owned();
    assert!(text.contains("## repo-a（知識庫） — 讀書筆記\n"), "{text}");
    let (ia, ip, ib) = (
        text.find(&a_readme).unwrap(),
        text.find(&a_plan).unwrap(),
        text.find(&b_wip).unwrap(),
    );
    assert!(ia < ip && ip < ib, "照 repo 分段、段內照挑的順序：{text}");
    assert_eq!(text.matches(&a_readme).count(), 1);
    assert_eq!(r["mentions"], format!("@{a_readme}\n@{a_plan}\n@{b_wip}"));

    assert!(err(&d, "pick_brief", json!({"files": []})).contains("還沒挑"));
    assert!(err(
        &d,
        "pick_brief",
        json!({"files": [{"id": "repo-a", "path": "../x"}]})
    )
    .contains("repo 裡"));
    assert!(err(
        &d,
        "pick_brief",
        json!({"files": [{"id": "nope", "path": "a"}]})
    )
    .contains("nope"));
    assert!(err(
        &d,
        "pick_brief",
        json!({"files": [{"id": "repo-a", "path": "ghost.md"}]})
    )
    .contains("不存在"));
    let many: Vec<Value> = (0..51)
        .map(|_| json!({"id": "repo-a", "path": "README.md"}))
        .collect();
    assert!(err(&d, "pick_brief", json!({ "files": many })).contains("最多 50"));
}

#[test]
fn profile_draft_brief_lists_undescribed_and_known_roles() {
    let (_t, d) = spine_with_repos();
    act(
        &d,
        "describe_repo",
        json!({"id": "repo-a", "description": "讀書筆記", "role": "知識庫"}),
    )
    .unwrap();
    let r = act(&d, "profile_draft_brief", json!({})).unwrap();
    assert_eq!(r["repos"], json!(["repo-b"]));
    let text = r["text"].as_str().unwrap();
    let rb = registry::get_repo(&d, "repo-b").unwrap().path();
    assert!(
        text.contains(&format!("- repo-b：{}", rb.to_string_lossy())),
        "{text}"
    );
    assert!(
        text.contains("README 第一句：這個 repo 談 widget 快取設計。"),
        "{text}"
    );
    assert!(
        text.contains("已經用過的角色（能沿用就沿用，用詞一致才好找）：知識庫"),
        "{text}"
    );
    assert!(text.contains("只讀，不改 repo 裡任何檔"), "{text}");
    assert!(
        text.contains("ctx registry describe <id> --description \"…\""),
        "{text}"
    );
    assert!(!text.contains("- repo-a："), "{text}");

    // 指定 id（就算寫過也照起草）；不存在的拒絕；都寫了就說沒有
    let one = act(&d, "profile_draft_brief", json!({"ids": ["repo-a"]})).unwrap();
    assert_eq!(one["repos"], json!(["repo-a"]));
    assert!(err(&d, "profile_draft_brief", json!({"ids": ["nope"]})).contains("nope"));
    act(
        &d,
        "describe_repo",
        json!({"id": "repo-b", "description": "快取"}),
    )
    .unwrap();
    assert!(err(&d, "profile_draft_brief", json!({})).contains("都寫了介紹"));
}

// ── 2026-10-14：會話狀態由 Claude Code hook 自動寫（Rust 才有）──

struct ArgvTerms(std::sync::Mutex<Vec<Vec<String>>>);

impl Terms for ArgvTerms {
    fn info(&self, _: &str) -> Option<TermInfo> {
        None
    }
    fn create_shell(
        &self,
        _: Option<&Path>,
        _: Option<String>,
    ) -> ctx_core::Result<(String, String)> {
        Ok(("t9".into(), "shell".into()))
    }
    fn create_agent(&self, l: AgentLaunch) -> ctx_core::Result<(String, String)> {
        self.0.lock().unwrap().push(l.argv);
        Ok(("t8".into(), "agent".into()))
    }
    fn kill(&self, _: &str) -> bool {
        false
    }
}

#[test]
fn term_create_gives_claude_code_the_session_hooks() {
    let (_t, d) = spine_with_repos();
    let terms = ArgvTerms(Default::default());
    handle_action(
        &d,
        "term_create",
        &json!({"kind": "agent", "repo": "repo-a"}),
        Some(&terms),
    )
    .unwrap();
    let argv = terms.0.lock().unwrap()[0].clone();
    let settings = d.join(".state").join("session_hooks.json");
    assert_eq!(argv[0], "claude");
    assert_eq!(
        argv[1],
        format!("--settings={}", settings.display()),
        "{argv:?}"
    );
    let v: Value = serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
    assert!(v["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .ends_with("hook stop"));
}

#[test]
fn session_hook_status_shows_on_its_own_session_and_newer_wins() {
    use ctx_core::session_status::{path_for, record_hook, StatusHook};
    use ctx_core::status::{save_status, StatusRecord};
    let (_t, d) = spine_with_repos();
    let started = (Local::now() - Duration::minutes(5))
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let t1 = json!({"sid": "t1", "kind": "agent", "scope": "g1", "started": started});
    let t2 = json!({"sid": "t2", "kind": "agent", "scope": "g1", "started": started});
    let status_of =
        |t: &Value| annotate_terms(&d, std::slice::from_ref(t)).unwrap()[0]["agent_status"].clone();
    let target = path_for(&d, "t1").to_string_lossy().into_owned();
    let now = Local::now().naive_local();
    assert!(record_hook(
        StatusHook::Stop,
        "{}",
        Some(&target),
        Some(now)
    ));
    // 只掛在自己的會話上（同範圍的另一個會話看不到）
    assert_eq!(status_of(&t1)["status"], "finished");
    assert_eq!(status_of(&t1)["text"], "這一輪做完了");
    assert!(status_of(&t2).is_null());
    // agent 自己回報、時間較早：hook 的比較新就用 hook 的
    let rec = |at: String| StatusRecord {
        status: "blocked".into(),
        text: "缺 API key".into(),
        at,
        scope: "g1".into(),
        group: Some("g1".into()),
        repos: None,
        repo: None,
    };
    save_status(
        &d,
        &rec((now - Duration::minutes(2))
            .format("%Y-%m-%d %H:%M")
            .to_string()),
    )
    .unwrap();
    assert_eq!(status_of(&t1)["status"], "finished");
    assert_eq!(status_of(&t2)["status"], "blocked");
    // 同一分鐘：取 agent 自己回報的
    save_status(&d, &rec(now.format("%Y-%m-%d %H:%M").to_string())).unwrap();
    assert_eq!(status_of(&t1)["status"], "blocked");
    // 會話開始前的 hook 狀態不算
    let old = json!({"sid": "t1", "kind": "agent", "scope": "repo-b",
                     "started": (now + Duration::minutes(10)).format("%Y-%m-%d %H:%M").to_string()});
    assert!(status_of(&old).is_null());
    // F-71：標題與最後回覆跟著自己的會話走，agent 自己回報蓋過狀態時也還在
    let tr = d.join("x.jsonl");
    std::fs::write(
        &tr,
        "{\"type\":\"ai-title\",\"aiTitle\":\"修登入\"}\n\
         {\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"好了\"}]}}\n",
    )
    .unwrap();
    let payload = json!({"transcript_path": tr}).to_string();
    assert!(record_hook(
        StatusHook::Stop,
        &payload,
        Some(&target),
        Some(now)
    ));
    let rows = annotate_terms(&d, &[t1.clone(), t2.clone(), old.clone()]).unwrap();
    assert_eq!(
        (rows[0]["ai_title"].as_str(), rows[0]["last_reply"].as_str()),
        (Some("修登入"), Some("好了"))
    );
    assert!(rows[1]["ai_title"].is_null() && rows[2]["ai_title"].is_null());
}

/// F-82：處理中帶正在用的工具與分鐘數；超過 10 分沒結束＝可能卡住；工具結束就沒有。
#[test]
fn annotate_terms_shows_the_tool_in_progress_and_flags_stalls() {
    use ctx_core::session_status::{path_for, record_hook, StatusHook};
    let (_t, d) = spine_with_repos();
    let started = (Local::now() - Duration::minutes(30))
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let t1 = json!({"sid": "t1", "kind": "agent", "alive": true, "scope": "repo-a",
                    "scope_repos": ["repo-a"], "started": started});
    let target = path_for(&d, "t1").to_string_lossy().into_owned();
    let start = r#"{"tool_name":"Bash","tool_input":{"command":"cargo build"}}"#;
    let twelve_ago = (Local::now() - Duration::minutes(12)).naive_local();
    assert!(record_hook(
        StatusHook::ToolStart,
        start,
        Some(&target),
        Some(twelve_ago)
    ));
    let row = &annotate_terms(&d, std::slice::from_ref(&t1)).unwrap()[0];
    assert_eq!(row["tool"]["label"], "Bash：cargo build");
    assert!(row["tool"]["minutes"].as_i64().unwrap() >= 12);
    assert_eq!(row["tool"]["stalled"], true);
    let now = Local::now().naive_local();
    assert!(record_hook(
        StatusHook::ToolStart,
        start,
        Some(&target),
        Some(now)
    ));
    let row = &annotate_terms(&d, std::slice::from_ref(&t1)).unwrap()[0];
    assert_eq!(row["tool"]["stalled"], false);
    assert!(record_hook(
        StatusHook::ToolEnd,
        start,
        Some(&target),
        Some(now)
    ));
    let row = &annotate_terms(&d, std::slice::from_ref(&t1)).unwrap()[0];
    assert!(row["tool"].is_null());
    assert_eq!(row["agent_status"]["text"], "處理中");
}

#[test]
fn session_attention_counts_live_agents_waiting_blocked_or_unseen_finished() {
    use ctx_core::session_status::{mark_seen, path_for, record_hook, StatusHook};
    use ctx_core::workbench::{session_attention, sessions_needing_you, Attention};
    let (_t, d) = spine_with_repos();
    let started = (Local::now() - Duration::minutes(5))
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let t = |sid: &str, alive: bool, kind: &str| {
        json!({"sid": sid, "kind": kind, "alive": alive, "scope": "repo-a",
               "scope_repos": ["repo-a"], "started": started})
    };
    let now = Some(Local::now().naive_local());
    for (sid, ev) in [
        ("t1", StatusHook::Stop),
        ("t2", StatusHook::Prompt),
        ("t3", StatusHook::Permission),
        ("t4", StatusHook::Stop),
        ("t6", StatusHook::Stop),
        ("t7", StatusHook::Notification),
    ] {
        let p = path_for(&d, sid).to_string_lossy().into_owned();
        assert!(record_hook(ev, "{}", Some(&p), now));
    }
    assert!(mark_seen(&d, "t6"));
    let terms = [
        t("t1", true, "agent"),  // 做完沒看
        t("t2", true, "agent"),  // 處理中
        t("t3", true, "agent"),  // 要你允許
        t("t4", false, "agent"), // 已結束
        t("t5", true, "shell"),
        t("t6", true, "agent"), // 做完看過了
        t("t7", true, "agent"), // 閒著等你
    ];
    assert_eq!(
        session_attention(&d, &terms).unwrap(),
        Attention {
            needs_you: 2,
            finished_unseen: 1,
            working: 1,
        }
    );
    assert_eq!(sessions_needing_you(&d, &terms).unwrap(), 2);
    // 看過之後徽章少一個；再做完一輪又算沒看
    assert!(mark_seen(&d, "t1"));
    assert_eq!(session_attention(&d, &terms).unwrap().finished_unseen, 0);
    let p = path_for(&d, "t1").to_string_lossy().into_owned();
    assert!(record_hook(StatusHook::Stop, "{}", Some(&p), now));
    assert_eq!(session_attention(&d, &terms).unwrap().finished_unseen, 1);
}

#[test]
fn sessions_asking_permission_lists_only_hook_permission_requests() {
    use ctx_core::session_status::{path_for, record_hook, StatusHook};
    use ctx_core::workbench::sessions_asking_permission;
    let (_t, d) = spine_with_repos();
    let started = (Local::now() - Duration::minutes(5))
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let t = |sid: &str, alive: bool| {
        json!({"sid": sid, "title": format!("claude · {sid}"), "kind": "agent", "alive": alive,
               "scope": "repo-a", "scope_repos": ["repo-a"], "started": started})
    };
    let now = Some(Local::now().naive_local());
    let perm = r#"{"message":"Claude needs your permission to use Bash"}"#;
    for (sid, ev, stdin) in [
        ("t1", StatusHook::Notification, perm),
        ("t2", StatusHook::Stop, "{}"),
        ("t3", StatusHook::Notification, perm),
        ("t4", StatusHook::Permission, r#"{"message":"Allow Edit?"}"#),
    ] {
        let p = path_for(&d, sid).to_string_lossy().into_owned();
        assert!(record_hook(ev, stdin, Some(&p), now));
    }
    let asks = sessions_asking_permission(
        &d,
        &[t("t1", true), t("t2", true), t("t3", false), t("t4", true)],
    );
    assert_eq!(asks.len(), 2);
    assert_eq!(asks[1]["text"], "Allow Edit?");
    assert_eq!(asks[0]["sid"], "t1");
    assert_eq!(asks[0]["title"], "claude · t1");
    assert_eq!(asks[0]["text"], "Claude needs your permission to use Bash");
}

#[test]
fn repo_changes_and_file_diff_via_actions() {
    let (_t, d) = spine_with_repos();
    let r = act(&d, "repo_changes", json!({"id": "repo-b"})).unwrap();
    let paths: Vec<(&str, &str)> = r["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["path"].as_str().unwrap(), c["status"].as_str().unwrap()))
        .collect();
    assert_eq!(paths, [("wip0.md", "untracked"), ("wip1.md", "untracked")]);
    let df = act(&d, "file_diff", json!({"id": "repo-b", "path": "wip0.md"})).unwrap();
    assert!(df["diff"].as_str().unwrap().contains("+"), "{df}");
    assert_eq!(df["truncated"], false);
    assert!(act(&d, "repo_changes", json!({"id": "nope"})).is_err());
    assert!(err(&d, "file_diff", json!({"id": "repo-b", "path": "../x"})).contains("repo 裡"));
}

#[test]
fn scope_changes_lists_only_repos_with_changes_in_scope_order() {
    let (_t, d) = spine_with_repos();
    let r = act(&d, "scope_changes", json!({"group": "g1"})).unwrap();
    let rows = r["repos"].as_array().unwrap();
    // repo-a 乾淨、repo-b 有兩個沒追蹤的草稿
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["id"], "repo-b");
    assert_eq!(rows[0]["changes"].as_array().unwrap().len(), 2);
    let ra = registry::get_repo(&d, "repo-a").unwrap().path();
    std::fs::write(ra.join("README.md"), "改了\n").unwrap();
    let r = act(&d, "scope_changes", json!({"repos": ["repo-b", "repo-a"]})).unwrap();
    let ids: Vec<&str> = r["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["repo-b", "repo-a"]);
    assert_eq!(r["repos"][1]["changes"][0]["status"], "modified");
    // 不是 git 的資料夾略過
    let plain = d.parent().unwrap().join("plainfolder");
    std::fs::create_dir_all(&plain).unwrap();
    registry::add_repo(&d, "plain", &plain, registry::AddRepo::default()).unwrap();
    let r = act(&d, "scope_changes", json!({"repos": ["plain"]})).unwrap();
    assert!(r["repos"].as_array().unwrap().is_empty());
}

#[test]
fn review_brief_lists_changes_and_forbids_editing() {
    let (_t, d) = spine_with_repos();
    let r = act(&d, "review_brief", json!({"id": "repo-b"})).unwrap();
    assert_eq!(r["count"], 2);
    let text = r["text"].as_str().unwrap();
    assert!(
        text.starts_with("審查 repo-b 還沒 commit 的變更（2 個檔）"),
        "{text}"
    );
    assert!(text.contains("- 新檔（未追蹤）：wip0.md"), "{text}");
    assert!(text.contains("先不要改任何檔、也不要 commit"), "{text}");
    assert!(text.contains("log_decision"), "{text}");
    // 乾淨的 repo：沒得審
    assert!(err(&d, "review_brief", json!({"id": "repo-a"})).contains("沒有還沒 commit 的變更"));
}

/// 組名有空白：事件標頭寫成 group:PM_Group，App 選「PM Group」時照樣算在範圍內。
#[test]
fn group_name_with_space_events_stay_in_scope_lane() {
    let (_t, d) = spine_with_repos();
    registry::add_group(&d, "PM Group", &["repo-b"]).unwrap();
    ev(
        &d,
        "decision",
        "agent",
        &["group:PM Group"],
        "組層級的判斷",
        ago(2),
    );
    let s = st_in(&d, Some("PM Group"), None);
    let rec = s["recent"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| {
            e["body"]
                .as_str()
                .is_some_and(|t| t.contains("組層級的判斷"))
        })
        .cloned()
        .unwrap_or_else(|| panic!("{}", s["recent"]));
    assert_eq!(rec["lane"], "scope", "{rec}");
    // 標籤用登記的原名
    assert_eq!(rec["scope_label"], "PM Group", "{rec}");
}

/// config 的 claude 是包裝指令（["npx", "claude"]）：--settings 插在整個指令之後，不插進中間。
#[test]
fn term_create_inserts_settings_after_wrapped_agent_command() {
    let (_t, d) = spine_with_repos();
    let cfg = d.join("config.yaml");
    let text = std::fs::read_to_string(&cfg).unwrap();
    std::fs::write(
        &cfg,
        format!("{text}\nagents:\n  claude:\n    cmd: [npx, claude]\n    mcp: mcp-config\n    prompt_flag: []\n"),
    )
    .unwrap();
    let terms = ArgvTerms(Default::default());
    handle_action(
        &d,
        "term_create",
        &json!({"kind": "agent", "repo": "repo-a"}),
        Some(&terms),
    )
    .unwrap();
    let argv = terms.0.lock().unwrap()[0].clone();
    assert_eq!(&argv[..2], ["npx", "claude"], "{argv:?}");
    assert!(argv[2].starts_with("--settings="), "{argv:?}");
}

/// F-63 App 助手：規則計畫照原樣給 App；執行完的紀錄落 presented [assist]（帶目前牌組）。
#[test]
fn assist_plan_and_log() {
    let (_t, d) = spine_with_repos();
    let p = act(
        &d,
        "assist_plan",
        json!({"text": "repo-b 加標籤 規格", "group": "g1"}),
    )
    .unwrap();
    assert_eq!(p["source"], json!("rules"));
    assert_eq!(p["steps"][0]["op"], json!("tag"));
    assert_eq!(p["steps"][0]["text"], json!("repo-b：加標籤「規格」"));
    assert!(act(&d, "assist_plan", json!({"text": "  "})).is_err());

    act(
        &d,
        "assist_log",
        json!({"text": "repo-b 加標籤 規格", "group": "g1",
               "done": ["repo-b：加標籤「規格」"], "skipped": []}),
    )
    .unwrap();
    let ev = spine::iter_events(&d).unwrap().pop().unwrap();
    assert_eq!(ev.source, "assist");
    assert_eq!(ev.kv("group"), Some("g1"));
    assert!(ev
        .body
        .starts_with("助手：repo-b 加標籤 規格\n做了：repo-b：加標籤「規格」"));
}

/// N-37「這組標為已讀」：只藏這個範圍、在標記之前的一般未讀；別組、全部、之後的新事件照舊；全域 last_seen 不動。
#[test]
fn ack_scope_hides_only_this_scope() {
    let (_t, d) = spine_with_repos();
    registry::add_group(&d, "only-b", &["repo-b"]).unwrap();
    ev(
        &d,
        "decision",
        "agent",
        &["repo:repo-a"],
        "a 的舊判斷",
        ago(30),
    );
    ev(
        &d,
        "decision",
        "agent",
        &["repo:repo-b"],
        "b 的舊判斷",
        ago(30),
    );
    let titles = |v: &Value| -> Vec<String> {
        cards_of(v, "event")
            .iter()
            .filter(|c| c["lane"] == "scope")
            .map(|c| c["title"].as_str().unwrap_or("").to_string())
            .collect()
    };
    let only_a = vec!["repo-a".to_string()];
    assert_eq!(titles(&st_in(&d, None, Some(&only_a))), vec!["a 的舊判斷"]);

    act(&d, "ack_scope", json!({"repos": ["repo-a"]})).unwrap();
    assert!(titles(&st_in(&d, None, Some(&only_a))).is_empty());
    // 別的範圍照舊
    assert_eq!(titles(&st_in(&d, Some("only-b"), None)), vec!["b 的舊判斷"]);
    assert_eq!(
        st(&d)["unread"].as_array().unwrap().len(),
        2,
        "全域未讀不動"
    );
    // 標記之後的新事件照樣出現
    ev(
        &d,
        "decision",
        "agent",
        &["repo:repo-a"],
        "a 的新判斷",
        Local::now().naive_local() + Duration::minutes(2),
    );
    assert_eq!(titles(&st_in(&d, None, Some(&only_a))), vec!["a 的新判斷"]);
    // 全部範圍不能用這個
    assert!(err(&d, "ack_scope", json!({})).contains("ack"));
}

// ── 2026-10-16：接續過去的 Claude Code 會話（F-72）──

#[test]
fn term_create_resume_runs_claude_resume_in_the_session_dir() {
    let (t, d) = spine_with_repos();
    let ra = ctx_core::registry::get_repo(&d, "repo-a").unwrap().path();
    let terms = ArgvTerms(Default::default());
    let out = handle_action(
        &d,
        "term_create",
        &json!({"kind": "agent", "resume": "aaaaaaaa-0001", "cwd": ra}),
        Some(&terms),
    )
    .unwrap();
    assert_eq!(out["sid"], "t8");
    let argv = terms.0.lock().unwrap()[0].clone();
    assert_eq!(argv[0], "claude");
    assert!(argv[1].starts_with("--settings="), "{argv:?}");
    assert_eq!(&argv[2..], ["--resume", "aaaaaaaa-0001"]);
    // 不在登記的 repo 底下、id 不對：拒絕
    for bad in [
        json!({"kind": "agent", "resume": "aaaaaaaa-0001", "cwd": t.path()}),
        json!({"kind": "agent", "resume": "x; rm -rf", "cwd": ra}),
    ] {
        assert!(handle_action(&d, "term_create", &bad, Some(&terms)).is_err());
    }
    // 清單是唯讀 action
    assert!(ctx_core::workbench::actions::is_read_only("past_sessions"));
    let r = handle_action(&d, "past_sessions", &json!({"group": "g1"}), None).unwrap();
    assert!(r["sessions"].is_array());
}

#[test]
fn diff_notes_brief_batches_line_notes_into_one_message() {
    let (_t, d) = spine_with_repos();
    let notes = json!([
        {"where": "第 12 行（新增）", "code": "+    let x = 1;", "note": "改名叫 count\n順便加註解"},
        {"where": "原第 3 行（刪除）", "code": "-old()", "note": "  "},
        {"where": "第 20 行", "code": " fn main() {", "note": "這裡要處理錯誤"},
    ]);
    let r = handle_action(
        &d,
        "diff_notes_brief",
        &json!({"id": "repo-a", "path": "src/a.rs", "notes": notes}),
        None,
    )
    .unwrap();
    assert_eq!(r["count"], 2);
    let text = r["text"].as_str().unwrap();
    assert!(
        text.contains("src/a.rs") && text.contains("留了 2 則意見"),
        "{text}"
    );
    assert!(
        text.contains("1. 第 12 行（新增）：`+ let x = 1;`\n   改名叫 count\n   順便加註解"),
        "{text}"
    );
    assert!(
        text.contains("2. 第 20 行：`fn main() {`\n   這裡要處理錯誤"),
        "{text}"
    );
    assert!(!text.contains("old()"));
    let empty = json!({"id": "repo-a", "path": "a", "notes": [{"where": "x", "note": ""}]});
    assert!(handle_action(&d, "diff_notes_brief", &empty, None).is_err());
}

// ── 2026-10-16：沒交接就結束的會話，卡片給「接續補交接」（F-77）──

#[test]
fn missed_handoff_card_offers_resume_with_handoff_prompt() {
    let (_t, d) = spine_with_repos();
    let ra = ctx_core::registry::get_repo(&d, "repo-a").unwrap().path();
    let body = format!(
        "agent 會話結束但沒有交接（2026-10-16 01:00 開始，exit）\n期間 1 個 commit\n下次開場先補交接。\n{}aaaaaaaa-1111（目錄 {}）",
        ctx_core::hooks::RESUME_PREFIX,
        ra.display()
    );
    assert_eq!(
        ctx_core::hooks::resume_line(&body),
        Some(("aaaaaaaa-1111".to_string(), ra.display().to_string()))
    );
    assert_eq!(
        ctx_core::hooks::resume_line("接續：claude --resume x;rm（目錄 /a）"),
        None
    );
    ctx_core::spine::append_event(&d, "suggestion", "hook", &["repo:repo-a"], &body, None).unwrap();
    let st = st(&d);
    let card = st["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["title"].as_str().is_some_and(|t| t.contains("沒有交接")))
        .expect("沒交接的卡");
    let a = &card["actions"][0];
    assert_eq!(a["action"], "session_resume");
    assert_eq!(a["payload"]["resume"], "aaaaaaaa-1111");
    assert!(a["payload"]["prompt"].as_str().unwrap().contains("handoff"));
    // 接續時帶第一句
    let terms = ArgvTerms(Default::default());
    let mut p = a["payload"].clone();
    p["kind"] = json!("agent");
    handle_action(&d, "term_create", &p, Some(&terms)).unwrap();
    let argv = terms.0.lock().unwrap()[0].clone();
    assert_eq!(argv[2..4], ["--resume", "aaaaaaaa-1111"]);
    assert!(argv[4].contains("handoff"));
}

#[test]
fn group_notes_never_keep_secrets() {
    let (_t, d) = spine_with_repos();
    let r = handle_action(
        &d,
        "note_add",
        &json!({"group": "g1", "text": "部署用 ghp_abcdefghijklmnop1234 先頂著"}),
        None,
    )
    .unwrap();
    let text = std::fs::read_to_string(ctx_core::notes::path(&d, "g1")).unwrap();
    assert!(!text.contains("ghp_abcdef"), "{text}");
    assert!(text.contains("[REDACTED TOKEN]"));
    assert!(r.to_string().contains("[REDACTED TOKEN]"));
}

/// 2026-10-18：repo 類別——App 改類別走 registry_class（驗證＋留痕），名片帶類別與選項；
/// 參考類不採集 git、不進程式碼搜尋；context_for 給所在 repo 的規則。
#[test]
fn repo_class_soft_labels_and_reference_behavior() {
    let (_t, d) = spine_with_repos();
    let p = act(&d, "repo_profile", json!({"id": "repo-a"})).unwrap();
    assert!(p.get("class").is_none());
    assert_eq!(p["classes"].as_array().unwrap().len(), 6);

    let n0 = of_type(&d, EventType::Decision).len();
    let r = act(
        &d,
        "repo_class",
        json!({"id": "repo-a", "class": "knowledge"}),
    )
    .unwrap();
    assert_eq!(r["class"]["id"], "knowledge");
    assert_eq!(r["rescan"], json!(true), "參考類不採集：改類別後要重新採集");
    assert_eq!(r["class"]["label"], "知識");
    let evs = of_type(&d, EventType::Decision);
    assert_eq!(evs.len(), n0 + 1, "改類別要留痕");
    assert!(evs.last().unwrap().body.contains("repo-a 類別：知識"));
    assert!(err(&d, "repo_class", json!({"id": "repo-a", "class": "pm"})).contains("類別需為"));

    // 參考：採集時不跑 git（note 說明），程式碼搜尋不搜
    act(
        &d,
        "repo_class",
        json!({"id": "repo-b", "class": "reference"}),
    )
    .unwrap();
    let rb = registry::get_repo(&d, "repo-b").unwrap();
    assert!(rb.is_reference());
    let st = ctx_core::collect::collect_repo(&ctx_core::collect::RepoSpec::from(&rb));
    assert_eq!(st.note, "參考 repo：不採集 git 狀態");
    assert_eq!(st.dirty, 0);
    std::fs::write(rb.path().join("lib.rs"), "fn widget_cache() {}\n").unwrap();
    ctx_core::testkit::git(&rb.path(), &["add", "-A"], &[]);
    let data = registry::load(&d).unwrap();
    let hits = ctx_core::crossref::code_search(&data.repos, "widget_cache", 8, 40);
    assert!(hits.iter().all(|h| h.repo != "repo-b"), "{hits:?}");

    // context_for：所在 repo 的類別規則
    let ctx = ctx_core::agentapi::context_for(
        &d,
        &ctx_core::agentapi::Target::at(registry::get_repo(&d, "repo-a").unwrap().path()),
        false,
    )
    .unwrap();
    assert_eq!(ctx["repo_class"]["label"], "知識");

    // 清掉＝未分類
    let r = act(&d, "repo_class", json!({"id": "repo-a", "class": ""})).unwrap();
    assert!(r.get("class").is_none());
}
