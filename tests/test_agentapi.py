"""Agent 友善輪（2026-09-25）：agent 是主要使用者。

釘住五件事：① 從任何 repo 的 cwd 都推得出主場；② 工作層動作回結構化結果、帶可引用 id；
③ 範圍是機制（跨組寫入被拒，使用者同意後才放行）；④ 交接會變成下一次的起點；
⑤ 知識 ↔ repo 相關度（中文 bigram、洩密檔不進索引）。
"""
import datetime as dt
import json

import pytest

from repo_context import agentapi as api
from repo_context import hooks, knowledge, mcpserver, registry, spine as spine_mod, summary
from tests.conftest import _run_git, make_git_repo


@pytest.fixture
def two_groups(spine, tmp_path):
    """兩組：產品（plan＋impl）、研究（notes）。impl 底下有子目錄。"""
    plan = make_git_repo(tmp_path, "plan")
    impl = make_git_repo(tmp_path, "impl")
    (impl / "src" / "deep").mkdir(parents=True)
    notes = make_git_repo(tmp_path, "notes")
    for rid, p in (("plan", plan), ("impl", impl), ("notes", notes)):
        registry.add_repo(spine, rid, p)
    registry.add_group(spine, "產品", ["plan", "impl"])
    registry.add_group(spine, "研究", ["notes"])
    return spine, {"plan": plan, "impl": impl, "notes": notes}


# ---- ① 定位 ----

def test_where_am_i_from_subdirectory(two_groups):
    sp, repos = two_groups
    w = api.where_am_i(sp, repos["impl"] / "src" / "deep")
    assert w["registered"] and w["repo"]["id"] == "impl"
    assert w["groups"] == ["產品"] and w["group"] == "產品"


def test_where_am_i_unregistered_and_spine(two_groups, tmp_path):
    sp, _ = two_groups
    stray = tmp_path / "stray"
    stray.mkdir()
    w = api.where_am_i(sp, stray)
    assert not w["registered"] and "registry add" in w["hint"]
    w = api.where_am_i(sp, sp)
    assert w["in_spine"] and w["repo"] is None and "全部" in w["note"]


# ---- ② 工作層動作 ----

def test_add_todo_numbers_and_tokens_follow_cwd(two_groups):
    sp, repos = two_groups
    a = api.add_todo(sp, "驗證驗收條件", cwd=repos["impl"])
    b = api.add_todo(sp, "補壓測", cwd=repos["impl"], due="2026-12-01")
    assert (a["id"], b["id"]) == ("#1", "#2")          # 編號自動配，agent 不用猜
    assert a["todo"]["group"] == "產品" and a["todo"]["repo"] == "impl"
    assert a["todo"]["due"]                             # 預設到期自動補
    assert b["todo"]["due"] == "2026-12-01"


def test_log_decision_returns_resolvable_ref(two_groups):
    sp, repos = two_groups
    r = api.log_decision(sp, "## 結論：用 LRU\n依據 plan/README.md", cwd=repos["impl"])
    assert r["ok"] and r["ref"].startswith("decision:") and r["ref"].endswith("-d1")
    # 內文的 markdown 標題被降級，不會被 validator 拒寫
    assert r["event"]["text"].startswith("### 結論")
    r2 = api.log_decision(sp, "延伸判斷", cwd=repos["impl"], ref=r["ref"])
    assert r2["ref"].endswith("-d2")
    assert spine_mod.lint(sp) == []                     # ref 解得回去


def test_queries_default_to_home_scope(two_groups):
    sp, repos = two_groups
    api.add_todo(sp, "產品的事", cwd=repos["impl"])
    api.add_todo(sp, "研究的事", cwd=repos["notes"])
    ctx = api.context_for(sp, cwd=repos["plan"])
    texts = [lp["text"] for lp in ctx["open_loops"]]
    assert texts == ["產品的事"]                         # 別組的未結不混進來
    assert ctx["scope"]["label"] == "產品" and "card" in ctx
    nxt = api.next_work(sp, cwd=repos["notes"])
    assert [i["text"] for i in nxt["items"] if i["kind"] == "todo"] == ["研究的事"]


