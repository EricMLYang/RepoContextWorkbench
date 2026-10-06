//! 跨 repo 參考輪（2026-09-25）：agent 在 A repo 工作，又快又準地拿到 B repo 的東西。對應 tests/test_crossref.py。
//!
//! Python 經 `agentapi.read_from(cwd=…)` 的地方，這裡直接呼叫 `crossref::read` 並給出 cwd 會推出的
//! 引用方（consumer）——where_am_i 屬 agentapi（第三波）。repo 狀態卡的測試在 tests/repocard.rs。

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use ctx_core::agents::ClaudeProvider;
use ctx_core::crossref::{self, MAX_READ_CHARS};
use ctx_core::knowledge::{self, Boosts};
use ctx_core::pack::RepoRef;
use ctx_core::registry::{self, AddRepo};
use ctx_core::testkit::{git, make_git_repo, spine};
use ctx_core::Error;

fn commit(repo: &Path, msg: &str) {
    git(repo, &["add", "-A"], &[]);
    git(repo, &["commit", "-q", "-m", msg], &[]);
}

struct World {
    _t: tempfile::TempDir,
    sp: PathBuf,
    notes: PathBuf,
    course: PathBuf,
    other: PathBuf,
}

/// notes（知識庫，有書摘目錄）feeds course（課程 repo）；other 是不相關的 repo。
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
    std::fs::write(
        notes.join("keys.md"),
        "api_key = 'ABCDEFGHIJKLMNOPQRSTUV'\n",
    )
    .unwrap();
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
        other,
    }
}

fn read(w: &World, r: &str, consumer: Option<&str>) -> ctx_core::Result<crossref::ReadOut> {
    crossref::read(&w.sp, r, consumer, None, MAX_READ_CHARS)
}

fn code_of(e: Error) -> (String, Option<String>) {
    match e {
        Error::Rejected { code, hint, .. } => (code, hint),
        other => panic!("預期 RefError，拿到 {other:?}"),
    }
}

// ---- ① exports ＋ resolve ----

#[test]
fn test_export_validation() {
    let w = world();
    let sp = &w.sp;
    let msg = |r: ctx_core::Result<registry::ExportSet>| r.unwrap_err().to_string();
    assert!(msg(registry::set_export(sp, "notes", "x", "nope", None)).contains("不存在"));
    assert!(msg(registry::set_export(sp, "notes", "x", "../course", None)).contains("跳出"));
    assert!(msg(registry::set_export(sp, "notes", "a:b", "books", None)).contains("不能"));
    assert!(
        registry::relate(sp, "notes", "other", "feeds", None, &["不存在的"])
            .unwrap_err()
            .to_string()
            .contains("沒有 export")
    );
    assert_eq!(
        registry::relations(sp, Some("notes")).unwrap()[0].exports,
        vec!["書摘"]
    );
    assert!(registry::remove_export(sp, "notes", "書摘").unwrap());
    // 關係上的引用一起清掉
    assert!(registry::relations(sp, Some("notes")).unwrap()[0]
        .exports
        .is_empty());
}

#[test]
fn test_resolve_by_name() {
    let w = world();
    let r = crossref::resolve(&w.sp, "notes:書摘/cache-book/ch01.md").unwrap();
    assert_eq!(r.export.as_deref(), Some("書摘"));
    assert_eq!(r.kind, crossref::PathKind::File);
    assert_eq!(r.rel, "books/cache-book/ch01.md");
    assert_eq!(r.desc.as_deref(), Some("逐章書摘"));
    assert_eq!(
        crossref::resolve(&w.sp, "notes/README.md").unwrap().kind,
        crossref::PathKind::File
    ); // repo/相對路徑
    assert_eq!(crossref::resolve(&w.sp, "notes").unwrap().rel, ".");
    let miss = crossref::resolve(&w.sp, "notes:書摘/nope.md").unwrap();
    assert!(!miss.exists && miss.hint.unwrap().contains("書摘"));
    let (code, _) = code_of(crossref::resolve(&w.sp, "notes:../../etc").unwrap_err());
    assert_eq!(code, "bad_request");
    let (code, hint) = code_of(crossref::resolve(&w.sp, "note").unwrap_err());
    assert!(code == "not_found" && hint.unwrap().contains("notes"));
}

// ---- ② read ＋ 引用紀錄 ----

