//! 反芻輪（2026-09-25）：新進的知識與別組的決策，主動對上每組手上的工作。對應 tests/test_ruminate.py。
//!
//! 釘住：① 別組新進的 md 跟手上工作（目標／未結／判斷）強相關才推，自己組的不推；
//! ② 只看上次反芻之後的新料，同一份不重推；③ 寧缺勿濫（共同少見詞不夠多不推）；
//! ④ 別組新判斷也會對上；⑤ 被 read_from 讀了＝用上了（命中率）、可略過。
//!
//! Python 經 `api.add_todo`／`api.log_decision`／`api.read_from(cwd=…)` 的地方（寫在 agentapi 搬來之前）：
//! 這裡照它們落的事件形狀直接寫 spine（tokens＝`#n`／`id:` ＋ `group:<主場組>` ＋ `repo:<主場 repo>`），
//! read_from 則直接呼叫 `crossref::read` 並給出 cwd 會推出的引用方。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::Local;
use ctx_core::crossref::{self, MAX_READ_CHARS};
use ctx_core::registry::{self, AddRepo};
use ctx_core::ruminate::{self, LinkKind, LinkStatus};
use ctx_core::spine;
use ctx_core::testkit::{make_git_repo, spine};

const CARD: &str = "# 向量索引的冷啟動與增量重建

增量重建要記水位線，冷啟動時先載入快照再補增量。
倒排索引的分段合併策略會影響查詢延遲；合併太頻繁會拖垮寫入吞吐。
";
const UNRELATED: &str = "# 週末料理\n紅燒肉要先汆燙，再用冰糖炒色，小火慢燉一小時。\n";

struct World {
    _t: tempfile::TempDir,
    sp: PathBuf,
    impl_: PathBuf,
    cards: PathBuf,
}

/// 照 agentapi.log_decision 落的事件（主場＝repo 所屬的組）。
fn log_decision(sp: &Path, text: &str, group: &str, repo: &str, n: usize) {
    let today = Local::now().format("%Y-%m-%d");
    spine::append_event(
        sp,
        "decision",
        "agent",
        &[
            format!("id:{today}-d{n}"),
            format!("group:{group}"),
            format!("repo:{repo}"),
        ],
        text,
        None,
    )
    .unwrap();
}

/// 兩組：產品（impl，手上在做索引重建）、知識（cards，卡片 repo）。
fn world() -> World {
    let (t, sp) = spine();
    let impl_ = make_git_repo(t.path(), "impl", 0, 0, true);
    let cards = make_git_repo(t.path(), "cards", 0, 0, true);
    registry::add_repo(&sp, "impl", &impl_, AddRepo::default()).unwrap();
    registry::add_repo(&sp, "cards", &cards, AddRepo::default()).unwrap();
    registry::add_group(&sp, "產品", &["impl"]).unwrap();
    registry::add_group(&sp, "知識", &["cards"]).unwrap();
    registry::set_group_field(&sp, "產品", "goal", "把搜尋服務的向量索引改成增量重建").unwrap();
    // api.add_todo(spine, "...", cwd=impl)
    spine::append_event(
        &sp,
        "open-loop",
        "agent",
        &["#1", "group:產品", "repo:impl"],
        "冷啟動太慢：載入快照後補增量，量測查詢延遲",
        None,
    )
    .unwrap();
    log_decision(
        &sp,
        "倒排索引分段合併改成每小時一次，避免拖垮寫入吞吐",
        "產品",
        "impl",
        1,
    );
    World {
        _t: t,
        sp,
        impl_,
        cards,
    }
}

fn age(p: &Path, days: u64) {
    let t = SystemTime::now() - Duration::from_secs(days * 86400);
    std::fs::File::options()
        .write(true)
        .open(p)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_accessed(t).set_modified(t))
        .unwrap();
}

fn refs_of(res: &ruminate::RunResult, g: &str) -> Vec<String> {
    res.group(g)
        .map(|v| v.iter().map(|l| l.reference.clone()).collect())
        .unwrap_or_default()
}

