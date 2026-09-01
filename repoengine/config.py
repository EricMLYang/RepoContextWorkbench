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
    # P16 會話可開的 agent CLI（terminal 只負責 spawn，帶料 prompt 走 positional）。
    # mcp: "mcp-config"＝支援 --mcp-config 掛本引擎（v1 只保證 Claude Code）；
    #      "cwd"＝靠 cwd 設定檔自動掛載；"none"＝該家 MCP 設定自理，引擎不代管。
    # prompt_flag: prompt 前置旗標（如某家要 ["-i"]），預設空＝直接接在指令後。
    "agents": {
        "claude": {"cmd": ["claude"], "mcp": "mcp-config", "prompt_flag": []},
    },
    "pack": {"token_budget": 60000},
    # P12：排程住殼內（S6）。每項 {task: brief|digest|upstream|commit, at: "HH:MM", group: <組名|null>}
    "schedule": [],
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
