"""P16 會話：指令組裝純函數＋.mcp.json 產生（MCP 自動掛載 v1 只保證 Claude Code）。"""
import json
from pathlib import Path

from repoengine import registry, session


def test_build_packs_and_mounts_via_cwd(spine_with_repos):
    cwd, cmd, pack_path = session.build(spine_with_repos, group="g1", task="寫文供料")
    assert Path(cwd) == Path(spine_with_repos)     # cwd=spine → 靠 cwd .mcp.json 自動掛載
    assert cmd[0] == "claude" and "--mcp-config" not in cmd
    assert "料已備好" in cmd[-1] and str(pack_path) in cmd[-1] and "寫文供料" in cmd[-1]
    assert pack_path.exists() and "widget" in pack_path.read_text(encoding="utf-8")
    mcp = json.loads((Path(spine_with_repos) / ".mcp.json").read_text(encoding="utf-8"))
    args = mcp["mcpServers"]["repoengine"]["args"]
    assert args[-1] == "mcp" and "-m" in args and "--spine" in args
    assert mcp["mcpServers"]["repoengine"]["env"]["PYTHONUTF8"] == "1"


def test_build_with_repo_cwd_uses_mcp_config_flag(spine_with_repos):
    cwd, cmd, _ = session.build(spine_with_repos, repos="repo-a", repo="repo-a")
    assert Path(cwd) == Path(registry.get_repo(spine_with_repos, "repo-a")["path"])
    i = cmd.index("--mcp-config")                  # cwd 不在 spine → 顯式掛載
    assert cmd[i + 1].endswith(".mcp.json")


def test_ensure_mcp_json_idempotent(spine):
    p1 = session.ensure_mcp_json(spine)
    before = p1.read_text(encoding="utf-8")
    assert session.ensure_mcp_json(spine).read_text(encoding="utf-8") == before
