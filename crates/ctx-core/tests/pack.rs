//! 對應 `tests/test_pack_route_collide.py` 裡 **pack** 相關的測試（route／collide 另搬）。
//! fixture 不經 registry：entries 由 `common::spine_with_repos` 手組（等同 `resolve_group(spine, "g1")`）。

mod common;

use ctx_core::pack::{self, RepoRef};
use ctx_core::testkit::make_git_repo;

#[test]
fn test_pack_budget_and_exclusion() {
    let w = common::spine_with_repos();
    let entries = w.refs();
    let p = pack::pack_group(&entries, 100000);
    assert!(p.excluded.is_empty() && p.included.len() >= 4); // 兩 repo 各 README+plan
    assert!(p.flagged.is_empty()); // fixture 無機密
    assert!(p.text.contains("widget 快取設計")); // 內容真的在
                                                 // 縮到很小的預算 → 有東西被擠出去，且標頭有列
    let p2 = pack::pack_group(&entries, 15);
    assert!(!p2.excluded.is_empty() && p2.text.contains("未納入"));
}

/// L4 實錘回歸：gitignored 的 clone 森林不得進料（rglob 時代 1551 檔擠占預算）。
#[test]
fn test_pack_respects_gitignore() {
    let w = common::spine_with_repos();
    let clones = w.repo_a.join("tool_clones").join("some-clone");
    std::fs::create_dir_all(&clones).unwrap();
    std::fs::write(clones.join("noise.md"), "clone 雜訊\n".repeat(50)).unwrap();
    std::fs::write(w.repo_a.join(".gitignore"), "tool_clones/\n").unwrap();
    let entries = vec![RepoRef::new("repo-a", &w.repo_a)];
    let p = pack::pack_group(&entries, 100000);
    assert!(!p.text.contains("clone 雜訊"));
    assert!(!p
        .included
        .iter()
        .chain(&p.excluded)
        .any(|x| x.contains("noise.md")));
    assert!(p.text.contains("widget 快取設計")); // 自己的正文還在
}

/// 洩密哨兵（Repomix Secretlint 課）：長得像憑證的檔案不進包、有列出。
#[test]
fn test_pack_secret_sentinel() {
    let w = common::spine_with_repos();
    std::fs::write(
        w.repo_a.join("oops.md"),
        "# 筆記\n-----BEGIN RSA PRIVATE KEY-----\nMIIEow…\n",
    )
    .unwrap();
    let entries = vec![RepoRef::new("repo-a", &w.repo_a)];
    let p = pack::pack_group(&entries, 100000);
    assert!(p.flagged.iter().any(|x| x.contains("oops.md")));
    assert!(!p.text.contains("BEGIN RSA PRIVATE KEY") && !p.text.contains("MIIEow")); // 內容不出 repo
    assert!(p.text.contains("疑似機密")); // 判定時知道被擋了什麼
    assert!(!p
        .included
        .iter()
        .chain(&p.excluded)
        .any(|x| x.contains("oops.md")));
}

/// token 預算函數（Repomix 課）：挑的粒度由數字反推——先算再挑。
#[test]
fn test_pack_estimate_budget_function() {
    let w = common::spine_with_repos();
    let entries = w.refs();
    let rows = pack::estimate(&entries);
    let ids: std::collections::HashSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["repo-a", "repo-b"].into_iter().collect());
    for r in &rows {
        assert!(r.files >= 2 && r.tokens > 0);
    }
    // estimate 的合計要跟真打包（大預算、全收錄）一致量級：全部裝得進該數字的預算
    let total: usize = rows.iter().map(|r| r.tokens).sum();
    let p = pack::pack_group(&entries, total);
    assert!(p.excluded.is_empty());
}

/// git 預設把非 ASCII 路徑跳脫加引號——中文檔名的卡片曾經整批被略過（2026-09-25）。
#[test]
fn test_doc_files_keep_non_ascii_names() {
    let tmp = tempfile::tempdir().unwrap();
    let r = make_git_repo(tmp.path(), "cards", 0, 0, true);
    std::fs::create_dir_all(r.join("卡片")).unwrap();
    std::fs::write(r.join("卡片").join("決策平台.md"), "# 決策平台\n").unwrap();
    let names: Vec<String> = pack::doc_files(&r)
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().any(|n| n == "決策平台.md"));
}

// route 相關在 tests/route.rs；collide 相關（test_collide_two_stage_mock、test_collide_inner_validation_retry、
// test_collide_system_unsure_surfaces、test_collision_id_sequence、test_agent_raw_log_written、
// test_mock_verdict_follows_idea_not_template、test_collide_adhoc_repos_token_roundtrip）在 tests/collide.rs。
// 其中 agents 層的行為（mock 重試、AgentUnsure、raw log、只看【想法】段）先在 tests/agents.rs 直接測。

// ---- 以下為 Rust 版補測（非 Python 一對一）----

/// 非 git 目錄 fallback 遞迴：略過 EXCLUDE_DIRS 與 `.` 開頭目錄。
#[test]
fn doc_files_fallback_without_git() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("plain");
    for d in ["a/b", "node_modules/x", ".hidden"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(root.join("top.md"), "t").unwrap();
    std::fs::write(root.join("a/b/deep.md"), "d").unwrap();
    std::fs::write(root.join("node_modules/x/skip.md"), "s").unwrap();
    std::fs::write(root.join(".hidden/skip2.md"), "s").unwrap();
    std::fs::write(root.join("a/not.txt"), "n").unwrap();
    let mut names: Vec<String> = pack::doc_files(&root)
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["deep.md", "top.md"]);
}

#[test]
fn pack_index_lists_map_not_fulltext() {
    let w = common::spine_with_repos();
    let text = pack::pack_index(&w.refs(), 80);
    assert!(text.starts_with("# 會話料（文件地圖"));
    assert!(text.contains("## repo-a（2 個 md，路徑："));
    assert!(text.contains("- README.md（3 行）｜repo-a"));
    assert!(text.contains("- notes/plan.md（3 行）｜計劃"));
    assert!(!text.contains("widget 快取設計"));
    let small = pack::pack_index(&w.refs(), 1);
    assert!(small.contains("- …另 1 個較小檔未列"));
}