#[test]
fn test_read_records_cite_once_per_day() {
    let w = world();
    let r = read(&w, "notes:書摘/cache-book/ch01.md", Some("course")).unwrap();
    assert!(
        r.content.unwrap().contains("快取失效") && r.commit.is_some() && r.file_commit.is_some()
    );
    read(&w, "notes:書摘/cache-book/ch01.md", Some("course")).unwrap();
    let cites = crossref::load_cites(&w.sp);
    assert_eq!(cites.len(), 1); // 同日同檔只記一次
    assert_eq!(
        (cites[0].consumer.as_str(), cites[0].provider.as_str()),
        ("course", "notes")
    );
    read(&w, "notes:README.md", Some("notes")).unwrap(); // 自己讀自己不記
    read(&w, "notes:README.md", None).unwrap(); // spine 裡讀（沒有引用方）不記
    assert_eq!(crossref::load_cites(&w.sp).len(), 1);
}

#[test]
fn test_read_dir_lines_and_secret() {
    let w = world();
    let d = read(&w, "notes:書摘", Some("course")).unwrap();
    assert_eq!(d.kind, crossref::PathKind::Dir);
    assert_eq!(
        serde_json::to_value(&d.entries.unwrap()[0]).unwrap(),
        serde_json::json!({"name": "cache-book/", "files": 2})
    );
    let r = crossref::read(
        &w.sp,
        "notes:書摘/cache-book/ch01.md",
        Some("course"),
        Some("2"),
        MAX_READ_CHARS,
    )
    .unwrap();
    assert_eq!(r.content.as_deref(), Some("快取失效是最難的問題之一。"));
    assert_eq!(r.lines.as_deref(), Some("2-2"));
    assert_eq!(
        code_of(read(&w, "notes:keys.md", Some("course")).unwrap_err()).0,
        "secret"
    );
    assert_eq!(
        code_of(read(&w, "notes:nope.md", Some("course")).unwrap_err()).0,
        "not_found"
    );
}

// ---- ③ drift ＋ suggest ----

#[test]
fn test_drift_after_upstream_change() {
    let w = world();
    read(&w, "notes:書摘/cache-book/ch01.md", Some("course")).unwrap();
    assert!(crossref::drift(&w.sp, Some("course"), None)
        .unwrap()
        .is_empty());
    let cb = w.notes.join("books").join("cache-book");
    std::fs::write(cb.join("ch02.md"), "改了別章\n").unwrap();
    commit(&w.notes, "改第二章");
    assert!(crossref::drift(&w.sp, Some("course"), None)
        .unwrap()
        .is_empty()); // 改的不是引用的那份
    std::fs::write(cb.join("ch01.md"), "# 第一章\n改寫\n").unwrap();
    commit(&w.notes, "改寫第一章");
    let d = crossref::drift(&w.sp, Some("course"), None).unwrap();
    assert!(d.len() == 1 && d[0].changes == Some(1));
    assert_eq!(d[0].latest.as_ref().unwrap().subject, "改寫第一章");
    assert!(crossref::drift_lines(&d, 5)[0].contains("改了 1 次"));
}

#[test]
fn test_suggest_exports() {
    let w = world();
    std::fs::create_dir_all(w.other.join("docs")).unwrap();
    for i in 0..3 {
        std::fs::write(
            w.other.join("docs").join(format!("d{i}.md")),
            format!("# d{i}\n"),
        )
        .unwrap();
    }
    commit(&w.other, "docs");
    for i in 0..3 {
        read(&w, &format!("other:docs/d{i}.md"), Some("course")).unwrap();
    }
    read(&w, "notes:書摘/cache-book/ch01.md", Some("course")).unwrap(); // 已被 export 涵蓋
    let sug = crossref::suggest_exports(&w.sp, 3, 1).unwrap();
    let got: Vec<(&str, &str, usize)> = sug
        .iter()
        .map(|s| (s.repo.as_str(), s.path.as_str(), s.cites))
        .collect();
    assert_eq!(got, vec![("other", "docs", 3)]);
    registry::set_export(&w.sp, "other", "文件", "docs", None).unwrap();
    assert!(crossref::suggest_exports(&w.sp, 3, 1).unwrap().is_empty());
}

// ---- ④ agent 文件路徑 lint ----

