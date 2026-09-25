"""組的維護（2026-09-25 使用者回報：「分群找不到如何新增或刪除 repo」）。

以前組只能建、不能改：加減組員、改名、刪組全都沒有入口。四個入口（registry／CLI／
工作台／MCP）走同一組 registry 函式；registry 以外的入口每次改動都落一筆 decision。
"""
import pytest

from repo_context import mcpserver, registry, webui
from repo_context import spine as spine_mod
from tests.conftest import make_git_repo, run_cli


@pytest.fixture
def three(spine_with_repos, tmp_path):
    """g1＝repo-a,repo-b；另登記 repo-c（不在任何組）。"""
    registry.add_repo(spine_with_repos, "repo-c", make_git_repo(tmp_path, "repo-c"))
    return spine_with_repos


def _members(d, name):
    return registry.get_group(d, name)["members"]


def _decisions(d):
    return [e for e in spine_mod.iter_events(d) if e.type == "decision"]


# ── registry ────────────────────────────────────────────────────────────

def test_add_members_idempotent_and_validated(three):
    g = registry.add_group_members(three, "g1", ["repo-c", "repo-a"])
    assert g["members"] == ["repo-a", "repo-b", "repo-c"]  # 已在組裡的不重複
    with pytest.raises(ValueError, match="未登記"):
        registry.add_group_members(three, "g1", ["ghost"])
    with pytest.raises(ValueError, match="組不存在"):
        registry.add_group_members(three, "nope", ["repo-a"])
    assert _members(three, "g1") == ["repo-a", "repo-b", "repo-c"]  # 失敗不寫半套


def test_remove_members_keeps_registry_and_allows_empty(three):
    g = registry.remove_group_members(three, "g1", ["repo-b"])
    assert g["members"] == ["repo-a"]
    assert registry.get_repo(three, "repo-b")  # 只是移出組，登記還在
    with pytest.raises(ValueError, match="不在組"):
        registry.remove_group_members(three, "g1", ["repo-c"])
    assert registry.remove_group_members(three, "g1", ["repo-a"])["members"] == []


def test_set_members_replaces_and_dedupes(three):
    g = registry.set_group_members(three, "g1", ["repo-c", "repo-a", "repo-c"])
    assert g["members"] == ["repo-c", "repo-a"]
    with pytest.raises(ValueError, match="未登記"):
        registry.set_group_members(three, "g1", ["repo-a", "ghost"])


def test_rename_keeps_fields_and_moves_materials(three):
    registry.set_group_field(three, "g1", "goal", "做完 X")
    mat = three / "groups" / "g1" / "materials"
    mat.mkdir(parents=True)
    (mat / "note.md").write_text("料\n", encoding="utf-8")
    r = registry.rename_group(three, "g1", "產品研究")
    assert r["moved_dir"] is True
    g = registry.get_group(three, "產品研究")
    assert g["members"] == ["repo-a", "repo-b"] and g["goal"] == "做完 X"
    assert (three / "groups" / "產品研究" / "materials" / "note.md").exists()
    assert not (three / "groups" / "g1").exists()
    with pytest.raises(ValueError, match="組不存在"):
        registry.get_group(three, "g1")


def test_rename_rejects_bad_names(three):
    registry.add_group(three, "g2", ["repo-c"])
    with pytest.raises(ValueError, match="組已存在"):
        registry.rename_group(three, "g1", "g2")
    for bad in ("", "  ", "a/b", "..", "a\\b"):
        with pytest.raises(ValueError):
            registry.rename_group(three, "g1", bad)
    with pytest.raises(ValueError, match="組不存在"):
        registry.rename_group(three, "nope", "x")


def test_rename_refuses_to_clobber_existing_dir(three):
    """新名字的資料夾已經有東西（例如以前刪掉的同名組留下的料）→ 不合併、不覆蓋。"""
    (three / "groups" / "g1" / "briefs").mkdir(parents=True)
    (three / "groups" / "g9" / "briefs").mkdir(parents=True)
    with pytest.raises(ValueError, match="資料夾已存在"):
        registry.rename_group(three, "g1", "g9")
    assert _members(three, "g1") == ["repo-a", "repo-b"]


def test_remove_group_keeps_materials_and_repos(three):
    (three / "groups" / "g1" / "briefs").mkdir(parents=True)
    r = registry.remove_group(three, "g1")
    assert r["kept_dir"] and (three / "groups" / "g1" / "briefs").exists()
    assert [g["name"] for g in registry.load(three)["groups"]] == []
    assert len(registry.load(three)["repos"]) == 3
    with pytest.raises(ValueError, match="組不存在"):
        registry.remove_group(three, "g1")


def test_schedule_refs_reported(three):
    """config 排程指到這個組 → 改名／刪組時提醒（config.yaml 是人寫的，不代改）。"""
    (three / "config.yaml").write_text(
        "schedule:\n  - {task: brief, at: '09:00', group: g1}\n", encoding="utf-8")
    assert registry.rename_group(three, "g1", "g2")["schedule_refs"] == ["brief@09:00"]
    assert registry.remove_group(three, "g2")["schedule_refs"] == []


# ── CLI ─────────────────────────────────────────────────────────────────

