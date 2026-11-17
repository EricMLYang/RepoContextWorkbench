//! spine 靜態入口（F-132）：registry → AGENTS.md／CLAUDE.md／groups/<組>/REPOS.md。

use std::collections::BTreeMap;
use std::path::Path;

use ctx_core::home::{self, Outcome, Pending, RepoLocal, MARKER};
use ctx_core::ops::{self, Via};
use ctx_core::registry::{self, AddRepo};
use ctx_core::testkit::{git, make_git_repo, spine, spine_with_repos};
use serde_json::json;

const FIXED_REGISTRY: &str = r#"repos:
- id: impl
  path: /work/impl
  type: mine
  tier: active
  class: code
  role: 實作
  description: 搜尋服務的後端
- id: plan
  path: /work/plan
  type: mine
  tier: active
  class: docs
- id: orca
  path: /work/ref/orca
  type: external
  tier: active
  upstream: stablyai/orca
  description: |
    別人的 agent 執行台，
    看它怎麼管 PTY。
- id: notes
  path: /work/notes
  type: mine
  tier: active
  class: knowledge
- id: loose
  path: /work/gone
  type: mine
  tier: active
groups:
- name: 產品
  members: [impl, plan, orca]
  goal: 把搜尋服務的向量索引改成增量重建
- name: 研究
  members: [notes, orca]
"#;

fn fixed_data(dir: &Path) -> registry::RegistryData {
    std::fs::write(dir.join("registry.yaml"), FIXED_REGISTRY).unwrap();
    registry::load(dir).unwrap()
}

fn fixed_local() -> BTreeMap<String, RepoLocal> {
    let l = |exists: bool, remote: Option<&str>| RepoLocal {
        exists,
        remote: remote.map(str::to_string),
    };
    BTreeMap::from([
        ("impl".into(), l(true, Some("git@github.com:me/impl.git"))),
        ("plan".into(), l(true, None)),
        (
            "orca".into(),
            l(true, Some("https://github.com/stablyai/orca")),
        ),
        (
            "notes".into(),
            l(true, Some("https://example.com/notes.git")),
        ),
        ("loose".into(), l(false, None)),
    ])
}

fn content<'a>(files: &'a [home::GenFile], rel: &str) -> &'a str {
    &files
        .iter()
        .find(|f| f.rel == rel)
        .unwrap_or_else(|| panic!("沒生成 {rel}"))
        .content
}

