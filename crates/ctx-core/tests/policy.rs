//! F-124 放行規則：沒規則永遠 wait；不能自動的類別／不可逆／沒建議即使手改 policy.yaml 也 wait；policy.yaml 壞掉不放行
//! 且報得出來；連續照建議出升級建議；trust 後放行並留痕；handoff 帶 ask 的放行；paths；抽查；overturn；人答得不同就降級；
//! policy.yaml 寫兩次位元組相同、並行寫不壞檔。

use std::path::{Path, PathBuf};

use chrono::{Duration, Local, NaiveDateTime};
use ctx_core::agentapi::{self as api, Target};
use ctx_core::ask::{AskInput, AskKind};
use ctx_core::decide::{self, AnsweredBy, FollowRec};
use ctx_core::policy::{self, Decision, Level};
use ctx_core::registry::{add_repo, AddRepo};
use ctx_core::relay::{self, RelayScope, RelayStatus};
use ctx_core::spine;
use ctx_core::testkit::{make_git_repo, spine};
use ctx_core::Error;

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

fn ago_h(hours: i64) -> NaiveDateTime {
    now() - Duration::hours(hours)
}

fn register(d: &Path, tmp: &Path, id: &str, dirty: usize) -> PathBuf {
    let p = make_git_repo(tmp, id, 6, dirty, false);
    add_repo(d, id, &p, AddRepo::default()).unwrap();
    p
}

fn ask(kind: &str) -> AskInput {
    AskInput {
        question: "要 commit 這次的改動嗎？".into(),
        kind: Some(kind.into()),
        options: vec!["要".into(), "先不要".into()],
        recommend: Some("1".into()),
        reversible: None,
        evidence: vec![],
    }
}

fn request(d: &Path, repo_path: &Path, a: &AskInput) -> Decision {
    decide::request(d, &Target::at(repo_path), a, false).unwrap()
}

fn wait_code(dec: &Decision) -> &'static str {
    match dec {
        Decision::Wait { reason } => reason.code,
        Decision::Auto(a) => panic!("不該放行：{a:?}"),
    }
}

fn auto_ref(dec: &Decision) -> String {
    match dec {
        Decision::Auto(a) => {
            assert_eq!(a.answer, "要");
            a.reference.clone()
        }
        Decision::Wait { reason } => panic!("應該放行：{reason:?}"),
    }
}

/// 照 agentapi::handoff 的格式在指定時間寫一筆帶提問的交接（不經過放行判定）。
fn handoff_at(d: &Path, repo: &str, when: NaiveDateTime, id: &str, a: &AskInput) {
    let mut lines = vec![
        "交接：做完一段".to_string(),
        "完成：做完一段".to_string(),
        "剩下：無".to_string(),
        "下次從這裡接：等使用者決定要不要 commit".to_string(),
    ];
    lines.extend(a.validate().unwrap().body_lines());
    spine::append_event(
        d,
        "decision",
        "agent",
        &[format!("id:{id}"), format!("repo:{repo}")],
        &lines.join("\n"),
        Some(when),
    )
    .unwrap();
}

fn root(tmp: &Path) -> PathBuf {
    tmp.join("claude-projects")
}

/// 連續 n 次「交接帶提問 → 人用編號回答」。
fn answer_n(d: &Path, tmp: &Path, repo: &str, replies: &[&str]) -> Vec<decide::Decided> {
    let n = replies.len() as i64;
    replies
        .iter()
        .enumerate()
        .map(|(i, r)| {
            handoff_at(
                d,
                repo,
                ago_h(n - i as i64 + 1),
                &format!("2026-01-01-d{repo}{i}"),
                &ask("commit"),
            );
            decide::answer_with(d, repo, r, Some(&root(tmp))).unwrap()
        })
        .collect()
}

fn write_policy(d: &Path, text: &str) {
    std::fs::write(d.join(policy::FILE), text).unwrap();
}

#[test]
fn no_rule_always_waits() {
    let (tmp, d) = spine();
    let a = register(&d, tmp.path(), "repo-a", 0);
    let before = spine::iter_events(&d).unwrap().len();
    assert_eq!(wait_code(&request(&d, &a, &ask("commit"))), "no_rule");
    assert_eq!(spine::iter_events(&d).unwrap().len(), before, "wait 不留痕");
}

