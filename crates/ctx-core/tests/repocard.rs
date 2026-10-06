//! repo 狀態卡：水位線、目錄彙總、dirty、正在跑的程序、agent 交接。
//! 對應 tests/test_crossref.py 的「⑦ repo 狀態卡」三個測試（Python 沒有獨立的 test_repocard.py）。
//!
//! Python 的 `api.handoff(...)`／`api.repo_status(...)` 照樣經 `ctx_core::agentapi` 呼叫。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use ctx_core::agentapi::{self as api, Target};
use ctx_core::registry::{self, AddRepo};
use ctx_core::repocard::{self, ProcProbe, PsRow, WindowKind};
use ctx_core::testkit::{git, make_git_repo, spine};

fn commit(repo: &Path, msg: &str) {
    git(repo, &["add", "-A"], &[]);
    git(repo, &["commit", "-q", "-m", msg], &[]);
}

struct World {
    _t: tempfile::TempDir,
    sp: PathBuf,
    notes: PathBuf,
    course: PathBuf,
}

/// 同 tests/crossref.rs 的 world（notes feeds course）。
fn world() -> World {
    let (t, sp) = spine();
    let notes = make_git_repo(t.path(), "notes", 0, 0, true);
    let cb = notes.join("books").join("cache-book");
    std::fs::create_dir_all(&cb).unwrap();
    std::fs::write(
        cb.join("ch01.md"),
        "# 第一章 快取失效\n快取失效是最難的問題之一。\n",
    )
    .unwrap();
    std::fs::write(cb.join("ch02.md"), "# 第二章 一致性\n寫入策略與一致性。\n").unwrap();
    commit(&notes, "books");
    let course = make_git_repo(t.path(), "course", 0, 0, true);
    let other = make_git_repo(t.path(), "other", 0, 0, true);
    for (rid, p) in [("notes", &notes), ("course", &course), ("other", &other)] {
        registry::add_repo(&sp, rid, p, AddRepo::default()).unwrap();
    }
    registry::add_group(&sp, "課程", &["course"]).unwrap();
    registry::add_group(&sp, "知識", &["notes"]).unwrap();
    registry::set_export(&sp, "notes", "書摘", "books", Some("逐章書摘")).unwrap();
    registry::relate(&sp, "notes", "course", "feeds", None, &["書摘"]).unwrap();
    World {
        _t: t,
        sp,
        notes,
        course,
    }
}

/// 不偵測真程序（測試不依賴機器上正在跑什麼）。
fn no_procs() -> ProcProbe {
    ProcProbe {
        ps_rows: Some(Vec::new()),
        cwd_pids: Some(HashSet::new()),
    }
}

#[test]
fn test_repo_card_since_seen_and_areas() {
    let w = world();
    let card = repocard::build(&w.sp, "notes", None, true, None, no_procs()).unwrap();
    let d = card.detail.as_ref().unwrap();
    assert_eq!(d.window.kind, WindowKind::Recent);
    assert!(d.marked_seen.is_some());
    std::fs::write(
        w.notes.join("books").join("cache-book").join("ch03.md"),
        "# 三\n",
    )
    .unwrap();
    commit(&w.notes, "第三章");
    let card = repocard::build(&w.sp, "notes", None, false, None, no_procs()).unwrap();
    let d = card.detail.as_ref().unwrap();
    assert_eq!(d.window.kind, WindowKind::Seen);
    let subjects: Vec<&str> = d.new_commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, vec!["第三章"]);
    assert!(d.areas[0].area == "books/" && d.areas[0].files == 1);
    // 沒 mark＝水位線沒動
    let again = repocard::build(&w.sp, "notes", None, false, None, no_procs()).unwrap();
    let subjects: Vec<String> = again
        .detail
        .unwrap()
        .new_commits
        .into_iter()
        .map(|c| c.subject)
        .collect();
    assert_eq!(subjects, vec!["第三章"]);
    assert!(repocard::render(&card).contains("books/ 1 檔"));
}