#[test]
fn fixed_registry_renders_fixed_files() {
    let t = tempfile::tempdir().unwrap();
    let files = home::render(&fixed_data(t.path()), &fixed_local());
    let rels: Vec<&str> = files.iter().map(|f| f.rel.as_str()).collect();
    assert_eq!(
        rels,
        [
            "AGENTS.md",
            "CLAUDE.md",
            "groups/產品/REPOS.md",
            "groups/研究/REPOS.md",
            "groups/_ungrouped/REPOS.md"
        ]
    );
    assert_eq!(
        content(&files, "CLAUDE.md"),
        format!("{MARKER}\n@AGENTS.md\n")
    );
    assert_eq!(
        content(&files, "groups/產品/REPOS.md"),
        format!(
            "{MARKER}
# 組「產品」的 repo（3 個）

目標：把搜尋服務的向量索引改成增量重建
每個 repo 一行：是什麼、本機路徑、沒 clone 時從哪 clone、動它之前要知道的事。要更多（從哪幾份讀起、最近判斷、最後交接）用 MCP `repo_entry`，或 `ctx repo entry <id> --json`。

- **impl**（實作：搜尋服務的後端）：/work/impl。Clone from git@github.com:me/impl.git if missing. 改之前先讀它的 AGENTS.md。
- **plan**（文件・PM）：/work/plan。事實別補、別外傳。
- **orca**（別人的 agent 執行台，看它怎麼管 PTY）：/work/ref/orca。Clone from https://github.com/stablyai/orca if missing. 只讀，不要改。
"
        )
    );
    assert_eq!(
        content(&files, "groups/_ungrouped/REPOS.md"),
        format!(
            "{MARKER}
# 未分組的 repo（1 個）

每個 repo 一行：是什麼、本機路徑、沒 clone 時從哪 clone、動它之前要知道的事。要更多（從哪幾份讀起、最近判斷、最後交接）用 MCP `repo_entry`，或 `ctx repo entry <id> --json`。

- **loose**（未分類）：/work/gone（路徑不存在）。改之前先讀它的 AGENTS.md。
"
        )
    );
    assert!(content(&files, "groups/研究/REPOS.md")
        .contains("- **notes**（知識）：/work/notes。Clone from https://example.com/notes.git if missing. 可自由整理，引用附出處。"));
    let agents = content(&files, "AGENTS.md");
    assert!(agents.starts_with(&format!("{MARKER}\n# repo-context spine\n")));
    assert!(agents.contains("登記了 5 個 repo、分成 2 個組"));
    let repos_section: Vec<&str> = agents
        .split("## Repos\n")
        .nth(1)
        .unwrap()
        .lines()
        .take_while(|l| !l.is_empty())
        .collect();
    assert_eq!(
        repos_section,
        [
            "- 產品（3 個 repo）：把搜尋服務的向量索引改成增量重建 → `groups/產品/REPOS.md`",
            "- 研究（2 個 repo） → `groups/研究/REPOS.md`",
            "- 未分組（1 個 repo） → `groups/_ungrouped/REPOS.md`",
        ]
    );
    // 工具名照 MCP 的真名（SKILL.md）；沒掛 MCP 的退路是 ctx --json
    for tool in [
        "where_am_i",
        "context_for",
        "search_knowledge",
        "repo_entry",
        "read_from",
    ] {
        assert!(agents.contains(&format!("`{tool}`")), "{tool}");
    }
    assert!(agents.contains("ctx <where|context|next|search|read|repo entry <id>> --json"));
    assert!(agents.contains("`registry.yaml`") && agents.contains("`config.yaml`"));
    assert!(agents.lines().count() <= home::AGENTS_MAX_LINES);
}

#[test]
fn no_ungrouped_file_when_every_repo_is_grouped() {
    let t = tempfile::tempdir().unwrap();
    let yaml = FIXED_REGISTRY.replace(
        "  members: [notes, orca]",
        "  members: [notes, orca, loose]",
    );
    std::fs::write(t.path().join("registry.yaml"), yaml).unwrap();
    let files = home::render(&registry::load(t.path()).unwrap(), &fixed_local());
    assert!(files.iter().all(|f| !f.rel.contains("_ungrouped")));
    assert!(!content(&files, "AGENTS.md").contains("未分組"));
}

#[test]
fn agents_md_stays_within_60_lines_with_many_groups() {
    for n in [40usize, 200] {
        let t = tempfile::tempdir().unwrap();
        let mut y = String::from("repos:\n");
        for i in 0..n {
            y.push_str(&format!("- id: r{i}\n  path: /w/r{i}\n"));
        }
        y.push_str("- id: lone\n  path: /w/lone\ngroups:\n");
        for i in 0..n {
            y.push_str(&format!(
                "- name: 組{i}\n  members: [r{i}]\n  goal: 這是一句很長很長很長很長很長很長很長很長很長很長很長很長的目標\n"
            ));
        }
        std::fs::write(t.path().join("registry.yaml"), y).unwrap();
        let files = home::render(&registry::load(t.path()).unwrap(), &BTreeMap::new());
        let agents = content(&files, "AGENTS.md");
        let lines = agents.lines().count();
        assert!(
            lines <= home::AGENTS_MAX_LINES,
            "{n} 組：{lines} 行\n{agents}"
        );
        // 每組都還找得到（短行或併行），細節在各組的檔
        assert!(agents.contains(&format!("組{}（1）", n - 1)), "{agents}");
        assert!(agents.contains("未分組（1）"));
        assert_eq!(files.len(), 2 + n + 1);
    }
}

fn write_fixed_spine() -> (tempfile::TempDir, std::path::PathBuf) {
    let (t, d) = spine();
    std::fs::write(d.join("registry.yaml"), FIXED_REGISTRY).unwrap();
    (t, d)
}