# ---- ③ 範圍是機制 ----

def test_cross_scope_write_is_refused_until_user_confirms(two_groups):
    sp, repos = two_groups
    with pytest.raises(api.AgentError) as e:
        api.add_todo(sp, "偷偷寫到研究組", cwd=repos["impl"], group="研究")
    assert e.value.code == "cross_scope" and "先問使用者" in e.value.hint
    ok = api.add_todo(sp, "使用者同意後寫入", cwd=repos["impl"], group="研究",
                      cross_scope_ok=True)
    assert ok["todo"]["group"] == "研究"
    # 同組的另一個 repo 不算跨組
    assert api.log_decision(sp, "同組", cwd=repos["impl"], repos="plan")["ok"]


def test_close_todo_respects_scope(two_groups):
    sp, repos = two_groups
    t = api.add_todo(sp, "研究的事", cwd=repos["notes"])
    with pytest.raises(api.AgentError) as e:
        api.close_todo(sp, t["id"], cwd=repos["impl"])
    assert e.value.code == "cross_scope"
    r = api.close_todo(sp, t["id"].lstrip("#"), note="讀完了", cwd=repos["notes"])
    assert r["closed"] == "研究的事"
    assert api.context_for(sp, cwd=repos["notes"])["open_loops"] == []
    with pytest.raises(api.AgentError) as e:
        api.close_todo(sp, t["id"], cwd=repos["notes"])
    assert e.value.code == "not_found"


# ---- ④ 交接是下一次的起點 ----

def test_handoff_becomes_next_step_and_opens_remaining(two_groups):
    sp, repos = two_groups
    h = api.handoff(sp, "寫完快取層", "補整合測試", remaining=["壓測", ""],
                    cwd=repos["impl"])
    assert h["todos"] == ["#1"]                          # 空字串不開
    ctx = api.context_for(sp, cwd=repos["plan"])
    assert ctx["last_handoff"]["next_step"] == "補整合測試"
    assert ctx["last_handoff"]["summary"] == "寫完快取層"
    assert ctx["agent_status"]["status"] == "done"
    nxt = api.next_work(sp, cwd=repos["plan"])
    assert nxt["items"][0]["kind"] == "handoff" and nxt["items"][0]["text"] == "補整合測試"
    s = summary.build_summary(sp, group="產品")
    assert s["next"]["text"] == "補整合測試" and "交接" in s["next"]["source"]
    assert s["progress"]["text"] == "交接：寫完快取層"


def test_overdue_todo_outranks_handoff(two_groups):
    sp, repos = two_groups
    api.add_todo(sp, "早該做完", cwd=repos["impl"], due="2020-01-01")
    api.handoff(sp, "做了一半", "繼續", cwd=repos["impl"])
    items = api.next_work(sp, cwd=repos["impl"])["items"]
    assert [i["kind"] for i in items[:2]] == ["todo", "handoff"]
    assert "逾期" in items[0]["why"]


def test_report_status_waiting_reaches_inbox(two_groups):
    sp, repos = two_groups
    r = api.report_status(sp, "working", "改快取", cwd=repos["impl"])
    assert "event" not in r                              # working 不吵人
    r = api.report_status(sp, "waiting", "要不要換 Redis？", cwd=repos["impl"])
    assert r["event"]["type"] == "suggestion" and "等你回應" in r["event"]["text"]
    assert api.get_status(sp, {"group": "產品", "repos": ["plan", "impl"]})["status"] == "waiting"
    with pytest.raises(api.AgentError):
        api.report_status(sp, "sleeping", cwd=repos["impl"])


# ---- ⑤ 知識相關度 ----

