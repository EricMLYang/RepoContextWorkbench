//! `ctx home sync [--check]`（F-132，Rust 才有）的 CLI 入口：exit code、--json、自動重生、health 提醒。

mod common;

use common::*;

#[test]
fn check_fails_when_stale_and_passes_after_sync() {
    let (t, d) = spine_with_repos();
    let c = ctx(t.path()).spine(&d);
    // testkit 直接呼叫 registry 函式（不經 ops）：還沒生成過
    let o = c.run(&["home", "sync", "--check"]);
    assert_eq!(o.code, 1, "{}", o.all());
    assert!(
        o.stdout.contains("過期（會新建）  AGENTS.md"),
        "{}",
        o.stdout
    );
    let j = c.run(&["home", "sync", "--check", "--json"]);
    assert_eq!(j.code, 1);
    let v = j.json();
    assert_eq!(v["up_to_date"], false);
    assert!(v["stale"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x == "groups/g1/REPOS.md"));

    let s = c.ok(&["home", "sync", "--json"]).json();
    assert_eq!(s["ok"], true);
    assert!(s["files"]
        .as_array()
        .unwrap()
        .iter()
        .all(|f| f["status"] == "written"));
    let o = c.ok(&["home", "sync", "--check"]);
    assert!(o.stdout.contains("都是最新的"), "{}", o.stdout);
    let again = c.ok(&["home", "sync", "--json"]).json();
    assert!(again["files"]
        .as_array()
        .unwrap()
        .iter()
        .all(|f| f["status"] == "unchanged"));

    // registry 經 CLI 改了：自動重生，check 仍是 0
    c.ok(&[
        "registry",
        "describe",
        "repo-a",
        "--description",
        "測試用的 repo",
    ]);
    c.ok(&["home", "sync", "--check"]);
    let g1 = std::fs::read_to_string(d.join("groups/g1/REPOS.md")).unwrap();
    assert!(g1.contains("**repo-a**（測試用的 repo）"), "{g1}");

    // 手寫檔：sync 不覆蓋、exit 1；health 多一行提醒
    std::fs::write(d.join("CLAUDE.md"), "我自己的\n").unwrap();
    let o = c.run(&["home", "sync"]);
    assert_eq!(o.code, 1, "{}", o.all());
    assert!(
        o.stdout.contains("衝突（手寫檔，沒覆蓋）  CLAUDE.md"),
        "{}",
        o.stdout
    );
    assert_eq!(
        std::fs::read_to_string(d.join("CLAUDE.md")).unwrap(),
        "我自己的\n"
    );
    let h = c.ok(&["health"]);
    assert!(
        h.stdout.contains("spine 入口檔") && h.stdout.contains("CLAUDE.md"),
        "{}",
        h.stdout
    );
    let hj = c.ok(&["health", "--json"]).json();
    assert_eq!(hj["home"]["conflicts"][0], "CLAUDE.md");
}

#[test]
fn health_reminds_when_stale() {
    let (t, d) = spine_with_repos();
    let c = ctx(t.path()).spine(&d);
    let h = c.ok(&["health"]);
    assert!(
        h.stdout.contains("spine 入口檔") && h.stdout.contains("ctx home sync"),
        "{}",
        h.stdout
    );
    c.ok(&["home", "sync"]);
    assert!(!c.ok(&["health"]).stdout.contains("spine 入口檔"));
}
