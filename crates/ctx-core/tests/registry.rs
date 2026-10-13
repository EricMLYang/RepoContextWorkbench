//! L1：P1 registry / P2 組 / audit 紅字。對應 tests/test_registry.py。

use std::collections::BTreeSet;

use ctx_core::registry::{self, AddRepo};
use ctx_core::testkit::{make_git_repo, spine, spine_with_repos};

fn ids(entries: &[registry::Repo]) -> Vec<String> {
    entries.iter().map(|e| e.id()).collect()
}

fn err_msg<T: std::fmt::Debug>(r: ctx_core::Result<T>) -> String {
    r.unwrap_err().to_string()
}

fn tier(t: &str) -> AddRepo {
    AddRepo {
        tier: t.into(),
        ..Default::default()
    }
}

/// L4 bug 回歸：（全部）範圍＝name=None 且無 repos，必須回全部而不是炸
/// 「組不存在: None」（工作台預設範圍的 agent 會話/簡報以前都死在這）。
#[test]
fn test_resolve_group_none_means_all() {
    let (_tmp, d) = spine_with_repos();
    let (gname, entries) = registry::resolve_group(&d, None, None).unwrap();
    assert_eq!(gname, "全部");
    assert_eq!(
        ids(&entries).into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from(["repo-a".into(), "repo-b".into()])
    );
}

#[test]
fn test_tag_repo_and_all_tags() {
    let (_tmp, d) = spine_with_repos();
    let none: &[&str] = &[];
    assert_eq!(
        registry::tag_repo(&d, "repo-a", &["PM", "開發"], none).unwrap(),
        vec!["PM", "開發"]
    );
    assert_eq!(
        registry::tag_repo(&d, "repo-a", &["PM"], none).unwrap(),
        vec!["PM", "開發"]
    ); // 冪等
    registry::tag_repo(&d, "repo-b", &["開發", "Demo"], none).unwrap();
    assert_eq!(registry::all_tags(&d).unwrap(), vec!["PM", "開發", "Demo"]); // 去重保序
    assert_eq!(
        registry::tag_repo(&d, "repo-a", none, &["開發", "不存在的"]).unwrap(),
        vec!["PM"]
    );
    assert!(err_msg(registry::tag_repo(&d, "ghost", &["x"], none)).contains("repo 不存在"));
}

#[test]
fn test_add_and_get() {
    let (tmp, d) = spine();
    let p = make_git_repo(tmp.path(), "repo-x", 0, 0, true);
    registry::add_repo(
        &d,
        "repo-x",
        &p,
        AddRepo {
            tags: vec!["dev".into()],
            ..Default::default()
        },
    )
    .unwrap();
    let r = registry::get_repo(&d, "repo-x").unwrap();
    assert_eq!(r.kind().as_deref(), Some("mine"));
    assert_eq!(r.tier_str().as_deref(), Some("active"));
    assert_eq!(r.tags(), vec!["dev"]);
    assert!(registry::add_repo(&d, "repo-x", &p, AddRepo::default()).is_err()); // id 重複
}

#[test]
fn test_group_and_adhoc() {
    let (_tmp, d) = spine_with_repos();
    let (name, entries) = registry::resolve_group(&d, Some("g1"), None).unwrap();
    assert!(name == "g1" && ids(&entries) == vec!["repo-a", "repo-b"]);
    let (name, entries) =
        registry::resolve_group(&d, None, Some(&registry::split_ids("repo-b"))).unwrap();
    assert!(name.starts_with("臨時(") && entries[0].id() == "repo-b");
    assert!(registry::add_group(&d, "g2", &["ghost"]).is_err()); // 組員未登記
}

#[test]
fn test_audit_red_flags() {
    let (tmp, d) = spine();
    let ok = make_git_repo(tmp.path(), "ok-repo", 1, 0, true);
    let stale = make_git_repo(tmp.path(), "stale-repo", 45, 0, true);
    registry::add_repo(&d, "ok-repo", &ok, AddRepo::default()).unwrap();
    registry::add_repo(&d, "stale-repo", &stale, AddRepo::default()).unwrap(); // tier 漂移
    registry::add_repo(
        &d,
        "ghost",
        tmp.path().join("no-such-dir"),
        AddRepo::default(),
    )
    .unwrap(); // 路徑失效
    registry::add_repo(&d, "napper", &ok, tier("paused")).unwrap(); // paused 無 resume_when
    registry::add_repo(
        &d,
        "ext",
        &ok,
        AddRepo {
            kind: "external".into(),
            ..Default::default()
        },
    )
    .unwrap(); // external 無 upstream
    let none: &[&str] = &[];
    let text = registry::audit(&d, none).unwrap().join("\n");
    assert!(text.contains("tier 漂移") && text.contains("stale-repo"));
    assert!(text.contains("路徑失效") && text.contains("ghost"));
    assert!(text.contains("resume_when") && text.contains("napper"));
    assert!(text.contains("external 無 upstream"));
    assert!(!text.contains("ok-repo"));
}

#[test]
fn test_audit_unregistered_scan() {
    let (tmp, d) = spine();
    make_git_repo(&tmp.path().join("zone"), "secret-repo", 0, 0, true);
    let reds = registry::audit(&d, &[tmp.path().join("zone")]).unwrap();
    assert!(reds
        .iter()
        .any(|r| r.contains("未登記") && r.contains("secret-repo")));
}

