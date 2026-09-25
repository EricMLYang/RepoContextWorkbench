"""不變式 4：跨行程 lock——多進程同時寫脊椎＋registry，事後全部完好。

用真實 subprocess（不是 thread）模擬 P16 多視窗 agent 併發寫入的常態。
"""
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import yaml

from repo_context import registry, spine as spine_mod

ROOT = Path(__file__).resolve().parents[1]

WRITER = """
import sys
sys.path.insert(0, r"{root}")
from repo_context import spine, registry
d = r"{spine}"
who = sys.argv[1]
for i in range(8):
    spine.append_event(d, "decision", "manual", [f"#{{int(who)*100+i}}"],
                       body=f"writer {{who}} event {{i}}")
registry.set_field(d, "repo-a", "tags", ["w" + who])
print("done", who)
"""


def test_inv_concurrent_writers(spine_with_repos, tmp_path):
    script = tmp_path / "writer.py"
    script.write_text(WRITER.format(root=ROOT, spine=spine_with_repos),
                      encoding="utf-8")

    def run(who):
        return subprocess.run([sys.executable, str(script), str(who)],
                              capture_output=True, text=True, timeout=120)

    with ThreadPoolExecutor(4) as ex:
        results = list(ex.map(run, [1, 2, 3, 4]))
    for r in results:
        assert r.returncode == 0, r.stderr

    # 脊椎：4 進程 × 8 筆全數寫入，且全部 parse 得回來（沒有撕裂行）
    evs = spine_mod.query(spine_with_repos, type="decision")
    assert len(evs) == 32
    nums = {t for e in evs for t in e.tokens if t.startswith("#")}
    assert len(nums) == 32

    # registry：仍是合法 YAML、結構完整（就地更新沒被撕裂）
    data = yaml.safe_load(
        (spine_with_repos / "registry.yaml").read_text(encoding="utf-8"))
    assert {r["id"] for r in data["repos"]} == {"repo-a", "repo-b"}
    assert registry.get_repo(spine_with_repos, "repo-a")["tags"][0].startswith("w")


TAGGER = """
import sys
sys.path.insert(0, r"{root}")
from repo_context import registry
who = sys.argv[1]
for i in range(10):
    registry.tag_repo(r"{spine}", "repo-a", add=[f"w{{who}}-{{i}}"])
"""


def test_inv_registry_no_lost_updates(spine_with_repos, tmp_path):
    """不變式 4 補強（2026-09-25 架構檢查實測：2 進程各寫 40 筆只剩 41）：
    registry 讀–改–寫要在同一把 lock 裡，否則後寫的蓋掉先寫的——YAML 沒撕裂但資料掉了。"""
    script = tmp_path / "tagger.py"
    script.write_text(TAGGER.format(root=ROOT, spine=spine_with_repos), encoding="utf-8")
    with ThreadPoolExecutor(4) as ex:
        results = list(ex.map(lambda w: subprocess.run(
            [sys.executable, str(script), str(w)], capture_output=True, text=True,
            timeout=120), [1, 2, 3, 4]))
    for r in results:
        assert r.returncode == 0, r.stderr
    tags = registry.get_repo(spine_with_repos, "repo-a")["tags"]
    assert len([t for t in tags if t.startswith("w")]) == 40


def test_registry_write_is_atomic(spine_with_repos, monkeypatch):
    """寫到一半炸掉 → 原檔完好（先寫暫存檔再 os.replace），讀的人不會讀到半份 YAML。"""
    before = (spine_with_repos / "registry.yaml").read_text(encoding="utf-8")

    def boom(*a, **k):
        raise RuntimeError("disk full")
    monkeypatch.setattr(yaml, "safe_dump", boom)
    import pytest
    with pytest.raises(RuntimeError):
        registry.tag_repo(spine_with_repos, "repo-a", add=["x"])
    assert (spine_with_repos / "registry.yaml").read_text(encoding="utf-8") == before


def test_registry_transaction_rolls_back_on_error(spine_with_repos):
    import pytest
    with pytest.raises(ValueError):
        with registry.transaction(spine_with_repos) as data:
            data["repos"].clear()
            raise ValueError("中途失敗")
    assert len(registry.load(spine_with_repos)["repos"]) == 2