#[test]
fn hand_edited_auto_never_releases_unsafe_asks() {
    let (tmp, d) = spine();
    let a = register(&d, tmp.path(), "repo-a", 0);
    let since = now().format("%Y-%m-%d %H:%M").to_string();
    let mut text = String::from("rules:\n");
    for k in ["push", "delete", "visual_check", "other", "commit"] {
        text.push_str(&format!(
            "- repo: repo-a\n  kind: {k}\n  level: auto\n  since: '{since}'\n"
        ));
    }
    write_policy(&d, &text);
    for (k, code) in [
        ("push", "kind_not_automatable"),
        ("delete", "kind_not_automatable"),
        ("visual_check", "kind_not_automatable"),
        ("other", "kind_not_automatable"),
    ] {
        let dec = request(&d, &a, &ask(k));
        assert_eq!(wait_code(&dec), code, "{k}");
        if let Decision::Wait { reason } = dec {
            assert!(
                reason.message.contains("永遠要人回答"),
                "{}",
                reason.message
            );
        }
    }
    let irreversible = AskInput {
        reversible: Some(false),
        ..ask("commit")
    };
    assert_eq!(wait_code(&request(&d, &a, &irreversible)), "irreversible");
    let no_rec = AskInput {
        recommend: None,
        ..ask("commit")
    };
    assert_eq!(wait_code(&request(&d, &a, &no_rec)), "no_recommendation");
    // commit 那條是好的：照樣放行
    auto_ref(&request(&d, &a, &ask("commit")));
    // 手改成 auto 的不能自動類別在 decide rules 被點名
    let r = decide::rules_report(&d).unwrap();
    assert_eq!(r.rules.len(), 1);
    for k in ["push", "delete", "visual_check", "other"] {
        assert!(
            r.problems
                .iter()
                .any(|p| p.contains(&format!("repo-a/{k}"))),
            "{k}：{:?}",
            r.problems
        );
    }
}

#[test]
fn broken_policy_never_releases_and_is_reported() {
    let (tmp, d) = spine();
    let a = register(&d, tmp.path(), "repo-a", 0);
    write_policy(&d, "rules: [this is: not: yaml\n");
    match request(&d, &a, &ask("commit")) {
        Decision::Wait { reason } => {
            assert_eq!(reason.code, "no_rule");
            assert!(reason.message.contains("寫壞"), "{}", reason.message);
        }
        other => panic!("{other:?}"),
    }
    assert!(!decide::rules_report(&d).unwrap().problems.is_empty());
    // 整份壞掉時 trust 拒絕（不覆蓋人手寫的）
    match decide::trust(&d, "repo-a", "commit", None).unwrap_err() {
        Error::Rejected { code, .. } => assert_eq!(code.as_str(), "failed"),
        other => panic!("{other:?}"),
    }
    let since = now().format("%Y-%m-%d %H:%M").to_string();
    write_policy(
        &d,
        &format!(
            "rules:\n- repo: repo-a\n  kind: commit\n  level: sometimes\n  since: '{since}'\n\
             - repo: ghost\n  kind: choose\n  level: auto\n  since: '{since}'\n\
             - repo: repo-a\n  kind: choose\n  level: auto\n  since: yesterday\n\
             - repo: repo-a\n  kind: config_change\n  level: auto\n  since: '{since}'\n  paths: [/etc]\n"
        ),
    );
    assert_eq!(wait_code(&request(&d, &a, &ask("commit"))), "no_rule");
    assert_eq!(wait_code(&request(&d, &a, &ask("choose"))), "no_rule");
    assert_eq!(
        wait_code(&request(&d, &a, &ask("config_change"))),
        "no_rule"
    );
    let r = decide::rules_report(&d).unwrap();
    assert!(r.rules.is_empty());
    let all = r.problems.join("\n");
    for want in ["sometimes", "ghost", "yesterday", "paths"] {
        assert!(all.contains(want), "缺 {want}：{all}");
    }
    // health 也說得出來
    let h = decide::health_stats(&d, 14, now()).unwrap();
    assert_eq!(h.policy_problems.len(), 4);
    // 同一個（repo、類別）兩條：都不生效
    write_policy(
        &d,
        &format!(
            "rules:\n- repo: repo-a\n  kind: commit\n  level: auto\n  since: '{since}'\n\
             - repo: repo-a\n  kind: commit\n  level: auto\n  since: '{since}'\n"
        ),
    );
    assert_eq!(wait_code(&request(&d, &a, &ask("commit"))), "no_rule");
    assert!(decide::rules_report(&d).unwrap().problems[0].contains("2 條"));
}

