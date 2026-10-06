//! Agent 友善輪（2026-09-25）：agent 是主要使用者。對應 tests/test_agentapi.py。
//!
//! 釘住五件事：① 從任何 repo 的 cwd 都推得出主場；② 工作層動作回結構化結果、帶可引用 id；
//! ③ 範圍是機制（跨組寫入被拒，使用者同意後才放行）；④ 交接會變成下一次的起點；
//! ⑤ 知識 ↔ repo 相關度（中文 bigram、洩密檔不進索引）。

use std::path::{Path, PathBuf};

use chrono::Local;
use ctx_core::agentapi::{self as api, SearchOpts, Target};
use ctx_core::hooks;
use ctx_core::registry::{self, AddRepo};
use ctx_core::testkit::{git, make_git_repo, spine};
use ctx_core::{spine as spine_mod, summary, Error};
use serde_json::{json, Value};

struct World {
    _t: tempfile::TempDir,
    sp: PathBuf,
    plan: PathBuf,
    impl_: PathBuf,
    notes: PathBuf,
}

/// 兩組：產品（plan＋impl）、研究（notes）。impl 底下有子目錄。
fn two_groups() -> World {
    let (t, sp) = spine();
    let plan = make_git_repo(t.path(), "plan", 0, 0, true);
    let impl_ = make_git_repo(t.path(), "impl", 0, 0, true);
    std::fs::create_dir_all(impl_.join("src").join("deep")).unwrap();
    let notes = make_git_repo(t.path(), "notes", 0, 0, true);
    for (rid, p) in [("plan", &plan), ("impl", &impl_), ("notes", &notes)] {
        registry::add_repo(&sp, rid, p, AddRepo::default()).unwrap();
    }
    registry::add_group(&sp, "產品", &["plan", "impl"]).unwrap();
    registry::add_group(&sp, "研究", &["notes"]).unwrap();
    World {
        _t: t,
        sp,
        plan,
        impl_,
        notes,
    }
}

fn at(p: &Path) -> Target {
    Target::at(p)
}

fn todo(w: &World, text: &str, cwd: &Path) -> Value {
    api::add_todo(&w.sp, text, None, &at(cwd), false, "agent").unwrap()
}

fn code_of(e: Error) -> (String, Option<String>) {
    match e {
        Error::Rejected { code, hint, .. } => (code, hint),
        other => panic!("預期 AgentError，拿到 {other:?}"),
    }
}

// ---- ① 定位 ----

#[test]
fn test_where_am_i_from_subdirectory() {
    let w = two_groups();
    let r = api::where_am_i(&w.sp, Some(&w.impl_.join("src").join("deep"))).unwrap();
    assert!(r.registered && r.repo.as_ref().unwrap().id == "impl");
    assert_eq!(r.groups, vec!["產品"]);
    assert_eq!(r.group.as_deref(), Some("產品"));
}

#[test]
fn test_where_am_i_unregistered_and_spine() {
    let w = two_groups();
    let stray = w._t.path().join("stray");
    std::fs::create_dir_all(&stray).unwrap();
    let r = api::where_am_i(&w.sp, Some(&stray)).unwrap();
    assert!(!r.registered && r.hint.unwrap().contains("registry add"));
    let r = api::where_am_i(&w.sp, Some(&w.sp)).unwrap();
    assert!(r.in_spine && r.repo.is_none() && r.note.unwrap().contains("全部"));
}

// ---- ② 工作層動作 ----

#[test]
fn test_add_todo_numbers_and_tokens_follow_cwd() {
    let w = two_groups();
    let a = todo(&w, "驗證驗收條件", &w.impl_);
    let b = api::add_todo(
        &w.sp,
        "補壓測",
        Some("2026-12-01"),
        &at(&w.impl_),
        false,
        "agent",
    )
    .unwrap();
    assert_eq!(
        (a["id"].clone(), b["id"].clone()),
        (json!("#1"), json!("#2"))
    ); // 編號自動配
    assert_eq!(a["todo"]["group"], "產品");
    assert_eq!(a["todo"]["repo"], "impl");
    assert!(a["todo"]["due"].as_str().is_some_and(|d| !d.is_empty())); // 預設到期自動補
    assert_eq!(b["todo"]["due"], "2026-12-01");
}

