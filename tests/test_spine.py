"""L1：脊椎寫入/查詢/dead-letter/stats/lint（不變式 2、6、7＋衛生迴圈）。"""
import datetime as dt

import pytest

from repoengine.spine import (
    ValidationError, ack_unread, append_event, get_unread,
    lint, open_loops, query, stats,
)


def _t(h, m):
    return dt.datetime.now().replace(hour=h, minute=m)


def test_append_and_query(spine):
    ev = append_event(spine, "presented", "morning-brief", ["group:g1"],
                      body="呈現了三件事", when=_t(7, 42))
    assert ev.time == "07:42"
    got = query(spine, type="presented")
    assert len(got) == 1 and got[0].body == "呈現了三件事"


def test_inv_reject_programmatic_raises(spine):
    with pytest.raises(ValidationError):
        append_event(spine, "not-a-type", "manual", body="x")
    assert query(spine) == []  # 什麼都沒寫進去


def test_inv_reject_human_goes_dead_letter(spine):
    ev = append_event(spine, "not-a-type", "hotkey",
                      body="珍貴的想法原文", dead_letter=True)
    assert ev is None
    dl = list((spine / "spine" / "dead-letter").glob("*.md"))
    assert len(dl) == 1
    assert "珍貴的想法原文" in dl[0].read_text(encoding="utf-8")
    assert query(spine) == []  # 壞事件沒進脊椎


def test_open_loops_view(spine):
    append_event(spine, "open-loop", "hotkey", ["#13", "due:2026-09-15"],
                 body="opened →「驗證 X」", when=_t(9, 0))
    append_event(spine, "open-loop", "hotkey", ["#14"],
                 body="opened →「另一件」", when=_t(9, 1))
    append_event(spine, "open-loop", "manual", ["#14"],
                 body="closed → 做完了", when=_t(10, 0))
    loops = open_loops(spine)
    assert len(loops) == 1
    assert "#13" in loops[0].tokens


def test_inv_stats_hit_rate_hand_checked(spine):
    """靈感命中率＝有 outcome 回連的碰撞 / 全部碰撞。手算：2 撞 1 中 = 50%。"""
    append_event(spine, "collision", "hotkey", ["id:2026-09-01-a"],
                 body="opened\n輸入：想法一", when=_t(9, 0))
    append_event(spine, "collision", "hotkey", ["id:2026-09-01-b"],
                 body="opened\n輸入：想法二", when=_t(9, 5))
    append_event(spine, "outcome", "manual", ["ref:collision:2026-09-01-a"],
                 body="長成了一篇文章", when=_t(21, 0))
    s = stats(spine)
    assert s["collisions"] == 2
    assert s["collisions_with_outcome"] == 1
    assert s["hit_rate"] == 0.5


def test_inv_chosen_refs_presented(spine):
    """chosen 帶 ref:presented:HH:MM 能解回原事件（M2 驗收①的資料基礎）。"""
    append_event(spine, "presented", "morning-brief", ["group:g1"],
                 body="呈現了", when=_t(7, 42))
    append_event(spine, "chosen", "morning-brief", ["ref:presented:07:42"],
                 body="選了第一項", when=_t(7, 45))
    chosen = query(spine, type="chosen")[0]
    ref_time = chosen.kv("ref").split(":", 1)[1]
    src = [e for e in query(spine, type="presented") if e.time == ref_time]
    assert len(src) == 1 and src[0].body == "呈現了"


def test_unread_marker(spine):
    append_event(spine, "decision", "manual", body="第一筆", when=_t(9, 0))
    assert len(get_unread(spine)) == 1
    ack_unread(spine, when=_t(9, 30))
    assert get_unread(spine) == []
    append_event(spine, "decision", "manual", body="第二筆", when=_t(10, 0))
    assert len(get_unread(spine)) == 1


# ── 衛生迴圈（second-brain lint 課＋Szapar staleness 課）──────────────


def test_lint_clean_spine(spine):
    """健康脊椎：好 ref、未到期 loop、今天的 opened 碰撞——零紅字。"""
    append_event(spine, "presented", "morning-brief", [], body="呈現了", when=_t(7, 42))
    append_event(spine, "chosen", "morning-brief", ["ref:presented:07:42"],
                 body="選了", when=_t(7, 45))
    append_event(spine, "open-loop", "hotkey", ["#1", "due:2099-01-01"],
                 body="opened →「未來的事」", when=_t(9, 0))
    append_event(spine, "collision", "hotkey", ["id:x-a"],
                 body="opened\n輸入：今天剛丟的", when=_t(9, 5))
    assert lint(spine) == []


def test_lint_broken_ref(spine):
    append_event(spine, "chosen", "monitor", ["ref:presented:09:15"],
                 body="ref 指到不存在的事件", when=_t(10, 0))
    reds = lint(spine)
    assert len(reds) == 1 and "ref 斷鏈" in reds[0]


def test_lint_overdue_loop_and_stale_collision(spine):
    yesterday = dt.datetime.now() - dt.timedelta(days=1)
    append_event(spine, "open-loop", "hotkey", ["#7", "due:2020-01-01"],
                 body="opened →「早該做的」", when=_t(9, 0))
    append_event(spine, "collision", "hotkey", ["id:old-a"],
                 body="opened\n輸入：昨天丟的想法",
                 when=yesterday.replace(hour=9, minute=0))
    reds = "\n".join(lint(spine))
    assert "open-loop 逾期" in reds and "#7" in reds
    assert "碰撞無回程" in reds and "old-a" in reds
    # 判定回來後（同 id 的非 opened 事件）就不再紅
    append_event(spine, "collision", "engine", ["id:old-a", "ref:collision:old-a"],
                 body="判定：真增量", when=_t(10, 0))
    assert "碰撞無回程" not in "\n".join(lint(spine))


def test_lint_dead_letter_backlog(spine):
    append_event(spine, "bad-type", "hotkey", body="拒寫原文", dead_letter=True)
    reds = lint(spine)
    assert any("dead-letter 積壓" in r and "1 件" in r for r in reds)
