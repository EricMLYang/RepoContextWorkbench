//! P16 會話：指令組裝純函數＋.mcp.json 產生（MCP 自動掛載 v1 只保證 Claude Code）。對應 tests/test_session.py。
//!
//! 與 Python 刻意的差異：`.mcp.json` 指向 Rust 的 `ctx` 執行檔（`--spine <spine> mcp`），不是
//! `python -m repo_context`、也不帶 `PYTHONUTF8`——相關斷言照 Rust 版改寫。測試用 `build_with` 給假的 ctx 路徑。

use std::path::{Path, PathBuf};

use ctx_core::registry;
use ctx_core::session::{self, SessionOpts, SessionPlan};
use ctx_core::spine;
use ctx_core::testkit::{spine, spine_with_repos};

const NONE: &[&str] = &[];

fn fake_ctx() -> PathBuf {
    PathBuf::from("/opt/ctx/bin/ctx")
}

fn build(d: &Path, opts: SessionOpts<'_>) -> SessionPlan {
    session::build_with(d, opts, &fake_ctx()).unwrap()
}

#[test]
fn test_build_packs_and_mounts_via_cwd() {
    let (_t, d) = spine_with_repos();
    let plan = build(
        &d,
        SessionOpts {
            group: Some("g1"),
            task: Some("寫文供料"),
            ..Default::default()
        },
    );
    assert_eq!(plan.cwd, d); // cwd=spine → 靠 cwd .mcp.json 自動掛載
    assert!(plan.cmd[0] == "claude" && !plan.cmd.iter().any(|c| c == "--mcp-config"));
    let last = plan.cmd.last().unwrap();
    assert!(last.contains("文件地圖"));
    assert!(last.contains(&plan.pack_path.display().to_string()) && last.contains("寫文供料"));
    let text = std::fs::read_to_string(&plan.pack_path).unwrap();
    // L4 回饋「料太長」：會話料是地圖不是全文——列路徑與標題，不含檔案內文
    assert!(
        text.contains("文件地圖") && text.contains("README.md") && text.contains("notes/plan.md")
    );
    assert!(!text.contains("widget 快取設計"));
    let mcp: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(d.join(".mcp.json")).unwrap()).unwrap();
    let srv = &mcp["mcpServers"]["repo_context"];
    assert_eq!(srv["command"], "/opt/ctx/bin/ctx");
    let spine_abs = d.canonicalize().unwrap();
    assert_eq!(
        srv["args"],
        serde_json::json!(["--spine", spine_abs.to_string_lossy(), "mcp"])
    );
    assert!(srv.get("env").is_none()); // Rust 版不需要 PYTHONUTF8
}

#[test]
fn test_build_with_repo_cwd_uses_mcp_config_flag() {
    let (_t, d) = spine_with_repos();
    let repos = vec!["repo-a".to_string()];
    let plan = build(
        &d,
        SessionOpts {
            repos: Some(&repos),
            repo: Some("repo-a"),
            ..Default::default()
        },
    );
    assert_eq!(plan.cwd, registry::get_repo(&d, "repo-a").unwrap().path());
    // cwd 不在 spine → 顯式掛載；必須等號形式——claude 的 --mcp-config 吃多值，
    // 空格形式會把帶料 prompt 吞成設定檔路徑（L4 bug 回歸）
    let flag: Vec<&String> = plan
        .cmd
        .iter()
        .filter(|c| c.starts_with("--mcp-config="))
        .collect();
    assert!(flag.len() == 1 && flag[0].ends_with(".mcp.json"));
    assert!(!plan.cmd.iter().any(|c| c == "--mcp-config"));
    let i = plan.cmd.iter().position(|c| c == flag[0]).unwrap();
    assert!(i < plan.cmd.len() - 1 && plan.cmd.last().unwrap().contains("文件地圖"));
}

#[test]
fn test_ensure_mcp_json_idempotent() {
    let (_t, d) = spine();
    // 已存在、而且 command 還在：第二次給別的路徑也不動（Rust 版只在 command 不見時才重寫）
    let real = d.join("ctx-bin");
    std::fs::write(&real, "").unwrap();
    let p1 = session::ensure_mcp_json_with(&d, &real).unwrap();
    let before = std::fs::read_to_string(&p1).unwrap();
    let p2 = session::ensure_mcp_json_with(&d, Path::new("/elsewhere/ctx")).unwrap();
    assert_eq!(std::fs::read_to_string(p2).unwrap(), before);
}

