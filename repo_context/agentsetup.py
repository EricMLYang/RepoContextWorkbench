"""`ctx setup`（2026-09-25 Agent 友善輪）——讓「在任何 repo 打開 agent」就接得上。

三件事，全部冪等（已經掛過就跳過，不重複寫）：
1. 使用者設定檔記住 spine 位置（之後 ctx／MCP／hook 都不用帶 --spine）。
2. `--claude`：
   - MCP：`claude mcp add --scope user repo-context -- <python> -m repo_context mcp`
     （user scope＝每個專案都掛得上；server 以專案目錄為 cwd 推主場）
   - hooks：~/.claude/settings.json 加 SessionStart／SessionEnd（寫入前備份一份 .bak-ctx）
   - skill：~/.claude/skills/repo-context/SKILL.md（工具怎麼用、什麼時候用）
沒有 `claude` 指令時，MCP 那步改印出手動指令，不失敗。
"""
import json
import os
import shlex
import shutil
import subprocess
import sys
from pathlib import Path

from . import config as _config

MCP_NAME = "repo-context"
HOOK_MARK = "repo_context hook"


def _python():
    return sys.executable or "python3"


def _cmdline(parts):
    return subprocess.list2cmdline(parts) if os.name == "nt" else shlex.join(parts)


def hook_command(event):
    return _cmdline([_python(), "-m", "repo_context", "hook", event])


def mcp_add_command():
    return ["claude", "mcp", "add", "--scope", "user", "-e", "PYTHONUTF8=1",
            MCP_NAME, "--", _python(), "-m", "repo_context", "mcp"]


def claude_home():
    return Path(os.environ.get("CLAUDE_CONFIG_DIR") or (Path.home() / ".claude"))


def skill_text(frontmatter=True):
    text = (Path(__file__).parent / "assets" / "SKILL.md").read_text(encoding="utf-8")
    if not frontmatter and text.startswith("---"):
        text = text.split("---", 2)[2].lstrip()
    return text


# ---- 各步驟（回傳一行結果說明）----

def save_spine(spine_dir):
    data = _config.load_user_config()
    data["spine"] = str(Path(spine_dir).resolve())
    p = _config.save_user_config(data)
    return f"已寫入 {p}"


def install_hooks(settings_path=None):
    p = Path(settings_path or (claude_home() / "settings.json"))
    try:
        data = json.loads(p.read_text(encoding="utf-8")) if p.exists() else {}
    except ValueError:
        return f"{p} 不是合法 JSON，沒有動它；請手動加 hooks（見 `ctx setup --claude --dry-run`）"
    hooks = data.setdefault("hooks", {})
    added = []
    for event, arg in (("SessionStart", "session-start"), ("SessionEnd", "session-end")):
        entries = hooks.setdefault(event, [])
        if any(HOOK_MARK in (h.get("command") or "")
               for e in entries for h in e.get("hooks", [])):
            continue
        entries.append({"hooks": [{"type": "command", "command": hook_command(arg)}]})
        added.append(event)
    if not added:
        return "hooks 已存在，略過"
    p.parent.mkdir(parents=True, exist_ok=True)
    if p.exists():
        shutil.copy2(p, p.with_name(p.name + ".bak-ctx"))
    p.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return f"已加入 {', '.join(added)} → {p}"


def install_skill():
    p = claude_home() / "skills" / MCP_NAME / "SKILL.md"
    p.parent.mkdir(parents=True, exist_ok=True)
    new = skill_text()
    if p.exists() and p.read_text(encoding="utf-8") == new:
        return "skill 已是最新，略過"
    p.write_text(new, encoding="utf-8")
    return f"已寫入 {p}"


def install_mcp():
    if shutil.which("claude") is None:
        return "找不到 claude 指令；請手動執行：" + _cmdline(mcp_add_command())
    got = subprocess.run(["claude", "mcp", "get", MCP_NAME], capture_output=True, text=True)
    if got.returncode == 0:
        return "MCP 已註冊，略過（要重設先 `claude mcp remove repo-context -s user`）"
    r = subprocess.run(mcp_add_command(), capture_output=True, text=True)
    if r.returncode != 0:
        return "註冊失敗：" + (r.stderr or r.stdout).strip()[:200]
    return "已註冊（user scope）"


def plan(spine_dir, claude=False):
    steps = [{"desc": f"記住 spine 位置：{Path(spine_dir).resolve()}",
              "run": lambda: save_spine(spine_dir)}]
    if claude:
        steps += [
            {"desc": "註冊 MCP server：" + _cmdline(mcp_add_command()), "run": install_mcp},
            {"desc": f"掛 hooks（SessionStart／SessionEnd）到 {claude_home() / 'settings.json'}："
                     + hook_command("session-start"), "run": install_hooks},
            {"desc": f"安裝 skill 到 {claude_home() / 'skills' / MCP_NAME / 'SKILL.md'}",
             "run": install_skill},
        ]
    return steps