def test_search_knowledge_ranks_repos(two_groups):
    sp, repos = two_groups
    (repos["notes"] / "cards").mkdir()
    (repos["notes"] / "cards" / "決策平台.md").write_text(
        "# 大數據自動決策平台\n決策平台需要特徵倉庫與即時推論。\n", encoding="utf-8")
    (repos["plan"] / "roadmap.md").write_text(
        "# Roadmap\n下一季做決策平台的特徵倉庫。\n", encoding="utf-8")
    r = api.search_knowledge(sp, "特徵倉庫 決策平台", cwd=repos["impl"])
    assert r["ok"] and r["scope"] == "全部"
    top_repos = [x["repo"] for x in r["repos"]]
    assert set(top_repos[:2]) == {"notes", "plan"} and "impl" not in top_repos
    f = r["files"][0]
    assert f["snippets"] and f["snippets"][0]["line"] >= 1
    here = api.search_knowledge(sp, "特徵倉庫", cwd=repos["impl"], everywhere=False)
    assert {x["repo"] for x in here["repos"]} == {"plan"}   # 主場＝產品組
    with pytest.raises(api.AgentError):
        api.search_knowledge(sp, "  ")


def test_search_skips_secret_files_and_caches(two_groups):
    sp, repos = two_groups
    (repos["plan"] / "keys.md").write_text(
        "# 部署\nAKIAABCDEFGHIJKLMNOP 部署金鑰\n", encoding="utf-8")
    r = api.search_knowledge(sp, "部署金鑰")
    assert all(f["file"] != "keys.md" for f in r["files"])
    assert (sp / ".state" / "knowledge_index.json").exists()


def test_tokenize_mixes_cjk_bigrams_and_words():
    toks = knowledge.tokenize("Agent 決策平台 for LRU")
    assert "agent" in toks and "lru" in toks and "for" not in toks
    assert {"決策", "策平", "平台"} <= set(toks)


# ---- hooks ----

def test_session_start_hook_injects_context(two_groups):
    sp, repos = two_groups
    api.add_todo(sp, "驗證驗收條件", cwd=repos["impl"])
    out = hooks.session_start(sp, {"session_id": "s1", "cwd": str(repos["impl"])})
    data = json.loads(out)["hookSpecificOutput"]
    assert data["hookEventName"] == "SessionStart"
    ctx = data["additionalContext"]
    assert "你在 impl" in ctx and "產品" in ctx and "#1 驗證驗收條件" in ctx and "handoff" in ctx
    assert (sp / ".state" / "hook_sessions" / "s1.json").exists()


def test_session_start_hook_silent_outside_registry(two_groups, tmp_path):
    sp, _ = two_groups
    assert hooks.session_start(sp, {"session_id": "x", "cwd": str(tmp_path)}) == ""


def test_session_end_records_missing_handoff(two_groups):
    sp, repos = two_groups
    start = dt.datetime.now() - dt.timedelta(minutes=5)
    hooks.session_start(sp, {"session_id": "s2", "cwd": str(repos["impl"])}, now=start)
    (repos["impl"] / "feature.md").write_text("新功能\n", encoding="utf-8")
    _run_git(repos["impl"], "add", "-A")
    _run_git(repos["impl"], "commit", "-q", "-m", "加新功能")
    ev = hooks.session_end(sp, {"session_id": "s2", "reason": "exit"})
    assert ev and ev["type"] == "suggestion" and ev["source"] == "hook"
    assert "沒有交接" in ev["body"] and "加新功能" in ev["body"]
    assert ev["group"] == "產品"
    assert not (sp / ".state" / "hook_sessions" / "s2.json").exists()


def test_session_end_quiet_after_handoff_or_nothing_happened(two_groups):
    sp, repos = two_groups
    start = dt.datetime.now() - dt.timedelta(minutes=5)
    hooks.session_start(sp, {"session_id": "a", "cwd": str(repos["plan"])}, now=start)
    assert hooks.session_end(sp, {"session_id": "a"}) is None      # 什麼都沒發生
    hooks.session_start(sp, {"session_id": "b", "cwd": str(repos["plan"])}, now=start)
    api.handoff(sp, "整理 roadmap", "排優先序", cwd=repos["plan"])
    assert hooks.session_end(sp, {"session_id": "b"}) is None      # 有交接就不吵
    assert hooks.session_end(sp, {"session_id": "never-started"}) is None