// Rust 才有（2026-10-21）：.mcp.json 裡的 command 不在了（例：install-app.sh 刪掉的 bundle）就重寫成現在的 ctx
#[test]
fn test_ensure_mcp_json_rewrites_when_command_is_gone() {
    let (_t, d) = spine();
    let gone = d.join("old-bundle").join("ctx");
    std::fs::write(
        d.join(".mcp.json"),
        serde_json::json!({"mcpServers": {"repo_context": {
            "command": gone.to_string_lossy(), "args": ["--spine", "x", "mcp"]}}})
        .to_string(),
    )
    .unwrap();
    let now = fake_ctx();
    let p = session::ensure_mcp_json_with(&d, &now).unwrap();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    assert_eq!(
        v["mcpServers"]["repo_context"]["command"],
        now.to_string_lossy().as_ref()
    );
    assert_eq!(v["mcpServers"]["repo_context"]["args"][2], "mcp");
}

/// L4 bug 回歸：不給 group/repos（工作台的（全部）範圍）也要組得出會話。
#[test]
fn test_build_all_scope() {
    let (_t, d) = spine_with_repos();
    let plan = build(&d, SessionOpts::default());
    assert!(plan.cmd[0] == "claude" && plan.pack_path.exists());
    let packed = std::fs::read_to_string(&plan.pack_path).unwrap();
    assert!(packed.contains("## repo-a") && packed.contains("## repo-b")); // 全部 repo 都進地圖
}

/// agent CLI 查表：config agents: 註冊誰就能開誰；terminal 層對 agent 無關。
#[test]
fn test_build_agent_registry() {
    let (_t, d) = spine_with_repos();
    std::fs::write(
        d.join("config.yaml"),
        "agents:\n  codex: {cmd: [codex], mcp: none, prompt_flag: []}\n  fake: {cmd: [fake-agent, --chat], mcp: none, prompt_flag: [-i]}\n",
    )
    .unwrap();
    let g1 = |agent: &'static str, task: Option<&'static str>| SessionOpts {
        group: Some("g1"),
        agent: Some(agent),
        task,
        ..Default::default()
    };
    let plan = build(&d, g1("codex", None));
    assert!(plan.cmd[0] == "codex" && !plan.cmd.iter().any(|c| c == "--mcp-config")); // mcp: none → 不掛
    let plan = build(&d, g1("fake", Some("x")));
    assert_eq!(&plan.cmd[..3], &["fake-agent", "--chat", "-i"]);
    assert!(plan.cmd.last().unwrap().contains("文件地圖"));
    // claude 仍在（DEFAULTS merge），且未知 agent 要講人話
    assert_eq!(build(&d, g1("claude", None)).cmd[0], "claude");
    let err = session::build_with(&d, g1("nope", None), &fake_ctx()).unwrap_err();
    assert!(err.to_string().contains("未知 agent"));
}

/// 2026-09-08 組為單位輪：會話料＝組情境卡＋文件地圖，prompt 先讀情境卡，開會話留 presented。
#[test]
fn test_build_includes_context_card_and_leaves_trace() {
    let (_t, d) = spine_with_repos();
    registry::relate(&d, "repo-a", "repo-b", "pm-of", None, NONE).unwrap();
    let plan = build(
        &d,
        SessionOpts {
            group: Some("g1"),
            ..Default::default()
        },
    );
    let text = std::fs::read_to_string(&plan.pack_path).unwrap();
    assert!(
        text.starts_with("# 組情境卡 g1")
            && text.contains("## 你的角色")
            && text.contains("文件地圖")
    );
    assert!(text.contains("規劃（PM）→ repo-b"));
    let prompt = plan.cmd.last().unwrap();
    assert!(prompt.contains("情境卡") && prompt.contains(&plan.pack_path.display().to_string()));
    assert!(prompt.contains("3 行")); // 沒給任務 → 預設任務：先給現況摘要再問
    let evs = spine::query(&d, &spine::Query::of_type(spine::EventType::Presented)).unwrap();
    assert!(evs.len() == 1 && evs[0].source == "session" && evs[0].kv("group") == Some("g1"));
    // 有任務時任務進 prompt、預設任務不出現
    let plan = build(
        &d,
        SessionOpts {
            group: Some("g1"),
            task: Some("寫週報"),
            ..Default::default()
        },
    );
    let last = plan.cmd.last().unwrap();
    assert!(last.contains("寫週報") && !last.contains("3 行"));
}

