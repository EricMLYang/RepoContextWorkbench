"""L3：真 agent 冒煙（人觸發）。需要已登入的 claude CLI。

跑法：python -m pytest tests -m agent -q
預設 skip：沒設 REPOENGINE_RUN_AGENT_TESTS=1 就不跑（不在 L1/L2 消耗額度）。
"""
import os
import shutil

import pytest

from repo_context import collide, spine as spine_mod

pytestmark = pytest.mark.agent


@pytest.mark.skipif(
    not os.environ.get("REPOENGINE_RUN_AGENT_TESTS")
    or not shutil.which("claude"),
    reason="需 REPOENGINE_RUN_AGENT_TESTS=1 且 claude CLI 在 PATH")
def test_real_collide_smoke(spine_with_repos):
    cid = collide.submit(spine_with_repos,
                         "想把快取策略改成 LRU（組內筆記已有相同結論）", group="g1")
    j = collide.run_judgement(spine_with_repos, cid, provider="claude")
    assert j is not None, "真 agent 判定失敗——看 .agent_logs/ 最新一筆 raw log"
    assert j["判定"] in ("已知", "衝突", "真增量")
    assert (spine_with_repos / ".agent_logs").glob("*.json")
    results = [e for e in spine_mod.query(spine_with_repos, type="collision")
               if e.kv("ref") == f"collision:{cid}"]
    assert len(results) == 1
