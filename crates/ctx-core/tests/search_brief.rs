//! F-126 漸進揭露：封存路徑降權、標題命中門檻、精簡模式（refs 保證含 answer_repo、上限、沒結果）、判斷歸屬、入口卡。

use std::path::{Path, PathBuf};

use ctx_core::agentapi::{self as api, SearchOpts, Target};
use ctx_core::registry::{add_repo, set_field, AddRepo};
use ctx_core::testkit::{make_git_repo, spine};
use serde_json::Value;

fn register(d: &Path, tmp: &Path, id: &str) -> PathBuf {
    let p = make_git_repo(tmp, id, 6, 0, false);
    add_repo(d, id, &p, AddRepo::default()).unwrap();
    p
}

fn write(p: &Path, rel: &str, text: &str) {
    let f = p.join(rel);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(f, text).unwrap();
}

fn full(d: &Path, q: &str, cwd: &Path) -> Value {
    let opts = SearchOpts {
        code: false,
        limit: 20,
        ..Default::default()
    };
    api::search_knowledge(d, q, &Target::at(cwd), &opts).unwrap()
}

fn file<'a>(v: &'a Value, repo: &str, rel: &str) -> &'a Value {
    v["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["repo"] == repo && f["file"] == rel)
        .unwrap_or_else(|| panic!("沒有 {repo}:{rel}：{v}"))
}

fn whys(f: &Value) -> Vec<String> {
    f["why"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn archive_paths_rank_lower_and_title_hit_needs_half() {
    let (tmp, d) = spine();
    let a = register(&d, tmp.path(), "alpha");
    let body = "# 向量索引冷啟動\n冷啟動時先載入快照，再補增量；向量索引的水位線要記。\n";
    write(&a, "notes/cold.md", body);
    write(&a, "archive/old/cold.md", body);
    write(&a, "notes/01_inbox_retired_20261104/cold.md", body);
    // 標題只碰到查詢的一小部分
    write(
        &a,
        "notes/misc.md",
        "# 快照\n向量索引冷啟動、水位線、增量、載入都在這裡講了一遍。\n",
    );
    let v = full(&d, "向量索引 冷啟動", &a);
    let live = file(&v, "alpha", "notes/cold.md");
    let arch = file(&v, "alpha", "archive/old/cold.md");
    let retired = file(&v, "alpha", "notes/01_inbox_retired_20261104/cold.md");
    assert!(live["score"].as_f64() > arch["score"].as_f64());
    assert!(whys(arch).contains(&"在封存路徑".to_string()));
    assert!(whys(retired).contains(&"在封存路徑".to_string()));
    assert!(!whys(live).contains(&"在封存路徑".to_string()));
    assert!(
        whys(live).contains(&"標題命中".to_string()),
        "{:?}",
        whys(live)
    );
    assert!(
        !whys(file(&v, "alpha", "notes/misc.md")).contains(&"標題命中".to_string()),
        "標題只碰到一小部分不標"
    );
}

#[test]
fn brief_picks_repo_and_keeps_its_ref() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let here = register(&d, t, "here");
    let deep = register(&d, t, "deep");
    let wide = register(&d, t, "wide");
    // deep：三份正中的檔；wide：很多份中等的檔＋名片寫得很清楚
    for i in 0..3 {
        write(
            &deep,
            &format!("docs/d{i}.md"),
            "# 排水縱斷面 高程 verifier\n排水縱斷面的高程 verifier 規則與例外，高程差、縱斷面、verifier。\n",
        );
    }
    for i in 0..10 {
        write(
            &wide,
            &format!("notes/w{i}.md"),
            &format!("# 筆記 {i}\n這份順帶提到排水與高程，還有一些別的東西，篇幅很長很長很長很長很長很長很長很長。\n"),
        );
    }
    set_field(
        &d,
        "wide",
        "description",
        "排水縱斷面 高程 verifier 的規格來源",
    )
    .unwrap();
    set_field(&d, "wide", "role", "規格來源").unwrap();
    let opts = SearchOpts::default();
    let b = api::search_brief(&d, "排水縱斷面 高程 verifier", &Target::at(&here), &opts).unwrap();
    let ans = b.answer_repo.as_ref().expect("有答案");
    let s = serde_json::to_string(&b).unwrap();
    assert!(
        s.chars().count() <= api::BRIEF_MAX_CHARS,
        "{}",
        s.chars().count()
    );
    assert!(b.refs.len() <= 3 && !b.refs.is_empty());
    assert!(
        b.refs
            .iter()
            .any(|r| r.reference.starts_with(&format!("{}:", ans.repo))),
        "起點要含 answer_repo（{}）：{:?}",
        ans.repo,
        b.refs
    );
    assert!(!ans.why.is_empty());
    assert!(b.next.contains(&format!("repo_entry {}", ans.repo)));
    assert!(b.also.iter().all(|x| x.repo != ans.repo));
    // 沒結果：不硬湊
    let none =
        api::search_brief(&d, "完全不相干的詞彙組合 zzqx", &Target::at(&here), &opts).unwrap();
    assert!(none.answer_repo.is_none());
    assert_eq!(none.note.as_deref(), Some("沒找到明顯相關的 repo"));
    assert!(none.refs.is_empty());
}

#[test]
fn brief_is_capped_with_long_content() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let a = register(&d, t, "alpha");
    let long = "冷啟動 水位線 ".repeat(400);
    for i in 0..30 {
        write(
            &a,
            &format!("docs/{i}/{}.md", "很長的檔名".repeat(10)),
            &format!("# 冷啟動 水位線 {}\n{long}\n", "標題".repeat(50)),
        );
    }
    for _ in 0..5 {
        api::log_decision(
            &d,
            &format!("冷啟動要記水位線。{long}"),
            &Target::at(&a),
            None,
            false,
            "agent",
        )
        .unwrap();
    }
    set_field(
        &d,
        "alpha",
        "description",
        "冷啟動與水位線的知識庫".repeat(20).as_str(),
    )
    .unwrap();
    let b =
        api::search_brief(&d, "冷啟動 水位線", &Target::at(&a), &SearchOpts::default()).unwrap();
    let n = serde_json::to_string(&b).unwrap().chars().count();
    assert!(n <= api::BRIEF_MAX_CHARS, "{n}");
    assert!(b.answer_repo.is_some() && !b.refs.is_empty());
}