/// 組名有空白（2026-10-14）：用事件標頭裡換寫過的名字（PM_Group）開會話，料放進登記原名的組資料夾。
#[test]
fn test_session_with_rewritten_group_name_uses_registered_folder() {
    let (_t, d) = spine_with_repos();
    registry::add_group(&d, "PM Group", &["repo-a"]).unwrap();
    let plan = build(
        &d,
        SessionOpts {
            group: Some("PM_Group"),
            ..Default::default()
        },
    );
    assert!(
        plan.pack_path
            .starts_with(d.join("groups").join("PM Group").join("materials")),
        "{:?}",
        plan.pack_path
    );
}

/// 會話事件（presented）的 `group:` 值；沒有就 None。
fn presented_group(d: &Path) -> Option<String> {
    spine::iter_events(d)
        .unwrap()
        .iter()
        .rev()
        .find(|ev| ev.etype == spine::EventType::Presented)
        .and_then(|ev| ev.kv("group").map(str::to_string))
}

/// 單一 repo 開會話、它所屬的組只有它（2026-10-14 實測 MyProjects）：情境卡就是那個組，不是「臨時(…)」。
#[test]
fn test_single_repo_session_in_solo_group_uses_group() {
    let (_t, d) = spine_with_repos();
    registry::add_group(&d, "My Solo", &["repo-b"]).unwrap();
    let repos = vec!["repo-b".to_string()];
    let plan = build(
        &d,
        SessionOpts {
            repos: Some(&repos),
            repo: Some("repo-b"),
            home_group: Some("My Solo"),
            ..Default::default()
        },
    );
    assert!(plan
        .pack_path
        .starts_with(d.join("groups").join("My Solo").join("materials")));
    let text = std::fs::read_to_string(&plan.pack_path).unwrap();
    // 範圍只有一個 repo：講 repo 不講組（#73），但說明它屬於哪個牌組
    assert!(text.starts_with("# 情境卡 repo-b") && !text.contains("臨時("));
    assert!(text.contains("屬於牌組「My Solo」") && text.contains("## 這個 repo"));
    assert!(text.contains("**repo 管家**") && !text.contains("組管家"));
    assert!(text.contains("## 未結（open loops，本 repo）"));
    let prompt = plan.cmd.last().unwrap();
    assert!(prompt.starts_with("情境卡＋文件地圖已備好") && prompt.contains("repo 現況"));
    assert_eq!(presented_group(&d).as_deref(), Some("My_Solo")); // 標頭空白換 _
    assert_eq!(plan.cwd, registry::get_repo(&d, "repo-b").unwrap().path());
}

/// 組裡還有別的 repo：卡只講這個 repo，但料夾與事件掛那個組（按組回查找得到）。
#[test]
fn test_single_repo_session_in_shared_group_files_under_group() {
    let (_t, d) = spine_with_repos();
    let repos = vec!["repo-a".to_string()];
    let plan = build(
        &d,
        SessionOpts {
            repos: Some(&repos),
            repo: Some("repo-a"),
            home_group: Some("g1"),
            ..Default::default()
        },
    );
    assert!(plan
        .pack_path
        .starts_with(d.join("groups").join("g1").join("materials")));
    let text = std::fs::read_to_string(&plan.pack_path).unwrap();
    assert!(text.starts_with("# 情境卡 repo-a") && !text.contains("## repo-b"));
    assert!(
        !text.contains("屬於牌組"),
        "組裡還有別人：卡只講這個 repo，不說它就是那組"
    );
    assert_eq!(presented_group(&d).as_deref(), Some("g1"));
}

/// home_group 只是提示：組不存在、repo 不在組裡，就照原本的臨時範圍。
#[test]
fn test_single_repo_session_ignores_bad_home_hint() {
    let (_t, d) = spine_with_repos();
    registry::add_group(&d, "only-b", &["repo-b"]).unwrap();
    let repos = vec!["repo-a".to_string()];
    for hint in ["no-such-group", "only-b"] {
        let plan = build(
            &d,
            SessionOpts {
                repos: Some(&repos),
                repo: Some("repo-a"),
                home_group: Some(hint),
                ..Default::default()
            },
        );
        assert!(plan
            .pack_path
            .starts_with(d.join("groups").join("adhoc").join("materials")));
        assert_eq!(presented_group(&d), None);
    }
}

/// 範圍是多個 repo 的組：照舊是組情境卡與組管家。
#[test]
fn test_group_session_keeps_group_wording() {
    let (_t, d) = spine_with_repos();
    let plan = build(
        &d,
        SessionOpts {
            group: Some("g1"),
            ..Default::default()
        },
    );
    let text = std::fs::read_to_string(&plan.pack_path).unwrap();
    assert!(text.starts_with("# 組情境卡 g1") && text.contains("組管家"));
    assert!(plan
        .cmd
        .last()
        .unwrap()
        .starts_with("組情境卡＋文件地圖已備好"));
}
