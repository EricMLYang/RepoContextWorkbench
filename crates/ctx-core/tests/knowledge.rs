//! knowledge 模組測試。Python 對 knowledge 函式的直接測試只有 `test_agentapi.py` 的
//! `test_tokenize_mixes_cjk_bigrams_and_words`；其餘（search_knowledge／crossref 的 boost、code search）
//! 都經 agentapi＋registry，等那邊搬完再搬。
//!
//! 未搬：test_agentapi.test_search_knowledge_ranks_repos、test_search_skips_secret_files_and_caches、
//! test_crossref.test_search_neighbor_boost_and_why——經 agentapi＋spine/registry（等第一波 spine/registry）。
//! 下面補的 Rust 測試直接在 knowledge 層驗同樣的行為（排序、哨兵、快取、boost＋why）。

mod common;

use std::collections::HashSet;

use ctx_core::knowledge::{self, Boosts, PathBoost};
use ctx_core::pack::RepoRef;

#[test]
fn test_tokenize_mixes_cjk_bigrams_and_words() {
    let toks = knowledge::tokenize("Agent 決策平台 for LRU");
    assert!(toks.contains(&"agent".to_string()) && toks.contains(&"lru".to_string()));
    assert!(!toks.contains(&"for".to_string()));
    let set: HashSet<&str> = toks.iter().map(String::as_str).collect();
    assert!(["決策", "策平", "平台"].iter().all(|t| set.contains(t)));
}

// ---- 以下為 Rust 版補測（非 Python 一對一）----

#[test]
fn tokenize_edge_cases() {
    assert_eq!(knowledge::tokenize("中"), vec!["中"]);
    assert_eq!(
        knowledge::tokenize("v1.2-rc a_b x"),
        vec!["v1.2-rc", "a_b", "x"]
    );
    assert!(knowledge::tokenize("the and of").is_empty());
}

#[test]
fn heading_text_has_stem_frontmatter_and_headings() {
    let h = knowledge::heading_text("---\ntitle: 快取\n---\n# 大標\n內文\n## 小標\n", "notes.md");
    assert_eq!(h, "notes\ntitle: 快取\n大標\n小標");
}

/// 對應 test_search_knowledge_ranks_repos 的精神：命中的 repo 排前面，片段帶行號。
#[test]
fn search_ranks_repos_and_snippets() {
    let w = common::spine_with_repos();
    std::fs::write(
        w.repo_b.join("cache.md"),
        "# 淘汰策略\n\nLRU 淘汰策略的細節\n淘汰\n",
    )
    .unwrap();
    let r = knowledge::search(&w.refs(), "淘汰策略", 8, None, &[], &Boosts::default());
    assert_eq!(r.repos[0].repo, "repo-b");
    assert_eq!(r.files[0].file, "cache.md");
    assert_eq!(r.files[0].why[0], "標題命中");
    assert!(r.files[0].why.iter().any(|x| x == "30 天內改過"));
    assert_eq!(r.files[0].snippets[0].line, 1);
    assert!(r.indexed_files.unwrap() >= 6 && r.note.is_none());
    let empty = knowledge::search(&w.refs(), "the of", 8, None, &[], &Boosts::default());
    assert_eq!(empty.note.as_deref(), Some("查詢字串斷不出可比對的詞"));
    let v = serde_json::to_value(&empty).unwrap();
    assert!(v.get("indexed_files").is_none());
}

/// 對應 test_search_skips_secret_files_and_caches：哨兵檔不進索引；有 spine 就寫快取，再查走快取結果一致。
#[test]
fn search_skips_secret_files_and_caches() {
    let w = common::spine_with_repos();
    std::fs::write(
        w.repo_a.join("keys.md"),
        "# 部署\nAKIAABCDEFGHIJKLMNOP 部署金鑰\n",
    )
    .unwrap();
    let r = knowledge::search(
        &w.refs(),
        "部署金鑰",
        8,
        Some(&w.spine),
        &[],
        &Boosts::default(),
    );
    assert!(r.files.iter().all(|f| f.file != "keys.md"));
    let cache = w.spine.join(".state").join("knowledge_index.json");
    assert!(cache.exists());
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cache).unwrap()).unwrap();
    let key = w.repo_a.join("keys.md").to_string_lossy().into_owned();
    assert_eq!(v[&key]["secret"], true);
    let readme = w.repo_a.join("README.md").to_string_lossy().into_owned();
    assert_eq!(v[&readme]["v"], 2);
    assert!(v[&readme]["tf"]["快取"].as_u64().unwrap() >= 1);
    let r2 = knowledge::search(
        &w.refs(),
        "widget 快取",
        8,
        Some(&w.spine),
        &[],
        &Boosts::default(),
    );
    let r3 = knowledge::search(&w.refs(), "widget 快取", 8, None, &[], &Boosts::default());
    assert_eq!(r2.files, r3.files);
}

/// 對應 test_search_neighbor_boost_and_why 的精神：boost 改變排序並留下 why；exclude_paths 排除來源檔。
#[test]
fn search_boosts_and_excludes() {
    let w = common::spine_with_repos();
    let refs = vec![
        RepoRef::new("repo-a", &w.repo_a),
        RepoRef::new("repo-b", &w.repo_b),
    ];
    let mut boosts = Boosts::default();
    boosts
        .repos
        .insert("repo-b".into(), (3.0, "鄰居 repo".into()));
    boosts.paths.push(PathBoost {
        repo: "repo-b".into(),
        prefix: "notes/".into(),
        factor: 1.5,
        reason: "export 內".into(),
    });
    let r = knowledge::search(&refs, "LRU 快取", 8, None, &[], &boosts);
    assert_eq!(r.files[0].repo, "repo-b");
    assert_eq!(r.files[0].file, "notes/plan.md");
    assert!(
        r.files[0].why.iter().any(|x| x == "鄰居 repo")
            && r.files[0].why.iter().any(|x| x == "export 內")
    );
    let ex = vec![w.repo_b.join("notes").join("plan.md")];
    let r2 = knowledge::search(&refs, "LRU 快取", 8, None, &ex, &boosts);
    assert!(r2
        .files
        .iter()
        .all(|f| !(f.repo == "repo-b" && f.file == "notes/plan.md")));
}