#[test]
fn test_lint_doc_refs_reports_only_confident_breaks() {
    let w = world();
    std::fs::write(
        w.course.join("AGENTS.md"),
        [
            "# 規則",
            "書摘讀 `../notes/books/cache-book/`。", // 相對路徑存在 → OK
            "上游在 `notes/books/`。",               // repo 名開頭、子路徑存在 → OK
            "舊位置 `projects/notes/books/`。",      // 失效 → 報
            "筆記 `notes/nope/`。",                  // repo 名開頭但子路徑不存在 → 報
            "示意 `notes/.../x.md`。",               // 省略號 → 不驗
            "~~廢棄 `projects/notes/`~~",            // 刪除線 → 不驗
            "自己的 `src/course/x.py`。",            // 沒提到別的 repo → 不驗
            "```",
            "cat projects/notes/a.md",
            "```", // 程式碼區塊 → 不驗
        ]
        .join("\n"),
    )
    .unwrap();
    let found = crossref::lint_doc_refs(&w.sp, Some(&["course".to_string()])).unwrap();
    let got: Vec<(usize, &str, &str)> = found
        .iter()
        .map(|b| (b.line, b.reference.as_str(), b.target.as_str()))
        .collect();
    assert_eq!(
        got,
        vec![
            (4, "projects/notes/books/", "notes"),
            (5, "notes/nope/", "notes")
        ]
    );
    assert!(found[0].fix.contains("`notes:books`"));
}

// 未搬：test_audit_cli_includes_doc_refs——等第三波 agentapi/ctx-cli（跑 `ctx registry audit`）。

// ---- ⑤ 搜尋 ----

/// Python 經 agentapi.search_knowledge；這裡在引擎層組同樣的呼叫：全部 repo、主場 boosts＝crossref::search_boosts。
#[test]
fn test_search_neighbor_boost_and_why() {
    let w = world();
    std::fs::write(
        w.other.join("misc.md"),
        "# 雜記\n快取失效是最難的問題之一。\n",
    )
    .unwrap();
    commit(&w.other, "misc");
    let refs: Vec<RepoRef> = registry::load(&w.sp)
        .unwrap()
        .repos
        .iter()
        .map(RepoRef::from)
        .collect();
    let search = |b: &Boosts| knowledge::search(&refs, "快取失效", 8, Some(&w.sp), &[], b);
    let plain = search(&crossref::search_boosts(&w.sp, None).unwrap());
    let from_course = search(&crossref::search_boosts(&w.sp, Some("course")).unwrap());
    let top = &from_course.files[0];
    assert!(top.repo == "notes" && top.file.starts_with("books/"));
    assert!(
        top.why.iter().any(|x| x.contains("有關係")) && top.why.iter().any(|x| x.contains("書摘"))
    );
    assert!(top.why.iter().any(|x| x == "標題命中"));
    let notes_score = |r: &knowledge::SearchResult| {
        r.files
            .iter()
            .filter(|f| f.repo == "notes")
            .map(|f| f.score)
            .fold(f64::MIN, f64::max)
    };
    assert!(notes_score(&from_course) > notes_score(&plain)); // 同一份檔，從鄰居查分數較高
}

/// Python 經 agentapi.search_knowledge（code 半邊）；這裡直接呼叫 crossref::code_search（limit＝max(3, 8//2)）。
#[test]
fn test_code_search() {
    let w = world();
    std::fs::write(
        w.other.join("cache.py"),
        "def evict_lru(cache):\n    return cache.popitem()\n",
    )
    .unwrap();
    commit(&w.other, "code");
    let entries = registry::load(&w.sp).unwrap().repos;
    let r = crossref::code_search(&entries, "evict_lru 怎麼做", 4, 40);
    assert!(r[0].file == "cache.py" && r[0].repo == "other");
    assert_eq!(r[0].snippets[0].line, 1);
    assert!(r.iter().all(|c| !c.file.ends_with(".md")));
}

// ---- ⑥ ask_repo ----