@pytest.mark.e2e
def test_cli_group_maintenance(three):
    r = run_cli("group", "add-member", "g1", "repo-c", spine_dir=three, check=True)
    assert "repo-c" in r.stdout
    run_cli("group", "remove-member", "g1", "repo-a,repo-b", spine_dir=three, check=True)
    assert _members(three, "g1") == ["repo-c"]
    run_cli("group", "rename", "g1", "新名", spine_dir=three, check=True)
    assert _members(three, "新名") == ["repo-c"]
    r = run_cli("group", "remove-member", "新名", "ghost", spine_dir=three)
    assert r.returncode != 0 and "不在組" in (r.stderr + r.stdout)
    run_cli("group", "remove", "新名", spine_dir=three, check=True)
    assert registry.load(three)["groups"] == []
    bodies = [e.body for e in _decisions(three)]
    assert sum("（從 CLI）" in b for b in bodies) == 4  # 每次改動都留痕


# ── 工作台 action ──────────────────────────────────────────────────────

def test_webui_update_and_remove_group(three):
    r = webui._handle_action(three, "update_group",
                             {"name": "g1", "repos": ["repo-c"], "new_name": "g1b"})
    assert r["ok"] and r["name"] == "g1b"
    assert _members(three, "g1b") == ["repo-c"]
    assert webui._handle_action(three, "update_group",
                                {"name": "g1b", "repos": []})["error"]
    assert webui._handle_action(three, "update_group",
                                {"name": "nope", "repos": ["repo-a"]})["error"]
    r = webui._handle_action(three, "remove_group", {"name": "g1b"})
    assert r["ok"] and registry.load(three)["groups"] == []
    assert sum("（從工作台）" in e.body for e in _decisions(three)) == 2


def test_webui_update_group_same_name_only_members(three):
    r = webui._handle_action(three, "update_group",
                             {"name": "g1", "repos": ["repo-a", "repo-c"], "new_name": "g1"})
    assert r["ok"] and _members(three, "g1") == ["repo-a", "repo-c"]


# ── MCP（管理層）───────────────────────────────────────────────────────

def _tool(d, name, args):
    return mcpserver.handle_message(
        d, {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": name, "arguments": args}}, admin=True)


def test_mcp_group_update_and_remove(three):
    r = _tool(three, "group_update", {"name": "g1", "add": ["repo-c"], "remove": ["repo-a"]})
    assert not r["result"].get("isError"), r
    assert _members(three, "g1") == ["repo-b", "repo-c"]
    r = _tool(three, "group_update", {"name": "g1", "rename": "g2"})
    assert _members(three, "g2") == ["repo-b", "repo-c"]
    r = _tool(three, "group_update", {"name": "g2", "add": ["ghost"]})
    assert r["result"].get("isError")
    r = _tool(three, "group_remove", {"name": "g2"})
    assert not r["result"].get("isError") and registry.load(three)["groups"] == []
    assert sum("（從 MCP）" in e.body for e in _decisions(three)) == 3
    work = mcpserver.handle_message(three, {"jsonrpc": "2.0", "id": 1, "method": "tools/list"},
                                    admin=False)
    names = {t["name"] for t in work["result"]["tools"]}
    assert "group_update" not in names and "group_remove" not in names  # 只在管理層


def test_webui_update_group_no_half_apply(three):
    """改名失敗（撞名）→ 組員也不能被改掉。"""
    registry.add_group(three, "g2", ["repo-c"])
    r = webui._handle_action(three, "update_group",
                             {"name": "g1", "repos": ["repo-c"], "new_name": "g2"})
    assert "組已存在" in r["error"]
    assert _members(three, "g1") == ["repo-a", "repo-b"]


def test_own_edits_stay_out_of_inbox_agent_edits_surface(three):
    """自己（CLI／工作台）改組只留痕不進收件匣；agent 經 MCP 改的要讓使用者看得到。"""
    from repo_context import ops
    ops.call(three, "group_update", {"name": "g1", "add": ["repo-c"]}, via="cli")
    ops.call(three, "group_update", {"name": "g1", "remove": ["repo-c"]}, via="ui")
    st = webui.build_state(three)
    assert not [c for c in st["cards"] if "編輯組" in " ".join(c.get("lines", []) + [c.get("title", "")])]
    ops.call(three, "group_update", {"name": "g1", "rename": "g9"}, via="mcp")
    st = webui.build_state(three)
    assert [c for c in st["cards"] if "改名 g1 → g9" in str(c)]


def test_ops_table_is_the_single_source(three):
    """MCP 管理層的寫入工具＝操作表的全部操作（加一個 @op 就自動多一個工具，schema 同源）。"""
    from repo_context import ops
    tools = mcpserver.tool_table(three, admin=True)
    for name, o in ops.OPS.items():
        assert tools[name][1] == o.schema()
    import pytest
    with pytest.raises(ValueError, match="不認得參數"):
        ops.call(three, "group_remove", {"name": "g1", "force": True}, via="cli")
    with pytest.raises(ValueError, match="缺參數"):
        ops.call(three, "group_add", {"name": "x"}, via="cli")


def test_admin_edits_are_not_progress(three):
    """改設定（建組、改組、改目標）不是工作進度：摘要的「上次進度」不拿它充數。"""
    from repo_context import ops, summary
    ops.call(three, "group_update", {"name": "g1", "add": ["repo-c"]}, via="ui")
    ops.call(three, "group_goal", {"name": "g1", "text": "做完 X"}, via="cli")
    s = summary.build_summary(three, "g1")
    assert s["progress"] is None and "沒有本組的進度紀錄" in s["progress_gap"]
    spine_mod.append_event(three, "decision", "monitor", ["group:g1"], body="會話結果：做完第一步")
    assert "會話結果" in summary.build_summary(three, "g1")["progress"]["text"]
