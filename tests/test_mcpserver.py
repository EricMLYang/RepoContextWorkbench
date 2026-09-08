"""MCP server：JSON-RPC 分派、tools 清單、拒寫去向（validator 錯誤原樣回 agent）、原語往返。"""
import datetime as dt

from repoengine import mcpserver
from repoengine import spine as spine_mod


def _call(spine_dir, method, params=None, mid=1):
    return mcpserver.handle_message(
        spine_dir, {"jsonrpc": "2.0", "id": mid, "method": method,
                    "params": params or {}})


def _tool(spine_dir, name, args=None):
    return _call(spine_dir, "tools/call", {"name": name, "arguments": args or {}})


def test_initialize_and_tools_list(spine):
    r = _call(spine, "initialize")
    assert r["result"]["serverInfo"]["name"] == "repoengine"
    r = _call(spine, "tools/list")
    names = {t["name"] for t in r["result"]["tools"]}
    # 原語全覆蓋：P1–P13、P15/P16 相關工具都在
    assert {"registry_add", "registry_audit", "registry_scan", "group_add",
            "collect", "pack", "pack_estimate", "brief", "spine_append",
            "spine_query", "spine_stats", "spine_lint", "open_loops",
            "unread", "collide_submit", "route", "upstream_check", "digest_run",
            "spawn"} <= names
    assert all(t["inputSchema"]["type"] == "object" for t in r["result"]["tools"])


def test_scan_lint_estimate_tools(spine_with_repos, tmp_path):
    """拆機報告落地輪三工具：掃描（mani 課）、lint（second-brain 課）、
    預算函數（Repomix 課）——agent 的介面＝人的介面。"""
    from tests.conftest import make_git_repo
    make_git_repo(tmp_path / "zone", "found-me")
    t = _tool(spine_with_repos, "registry_scan",
              {"dir": str(tmp_path / "zone")})["result"]["content"][0]["text"]
    assert "found-me" in t and "候選" in t          # 預設只列不登記
    t = _tool(spine_with_repos, "registry_scan",
              {"dir": str(tmp_path / "zone"), "apply": True})
    assert "已登記" in t["result"]["content"][0]["text"]
    t = _tool(spine_with_repos, "spine_lint")["result"]["content"][0]["text"]
    assert "零紅字" in t
    t = _tool(spine_with_repos, "pack_estimate",
              {"group": "g1"})["result"]["content"][0]["text"]
    assert "repo-a" in t and "tokens" in t and "合計" in t


def test_notifications_ignored_and_unknowns(spine):
    assert mcpserver.handle_message(
        spine, {"jsonrpc": "2.0", "method": "notifications/initialized"}) is None
    assert _call(spine, "no/such")["error"]["code"] == -32601
    assert _tool(spine, "no_such_tool")["error"]["code"] == -32602


def test_validator_error_passthrough_no_dead_letter(spine):
    r = _tool(spine, "spine_append", {"type": "nope", "source": "agent"})
    res = r["result"]
    assert res["isError"] is True and "未知事件型別" in res["content"][0]["text"]
    # MCP 是程式入口：不落 dead-letter（那是人的入口專屬）
    assert not (spine / "spine" / "dead-letter").exists()


def test_append_query_roundtrip_and_default_due(spine):
    r = _tool(spine, "spine_append",
              {"type": "decision", "source": "agent", "body": "決定 X"})
    assert r["result"]["isError"] is False
    r = _tool(spine, "spine_append",
              {"type": "open-loop", "source": "agent", "kv": ["#1"],
               "body": "opened → 驗證 Y"})
    assert "due:" in r["result"]["content"][0]["text"]  # §3.4 預設到期自動補
    q = _tool(spine, "spine_query", {"today": True})["result"]["content"][0]["text"]
    assert "decision" in q and "open-loop" in q
    loops = _tool(spine, "open_loops")["result"]["content"][0]["text"]
    assert "#1" in loops


def test_collide_submit_wait_writes_two_events(spine_with_repos):
    r = _tool(spine_with_repos, "collide_submit",
              {"idea": "已知的快取想法", "group": "g1", "wait": True})
    text = r["result"]["content"][0]["text"]
    assert r["result"]["isError"] is False and "判定" in text
    cols = [e for e in spine_mod.iter_events(spine_with_repos)
            if e.type == "collision"]
    assert len(cols) == 2  # opened ＋ 判定結果
    today = f"{dt.date.today():%Y-%m-%d}"
    assert any(e.kv("id") == f"{today}-a" for e in cols)


def test_stats_includes_survival(spine):
    text = _tool(spine, "spine_stats")["result"]["content"][0]["text"]
    assert "靈感命中率" in text and "存活率" in text


def test_relation_and_context_tools(spine_with_repos):
    """2026-09-08 組為單位輪：agent 也能設關係、讀關係、隨時重抓組情境卡。"""
    names = {t["name"] for t in _call(spine_with_repos, "tools/list")["result"]["tools"]}
    assert {"registry_relate", "registry_relations", "group_context"} <= names
    t = _tool(spine_with_repos, "registry_relate",
              {"a": "repo-a", "b": "repo-b", "kind": "pm-of"})["result"]["content"][0]["text"]
    assert "pm-of" in t
    t = _tool(spine_with_repos, "registry_relations",
              {"id": "repo-a"})["result"]["content"][0]["text"]
    assert "repo-b" in t and "規劃（PM）" in t
    r = _tool(spine_with_repos, "registry_relate",
              {"a": "repo-a", "b": "repo-a", "kind": "pm-of"})
    assert r["result"].get("isError") and "自己" in r["result"]["content"][0]["text"]
    t = _tool(spine_with_repos, "group_context", {"group": "g1"})["result"]["content"][0]["text"]
    assert t.startswith("# 組情境卡 g1") and "## 脈動" in t
