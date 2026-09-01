"""config.yaml 載入與預設值（v2 §3.4 閾值 schema）。公私切分線：私有內容全在這。"""
import copy
from pathlib import Path

import yaml

DEFAULTS = {
    "thresholds": {
        "dirty_stale_days": 4,
        "inactive_days": 21,
        "upstream": {"kinds": ["release"], "prerelease": False},
        "openloop_default_due_days": 14,
    },
    "interrupt": ["irreversible-confirm", "costly-anomaly", "system-unsure"],
    "provider": {
        "default": "mock",
        "models": {"digest": "haiku", "collide": "strong"},
    },
    "pack": {"token_budget": 60000},
}


def _merge(base, over):
    out = copy.deepcopy(base)
    for k, v in (over or {}).items():
        if isinstance(v, dict) and isinstance(out.get(k), dict):
            out[k] = _merge(out[k], v)
        else:
            out[k] = v
    return out


def load(spine_dir):
    p = Path(spine_dir) / "config.yaml"
    data = {}
    if p.exists():
        data = yaml.safe_load(p.read_text(encoding="utf-8")) or {}
    return _merge(DEFAULTS, data)