#[test]
fn streak_suggests_and_differed_resets() {
    let (tmp, d) = spine();
    let t = tmp.path();
    register(&d, t, "good", 0);
    register(&d, t, "mixed", 0);
    let rs = answer_n(&d, t, "good", &["1", "1", "1"]);
    assert!(rs[0].suggestion.is_none() && rs[1].suggestion.is_none());
    let s = rs[2].suggestion.as_ref().expect("第 3 次出升級建議");
    assert_eq!(
        (s.repo.as_str(), s.kind, s.streak),
        ("good", AskKind::Commit, 3)
    );
    assert_eq!(s.command, "ctx decide trust good commit");
    let rs = answer_n(&d, t, "mixed", &["1", "1", "2", "1", "1"]);
    assert_eq!(rs[2].answered.follow, FollowRec::Differed);
    assert!(rs.iter().all(|r| r.suggestion.is_none()));
    let q = decide::list(&d, &RelayScope::All).unwrap();
    assert_eq!(
        q.suggestions
            .iter()
            .map(|h| h.repo.as_str())
            .collect::<Vec<_>>(),
        ["good"]
    );
    assert_eq!(decide::rules_report(&d).unwrap().suggestions.len(), 1);
}

#[test]
fn trust_then_request_auto_and_record() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let a = register(&d, t, "repo-a", 0);
    answer_n(&d, t, "repo-a", &["1", "1", "1"]);
    let h0 = decide::history(&d).unwrap();
    let streak0 = ctx_core::decision_log::streak(&h0, "repo-a", AskKind::Commit, None);
    let tr = decide::trust(&d, "repo-a", "commit", None).unwrap();
    assert_eq!(tr.previous, None);
    assert_eq!(tr.rule.level, Level::Auto);
    assert_eq!(
        (tr.past.answers, tr.past.followed, tr.past.differed),
        (3, 3, 0)
    );
    let r = auto_ref(&request(&d, &a, &ask("commit")));
    let ev = spine::iter_events(&d).unwrap().pop().unwrap();
    assert_eq!(ev.reference(), r);
    assert_eq!(ev.source, "policy");
    assert!(spine::is_decide_record(&ev));
    assert!(ev.kv("ref").is_none(), "做到一半問的沒有交接可指");
    let h = decide::history(&d).unwrap();
    let last = h.last().unwrap();
    assert_eq!(last.by, AnsweredBy::Rule);
    assert_eq!(last.rule.as_deref(), Some("repo-a/commit"));
    // 不算人的回答、不影響連續計數
    assert_eq!(
        ctx_core::decision_log::streak(&h, "repo-a", AskKind::Commit, None),
        streak0
    );
    assert_eq!(h.iter().filter(|x| x.by == AnsweredBy::Human).count(), 3);
    // 不進 agent 的最近事件
    let ctx = api::context_for(&d, &Target::at(&a), true).unwrap();
    assert!(!serde_json::to_string(&ctx["recent_events"])
        .unwrap()
        .contains(spine::DECIDE_TAG));
    assert!(!ctx["card"].as_str().unwrap().contains(spine::DECIDE_TAG));
    // 已經是 auto：不再出升級建議
    assert!(decide::rules_report(&d).unwrap().suggestions.is_empty());
    let l = decide::auto_list(&d, None).unwrap();
    assert_eq!(l.items.len(), 1);
    assert_eq!(
        l.items[0].command.as_deref(),
        Some(format!("ctx decide overturn {r} --why \"…\"").as_str())
    );
    let rr = decide::rules_report(&d).unwrap();
    assert_eq!((rr.rules[0].approvals, rr.rules[0].overturned), (1, 0));
}

