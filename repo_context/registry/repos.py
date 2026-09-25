"""repo 登記：新增／改欄位／tag／移除／存活率。"""
from .store import load, transaction


TIERS = ("active", "paused", "dormant", "archived")


def add_repo(spine_dir, id, path, type="mine", tags=None, tier="active",
             upstream=None, origin=None):
    if type not in ("mine", "external"):
        raise ValueError("type 需為 mine|external")
    if tier not in TIERS:
        raise ValueError(f"tier 需為 {'|'.join(TIERS)}")
    with transaction(spine_dir) as data:
        if any(r["id"] == id for r in data["repos"]):
            raise ValueError(f"repo id 已存在: {id}")
        entry = {"id": id, "path": str(path), "type": type,
                 "tags": tags or [], "tier": tier}
        if upstream:
            entry["upstream"] = upstream
        if origin:
            entry["origin"] = origin
        data["repos"].append(entry)
        return entry


def set_field(spine_dir, id, key, value):
    with transaction(spine_dir) as data:
        for r in data["repos"]:
            if r["id"] == id:
                r[key] = value
                return r
        raise ValueError(f"repo 不存在: {id}")


def tag_repo(spine_dir, id, add=None, remove=None):
    """P1 tag 操作（2026-09-01 使用者需求：分群以外要能很容易下 tag）。
    回傳更新後的 tags；重複 add 冪等、remove 不存在的 tag 靜默略過。"""
    with transaction(spine_dir) as data:
        for r in data["repos"]:
            if r["id"] == id:
                tags = list(r.get("tags") or [])
                for t in (add or []):
                    t = str(t).strip()
                    if t and t not in tags:
                        tags.append(t)
                for t in (remove or []):
                    if t in tags:
                        tags.remove(t)
                r["tags"] = tags
                return tags
        raise ValueError(f"repo 不存在: {id}")


def all_tags(spine_dir):
    """registry 內既有 tag 的去重清單（保持首次出現順序）。"""
    out = []
    for r in load(spine_dir)["repos"]:
        for t in r.get("tags") or []:
            if t not in out:
                out.append(t)
    return out


def remove_repo(spine_dir, id):
    """移除 repo 並清掉所有組的 membership（組保留，允許空組）。"""
    with transaction(spine_dir) as data:
        if not any(r["id"] == id for r in data["repos"]):
            raise ValueError(f"repo 不存在: {id}")
        data["repos"] = [r for r in data["repos"] if r["id"] != id]
        for r in data["repos"]:
            if r.get("relations"):
                r["relations"] = [x for x in r["relations"] if x.get("to") != id]
                if not r["relations"]:
                    del r["relations"]
        for g in data["groups"]:
            if id in g["members"]:
                g["members"].remove(id)


def get_repo(spine_dir, id):
    for r in load(spine_dir)["repos"]:
        if r["id"] == id:
            return r
    raise ValueError(f"repo 不存在: {id}")


def survival(spine_dir):
    """品味量測第二視圖（P11）：碰撞生出來的 repo（有 origin 血統）還活著幾個。"""
    spawned = [r for r in load(spine_dir)["repos"] if r.get("origin")]
    alive = [r for r in spawned if r.get("tier") in ("active", "paused")]
    n = len(spawned)
    return {"spawned": n, "alive": len(alive),
            "survival_rate": (len(alive) / n) if n else None}
