//! F-121 對講：兩個 repo 的 agent 互傳訊息——送、hook 收、回覆送回原會話、跨組要同意、開場注入、閒著時 App 打字。

use std::path::Path;

use ctx_core::agentapi::mailbox::{self, Via};
use ctx_core::agentapi::Target;
use ctx_core::registry::{add_group, add_repo, AddRepo};
use ctx_core::session_status::{path_for, record_hook, StatusHook};
use ctx_core::testkit::{make_git_repo, spine_with_repos};
use ctx_core::{hooks, spine};
use serde_json::{json, Value};

fn repo_path(tmp: &Path, id: &str) -> std::path::PathBuf {
    tmp.join(id)
}

/// App 會話 sid 在 repo 目錄跑了一個工具（寫狀態檔：cwd、working）。
fn app_session(d: &Path, sid: &str, cwd: &Path, ev: StatusHook) {
    let st = path_for(d, sid);
    let payload =
        json!({"cwd": cwd.to_string_lossy(), "session_id": "11111111-2222-3333-4444-555555555555"});
    assert!(record_hook(ev, &payload.to_string(), st.to_str(), None));
}

#[allow(clippy::too_many_arguments)]
fn send(
    d: &Path,
    to: Option<&str>,
    text: &str,
    re: Option<&str>,
    t: &Target,
    ok: bool,
    from_sid: Option<&str>,
    now: Option<chrono::NaiveDateTime>,
) -> ctx_core::Result<Value> {
    let out = mailbox::Outgoing {
        to,
        text,
        re,
        from_sid,
    };
    mailbox::send_message(d, &out, t, ok, now)
}

fn hook(d: &Path, sid: &str, name: &str, cwd: &Path) -> String {
    let st = path_for(d, sid);
    mailbox::deliver_on_hook(name, &json!({"cwd": cwd.to_string_lossy()}), st.to_str())
}

