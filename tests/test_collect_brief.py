"""L1：P3 採集 ＋ 早晨簡報（樣貌 A：問句形狀、三段結構、presented 留痕）。"""
import datetime as dt

from repoengine import brief, collect, registry, spine as spine_mod


def test_collect_states(spine_with_repos):
    _, entries = registry.resolve_group(spine_with_repos, "g1")
    states = {s["id"]: s for s in collect.collect_group(entries)}
    assert states["repo-a"]["dirty"] == 0
    assert states["repo-b"]["dirty"] == 2
    assert states["repo-b"]["dirty_days"] >= 4.5   # fixture 設 5 天
    assert states["repo-a"]["last_commit_days"] is not None
    assert states["repo-a"]["branch"] in ("main", "master")
    assert states["repo-a"]["last_subject"] == "init"
    # fixture repo 無 remote → ahead/behind 查不到＝None（fetch 後才準的 caveat）
    assert states["repo-a"]["ahead"] is None
    table = collect.format_table(collect.collect_group(entries))
    assert "↑未push" in table and "branch" in table


def test_collect_external_skips_dirty(spine, tmp_path):
    from tests.conftest import make_git_repo
    p = make_git_repo(tmp_path, "ext-repo", dirty_files=3)
    registry.add_repo(spine, "ext-repo", p, type="external", upstream="github.com/x/y")
    st = collect.collect_repo(registry.get_repo(spine, "ext-repo"))
    assert st["dirty"] == 0 and "P4" in st["note"]


def test_brief_shape(spine_with_repos):
    # 加一條今天到期的 open loop
    spine_mod.append_event(
        spine_with_repos, "open-loop", "hotkey",
        ["#12", f"due:{dt.date.today():%Y-%m-%d}"],
        body="opened →「驗證 X」", when=dt.datetime.now().replace(hour=8, minute=0))
    out, text = brief.run(spine_with_repos, "g1")
    assert out.exists()
    # 三段固定結構
    assert "## 需要你判斷的" in text
    assert "## 未結" in text
    assert "## 沉默摘要" in text
    # dirty 5 天 >= 閾值 4 → 必須出現且是問句＋出口
    assert "repo-b" in text and "嗎？" in text and "〔" in text
    # 乾淨的 repo-a 不該出現在判斷段（無事不報）
    judge_section = text.split("## 未結")[0]
    assert "repo-a" not in judge_section
    # 到期 loop 有列出
    assert "#12" in text and "今天到期" in text
    # 留痕：presented 事件寫進脊椎
    evs = spine_mod.query(spine_with_repos, type="presented")
    assert len(evs) == 1 and evs[0].source == "morning-brief"


def test_brief_quiet_when_all_clean(spine, tmp_path):
    from tests.conftest import make_git_repo
    p = make_git_repo(tmp_path, "calm-repo", days_old=1)
    registry.add_repo(spine, "calm-repo", p)
    registry.add_group(spine, "calm", ["calm-repo"])
    _, text = brief.run(spine, "calm")
    assert "今天沒有需要判斷的事" in text


def test_brief_over_three_questions_warns(spine, tmp_path):
    from tests.conftest import make_git_repo
    import os, time
    ids = []
    for i in range(4):
        p = make_git_repo(tmp_path, f"noisy-{i}", days_old=3, dirty_files=1)
        old = time.time() - 6 * 86400
        for f in p.glob("wip*.md"):
            os.utime(f, (old, old))
        registry.add_repo(spine, f"noisy-{i}", p)
        ids.append(f"noisy-{i}")
    registry.add_group(spine, "noisy", ids)
    _, text = brief.run(spine, "noisy")
    assert "> 3——過濾閾值該修了" in text   # 免費校準資料要浮出
