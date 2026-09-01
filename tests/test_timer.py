"""P12 timer：到期判定純函數（當日補課、同日不重跑、跨日重來）＋tick 真跑任務＋失敗浮出。"""
import datetime as dt
import json
from pathlib import Path

from repoengine import timer
from repoengine import spine as spine_mod

SCHED = [{"task": "brief", "at": "07:30", "group": "g1"}]
KEY = "brief@07:30@g1"


def test_due_entries_rules():
    def due(hhmm, state):
        now = dt.datetime(2026, 9, 1, *map(int, hhmm.split(":")))
        return timer.due_entries(SCHED, state, now)
    assert due("07:00", {}) == []                       # 還沒到點
    assert due("07:30", {}) == SCHED                    # 到點
    assert due("23:00", {}) == SCHED                    # 補課：當日內晚到也跑
    assert due("23:00", {KEY: "2026-09-01"}) == []      # 同日跑過不重跑
    assert timer.due_entries(SCHED, {KEY: "2026-09-01"},
                             dt.datetime(2026, 9, 2, 8, 0)) == SCHED  # 跨日重來
    # 跨日不補昨天的：state 空、新的一天早於排程時刻 → 不跑
    assert timer.due_entries(SCHED, {}, dt.datetime(2026, 9, 2, 6, 0)) == []


def test_tick_runs_brief_once_per_day(spine_with_repos):
    sched = [{"task": "brief", "at": "00:00", "group": "g1"}]
    ran = timer.tick(spine_with_repos, schedule=sched)
    assert ran == [("brief@00:00@g1", True)]
    assert any(e.type == "presented" for e in spine_mod.iter_events(spine_with_repos))
    today = f"{dt.date.today():%Y-%m-%d}"
    assert (Path(spine_with_repos) / "groups" / "g1" / "briefs" / f"{today}.md").exists()
    state = json.loads((Path(spine_with_repos) / ".state" / "timer.json")
                       .read_text(encoding="utf-8"))
    assert state["brief@00:00@g1"] == today
    assert timer.tick(spine_with_repos, schedule=sched) == []  # 同日第二輪不重跑


def test_tick_failure_floats_and_does_not_retry_bomb(spine):
    sched = [{"task": "brief", "at": "00:00", "group": "no-such-group"}]
    ran = timer.tick(spine, schedule=sched)
    assert ran == [("brief@00:00@no-such-group", False)]
    evs = [e for e in spine_mod.iter_events(spine) if e.source == "timer"]
    assert len(evs) == 1 and "system-unsure" in evs[0].body  # 失敗必須浮出
    assert timer.tick(spine, schedule=sched) == []  # 失敗也記今天跑過——不轟炸