#[test]
fn test_log_decision_returns_resolvable_ref() {
    let w = two_groups();
    let r = api::log_decision(
        &w.sp,
        "## 結論：用 LRU\n依據 plan/README.md",
        &at(&w.impl_),
        None,
        false,
        "agent",
    )
    .unwrap();
    let rf = r["ref"].as_str().unwrap().to_string();
    assert!(r["ok"] == true && rf.starts_with("decision:") && rf.ends_with("-d1"));
    // 內文的 markdown 標題被降級，不會被 validator 拒寫
    assert!(r["event"]["text"].as_str().unwrap().starts_with("### 結論"));
    let r2 =
        api::log_decision(&w.sp, "延伸判斷", &at(&w.impl_), Some(&rf), false, "agent").unwrap();
    assert!(r2["ref"].as_str().unwrap().ends_with("-d2"));
    assert!(spine_mod::lint(&w.sp, None).unwrap().is_empty()); // ref 解得回去
}

#[test]
fn test_queries_default_to_home_scope() {
    let w = two_groups();
    todo(&w, "產品的事", &w.impl_);
    todo(&w, "研究的事", &w.notes);
    let ctx = api::context_for(&w.sp, &at(&w.plan), true).unwrap();
    let texts: Vec<&str> = ctx["open_loops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|lp| lp["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, vec!["產品的事"]); // 別組的未結不混進來
    assert_eq!(ctx["scope"]["label"], "產品");
    assert!(ctx.get("card").is_some());
    let nxt = api::next_work(&w.sp, &at(&w.notes), 5).unwrap();
    let todos: Vec<&str> = nxt["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "todo")
        .map(|i| i["text"].as_str().unwrap())
        .collect();
    assert_eq!(todos, vec!["研究的事"]);
}

// ---- ③ 範圍是機制 ----

#[test]
fn test_cross_scope_write_is_refused_until_user_confirms() {
    let w = two_groups();
    let t = at(&w.impl_).with_group("研究");
    let e = api::add_todo(&w.sp, "偷偷寫到研究組", None, &t, false, "agent").unwrap_err();
    let (code, hint) = code_of(e);
    assert!(code == "cross_scope" && hint.unwrap().contains("先問使用者"));
    let ok = api::add_todo(&w.sp, "使用者同意後寫入", None, &t, true, "agent").unwrap();
    assert_eq!(ok["todo"]["group"], "研究");
    // 同組的另一個 repo 不算跨組
    let same = api::log_decision(
        &w.sp,
        "同組",
        &at(&w.impl_).with_repos(&registry::split_ids("plan")),
        None,
        false,
        "agent",
    )
    .unwrap();
    assert_eq!(same["ok"], true);
}

#[test]
fn test_close_todo_respects_scope() {
    let w = two_groups();
    let t = todo(&w, "研究的事", &w.notes);
    let id = t["id"].as_str().unwrap();
    let e = api::close_todo(&w.sp, id, None, Some(&w.impl_), false, "agent").unwrap_err();
    assert_eq!(code_of(e).0, "cross_scope");
    let r = api::close_todo(
        &w.sp,
        id.trim_start_matches('#'),
        Some("讀完了"),
        Some(&w.notes),
        false,
        "agent",
    )
    .unwrap();
    assert_eq!(r["closed"], "研究的事");
    let ctx = api::context_for(&w.sp, &at(&w.notes), true).unwrap();
    assert_eq!(ctx["open_loops"], json!([]));
    let e = api::close_todo(&w.sp, id, None, Some(&w.notes), false, "agent").unwrap_err();
    assert_eq!(code_of(e).0, "not_found");
}

// ---- ④ 交接是下一次的起點 ----

#[test]
fn test_handoff_becomes_next_step_and_opens_remaining() {
    let w = two_groups();
    let h = api::handoff(
        &w.sp,
        "寫完快取層",
        "補整合測試",
        &["壓測", ""],
        &at(&w.impl_),
        false,
        "agent",
    )
    .unwrap();
    assert_eq!(h["todos"], json!(["#1"])); // 空字串不開
    let ctx = api::context_for(&w.sp, &at(&w.plan), true).unwrap();
    assert_eq!(ctx["last_handoff"]["next_step"], "補整合測試");
    assert_eq!(ctx["last_handoff"]["summary"], "寫完快取層");
    assert_eq!(ctx["agent_status"]["status"], "done");
    let nxt = api::next_work(&w.sp, &at(&w.plan), 5).unwrap();
    assert_eq!(nxt["items"][0]["kind"], "handoff");
    assert_eq!(nxt["items"][0]["text"], "補整合測試");
    let s = summary::build_summary(&w.sp, Some("產品"), None, None, None, None).unwrap();
    let next = s.next.unwrap();
    assert!(next.text == "補整合測試" && next.source.contains("交接"));
    assert_eq!(s.progress.unwrap().text, "交接：寫完快取層");
}

#[test]
fn test_overdue_todo_outranks_handoff() {
    let w = two_groups();
    api::add_todo(
        &w.sp,
        "早該做完",
        Some("2020-01-01"),
        &at(&w.impl_),
        false,
        "agent",
    )
    .unwrap();
    api::handoff::<&str>(
        &w.sp,
        "做了一半",
        "繼續",
        &[],
        &at(&w.impl_),
        false,
        "agent",
    )
    .unwrap();
    let nxt = api::next_work(&w.sp, &at(&w.impl_), 5).unwrap();
    let items = nxt["items"].as_array().unwrap();
    let kinds: Vec<&str> = items[..2]
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["todo", "handoff"]);
    assert!(items[0]["why"].as_str().unwrap().contains("逾期"));
}

