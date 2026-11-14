//! F-122 接力：plan 的每種狀態、略過參考類／封存、排序、純讀取；report 的每種結論與總計。

use std::path::{Path, PathBuf};

use chrono::{Duration, Local, NaiveDateTime};
use ctx_core::registry::{add_group, add_repo, set_class, set_field, AddRepo, RepoClass};
use ctx_core::relay::{self, RelayScope, RelayStatus, SessionNow, Verdict};
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
    assert_eq!(row(&p, "busy-ask").status, RelayStatus::Busy);
    assert!(row(&p, "busy-ask").notes[0].contains("等你"));
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
