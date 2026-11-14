//! F-122 接力：plan 的每種狀態、略過參考類／封存、排序、純讀取；report 的每種結論與總計。

use std::path::{Path, PathBuf};

use chrono::{Duration, Local, NaiveDateTime};
use ctx_core::registry::{add_group, add_repo, set_class, set_field, AddRepo, RepoClass};
use ctx_core::relay::{self, Delivery, RelayScope, RelayStatus, SessionNow, Verdict};
use ctx_core::session_status::{path_for, record_hook, StatusHook};
use ctx_core::spine;
use ctx_core::testkit::{make_git_repo, spine};
use serde_json::json;

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

fn ago(days: i64, hours: i64) -> NaiveDateTime {
    now() - Duration::days(days) - Duration::hours(hours)
}

/// 照 `agentapi::handoff` 的格式寫一筆交接。
fn handoff(d: &Path, repo: &str, when: NaiveDateTime, done: &str, next: &str) {
    let body = format!("交接：{done}\n完成：{done}\n剩下：無\n下次從這裡接：{next}");
    spine::append_event(
        d,
        "decision",
        "agent",
        &[format!("repo:{repo}")],
        &body,
        Some(when),
    )
    .unwrap();
}

/// 照 hooks 的 SessionEnd 寫一筆「會話結束但沒有交接」。
fn no_handoff(d: &Path, repo: &str, when: NaiveDateTime) {
    let body = format!(
        "agent 會話結束但沒有交接（{} 開始，結束）\n期間 0 個 commit；{repo} 目前未提交 1 個檔；範圍內留痕 1 筆\n下次開場先補交接：做到哪、剩什麼、下次從哪接。",
        when.format("%Y-%m-%d %H:%M")
    );
    spine::append_event(
        d,
        "suggestion",
        "hook",
        &[format!("repo:{repo}")],
        &body,
        Some(when),
    )
    .unwrap();
}

/// App 會話 sid 在 cwd 跑（寫狀態檔）。
fn session(d: &Path, sid: &str, cwd: &Path, ev: StatusHook) {
    let st = path_for(d, sid);
    let payload =
        json!({"cwd": cwd.to_string_lossy(), "session_id": "11111111-2222-3333-4444-555555555555"});
    assert!(record_hook(ev, &payload.to_string(), st.to_str(), None));
}

fn register(d: &Path, tmp: &Path, id: &str, days_old: i64) -> PathBuf {
    let p = make_git_repo(tmp, id, days_old, 0, false);
    add_repo(d, id, &p, AddRepo::default()).unwrap();
    p
}

fn row<'a>(p: &'a relay::Plan, id: &str) -> &'a relay::PlanRow {
    p.rows
        .iter()
        .find(|r| r.repo == id)
        .unwrap_or_else(|| panic!("plan 沒有 {id}"))
}

fn event_count(d: &Path) -> usize {
    spine::iter_events(d).unwrap().len()
}

