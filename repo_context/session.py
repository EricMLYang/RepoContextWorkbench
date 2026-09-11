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
from . import context as _context
from . import pack as _pack
from . import registry, spine

DEFAULT_TASK = "先讀情境卡，用 3 行告訴我這組現況（活動、異常、未結），再問我要做什麼"


def ensure_mcp_json(spine_dir):
    """寫 spine repo 的 .mcp.json（已存在且含 repo_context 就不動）。回傳路徑。"""
    p = Path(spine_dir) / ".mcp.json"
    if p.exists():
        try:
            if "repo_context" in json.loads(p.read_text(encoding="utf-8")).get(
                    "mcpServers", {}):
                return p
        except (json.JSONDecodeError, OSError):
            pass
    data = {"mcpServers": {"repo_context": {
        "command": sys.executable,
        "args": ["-m", "repo_context", "--spine", str(Path(spine_dir).resolve()), "mcp"],
        "env": {"PYTHONUTF8": "1"},
    }}}
    p.write_text(json.dumps(data, ensure_ascii=False, indent=1), encoding="utf-8")
    return p


def build(spine_dir, group=None, repos=None, repo=None, task=None,
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
    gname, entries = registry.resolve_group(spine_dir, group, repos)
    now = _dt.datetime.now()
    # 會話料＝組情境卡（成員角色／脈動／未結／最近事件／組管家角色）＋文件地圖
    # （L4 回饋「料太長」：地圖不是全文，agent 自己會讀檔，先挑再細讀）
    text = _context.build_context(spine_dir, group, repos, when=now) + "\n" \
        + _pack.pack_index(entries)
    out = spine_dir / "groups" / (group or "adhoc") / "materials" / \
        f"session-{now:%Y%m%d-%H%M%S}.md"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(text, encoding="utf-8")
    # 教練規格：開會話也是出手，留 presented（自己的留痕，不進收件匣）
    spine.append_event(
        spine_dir, "presented", "session",
        ([f"group:{group}"] if group else []) + ([f"repo:{repo}"] if repo else []),
        body=f"組情境卡＋文件地圖：{gname}（{len(entries)} repo）→ {out.name}"
             + (f"；任務：{task.splitlines()[0][:60]}" if task else ""),
        when=now)
    mcp_json = ensure_mcp_json(spine_dir)
    cwd = Path(registry.get_repo(spine_dir, repo)["path"]).expanduser() \
        if repo else spine_dir
    prompt = (f"組情境卡＋文件地圖已備好：{out}——先讀情境卡（成員角色、脈動、未結、你的角色），"
              f"需要細節再照地圖挑檔細讀，不要整包讀完。任務：{task or DEFAULT_TASK}")
    cmd = list(spec["cmd"])
    if repo and spec.get("mcp") == "mcp-config":
        # 等號形式必須：claude 的 --mcp-config 吃多值，空格形式會把後面的
        # 帶料 prompt 也吞成設定檔路徑（L4 實錘：Invalid MCP configuration）
        cmd.append(f"--mcp-config={mcp_json}")
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
