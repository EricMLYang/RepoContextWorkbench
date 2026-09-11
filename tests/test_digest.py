"""P5 digest：mock provider——首輪產摘要落事件、二輪水位線擋住（一律增量）、失敗浮出。"""
from repo_context import digest
from repo_context import spine as spine_mod
from repo_context.agents import AgentUnsure


def test_digest_then_incremental_watermark(spine_with_repos):
    res = digest.run(spine_with_repos, group="g1")  # config 預設 provider=mock
    ok = {r["repo"]: r for r in res if r["note"] == "ok"}
    assert "repo-a" in ok and "repo-b" in ok  # 兩個 repo 的 commit 都在 7 天內
    for r in ok.values():
        assert r["file"] and r["file"].endswith(".md")
    evs = [e for e in spine_mod.iter_events(spine_with_repos) if e.source == "digest"]
    assert len(evs) == 2 and all(e.type == "suggestion" for e in evs)
    # 二輪：水位線＝剛剛的事件時間，無新 commit → 不寫事件（無事不報）
    res2 = digest.run(spine_with_repos, group="g1")
    assert all(r["note"] == "無新 commit" for r in res2)
    assert len([e for e in spine_mod.iter_events(spine_with_repos)
                if e.source == "digest"]) == 2


def test_digest_failure_floats_system_unsure(spine_with_repos, monkeypatch):
    def boom(*a, **kw):
        raise AgentUnsure("模擬 agent 掛掉")
    monkeypatch.setattr(digest, "run_text", boom)
    res = digest.run(spine_with_repos, group="g1")
    assert all(r["note"] == "失敗（已浮出）" for r in res)
    evs = [e for e in spine_mod.iter_events(spine_with_repos) if e.source == "digest"]
    assert evs and all("system-unsure" in e.body for e in evs)
