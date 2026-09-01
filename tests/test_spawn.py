"""P9 生：升格建 repo＋出生登記（origin 血統）＋outcome 回連（命中率）＋存活率視圖。"""
from pathlib import Path

import pytest

from repoengine import collide, registry, spawn
from repoengine import spine as spine_mod


def test_spawn_from_collision_closes_the_loop(spine, tmp_path):
    cid = collide.submit(spine, "值得長成 repo 的想法")
    target = tmp_path / "born-repo"
    p = spawn.spawn(spine, "born-repo", target, origin=f"collision:{cid}")
    assert (Path(p) / ".git").is_dir() and (Path(p) / "README.md").is_file()
    assert registry.get_repo(spine, "born-repo")["origin"] == f"collision:{cid}"
    # decision 出生登記 ＋ outcome 回連
    types = [e.type for e in spine_mod.iter_events(spine)]
    assert "decision" in types and "outcome" in types
    s = spine_mod.stats(spine)
    assert s["collisions"] == 1 and s["collisions_with_outcome"] == 1
    assert s["hit_rate"] == 1.0  # M2 驗收③：命中率從真資料算得出來
    sv = registry.survival(spine)
    assert sv == {"spawned": 1, "alive": 1, "survival_rate": 1.0}


def test_spawn_moves_incubator_material(spine, tmp_path):
    inc = Path(spine) / "incubator"
    (inc / "idea.md").write_text("種子內容\n", encoding="utf-8")
    p = spawn.spawn(spine, "r2", tmp_path / "r2", from_incubator="idea.md")
    assert not (inc / "idea.md").exists()          # 搬離 incubator＝升格完成
    assert (Path(p) / "idea.md").read_text(encoding="utf-8") == "種子內容\n"


def test_spawn_rejects_nonempty_target_and_missing_material(spine, tmp_path):
    busy = tmp_path / "busy"
    busy.mkdir()
    (busy / "x.txt").write_text("x", encoding="utf-8")
    with pytest.raises(ValueError, match="非空"):
        spawn.spawn(spine, "r3", busy)
    with pytest.raises(ValueError, match="素材不存在"):
        spawn.spawn(spine, "r4", tmp_path / "r4", from_incubator="nope.md")
    # 失敗的 spawn 不准弄髒 registry
    assert all(r["id"] not in ("r3", "r4")
               for r in registry.load(spine)["repos"])