# ---- MCP 分層 ----

def _mcp(sp, method, params=None, **kw):
    return mcpserver.handle_message(
        sp, {"jsonrpc": "2.0", "id": 1, "method": method, "params": params or {}}, **kw)


def test_mcp_default_exposes_only_work_tools(two_groups):
    sp, _ = two_groups
    names = {t["name"] for t in _mcp(sp, "tools/list")["result"]["tools"]}
    assert names == set(mcpserver.WORK_TOOLS)
    assert "spawn" not in names and "unread" not in names
    full = {t["name"] for t in _mcp(sp, "tools/list", admin=True)["result"]["tools"]}
    assert set(mcpserver.ADMIN_TOOLS) <= full
    # 說明不再用內部規格代號
    for t in _mcp(sp, "tools/list", admin=True)["result"]["tools"]:
        assert not t["description"].startswith("P1"), t["name"]
    r = _mcp(sp, "tools/call", {"name": "spawn", "arguments": {}})
    assert "--admin" in r["error"]["message"]


def test_mcp_admin_via_config(two_groups):
    sp, _ = two_groups
    (sp / "config.yaml").write_text("mcp:\n  admin: true\n", encoding="utf-8")
    names = {t["name"] for t in _mcp(sp, "tools/list")["result"]["tools"]}
    assert "registry_add" in names


def test_mcp_work_tools_return_json_and_structured_errors(two_groups):
    sp, repos = two_groups
    r = _mcp(sp, "tools/call", {"name": "add_todo", "arguments": {"text": "驗證"}},
             cwd=str(repos["impl"]))
    body = json.loads(r["result"]["content"][0]["text"])
    assert body["ok"] and body["id"] == "#1" and body["todo"]["group"] == "產品"
    r = _mcp(sp, "tools/call", {"name": "add_todo",
                                "arguments": {"text": "跨組", "group": "研究"}},
             cwd=str(repos["impl"]))
    assert r["result"]["isError"] is True
    err = json.loads(r["result"]["content"][0]["text"])
    assert err["error"] == "cross_scope" and err["hint"]
    r = _mcp(sp, "tools/call", {"name": "context_for", "arguments": {"card": False}},
             cwd=str(repos["impl"]))
    ctx = json.loads(r["result"]["content"][0]["text"])
    assert ctx["where"]["repo"]["id"] == "impl" and "card" not in ctx
    schema = next(t for t in _mcp(sp, "tools/list")["result"]["tools"]
                  if t["name"] == "report_status")["inputSchema"]
    assert schema["properties"]["status"]["enum"] == list(api.STATUSES)


def test_mcp_admin_open_loops_is_scoped(two_groups):
    sp, repos = two_groups
    api.add_todo(sp, "產品的事", cwd=repos["impl"])
    api.add_todo(sp, "研究的事", cwd=repos["notes"])
    r = _mcp(sp, "tools/call", {"name": "open_loops", "arguments": {"group": "研究"}},
             admin=True)
    text = r["result"]["content"][0]["text"]
    assert "研究的事" in text and "產品的事" not in text


def test_workbench_session_shows_agent_status(two_groups):
    from repo_context import webui
    sp, repos = two_groups
    started = f"{dt.datetime.now() - dt.timedelta(minutes=1):%Y-%m-%d %H:%M}"
    term = {"sid": "t1", "kind": "agent", "scope": "產品", "started": started}
    assert webui.annotate_terms(sp, [term])[0]["agent_status"] is None
    api.report_status(sp, "blocked", "缺 API key", cwd=repos["impl"])
    rec = webui.annotate_terms(sp, [term])[0]
    assert rec["agent_status"]["status"] == "blocked"