#[test]
fn test_ask_repo_read_only_and_cites() {
    let w = world();
    let cmd = ClaudeProvider.build_cmd("q", None, true);
    let di = cmd.iter().position(|c| c == "--disallowedTools").unwrap();
    assert!(cmd.iter().any(|c| c == "--allowedTools") && cmd[di..].iter().any(|c| c == "Edit"));
    #[derive(Default)]
    struct Seen {
        provider: String,
        cwd: Option<PathBuf>,
        read_only: bool,
        prompt: String,
    }
    let seen = RefCell::new(Seen::default());
    let fake = |provider: &str,
                prompt: &str,
                _sp: &Path,
                _model: Option<&str>,
                cwd: Option<&Path>,
                read_only: bool| {
        *seen.borrow_mut() = Seen {
            provider: provider.into(),
            cwd: cwd.map(Path::to_path_buf),
            read_only,
            prompt: prompt.into(),
        };
        Ok(r#"x {"answer": "LRU", "citations": [{"file": "books/cache-book/ch01.md", "why": "定義"}]}"#.to_string())
    };
    let r = crossref::ask_repo_with(
        &w.sp,
        "notes",
        "快取怎麼失效？",
        Some("course"),
        None,
        None,
        &fake,
    )
    .unwrap();
    assert!(r.parsed.answer == "LRU" && r.provider == "claude"); // 預設 claude
    let s = seen.borrow();
    assert_eq!(s.provider, "claude");
    assert_eq!(
        s.cwd.as_deref(),
        Some(w.notes.canonicalize().unwrap().as_path())
    );
    assert!(s.read_only);
    assert!(s.prompt.contains("快取怎麼失效") && s.prompt.contains("course"));
    drop(s);
    assert_eq!(crossref::load_cites(&w.sp)[0].via, "ask");
    let plain = |_: &str, _: &str, _: &Path, _: Option<&str>, _: Option<&Path>, _: bool| {
        Ok("純文字回答".to_string())
    };
    let r = crossref::ask_repo_with(
        &w.sp,
        "notes",
        "再問一次",
        Some("course"),
        None,
        None,
        &plain,
    )
    .unwrap();
    assert!(r.parsed.answer == "純文字回答" && r.parsed.citations.is_empty());
    assert!(r.parsed.note.is_some());
}

#[test]
fn test_ask_repo_mock_provider() {
    let w = world();
    let r =
        crossref::ask_repo(&w.sp, "notes", "快取？", Some("course"), Some("mock"), None).unwrap();
    assert!(r.ok && r.provider == "mock" && !r.parsed.answer.is_empty());
}

// ---- ⑦ repo 狀態卡：見 tests/repocard.rs ----

// 未搬：test_session_start_injects_neighbor_map_and_drift——等第三波 agentapi/hooks（hooks.session_start）。
// 未搬：test_mcp_work_tools_include_crossref——等第三波 agentapi/ctx-cli（mcpserver.handle_message）。
// 未搬：test_cli_crossref_flow——等第三波 agentapi/ctx-cli（CLI e2e）。

// ---- Rust 版補測（非 Python 一對一）----

/// cites.jsonl 一行的格式與 Python `json.dumps(ensure_ascii=False)` 相同（`, `／`: ` 分隔、中文不跳脫）。
#[test]
fn cites_line_format_matches_python() {
    let w = world();
    let at = chrono::NaiveDate::from_ymd_opt(2026, 10, 6)
        .unwrap()
        .and_hms_opt(9, 5, 0)
        .unwrap();
    crossref::record_cite(
        &w.sp,
        Some("course"),
        "notes",
        "books/甲.md",
        Some("abc1234"),
        "manual",
        Some("複製"),
        Some(at),
    )
    .unwrap()
    .unwrap();
    let text = std::fs::read_to_string(w.sp.join("refs").join("cites.jsonl")).unwrap();
    assert_eq!(
        text,
        "{\"at\": \"2026-10-06 09:05\", \"consumer\": \"course\", \"provider\": \"notes\", \"rel\": \"books/甲.md\", \"commit\": \"abc1234\", \"via\": \"manual\", \"note\": \"複製\"}\n"
    );
}

#[test]
fn neighbor_lines_show_used_exports() {
    let w = world();
    let nbs = crossref::neighbors(&w.sp, "course").unwrap();
    assert_eq!(nbs.len(), 1);
    let lines = crossref::neighbor_lines(&nbs, 4);
    assert!(lines[0].contains("`notes:書摘`→books") && lines[0].contains("逐章書摘"));
    let v = serde_json::to_value(&nbs[0]).unwrap();
    assert_eq!(v["uses"], serde_json::json!(["書摘"]));
    assert_eq!(v["exports"]["書摘"]["path"], "books/");
}
