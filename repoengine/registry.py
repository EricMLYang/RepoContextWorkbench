"""P1 registry（register/edit/query/audit）＋ P2 組（同檔 groups 段）。

registry.yaml 就地更新——寫入必經同一把 lock（撕裂寫入比 append 更可怕）。
audit 抄 render_registry.py 稽核思路：tier 漂移、路徑失效、paused 無 resume_when、未登記 repo。
"""
import subprocess
import time
from pathlib import Path

import yaml

from .locking import FileLock
from .spine import LOCK_NAME

TIERS = ("active", "paused", "dormant", "archived")
RELATION_KINDS = ("derived-from", "feeds", "sibling-topic", "upstream-of")


def _path(spine_dir):
    return Path(spine_dir) / "registry.yaml"


def load(spine_dir):
    p = _path(spine_dir)
    if not p.exists():
        return {"repos": [], "groups": []}
    data = yaml.safe_load(p.read_text(encoding="utf-8")) or {}
    data.setdefault("repos", [])
    data.setdefault("groups", [])
    return data


def save(spine_dir, data):
    with FileLock(Path(spine_dir) / LOCK_NAME):
        _path(spine_dir).write_text(
            yaml.safe_dump(data, allow_unicode=True, sort_keys=False),
            encoding="utf-8")


def add_repo(spine_dir, id, path, type="mine", tags=None, tier="active",
             upstream=None, origin=None):
    if type not in ("mine", "external"):
        raise ValueError("type 需為 mine|external")
    if tier not in TIERS:
        raise ValueError(f"tier 需為 {'|'.join(TIERS)}")
    data = load(spine_dir)
    if any(r["id"] == id for r in data["repos"]):
        raise ValueError(f"repo id 已存在: {id}")
    entry = {"id": id, "path": str(path), "type": type,
             "tags": tags or [], "tier": tier}
    if upstream:
        entry["upstream"] = upstream
    if origin:
        entry["origin"] = origin
    data["repos"].append(entry)
    save(spine_dir, data)
    return entry


def set_field(spine_dir, id, key, value):
    data = load(spine_dir)
    for r in data["repos"]:
        if r["id"] == id:
            r[key] = value
            save(spine_dir, data)
            return r
    raise ValueError(f"repo 不存在: {id}")


def remove_repo(spine_dir, id):
    """移除 repo 並清掉所有組的 membership（組保留，允許空組）。"""
    data = load(spine_dir)
    if not any(r["id"] == id for r in data["repos"]):
        raise ValueError(f"repo 不存在: {id}")
    data["repos"] = [r for r in data["repos"] if r["id"] != id]
    for g in data["groups"]:
        if id in g["members"]:
            g["members"].remove(id)
    save(spine_dir, data)


def get_repo(spine_dir, id):
    for r in load(spine_dir)["repos"]:
        if r["id"] == id:
            return r
    raise ValueError(f"repo 不存在: {id}")


def add_group(spine_dir, name, members, landing="default"):
    data = load(spine_dir)
    if any(g["name"] == name for g in data["groups"]):
        raise ValueError(f"組已存在: {name}")
    known = {r["id"] for r in data["repos"]}
    unknown = [m for m in members if m not in known]
    if unknown:
        raise ValueError(f"組員未登記: {', '.join(unknown)}")
    data["groups"].append({"name": name, "members": members, "landing": landing})
    save(spine_dir, data)


def resolve_group(spine_dir, name=None, repos=None):
    """回傳 (組名, [repo entries])。repos 給定＝臨時組合免建組（P2）。"""
    data = load(spine_dir)
    if repos:
        ids = [r.strip() for r in repos.split(",")] if isinstance(repos, str) else repos
        return f"臨時({','.join(ids)})", [get_repo(spine_dir, i) for i in ids]
    for g in data["groups"]:
        if g["name"] == name:
            return name, [get_repo(spine_dir, m) for m in g["members"]]
    raise ValueError(f"組不存在: {name}")


def _git(repo_path, *args):
    r = subprocess.run(["git", "-C", str(repo_path)] + list(args),
                       capture_output=True, text=True, encoding="utf-8")
    return r.returncode, (r.stdout or "").strip()


def last_commit_days(repo_path):
    rc, out = _git(repo_path, "log", "-1", "--format=%ct")
    if rc != 0 or not out:
        return None
    return (time.time() - int(out)) / 86400.0


def survival(spine_dir):
    """品味量測第二視圖（P11）：碰撞生出來的 repo（有 origin 血統）還活著幾個。"""
    spawned = [r for r in load(spine_dir)["repos"] if r.get("origin")]
    alive = [r for r in spawned if r.get("tier") in ("active", "paused")]
    n = len(spawned)
    return {"spawned": n, "alive": len(alive),
            "survival_rate": (len(alive) / n) if n else None}


def audit(spine_dir, scan_dirs=None, active_max_days=30):
    """紅字清單。scan_dirs：額外掃「資料夾在但 registry 沒有」的洞。"""
    data = load(spine_dir)
    reds = []
    known_paths = set()
    for r in data["repos"]:
        p = Path(r["path"]).expanduser()
        known_paths.add(p.resolve() if p.exists() else p)
        if not p.is_dir():
            reds.append(f"[路徑失效] {r['id']}: {r['path']} 不存在")
            continue
        if r.get("tier") == "paused" and not r.get("resume_when"):
            reds.append(f"[paused 無 resume_when] {r['id']}：說不出復工條件就不准掛 paused")
        if r.get("tier") == "active" and r.get("type") == "mine":
            days = last_commit_days(p)
            if days is None:
                reds.append(f"[非 git repo] {r['id']}: {r['path']}")
            elif days > active_max_days:
                reds.append(f"[tier 漂移] {r['id']}: active 但 {days:.0f} 天無 commit")
        if r.get("type") == "external" and not r.get("upstream"):
            reds.append(f"[external 無 upstream] {r['id']}：P4 無從查起")
    for d in (scan_dirs or []):
        base = Path(d).expanduser()
        if not base.is_dir():
            continue
        for child in base.iterdir():
            if child.is_dir() and (child / ".git").exists() \
                    and child.resolve() not in known_paths:
                reds.append(f"[未登記] {child}: 是 git repo 但 registry 沒有")
    return reds