#[test]
fn test_repo_card_dirty_running_and_agent() {
    let w = world();
    let wip = w.course.join("wip.md");
    std::fs::write(&wip, "草稿\n").unwrap();
    let old = SystemTime::now() - Duration::from_secs(3 * 86400);
    std::fs::File::options()
        .write(true)
        .open(&wip)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_accessed(old)
                .set_modified(old),
        )
        .unwrap();
    let started = chrono::Local::now().naive_local() - chrono::Duration::hours(1);
    let root = w.course.canonicalize().unwrap();
    let rs = root.display();
    let ps = vec![
        PsRow {
            pid: 99999,
            start: started,
            command: format!("node {rs}/node_modules/.bin/vite"),
        },
        PsRow {
            pid: 99998,
            start: started,
            command: format!("{rs}/node_modules/esbuild/bin/esbuild --ping"),
        },
        PsRow {
            pid: 99997,
            start: started,
            command: "node /somewhere/else/server.js".into(),
        },
    ];
    std::fs::write(w.course.join("README.md"), "# 改過\n").unwrap(); // 啟動後改
    api::handoff::<&str>(
        &w.sp,
        "做完大綱",
        "下次寫第二週",
        &[],
        &Target::at(&w.course),
        false,
        "agent",
    )
    .unwrap();
    let card = repocard::build(
        &w.sp,
        "course",
        None,
        false,
        None,
        ProcProbe {
            ps_rows: Some(ps),
            cwd_pids: Some(HashSet::new()),
        },
    )
    .unwrap();
    let d = card.detail.as_ref().unwrap();
    assert_eq!(d.dirty[0].file, "wip.md");
    assert!(d.dirty[0].days.unwrap() >= 2.9);
    let pids: Vec<i64> = d.running.iter().map(|p| p.pid).collect();
    assert_eq!(pids, vec![99999]); // 子程序與別的 repo 的程序不列
    assert!(d.running[0].changed_since_start >= 1);
    assert!(d.running[0].changed_files.iter().any(|f| f == "README.md"));
    assert!(d
        .last_handoff
        .as_ref()
        .unwrap()
        .text
        .starts_with("做完大綱"));
    let text = repocard::render(&card);
    assert!(text.contains("正在跑") && text.contains("要重開") && text.contains("放最久 3 天"));
}

#[test]
fn test_repo_status_agent_does_not_move_watermark() {
    let w = world();
    let r = api::repo_status(&w.sp, None, Some(&w.course), None).unwrap();
    assert_eq!(r["repo"], "course");
    assert!(r.get("marked_seen").is_none());
    assert!(repocard::get_seen(&w.sp, "course").is_none());
}

// ---- Rust 版補測（非 Python 一對一）----

/// 水位線檔格式同 Python `json.dumps({"head", "at"})`；卡的 JSON 鍵順序同 Python dict。
#[test]
fn seen_file_and_card_json_shape() {
    let w = world();
    let at = chrono::NaiveDate::from_ymd_opt(2026, 10, 6)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap();
    repocard::mark_seen(&w.sp, "a/b", "abc1234", Some(at)).unwrap();
    let p = w.sp.join(".state").join("seen").join("a_b.json");
    assert_eq!(
        std::fs::read_to_string(p).unwrap(),
        "{\"head\": \"abc1234\", \"at\": \"2026-10-06 08:00\"}"
    );
    let card = repocard::build(&w.sp, "course", Some("7d"), false, None, no_procs()).unwrap();
    let v = serde_json::to_value(&card).unwrap();
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "ok",
            "repo",
            "path",
            "tier",
            "exists",
            "branch",
            "note",
            "ahead",
            "behind",
            "commits_7d",
            "commits_30d",
            "last_commit_days",
            "head",
            "window",
            "new_commits",
            "areas",
            "dirty",
            "running",
            "last_handoff",
            "last_unhanded",
            "broken_refs",
            "drift"
        ]
    );
    assert_eq!(v["window"]["kind"], "days");
    let err = repocard::build(&w.sp, "course", Some("deadbeef"), false, None, no_procs());
    assert!(err.unwrap_err().to_string().contains("找不到 commit"));
}