#[test]
fn sync_writes_then_is_idempotent_and_check_agrees() {
    let (_t, d) = write_fixed_spine();
    let c = home::check(&d).unwrap();
    assert!(c.files.iter().all(|f| f.status == Pending::Create), "{c:?}");
    assert_eq!(c.stale().len(), 5);
    let s = home::sync(&d).unwrap();
    assert!(
        s.files.iter().all(|f| f.status == Outcome::Written),
        "{s:?}"
    );
    assert!(d.join("AGENTS.md").is_file() && d.join("groups/產品/REPOS.md").is_file());
    // 第二次：全部 Unchanged
    let s2 = home::sync(&d).unwrap();
    assert!(
        s2.files.iter().all(|f| f.status == Outcome::Unchanged),
        "{s2:?}"
    );
    let c2 = home::check(&d).unwrap();
    assert!(c2.stale().is_empty() && c2.conflicts().is_empty());
    // 生成檔被改過（標記還在）＝過期，sync 會蓋回去
    std::fs::write(d.join("CLAUDE.md"), format!("{MARKER}\n亂改\n")).unwrap();
    let c3 = home::check(&d).unwrap();
    assert_eq!(c3.stale(), ["CLAUDE.md"]);
    home::sync(&d).unwrap();
    assert_eq!(
        std::fs::read_to_string(d.join("CLAUDE.md")).unwrap(),
        format!("{MARKER}\n@AGENTS.md\n")
    );
}

#[test]
fn handwritten_file_is_not_overwritten_and_reports_conflict() {
    let (_t, d) = write_fixed_spine();
    std::fs::write(d.join("AGENTS.md"), "# 我自己的 spine 說明\n").unwrap();
    std::fs::create_dir_all(d.join("groups/研究")).unwrap();
    std::fs::write(d.join("groups/研究/REPOS.md"), "手寫清單\n").unwrap();
    let c = home::check(&d).unwrap();
    assert_eq!(c.conflicts(), ["AGENTS.md", "groups/研究/REPOS.md"]);
    let s = home::sync(&d).unwrap();
    assert_eq!(s.conflicts(), ["AGENTS.md", "groups/研究/REPOS.md"]);
    assert_eq!(
        std::fs::read_to_string(d.join("AGENTS.md")).unwrap(),
        "# 我自己的 spine 說明\n"
    );
    assert_eq!(
        std::fs::read_to_string(d.join("groups/研究/REPOS.md")).unwrap(),
        "手寫清單\n"
    );
    // 其他檔照寫
    assert!(std::fs::read_to_string(d.join("CLAUDE.md"))
        .unwrap()
        .starts_with(MARKER));
    // 手寫檔所在的組被刪了：不是我們的檔，不刪
    let s2 = {
        registry::remove_group(&d, "研究").unwrap();
        home::sync(&d).unwrap()
    };
    assert!(
        s2.files.iter().all(|f| f.path != "groups/研究/REPOS.md"),
        "{s2:?}"
    );
    assert!(d.join("groups/研究/REPOS.md").is_file());
}

