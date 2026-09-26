"""回填（2026-09-26）：從過去的 Claude Code 會話把判斷／待辦／交接撈回 spine。

釘住：① cwd 對到 repo（含 Drive 舊路徑依資料夾名）、還在寫的／對不到的略過；
② 壓縮只留原話、agent 文字、工具一行（tool_result／系統標記丟掉）、輪次編號；
③ 可續跑：做過的不重做、單場失敗不擋整批；④ 候選不自動進 spine，accept 才寫、source=backfill、
不重收、交接是最新一筆；⑤ 反芻吃得到收下的判斷。
"""
import json
import os
import time

import pytest

from repo_context import agentapi as api
from repo_context import backfill as bf
from repo_context import registry, spine as spine_mod
from tests.conftest import make_git_repo


def _write_session(root, proj, sid, cwd, turns, age_hours=5):
    d = root / proj
    d.mkdir(parents=True, exist_ok=True)
    rows = [{"type": "ai-title", "aiTitle": "索引重建"}]
    ts = "2026-09-10T10:00:00Z"
    for u, a in turns:
        rows.append({"type": "user", "cwd": str(cwd), "timestamp": ts,
                     "message": {"role": "user", "content": u}})
        rows.append({"type": "assistant", "cwd": str(cwd), "timestamp": ts,
                     "message": {"role": "assistant", "content": [
                         {"type": "thinking", "thinking": "祕密思考"},
                         {"type": "text", "text": a},
                         {"type": "tool_use", "name": "Edit", "input": {"file_path": "/x/a.py"}}]}})
        rows.append({"type": "user", "cwd": str(cwd), "timestamp": ts, "message": {
            "role": "user", "content": [{"type": "tool_result", "content": "巨量輸出" * 50}]}})
    p = d / f"{sid}.jsonl"
    p.write_text("\n".join(json.dumps(r, ensure_ascii=False) for r in rows), encoding="utf-8")
    t = time.time() - age_hours * 3600
    os.utime(p, (t, t))
    return p


EXTRACTED = {"summary": "改索引", "decisions": [{"text": "索引改增量重建", "basis": "全量太慢", "turn": "t1"}],
             "todos": [{"text": "補壓測", "turn": "t2"}], "done": ["加水位線"], "next": "跑壓測",
             "goal_hints": ["讓搜尋更快"]}
CONSOLIDATED = {"decisions": [{"text": "索引改增量重建", "basis": "全量太慢", "sid": "s1aaaaaa", "date": "2026-09-10"}],
                "open_todos": [{"text": "補壓測", "sid": "s1aaaaaa", "date": "2026-09-10", "uncertain": False}],
                "handoff": {"done": "加水位線", "next": "跑壓測", "sid": "s1aaaaaa", "date": "2026-09-10"},
                "goal_hints": [{"text": "讓搜尋更快", "sid": "s1aaaaaa"}]}


class FakeCall:
    def __init__(self, fail_sids=()):
        self.prompts, self.fail_sids = [], fail_sids

    def __call__(self, prompt):
        self.prompts.append(prompt)
        if any(s in prompt for s in self.fail_sids):
            raise bf._agents.AgentUnsure("boom")
        if "逐場整理" in prompt:
            return "好的：" + json.dumps(CONSOLIDATED, ensure_ascii=False)
        return json.dumps(EXTRACTED, ensure_ascii=False)


@pytest.fixture
def world(spine, tmp_path):
    impl = make_git_repo(tmp_path, "impl")
    registry.add_repo(spine, "impl", impl)
    registry.add_group(spine, "產品", ["impl"])
    root = tmp_path / "projects"
    return spine, impl, root


def test_scan_maps_cwd_and_skips(world, tmp_path):
    sp, impl, root = world
    _write_session(root, "a", "s1", impl / "src", [("改索引", "好")])
    _write_session(root, "b", "s2", tmp_path / "Drive" / "old" / "impl", [("舊路徑", "好")])
    _write_session(root, "c", "s3", tmp_path / "elsewhere", [("別的", "好")])
    _write_session(root, "d", "s4", impl, [("正在寫", "好")], age_hours=0)
    st = {r["sid"]: (r["repo"], r["status"]) for r in bf.scan(sp, root)}
    assert st["s1"] == ("impl", "pending")          # 子目錄
    assert st["s2"] == ("impl", "pending")          # Drive 舊路徑：資料夾名對上
    assert st["s3"] == (None, "unmapped")
    assert st["s4"][1] == "live"                     # 還在寫的不碰


def test_compact_keeps_words_drops_noise(world):
    sp, impl, root = world
    p = _write_session(root, "a", "s1", impl, [
        ("<system-reminder>別理我</system-reminder>改索引吧", "我改成增量"), ("再補壓測", "好")])
    text = "\n".join(bf.compact(p))
    assert "[t1] 【使用者】改索引吧" in text and "[t2] 【使用者】再補壓測" in text
    assert "【agent】我改成增量" in text and "〔工具 Edit〕/x/a.py" in text
    assert "別理我" not in text and "巨量輸出" not in text and "祕密思考" not in text


def test_chunks_split_by_size():
    parts = bf.chunks(["a" * 30] * 10, size=100)
    assert len(parts) == 4 and all(len(p) <= 100 for p in parts)