#[test]
fn plan_classifies_every_status() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let ids = [
        "ready-old",
        "ready-new",
        "needs",
        "stale",
        "none",
        "no-next",
        "gap",
        "busy",
        "gone",
        "ref",
        "arch",
    ];
    for id in ids {
        register(&d, t, id, 6);
    }
    add_group(&d, "g", &ids).unwrap();

    handoff(&d, "ready-old", ago(3, 0), "做完 A", "把 B 的測試補上。");
    handoff(&d, "ready-new", ago(1, 0), "做完 C", "接著寫 D 的文件");
    handoff(
        &d,
        "needs",
        ago(1, 0),
        "草稿好了",
        "等使用者挑一個方案再改正文",
    );
    handoff(&d, "stale", ago(20, 0), "做完 E", "繼續 F");
    handoff(&d, "no-next", ago(1, 0), "做完 G", "無");
    handoff(&d, "gap", ago(3, 0), "做完 H", "做 I");
    no_handoff(&d, "gap", ago(1, 0));
    handoff(&d, "busy", ago(1, 0), "做完 J", "做 K");
    session(&d, "t1", &t.join("busy"), StatusHook::ToolStart);
    handoff(&d, "gone", ago(1, 0), "做完 L", "做 M");
    std::fs::remove_dir_all(t.join("gone")).unwrap();
    set_class(&d, "ref", Some(RepoClass::Reference)).unwrap();
    set_field(&d, "arch", "tier", "archived").unwrap();

    let before = event_count(&d);
    let p = relay::plan(&d, &RelayScope::Group("g".into())).unwrap();
    assert_eq!(event_count(&d), before, "plan 不寫事件");

    assert_eq!(p.scope, "g");
    assert_eq!(p.skipped, 2, "參考類與封存不列");
    assert!(p.rows.iter().all(|r| r.repo != "ref" && r.repo != "arch"));
    let st = |id: &str| row(&p, id).status;
    assert_eq!(st("ready-old"), RelayStatus::Ready);
    assert_eq!(st("ready-new"), RelayStatus::Ready);
    assert_eq!(st("needs"), RelayStatus::NeedsYou);
    assert_eq!(st("stale"), RelayStatus::Stale);
    assert_eq!(st("none"), RelayStatus::NoHandoff);
    assert_eq!(st("no-next"), RelayStatus::NoHandoff);
    assert_eq!(st("gap"), RelayStatus::Unhandoffed);
    assert_eq!(st("busy"), RelayStatus::Busy);
    assert_eq!(st("gone"), RelayStatus::Missing);

    // 可接在前、交接新的在前
    assert_eq!(p.rows[0].repo, "ready-new");
    assert_eq!(p.rows[1].repo, "ready-old");
    assert_eq!(p.rows.last().unwrap().repo, "gone");

    let r = row(&p, "ready-old");
    let prompt = r.prompt.as_deref().unwrap();
    assert!(prompt.starts_with("接續上次交接（"), "{prompt}");
    assert!(
        prompt.contains("：把 B 的測試補上。先確認這一步還成立再動手；做完 handoff。"),
        "{prompt}"
    );
    assert_eq!(r.next_step.as_deref(), Some("把 B 的測試補上。"));
    assert!(r.handoff_ref.is_some() && r.handoff_at.is_some());
    assert_eq!(r.group.as_deref(), Some("g"));

    assert!(row(&p, "needs").prompt.is_none(), "等你的不給 agent 任務");
    assert!(row(&p, "busy").prompt.is_none());
    assert!(row(&p, "busy").notes.iter().any(|n| n.contains("t1")));
    assert!(row(&p, "stale").prompt.as_deref().unwrap().contains("天前"));
    let gap = row(&p, "gap");
    assert!(gap.prompt.as_deref().unwrap().contains("補一筆 handoff"));
    assert!(gap.notes.iter().any(|n| n.contains("沒有交接")));
    assert!(row(&p, "no-next")
        .notes
        .iter()
        .any(|n| n.contains("沒寫下一步")));
    assert!(row(&p, "gone").notes[0].contains("路徑不存在"));

    let count = |s: RelayStatus| {
        p.counts
            .iter()
            .find(|c| c.status == s)
            .map(|c| c.count)
            .unwrap_or(0)
    };
    assert_eq!(count(RelayStatus::Ready), 2);
    assert_eq!(count(RelayStatus::NoHandoff), 2);

    // JSON 合約：狀態是 snake_case 字串
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(v["rows"][0]["status"], "ready");
}