#[test]
fn group_rename_and_remove_clean_old_generated_files_only() {
    let (t, d) = spine_with_repos();
    let rc = make_git_repo(t.path(), "repo-c", 0, 0, true);
    registry::add_repo(&d, "repo-c", &rc, AddRepo::default()).unwrap();
    ops::call(
        &d,
        "group_add",
        json!({"name": "g2", "members": ["repo-c"]}),
        Via::Cli,
    )
    .unwrap();
    // ops::call 之後自動重生
    assert!(d.join("groups/g1/REPOS.md").is_file());
    assert!(d.join("groups/g2/REPOS.md").is_file());
    assert!(!d.join("groups/_ungrouped").exists());

    // g2 只有生成檔：改名不算「搬料」，舊資料夾整個收掉、新名字底下重生
    let (r, say) = ops::call(
        &d,
        "group_update",
        json!({"name": "g2", "rename": "g3"}),
        Via::Cli,
    )
    .unwrap();
    assert_eq!(r["moved_dir"], false, "{r}");
    assert!(!say.contains("已搬到"), "{say}");
    assert!(!d.join("groups/g2").exists());
    let g3 = std::fs::read_to_string(d.join("groups/g3/REPOS.md")).unwrap();
    assert!(g3.contains("# 組「g3」的 repo（1 個）") && g3.contains("**repo-c**"));
    assert!(std::fs::read_to_string(d.join("AGENTS.md"))
        .unwrap()
        .contains("`groups/g3/REPOS.md`"));

    // g1 有使用者的料：改名照舊搬資料夾，生成檔跟著更新
    std::fs::create_dir_all(d.join("groups/g1/briefs")).unwrap();
    std::fs::write(d.join("groups/g1/briefs/b.md"), "簡報\n").unwrap();
    std::fs::write(d.join("groups/g1/notes.md"), "筆記\n").unwrap();
    let (r, _) = ops::call(
        &d,
        "group_update",
        json!({"name": "g1", "rename": "主組"}),
        Via::Cli,
    )
    .unwrap();
    assert_eq!(r["moved_dir"], true);
    assert!(!d.join("groups/g1").exists());
    assert_eq!(
        std::fs::read_to_string(d.join("groups/主組/notes.md")).unwrap(),
        "筆記\n"
    );
    assert!(std::fs::read_to_string(d.join("groups/主組/REPOS.md"))
        .unwrap()
        .contains("# 組「主組」的 repo（2 個）"));

    // 刪組：生成檔刪掉，資料夾裡其他東西不動
    let (r, _) = ops::call(&d, "group_remove", json!({"name": "主組"}), Via::Cli).unwrap();
    assert!(r["kept_dir"].is_string(), "{r}");
    assert!(!d.join("groups/主組/REPOS.md").exists());
    assert_eq!(
        std::fs::read_to_string(d.join("groups/主組/notes.md")).unwrap(),
        "筆記\n"
    );
    assert!(d.join("groups/主組/briefs/b.md").is_file());
    // repo-a、repo-b 變成未分組
    let ung = std::fs::read_to_string(d.join("groups/_ungrouped/REPOS.md")).unwrap();
    assert!(ung.contains("**repo-a**") && ung.contains("**repo-b**"));

    // 只剩生成檔的組被刪：不回報「保留了料」
    let (r, _) = ops::call(&d, "group_remove", json!({"name": "g3"}), Via::Cli).unwrap();
    assert!(r["kept_dir"].is_null(), "{r}");
    assert!(!d.join("groups/g3").exists());
    let s = home::sync(&d).unwrap();
    assert!(
        s.files.iter().all(|f| f.status == Outcome::Unchanged),
        "{s:?}"
    );
}

#[test]
fn local_facts_prefer_upstream_then_origin_and_strip_credentials() {
    let (t, d) = spine();
    let a = make_git_repo(t.path(), "with-origin", 0, 0, true);
    git(
        &a,
        &[
            "remote",
            "add",
            "origin",
            "https://me:secret@example.com/a.git",
        ],
        &[],
    );
    let b = make_git_repo(t.path(), "with-upstream", 0, 0, true);
    git(
        &b,
        &["remote", "add", "origin", "https://example.com/ignored.git"],
        &[],
    );
    let c = make_git_repo(t.path(), "no-remote", 0, 0, true);
    registry::add_repo(&d, "a", &a, AddRepo::default()).unwrap();
    registry::add_repo(
        &d,
        "b",
        &b,
        AddRepo {
            upstream: Some("owner/b".into()),
            ..Default::default()
        },
    )
    .unwrap();
    registry::add_repo(&d, "c", &c, AddRepo::default()).unwrap();
    let f = home::local_facts(&registry::load(&d).unwrap());
    assert_eq!(f["a"].remote.as_deref(), Some("https://example.com/a.git"));
    assert_eq!(f["b"].remote.as_deref(), Some("https://github.com/owner/b"));
    assert_eq!(f["c"].remote, None);
    assert!(f.values().all(|l| l.exists));
}
