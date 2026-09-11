"""不變式 5：引擎原始碼不得含私有專名（引擎/私有分離鐵律的可證偽版）。"""
from pathlib import Path

FORBIDDEN = ["Eric", "PM_Head", "AgentCodingPM", "1806011"]


def test_inv_no_private_names_in_engine():
    src = Path(__file__).resolve().parents[1] / "repo_context"
    hits = []
    for f in src.rglob("*.py"):
        text = f.read_text(encoding="utf-8")
        for word in FORBIDDEN:
            if word in text:
                hits.append(f"{f.name}: {word}")
    assert not hits, f"引擎裡出現私有專名: {hits}"
