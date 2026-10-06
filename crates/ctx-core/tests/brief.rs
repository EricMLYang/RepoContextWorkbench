//! 對應 `tests/test_collect_brief.py` 裡 **brief** 相關的測試（collect 的在 tests/collect.rs）。

use std::path::Path;

use ctx_core::brief::{self, QKind};
use ctx_core::collect::{self, RepoSpec, RepoState};
use ctx_core::registry::{self, AddRepo, Repo};
use ctx_core::spine::{self, EventType, Query};
use ctx_core::testkit::{make_git_repo, spine, spine_with_repos, today_at};
use ctx_core::{config, Error};

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn specs(entries: &[Repo]) -> Vec<RepoSpec> {
    entries
        .iter()
        .map(|r| {
            RepoSpec::new(r.id(), r.path())
                .with_type(r.kind().unwrap_or_else(|| "mine".into()))
                .with_tier(r.tier_str().unwrap_or_else(|| "active".into()))
        })
        .collect()
}

fn group_states(d: &Path, g: &str) -> Vec<RepoState> {
    let (_, entries) = registry::resolve_group(d, Some(g), None).unwrap();
    collect::collect_group(&specs(&entries), None)
}

fn age_file(p: &Path, days: u64) {
    let when = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 86400);
    std::fs::File::options()
        .write(true)
        .open(p)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn test_brief_shape() {
    let (_t, d) = spine_with_repos();
    // 加一條今天到期的 open loop
    spine::append_event(
        &d,
        "open-loop",
        "hotkey",
        &["#12".to_string(), format!("due:{}", today())],
        "opened →「驗證 X」",
        Some(today_at(8, 0)),
    )
    .unwrap();
    let (out, text) = brief::run(&d, Some("g1"), None, None).unwrap();
    assert!(out.exists());
    // 三段固定結構
    assert!(text.contains("## 需要你判斷的"));
    assert!(text.contains("## 未結"));
    assert!(text.contains("## 沉默摘要"));
    // dirty 5 天 >= 閾值 4 → 必須出現且是問句＋出口
    assert!(text.contains("repo-b") && text.contains("嗎？") && text.contains("〔"));
    // 乾淨的 repo-a 不該出現在判斷段（無事不報）
    let judge_section = text.split("## 未結").next().unwrap();
    assert!(!judge_section.contains("repo-a"));
    // 到期 loop 有列出
    assert!(text.contains("#12") && text.contains("今天到期"));
    // 留痕：presented 事件寫進脊椎
    let evs = spine::query(&d, &Query::of_type(EventType::Presented)).unwrap();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].source, "morning-brief");
}

#[test]
fn test_brief_quiet_when_all_clean() {
    let (t, d) = spine();
    let p = make_git_repo(t.path(), "calm-repo", 1, 0, true);
    registry::add_repo(&d, "calm-repo", &p, AddRepo::default()).unwrap();
    registry::add_group(&d, "calm", &["calm-repo"]).unwrap();
    let (_, text) = brief::run(&d, Some("calm"), None, None).unwrap();
    assert!(text.contains("今天沒有需要判斷的事"));
}

#[test]
fn test_brief_over_three_questions_warns() {
    let (t, d) = spine();
    let mut ids = Vec::new();
    for i in 0..4 {
        let name = format!("noisy-{i}");
        let p = make_git_repo(t.path(), &name, 3, 1, true);
        age_file(&p.join("wip0.md"), 6);
        registry::add_repo(&d, &name, &p, AddRepo::default()).unwrap();
        ids.push(name);
    }
    registry::add_group(&d, "noisy", &ids).unwrap();
    let (_, text) = brief::run(&d, Some("noisy"), None, None).unwrap();
    assert!(text.contains("> 3——過濾閾值該修了")); // 免費校準資料要浮出
}

/// 問句改結構化卡：文字版（簡報 md）由卡渲染而來，兩者不會分岔。
#[test]
fn test_question_cards_structured() {
    let (_t, d) = spine_with_repos();
    let states = group_states(&d, "g1");
    let th = config::load(&d).unwrap().thresholds();
    let cards = brief::build_question_cards(&states, &th);
    assert_eq!(
        cards.iter().map(|c| c.repo.as_str()).collect::<Vec<_>>(),
        vec!["repo-b"]
    );
    let c = &cards[0];
    assert_eq!(c.qkind, QKind::Dirty);
    assert!(c.title.contains("嗎？"));
    assert!(!c.title.contains("dirty")); // 內部術語不丟給人看（§8）
    assert_eq!(
        c.actions
            .iter()
            .map(|a| a.label.as_str())
            .collect::<Vec<_>>(),
        vec!["與 Agent 一起收尾", "七天後再提醒"]
    );
    assert_eq!(c.actions[0].action, "term_create");
    assert_eq!(c.actions[0].payload["repo"], "repo-b");
    assert_eq!(c.actions[1].action, "defer");
    let texts = brief::build_questions(&states, &th);
    assert_eq!(
        texts,
        cards.iter().map(brief::card_text).collect::<Vec<_>>()
    );
    assert!(texts[0].contains("〔與 Agent 一起收尾〕〔七天後再提醒〕"));
    // 序列化形狀＝Python dict（給 MCP／工作台）
    let v = serde_json::to_value(c).unwrap();
    assert_eq!(v["kind"], "question");
    assert_eq!(v["qkind"], "dirty");
    assert_eq!(v["key"], "q:dirty:repo-b");
}

