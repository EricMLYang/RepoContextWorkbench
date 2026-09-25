"""反芻輪（2026-09-25）：新進的知識與別組的決策，主動對上每組手上的工作。

釘住：① 別組新進的 md 跟手上工作（目標／未結／判斷）強相關才推，自己組的不推；
② 只看上次反芻之後的新料，同一份不重推；③ 寧缺勿濫（共同少見詞不夠多不推）；
④ 別組新判斷也會對上；⑤ 被 read_from 讀了＝用上了（命中率）、可略過；
⑥ 開場注入看得到；⑦ 排程任務與 CLI 走同一份。
"""
import datetime as dt
import json
import os
import time

import pytest

from repo_context import agentapi as api
from repo_context import hooks, registry, ruminate, timer
from tests.conftest import make_git_repo, run_cli

CARD = """# 向量索引的冷啟動與增量重建

增量重建要記水位線，冷啟動時先載入快照再補增量。
倒排索引的分段合併策略會影響查詢延遲；合併太頻繁會拖垮寫入吞吐。
"""
UNRELATED = "# 週末料理\n紅燒肉要先汆燙，再用冰糖炒色，小火慢燉一小時。\n"


@pytest.fixture
def world(spine, tmp_path):
    """兩組：產品（impl，手上在做索引重建）、知識（cards，卡片 repo）。"""
    impl = make_git_repo(tmp_path, "impl")
    cards = make_git_repo(tmp_path, "cards")
    registry.add_repo(spine, "impl", impl)
    registry.add_repo(spine, "cards", cards)
    registry.add_group(spine, "產品", ["impl"])
    registry.add_group(spine, "知識", ["cards"])
    registry.set_group_field(spine, "產品", "goal", "把搜尋服務的向量索引改成增量重建")
    api.add_todo(spine, "冷啟動太慢：載入快照後補增量，量測查詢延遲", cwd=impl)
    api.log_decision(spine, "倒排索引分段合併改成每小時一次，避免拖垮寫入吞吐", cwd=impl)
    return spine, {"impl": impl, "cards": cards}


def _age(path, days):
    t = time.time() - days * 86400
    os.utime(path, (t, t))


def test_new_related_card_is_pushed_to_group(world):
    sp, r = world
    (r["cards"] / "索引重建.md").write_text(CARD, encoding="utf-8")
    (r["cards"] / "料理.md").write_text(UNRELATED, encoding="utf-8")
    res = ruminate.run(sp)
    refs = [l["ref"] for l in res["groups"].get("產品", [])]
    assert refs == ["cards:索引重建.md"]                  # 相關的推、不相關的不推
    link = res["groups"]["產品"][0]
    assert {"增量", "重建"} & set(link["terms"])          # 說得出為什麼
    assert link["snippets"] and "目標" in link["basis"]


def test_own_group_and_old_files_not_pushed(world):
    sp, r = world
    (r["impl"] / "索引筆記.md").write_text(CARD, encoding="utf-8")   # 自己組的
    old = r["cards"] / "舊卡.md"
    old.write_text(CARD, encoding="utf-8")
    _age(old, 60)                                                   # 反芻窗之前
    for f in (r["cards"] / "README.md", r["cards"] / "notes" / "plan.md"):
        _age(f, 60)
    res = ruminate.run(sp)
    assert "產品" not in res["groups"]


def test_no_repeat_and_watermark(world):
    sp, r = world
    (r["cards"] / "索引重建.md").write_text(CARD, encoding="utf-8")
    first = ruminate.run(sp)
    assert first["groups"]["產品"]
    second = ruminate.run(sp, now=dt.datetime.now() + dt.timedelta(minutes=1))
    assert "產品" not in second["groups"]                 # 同一份不重推
    assert len(ruminate.fresh(sp, "產品")) == 1


def test_weak_overlap_is_not_pushed(world):
    sp, r = world
    (r["cards"] / "一句.md").write_text("# 雜記\n今天讀到索引兩個字。\n", encoding="utf-8")
    res = ruminate.run(sp)
    assert "產品" not in res["groups"]                    # 寧缺勿濫


def test_other_group_decision_matches(world):
    sp, r = world
    api.log_decision(sp, "讀完論文：向量索引增量重建要先記水位線，冷啟動載入快照後補增量，"
                         "查詢延遲才穩", cwd=r["cards"])
    res = ruminate.run(sp)
    kinds = {l["kind"] for l in res["groups"].get("產品", [])}
    assert "decision" in kinds
    # 反過來：產品組自己的判斷不會推回給產品組
    assert all(l["repo"] != "impl" for l in res["groups"]["產品"])