#[test]
fn decisions_mark_which_repo_they_talk_about() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let a = register(&d, t, "draftagent");
    register(&d, t, "RepoContextWorkbench");
    api::log_decision(
        &d,
        "RepoContextWorkbench 的接力：verifier 那段先不做",
        &Target::at(&a),
        None,
        false,
        "agent",
    )
    .unwrap();
    api::log_decision(
        &d,
        "verifier 的高程規則改成容許 5 公分",
        &Target::at(&a),
        None,
        false,
        "agent",
    )
    .unwrap();
    let opts = SearchOpts {
        everywhere: false,
        code: false,
        ..Default::default()
    };
    let v = api::search_knowledge(
        &d,
        "verifier",
        &Target::at(&a).with_repos(&["draftagent"]),
        &opts,
    )
    .unwrap();
    let decs = v["decisions"].as_array().unwrap();
    assert_eq!(decs.len(), 2);
    // 限定範圍：講別的 repo 的排到後面，並標出來
    assert!(decs[0].get("about_repo").is_none(), "{decs:?}");
    assert_eq!(decs[1]["about_repo"], "RepoContextWorkbench");
    // 精簡模式：沒有歸屬疑慮的優先
    let b = api::search_brief(&d, "verifier", &Target::at(&a), &SearchOpts::default()).unwrap();
    assert!(b.decisions[0].about_repo.is_none(), "{:?}", b.decisions);
}

