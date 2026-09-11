"""P16 會話：指令組裝純函數＋.mcp.json 產生（MCP 自動掛載 v1 只保證 Claude Code）。"""
import json
from pathlib import Path

from repo_context import registry, session


def test_build_packs_and_mounts_via_cwd(spine_with_repos):
    cwd, cmd, pack_path = session.build(spine_with_repos, group="g1", task="寫文供料")
    assert Path(cwd) == Path(spine_with_repos)     # cwd=spine → 靠 cwd .mcp.json 自動掛載
    assert cmd[0] == "claude" and "--mcp-config" not in cmd
    assert "文件地圖" in cmd[-1] and str(pack_path) in cmd[-1] and "寫文供料" in cmd[-1]
    text = pack_path.read_text(encoding="utf-8")
    # L4 回饋「料太長」：會話料是地圖不是全文——列路徑與標題，不含檔案內文
    assert "文件地圖" in text and "README.md" in text and "notes/plan.md" in text
    assert "widget 快取設計" not in text
    mcp = json.loads((Path(spine_with_repos) / ".mcp.json").read_text(encoding="utf-8"))
    args = mcp["mcpServers"]["repo_context"]["args"]
    assert args[-1] == "mcp" and "-m" in args and "--spine" in args
    assert mcp["mcpServers"]["repo_context"]["env"]["PYTHONUTF8"] == "1"


def test_build_with_repo_cwd_uses_mcp_config_flag(spine_with_repos):
    cwd, cmd, _ = session.build(spine_with_repos, repos="repo-a", repo="repo-a")
    assert Path(cwd) == Path(registry.get_repo(spine_with_repos, "repo-a")["path"])
    # cwd 不在 spine → 顯式掛載；必須等號形式——claude 的 --mcp-config 吃多值，
    # 空格形式會把帶料 prompt 吞成設定檔路徑（L4 bug 回歸）
    flag = [c for c in cmd if c.startswith("--mcp-config=")]
    assert len(flag) == 1 and flag[0].endswith(".mcp.json")
    assert "--mcp-config" not in cmd
    assert cmd.index(flag[0]) < len(cmd) - 1 and "文件地圖" in cmd[-1]


def test_ensure_mcp_json_idempotent(spine):
    p1 = session.ensure_mcp_json(spine)
    before = p1.read_text(encoding="utf-8")
    assert session.ensure_mcp_json(spine).read_text(encoding="utf-8") == before


def test_build_all_scope(spine_with_repos):
    """L4 bug 回歸：不給 group/repos（工作台的（全部）範圍）也要組得出會話。"""
    cwd, cmd, pack_path = session.build(spine_with_repos)
    assert cmd[0] == "claude" and pack_path.exists()
    packed = pack_path.read_text(encoding="utf-8")
    assert "## repo-a" in packed and "## repo-b" in packed  # 全部 repo 都進地圖


def test_build_agent_registry(spine_with_repos):
    """agent CLI 查表：config agents: 註冊誰就能開誰；terminal 層對 agent 無關。"""
    import pytest
    (Path(spine_with_repos) / "config.yaml").write_text(
        "agents:\n"
        "  codex: {cmd: [codex], mcp: none, prompt_flag: []}\n"
        "  fake: {cmd: [fake-agent, --chat], mcp: none, prompt_flag: [-i]}\n",
        encoding="utf-8")
    _, cmd, _ = session.build(spine_with_repos, group="g1", agent="codex")
    assert cmd[0] == "codex" and "--mcp-config" not in cmd  # mcp: none → 不掛
    _, cmd, _ = session.build(spine_with_repos, group="g1", agent="fake", task="x")
    assert cmd[:3] == ["fake-agent", "--chat", "-i"] and "文件地圖" in cmd[-1]
    # claude 仍在（DEFAULTS merge），且未知 agent 要講人話
    _, cmd, _ = session.build(spine_with_repos, group="g1", agent="claude")
    assert cmd[0] == "claude"
    with pytest.raises(ValueError, match="未知 agent"):
        session.build(spine_with_repos, group="g1", agent="nope")


def test_build_includes_context_card_and_leaves_trace(spine_with_repos):
    """2026-09-08 組為單位輪：會話料＝組情境卡＋文件地圖，prompt 先讀情境卡，開會話留 presented。"""
    from repo_context import spine as spine_mod
    registry.relate(spine_with_repos, "repo-a", "repo-b", "pm-of")
    cwd, cmd, pack_path = session.build(spine_with_repos, group="g1")
    text = pack_path.read_text(encoding="utf-8")
    assert text.startswith("# 組情境卡 g1") and "## 你的角色" in text and "文件地圖" in text
    assert "規劃（PM）→ repo-b" in text
    prompt = cmd[-1]
    assert "情境卡" in prompt and str(pack_path) in prompt
    assert "3 行" in prompt                      # 沒給任務 → 預設任務：先給現況摘要再問
    evs = spine_mod.query(spine_with_repos, type="presented")
    assert len(evs) == 1 and evs[0].source == "session" and evs[0].kv("group") == "g1"
    # 有任務時任務進 prompt、預設任務不出現
    _, cmd, _ = session.build(spine_with_repos, group="g1", task="寫週報")
    assert "寫週報" in cmd[-1] and "3 行" not in cmd[-1]
