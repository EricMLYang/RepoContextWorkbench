//! Agent 友善輪（2026-09-25）L2：真 CLI 行程——--json、exit code、hook stdin、setup。
//! 對應 tests/test_agent_cli_e2e.py（含 ctx-core tests/agentsetup.rs、tests/hooks.rs 標「等 ctx-cli」的部分）。

mod common;

use std::path::{Path, PathBuf};

use common::*;
use serde_json::{json, Value};

struct Env {
    t: tempfile::TempDir,
    sp: PathBuf,
    impl_: PathBuf,
}

impl Env {
    fn tmp(&self) -> &Path {
        self.t.path()
    }
}

fn env_spine() -> Env {
    let t = tempfile::tempdir().unwrap();
    let sp = t.path().join("spine");
    let c = ctx(t.path());
    c.ok(&["init", s(&sp)]);
    let impl_ = make_git_repo(t.path(), "impl", 0, 0, true);
    let notes = make_git_repo(t.path(), "notes", 0, 0, true);
    let cs = c.spine(&sp);
    cs.ok(&["registry", "add", "impl", s(&impl_)]);
    cs.ok(&["registry", "add", "notes", s(&notes)]);
    cs.ok(&["group", "add", "產品", "impl"]);
    cs.ok(&["group", "add", "研究", "notes"]);
    Env { t, sp, impl_ }
}

#[test]
fn test_json_flag_after_subcommand_and_exit_codes() {
    let e = env_spine();
    let c = ctx(e.tmp()).cwd(&e.impl_).env("REPOENGINE_SPINE", &e.sp);
    let r = c.run(&["where", "--json"]);
    assert!(r.code == 0 && r.json()["group"] == "產品");
    let r = c.run(&["todo", "add", "驗證", "--json"]);
    assert_eq!(r.json()["id"], "#1");
    let r = c.run(&["todo", "add", "跨組", "--group", "研究", "--json"]);
    assert!(r.code == 2 && r.json()["error"] == "cross_scope");
    let r = c.run(&["next", "--json"]);
    assert_eq!(r.json()["items"][0]["id"], "#1");
    let r = c.run(&["loops", "--group", "研究", "--json"]);
    assert_eq!(r.json(), json!([]));
    let home = e.tmp().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let r = ctx(e.tmp())
        .cwd(e.tmp())
        .home(&home)
        .run(&["where", "--json"]);
    assert!(r.code == 3 && r.json()["error"] == "no_spine");
}

#[test]
fn test_setup_remembers_spine_and_installs_claude_bits() {
    let e = env_spine();
    let home = e.tmp().join("home");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(
        home.join(".claude/settings.json"),
        json!({"model": "x", "hooks": {"SessionStart": [
            {"hooks": [{"type": "command", "command": "echo mine"}]}]}})
        .to_string(),
    )
    .unwrap();
    let c = ctx(e.tmp()).cwd(e.tmp()).home(&home);
    let r = c.run(&["setup", "--spine", s(&e.sp), "--claude", "--dry-run"]);
    assert!(r.code == 0 && r.stdout.contains("試跑"));
    assert!(!home.join(".config/repo-context/config.json").exists());
    let r = c.run(&["setup", "--spine", s(&e.sp), "--claude"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("找不到 claude 指令")); // 沒 claude：印手動指令不失敗
    let settings: Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".claude/settings.json")).unwrap())
            .unwrap();
    assert_eq!(settings["model"], "x"); // 既有設定不動
    let starts = settings["hooks"]["SessionStart"].as_array().unwrap();
    // Rust 版的 hook 指令是 `<ctx> hook session-start`（Python 版是 `<python> -m repo_context hook …`）
    assert!(
        starts.len() == 2
            && settings["hooks"]["SessionStart"]
                .to_string()
                .contains("hook session-start")
    );
    assert!(settings["hooks"].get("SessionEnd").is_some());
    assert!(home.join(".claude/settings.json.bak-ctx").exists());
    assert!(home.join(".claude/skills/repo-context/SKILL.md").exists());
    c.run(&["setup", "--spine", s(&e.sp), "--claude"]);
    let settings: Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".claude/settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        settings["hooks"]["SessionStart"].as_array().unwrap().len(),
        2
    ); // 冪等
       // 記住 spine 之後：不帶 --spine、不設環境變數也找得到
    let r = ctx(e.tmp())
        .cwd(&e.impl_)
        .home(&home)
        .run(&["where", "--json"]);
    assert!(r.code == 0 && r.json()["repo"]["id"] == "impl");
}