def test_used_via_read_from_and_dismiss(world):
    sp, r = world
    (r["cards"] / "索引重建.md").write_text(CARD, encoding="utf-8")
    ruminate.run(sp, now=dt.datetime.now() - dt.timedelta(minutes=5))
    api.read_from(sp, "cards:索引重建.md", cwd=r["impl"])
    s = ruminate.stats(sp)
    assert s["used"] == 1 and s["hit_rate"] == 1.0
    assert ruminate.fresh(sp, "產品") == []               # 用上了就不再當新推薦
    assert ruminate.fresh(sp, "產品", include_seen=True)[0]["status"] == "used"

    (r["cards"] / "索引二.md").write_text(CARD + "\n分段合併的查詢延遲量測。\n", encoding="utf-8")
    ruminate.run(sp)
    lid = ruminate.fresh(sp, "產品")[0]["id"]
    ruminate.dismiss(sp, lid)
    assert ruminate.fresh(sp, "產品") == []
    with pytest.raises(ValueError):
        ruminate.dismiss(sp, "r999")


def test_session_start_shows_fresh(world):
    sp, r = world
    (r["cards"] / "索引重建.md").write_text(CARD, encoding="utf-8")
    ruminate.run(sp)
    out = hooks.session_start(sp, {"session_id": "s1", "cwd": str(r["impl"])})
    text = json.loads(out)["hookSpecificOutput"]["additionalContext"]
    assert "反芻" in text and "cards:索引重建.md" in text
    ctx = api.context_for(sp, cwd=r["impl"], card=False)
    assert ctx["fresh"][0]["ref"] == "cards:索引重建.md"


def test_stale_detection(world):
    sp, _ = world
    assert ruminate.is_stale(sp)
    ruminate.run(sp)
    assert not ruminate.is_stale(sp)
    assert ruminate.is_stale(sp, dt.datetime.now() + dt.timedelta(days=2))


def test_timer_task(world):
    sp, r = world
    (r["cards"] / "索引重建.md").write_text(CARD, encoding="utf-8")
    ran = timer.tick(sp, now=dt.datetime.now().replace(hour=23, minute=0),
                     schedule=[{"task": "ruminate", "at": "06:00"}])
    assert ran == [("ruminate@06:00@", True)]
    assert ruminate.fresh(sp, "產品")


@pytest.mark.e2e
def test_cli_fresh(world):
    sp, r = world
    (r["cards"] / "索引重建.md").write_text(CARD, encoding="utf-8")
    p = run_cli("fresh", "--run", spine_dir=sp)
    assert p.returncode == 0 and "1 筆新推薦" in p.stdout
    p = run_cli("fresh", "--json", "--cwd", str(r["impl"]), spine_dir=sp)
    data = json.loads(p.stdout)
    assert data["links"][0]["ref"] == "cards:索引重建.md"
    p = run_cli("fresh", "--dismiss", data["links"][0]["id"], spine_dir=sp)
    assert p.returncode == 0
    p = run_cli("fresh", "--stats", spine_dir=sp)
    assert "略過 1" in p.stdout


# ---- 真資料校準（2026-09-25）留下的三條 ----

def test_rare_floor_not_median():
    # 卡片庫大部分詞只出現一兩次：門檻要能收「出現在少數幾份」的詞，不能只剩只出現一次的
    import math
    n = 1500
    idf_df5 = math.log(1 + (n - 5 + 0.5) / (5 + 0.5))
    assert idf_df5 >= ruminate._rare_floor(n)


def test_function_char_bigrams_do_not_count():
    assert not ruminate._meaningful("接是") and not ruminate._meaningful("得上")
    assert ruminate._meaningful("交接") and ruminate._meaningful("hook")


def test_dot_dirs_and_duplicates_skipped(world):
    sp, r = world
    for sub in (".claude/skills", ".agents/skills"):
        (r["cards"] / sub).mkdir(parents=True)
        (r["cards"] / sub / "SKILL.md").write_text(CARD, encoding="utf-8")
    (r["cards"] / "a.md").write_text(CARD, encoding="utf-8")
    (r["cards"] / "b.md").write_text(CARD, encoding="utf-8")   # 同內容第二份
    refs = [l["ref"] for l in ruminate.run(sp)["groups"]["產品"]]
    assert refs in (["cards:a.md"], ["cards:b.md"])
