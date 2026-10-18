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
    v(api::add_todo(&w.sp, text, None, &at(cwd), false, "agent").unwrap())
}

/// 型別化的回傳 → JSON：斷言比的是 MCP／CLI 看到的欄位（合約）。
fn v<T: serde::Serialize>(x: T) -> Value {
    serde_json::to_value(x).unwrap()
}

fn code_of(e: Error) -> (String, Option<String>) {
    match e {
        Error::Rejected { code, hint, .. } => (code.as_str().to_string(), hint),
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
    let a = v(a);
    let b = v(b);
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
    let r = v(r);
    let rf = r["ref"].as_str().unwrap().to_string();
    assert!(r["ok"] == true && rf.starts_with("decision:") && rf.ends_with("-d1"));
    // 內文的 markdown 標題被降級，不會被 validator 拒寫
    assert!(r["event"]["text"].as_str().unwrap().starts_with("### 結論"));
    let r2 =
        api::log_decision(&w.sp, "延伸判斷", &at(&w.impl_), Some(&rf), false, "agent").unwrap();
    let r2 = v(r2);
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
    let ok = v(ok);
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
    let same = v(same);
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
    let r = v(r);
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
    let h = v(h);
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
    let r = v(r);
    assert!(r.get("event").is_none()); // working 不吵人
    let r = api::report_status(&w.sp, "waiting", "要不要換 Redis？", &at(&w.impl_)).unwrap();
    let r = v(r);
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

// Rust 才有（刻意差異 N-14）：沒 session_id＝手動試跑／量測，照樣注入但不留 marker
#[test]
fn session_start_without_session_id_leaves_no_marker() {
    let w = two_groups();
    let cwd = w.impl_.to_string_lossy().to_string();
    for payload in [json!({"cwd": cwd}), json!({"session_id": "", "cwd": cwd})] {
        let out = hooks::session_start(&w.sp, &payload, None);
        assert!(out.contains("你在 impl"));
        assert!(hooks::session_end(&w.sp, &payload, None).is_none());
    }
    let d = w.sp.join(".state/hook_sessions");
    assert!(!d.exists() || std::fs::read_dir(&d).unwrap().next().is_none());
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
    let d = api::error_dict(&api::agent_error(
        ctx_core::RejectCode::BadRequest,
        "x",
        None,
    ));
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
            "fresh",
            // Rust 才有（2026-10-13）：組員名冊
            "roster"
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

// ---- Rust 才有（F-21）：未結事項體檢 ----

fn review_item<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|it| it["id"] == id)
        .unwrap()
}

#[test]
fn review_finds_commit_that_covers_the_todo() {
    let w = two_groups();
    let id = todo(&w, "加上快取失效機制", &w.impl_)["id"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::write(w.impl_.join("cache.rs"), "// 失效\n").unwrap();
    git(&w.impl_, &["add", "-A"], &[]);
    git(
        &w.impl_,
        &["commit", "-q", "-m", "加上快取失效機制：寫入時清掉舊值"],
        &[],
    );
    let v = api::review_todos(&w.sp, &at(&w.impl_)).unwrap();
    let it = review_item(&v, &id);
    assert_eq!(it["verdict"], "maybe_done");
    assert_eq!(it["evidence"][0]["kind"], "commit");
    assert_eq!(v["counts"]["maybe_done"], 1);
}

/// Rust 才有（2026-10-18）：做事的 commit 標題點名 #N＝證據；docs／chore 提到只算有人提到。
#[test]
fn review_counts_work_commit_that_names_the_todo() {
    let w = two_groups();
    let id = todo(&w, "左欄新建牌組預設不勾", &w.impl_)["id"]
        .as_str()
        .unwrap()
        .to_string();
    let other = todo(&w, "端到端驗證", &w.impl_)["id"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::write(w.impl_.join("picker.ts"), "x\n").unwrap();
    git(&w.impl_, &["add", "-A"], &[]);
    let msg = format!("fix(desktop)：PickerDialog 改初始值（{id}）");
    git(&w.impl_, &["commit", "-q", "-m", &msg], &[]);
    std::fs::write(w.impl_.join("e2e.sh"), "x\n").unwrap();
    git(&w.impl_, &["add", "-A"], &[]);
    let msg = format!("chore(scripts)：e2e 腳本（{other}）");
    git(&w.impl_, &["commit", "-q", "-m", &msg], &[]);
    let v = api::review_todos(&w.sp, &at(&w.impl_)).unwrap();
    let it = review_item(&v, &id);
    assert_eq!(it["verdict"], "maybe_done", "{it}");
    assert_eq!(it["evidence"][0]["shared"][0], id.as_str());
    assert_eq!(review_item(&v, &other)["verdict"], "active");
}

#[test]
fn review_does_not_count_the_handoff_that_opened_the_todo() {
    let w = two_groups();
    let r = api::handoff::<&str>(
        &w.sp,
        "整理 roadmap",
        "排優先序",
        &["補上匯出 CSV 功能"],
        &at(&w.plan),
        false,
        "agent",
    )
    .unwrap();
    let r = v(r);
    let id = r["todos"][0].as_str().unwrap().to_string();
    // 剛開：還在進行
    let now = api::review_todos(&w.sp, &at(&w.plan)).unwrap();
    assert_eq!(review_item(&now, &id)["verdict"], "active");
    // 20 天後、期間沒人提到：擱置（開出它的那筆交接不算「有人提到」，也不算證據）
    let later = Local::now().naive_local() + chrono::Duration::days(20);
    let v = api::review_todos_at(&w.sp, &at(&w.plan), Some(later)).unwrap();
    let it = review_item(&v, &id);
    assert_eq!(it["verdict"], "stale");
    assert!(it["evidence"].as_array().unwrap().is_empty());
}

#[test]
fn review_keeps_mentioned_todo_active() {
    let w = two_groups();
    let id = todo(&w, "等法務回覆授權條款", &w.impl_)["id"]
        .as_str()
        .unwrap()
        .to_string();
    let later = Local::now().naive_local() + chrono::Duration::days(20);
    // 有人後來提到 #N（不是做完，只是還在等）
    spine_mod::append_event(
        &w.sp,
        "decision",
        "agent",
        &["group:產品", "repo:impl"],
        &format!("{id} 還在等，先做別的"),
        Some(Local::now().naive_local() + chrono::Duration::minutes(2)),
    )
    .unwrap();
    let v = api::review_todos_at(&w.sp, &at(&w.impl_), Some(later)).unwrap();
    assert_eq!(review_item(&v, &id)["verdict"], "active");
}

#[test]
fn session_start_shows_review_hint_for_overdue_untouched_todo() {
    let w = two_groups();
    let yesterday = (Local::now() - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    let r = api::add_todo(
        &w.sp,
        "補 Windows 測試",
        Some(&yesterday),
        &at(&w.impl_),
        false,
        "agent",
    )
    .unwrap();
    let r = v(r);
    let id = r["id"].as_str().unwrap();
    let out = hooks::session_start(
        &w.sp,
        &json!({"session_id": "rv", "cwd": w.impl_.to_string_lossy()}),
        None,
    );
    assert!(
        out.contains("體檢：") && out.contains(&format!("逾期或擱置沒動靜 {id}")),
        "{out}"
    );
}

// ---- Rust 才有（F-07）：判斷取代＋何時重看 ----

fn log_ext(
    w: &World,
    text: &str,
    sup: Option<&str>,
    reopen: Option<&str>,
) -> ctx_core::Result<Value> {
    api::log_decision_ext(
        &w.sp,
        text,
        &at(&w.impl_),
        None,
        &api::DecisionExtra {
            supersedes: sup.map(str::to_string),
            reopen_when: reopen.map(str::to_string),
            outcome_of: None,
        },
        false,
        "agent",
    )
    .map(v)
}

#[test]
fn superseded_decision_leaves_recent_events() {
    let w = two_groups();
    let old = log_ext(&w, "快取用 LRU", None, None).unwrap()["ref"]
        .as_str()
        .unwrap()
        .to_string();
    let new = log_ext(&w, "快取改用 LFU", Some(&old), None).unwrap();
    let body = new["event"]["body"].as_str().unwrap();
    assert!(body.contains(&format!("取代：{old}")));
    let ev = spine_mod::iter_events(&w.sp)
        .unwrap()
        .into_iter()
        .last()
        .unwrap();
    assert_eq!(ev.kv("ref"), Some(old.as_str())); // 也寫成 ref token（Python 看得到關聯）
    let ctx = api::context_for(&w.sp, &at(&w.impl_), false).unwrap();
    let refs: Vec<&str> = ctx["recent_events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["ref"].as_str())
        .collect();
    assert!(!refs.contains(&old.as_str()) && refs.contains(&new["ref"].as_str().unwrap()));
    assert_eq!(
        api::superseded_map(&spine_mod::iter_events(&w.sp).unwrap())[&old],
        new["ref"]
    );
}

#[test]
fn supersedes_must_point_to_existing_decision() {
    let w = two_groups();
    let e = log_ext(&w, "亂取代", Some("decision:2099-01-01-d1"), None).unwrap_err();
    assert_eq!(code_of(e).0, "bad_request");
}

#[test]
fn reopen_condition_with_past_date_is_flagged_at_session_start() {
    let w = two_groups();
    let yesterday = (Local::now() - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    log_ext(
        &w,
        "先不做 Windows",
        None,
        Some(&format!("VDI 使用者回報問題，或 {yesterday}")),
    )
    .unwrap();
    log_ext(&w, "索引用 JSON", None, Some("repo 超過 50 個")).unwrap();
    let ctx = api::context_for(&w.sp, &at(&w.impl_), false).unwrap();
    let reopen = ctx["reopen"].as_array().unwrap();
    assert_eq!(reopen.len(), 2);
    let out = hooks::session_start(
        &w.sp,
        &json!({"session_id": "ro", "cwd": w.impl_.to_string_lossy()}),
        None,
    );
    assert!(
        out.contains("該重看：先不做 Windows") && out.contains("有條件的判斷：索引用 JSON"),
        "{out}"
    );
}

// ---- Rust 才有（F-22）：判斷可搜尋 ----

#[test]
fn search_finds_past_decisions_with_supersede_and_outcome() {
    let w = two_groups();
    let old = log_ext(&w, "快取淘汰策略用 LRU：讀多寫少", None, None).unwrap()["ref"]
        .as_str()
        .unwrap()
        .to_string();
    let new = log_ext(&w, "快取淘汰策略改用 LFU：熱點集中", Some(&old), None).unwrap()["ref"]
        .as_str()
        .unwrap()
        .to_string();
    spine_mod::append_event(
        &w.sp,
        "outcome",
        "manual",
        &["group:產品", &format!("ref:{new}")],
        "上線一週命中率 92%",
        None,
    )
    .unwrap();
    let r = api::search_knowledge(&w.sp, "快取淘汰策略", &at(&w.impl_), &SearchOpts::default())
        .unwrap();
    let ds = r["decisions"].as_array().expect("要搜到判斷");
    assert_eq!(ds[0]["ref"], new.as_str()); // 有效的排前面
    assert_eq!(ds[0]["outcomes"][0]["text"], "上線一週命中率 92%");
    let o = ds.iter().find(|d| d["ref"] == old.as_str()).unwrap();
    assert_eq!(o["superseded_by"], new.as_str());
}

#[test]
fn search_decisions_respect_scope_and_skip_noise() {
    let w = two_groups();
    // 研究組（notes）的判斷：在產品組搜不到（--here）
    api::log_decision(
        &w.sp,
        "向量索引用 HNSW",
        &at(&w.notes),
        None,
        false,
        "agent",
    )
    .unwrap();
    let here = SearchOpts {
        everywhere: false,
        ..SearchOpts::default()
    };
    let r = api::search_knowledge(&w.sp, "向量索引", &at(&w.impl_), &here).unwrap();
    assert!(r.get("decisions").is_none());
    // 沒有相關判斷：不放這個鍵
    let r = api::search_knowledge(
        &w.sp,
        "完全無關的詞彙",
        &at(&w.impl_),
        &SearchOpts::default(),
    )
    .unwrap();
    assert!(r.get("decisions").is_none());
}

// ---- Rust 才有（F-42）：任務包 ----

#[test]
fn todo_brief_assembles_goal_context_inputs_constraints() {
    let w = two_groups();
    registry::set_group_field(&w.sp, "產品", "goal", "把快取做穩").unwrap();
    std::fs::write(
        w.impl_.join("cache_design.md"),
        "# 快取淘汰設計\n快取淘汰策略的取捨\n",
    )
    .unwrap();
    log_ext(&w, "快取淘汰策略用 LFU", None, Some("熱點分散時")).unwrap();
    let id = todo(&w, "實作快取淘汰策略", &w.impl_)["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = api::todo_brief(&w.sp, &id, &at(&w.plan)).unwrap();
    let md = r["markdown"].as_str().unwrap();
    for part in [
        "## 目標",
        "實作快取淘汰策略",
        "組目標：把快取做穩",
        "快取淘汰策略用 LFU",
        "impl:cache_design.md",
        "熱點分散時",
        &format!("close_todo {id}"),
    ] {
        assert!(md.contains(part), "缺 {part}：\n{md}");
    }
    assert_eq!(
        code_of(api::todo_brief(&w.sp, "#999", &at(&w.plan)).unwrap_err()).0,
        "not_found"
    );
}

// Rust 才有（N-16）：開了超過 7 天沒結束訊號的會話 marker，開場時當成已結束處理並清掉
#[test]
fn stale_session_markers_are_swept_at_session_start() {
    let w = two_groups();
    let cwd = w.impl_.to_string_lossy().to_string();
    let long_ago = Local::now().naive_local() - chrono::Duration::days(10);
    hooks::session_start(
        &w.sp,
        &json!({"session_id": "old", "cwd": cwd}),
        Some(long_ago),
    );
    // 那個會話期間有事發生（commit），卻沒有交接
    std::fs::write(w.impl_.join("f.md"), "x\n").unwrap();
    git(&w.impl_, &["add", "-A"], &[]);
    git(&w.impl_, &["commit", "-q", "-m", "做了一半"], &[]);
    // 2 天前開的還不算陳舊（那一次開場會先清掉 old）
    let recent = Local::now().naive_local() - chrono::Duration::days(2);
    hooks::session_start(
        &w.sp,
        &json!({"session_id": "recent", "cwd": cwd}),
        Some(recent),
    );
    hooks::session_start(&w.sp, &json!({"session_id": "now", "cwd": cwd}), None);
    let d = w.sp.join(".state/hook_sessions");
    assert!(!d.join("old.json").exists());
    assert!(d.join("recent.json").exists() && d.join("now.json").exists());
    let ev = spine_mod::iter_events(&w.sp)
        .unwrap()
        .into_iter()
        .rfind(|e| e.source == "hook")
        .expect("要記一筆沒交接");
    assert!(
        ev.body.contains("沒有交接") && ev.body.contains("沒有收到結束訊號"),
        "{}",
        ev.body
    );
}

// ---- Rust 才有（F-23）：判斷回顧 ----

#[test]
fn retro_candidates_and_outcome_closes_the_loop() {
    let w = two_groups();
    let old = Local::now().naive_local() - chrono::Duration::days(20);
    let tok = |id: &str| vec![format!("id:{id}"), "group:產品".into(), "repo:impl".into()];
    spine_mod::append_event(
        &w.sp,
        "decision",
        "agent",
        &tok("2026-01-01-d1"),
        "快取用 LRU",
        Some(old),
    )
    .unwrap();
    spine_mod::append_event(
        &w.sp,
        "decision",
        "registry",
        &tok("2026-01-01-d2"),
        "〔registry〕登記 repo：x",
        Some(old),
    )
    .unwrap();
    spine_mod::append_event(
        &w.sp,
        "decision",
        "agent",
        &tok("2026-01-01-d3"),
        "剛做的判斷",
        None,
    )
    .unwrap();
    let events = spine_mod::iter_events(&w.sp).unwrap();
    let now = Local::now().naive_local();
    let ids = vec!["plan".to_string(), "impl".to_string()];
    let c = api::retro_candidates(&w.sp, &events, Some(("產品", &ids)), now);
    let refs: Vec<&str> = c.iter().filter_map(|x| x["ref"].as_str()).collect();
    assert_eq!(refs, vec!["decision:2026-01-01-d1"]); // 工具紀錄、太新的都不算
                                                      // 開場提一行
    let out = hooks::session_start(
        &w.sp,
        &json!({"session_id": "rt", "cwd": w.impl_.to_string_lossy()}),
        None,
    );
    assert!(
        out.contains("判斷回顧（共 1 筆待回顧）：decision:2026-01-01-d1"),
        "{out}"
    );
    // 記結果：寫成 outcome、ref 指回它；之後不再待回顧，判斷搜尋帶出結果
    let r = api::log_decision_ext(
        &w.sp,
        "上線後命中率 92%，判斷正確",
        &at(&w.impl_),
        None,
        &api::DecisionExtra {
            outcome_of: Some("decision:2026-01-01-d1".into()),
            ..Default::default()
        },
        false,
        "agent",
    )
    .unwrap();
    let r = v(r);
    assert_eq!(r["event"]["type"], "outcome");
    let events = spine_mod::iter_events(&w.sp).unwrap();
    assert!(api::retro_candidates(&w.sp, &events, Some(("產品", &ids)), now).is_empty());
    let s =
        api::search_knowledge(&w.sp, "快取 LRU", &at(&w.impl_), &SearchOpts::default()).unwrap();
    assert_eq!(
        s["decisions"][0]["outcomes"][0]["text"],
        "上線後命中率 92%，判斷正確"
    );
    // 結果要指向存在的判斷
    let bad = api::log_decision_ext(
        &w.sp,
        "x",
        &at(&w.impl_),
        None,
        &api::DecisionExtra {
            outcome_of: Some("decision:2099-01-01-d9".into()),
            ..Default::default()
        },
        false,
        "agent",
    );
    assert_eq!(code_of(bad.unwrap_err()).0, "bad_request");
}

// Rust 才有（N-22）：提醒集中一段，MCP context_for 的 hints 是同一份
#[test]
fn hints_shared_between_hook_and_context_for() {
    let w = two_groups();
    let yesterday = (Local::now() - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    log_ext(&w, "先不做 Windows", None, Some(&yesterday)).unwrap();
    api::add_todo(
        &w.sp,
        "補 VDI 測試（回填自 2026-09-12 會話 abc）",
        Some(&yesterday),
        &at(&w.impl_),
        false,
        "agent",
    )
    .unwrap();
    let ctx = api::context_for(&w.sp, &at(&w.impl_), false).unwrap();
    let kinds: Vec<&str> = ctx["hints"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["kind"].as_str())
        .collect();
    assert_eq!(kinds, vec!["review", "reopen"]);
    let out = hooks::session_start(
        &w.sp,
        &json!({"session_id": "hn", "cwd": w.impl_.to_string_lossy()}),
        None,
    );
    let v: Value = serde_json::from_str(&out).unwrap();
    let text = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    let block = text.split("## 提醒").nth(1).expect("要有提醒段");
    for h in ctx["hints"].as_array().unwrap() {
        assert!(block.contains(h["text"].as_str().unwrap()));
    }
    assert!(!text.contains("回填自")); // 回填尾巴開場不印
    assert!(text.contains("log_decision 帶 supersedes"));
}

// Rust 才有（F-40）：判斷匯出成 AgDR md
#[test]
fn export_agdr_marks_superseded_and_outcomes_idempotently() {
    let w = two_groups();
    let old = log_ext(&w, "快取用 LRU", None, Some("熱點分散時")).unwrap()["ref"]
        .as_str()
        .unwrap()
        .to_string();
    let new = log_ext(&w, "快取改用 LFU", Some(&old), None).unwrap()["ref"]
        .as_str()
        .unwrap()
        .to_string();
    api::log_decision_ext(
        &w.sp,
        "上線後命中率 92%",
        &at(&w.impl_),
        None,
        &api::DecisionExtra {
            outcome_of: Some(new.clone()),
            ..Default::default()
        },
        false,
        "agent",
    )
    .unwrap();
    api::handoff::<&str>(
        &w.sp,
        "做完快取",
        "壓測",
        &[],
        &at(&w.impl_),
        false,
        "agent",
    )
    .unwrap();
    let out = w._t.path().join("agdr");
    let r = api::export_agdr(&w.sp, &at(&w.impl_), &out).unwrap();
    assert_eq!(r["written"].as_array().unwrap().len(), 2); // 交接不匯出
    let id_of = |x: &str| x.trim_start_matches("decision:").to_string();
    let old_md = std::fs::read_to_string(out.join(format!("AgDR-{}.md", id_of(&old)))).unwrap();
    assert!(
        old_md.contains("status: superseded") && old_md.contains(&format!("superseded_by: {new}"))
    );
    assert!(old_md.contains("## 何時重看\n熱點分散時"));
    let new_md = std::fs::read_to_string(out.join(format!("AgDR-{}.md", id_of(&new)))).unwrap();
    assert!(
        new_md.contains(&format!("supersedes: [{old}]")) && new_md.contains("上線後命中率 92%")
    );
    let again = api::export_agdr(&w.sp, &at(&w.impl_), &out).unwrap();
    assert!(again["written"].as_array().unwrap().is_empty() && again["unchanged"] == 2);
}

// Rust 才有（N-26）：未結太多時 context_for 只給最該處理的 30 項，開場仍報總數
#[test]
fn context_for_caps_open_loops() {
    let w = two_groups();
    let yesterday = (Local::now() - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    for i in 0..34 {
        todo(&w, &format!("待辦 {i}"), &w.impl_);
    }
    api::add_todo(
        &w.sp,
        "逾期那一項",
        Some(&yesterday),
        &at(&w.impl_),
        false,
        "agent",
    )
    .unwrap();
    let ctx = api::context_for(&w.sp, &at(&w.impl_), false).unwrap();
    let loops = ctx["open_loops"].as_array().unwrap();
    assert_eq!(loops.len(), api::CONTEXT_LOOP_CAP);
    assert_eq!(loops[0]["text"], "逾期那一項"); // 逾期的排最前
    assert_eq!(ctx["open_loops_more"]["total"], 35);
    let out = hooks::session_start(
        &w.sp,
        &json!({"session_id": "cap", "cwd": w.impl_.to_string_lossy()}),
        None,
    );
    assert!(
        out.contains("未結（35）") && out.contains("…另 29 項"),
        "{out}"
    );
}

// ---- repo 名片（2026-10-13，Rust 才有）：組員名冊、先找 repo、agent 寫介紹 ----

#[test]
fn test_roster_lists_group_members_with_inferred_description() {
    let w = two_groups();
    let c = api::context_for(&w.sp, &at(&w.impl_), false).unwrap();
    let roster = c["roster"].as_array().unwrap();
    let ids: Vec<&str> = roster.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["plan", "impl"]);
    let me = &roster[1];
    assert_eq!(me["you"], true);
    assert_eq!(me["path"], w.impl_.to_string_lossy().as_ref());
    // 沒填介紹：從 README 截第一句（只讀 repo）
    assert_eq!(me["description"], "這個 repo 談 widget 快取設計。");
    assert_eq!(me["description_source"], "readme");
    assert!(roster[0].get("you").is_none());
    // 只有一個 repo 的範圍沒有「同組」：不給 roster 鍵
    let solo = api::context_for(&w.sp, &at(&w.notes), false).unwrap();
    assert!(solo.get("roster").is_none());
}

#[test]
fn test_describe_repo_is_scoped_and_feeds_roster_search_and_hook() {
    let w = two_groups();
    let d = registry::Describe {
        description: Some("產品規格與決策紀錄".into()),
        role: Some("規格來源".into()),
        keywords: Some(vec!["roadmap".into()]),
    };
    let r = v(api::describe_repo(&w.sp, Some("plan"), &d, Some(&w.impl_), false).unwrap());
    assert_eq!(r["repo"]["role"], "規格來源");
    assert!(r["said"].as_str().unwrap().contains("規格來源"));
    // 留痕：agent 改了 registry，使用者看得到
    let evs = spine_mod::iter_events(&w.sp).unwrap();
    let last = evs.last().unwrap();
    assert_eq!(last.source, "agent");
    assert!(last.body.contains("plan 名片"));
    // 別組的 repo：先問使用者
    let e = api::describe_repo(&w.sp, Some("notes"), &d, Some(&w.impl_), false).unwrap_err();
    assert_eq!(code_of(e).0, "cross_scope");
    assert!(api::describe_repo(&w.sp, Some("notes"), &d, Some(&w.impl_), true).is_ok());
    // 什麼都沒給
    let e = api::describe_repo(
        &w.sp,
        None,
        &registry::Describe::default(),
        Some(&w.impl_),
        false,
    )
    .unwrap_err();
    assert_eq!(code_of(e).0, "bad_request");
    // 空字串＝清掉
    let clear = registry::Describe {
        description: Some(String::new()),
        ..Default::default()
    };
    api::describe_repo(&w.sp, Some("notes"), &clear, Some(&w.impl_), true).unwrap();
    assert!(registry::get_repo(&w.sp, "notes")
        .unwrap()
        .description()
        .is_none());

    // 搜尋先回答「是哪個 repo」
    let s = api::search_knowledge(&w.sp, "roadmap 規格", &at(&w.impl_), &SearchOpts::default())
        .unwrap();
    assert_eq!(s["repo_profiles"][0]["repo"], "plan");
    assert!(s["repo_profiles"][0]["why"]
        .as_array()
        .unwrap()
        .contains(&json!("關鍵字命中")));
    // 開場名冊
    let out = hooks::session_start(
        &w.sp,
        &json!({"session_id": "s1", "cwd": w.impl_.to_string_lossy()}),
        None,
    );
    let ctx: Value = serde_json::from_str(&out).unwrap();
    let ctx = ctx["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(ctx.contains("## 同組成員（2 個"), "{ctx}");
    assert!(ctx.contains("- **plan**｜規格來源｜") && ctx.contains("產品規格與決策紀錄"));
    assert!(ctx.contains("- **impl**（你在這）") && ctx.contains("（推測自 README）"));
    assert!(ctx.contains("describe_repo"));
}

// ---- 牌組筆記給 agent（2026-10-14，Rust 才有）----

#[test]
fn test_group_notes_reach_context_for_and_session_start() {
    let w = two_groups();
    // 沒有筆記：不給鍵、開場不印
    let c = api::context_for(&w.sp, &at(&w.impl_), false).unwrap();
    assert!(c.get("notes").is_none());
    let long = "跟 AMSC 窗口約好週五看 demo".to_string() + &"細節".repeat(200);
    ctx_core::notes::add(&w.sp, "產品", "先記：登入要支援 SSO", None).unwrap();
    ctx_core::notes::add(&w.sp, "產品", &long, None).unwrap();
    let c = api::context_for(&w.sp, &at(&w.impl_), false).unwrap();
    let n = &c["notes"];
    assert_eq!(n["total"], 2);
    assert_eq!(
        n["path"],
        w.sp.join("groups/產品/notes.md").to_string_lossy().as_ref()
    );
    let items = n["items"].as_array().unwrap();
    assert!(items[0]["text"]
        .as_str()
        .unwrap()
        .starts_with("跟 AMSC 窗口約好週五看 demo"));
    assert!(items[0]["text"].as_str().unwrap().ends_with('…'));
    assert_eq!(items[0]["text"].as_str().unwrap().chars().count(), 301);
    assert_eq!(items[1]["text"], "先記：登入要支援 SSO");
    // 別組（研究）看不到產品的筆記
    let other = api::context_for(&w.sp, &at(&w.notes), false).unwrap();
    assert!(other.get("notes").is_none());
    // 開場注入：只一行
    let out = hooks::session_start(
        &w.sp,
        &json!({"session_id": "notes", "cwd": w.impl_.to_string_lossy()}),
        None,
    );
    let line = out
        .lines()
        .find(|l| l.contains("組筆記（2 則"))
        .unwrap_or_else(|| panic!("{out}"));
    assert!(line.contains("跟 AMSC 窗口約好週五看 demo"), "{line}");
    assert!(line.contains("…——全文："), "{line}");
    assert!(line.contains("notes.md"), "{line}");
}

/// 2026-10-14：組員有別的會話在跑時，名冊多「還沒 commit 的檔數」（別的 agent 可能正在改）。
#[test]
fn test_roster_shows_uncommitted_files_of_members_with_running_sessions() {
    let w = two_groups();
    // plan 有一個會話在跑，而且改了一個檔還沒 commit
    hooks::session_start(
        &w.sp,
        &json!({"session_id": "other", "cwd": w.plan.to_string_lossy()}),
        None,
    );
    std::fs::write(w.plan.join("draft.md"), "寫到一半\n").unwrap();
    let c = api::context_for(&w.sp, &at(&w.impl_), false).unwrap();
    let roster = c["roster"].as_array().unwrap();
    let plan = roster.iter().find(|r| r["id"] == "plan").unwrap();
    assert_eq!(plan["sessions"], 1);
    assert_eq!(plan["uncommitted"], 1);
    // 自己所在的 repo、沒有會話的組員不查
    let me = roster.iter().find(|r| r["id"] == "impl").unwrap();
    assert!(me.get("uncommitted").is_none());
    let out = hooks::session_start(
        &w.sp,
        &json!({"session_id": "me", "cwd": w.impl_.to_string_lossy()}),
        None,
    );
    assert!(
        out.contains("1 個會話在跑，1 個檔還沒 commit（可能正在被改）"),
        "{out}"
    );
}

// ---- 組名有空白（2026-10-14 實測 PM Group 的寫入全部失敗）----

/// 「PM Group」這種有空白的組名：留痕照常寫得進去（標頭寫成 group:PM_Group），讀回來也算這組的。
#[test]
fn test_group_name_with_space_can_log_handoff_and_close() {
    let (t, sp) = spine();
    let vms = make_git_repo(t.path(), "vms", 0, 0, true);
    let head = make_git_repo(t.path(), "head", 0, 0, true);
    registry::add_repo(&sp, "VMS PM", &vms, AddRepo::default()).unwrap();
    registry::add_repo(&sp, "head", &head, AddRepo::default()).unwrap();
    registry::add_group(&sp, "PM Group", &["VMS PM", "head"]).unwrap();
    let here = at(&vms);

    let d = v(api::log_decision(&sp, "採用 A 方案\n依據：x", &here, None, false, "agent").unwrap());
    assert_eq!(d["ok"], true, "{d}");
    let todo = v(api::add_todo(&sp, "補測試", None, &here, false, "agent").unwrap());
    let num = todo["id"].as_str().unwrap().to_string();
    let h = api::handoff(
        &sp,
        "做完 A",
        "接著做 B",
        &[] as &[&str],
        &here,
        false,
        "agent",
    );
    assert!(h.is_ok(), "{h:?}");

    // 標頭裡沒有空白的 token
    let evs = spine_mod::iter_events(&sp).unwrap();
    let last = evs.last().unwrap();
    assert_eq!(last.kv("group"), Some("PM_Group"));
    assert_eq!(last.kv("repo"), Some("VMS_PM"));

    // 讀回來：同組的另一個 repo 看得到交接與未結
    let ctx = api::context_for(&sp, &at(&head), false).unwrap();
    assert_eq!(ctx["last_handoff"]["next_step"], "接著做 B");
    assert!(ctx["open_loops"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["id"] == num.as_str()));
    // 關掉（主場檢查要認得換寫過的組名）
    assert!(api::close_todo(&sp, &num, None, Some(&vms), false, "agent").is_ok());
    // 任務包照樣找得到組
    let ctx = api::context_for(&sp, &at(&vms), false).unwrap();
    assert!(ctx["recent_events"].as_array().unwrap().len() >= 3);
}

/// F-63 MCP workbench_apply：預設只預覽；confirm 才執行；主場外要同意；畫面動作略過；記一筆。
#[test]
fn workbench_apply_previews_then_applies() {
    let (_t, d) = ctx_core::testkit::spine_with_repos();
    let here = registry::get_repo(&d, "repo-a").unwrap().path();
    registry::add_group(&d, "other", &["repo-b"]).unwrap();
    // repo-a 的主場＝g1（repo-a、repo-b），所以 repo-b 也在主場；另建一個只有外人的組測跨組
    let steps = json!([
        {"op": "tag", "args": {"id": "repo-a", "add": ["規格"]}},
        {"op": "switch_scope", "args": {"group": "g1"}},
        {"op": "set_goal", "args": {"group": "g1", "text": "把登入做完"}},
    ]);
    let pre = api::workbench_apply(&d, &steps, false, Some(&here), false, "agent").unwrap();
    assert_eq!(pre["preview"], json!(true));
    assert!(pre["steps"][1]["error"]
        .as_str()
        .unwrap()
        .contains("只在 App"));
    assert!(
        registry::get_repo(&d, "repo-a").unwrap().tags().is_empty(),
        "預覽不寫"
    );

    let r = api::workbench_apply(&d, &steps, true, Some(&here), false, "agent").unwrap();
    assert_eq!(r["ok"], json!(true));
    assert_eq!(r["done"].as_array().unwrap().len(), 2);
    assert_eq!(
        registry::get_repo(&d, "repo-a").unwrap().tags(),
        vec!["規格"]
    );
    let ev = spine_mod::iter_events(&d).unwrap().pop().unwrap();
    assert!(ev.body.starts_with("助手（agent）") && ev.body.contains("略過：切到牌組「g1」"));
}

/// 2026-10-18：agent 也改得了類別（同 App 助手的 set_class，走 repo_class action）。
#[test]
fn workbench_apply_sets_repo_class() {
    let (_t, d) = ctx_core::testkit::spine_with_repos();
    let here = registry::get_repo(&d, "repo-a").unwrap().path();
    let steps = json!([{"op": "set_class", "args": {"id": "repo-b", "class": "參考"}}]);
    let pre = api::workbench_apply(&d, &steps, false, Some(&here), false, "agent").unwrap();
    assert_eq!(pre["steps"][0]["text"], "repo-b：類別 → 參考");
    assert!(
        registry::get_repo(&d, "repo-b").unwrap().class().is_none(),
        "預覽不寫"
    );
    api::workbench_apply(&d, &steps, true, Some(&here), false, "agent").unwrap();
    assert_eq!(
        registry::get_repo(&d, "repo-b").unwrap().class(),
        Some(registry::RepoClass::Reference)
    );
}

#[test]
fn workbench_apply_guards_cross_scope() {
    let (_t, d) = ctx_core::testkit::spine_with_repos();
    let outside = d.parent().unwrap().join("outside-repo");
    std::fs::create_dir_all(&outside).unwrap();
    registry::add_repo(&d, "repo-x", &outside, AddRepo::default()).unwrap();
    let here = registry::get_repo(&d, "repo-a").unwrap().path();
    let steps = json!([{"op": "tag", "args": {"id": "repo-x", "add": ["x"]}}]);
    let r = api::workbench_apply(&d, &steps, true, Some(&here), false, "agent").unwrap();
    assert!(r["done"].as_array().unwrap().is_empty());
    assert!(registry::get_repo(&d, "repo-x").unwrap().tags().is_empty());
    let r = api::workbench_apply(&d, &steps, true, Some(&here), true, "agent").unwrap();
    assert_eq!(r["done"].as_array().unwrap().len(), 1);
}

/// F-69：read_from 讀 md 時附上「哪些 md 連到這份」；沒有就不加鍵。
#[test]
fn read_from_md_includes_linked_from() {
    let (_t, d) = ctx_core::testkit::spine_with_repos();
    let a = registry::get_repo(&d, "repo-a").unwrap().path();
    std::fs::write(a.join("notes").join("x.md"), "回到 [說明](../README.md)\n").unwrap();
    let here = registry::get_repo(&d, "repo-b").unwrap().path();
    let v = api::read_from(&d, "repo-a:README.md", None, Some(&here)).unwrap();
    assert_eq!(
        v["linked_from"],
        json!([{"path": "notes/x.md", "line": 1, "text": "回到 [說明](../README.md)"}])
    );
    let v = api::read_from(&d, "repo-a:notes/x.md", None, Some(&here)).unwrap();
    assert!(v.get("linked_from").is_none());
}

#[test]
fn session_end_records_how_to_resume_when_the_id_is_valid() {
    let w = two_groups();
    let start = Local::now().naive_local() - chrono::Duration::minutes(5);
    let id = "aaaaaaaa-2222";
    hooks::session_start(
        &w.sp,
        &json!({"session_id": id, "cwd": w.impl_.to_string_lossy()}),
        Some(start),
    );
    std::fs::write(w.impl_.join("f.md"), "x\n").unwrap();
    let ev = hooks::session_end(
        &w.sp,
        &json!({"session_id": id, "cwd": w.impl_.to_string_lossy()}),
        None,
    )
    .unwrap();
    let body = ev["body"].as_str().unwrap();
    assert_eq!(
        hooks::resume_line(body),
        Some((id.to_string(), w.impl_.to_string_lossy().into_owned()))
    );
}