#[test]
fn test_hook_commands_via_stdin() {
    let e = env_spine();
    let c = ctx(e.tmp()).cwd(e.tmp()).env("REPOENGINE_SPINE", &e.sp);
    let r = c
        .clone()
        .stdin(&json!({"session_id": "abc", "cwd": s(&e.impl_)}).to_string())
        .run(&["hook", "session-start"]);
    assert_eq!(r.code, 0);
    let out = &r.json()["hookSpecificOutput"];
    assert!(out["additionalContext"]
        .as_str()
        .unwrap()
        .contains("你在 impl"));
    let r = c
        .stdin(&json!({"session_id": "abc"}).to_string())
        .run(&["hook", "session-end"]);
    assert!(r.code == 0 && r.stdout.is_empty());
    // 壞輸入、沒 spine：永遠 exit 0，不擋開 agent
    let home = e.tmp().join("h2");
    std::fs::create_dir_all(&home).unwrap();
    let r = ctx(e.tmp())
        .cwd(e.tmp())
        .home(&home)
        .stdin("not json")
        .run(&["hook", "session-start"]);
    assert!(r.code == 0 && r.stdout.is_empty());
}

#[test]
fn test_instructions_print_skill_without_frontmatter() {
    let t = tempfile::tempdir().unwrap();
    let r = ctx(t.path()).run(&["instructions"]);
    assert!(r.code == 0 && r.stdout.contains("handoff") && !r.stdout.starts_with("---"));
}

// test_mcp_add_name_before_variadic_env：引擎層測試，已在 ctx-core tests/agentsetup.rs（Rust 版指令沒有 -e）。

/// 註冊失敗不能 exit 0——使用者（或 agent）會以為掛好了。
#[test]
fn test_setup_mcp_failure_is_nonzero() {
    let e = env_spine();
    let home = e.tmp().join("home");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    let fake = e.tmp().join("bin");
    std::fs::create_dir_all(&fake).unwrap();
    std::fs::write(fake.join("claude"), "#!/bin/sh\necho boom >&2\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(fake.join("claude"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    let r = ctx(e.tmp())
        .cwd(e.tmp())
        .home(&home)
        .env("PATH", format!("{}:/usr/bin:/bin", fake.display()))
        .run(&["setup", "--spine", s(&e.sp), "--claude"]);
    assert!(r.stdout.contains("註冊失敗") && r.code == 1, "{}", r.stdout);
}

/// 不是 spine 的 --spine：--json 回 no_spine＋exit 3。
#[test]
fn test_setup_rejects_non_spine() {
    let t = tempfile::tempdir().unwrap();
    let r = ctx(t.path()).run(&["setup", "--spine", s(t.path()), "--json"]);
    assert!(r.code == 3 && r.json()["error"] == "no_spine");
}

// Rust 才有（N-26）：todo review 預設精簡（進行中只給編號／文字／理由），--all 才全給
#[test]
fn todo_review_trims_by_default() {
    let e = env_spine();
    let c = ctx(e.tmp()).cwd(&e.impl_).env("REPOENGINE_SPINE", &e.sp);
    for t in ["甲", "乙"] {
        assert_eq!(c.clone().run(&["todo", "add", t, "--json"]).code, 0);
    }
    let r = c.clone().run(&["todo", "review", "--json"]);
    let v = r.json();
    assert!(v["items"].as_array().unwrap().is_empty());
    let active = v["active"].as_array().unwrap();
    assert_eq!(active.len(), 2);
    assert!(active[0].get("evidence").is_none());
    let all = c.run(&["todo", "review", "--all", "--json"]).json();
    assert_eq!(all["items"].as_array().unwrap().len(), 2);
    assert!(all.get("active").is_none());
}

/// Rust 才有（2026-10-14）：App 開的 Claude Code 會話，Stop／Notification／UserPromptSubmit hook 照環境變數給的
/// 路徑寫會話狀態；不是 App 開的（沒有環境變數）什麼都不做；永遠 exit 0、不印東西（不干擾 agent）。
#[test]
fn test_session_status_hooks_write_only_with_app_env() {
    let t = tempfile::tempdir().unwrap();
    let target = t
        .path()
        .join(".state")
        .join("session_status")
        .join("t2.json");
    let plain = || ctx(t.path()).cwd(t.path());
    let app = || plain().env("REPO_CONTEXT_SESSION_STATUS", &target);
    let r = plain().stdin("{}").run(&["hook", "stop"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, ""));
    assert!(!target.exists());
    let r = app()
        .stdin(r#"{"hook_event_name":"Notification","message":"Claude needs your permission to use Bash"}"#)
        .run(&["hook", "notification"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, ""));
    let read =
        || -> Value { serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap() };
    assert_eq!(read()["status"], "waiting");
    assert_eq!(read()["text"], "Claude needs your permission to use Bash");
    let r = app().stdin("").run(&["hook", "prompt"]);
    assert_eq!(r.code, 0);
    assert_eq!(read()["status"], "working");
}