def test_run_resumable_and_failure_does_not_stop(world):
    sp, impl, root = world
    _write_session(root, "a", "s1", impl, [("改索引", "好")])
    _write_session(root, "a", "s2", impl, [("壞掉這場 s2marker", "好")])
    call = FakeCall(fail_sids=("s2marker",))
    res = bf.run(sp, call=call, root=root, log=lambda *_: None)
    assert res["extracted"] == 2 and len(res["failed"]) == 1 and res["consolidated"] == ["impl"]
    n = len(call.prompts)
    res = bf.run(sp, call=FakeCall(), root=root, log=lambda *_: None)   # 續跑：只補失敗那場
    assert res["extracted"] == 1 and not res["failed"]
    assert n >= 2


def test_candidates_not_written_until_accept(world):
    sp, impl, root = world
    _write_session(root, "a", "s1", impl, [("改索引", "好")])
    bf.run(sp, call=FakeCall(), root=root, log=lambda *_: None)
    assert not [e for e in spine_mod.iter_events(sp) if e.source == "backfill"]
    text = bf.review_text(sp)
    assert "[d1]" in text and "[t1]" in text and "[h]" in text and "讓搜尋更快" in text

    refs = bf.accept(sp, "impl", ["all"])
    assert len(refs) == 3
    evs = [e for e in spine_mod.iter_events(sp) if e.source == "backfill"]
    assert all(e.kv("repo") == "impl" and e.kv("group") == "產品" for e in evs)
    assert "回填自 2026-09-10" in evs[0].body
    ctx = api.context_for(sp, cwd=impl, card=False)
    assert ctx["last_handoff"]["summary"].startswith("加水位線")   # 交接最後寫＝上次做到哪
    assert any("補壓測" in l["text"] for l in ctx["open_loops"])
    assert bf.accept(sp, "impl", ["all"]) == []                        # 不重收
    assert "✔ 已收" in bf.review_text(sp)
    with pytest.raises(ValueError):
        bf.accept(sp, "impl", ["d99"])


def test_goal_is_never_written(world):
    sp, impl, root = world
    _write_session(root, "a", "s1", impl, [("改索引", "好")])
    bf.run(sp, call=FakeCall(), root=root, log=lambda *_: None)
    bf.accept(sp, "impl", ["all"])
    assert not registry.get_group(sp, "產品").get("goal")    # 目標只由使用者給


def test_remap_when_more_specific_repo_registered(world, tmp_path):
    """先只登記上層 repo、後來子專案也登記了：已抽過的會話改歸子專案，兩邊都重彙整，不重抽。"""
    sp, impl, root = world
    sub = impl / "projects" / "sub"
    sub.mkdir(parents=True)
    _write_session(root, "a", "s1", sub, [("改索引", "好")])
    bf.run(sp, call=FakeCall(), root=root, log=lambda *_: None)
    assert bf.candidates(sp, "impl")
    registry.add_repo(sp, "sub", sub)
    call = FakeCall()
    res = bf.run(sp, call=call, root=root, log=lambda *_: None)
    assert res["extracted"] == 0 and res["consolidated"] == ["sub"]
    assert all("逐場整理" in p for p in call.prompts)        # 只重彙整，沒重抽
    assert bf.candidates(sp, "sub") and not bf.candidates(sp, "impl")   # 沒會話了的 repo 候選清掉


def test_parallel_jobs_same_result(world):
    sp, impl, root = world
    for i in range(6):
        _write_session(root, "a", f"s{i}", impl, [(f"改索引 {i}", "好")])
    res = bf.run(sp, call=FakeCall(), root=root, log=lambda *_: None, jobs=4)
    assert res["extracted"] == 6 and not res["failed"] and res["consolidated"] == ["impl"]
    assert len(list((sp / ".state" / "backfill" / "sessions").glob("*.json"))) == 6


def test_interrupted_remap_heals_on_next_run(world):
    """改掛會話後、彙整前被中斷：下一輪要看出候選與實際會話對不上，清掉／重彙整（不能只靠「這輪有沒有改掛」）。"""
    sp, impl, root = world
    sub = impl / "projects" / "sub"
    sub.mkdir(parents=True)
    _write_session(root, "a", "s1", sub, [("改索引", "好")])
    bf.run(sp, call=FakeCall(), root=root, log=lambda *_: None)
    registry.add_repo(sp, "sub", sub)
    p = sp / ".state" / "backfill" / "sessions" / "s1.json"      # 模擬：改掛寫進去了，但沒彙整就被殺
    s = json.loads(p.read_text(encoding="utf-8"))
    s["repo"] = "sub"
    p.write_text(json.dumps(s, ensure_ascii=False), encoding="utf-8")
    res = bf.run(sp, call=FakeCall(), root=root, log=lambda *_: None)
    assert res["consolidated"] == ["sub"]
    assert not bf.candidates(sp, "impl") and bf.candidates(sp, "sub")


@pytest.mark.e2e
def test_cli_accept_positional_repo(world):
    """真 CLI：`accept <repo> d1 t1`；argparse 對「--repo 在前、編號在後」吃不下，一律用位置參數。"""
    from tests.conftest import run_cli
    sp, impl, root = world
    _write_session(root, "a", "s1", impl, [("改索引", "好")])
    bf.run(sp, call=FakeCall(), root=root, log=lambda *_: None)
    r = run_cli("backfill", "accept", "impl", "d1", "t1", spine_dir=sp)
    assert r.returncode == 0, r.stderr
    assert "寫進 spine 2 筆" in r.stdout
    r = run_cli("backfill", "accept", "d1", "--repo", "impl", spine_dir=sp)
    assert r.returncode == 0 and "都收過了" in r.stdout
