"""P16 agent 會話——帶料開原生終端視窗；MCP 自動掛載 v1 只保證 Claude Code（v2 §2 P16）。

流程：P6 打包選料 → 確保 spine repo 有 .mcp.json（掛本引擎的 MCP server）→
組出 claude 啟動指令（cwd=spine 時靠 cwd .mcp.json 自動掛載；cwd=某 repo 時用 --mcp-config）。
開視窗是 best-effort（Windows start / macOS Terminal / 其他印指令）；指令組裝是純函數可測。
"""
import json
import subprocess
import sys
from pathlib import Path

from . import config as _config
from . import pack as _pack
from . import registry


def ensure_mcp_json(spine_dir):
    """寫 spine repo 的 .mcp.json（已存在且含 repoengine 就不動）。回傳路徑。"""
    p = Path(spine_dir) / ".mcp.json"
    if p.exists():
        try:
            if "repoengine" in json.loads(p.read_text(encoding="utf-8")).get(
                    "mcpServers", {}):
                return p
        except (json.JSONDecodeError, OSError):
            pass
    data = {"mcpServers": {"repoengine": {
        "command": sys.executable,
        "args": ["-m", "repoengine", "--spine", str(Path(spine_dir).resolve()), "mcp"],
        "env": {"PYTHONUTF8": "1"},
    }}}
    p.write_text(json.dumps(data, ensure_ascii=False, indent=1), encoding="utf-8")
    return p


def build(spine_dir, group=None, repos=None, repo=None, task=None, budget=None,
          agent="claude"):
    """組會話：打包＋掛載＋指令。回傳 (cwd, cmd list, pack 檔路徑)。

    agent＝config `agents:` 註冊表的 key（預設 claude）——terminal 對 agent 無關，
    spawn 哪家 CLI 只是查表；MCP 掛載是各家不同的膠水，由表上 `mcp` 欄決定
    （v1 只保證 Claude Code 的自動掛載，其他家 MCP 設定自理）。"""
    import datetime as _dt
    spine_dir = Path(spine_dir)
    cfg = _config.load(spine_dir)
    spec = cfg["agents"].get(agent)
    if spec is None:
        raise ValueError(f"未知 agent: {agent}（config agents: 目前有 "
                         f"{', '.join(cfg['agents'])}）")
    _, entries = registry.resolve_group(spine_dir, group, repos)
    text, _, _ = _pack.pack_group(entries, budget or cfg["pack"]["token_budget"])
    out = spine_dir / "groups" / (group or "adhoc") / "materials" / \
        f"session-{_dt.datetime.now():%Y%m%d-%H%M%S}.md"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(text, encoding="utf-8")
    mcp_json = ensure_mcp_json(spine_dir)
    cwd = Path(registry.get_repo(spine_dir, repo)["path"]).expanduser() \
        if repo else spine_dir
    prompt = f"料已備好：{out}。任務：{task or '（未指定——先讀料，再問我要做什麼）'}"
    cmd = list(spec["cmd"])
    if repo and spec.get("mcp") == "mcp-config":
        cmd += ["--mcp-config", str(mcp_json)]  # cwd 不在 spine → 顯式掛載
    cmd += list(spec.get("prompt_flag") or []) + [prompt]
    return cwd, cmd, out


def open_terminal(cwd, cmd):
    """開原生終端視窗跑 cmd（best-effort；打不開就把指令印出來讓人自己跑）。"""
    try:
        if sys.platform == "win32":
            subprocess.Popen(["cmd", "/c", "start", "cmd", "/k",
                              subprocess.list2cmdline(cmd)], cwd=str(cwd))
        elif sys.platform == "darwin":
            quoted = " ".join(f"'{c}'" for c in cmd).replace("\\", "\\\\").replace('"', '\\"')
            script = f'tell app "Terminal" to do script "cd \'{cwd}\' && {quoted}"'
            subprocess.Popen(["osascript", "-e", script])
        else:
            raise OSError("無已知終端")
        return True
    except OSError:
        print(f"開不了終端視窗——自己跑：\ncd {cwd}\n{subprocess.list2cmdline(cmd)}")
        return False
