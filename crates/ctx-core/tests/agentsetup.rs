//! `ctx setup` 引擎層。對應 tests/test_agent_cli_e2e.py 裡 setup／instructions 相關的測試——
//! Python 是起真 CLI 行程（HOME／XDG／CLAUDE_CONFIG_DIR 指到 tmp）；Rust 在引擎層用顯式路徑
//! （[`SetupEnv`]）跑同樣的步驟，不改行程環境變數。
//!
//! CLI 層（`--dry-run` 不寫檔、exit code（註冊失敗回 1）、setup 後不帶 --spine 的 `ctx where`、
//! test_json_flag_after_subcommand_and_exit_codes、test_hook_commands_via_stdin）：
//! 已搬到 crates/ctx-cli/tests/agent_cli_e2e.rs（跑真 binary）。

use std::path::{Path, PathBuf};

use ctx_core::agentsetup::{self, CmdOutput, SetupEnv, Step};
use ctx_core::testkit::spine;
use serde_json::Value;

fn env_in(tmp: &Path, spine_dir: &Path) -> SetupEnv {
    SetupEnv {
        spine_dir: spine_dir.to_path_buf(),
        exe: PathBuf::from("/opt/ctx/bin/ctx"),
        claude_home: tmp.join("home/.claude"),
        user_config: tmp.join("home/.config/repo-context/config.json"),
    }
}

fn read_json(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// 沒有 claude 指令（Python 測試把 PATH 縮成 /usr/bin:/bin）。
fn no_claude(_: &[String]) -> std::io::Result<CmdOutput> {
    panic!("沒有 claude 時不該跑外部指令")
}

#[test]
fn test_setup_remembers_spine_and_installs_claude_bits() {
    let (t, sp) = spine();
    let env = env_in(t.path(), &sp);
    std::fs::create_dir_all(&env.claude_home).unwrap();
    std::fs::write(
        env.settings_path(),
        r#"{"model": "x", "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "echo mine"}]}]}}"#,
    )
    .unwrap();
    let steps = agentsetup::plan(true);
    assert_eq!(
        steps,
        vec![Step::SaveSpine, Step::Mcp, Step::Hooks, Step::Skill]
    );
    // 沒 claude：印手動指令不失敗
    let mcp = agentsetup::install_mcp_with(&env.exe, false, &mut no_claude);
    assert!(mcp.ok && mcp.message.contains("找不到 claude 指令"));
    for st in [Step::SaveSpine, Step::Hooks, Step::Skill] {
        assert!(st.run(&env).unwrap().ok);
    }
    let settings = read_json(&env.settings_path());
    assert_eq!(settings["model"], "x"); // 既有設定不動
    let starts = settings["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(starts.len(), 2);
    assert!(serde_json::to_string(starts)
        .unwrap()
        .contains("/opt/ctx/bin/ctx hook session-start"));
    assert!(settings["hooks"].get("SessionEnd").is_some());
    assert!(env.claude_home.join("settings.json.bak-ctx").exists());
    assert!(env
        .claude_home
        .join("skills/repo-context/SKILL.md")
        .exists());
    // 冪等
    for st in [Step::SaveSpine, Step::Hooks, Step::Skill] {
        st.run(&env).unwrap();
    }
    let settings = read_json(&env.settings_path());
    assert_eq!(
        settings["hooks"]["SessionStart"].as_array().unwrap().len(),
        2
    );
    assert_eq!(Step::Hooks.run(&env).unwrap().message, "hooks 已存在，略過");
    assert_eq!(
        Step::Skill.run(&env).unwrap().message,
        "skill 已是最新，略過"
    );
    // 記住 spine
    let cfg = read_json(&env.user_config);
    assert_eq!(
        cfg["spine"].as_str().unwrap(),
        ctx_core::fsutil::resolve(&sp).to_string_lossy()
    );
}

/// Python 版已掛過 hooks（`<python> -m repo_context hook …`）→ Rust 版不重複掛。Python 沒有對應測試。
#[test]
fn python_hooks_count_as_installed() {
    let (t, sp) = spine();
    let env = env_in(t.path(), &sp);
    std::fs::create_dir_all(&env.claude_home).unwrap();
    std::fs::write(
        env.settings_path(),
        r#"{"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "/usr/bin/python3 -m repo_context hook session-start"}]}],
                      "SessionEnd": [{"hooks": [{"type": "command", "command": "'/Applications/My App/ctx' hook session-end"}]}]}}"#,
    )
    .unwrap();
    assert_eq!(
        agentsetup::install_hooks(&env.settings_path(), &env.exe)
            .unwrap()
            .message,
        "hooks 已存在，略過"
    );
    assert!(!agentsetup::is_our_hook("echo mine"));
    assert!(!agentsetup::is_our_hook("/bin/myctx hook x"));
}

#[test]
fn test_instructions_print_skill_without_frontmatter() {
    let text = agentsetup::skill_text(false);
    assert!(text.contains("handoff") && !text.starts_with("---"));
    assert!(agentsetup::skill_text(true).starts_with("---"));
}

/// claude mcp add 的 -e 吃多值：名稱放在 -e 後面會被當成環境變數吞掉
/// （2026-09-25 真機 setup：Invalid environment variable format: repo-context）。
/// Rust 版不帶 `-e PYTHONUTF8=1`（刻意差異），仍守「名稱在任何 -e 與 `--` 之前」。
#[test]
fn test_mcp_add_name_before_variadic_env() {
    let cmd = agentsetup::mcp_add_command(Path::new("/opt/ctx/bin/ctx"));
    let name = cmd.iter().position(|a| a == agentsetup::MCP_NAME).unwrap();
    let dashdash = cmd.iter().position(|a| a == "--").unwrap();
    assert!(name < dashdash);
    if let Some(e) = cmd.iter().position(|a| a == "-e") {
        assert!(name < e && e < dashdash);
    }
    assert_eq!(&cmd[dashdash + 1..], ["/opt/ctx/bin/ctx", "mcp"]);
}

/// 註冊失敗不能算成功——使用者（或 agent）會以為掛好了（入口據 `ok` 回非 0）。
#[test]
fn test_setup_mcp_failure_is_nonzero() {
    let mut calls: Vec<Vec<String>> = Vec::new();
    let mut fake = |argv: &[String]| -> std::io::Result<CmdOutput> {
        calls.push(argv.to_vec());
        Ok(CmdOutput {
            success: false,
            stdout: String::new(),
            stderr: "boom\n".into(),
        })
    };
    let r = agentsetup::install_mcp_with(Path::new("/opt/ctx/bin/ctx"), true, &mut fake);
    assert!(!r.ok);
    assert_eq!(r.message, "註冊失敗：boom");
    assert_eq!(calls[0], ["claude", "mcp", "get", "repo-context"]);
    assert_eq!(calls[1][..3], ["claude", "mcp", "add"]);
}
