"""L1：組情境卡（P16 帶料升級）——預設 agent 以「一組 repo」為工作單位的那份開場料。"""
import datetime as dt
from pathlib import Path

from repo_context import context, registry, spine as spine_mod


def test_context_card_sections(spine_with_repos):
    d = spine_with_repos
    registry.relate(d, "repo-a", "repo-b", "pm-of")
    now = dt.datetime.now()
    spine_mod.append_event(d, "open-loop", "hotkey", ["#7", "group:g1", "due:2099-01-01"],
                           body="opened →「驗證 Y」", when=now - dt.timedelta(minutes=5))
    spine_mod.append_event(d, "decision", "manual", ["group:g1"],
                           body="決定先做 a", when=now - dt.timedelta(minutes=4))
    spine_mod.append_event(d, "decision", "manual", ["group:other"],
                           body="別組的事", when=now - dt.timedelta(minutes=3))
    text = context.build_context(d, group="g1")
    assert text.startswith("# 組情境卡 g1")
    for sec in ("## 成員與角色", "## 脈動", "## 未結", "## 最近事件", "## 你的角色"):
        assert sec in text
    # 成員列帶 tier／關係讀法／近 7 天 commit 數
    assert "repo-a" in text and "規劃（PM）→ repo-b" in text and "PM 是 repo-a" in text
    assert "近 7 天" in text
    # 本組 loops 與事件進來，別組的不進
    assert "#7" in text and "驗證 Y" in text
    assert "決定先做 a" in text and "別組的事" not in text
    # 角色說明是通用措辭：講「組管家」，並點名留痕工具
    assert "組管家" in text and "log_decision" in text and "handoff" in text


def test_context_role_prompt_override(spine_with_repos):
    d = spine_with_repos
    (Path(d) / "config.yaml").write_text(
        "session:\n  role_prompt: |\n    你是我的私人助理，名字叫小助。\n", encoding="utf-8")
    text = context.build_context(d, group="g1")
    assert "小助" in text and "組管家" not in text


def test_context_adhoc_scope(spine_with_repos):
    text = context.build_context(spine_with_repos, repos="repo-b")
    members = text.split("## 脈動")[0]
    assert "repo-b" in members and "- repo-a" not in members
