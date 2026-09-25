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
    # tag 詞彙建議（顯示在 tag 編輯器裡供一鍵點選；registry 既有 tag 會自動併入）
    "tags": {"suggestions": []},
    # P12：排程住殼內（S6）。每項 {task: brief|digest|upstream|ruminate|commit, at: "HH:MM", group: <組名|null>}
    "schedule": [],
    # 反芻（ctx fresh）：window_days＝第一次回看幾天；min_shared＝至少幾個共同少見詞才推；
    # max_per_group＝每組每輪最多推幾筆。命中率（ctx fresh --stats）低就把 min_shared 調高。
    "ruminate": {"window_days": 14, "min_shared": 3, "max_per_group": 3},
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


# ---- 使用者層設定（2026-09-25 Agent 友善輪）----
# agent 從任何 repo 開起來都要找得到 spine：不能每個指令都帶 --spine、也不能靠 cwd。
# 解析順序：--spine ＞ $REPOENGINE_SPINE ＞ 使用者設定檔 ＞ cwd。`ctx setup` 寫這個檔。

def user_config_path():
    import os
    base = os.environ.get("XDG_CONFIG_HOME") or (Path.home() / ".config")
    return Path(base) / "repo-context" / "config.json"


def load_user_config():
    import json
    try:
        return json.loads(user_config_path().read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}


def save_user_config(data):
    import json
    p = user_config_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(data, ensure_ascii=False, indent=1), encoding="utf-8")
    return p


def is_spine(d):
    d = Path(d)
    return (d / "config.yaml").exists() or (d / "registry.yaml").exists()


def find_spine(explicit=None):
    """回傳 spine 路徑或 None（呼叫端決定要報錯還是靜默——hook 要靜默）。"""
    import os
    for cand in (explicit, os.environ.get("REPOENGINE_SPINE"),
                 load_user_config().get("spine"), "."):
        if cand and is_spine(Path(cand).expanduser()):
            return Path(cand).expanduser().resolve()
    return None
