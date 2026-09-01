"""L1：P6 打包選料 / P8 路由 / P7 兩段式碰撞（mock provider）。"""
import os

import pytest

from repoengine import collide, pack, registry, route, spine as spine_mod


def test_pack_budget_and_exclusion(spine_with_repos):
    _, entries = registry.resolve_group(spine_with_repos, "g1")
    text, inc, exc = pack.pack_group(entries, token_budget=100000)
    assert exc == [] and len(inc) >= 4          # 兩 repo 各 README+plan
    assert "widget 快取設計" in text            # 內容真的在
    # 縮到很小的預算 → 有東西被擠出去，且標頭有列
    text2, inc2, exc2 = pack.pack_group(entries, token_budget=15)
    assert exc2 and "未納入" in text2


def test_pack_respects_gitignore(spine_with_repos, tmp_path):
    """L4 實錘回歸：gitignored 的 clone 森林不得進料（rglob 時代 1551 檔擠占預算）。"""
    from pathlib import Path
    ra = Path(registry.get_repo(spine_with_repos, "repo-a")["path"])
    clones = ra / "tool_clones" / "some-clone"
    clones.mkdir(parents=True)
    (clones / "noise.md").write_text("clone 雜訊\n" * 50, encoding="utf-8")
    (ra / ".gitignore").write_text("tool_clones/\n", encoding="utf-8")
    _, entries = registry.resolve_group(spine_with_repos, None, "repo-a")
    text, inc, exc = pack.pack_group(entries, token_budget=100000)
    assert "clone 雜訊" not in text
    assert not any("noise.md" in x for x in inc + exc)
    assert "widget 快取設計" in text  # 自己的正文還在


def test_route_to_repo_inbox(spine_with_repos):
    p = route.route(spine_with_repos, "碰撞結論內容", "repo:repo-a", title="cache-idea")
    assert p.exists() and "01_inbox" in str(p)
    assert p.read_text(encoding="utf-8") == "碰撞結論內容"
    evs = spine_mod.query(spine_with_repos, type="decision")
    assert len(evs) == 1 and "repo:repo-a" in evs[0].header()


def test_route_to_incubator_and_group(spine_with_repos):
    p1 = route.route(spine_with_repos, "x", "incubator")
    p2 = route.route(spine_with_repos, "y", "group:g1")
    assert "incubator" in str(p1) and "materials" in str(p2)
    with pytest.raises(ValueError):
        route.route(spine_with_repos, "z", "nowhere:abc")


def test_collide_two_stage_mock(spine_with_repos):
    # 段一：想法即刻落脊椎
    cid = collide.submit(spine_with_repos, "把快取改成 LFU 如何？（已知 的變形）",
                         group="g1")
    opened = [e for e in spine_mod.query(spine_with_repos, type="collision")
              if e.body.startswith("opened")]
    assert len(opened) == 1 and "LFU" in opened[0].body
    assert opened[0].kv("id") == cid
    # 段二：mock 判定落脊椎，五欄位齊
    j = collide.run_judgement(spine_with_repos, cid, provider="mock")
    assert j["判定"] in ("已知", "衝突", "真增量")
    results = [e for e in spine_mod.query(spine_with_repos, type="collision")
               if e.kv("ref") == f"collision:{cid}"]
    assert len(results) == 1
    for field in ("判定：", "理由：", "證據：", "落點建議：", "下一步："):
        assert field in results[0].body
    # 判定結果是未讀（回程通知的資料來源）
    assert any(e.kv("ref") == f"collision:{cid}"
               for e in spine_mod.get_unread(spine_with_repos))


def test_collide_inner_validation_retry(spine_with_repos, monkeypatch):
    """內層驗證失敗重試一次：第 1 次壞、第 2 次好 → 成功。"""
    from repoengine.agents import MockProvider
    MockProvider._calls = 0
    monkeypatch.setenv("REPOENGINE_MOCK_FAIL", "1")
    cid = collide.submit(spine_with_repos, "重試路徑測試", group="g1")
    j = collide.run_judgement(spine_with_repos, cid, provider="mock")
    assert j is not None


def test_collide_system_unsure_surfaces(spine_with_repos, monkeypatch):
    """兩次都壞 → 不准沉默：落 system-unsure 事件、回傳 None。"""
    from repoengine.agents import MockProvider
    MockProvider._calls = 0
    monkeypatch.setenv("REPOENGINE_MOCK_FAIL", "5")
    cid = collide.submit(spine_with_repos, "必敗測試", group="g1")
    j = collide.run_judgement(spine_with_repos, cid, provider="mock")
    assert j is None
    fails = [e for e in spine_mod.query(spine_with_repos, type="collision")
             if "system-unsure" in e.body]
    assert len(fails) == 1


def test_collision_id_sequence(spine_with_repos):
    a = collide.submit(spine_with_repos, "想法一", group="g1")
    b = collide.submit(spine_with_repos, "想法二", group="g1")
    assert a.endswith("-a") and b.endswith("-b")


def test_agent_raw_log_written(spine_with_repos):
    from repoengine.agents import MockProvider
    MockProvider._calls = 0
    os.environ.pop("REPOENGINE_MOCK_FAIL", None)
    cid = collide.submit(spine_with_repos, "raw log 測試", group="g1")
    collide.run_judgement(spine_with_repos, cid, provider="mock")
    logs = list((spine_with_repos / ".agent_logs").glob("*.json"))
    assert logs, "每次 agent 呼叫 raw request/response 必落檔"


def test_mock_verdict_follows_idea_not_template(spine_with_repos):
    """回歸：prompt 模板含三個判定詞，mock 只准看【想法】段。"""
    from repoengine.agents import MockProvider
    import os
    MockProvider._calls = 0
    os.environ.pop("REPOENGINE_MOCK_FAIL", None)
    cid = collide.submit(spine_with_repos, "一個完全嶄新的方向", group="g1")
    j = collide.run_judgement(spine_with_repos, cid, provider="mock")
    assert j["判定"] == "真增量"
    cid2 = collide.submit(spine_with_repos, "這與既有結論衝突吧", group="g1")
    j2 = collide.run_judgement(spine_with_repos, cid2, provider="mock")
    assert j2["判定"] == "衝突"


def test_collide_adhoc_repos_token_roundtrip(spine_with_repos):
    """P2 臨時組合免建組：submit(repos=...) 落 group:臨時(...) token，
    run_judgement 不帶參數也能從事件解回選料範圍（detached 進程同路徑）。"""
    from repoengine.agents import MockProvider
    import os
    MockProvider._calls = 0
    os.environ.pop("REPOENGINE_MOCK_FAIL", None)
    cid = collide.submit(spine_with_repos, "臨時組合的想法", repos=["repo-a"])
    opened = [e for e in spine_mod.query(spine_with_repos, type="collision")
              if e.kv("id") == cid][0]
    assert opened.kv("group") == "臨時(repo-a)"
    j = collide.run_judgement(spine_with_repos, cid)  # 不給 group/repos
    assert j is not None and j["判定"] in ("已知", "衝突", "真增量")
