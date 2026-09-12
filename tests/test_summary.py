"""L1：本組工作摘要（2026-09-12 UX 檢討 §3）。

這組測試守的是紀律不是版面：每格要嘛有依據（source），要嘛明講沒有依據；
commit 數只能當活動陳述，不准當成進度；範圍跟情境卡、簡報同一份規則。
"""
import datetime as dt

from repo_context import registry, spine as spine_mod, summary
from tests.conftest import make_git_repo


def _two_groups(spine, tmp_path):
    registry.add_repo(spine, "prod", make_git_repo(tmp_path, "prod", days_old=1))
    registry.add_repo(spine, "notes", make_git_repo(tmp_path, "notes", days_old=1))
    registry.add_group(spine, "產品開發", ["prod"])
    registry.add_group(spine, "知識研究", ["notes"])


def test_summary_每格都帶依據(spine, tmp_path):
    _two_groups(spine, tmp_path)
    registry.set_group_field(spine, "產品開發", "goal", "月底交出可驗收的版本")
    spine_mod.append_event(spine, "decision", "agent", ["group:產品開發"],
                           body="會話結果：比對完驗收條件\n兩邊差三條")
    spine_mod.append_event(
        spine, "open-loop", "hotkey",
        ["#1", "group:產品開發", f"due:{dt.date.today():%Y-%m-%d}"],
        body="補齊缺的三條驗收條件")
    s = summary.build_summary(spine, group="產品開發")
    assert s["goal"]["text"] == "月底交出可驗收的版本"
    assert "registry.yaml" in s["goal"]["source"]
    assert s["progress"]["text"] == "會話結果：比對完驗收條件"
    assert s["progress"]["source"] == "脊椎 decision [agent]" and s["progress"]["at"]
    assert "補齊缺的三條驗收條件" in s["next"]["text"]
    assert s["next"]["source"].startswith("脊椎 open-loop due:")
    assert s["gaps"] == []


def test_summary_沒有依據就直說沒有(spine, tmp_path):
    """資訊不足時直接說明——不拿 commit 數充當進度或價值。"""
    _two_groups(spine, tmp_path)
    s = summary.build_summary(spine, group="知識研究")
    assert s["goal"] is None and "尚未登記這組的目標" in s["goal_gap"]
    assert s["progress"] is None and "脊椎沒有本組的進度紀錄" in s["progress_gap"]
    assert s["next"] is None and s["next_gap"]
    assert len(s["gaps"]) == 3
    # 有 commit，但只以「活動」形式出現在變化欄，不會爬進進度或目標
    assert s["changes"] and all("git log" in c["source"] or "git status" in c["source"]
                                for c in s["changes"])
    assert "沒有上次紀錄" in s["changes_window"]


def test_summary_範圍跟簡報同一份規則(spine, tmp_path):
    """別組的進度與未結不得出現在本組摘要（§6.2 的同一條線）。"""
    _two_groups(spine, tmp_path)
    spine_mod.append_event(spine, "decision", "agent", ["group:產品開發"],
                           body="產品組的判斷")
    spine_mod.append_event(spine, "open-loop", "hotkey",
                           ["#9", "group:產品開發", "due:2099-01-01"],
                           body="產品組的未結")
    s = summary.build_summary(spine, group="知識研究")
    assert s["progress"] is None and s["next"] is None
    assert all("產品組" not in c["text"] for c in s["changes"])
    # 換成產品組就看得到
    p = summary.build_summary(spine, group="產品開發")
    assert p["progress"]["text"] == "產品組的判斷" and "產品組的未結" in p["next"]["text"]


def test_summary_臨時範圍與全部講清楚限制(spine, tmp_path):
    _two_groups(spine, tmp_path)
    adhoc = summary.build_summary(spine, repos=["prod", "notes"])
    assert adhoc["goal"] is None and "存成牌組" in adhoc["goal_gap"]
    everything = summary.build_summary(spine)
    assert everything["group"] == "全部" and "跨組總覽" in everything["goal_gap"]


def test_resume_task_只複述有依據的內容(spine, tmp_path):
    _two_groups(spine, tmp_path)
    empty = summary.build_summary(spine, group="知識研究")["resume_task"]
    assert "脊椎沒有本組的進度紀錄" in empty and "先問我" in empty
    spine_mod.append_event(spine, "decision", "agent", ["group:知識研究"],
                           body="會話結果：整理完三份筆記")
    filled = summary.build_summary(spine, group="知識研究")["resume_task"]
    assert "整理完三份筆記" in filled and "脊椎 decision [agent]" in filled