#[test]
fn test_audit_clean() {
    let (tmp, d) = spine();
    let p = make_git_repo(tmp.path(), "clean-repo", 2, 0, true);
    registry::add_repo(&d, "clean-repo", &p, AddRepo::default()).unwrap();
    let none: &[&str] = &[];
    assert_eq!(registry::audit(&d, none).unwrap(), Vec::<String>::new());
}

/// mani「init 自動掃描＋手寫補語意」混合模式：下半身機器填、上半身留人。
#[test]
fn test_scan_dir_candidates_and_register() {
    let (tmp, d) = spine();
    let zone = tmp.path().join("zone");
    let a = make_git_repo(&zone, "new-a", 0, 0, true);
    make_git_repo(&zone, "new-b", 0, 0, true);
    std::fs::create_dir(zone.join("not-a-repo")).unwrap(); // 沒 .git 不算
    registry::add_repo(&d, "new-a", &a, AddRepo::default()).unwrap(); // 已登記者不重列
    let cands = registry::scan_dir(&d, &zone).unwrap();
    assert_eq!(
        cands.iter().map(|c| c.1.as_str()).collect::<Vec<_>>(),
        vec!["new-b"]
    );
    assert_eq!(registry::scan_register(&d, &zone).unwrap(), vec!["new-b"]);
    let r = registry::get_repo(&d, "new-b").unwrap();
    assert_eq!(r.kind().as_deref(), Some("mine")); // 下半身預設值
    assert_eq!(r.tier_str().as_deref(), Some("active"));
    assert!(registry::scan_register(&d, &zone).unwrap().is_empty()); // 冪等：再掃無新增
}

/// 同名資料夾登在別處 → 建議 id 加序號，不覆蓋既有登記。
#[test]
fn test_scan_dir_id_collision_gets_suffix() {
    let (tmp, d) = spine();
    let p1 = make_git_repo(&tmp.path().join("elsewhere"), "dup", 0, 0, true);
    registry::add_repo(&d, "dup", &p1, AddRepo::default()).unwrap();
    make_git_repo(&tmp.path().join("zone"), "dup", 0, 0, true);
    let cands = registry::scan_dir(&d, tmp.path().join("zone")).unwrap();
    assert_eq!(
        cands.iter().map(|c| c.1.as_str()).collect::<Vec<_>>(),
        vec!["dup-2"]
    );
    assert!(
        err_msg(registry::scan_dir(&d, tmp.path().join("no-such-zone"))).contains("目錄不存在")
    );
}

#[test]
fn test_remove_repo_cleans_groups() {
    let (_tmp, d) = spine_with_repos();
    registry::remove_repo(&d, "repo-b").unwrap();
    let data = registry::load(&d).unwrap();
    assert_eq!(ids(&data.repos), vec!["repo-a"]);
    assert_eq!(data.group("g1").unwrap().members(), vec!["repo-a"]); // membership 同步清掉
    assert!(registry::remove_repo(&d, "ghost").is_err());
}

/// 反向 tier 漂移（2026-09-08 真資料實錘）：dormant 卻近 7 天有 commit → 紅字。
#[test]
fn test_audit_dormant_but_recently_active() {
    let (tmp, d) = spine();
    registry::add_repo(
        &d,
        "sleepy",
        make_git_repo(tmp.path(), "sleepy", 1, 0, true),
        tier("dormant"),
    )
    .unwrap();
    registry::add_repo(
        &d,
        "truly",
        make_git_repo(tmp.path(), "truly", 60, 0, true),
        tier("dormant"),
    )
    .unwrap();
    let none: &[&str] = &[];
    let reds = registry::audit(&d, none).unwrap();
    assert!(reds
        .iter()
        .any(|r| r.contains("sleepy") && r.contains("dormant 但")));
    assert!(!reds.iter().any(|r| r.contains("truly")));
}

// ── 合約：registry.yaml 保留未知鍵與鍵順序，Python 讀得回 ───────────────

#[test]
fn registry_yaml_keeps_unknown_keys_and_order() {
    let (_tmp, d) = spine();
    std::fs::write(
        d.join("registry.yaml"),
        "# 手寫\nversion: 2\nrepos:\n- id: r1\n  zeta: 1\n  path: /nowhere\n  type: mine\n  tags: []\n  tier: active\n  custom:\n    nested: [a, b]\ngroups: []\nfooter: keep\n",
    )
    .unwrap();
    registry::set_field(&d, "r1", "note", "新增欄位").unwrap();
    registry::set_field(&d, "r1", "zeta", 2).unwrap();
    let text = std::fs::read_to_string(d.join("registry.yaml")).unwrap();
    assert_eq!(
        text,
        "version: 2\nrepos:\n- id: r1\n  zeta: 2\n  path: /nowhere\n  type: mine\n  tags: []\n  tier: active\n  custom:\n    nested:\n    - a\n    - b\n  note: 新增欄位\ngroups: []\nfooter: keep\n"
    );
}

#[test]
fn refuses_root_and_home_as_repo() {
    let (_t, d) = spine();
    let err = registry::add_repo(&d, "root", "/", AddRepo::default()).unwrap_err();
    assert!(err.to_string().contains("範圍太大"), "{err}");
    if let Some(home) = std::env::var_os("HOME") {
        assert!(registry::add_repo(&d, "home", &home, AddRepo::default()).is_err());
        assert!(registry::add_repo(&d, "tilde", "~", AddRepo::default()).is_err());
        let parent = std::path::Path::new(&home).parent().unwrap().to_path_buf();
        assert!(registry::add_repo(&d, "above-home", &parent, AddRepo::default()).is_err());
    }
}
