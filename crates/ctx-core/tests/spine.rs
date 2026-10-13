//! L1：脊椎寫入/查詢/dead-letter/stats/lint（不變式 2、6、7＋衛生迴圈）。對應 tests/test_spine.py。

use ctx_core::spine::{
    ack_unread, append_event, append_event_or_dead_letter, get_unread, lint, open_loops, query,
    stats, EventType, Query, NO_TOKENS,
};
use ctx_core::testkit::{spine, today_at as t};
use ctx_core::Error;

fn all(d: &std::path::Path) -> Vec<ctx_core::spine::Event> {
    query(d, &Query::default()).unwrap()
}

fn dead_letters(d: &std::path::Path) -> Vec<std::path::PathBuf> {
    let dl = d.join("spine").join("dead-letter");
    std::fs::read_dir(dl)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().unwrap() == "md")
        .collect()
}

#[test]
fn test_append_and_query() {
    let (_tmp, d) = spine();
    let ev = append_event(
        &d,
        "presented",
        "morning-brief",
        &["group:g1"],
        "呈現了三件事",
        Some(t(7, 42)),
    )
    .unwrap();
    assert_eq!(ev.time, "07:42");
    let got = query(&d, &Query::of_type(EventType::Presented)).unwrap();
    assert!(got.len() == 1 && got[0].body == "呈現了三件事");
}

#[test]
fn test_inv_reject_programmatic_raises() {
    let (_tmp, d) = spine();
    let r = append_event(&d, "not-a-type", "manual", NO_TOKENS, "x", None);
    assert!(matches!(r, Err(Error::Validation(_))));
    assert!(all(&d).is_empty()); // 什麼都沒寫進去
}

#[test]
fn test_inv_reject_human_goes_dead_letter() {
    let (_tmp, d) = spine();
    let ev = append_event_or_dead_letter(
        &d,
        "not-a-type",
        "hotkey",
        NO_TOKENS,
        "珍貴的想法原文",
        None,
    )
    .unwrap();
    assert!(ev.is_none());
    let dl = dead_letters(&d);
    assert_eq!(dl.len(), 1);
    assert!(std::fs::read_to_string(&dl[0])
        .unwrap()
        .contains("珍貴的想法原文"));
    assert!(all(&d).is_empty()); // 壞事件沒進脊椎
}

#[test]
fn test_open_loops_view() {
    let (_tmp, d) = spine();
    append_event(
        &d,
        "open-loop",
        "hotkey",
        &["#13", "due:2026-09-15"],
        "opened →「驗證 X」",
        Some(t(9, 0)),
    )
    .unwrap();
    append_event(
        &d,
        "open-loop",
        "hotkey",
        &["#14"],
        "opened →「另一件」",
        Some(t(9, 1)),
    )
    .unwrap();
    append_event(
        &d,
        "open-loop",
        "manual",
        &["#14"],
        "closed → 做完了",
        Some(t(10, 0)),
    )
    .unwrap();
    let loops = open_loops(&d).unwrap();
    assert_eq!(loops.len(), 1);
    assert!(loops[0].tokens.contains(&"#13".to_string()));
}

/// 靈感命中率＝有 outcome 回連的碰撞 / 全部碰撞。手算：2 撞 1 中 = 50%。
#[test]
fn test_inv_stats_hit_rate_hand_checked() {
    let (_tmp, d) = spine();
    append_event(
        &d,
        "collision",
        "hotkey",
        &["id:2026-09-01-a"],
        "opened\n輸入：想法一",
        Some(t(9, 0)),
    )
    .unwrap();
    append_event(
        &d,
        "collision",
        "hotkey",
        &["id:2026-09-01-b"],
        "opened\n輸入：想法二",
        Some(t(9, 5)),
    )
    .unwrap();
    append_event(
        &d,
        "outcome",
        "manual",
        &["ref:collision:2026-09-01-a"],
        "長成了一篇文章",
        Some(t(21, 0)),
    )
    .unwrap();
    let s = stats(&d).unwrap();
    assert_eq!(s.collisions, 2);
    assert_eq!(s.collisions_with_outcome, 1);
    assert_eq!(s.hit_rate, Some(0.5));
}

/// chosen 帶 ref:presented:HH:MM 能解回原事件（M2 驗收①的資料基礎）。
#[test]
fn test_inv_chosen_refs_presented() {
    let (_tmp, d) = spine();
    append_event(
        &d,
        "presented",
        "morning-brief",
        &["group:g1"],
        "呈現了",
        Some(t(7, 42)),
    )
    .unwrap();
    append_event(
        &d,
        "chosen",
        "morning-brief",
        &["ref:presented:07:42"],
        "選了第一項",
        Some(t(7, 45)),
    )
    .unwrap();
    let chosen = query(&d, &Query::of_type(EventType::Chosen))
        .unwrap()
        .remove(0);
    let ref_time = chosen
        .kv("ref")
        .unwrap()
        .split_once(':')
        .unwrap()
        .1
        .to_string();
    let src: Vec<_> = query(&d, &Query::of_type(EventType::Presented))
        .unwrap()
        .into_iter()
        .filter(|e| e.time == ref_time)
        .collect();
    assert!(src.len() == 1 && src[0].body == "呈現了");
}

#[test]
fn test_unread_marker() {
    let (_tmp, d) = spine();
    append_event(&d, "decision", "manual", NO_TOKENS, "第一筆", Some(t(9, 0))).unwrap();
    assert_eq!(get_unread(&d).unwrap().len(), 1);
    ack_unread(&d, Some(t(9, 30))).unwrap();
    assert!(get_unread(&d).unwrap().is_empty());
    append_event(
        &d,
        "decision",
        "manual",
        NO_TOKENS,
        "第二筆",
        Some(t(10, 0)),
    )
    .unwrap();
    assert_eq!(get_unread(&d).unwrap().len(), 1);
}

