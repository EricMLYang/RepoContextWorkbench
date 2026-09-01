"""L1：P1 registry / P2 組 / audit 紅字。"""
import pytest

from repoengine import registry
from tests.conftest import make_git_repo


def test_resolve_group_none_means_all(spine_with_repos):
    """L4 bug 回歸：（全部）範圍＝name=None 且無 repos，必須回全部而不是炸
    「組不存在: None」（工作台預設範圍的 agent 會話/簡報以前都死在這）。"""
    gname, entries = registry.resolve_group(spine_with_repos)
    assert gname == "全部" and {e["id"] for e in entries} == {"repo-a", "repo-b"}


def test_tag_repo_and_all_tags(spine_with_repos):
    tags = registry.tag_repo(spine_with_repos, "repo-a", add=["PM", "開發"])
    assert tags == ["PM", "開發"]
    assert registry.tag_repo(spine_with_repos, "repo-a", add=["PM"]) == ["PM", "開發"]  # 冪等
    registry.tag_repo(spine_with_repos, "repo-b", add=["開發", "Demo"])
    assert registry.all_tags(spine_with_repos) == ["PM", "開發", "Demo"]  # 去重保序
    assert registry.tag_repo(spine_with_repos, "repo-a",
                             remove=["開發", "不存在的"]) == ["PM"]
    with pytest.raises(ValueError, match="repo 不存在"):
        registry.tag_repo(spine_with_repos, "ghost", add=["x"])


def test_add_and_get(spine, tmp_path):
    p = make_git_repo(tmp_path, "repo-x")
    registry.add_repo(spine, "repo-x", p, tags=["dev"])
    r = registry.get_repo(spine, "repo-x")
    assert r["type"] == "mine" and r["tier"] == "active" and r["tags"] == ["dev"]
    with pytest.raises(ValueError):
        registry.add_repo(spine, "repo-x", p)  # id 重複


def test_group_and_adhoc(spine_with_repos):
    name, entries = registry.resolve_group(spine_with_repos, "g1")
    assert name == "g1" and [e["id"] for e in entries] == ["repo-a", "repo-b"]
    name, entries = registry.resolve_group(spine_with_repos, repos="repo-b")
    assert name.startswith("臨時(") and entries[0]["id"] == "repo-b"
    with pytest.raises(ValueError):
        registry.add_group(spine_with_repos, "g2", ["ghost"])  # 組員未登記


def test_audit_red_flags(spine, tmp_path):
    ok = make_git_repo(tmp_path, "ok-repo", days_old=1)
    stale = make_git_repo(tmp_path, "stale-repo", days_old=45)
    registry.add_repo(spine, "ok-repo", ok)
    registry.add_repo(spine, "stale-repo", stale)                     # tier 漂移
    registry.add_repo(spine, "ghost", tmp_path / "no-such-dir")      # 路徑失效
    registry.add_repo(spine, "napper", ok, tier="paused")            # paused 無 resume_when
    registry.add_repo(spine, "ext", ok, type="external")             # external 無 upstream
    reds = registry.audit(spine)
    text = "\n".join(reds)
    assert "tier 漂移" in text and "stale-repo" in text
    assert "路徑失效" in text and "ghost" in text
    assert "resume_when" in text and "napper" in text
    assert "external 無 upstream" in text
    assert "ok-repo" not in text


def test_audit_unregistered_scan(spine, tmp_path):
    make_git_repo(tmp_path / "zone", "secret-repo")
    reds = registry.audit(spine, scan_dirs=[tmp_path / "zone"])
    assert any("未登記" in r and "secret-repo" in r for r in reds)


def test_audit_clean(spine, tmp_path):
    p = make_git_repo(tmp_path, "clean-repo", days_old=2)
    registry.add_repo(spine, "clean-repo", p)
    assert registry.audit(spine) == []


def test_remove_repo_cleans_groups(spine_with_repos):
    registry.remove_repo(spine_with_repos, "repo-b")
    data = registry.load(spine_with_repos)
    assert [r["id"] for r in data["repos"]] == ["repo-a"]
    g1 = next(g for g in data["groups"] if g["name"] == "g1")
    assert g1["members"] == ["repo-a"]  # membership 同步清掉
    with pytest.raises(ValueError):
        registry.remove_repo(spine_with_repos, "ghost")