/// 2026-09-12 §6.1：「本週排進度」其實只呼叫 defer(days=7)，沒有安排任何進度。
/// 文案改成「七天後再提醒」——按鈕說什麼，程式就做什麼。
#[test]
fn test_defer_label_matches_behaviour() {
    let (t, d) = spine();
    let p = make_git_repo(t.path(), "stale-repo", 40, 0, true);
    registry::add_repo(&d, "stale-repo", &p, AddRepo::default()).unwrap();
    let entry = registry::get_repo(&d, "stale-repo").unwrap();
    let states = collect::collect_group(&specs(&[entry]), None);
    let cards = brief::build_question_cards(&states, &config::load(&d).unwrap().thresholds());
    let card = cards.iter().find(|c| c.qkind == QKind::Inactive).unwrap();
    assert!(!card.actions.iter().any(|a| a.label == "本週排進度"));
    let defer = card.actions.iter().find(|a| a.action == "defer").unwrap();
    assert_eq!(defer.label, "七天後再提醒");
    assert_eq!(defer.payload["days"], 7);
    // 主要動作＝先看變化（工作），不是維護登記狀態（§3）
    assert_eq!(card.actions[0].action, "term_create");
    assert_eq!(
        card.actions
            .iter()
            .map(|a| a.action.as_str())
            .collect::<Vec<_>>(),
        vec!["term_create", "tier", "defer"]
    );
}

/// 2026-09-12 §6.2：組別簡報不得混入其他組的待辦；全域／未分類獨立標示。
#[test]
fn test_brief_loops_stay_inside_the_named_group() {
    let (t, d) = spine();
    let today = today();
    for name in ["prod", "notes"] {
        let p = make_git_repo(t.path(), name, 1, 0, true);
        registry::add_repo(&d, name, &p, AddRepo::default()).unwrap();
    }
    registry::add_group(&d, "產品開發", &["prod"]).unwrap();
    registry::add_group(&d, "知識研究", &["notes"]).unwrap();
    spine::append_event(
        &d,
        "open-loop",
        "hotkey",
        &[
            "#1".to_string(),
            "group:產品開發".into(),
            format!("due:{today}"),
        ],
        "確認規劃與實作的驗收條件是否一致",
        None,
    )
    .unwrap();
    // 全域：沒有 group／repo token
    spine::append_event(
        &d,
        "open-loop",
        "hotkey",
        &["#2".to_string(), format!("due:{today}")],
        "備份脊椎",
        None,
    )
    .unwrap();
    let (_, text) = brief::run(&d, Some("知識研究"), None, None).unwrap();
    assert!(!text.contains("驗收條件")); // 別組的待辦不進來
    assert!(text.contains("備份脊椎") && text.contains("全域／未分類")); // 全域的保留但獨立標示
    let own_section = text.split("## 未結（全域").next().unwrap();
    assert!(!own_section.contains("備份脊椎"));
    let (_, prod) = brief::run(&d, Some("產品開發"), None, None).unwrap();
    assert!(prod.contains("驗收條件"));
    // 沉默摘要只講採集支持得起的話
    assert!(prod.contains("未觸發提醒條件") && !prod.contains("無異常、無變化"));
}

#[test]
fn test_brief_has_pulse_line() {
    let (_t, d) = spine_with_repos();
    let (_, text) = brief::run(&d, Some("g1"), None, None).unwrap();
    assert!(text.contains("## 脈動（一行）") && text.contains("近 7 天"));
    // 脈動是資訊不是問句：判斷段仍只有 dirty 那一條
    let judge = text.split("## 未結").next().unwrap();
    assert_eq!(judge.matches("嗎？").count(), 1);
}

/// 不存在的組＝Invalid（Python ValueError）。Python 沒有這條測試；守 brief::run 的錯誤型別。
#[test]
fn brief_unknown_group_is_invalid() {
    let (_t, d) = spine_with_repos();
    assert!(matches!(
        brief::run(&d, Some("nope"), None, None),
        Err(Error::Invalid(_))
    ));
}

/// snoozed：defer 留痕（webui actions.defer 寫的 chosen 事件）→ (repo, qkind)，過期的不算。
/// Python 沒有獨立測試（經 webui／agentapi 測）；在引擎層先釘住。
#[test]
fn snoozed_reads_defer_events() {
    let (_t, d) = spine();
    spine::append_event(
        &d,
        "chosen",
        "monitor",
        &["repo:repo-b"],
        "7 天後再提醒（repo-b 的「未提交檔案放太久」）\nsnooze:dirty until:2099-01-01",
        None,
    )
    .unwrap();
    spine::append_event(
        &d,
        "chosen",
        "monitor",
        &["repo:repo-a"],
        "snooze:inactive until:2000-01-01",
        None,
    )
    .unwrap();
    let s = brief::snoozed(&d, None).unwrap();
    assert!(s.contains(&("repo-b".to_string(), "dirty".to_string())));
    assert_eq!(s.len(), 1);
}