// ── 衛生迴圈（second-brain lint 課＋Szapar staleness 課）──────────────

/// 健康脊椎：好 ref、未到期 loop、今天的 opened 碰撞——零紅字。
#[test]
fn test_lint_clean_spine() {
    let (_tmp, d) = spine();
    append_event(
        &d,
        "presented",
        "morning-brief",
        NO_TOKENS,
        "呈現了",
        Some(t(7, 42)),
    )
    .unwrap();
    append_event(
        &d,
        "chosen",
        "morning-brief",
        &["ref:presented:07:42"],
        "選了",
        Some(t(7, 45)),
    )
    .unwrap();
    append_event(
        &d,
        "open-loop",
        "hotkey",
        &["#1", "due:2099-01-01"],
        "opened →「未來的事」",
        Some(t(9, 0)),
    )
    .unwrap();
    append_event(
        &d,
        "collision",
        "hotkey",
        &["id:x-a"],
        "opened\n輸入：今天剛丟的",
        Some(t(9, 5)),
    )
    .unwrap();
    assert_eq!(lint(&d, None).unwrap(), Vec::<String>::new());
}

#[test]
fn test_lint_broken_ref() {
    let (_tmp, d) = spine();
    append_event(
        &d,
        "chosen",
        "monitor",
        &["ref:presented:09:15"],
        "ref 指到不存在的事件",
        Some(t(10, 0)),
    )
    .unwrap();
    let reds = lint(&d, None).unwrap();
    assert!(reds.len() == 1 && reds[0].contains("ref 斷鏈"));
}

#[test]
fn test_lint_overdue_loop_and_stale_collision() {
    let (_tmp, d) = spine();
    let yesterday = t(9, 0) - chrono::Duration::days(1);
    append_event(
        &d,
        "open-loop",
        "hotkey",
        &["#7", "due:2020-01-01"],
        "opened →「早該做的」",
        Some(t(9, 0)),
    )
    .unwrap();
    append_event(
        &d,
        "collision",
        "hotkey",
        &["id:old-a"],
        "opened\n輸入：昨天丟的想法",
        Some(yesterday),
    )
    .unwrap();
    let reds = lint(&d, None).unwrap().join("\n");
    assert!(reds.contains("open-loop 逾期") && reds.contains("#7"));
    assert!(reds.contains("碰撞無回程") && reds.contains("old-a"));
    // 判定回來後（同 id 的非 opened 事件）就不再紅
    append_event(
        &d,
        "collision",
        "engine",
        &["id:old-a", "ref:collision:old-a"],
        "判定：真增量",
        Some(t(10, 0)),
    )
    .unwrap();
    assert!(!lint(&d, None).unwrap().join("\n").contains("碰撞無回程"));
}

#[test]
fn test_lint_dead_letter_backlog() {
    let (_tmp, d) = spine();
    append_event_or_dead_letter(&d, "bad-type", "hotkey", NO_TOKENS, "拒寫原文", None).unwrap();
    let reds = lint(&d, None).unwrap();
    assert!(reds
        .iter()
        .any(|r| r.contains("dead-letter 積壓") && r.contains("1 件")));
}

// ── 合約：寫出的位元組與 Python 版一致 ─────────────────────────────────

#[test]
fn event_file_bytes_match_python_format() {
    let (_tmp, d) = spine();
    let when = chrono::NaiveDate::from_ymd_opt(2026, 9, 1)
        .unwrap()
        .and_hms_opt(7, 5, 0)
        .unwrap();
    append_event(
        &d,
        "decision",
        "manual",
        &["group:g1", "#3"],
        "  第一行\n第二行  \n",
        Some(when),
    )
    .unwrap();
    append_event(&d, "presented", "engine", NO_TOKENS, "", Some(when)).unwrap();
    let text = std::fs::read_to_string(d.join("spine/events/2026-09-01.md")).unwrap();
    assert_eq!(
        text,
        "## 07:05 decision [manual] group:g1 #3\n第一行\n第二行\n## 07:05 presented [engine]\n"
    );
    // dead-letter 檔：檔頭註解＋原標頭＋原 body
    append_event_or_dead_letter(&d, "nope", "hotkey", NO_TOKENS, "原文", Some(when)).unwrap();
    let dl = dead_letters(&d);
    assert_eq!(
        dl[0].file_name().unwrap().to_str().unwrap(),
        "2026-09-01-070500-000000.md"
    );
    assert_eq!(
        std::fs::read_to_string(&dl[0]).unwrap(),
        "<!-- validator 拒寫: 未知事件型別 'nope'（合法: presented, chosen, decision, suggestion, collision, open-loop, outcome） -->\n## 07:05 nope [hotkey]\n原文\n"
    );
}

/// 讀到壞標頭：照舊整份失敗，但訊息說出是哪個檔第幾行（Rust 才有，2026-10-14，ideation N-35）。
#[test]
fn bad_header_error_names_file_and_line() {
    let (_t, d) = ctx_core::testkit::spine();
    let dir = d.join("spine").join("events");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("2026-10-14.md"),
        "## 09:00 decision [agent] repo:a\n好的一筆\n\n## 09:05 decision [agent] group:PM Group\n壞的一筆\n",
    )
    .unwrap();
    let e = ctx_core::spine::iter_events(&d).unwrap_err().to_string();
    assert!(e.contains("kv token 不合法"), "{e}");
    assert!(e.contains("spine/events/2026-10-14.md 第 4 行"), "{e}");
}