#[test]
fn test_new_related_card_is_pushed_to_group() {
    let w = world();
    std::fs::write(w.cards.join("索引重建.md"), CARD).unwrap();
    std::fs::write(w.cards.join("料理.md"), UNRELATED).unwrap();
    let res = ruminate::run(&w.sp, None, None).unwrap();
    assert_eq!(refs_of(&res, "產品"), vec!["cards:索引重建.md"]); // 相關的推、不相關的不推
    let link = &res.group("產品").unwrap()[0];
    assert!(link.terms.iter().any(|t| t == "增量" || t == "重建")); // 說得出為什麼
    assert!(!link.snippets.as_ref().unwrap().is_empty());
    assert!(link.basis.iter().any(|b| b == "目標"));
}

#[test]
fn test_own_group_and_old_files_not_pushed() {
    let w = world();
    std::fs::write(w.impl_.join("索引筆記.md"), CARD).unwrap(); // 自己組的
    let old = w.cards.join("舊卡.md");
    std::fs::write(&old, CARD).unwrap();
    age(&old, 60); // 反芻窗之前
    for f in [
        w.cards.join("README.md"),
        w.cards.join("notes").join("plan.md"),
    ] {
        age(&f, 60);
    }
    let res = ruminate::run(&w.sp, None, None).unwrap();
    assert!(res.group("產品").is_none());
}

#[test]
fn test_no_repeat_and_watermark() {
    let w = world();
    std::fs::write(w.cards.join("索引重建.md"), CARD).unwrap();
    let first = ruminate::run(&w.sp, None, None).unwrap();
    assert!(!first.group("產品").unwrap().is_empty());
    let later = Local::now().naive_local() + chrono::Duration::minutes(1);
    let second = ruminate::run(&w.sp, Some(later), None).unwrap();
    assert!(second.group("產品").is_none()); // 同一份不重推
    assert_eq!(ruminate::fresh(&w.sp, "產品", false).unwrap().len(), 1);
}

#[test]
fn test_weak_overlap_is_not_pushed() {
    let w = world();
    std::fs::write(w.cards.join("一句.md"), "# 雜記\n今天讀到索引兩個字。\n").unwrap();
    let res = ruminate::run(&w.sp, None, None).unwrap();
    assert!(res.group("產品").is_none()); // 寧缺勿濫
}

#[test]
fn test_other_group_decision_matches() {
    let w = world();
    log_decision(
        &w.sp,
        "讀完論文：向量索引增量重建要先記水位線，冷啟動載入快照後補增量，查詢延遲才穩",
        "知識",
        "cards",
        2,
    );
    let res = ruminate::run(&w.sp, None, None).unwrap();
    let links = res.group("產品").unwrap();
    assert!(links.iter().any(|l| l.kind == LinkKind::Decision));
    // 反過來：產品組自己的判斷不會推回給產品組
    assert!(links.iter().all(|l| l.repo != "impl"));
}

#[test]
fn test_used_via_read_from_and_dismiss() {
    let w = world();
    std::fs::write(w.cards.join("索引重建.md"), CARD).unwrap();
    let before = Local::now().naive_local() - chrono::Duration::minutes(5);
    ruminate::run(&w.sp, Some(before), None).unwrap();
    // api.read_from(sp, "cards:索引重建.md", cwd=impl)
    crossref::read(
        &w.sp,
        "cards:索引重建.md",
        Some("impl"),
        None,
        MAX_READ_CHARS,
    )
    .unwrap();
    let s = ruminate::stats(&w.sp).unwrap();
    assert!(s.used == 1 && s.hit_rate == Some(1.0));
    assert!(ruminate::fresh(&w.sp, "產品", false).unwrap().is_empty()); // 用上了就不再當新推薦
    assert_eq!(
        ruminate::fresh(&w.sp, "產品", true).unwrap()[0].status,
        LinkStatus::Used
    );

    std::fs::write(
        w.cards.join("索引二.md"),
        format!("{CARD}\n分段合併的查詢延遲量測。\n"),
    )
    .unwrap();
    ruminate::run(&w.sp, None, None).unwrap();
    let lid = ruminate::fresh(&w.sp, "產品", false).unwrap()[0]
        .id
        .clone()
        .unwrap();
    ruminate::dismiss(&w.sp, &lid).unwrap();
    assert!(ruminate::fresh(&w.sp, "產品", false).unwrap().is_empty());
    assert!(ruminate::dismiss(&w.sp, "r999").is_err());
}