#[test]
fn test_report_status_waiting_reaches_inbox() {
    let w = two_groups();
    let r = api::report_status(&w.sp, "working", "改快取", &at(&w.impl_)).unwrap();
    assert!(r.get("event").is_none()); // working 不吵人
    let r = api::report_status(&w.sp, "waiting", "要不要換 Redis？", &at(&w.impl_)).unwrap();
    assert_eq!(r["event"]["type"], "suggestion");
    assert!(r["event"]["text"].as_str().unwrap().contains("等你回應"));
    let st = api::get_status(
        &w.sp,
        Some("產品"),
        Some(&["plan".to_string(), "impl".to_string()][..]),
    )
    .unwrap();
    assert_eq!(st["status"], "waiting");
    let e = api::report_status(&w.sp, "sleeping", "", &at(&w.impl_)).unwrap_err();
    assert!(matches!(e, Error::Rejected { .. }));
}

// ---- ⑤ 知識相關度 ----

#[test]
fn test_search_knowledge_ranks_repos() {
    let w = two_groups();
    std::fs::create_dir_all(w.notes.join("cards")).unwrap();
    std::fs::write(
        w.notes.join("cards").join("決策平台.md"),
        "# 大數據自動決策平台\n決策平台需要特徵倉庫與即時推論。\n",
    )
    .unwrap();
    std::fs::write(
        w.plan.join("roadmap.md"),
        "# Roadmap\n下一季做決策平台的特徵倉庫。\n",
    )
    .unwrap();
    let r = api::search_knowledge(
        &w.sp,
        "特徵倉庫 決策平台",
        &at(&w.impl_),
        &SearchOpts::default(),
    )
    .unwrap();
    assert!(r["ok"] == true && r["scope"] == "全部");
    let top: Vec<&str> = r["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["repo"].as_str().unwrap())
        .collect();
    let mut first2 = top[..2].to_vec();
    first2.sort();
    assert_eq!(first2, vec!["notes", "plan"]);
    assert!(!top.contains(&"impl"));
    let f = &r["files"][0];
    assert!(f["snippets"][0]["line"].as_u64().unwrap() >= 1);
    assert!(f["ref"].as_str().unwrap().contains(':'));
    let here = api::search_knowledge(
        &w.sp,
        "特徵倉庫",
        &at(&w.impl_),
        &SearchOpts {
            everywhere: false,
            ..Default::default()
        },
    )
    .unwrap();
    let repos: Vec<&str> = here["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["repo"].as_str().unwrap())
        .collect();
    assert_eq!(repos, vec!["plan"]); // 主場＝產品組
    let e =
        api::search_knowledge(&w.sp, "  ", &Target::default(), &SearchOpts::default()).unwrap_err();
    assert_eq!(code_of(e).0, "bad_request");
}

#[test]
fn test_search_skips_secret_files_and_caches() {
    let w = two_groups();
    std::fs::write(
        w.plan.join("keys.md"),
        "# 部署\nAKIAABCDEFGHIJKLMNOP 部署金鑰\n",
    )
    .unwrap();
    let r = api::search_knowledge(&w.sp, "部署金鑰", &at(&w.sp), &SearchOpts::default()).unwrap();
    assert!(r["files"]
        .as_array()
        .unwrap()
        .iter()
        .all(|f| f["file"] != "keys.md"));
    assert!(w.sp.join(".state").join("knowledge_index.json").exists());
}

// test_tokenize_mixes_cjk_bigrams_and_words：純 knowledge 函式，見 tests/knowledge.rs 同名測試。

// ---- hooks ----

#[test]
fn test_session_start_hook_injects_context() {
    let w = two_groups();
    todo(&w, "驗證驗收條件", &w.impl_);
    let out = hooks::session_start(
        &w.sp,
        &json!({"session_id": "s1", "cwd": w.impl_.to_string_lossy()}),
        None,
    );
    let v: Value = serde_json::from_str(&out).unwrap();
    let data = &v["hookSpecificOutput"];
    assert_eq!(data["hookEventName"], "SessionStart");
    let ctx = data["additionalContext"].as_str().unwrap();
    assert!(
        ctx.contains("你在 impl")
            && ctx.contains("產品")
            && ctx.contains("#1 驗證驗收條件")
            && ctx.contains("handoff")
    );
    assert!(w.sp.join(".state/hook_sessions/s1.json").exists());
}

#[test]
fn test_session_start_hook_silent_outside_registry() {
    let w = two_groups();
    let out = hooks::session_start(
        &w.sp,
        &json!({"session_id": "x", "cwd": w._t.path().to_string_lossy()}),
        None,
    );
    assert_eq!(out, "");
}

#[test]
fn test_session_end_records_missing_handoff() {
    let w = two_groups();
    let start = Local::now().naive_local() - chrono::Duration::minutes(5);
    hooks::session_start(
        &w.sp,
        &json!({"session_id": "s2", "cwd": w.impl_.to_string_lossy()}),
        Some(start),
    );
    std::fs::write(w.impl_.join("feature.md"), "新功能\n").unwrap();
    git(&w.impl_, &["add", "-A"], &[]);
    git(&w.impl_, &["commit", "-q", "-m", "加新功能"], &[]);
    let ev =
        hooks::session_end(&w.sp, &json!({"session_id": "s2", "reason": "exit"}), None).unwrap();
    assert!(ev["type"] == "suggestion" && ev["source"] == "hook");
    let body = ev["body"].as_str().unwrap();
    assert!(body.contains("沒有交接") && body.contains("加新功能"));
    assert_eq!(ev["group"], "產品");
    assert!(!w.sp.join(".state/hook_sessions/s2.json").exists());
}

#[test]
fn test_session_end_quiet_after_handoff_or_nothing_happened() {
    let w = two_groups();
    let start = Local::now().naive_local() - chrono::Duration::minutes(5);
    let p = w.plan.to_string_lossy().to_string();
    hooks::session_start(&w.sp, &json!({"session_id": "a", "cwd": p}), Some(start));
    assert!(hooks::session_end(&w.sp, &json!({"session_id": "a"}), None).is_none()); // 什麼都沒發生
    hooks::session_start(&w.sp, &json!({"session_id": "b", "cwd": p}), Some(start));
    api::handoff::<&str>(
        &w.sp,
        "整理 roadmap",
        "排優先序",
        &[],
        &at(&w.plan),
        false,
        "agent",
    )
    .unwrap();
    assert!(hooks::session_end(&w.sp, &json!({"session_id": "b"}), None).is_none()); // 有交接就不吵
    assert!(hooks::session_end(&w.sp, &json!({"session_id": "never-started"}), None).is_none());
}

// ---- MCP 分層 ----
// test_mcp_default_exposes_only_work_tools、test_mcp_admin_via_config、
// test_mcp_work_tools_return_json_and_structured_errors、test_mcp_admin_open_loops_is_scoped：
// 已搬到 crates/ctx-cli/tests/agentapi.rs（MCP 協定 handle_message）。
// 未搬：test_workbench_session_shows_agent_status——等 workbench::state（annotate_terms，另一個 agent 在搬）。

// ---- Rust 版補測（非 Python 一對一）----

/// AgentError.to_dict 的形狀（MCP／CLI 的錯誤輸出靠它）。
#[test]
fn error_dict_shape() {
    let w = two_groups();
    let e = api::add_todo(
        &w.sp,
        "跨組",
        None,
        &at(&w.impl_).with_group("研究"),
        false,
        "agent",
    )
    .unwrap_err();
    let d = api::error_dict(&e);
    assert_eq!(d["ok"], false);
    assert_eq!(d["error"], "cross_scope");
    assert!(d["message"].as_str().unwrap().contains("研究"));
    assert!(d["hint"].as_str().is_some());
    let d = api::error_dict(&api::agent_error("bad_request", "x", None));
    assert!(d.get("hint").is_none());
}

/// context_for(card=false) 不帶 card；鍵序同 Python。
#[test]
fn context_for_keys_and_no_card() {
    let w = two_groups();
    let ctx = api::context_for(&w.sp, &at(&w.impl_), false).unwrap();
    let keys: Vec<&str> = ctx
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "ok",
            "where",
            "scope",
            "goal",
            "goal_gap",
            "last_progress",
            "last_handoff",
            "next",
            "next_gap",
            "changes",
            "open_loops",
            "recent_events",
            "relations",
            "pulse",
            "agent_status",
            "attention",
            "neighbors",
            "drift",
            "fresh"
        ]
    );
    assert_eq!(ctx["where"]["repo"]["id"], "impl");
}