#[test]
fn handoff_with_ask_auto_released() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let a = register(&d, t, "repo-a", 0);
    decide::trust(&d, "repo-a", "commit", None).unwrap();
    let h = api::handoff_with_ask(
        &d,
        "寫完",
        "等使用者決定要不要 commit",
        &[] as &[&str],
        Some(&ask("commit")),
        &Target::at(&a),
        false,
        "agent",
    )
    .unwrap();
    let auto = h.auto.as_ref().expect("交接回傳多 auto");
    assert_eq!(auto.answer, "要");
    let v = serde_json::to_value(&h).unwrap();
    assert_eq!(v["auto"]["rule"], "repo-a/commit");
    let rec = spine::iter_events(&d)
        .unwrap()
        .into_iter()
        .find(|e| e.reference() == auto.reference)
        .unwrap();
    assert_eq!(rec.kv("ref"), Some(h.reference.as_str()));
    // 交接算已回答：不出卡；relay plan 可接、任務帶答案
    assert!(decide::list(&d, &RelayScope::All).unwrap().cards.is_empty());
    let p = relay::plan(&d, &RelayScope::All).unwrap();
    assert_eq!(p.rows[0].status, RelayStatus::Ready);
    assert_eq!(p.waiting_on_you, 0);
    assert_eq!(p.rows[0].answered.as_deref(), Some(auto.reference.as_str()));
    let task = p.rows[0].prompt.as_deref().unwrap();
    assert!(task.contains("的回答是：要"), "{task}");
    // 推翻 → 卡回到佇列
    let o = decide::overturn(&d, &auto.reference, Some("其實要先跑測試")).unwrap();
    assert_eq!(o.rule.as_ref().unwrap().level, Level::Ask);
    let q = decide::list(&d, &RelayScope::All).unwrap();
    assert_eq!(q.cards.len(), 1);
    assert_eq!(relay::plan(&d, &RelayScope::All).unwrap().waiting_on_you, 1);
    // 沒規則的交接不回 auto 鍵
    let plain = api::handoff_with_ask(
        &d,
        "寫完",
        "等使用者決定",
        &[] as &[&str],
        Some(&ask("push")),
        &Target::at(&a),
        false,
        "agent",
    )
    .unwrap();
    assert!(serde_json::to_value(&plain).unwrap().get("auto").is_none());
}

#[test]
fn paths_are_checked_by_git() {
    let (tmp, d) = spine();
    let t = tmp.path();
    // dirty＝根目錄的 wip0.md（不在 docs/ 底下）
    let messy = register(&d, t, "messy", 1);
    let tidy = register(&d, t, "tidy", 0);
    std::fs::create_dir_all(tidy.join("docs/sub")).unwrap();
    std::fs::write(tidy.join("docs/sub/new.md"), "x\n").unwrap();
    std::fs::write(tidy.join("README.md"), "改了\n").unwrap();
    for r in ["messy", "tidy"] {
        decide::trust(&d, r, "commit", Some(vec!["docs/".into()])).unwrap();
    }
    match request(&d, &messy, &ask("commit")) {
        Decision::Wait { reason } => {
            assert_eq!(reason.code, "outside_paths");
            assert!(reason.message.contains("wip0.md"), "{}", reason.message);
        }
        other => panic!("{other:?}"),
    }
    // README 改了 → 也在外面
    assert_eq!(
        wait_code(&request(&d, &tidy, &ask("commit"))),
        "outside_paths"
    );
    ctx_core::testkit::git(&tidy, &["checkout", "--", "README.md"], &[]);
    auto_ref(&request(&d, &tidy, &ask("commit")));
    // 路徑不在
    std::fs::remove_dir_all(&messy).unwrap();
    let dec = decide::request(
        &d,
        &Target::at(t).with_repos(&["messy"]),
        &ask("commit"),
        false,
    )
    .unwrap();
    assert_eq!(wait_code(&dec), "repo_missing");
}

