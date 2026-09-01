"""P4 上游：fake fetch 注入——新 release 落事件、同一筆不重報（無事不報）、kinds/prerelease 過濾。"""
from pathlib import Path

from repoengine import registry, upstream
from repoengine import spine as spine_mod


def _reg_external(spine_dir, tmp_path, rid="ext-lib", upstream_field="owner/lib"):
    p = tmp_path / rid
    p.mkdir()
    registry.add_repo(spine_dir, rid, p, type="external", upstream=upstream_field)
    return rid


def test_normalize_tolerates_url_forms():
    assert upstream.normalize("owner/lib") == "owner/lib"
    assert upstream.normalize("https://github.com/owner/lib.git") == "owner/lib"
    assert upstream.normalize("github.com/owner/lib/") == "owner/lib"


def test_new_release_event_then_no_repeat(spine, tmp_path):
    rid = _reg_external(spine, tmp_path)
    calls = []

    def fetch(up, kind, pre):
        calls.append((up, kind, pre))
        return {"kind": "release", "id": "v1.2.0", "title": "big", "url": "http://x"}

    n, findings = upstream.check(spine, fetch=fetch)
    assert n == 1 and len(findings) == 1 and findings[0]["repo"] == rid
    assert calls == [("owner/lib", "release", False)]  # 預設 kinds=[release]、無 prerelease
    evs = [e for e in spine_mod.iter_events(spine) if e.source == "upstream"]
    assert len(evs) == 1 and evs[0].type == "suggestion"
    assert "v1.2.0" in evs[0].body and evs[0].kv("repo") == rid
    # 同一筆不重報（洪流不進未讀）
    n2, findings2 = upstream.check(spine, fetch=fetch)
    assert n2 == 1 and findings2 == []
    assert len([e for e in spine_mod.iter_events(spine) if e.source == "upstream"]) == 1


def test_kinds_config_drives_fetch(spine, tmp_path):
    _reg_external(spine, tmp_path)
    (Path(spine) / "config.yaml").write_text(
        "thresholds:\n  upstream:\n    kinds: [release, commit]\n    prerelease: true\n",
        encoding="utf-8")
    seen_kinds = []

    def fetch(up, kind, pre):
        seen_kinds.append((kind, pre))
        return None  # 查無 → 不落事件

    n, findings = upstream.check(spine, fetch=fetch)
    assert seen_kinds == [("release", True), ("commit", True)]
    assert findings == []


def test_no_external_repos(spine_with_repos):
    n, findings = upstream.check(spine_with_repos)  # g 內全是 mine
    assert n == 0 and findings == []