#[test]
fn request_reaches_running_session_and_reply_goes_back_to_asker() {
    let (tmp, d) = spine_with_repos();
    let a = repo_path(tmp.path(), "repo-a");
    let b = repo_path(tmp.path(), "repo-b");
    app_session(&d, "t1", &a, StatusHook::ToolStart);
    app_session(&d, "t2", &b, StatusHook::ToolEnd);

    // repo-a 的會話 t1 請 repo-b 幫忙
    let r = send(
        &d,
        Some("repo-b"),
        "請把 API 加一個 limit 參數",
        None,
        &Target::at(&a),
        false,
        Some("t1"),
        None,
    )
    .unwrap();
    assert_eq!(r["ok"], true);
    let id = r["id"].as_str().unwrap().to_string();
    assert!(id.ends_with("-m1"), "{id}");
    assert_eq!(r["peers_in_target"].as_array().unwrap().len(), 1, "{r}");
    // 留痕：suggestion [agent] id: repo:repo-b group:g1，第一行〔對講〕
    let ev = spine::iter_events(&d).unwrap().into_iter().last().unwrap();
    assert_eq!(ev.etype.as_str(), "suggestion");
    assert_eq!(ev.kv("repo"), Some("repo-b"));
    assert_eq!(ev.kv("group"), Some("g1"));
    assert!(
        ev.body.starts_with("〔對講〕repo-a → repo-b："),
        "{}",
        ev.body
    );

    // repo-a 自己的 hook 收不到（不是給它的）
    assert_eq!(hook(&d, "t1", "PostToolUse", &a), "");
    // repo-b 的會話用完工具 → 塞進脈絡，只一次
    let out = hook(&d, "t2", "PostToolUse", &b.join("src"));
    let v: Value = serde_json::from_str(&out).unwrap();
    let ctxt = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(ctxt.contains(&format!("[{id}] repo-a → repo-b")), "{ctxt}");
    assert!(ctxt.contains("limit 參數"));
    assert_eq!(hook(&d, "t2", "PostToolUse", &b), "", "同一則不送第二次");

    // 開場名冊：repo-b 那場在做什麼看得到
    let c = ctx_core::agentapi::context_for(&d, &Target::at(&a), false).unwrap();
    let rb = c["roster"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "repo-b")
        .unwrap()
        .clone();
    assert!(
        rb["working_on"][0].as_str().unwrap().contains("處理中"),
        "{rb}"
    );
    // 會話列：t1 在等 repo-b 回覆；t2 收到一則
    let talk = mailbox::session_talk(&d, "t1", "2000-01-01 00:00").unwrap();
    assert_eq!(talk["label"], "等 repo-b 回覆");
    assert!(talk["lines"][0].as_str().unwrap().starts_with("→ repo-b"));
    assert_eq!(
        mailbox::session_talk(&d, "t2", "2000-01-01 00:00").unwrap()["label"],
        "對講 1 則"
    );
    assert!(
        mailbox::session_talk(&d, "t1", "2999-01-01 00:00").is_none(),
        "會話開始前的不算"
    );
    // repo-b 回覆（只帶 re）→ 回到 repo-a，指定給 t1
    let r2 = send(
        &d,
        None,
        "加好了，commit abc123",
        Some(&id),
        &Target::at(&b),
        false,
        Some("t2"),
        None,
    )
    .unwrap();
    assert_eq!(r2["to"], "repo-a");
    assert_eq!(r2["re"], json!(id));
    // repo-a 另一場會話 t3 也在跑：t1 還活著，回覆只給 t1
    app_session(&d, "t3", &a, StatusHook::ToolEnd);
    assert_eq!(hook(&d, "t3", "PostToolUse", &a), "");
    let back = hook(&d, "t1", "UserPromptSubmit", &a);
    assert_eq!(
        mailbox::session_talk(&d, "t1", "2000-01-01 00:00").unwrap()["label"],
        "對講 2 則"
    );
    assert!(
        back.contains("加好了") && back.contains(&format!("回你的 {id}")),
        "{back}"
    );

    // messages：看得到往來與同組在跑的人（不含自己）
    let m = mailbox::messages(&d, &Target::at(&a), Some("t1"), 10).unwrap();
    let dirs: Vec<&str> = m["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["direction"].as_str().unwrap())
        .collect();
    assert_eq!(dirs, vec!["in", "out"]);
    let peers: Vec<&str> = m["peers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["sid"].as_str().unwrap())
        .collect();
    assert!(
        peers.contains(&"t2") && peers.contains(&"t3") && !peers.contains(&"t1"),
        "{peers:?}"
    );
}

#[test]
fn reply_to_idle_asker_is_typed_in_by_app_but_new_requests_are_not() {
    let (tmp, d) = spine_with_repos();
    let a = repo_path(tmp.path(), "repo-a");
    let b = repo_path(tmp.path(), "repo-b");
    app_session(&d, "t1", &a, StatusHook::Stop); // 閒著（這一輪做完）
    let r = send(
        &d,
        Some("repo-b"),
        "幫我看一下",
        None,
        &Target::at(&a),
        false,
        Some("t1"),
        None,
    )
    .unwrap();
    let id = r["id"].as_str().unwrap().to_string();
    // 新請求：沒人在 repo-b 跑 → 不打字（交給收件匣卡片）
    assert!(mailbox::idle_replies(&d, &["t1".into()]).is_empty());
    assert_eq!(mailbox::undelivered(&d).len(), 1);
    let talk_unread = |d: &Path| {
        ctx_core::notify::pending(d)
            .unwrap()
            .normal
            .iter()
            .filter(|e| e.body.starts_with(mailbox::TAG))
            .count()
    };
    assert_eq!(talk_unread(&d), 1, "還沒送到：收件匣有卡");
    // repo-b 那邊後來開場（沒有 App 會話也行）：開場注入收到
    let start = hooks::session_start(
        &d,
        &json!({"cwd": b.to_string_lossy(), "session_id": "s-b"}),
        None,
    );
    assert!(
        start.contains("其他 repo 的 agent 傳話給你") && start.contains(&id),
        "{start}"
    );
    // 回覆 → t1 閒著 → App 打進去一次
    send(
        &d,
        None,
        "看完了，沒問題",
        Some(&id),
        &Target::at(&b),
        false,
        None,
        None,
    )
    .unwrap();
    let typed = mailbox::idle_replies(&d, &["t1".into()]);
    assert_eq!(typed.len(), 1);
    assert_eq!(typed[0].0, "t1");
    assert!(typed[0].1.contains("看完了") && !typed[0].1.contains('\n'));
    assert!(
        mailbox::idle_replies(&d, &["t1".into()]).is_empty(),
        "只打一次"
    );
    assert!(mailbox::undelivered(&d).is_empty());
    assert_eq!(talk_unread(&d), 0, "送到了、回覆也送回去了：收件匣不再佔位");
    let _ = Via::App;
}

#[test]
fn cross_group_needs_consent_and_unknown_targets_are_rejected() {
    let (tmp, d) = spine_with_repos();
    let a = repo_path(tmp.path(), "repo-a");
    let c = make_git_repo(tmp.path(), "repo-c", 1, 0, true);
    add_repo(&d, "repo-c", &c, AddRepo::default()).unwrap();
    add_group(&d, "g2", &["repo-c"]).unwrap();
    let t = Target::at(&a);
    let e = send(&d, Some("repo-c"), "嗨", None, &t, false, None, None).unwrap_err();
    assert!(e.to_string().contains("不在你的主場"), "{e}");
    assert!(send(&d, Some("repo-c"), "嗨", None, &t, true, None, None).is_ok());
    let e = send(&d, Some("nope"), "嗨", None, &t, false, None, None).unwrap_err();
    assert!(e.to_string().contains("沒有登記"), "{e}");
    let e = send(&d, None, "嗨", None, &t, false, None, None).unwrap_err();
    assert!(e.to_string().contains("要給哪個 repo"), "{e}");
    let e = send(&d, None, "嗨", Some("2026-01-01-m9"), &t, false, None, None).unwrap_err();
    assert!(e.to_string().contains("找不到訊息"), "{e}");
    // 沒有信箱檔時 hook 什麼都不做
    let (_t2, d2) = ctx_core::testkit::spine();
    let st = path_for(&d2, "t9");
    assert_eq!(
        mailbox::deliver_on_hook("PostToolUse", &json!({"cwd": "/x"}), st.to_str()),
        ""
    );
}
