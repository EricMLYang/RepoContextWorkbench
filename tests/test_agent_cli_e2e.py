"""Agent 友善輪（2026-09-25）L2：真 CLI 行程——--json、exit code、hook stdin、setup。"""
import json
import os
import subprocess
import sys

import pytest

from tests.conftest import ROOT, make_git_repo, run_cli

pytestmark = pytest.mark.e2e


@pytest.fixture
def env_spine(tmp_path):
    sp = tmp_path / "spine"
    run_cli("init", str(sp), check=True)
    impl = make_git_repo(tmp_path, "impl")
    notes = make_git_repo(tmp_path, "notes")
    run_cli("registry", "add", "impl", str(impl), spine_dir=sp, check=True)
    run_cli("registry", "add", "notes", str(notes), spine_dir=sp, check=True)
    run_cli("group", "add", "產品", "impl", spine_dir=sp, check=True)
    run_cli("group", "add", "研究", "notes", spine_dir=sp, check=True)
    return sp, impl, notes


def _ctx(*args, cwd, env=None, stdin=None, home=None):
    e = dict(os.environ, PYTHONIOENCODING="utf-8")
    e.pop("PYTHONUTF8", None)            # 不設 PYTHONUTF8 也要能跑（UTF-8 由程式自己處理）
    e["PYTHONPATH"] = str(ROOT)
    if home:
        e.update(HOME=str(home), XDG_CONFIG_HOME=str(home / ".config"),
                 CLAUDE_CONFIG_DIR=str(home / ".claude"), PATH="/usr/bin:/bin")
    e.update(env or {})
    return subprocess.run([sys.executable, "-m", "repo_context", *args], cwd=cwd,
                          capture_output=True, text=True, encoding="utf-8",
                          input=stdin, env=e, timeout=60)


def test_json_flag_after_subcommand_and_exit_codes(env_spine, tmp_path):
    sp, impl, _ = env_spine
    env = {"REPOENGINE_SPINE": str(sp)}
    r = _ctx("where", "--json", cwd=impl, env=env)
    assert r.returncode == 0 and json.loads(r.stdout)["group"] == "產品"
    r = _ctx("todo", "add", "驗證", "--json", cwd=impl, env=env)
    assert json.loads(r.stdout)["id"] == "#1"
    r = _ctx("todo", "add", "跨組", "--group", "研究", "--json", cwd=impl, env=env)
    assert r.returncode == 2 and json.loads(r.stdout)["error"] == "cross_scope"
    r = _ctx("next", "--json", cwd=impl, env=env)
    assert json.loads(r.stdout)["items"][0]["id"] == "#1"
    r = _ctx("loops", "--group", "研究", "--json", cwd=impl, env=env)
    assert json.loads(r.stdout) == []
    home = tmp_path / "home"
    home.mkdir()
    r = _ctx("where", "--json", cwd=tmp_path, home=home)
    assert r.returncode == 3 and json.loads(r.stdout)["error"] == "no_spine"


def test_setup_remembers_spine_and_installs_claude_bits(env_spine, tmp_path):
    sp, impl, _ = env_spine
    home = tmp_path / "home"
    (home / ".claude").mkdir(parents=True)
    (home / ".claude" / "settings.json").write_text(
        json.dumps({"model": "x", "hooks": {"SessionStart": [
            {"hooks": [{"type": "command", "command": "echo mine"}]}]}}), encoding="utf-8")
    r = _ctx("setup", "--spine", str(sp), "--claude", "--dry-run", cwd=tmp_path, home=home)
    assert r.returncode == 0 and "試跑" in r.stdout
    assert not (home / ".config" / "repo-context" / "config.json").exists()
    r = _ctx("setup", "--spine", str(sp), "--claude", cwd=tmp_path, home=home)
    assert r.returncode == 0, r.stderr
    assert "找不到 claude 指令" in r.stdout                  # 沒 claude：印手動指令不失敗
    settings = json.loads((home / ".claude" / "settings.json").read_text(encoding="utf-8"))
    assert settings["model"] == "x"                          # 既有設定不動
    starts = settings["hooks"]["SessionStart"]
    assert len(starts) == 2 and "repo_context hook session-start" in json.dumps(starts)
    assert "SessionEnd" in settings["hooks"]
    assert (home / ".claude" / "settings.json.bak-ctx").exists()
    assert (home / ".claude" / "skills" / "repo-context" / "SKILL.md").exists()
    r = _ctx("setup", "--spine", str(sp), "--claude", cwd=tmp_path, home=home)
    settings = json.loads((home / ".claude" / "settings.json").read_text(encoding="utf-8"))
    assert len(settings["hooks"]["SessionStart"]) == 2       # 冪等
    # 記住 spine 之後：不帶 --spine、不設環境變數也找得到
    r = _ctx("where", "--json", cwd=impl, home=home)
    assert r.returncode == 0 and json.loads(r.stdout)["repo"]["id"] == "impl"


def test_hook_commands_via_stdin(env_spine, tmp_path):
    sp, impl, _ = env_spine
    env = {"REPOENGINE_SPINE": str(sp)}
    r = _ctx("hook", "session-start", cwd=tmp_path, env=env,
             stdin=json.dumps({"session_id": "abc", "cwd": str(impl)}))
    assert r.returncode == 0
    out = json.loads(r.stdout)["hookSpecificOutput"]
    assert "你在 impl" in out["additionalContext"]
    r = _ctx("hook", "session-end", cwd=tmp_path, env=env,
             stdin=json.dumps({"session_id": "abc"}))
    assert r.returncode == 0 and r.stdout == ""
    # 壞輸入、沒 spine：永遠 exit 0，不擋開 agent
    home = tmp_path / "h2"
    home.mkdir()
    r = _ctx("hook", "session-start", cwd=tmp_path, stdin="not json", home=home)
    assert r.returncode == 0 and r.stdout == ""


def test_instructions_print_skill_without_frontmatter(tmp_path):
    r = _ctx("instructions", cwd=tmp_path)
    assert r.returncode == 0 and "handoff" in r.stdout and not r.stdout.startswith("---")


def test_mcp_add_name_before_variadic_env(monkeypatch):
    """claude mcp add 的 -e 吃多值：名稱放在 -e 後面會被當成環境變數吞掉
    （2026-09-25 真機 setup：Invalid environment variable format: repo-context）。"""
    from repo_context import agentsetup
    cmd = agentsetup.mcp_add_command()
    assert cmd.index(agentsetup.MCP_NAME) < cmd.index("-e") < cmd.index("--")


def test_setup_mcp_failure_is_nonzero(env_spine, tmp_path):
    """註冊失敗不能 exit 0——使用者（或 agent）會以為掛好了。"""
    sp, _, _ = env_spine
    home = tmp_path / "home"
    (home / ".claude").mkdir(parents=True)
    fake = tmp_path / "bin"
    fake.mkdir()
    (fake / "claude").write_text("#!/bin/sh\necho boom >&2\nexit 1\n")
    (fake / "claude").chmod(0o755)
    import os
    r = _ctx("setup", "--spine", str(sp), "--claude", cwd=tmp_path, home=home,
             env={"PATH": f"{fake}{os.pathsep}{os.environ['PATH']}"})
    assert "註冊失敗" in r.stdout and r.returncode == 1
