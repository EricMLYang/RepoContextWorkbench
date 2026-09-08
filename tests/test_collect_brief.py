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


def test_collect_worktrees(spine_with_repos, tmp_path):
    """gitpane 欄位課：worktree 數（agent 並行開發的訊號）。"""
    import subprocess
    from pathlib import Path
    ra = Path(registry.get_repo(spine_with_repos, "repo-a")["path"])
    subprocess.run(["git", "-C", str(ra), "worktree", "add", "-q",
                    str(tmp_path / "wt-a"), "-b", "wt-branch"],
                   check=True, capture_output=True)
    st = collect.collect_repo(registry.get_repo(spine_with_repos, "repo-a"))
    assert st["worktrees"] == 1
    st_b = collect.collect_repo(registry.get_repo(spine_with_repos, "repo-b"))
    assert st_b["worktrees"] == 0


def test_collect_agent_detection(spine_with_repos):
    """gitpane 課的 agent 偵測：存活會話標在 cwd 所屬 repo；pid 死了的殘骸自動清。"""
    import os
    import subprocess
    import sys
    from repoengine import agentmark
    ra = registry.get_repo(spine_with_repos, "repo-a")["path"]
    # 活會話（用自己的 pid＝必活）
    agentmark.mark(spine_with_repos, "t1", ra, "claude", os.getpid())
    # 殘骸：spawn 一個立即結束的進程，拿它的 pid
    p = subprocess.Popen([sys.executable, "-c", "pass"])
    p.wait()
    agentmark.mark(spine_with_repos, "t2", ra, "codex", p.pid)
    _, entries = registry.resolve_group(spine_with_repos, "g1")
    states = {s["id"]: s for s in collect.collect_group(entries, spine_with_repos)}
    assert states["repo-a"]["agents"] == ["claude"]   # 殘骸不算
    assert states["repo-b"]["agents"] == []
    # 殘骸 marker 已被清掉
    assert not (spine_with_repos / ".state" / "agent_sessions" / "t2.json").exists()
    # 不帶 spine_dir（舊呼叫路徑）不標，也不炸
    states2 = {s["id"]: s for s in collect.collect_group(entries)}
    assert states2["repo-a"]["agents"] == []
    agentmark.unmark(spine_with_repos, "t1")


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


def test_question_cards_structured(spine_with_repos):
    """問句改結構化卡：文字版（簡報 md）由卡渲染而來，兩者不會分岔。"""
    from repoengine import config
    states = collect.collect_group(registry.resolve_group(spine_with_repos, "g1")[1])
    th = config.load(spine_with_repos)["thresholds"]
    cards = brief.build_question_cards(states, th)
    assert [c["repo"] for c in cards] == ["repo-b"]
    c = cards[0]
    assert c["qkind"] == "dirty" and "嗎？" in c["title"]
    assert [a["label"] for a in c["actions"]] == ["開 agent 收尾", "標記本週不動"]
    assert c["actions"][0]["action"] == "term_create"
    assert c["actions"][0]["payload"]["repo"] == "repo-b"
    assert c["actions"][1]["action"] == "defer"
    texts = brief.build_questions(states, th)
    assert texts == [brief.card_text(c) for c in cards]
    assert "〔開 agent 收尾〕〔標記本週不動〕" in texts[0]


def test_collect_activity_and_pulse(spine_with_repos, tmp_path):
    """2026-09-08 組為單位輪：監控專注活動頻繁度＋近期 commit 內容。"""
    from pathlib import Path
    from tests.conftest import _run_git
    _, entries = registry.resolve_group(spine_with_repos, "g1")
    ra = Path(entries[0]["path"])
    (ra / "x.md").write_text("x\n", encoding="utf-8")
    _run_git(ra, "add", "-A")
    _run_git(ra, "commit", "-q", "-m", "feat: 加 x 頁")
    states = {s["id"]: s for s in collect.collect_group(entries)}
    a, b = states["repo-a"], states["repo-b"]
    assert a["commits_7d"] == 2 and a["commits_30d"] == 2      # init（1 天前）＋ 今天
    assert b["commits_7d"] == 1 and b["commits_30d"] == 1      # init（6 天前）
    assert a["recent_commits"][0]["subject"] == "feat: 加 x 頁"
    assert set(a["recent_commits"][0]) == {"date", "hash", "subject"}
    pulse = collect.group_pulse(list(states.values()))
    assert pulse["commits_7d"] == 3 and pulse["commits_30d"] == 3
    assert pulse["most_active"] == "repo-a"
    assert pulse["latest"]["repo"] == "repo-a" and "加 x 頁" in pulse["latest"]["subject"]
    assert pulse["quiet_30d"] == []                            # 兩個都有 30 天內 commit
    table = collect.format_table(list(states.values()))
    assert "7d" in table
    # 近期 commit 清單（跨 repo 按日期新→舊）
    recent = collect.recent_across(list(states.values()), limit=10)
    assert recent[0]["repo"] == "repo-a" and recent[0]["subject"] == "feat: 加 x 頁"
    assert len(recent) == 3


def test_brief_has_pulse_line(spine_with_repos):
    _, text = brief.run(spine_with_repos, "g1")
    assert "## 脈動（一行）" in text and "近 7 天" in text
    # 脈動是資訊不是問句：判斷段仍只有 dirty 那一條
    judge = text.split("## 未結")[0]
    assert judge.count("嗎？") == 1