/// ask_repo 走注入的文字 agent，不打真網路；引用方＝cwd 所在 repo。
#[test]
fn ask_repo_with_injected_provider() {
    let w = two_groups();
    let fake = |_: &str, _: &str, _: &Path, _: Option<&str>, _: Option<&Path>, ro: bool| {
        assert!(ro);
        Ok(r#"{"answer": "用 LRU", "citations": ["README.md"]}"#.to_string())
    };
    let r = api::ask_repo_with(
        &w.sp,
        "plan",
        "快取？",
        Some(&w.impl_),
        Some("claude"),
        &fake,
    )
    .unwrap();
    assert_eq!(r["ok"], true);
    assert_eq!(r["repo"], "plan");
    let e = api::ask_repo_with(&w.sp, "nope", "快取？", Some(&w.impl_), None, &fake).unwrap_err();
    assert_eq!(code_of(e).0, "not_found");
}

/// repo_status 省略 repo＝cwd 所在 repo；不在已登記 repo＝bad_request。
#[test]
fn repo_status_defaults_to_cwd() {
    let w = two_groups();
    let r = api::repo_status(&w.sp, None, Some(&w.impl_), None).unwrap();
    assert_eq!(r["repo"], "impl");
    let e = api::repo_status(&w.sp, None, Some(w._t.path()), None).unwrap_err();
    assert_eq!(code_of(e).0, "bad_request");
}