#[test]
fn tenth_is_spot_check() {
    let (tmp, d) = spine();
    let a = register(&d, tmp.path(), "repo-a", 0);
    decide::trust(&d, "repo-a", "commit", None).unwrap();
    for _ in 0..9 {
        auto_ref(&request(&d, &a, &ask("commit")));
    }
    match request(&d, &a, &ask("commit")) {
        Decision::Wait { reason } => {
            assert_eq!(reason.code, "spot_check");
            assert!(reason.message.contains("第 10 次"), "{}", reason.message);
        }
        other => panic!("{other:?}"),
    }
    // 決定性：沒人回答之前一直是抽查
    assert_eq!(wait_code(&request(&d, &a, &ask("commit"))), "spot_check");
}

#[test]
fn overturn_downgrades_and_resets() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let a = register(&d, t, "repo-a", 0);
    answer_n(&d, t, "repo-a", &["1", "1", "1"]);
    decide::trust(&d, "repo-a", "commit", None).unwrap();
    let r = auto_ref(&request(&d, &a, &ask("commit")));
    let o = decide::overturn(&d, &r, Some("不該自動")).unwrap();
    assert_eq!(o.rule.as_ref().unwrap().level, Level::Ask);
    assert_eq!(
        o.approval.overturned_by.as_deref(),
        Some(o.reference.as_str())
    );
    assert_eq!(wait_code(&request(&d, &a, &ask("commit"))), "rule_is_ask");
    // since 更新：連續計數從頭算，不會馬上又跳建議
    let rr = decide::rules_report(&d).unwrap();
    assert!(rr.suggestions.is_empty());
    assert_eq!(rr.rules[0].overturned, 1);
    // 推翻兩次、推翻人的回答都拒絕
    assert!(decide::overturn(&d, &r, None).is_err());
    let human = decide::history(&d).unwrap()[0].reference.clone();
    match decide::overturn(&d, &human, None).unwrap_err() {
        Error::Rejected { code, .. } => assert_eq!(code.as_str(), "bad_request"),
        other => panic!("{other:?}"),
    }
    match decide::overturn(&d, "chosen:nope", None).unwrap_err() {
        Error::Rejected { code, .. } => assert_eq!(code.as_str(), "not_found"),
        other => panic!("{other:?}"),
    }
    // 推翻也是工具留痕：不是人的回答
    let h = decide::history(&d).unwrap();
    assert_eq!(h.iter().filter(|x| x.by == AnsweredBy::Human).count(), 3);
}

#[test]
fn human_differed_downgrades_auto_rule() {
    let (tmp, d) = spine();
    let t = tmp.path();
    // 有個檔在 paths 外 → 交接的提問不會被放行、成為卡
    let a = register(&d, t, "repo-a", 1);
    decide::trust(&d, "repo-a", "commit", Some(vec!["docs/".into()])).unwrap();
    let h = api::handoff_with_ask(
        &d,
        "寫完",
        "等使用者決定要不要 commit",
        &[] as &[&str],
        Some(&ask("commit")),
        &Target::at(&a),
        false,
        "agent",
    )
    .unwrap();
    assert!(h.auto.is_none());
    let r = decide::answer_with(&d, "repo-a", "2", Some(&root(t))).unwrap();
    assert_eq!(r.answered.follow, FollowRec::Differed);
    let down = r.downgraded.expect("降回 ask");
    assert_eq!(down.level, Level::Ask);
    assert_eq!(down.paths, ["docs/"], "降級保留 paths");
    assert_eq!(
        policy::load(&d)
            .rule("repo-a", AskKind::Commit)
            .unwrap()
            .level,
        Level::Ask
    );
}