// test_session_start_shows_fresh：搬在 tests/hooks.rs。

#[test]
fn test_stale_detection() {
    let w = world();
    assert!(ruminate::is_stale(&w.sp, None));
    ruminate::run(&w.sp, None, None).unwrap();
    assert!(!ruminate::is_stale(&w.sp, None));
    let later = Local::now().naive_local() + chrono::Duration::days(2);
    assert!(ruminate::is_stale(&w.sp, Some(later)));
}

// test_timer_task：搬在 tests/timer.rs。
// test_cli_fresh：已搬到 crates/ctx-cli/tests/ruminate.rs。

// ---- 真資料校準（2026-09-25）留下的三條 ----

#[test]
fn test_rare_floor_not_median() {
    // 卡片庫大部分詞只出現一兩次：門檻要能收「出現在少數幾份」的詞，不能只剩只出現一次的
    let n = 1500.0_f64;
    let idf_df5 = (1.0 + (n - 5.0 + 0.5) / (5.0 + 0.5)).ln();
    assert!(idf_df5 >= ruminate::rare_floor(1500));
}

#[test]
fn test_function_char_bigrams_do_not_count() {
    assert!(!ruminate::meaningful("接是") && !ruminate::meaningful("得上"));
    assert!(ruminate::meaningful("交接") && ruminate::meaningful("hook"));
}

#[test]
fn test_dot_dirs_and_duplicates_skipped() {
    let w = world();
    for sub in [".claude/skills", ".agents/skills"] {
        let d = w.cards.join(sub);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), CARD).unwrap();
    }
    std::fs::write(w.cards.join("a.md"), CARD).unwrap();
    std::fs::write(w.cards.join("b.md"), CARD).unwrap(); // 同內容第二份
    let res = ruminate::run(&w.sp, None, None).unwrap();
    let refs = refs_of(&res, "產品");
    assert!(refs == vec!["cards:a.md"] || refs == vec!["cards:b.md"]);
}

// ---- Rust 版補測（非 Python 一對一）----

/// `.state/ruminate.json` 讀回 Python 寫的形狀（stamp 可為 int 或字串、不認得的鍵保留）。
#[test]
fn state_roundtrips_python_shape() {
    let w = world();
    let p = w.sp.join(".state").join("ruminate.json");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(
        &p,
        r#"{"last_run": "2026-10-01T06:00:00", "links": [
          {"id": "r3", "group": "產品", "kind": "file", "repo": "cards", "ref": "cards:x.md",
           "title": "x.md", "stamp": 1759000000, "score": 4.2, "terms": ["增量"], "basis": ["目標"],
           "found": "2026-10-01 06:00", "status": "new", "snippets": [{"line": 1, "text": "x"}]},
          {"id": "r4", "group": "產品", "kind": "decision", "repo": "cards", "ref": "decision:d1",
           "title": "t", "stamp": "2026-10-01 05:00", "score": 1.0, "terms": [], "basis": [],
           "found": "2026-10-01 06:00", "status": "dismissed", "extra_key": 1}]}"#,
    )
    .unwrap();
    let st = ruminate::load_state(&w.sp);
    assert_eq!(st.links.len(), 2);
    assert_eq!(st.links[1].extra["extra_key"], 1);
    let s = ruminate::stats(&w.sp).unwrap();
    assert!(s.total == 2 && s.new == 1 && s.dismissed == 1 && s.hit_rate == Some(0.0));
    assert_eq!(s.last_run.as_deref(), Some("2026-10-01T06:00:00"));
    let lines = ruminate::link_lines(&ruminate::fresh(&w.sp, "產品", true).unwrap(), 3);
    assert!(lines
        .iter()
        .any(|l| l.starts_with("- [r3] 新料 cards:x.md——共同詞：增量")));
    assert!(lines.iter().any(|l| l == "    L1: x"));
}