#[test]
fn repo_entry_sections_and_limits() {
    let (tmp, d) = spine();
    let t = tmp.path();
    let a = register(&d, t, "alpha");
    write(&a, "AGENTS.md", "# alpha 的 agent 規則\n…\n");
    write(&a, "CLAUDE.md", "# alpha 的 agent 規則\n@AGENTS.md\n");
    register(&d, t, "beta");
    write(&a, "docs/index.md", "---\nx: y\n---\n# 文件索引\n");
    set_field(&d, "alpha", "description", "冷啟動與水位線的知識庫").unwrap();
    set_field(&d, "alpha", "role", "知識庫").unwrap();
    api::log_decision(
        &d,
        "冷啟動先載入快照",
        &Target::at(&a),
        None,
        false,
        "agent",
    )
    .unwrap();
    api::log_decision(
        &d,
        "水位線記在 state",
        &Target::at(&a),
        None,
        false,
        "agent",
    )
    .unwrap();
    // 記在 alpha、內容在講 beta 的判斷不列
    api::log_decision(
        &d,
        "beta：介面改成 v2",
        &Target::at(&a),
        None,
        false,
        "agent",
    )
    .unwrap();
    api::log_decision(
        &d,
        "最新的判斷 sk-ant-FAKEFAKEFAKEFAKEFAKE",
        &Target::at(&a),
        None,
        false,
        "agent",
    )
    .unwrap(); // 假憑證，只用來測遮蔽
    api::handoff(
        &d,
        "寫完冷啟動",
        "補測試",
        &[] as &[&str],
        &Target::at(&a),
        false,
        "agent",
    )
    .unwrap();
    std::fs::write(a.join("wip.md"), "x").unwrap();
    let e = api::repo_entry(&d, "alpha").unwrap();
    let paths: Vec<&str> = e.read_first.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["AGENTS.md", "README.md", "docs/index.md"]);
    assert_eq!(e.read_first[0].title, "alpha 的 agent 規則");
    assert_eq!(e.read_first[2].title, "文件索引");
    assert_eq!(e.decisions.len(), 2, "最近 2 筆、不含交接");
    assert!(
        e.decisions.iter().all(|x| !x.text.starts_with("beta")),
        "講別的 repo 的不列：{:?}",
        e.decisions
    );
    assert!(
        !e.decisions[0].text.contains("sk-ant-FAKE"),
        "{:?}",
        e.decisions
    );
    assert!(e.last_handoff.as_ref().unwrap().done.contains("寫完冷啟動"));
    assert_eq!(
        e.now.dirty,
        Some(4),
        "AGENTS.md、CLAUDE.md、docs/index.md、wip.md 都還沒 commit"
    );
    assert_eq!(e.about.as_deref(), Some("冷啟動與水位線的知識庫"));
    assert!(e.next.contains("read_from"));
    assert!(serde_json::to_string(&e).unwrap().chars().count() <= api::ENTRY_MAX_CHARS);
    // 很長的內容也不超過上限
    set_field(
        &d,
        "alpha",
        "description",
        "很長的介紹".repeat(100).as_str(),
    )
    .unwrap();
    for i in 0..6 {
        write(&a, &format!("x{i}.md"), "x");
    }
    let e = api::repo_entry(&d, "alpha").unwrap();
    assert!(serde_json::to_string(&e).unwrap().chars().count() <= api::ENTRY_MAX_CHARS);
    // 路徑不存在／沒登記：可補救的錯誤
    std::fs::remove_dir_all(&a).unwrap();
    for id in ["alpha", "ghost"] {
        match api::repo_entry(&d, id).unwrap_err() {
            ctx_core::Error::Rejected { code, hint, .. } => {
                assert_eq!(code.as_str(), "not_found");
                assert!(hint.is_some());
            }
            other => panic!("{other:?}"),
        }
    }
}