#[test]
fn trust_rejects_unsafe_kinds_and_bad_input() {
    let (tmp, d) = spine();
    register(&d, tmp.path(), "repo-a", 0);
    for k in [
        "push",
        "publish",
        "delete",
        "send_external",
        "visual_check",
        "manual_test",
        "other",
    ] {
        match decide::trust(&d, "repo-a", k, None).unwrap_err() {
            Error::Rejected { code, message, .. } => {
                assert_eq!(code.as_str(), "bad_request");
                assert!(message.contains("永遠要人回答"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }
    assert!(decide::trust(&d, "repo-a", "merge", None).is_err());
    assert!(decide::trust(&d, "ghost", "commit", None).is_err());
    assert!(decide::trust(&d, "repo-a", "commit", Some(vec!["../x".into()])).is_err());
    assert!(!d.join(policy::FILE).exists(), "被拒就不寫檔");
    // untrust 沒有規則也寫一條 ask
    let u = decide::untrust(&d, "repo-a", "push").unwrap();
    assert_eq!(u.rule.level, Level::Ask);
}

#[test]
fn policy_writes_are_stable_and_safe_concurrently() {
    let (tmp, d) = spine();
    for i in 0..6 {
        register(&d, tmp.path(), &format!("r{i}"), 0);
    }
    let at = NaiveDateTime::parse_from_str("2026-11-14 10:00", "%Y-%m-%d %H:%M").unwrap();
    policy::set_level(
        &d,
        "r0",
        AskKind::Commit,
        Level::Auto,
        Some(vec!["docs/".into()]),
        at,
    )
    .unwrap();
    let first = std::fs::read(d.join(policy::FILE)).unwrap();
    policy::set_level(&d, "r0", AskKind::Commit, Level::Auto, None, at).unwrap();
    assert_eq!(
        std::fs::read(d.join(policy::FILE)).unwrap(),
        first,
        "同一份資料位元組相同"
    );
    let text = String::from_utf8(first).unwrap();
    assert!(text.starts_with("# 放行規則"), "{text}");
    // 並行：6 個 repo × 4 類同時寫
    let handles: Vec<_> = (0..6)
        .map(|i| {
            let d = d.clone();
            std::thread::spawn(move || {
                for k in [
                    AskKind::Commit,
                    AskKind::Choose,
                    AskKind::ContentReview,
                    AskKind::ConfigChange,
                ] {
                    policy::set_level(&d, &format!("r{i}"), k, Level::Auto, None, now()).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let p = policy::load(&d);
    assert!(p.problems.is_empty(), "{:?}", p.problems);
    assert_eq!(p.rules.len(), 24);
    assert_eq!(
        p.rule("r0", AskKind::Commit).unwrap().paths,
        ["docs/"],
        "沒給 paths 就保留"
    );
}

/// SessionEnd：一場會話只拿到 request_decision 的自動放行（放行留痕不算這場會話的留痕）——
/// 沒 commit、沒改檔＝無事不報；有 commit 沒交接＝照樣記「沒有交接」，留痕數不含放行。
#[test]
fn session_end_ignores_policy_approvals() {
    use ctx_core::hooks;
    use serde_json::json;
    let (tmp, d) = spine();
    let a = register(&d, tmp.path(), "repo-a", 0);
    decide::trust(&d, "repo-a", "commit", None).unwrap();
    let start = now() - Duration::minutes(5);
    let cwd = a.to_string_lossy().to_string();
    hooks::session_start(&d, &json!({"session_id": "s1", "cwd": cwd}), Some(start));
    auto_ref(&request(&d, &a, &ask("commit")));
    assert!(
        hooks::session_end(&d, &json!({"session_id": "s1"}), None).is_none(),
        "只有放行留痕＝什麼都沒發生"
    );
    hooks::session_start(&d, &json!({"session_id": "s2", "cwd": cwd}), Some(start));
    auto_ref(&request(&d, &a, &ask("commit")));
    std::fs::write(a.join("feature.md"), "x\n").unwrap();
    ctx_core::testkit::git(&a, &["add", "-A"], &[]);
    ctx_core::testkit::git(&a, &["commit", "-q", "-m", "加功能"], &[]);
    let ev = hooks::session_end(&d, &json!({"session_id": "s2"}), None).expect("有 commit 沒交接");
    let body = ev["body"].as_str().unwrap();
    assert!(body.contains("沒有交接"), "{body}");
    assert!(body.contains("範圍內留痕 0 筆"), "放行不算留痕：{body}");
}