// ---- Rust 才有（N-03 推送降噪）----

#[test]
fn numbers_and_dates_are_not_shared_terms() {
    for t in ["54", "2026-10-06", "20260908", "1.2.3"] {
        assert!(!ruminate::meaningful(t), "{t}");
    }
    assert!(ruminate::meaningful("v0.2") && ruminate::meaningful("q1"));
}

#[test]
fn cross_boundary_bigrams_are_not_word_like() {
    // 「者」「決」各自很常見，「者決」只是「使用者決定」切出來的
    let mut docs = Vec::new();
    for i in 0..200 {
        let text = match i % 4 {
            0 if i % 20 == 0 => "使用者決定要不要重建索引".to_string(),
            0 => format!("編者{i}說明決策"),
            1 => format!("作者{i}寫了筆記，結論是要決議"),
            2 => format!("讀者{i}看完後決定延後"),
            _ => format!("鄰居{i}的狀態，管理者負責解決"),
        };
        let tf = ctx_core::knowledge::tokenize(&text).into_iter().fold(
            std::collections::HashMap::new(),
            |mut m, t| {
                *m.entry(t).or_insert(0.0) += 1.0;
                m
            },
        );
        docs.push(ctx_core::knowledge::Doc {
            repo: "r".into(),
            path: format!("/x/{i}.md"),
            rel: format!("{i}.md"),
            tf,
            len: 1.0,
            htf: Default::default(),
            hlen: 0.0,
            mtime: 0.0,
        });
    }
    let st = ruminate::CharStats::from_docs(docs.iter());
    assert!(!st.word_like("者決"));
    assert!(st.word_like("鄰居") && st.word_like("hook"));
}

#[test]
fn changed_file_is_not_pushed_again_while_pending() {
    let w = world();
    let card = w.cards.join("索引重建.md");
    std::fs::write(&card, CARD).unwrap();
    let first = ruminate::run(&w.sp, None, None).unwrap();
    assert_eq!(refs_of(&first, "產品"), vec!["cards:索引重建.md"]);
    // 檔案改了（mtime 變）：還有一筆沒處理的推薦 → 不再推一筆新的
    std::fs::write(
        &card,
        format!("{CARD}\n補一段：增量重建的水位線要持久化。\n"),
    )
    .unwrap();
    let later = Local::now().naive_local() + chrono::Duration::minutes(1);
    let second = ruminate::run(&w.sp, Some(later), Some(1)).unwrap();
    assert!(second.group("產品").is_none());
    assert_eq!(ruminate::fresh(&w.sp, "產品", false).unwrap().len(), 1);
}

fn doc_in(repo: &str, i: usize, text: &str) -> ctx_core::knowledge::Doc {
    let tf = ctx_core::knowledge::tokenize(text).into_iter().fold(
        std::collections::HashMap::new(),
        |mut m, t| {
            *m.entry(t).or_insert(0.0) += 1.0;
            m
        },
    );
    ctx_core::knowledge::Doc {
        repo: repo.into(),
        path: format!("/{repo}/{i}.md"),
        rel: format!("{i}.md"),
        tf,
        len: 1.0,
        htf: Default::default(),
        hlen: 0.0,
        mtime: 0.0,
    }
}

