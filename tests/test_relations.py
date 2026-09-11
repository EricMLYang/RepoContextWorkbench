"""L1：P1 關係層（2026-09-08 組為單位輪）——A 是 B 的 PM 這類 repo 間關係要能設、能查、能稽核。"""
from pathlib import Path

import pytest

from repo_context import registry


def test_relate_roundtrip_and_rejects(spine_with_repos):
    d = spine_with_repos
    r = registry.relate(d, "repo-a", "repo-b", "pm-of", note="a 規劃 b")
    assert r == {"to": "repo-b", "kind": "pm-of", "note": "a 規劃 b"}
    # 冪等：同 (a,b,kind) 再設一次不會變兩筆，note 以最新為準
    registry.relate(d, "repo-a", "repo-b", "pm-of", note="改註")
    rels = registry.relations(d)
    assert rels == [{"from": "repo-a", "to": "repo-b", "kind": "pm-of", "note": "改註"}]
    # 不同 kind 可並存
    registry.relate(d, "repo-a", "repo-b", "feeds")
    assert len(registry.relations(d)) == 2
    with pytest.raises(ValueError, match="自己"):
        registry.relate(d, "repo-a", "repo-a", "pm-of")
    with pytest.raises(ValueError, match="不存在"):
        registry.relate(d, "repo-a", "nope", "pm-of")
    with pytest.raises(ValueError, match="kind"):
        registry.relate(d, "repo-a", "repo-b", "boss-of")
    # unrelate：指定 kind 只刪那一筆；不給 kind 全刪
    registry.unrelate(d, "repo-a", "repo-b", "feeds")
    assert [x["kind"] for x in registry.relations(d)] == ["pm-of"]
    # 寫入真的落 registry.yaml（資產是純文字，下一代工具直接讀得到）
    assert "pm-of" in (Path(d) / "registry.yaml").read_text(encoding="utf-8")
    registry.unrelate(d, "repo-a", "repo-b")
    assert registry.relations(d) == []


def test_relations_of_labels_and_neighbors(spine_with_repos, tmp_path):
    from tests.conftest import make_git_repo
    d = spine_with_repos
    registry.add_repo(d, "repo-c", make_git_repo(tmp_path, "repo-c"))
    registry.relate(d, "repo-a", "repo-b", "pm-of")
    registry.relate(d, "repo-c", "repo-a", "feeds")
    registry.relate(d, "repo-b", "repo-c", "sibling-topic")
    at_a = {(x["peer"], x["direction"]): x["label"] for x in registry.relations_of(d, "repo-a")}
    assert at_a[("repo-b", "out")] == "規劃（PM）"     # a → b：a 是 b 的 PM
    assert at_a[("repo-c", "in")] == "吃料自"          # c feeds a：a 吃料自 c
    at_b = {(x["peer"], x["direction"]): x["label"] for x in registry.relations_of(d, "repo-b")}
    assert at_b[("repo-a", "in")] == "PM 是"            # 反向讀法
    assert at_b[("repo-c", "out")] == "同主題"          # 對稱關係兩向同字
    at_c = {(x["peer"], x["direction"]): x["label"] for x in registry.relations_of(d, "repo-c")}
    assert at_c[("repo-b", "in")] == "同主題"
    # 一跳鄰居（兩向），臨時組〔＋相關 repo〕用
    assert registry.related_ids(d, ["repo-a"]) == {"repo-b", "repo-c"}
    assert registry.related_ids(d, ["repo-b"]) == {"repo-a", "repo-c"}
    # 人話行：組情境卡用
    lines = registry.relation_lines(d, ["repo-a", "repo-b"])
    assert any("repo-a" in ln and "規劃（PM）" in ln and "repo-b" in ln for ln in lines)
    assert any("repo-c" in ln and "組外" in ln for ln in lines)   # 指到組外要標明


def test_custom_kind_from_config(spine_with_repos):
    d = spine_with_repos
    (Path(d) / "config.yaml").write_text(
        "relations:\n  kinds:\n    tests: {forward: 驗證, inverse: 被驗證於}\n",
        encoding="utf-8")
    assert "tests" in registry.relation_kinds(d) and "pm-of" in registry.relation_kinds(d)
    registry.relate(d, "repo-b", "repo-a", "tests")
    at_a = {x["peer"]: x["label"] for x in registry.relations_of(d, "repo-a")}
    assert at_a["repo-b"] == "被驗證於"


def test_remove_repo_strips_relations_and_audit_dangling(spine_with_repos):
    d = spine_with_repos
    registry.relate(d, "repo-a", "repo-b", "pm-of")
    registry.remove_repo(d, "repo-b")
    assert registry.relations(d) == []          # 指向被移除 repo 的關係一併清掉
    # 手改 yaml 留下斷鏈 → audit 紅字
    data = registry.load(d)
    data["repos"][0]["relations"] = [{"to": "ghost", "kind": "pm-of"}]
    registry.save(d, data)
    reds = registry.audit(d)
    assert any("關係斷鏈" in r and "ghost" in r for r in reds)