#[test]
fn plan_priority_and_dirty_note() {
    let (tmp, d) = spine();
    let t = tmp.path();
    // 交接之後才補交接：舊的「沒交接」不算
    register(&d, t, "fixed", 6);
    no_handoff(&d, "fixed", ago(2, 0));
    handoff(&d, "fixed", ago(1, 0), "補交接", "做 N");
    // 沒交接過、最近有沒交接的會話 → 先補交接（優先於沒有交接）
    register(&d, t, "never", 6);
    no_handoff(&d, "never", ago(1, 0));
    // 很久以前的沒交接不算
    register(&d, t, "ancient", 6);
    no_handoff(&d, "ancient", ago(30, 0));
    // 太舊又在等你：算太舊、不給任務
    register(&d, t, "old-ask", 6);
    handoff(&d, "old-ask", ago(20, 0), "草稿", "使用者確認後 commit");
    // 會話在跑的優先於等你
    let b = register(&d, t, "busy-ask", 6);
    handoff(&d, "busy-ask", ago(1, 0), "草稿", "等你決定");
    session(&d, "t2", &b, StatusHook::Permission);
    // 會話開著、這一輪做完（閒著），下一步不是在等你
    let io = register(&d, t, "idle-open", 6);
    handoff(&d, "idle-open", ago(1, 0), "做完", "做 T");
    session(&d, "t5", &io, StatusHook::Stop);
    // 沒 commit 的檔
    let w = register(&d, t, "dirty", 6);
    std::fs::write(w.join("x.md"), "x").unwrap();
    std::fs::write(w.join("y.md"), "y").unwrap();
    handoff(&d, "dirty", ago(1, 0), "做完", "做 O");

    let p = relay::plan(&d, &RelayScope::All).unwrap();
    assert_eq!(p.scope, "全部");
    assert_eq!(row(&p, "fixed").status, RelayStatus::Ready);
    assert_eq!(row(&p, "never").status, RelayStatus::Unhandoffed);
    assert_eq!(row(&p, "ancient").status, RelayStatus::NoHandoff);
    let oa = row(&p, "old-ask");
    assert_eq!(oa.status, RelayStatus::Stale);
    assert!(oa.prompt.is_none());
    // 會話開著、在等你：不是「處理中」，而是在等你回一句
    let ba = row(&p, "busy-ask");
    assert_eq!(ba.status, RelayStatus::SessionOpen);
    assert!(ba.notes[0].contains("等你"));
    assert!(ba.waits_on_you);
    assert!(oa.waits_on_you, "太舊且在等你也算等你回覆");
    assert!(!row(&p, "dirty").waits_on_you);
    let io = row(&p, "idle-open");
    assert_eq!(io.status, RelayStatus::SessionOpen, "閒著的會話也算開著");
    assert!(!io.waits_on_you, "下一步不是在等你");
    // 等你回覆的：old-ask（太舊＋等你）、busy-ask（會話開著＋等你）
    assert_eq!(p.waiting_on_you, 2);
    let v = serde_json::to_value(&p).unwrap();
    assert!(v["rows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["status"] == "session_open"));
    let dr = row(&p, "dirty");
    assert_eq!(dr.status, RelayStatus::Ready);
    assert!(
        dr.notes.iter().any(|n| n == "2 個檔沒 commit"),
        "{:?}",
        dr.notes
    );
    assert_eq!(dr.group, None, "全部範圍、不在任何組");
}

#[test]
fn report_verdicts_and_total() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let since_t = ago(2, 0);
    let since = since_t.format("%Y-%m-%d %H:%M").to_string();
    for id in ["done", "wait-ho", "wait-sess", "run", "commit-only", "idle"] {
        register(&d, t, id, 6);
    }
    // commit-only：since 之後有 commit、沒交接
    let c = t.join("commit-only");
    std::fs::write(c.join("new.md"), "新\n").unwrap();
    ctx_core::testkit::git(&c, &["add", "-A"], &[]);
    ctx_core::testkit::git(&c, &["commit", "-q", "-m", "加上新檔"], &[]);
    // done：since 之前一筆（不算）、之後一筆
    handoff(&d, "done", ago(5, 0), "舊的", "舊的下一步");
    handoff(&d, "done", ago(1, 0), "做完 P", "做 Q");
    // wait-ho：交接的下一步在等你
    handoff(&d, "wait-ho", ago(1, 0), "草稿", "請使用者看過再決定");
    // wait-sess：會話要你允許
    session(&d, "t3", &t.join("wait-sess"), StatusHook::Permission);
    // run：會話處理中（就算有交接也算在跑）
    handoff(&d, "run", ago(1, 0), "做完 R", "做 S");
    session(&d, "t4", &t.join("run"), StatusHook::ToolStart);
    // idle 的 repo 有沒 commit 的檔，但沒 commit、沒交接
    std::fs::write(t.join("idle").join("wip.md"), "x").unwrap();

    let before = event_count(&d);
    let r = relay::report(&d, &RelayScope::All, &since).unwrap();
    assert_eq!(event_count(&d), before, "report 不寫事件");
    assert_eq!(r.since, since);
    let get = |id: &str| r.rows.iter().find(|x| x.repo == id).unwrap();
    assert_eq!(get("done").verdict, Verdict::Done);
    assert_eq!(get("done").handoffs.len(), 1, "since 之前的交接不算");
    assert_eq!(get("done").handoffs[0].next_step.as_deref(), Some("做 Q"));
    assert_eq!(get("wait-ho").verdict, Verdict::Waiting);
    assert_eq!(get("wait-sess").verdict, Verdict::Waiting);
    assert_eq!(get("wait-sess").session, SessionNow::NeedsYou);
    assert_eq!(get("run").verdict, Verdict::Running);
    assert_eq!(get("run").session, SessionNow::Running);
    let co = get("commit-only");
    assert_eq!(co.verdict, Verdict::NoHandoff);
    assert_eq!(co.commits, 1);
    assert_eq!(co.recent_commits[0].subject, "加上新檔");
    let idle = get("idle");
    assert_eq!(idle.verdict, Verdict::Idle);
    assert_eq!(idle.commits, 0);
    assert_eq!(idle.dirty, 1);
    assert_eq!(idle.session, SessionNow::None);

    // 照結論排
    assert_eq!(r.rows[0].verdict, Verdict::Done);
    assert_eq!(r.rows.last().unwrap().verdict, Verdict::Idle);
    let tot = &r.total;
    assert_eq!(
        (
            tot.repos,
            tot.done,
            tot.waiting,
            tot.running,
            tot.no_handoff,
            tot.idle
        ),
        (6, 1, 2, 1, 1, 1)
    );
    assert_eq!(tot.commits, 1);
    assert_eq!(tot.handoffs, 3);
    assert!(tot.line.starts_with("6 個 repo："), "{}", tot.line);
    assert!(
        tot.line.contains("共 3 筆 handoff、1 個 commit"),
        "{}",
        tot.line
    );

    // 指定 repo：照列；不認得的 since 是錯誤
    let only = relay::report(&d, &RelayScope::Repos(vec!["done".into()]), &since).unwrap();
    assert_eq!(only.rows.len(), 1);
    assert!(relay::report(&d, &RelayScope::All, "上週").is_err());
}

/// 寫一份假的 Claude Code 逐字稿（`history::past_sessions` 讀得到的格式），內容含 `extra`。
fn transcript(root: &Path, cwd: &Path, id: &str, started: &str, extra: &str) {
    let dir = root.join(ctx_core::history::project_dir_name(cwd));
    std::fs::create_dir_all(&dir).unwrap();
    let rows = [
        json!({"type":"user","entrypoint":"cli","cwd":cwd,"timestamp":started,
               "gitBranch":"main","message":{"content":"接著做"}}),
        json!({"type":"user","message":{"content":[{"type":"tool_result","content":extra}]}}),
        json!({"type":"assistant","message":{"content":[{"type":"text","text":"交接好了"}]}}),
    ];
    let text: Vec<String> = rows.iter().map(|r| r.to_string()).collect();
    let f = dir.join(format!("{id}.jsonl"));
    std::fs::write(&f, text.join("\n") + "\n").unwrap();
    // 最後活動設成 10 分鐘前（兩分鐘內還在寫的會被當成可能還開著）
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(600);
    std::fs::File::options()
        .write(true)
        .open(&f)
        .unwrap()
        .set_modified(old)
        .unwrap();
}

/// 照 `agentapi::handoff` 寫一筆有 id 的交接，回傳 ref。
fn handoff_with_id(d: &Path, repo: &str, when: NaiveDateTime, id: &str, next: &str) -> String {
    let body = format!("交接：做完 Z\n完成：做完 Z\n剩下：無\n下次從這裡接：{next}");
    spine::append_event(
        d,
        "decision",
        "agent",
        &[format!("id:{id}"), format!("repo:{repo}")],
        &body,
        Some(when),
    )
    .unwrap();
    format!("decision:{id}")
}

#[test]
fn answer_delivery_kinds() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let root = t.join("claude-projects");
    let when = ago(0, 3);
    let utc = |x: NaiveDateTime| {
        x.and_local_timezone(Local)
            .unwrap()
            .with_timezone(&chrono::Utc)
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    };

    // 開著的會話（等你）→ 在那場會話貼上
    let open = register(&d, t, "open", 6);
    handoff(&d, "open", when, "草稿", "等你挑方案");
    session(&d, "t1", &open, StatusHook::Permission);
    // 逐字稿裡有這筆交接的 ref → resume
    let res = register(&d, t, "res", 6);
    let rf = handoff_with_id(&d, "res", when, "2026-01-01-d7", "等使用者決定 A 或 B");
    transcript(
        &root,
        &res,
        "aaaaaaaa-1111-2222-3333-444444444444",
        &utc(when - Duration::hours(1)),
        &format!("{{\"ok\":true,\"ref\":\"{rf}\"}}"),
    );
    // 時間對得上、但逐字稿裡沒有這筆 ref → 不猜，開新會話
    let other = register(&d, t, "other", 6);
    handoff_with_id(&d, "other", when, "2026-01-01-d8", "等你決定");
    transcript(
        &root,
        &other,
        "bbbbbbbb-1111-2222-3333-444444444444",
        &utc(when - Duration::hours(1)),
        "{\"ok\":true,\"ref\":\"decision:2026-01-01-d9\"}",
    );
    // 交接沒有 id（ref 只是時間）→ 不猜
    register(&d, t, "noid", 6);
    handoff(&d, "noid", when, "草稿", "等你決定");

    let before = event_count(&d);
    let a = relay::answer_with(&d, "open", "選方案 B", Some(&root)).unwrap();
    assert!(
        matches!(&a.delivery, Delivery::ExistingSession { sid, state } if sid == "t1" && *state == SessionNow::NeedsYou)
    );
    assert!(a.commands.is_empty());
    assert!(a.task.contains("使用者的回覆：選方案 B"), "{}", a.task);
    assert!(a.task.contains("下次從這裡接：等你挑方案"), "{}", a.task);
    assert!(a.task.contains("不要自己猜"), "{}", a.task);

    let r = relay::answer_with(&d, "res", "選 A", Some(&root)).unwrap();
    match &r.delivery {
        Delivery::Resume {
            claude_session_id,
            cwd,
            agent,
        } => {
            assert_eq!(*agent, "claude");
            assert_eq!(claude_session_id, "aaaaaaaa-1111-2222-3333-444444444444");
            assert_eq!(Path::new(cwd), res.as_path());
        }
        other => panic!("應該 resume：{other:?}"),
    }
    assert!(
        r.commands[0].contains("claude --resume aaaaaaaa-1111-2222-3333-444444444444 '"),
        "{}",
        r.commands[0]
    );
    assert_eq!(r.handoff_ref, rf);

    let o = relay::answer_with(&d, "other", "好", Some(&root)).unwrap();
    assert!(
        matches!(o.delivery, Delivery::NewSession { .. }),
        "{:?}",
        o.delivery
    );
    assert!(
        o.commands[0].starts_with("ctx session --repos other --repo other --task '"),
        "{}",
        o.commands[0]
    );
    let n = relay::answer_with(&d, "noid", "好", Some(&root)).unwrap();
    assert!(matches!(n.delivery, Delivery::NewSession { .. }));
    assert_eq!(event_count(&d), before, "answer 不寫事件");

    // JSON：送達方式帶 kind
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["delivery"]["kind"], "resume");

    // 交接摘要很長也不超過約 800 字；回覆經過 redact
    let long = "很長的下一步".repeat(300);
    handoff(&d, "noid", ago(0, 1), &"完成很多事".repeat(200), &long);
    let secret = "sk-ant-FAKEFAKEFAKEFAKE"; // 假的憑證樣式，只用來測遮蔽
    let l = relay::answer_with(&d, "noid", &format!("用這把 key {secret}"), Some(&root)).unwrap();
    assert!(l.task.chars().count() <= 900, "{}", l.task.chars().count());
    assert!(!l.task.contains(secret));
    assert!(l.redacted >= 1);
}

#[test]
fn answer_errors() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let code = |e: ctx_core::Error| match e {
        ctx_core::Error::Rejected { code, .. } => code.as_str(),
        other => panic!("應該是 Rejected：{other:?}"),
    };
    register(&d, t, "none", 6);
    register(&d, t, "gone", 6);
    handoff(&d, "gone", ago(1, 0), "做完", "等你決定");
    std::fs::remove_dir_all(t.join("gone")).unwrap();
    let busy = register(&d, t, "busy", 6);
    handoff(&d, "busy", ago(1, 0), "做完", "等你決定");
    session(&d, "t1", &busy, StatusHook::ToolStart);

    assert_eq!(
        code(relay::answer(&d, "nope", "好").unwrap_err()),
        "not_found"
    );
    assert_eq!(
        code(relay::answer(&d, "gone", "好").unwrap_err()),
        "not_found"
    );
    assert_eq!(
        code(relay::answer(&d, "none", "好").unwrap_err()),
        "not_found"
    );
    assert_eq!(code(relay::answer(&d, "busy", "好").unwrap_err()), "busy");
    assert_eq!(
        code(relay::answer(&d, "busy", "  ").unwrap_err()),
        "bad_request"
    );
}