#[test]
fn words_in_almost_every_repo_are_generic() {
    let mut docs = Vec::new();
    for r in 0..9 {
        docs.push(doc_in(&format!("r{r}"), 0, "這件事尚未交付，等待確認"));
    }
    docs.push(doc_in("r0", 1, "試播店型的觀看率"));
    docs.push(doc_in("r1", 1, "試播排程"));
    let st = ruminate::CharStats::from_docs(docs.iter());
    assert!(st.generic("尚未") && st.generic("交付"));
    assert!(!st.generic("試播"));
    // repo 太少不判斷（測試世界只有兩三個 repo）
    let few: Vec<_> = docs
        .iter()
        .filter(|d| d.repo == "r0" || d.repo == "r1")
        .cloned()
        .collect();
    assert!(!ruminate::CharStats::from_docs(few.iter()).generic("尚未"));
}

#[test]
fn background_claim_is_exclusive_until_run_finishes() {
    let w = world();
    assert!(ruminate::claim_background(&w.sp));
    assert!(!ruminate::claim_background(&w.sp)); // 另一個開場：別人正在跑
    ruminate::run(&w.sp, None, None).unwrap();
    assert!(ruminate::claim_background(&w.sp)); // 跑完清掉標記
}

#[test]
fn concurrent_runs_do_not_duplicate_links() {
    let w = world();
    std::fs::write(w.cards.join("索引重建.md"), CARD).unwrap();
    // 兩個背景反芻同時從同一份舊狀態算：第二個寫入時要看到第一個已經寫了
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let hs: Vec<_> = (0..2)
        .map(|_| {
            let (sp, b) = (w.sp.clone(), barrier.clone());
            std::thread::spawn(move || {
                b.wait();
                ruminate::run(&sp, None, None).unwrap();
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    assert_eq!(ruminate::fresh(&w.sp, "產品", false).unwrap().len(), 1);
}

// ---- Rust 才有（N-20）：推薦規則版本＋衰減 ----

/// 把狀態檔裡的推薦改成「舊規則」（拿掉 rules）或改 found 時間。
fn rewrite_links(sp: &Path, f: impl Fn(&mut serde_json::Value)) {
    let p = sp.join(".state/ruminate.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    for l in v["links"].as_array_mut().unwrap() {
        f(l);
    }
    std::fs::write(&p, serde_json::to_string(&v).unwrap()).unwrap();
}

#[test]
fn old_rule_links_are_not_pushed_but_kept() {
    let w = world();
    std::fs::write(w.cards.join("索引重建.md"), CARD).unwrap();
    ruminate::run(&w.sp, None, None).unwrap();
    assert_eq!(ruminate::push_links(&w.sp, "產品", None).unwrap().len(), 1);
    rewrite_links(&w.sp, |l| {
        l.as_object_mut().unwrap().remove("rules");
    });
    assert!(ruminate::push_links(&w.sp, "產品", None)
        .unwrap()
        .is_empty()); // 被取代：不推
    assert_eq!(ruminate::fresh(&w.sp, "產品", false).unwrap().len(), 1); // 資料還在
}

#[test]
fn stale_links_decay_out_of_push() {
    let w = world();
    std::fs::write(w.cards.join("索引重建.md"), CARD).unwrap();
    ruminate::run(&w.sp, None, None).unwrap();
    let later = Local::now().naive_local() + chrono::Duration::days(ruminate::PUSH_DAYS + 1);
    assert!(ruminate::push_links(&w.sp, "產品", Some(later))
        .unwrap()
        .is_empty());
}

#[test]
fn old_rule_pending_link_does_not_block_new_rules() {
    let w = world();
    std::fs::write(w.cards.join("索引重建.md"), CARD).unwrap();
    ruminate::run(&w.sp, None, None).unwrap();
    rewrite_links(&w.sp, |l| {
        l.as_object_mut().unwrap().remove("rules");
    });
    // 新規則重新判斷：舊規則那筆不算「已推過」，同一份料可以用新規則再推一次
    let later = Local::now().naive_local() + chrono::Duration::minutes(1);
    let again = ruminate::run(&w.sp, Some(later), Some(1)).unwrap();
    assert_eq!(refs_of(&again, "產品"), vec!["cards:索引重建.md"]);
    assert_eq!(ruminate::push_links(&w.sp, "產品", None).unwrap().len(), 1);
}
