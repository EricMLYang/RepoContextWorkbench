"""P1 registry（register/edit/query/audit）＋ P2 組（同檔 groups 段）。

registry.yaml 就地更新——寫入必經同一把 lock（撕裂寫入比 append 更可怕）。
audit 抄 render_registry.py 稽核思路：tier 漂移、路徑失效、paused 無 resume_when、未登記 repo。

關係層（2026-09-08 組為單位輪）：關係存在**來源 repo** 的 `relations:` 欄（`{to, kind, note?}`，v1 §3.1 草案落地）。
詞彙表是起手式（用滿四週在週復盤修），私有層可用 config `relations.kinds` 擴充：
    relations:
      kinds:
        tests: {forward: 驗證, inverse: 被驗證於}
forward＝從來源 repo 讀的說法（A 規劃（PM）B）、inverse＝從目標 repo 讀的說法（B 的 PM 是 A）；
symmetric＝對稱關係兩向同字。
"""
import subprocess
import time
from pathlib import Path

import yaml

from . import config as _config
from .locking import FileLock
from .spine import LOCK_NAME

TIERS = ("active", "paused", "dormant", "archived")
RELATION_KINDS = {
    "pm-of":        {"forward": "規劃（PM）", "inverse": "PM 是"},
    "feeds":        {"forward": "供料給", "inverse": "吃料自"},
    "derived-from": {"forward": "衍生自", "inverse": "衍生出"},
    "upstream-of":  {"forward": "是其上游", "inverse": "上游是"},
    "sibling-topic": {"forward": "同主題", "inverse": "同主題", "symmetric": True},
}


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


def tag_repo(spine_dir, id, add=None, remove=None):
    """P1 tag 操作（2026-09-01 使用者需求：分群以外要能很容易下 tag）。
    回傳更新後的 tags；重複 add 冪等、remove 不存在的 tag 靜默略過。"""
    data = load(spine_dir)
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
            save(spine_dir, data)
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
    data = load(spine_dir)
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
    """回傳 (組名, [repo entries])。repos 給定＝臨時組合免建組（P2）；
    name=None 且無 repos＝全部（工作台的（全部）範圍——L4 bug：以前這裡直接炸）。"""
    data = load(spine_dir)
    if repos:
        ids = [r.strip() for r in repos.split(",")] if isinstance(repos, str) else repos
        return f"臨時({','.join(ids)})", [get_repo(spine_dir, i) for i in ids]
    if name is None:
        return "全部", list(data["repos"])
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


# ── 關係層 ──────────────────────────────────────────────────────────────

def relation_kinds(spine_dir):
    """內建詞彙 ∪ config `relations.kinds`（私有層擴充；同名以 config 為準）。"""
    kinds = {k: dict(v) for k, v in RELATION_KINDS.items()}
    extra = (_config.load(spine_dir).get("relations") or {}).get("kinds") or {}
    for k, v in extra.items():
        if isinstance(v, dict):
            kinds[k] = {"forward": str(v.get("forward") or k),
                        "inverse": str(v.get("inverse") or v.get("forward") or k),
                        "symmetric": bool(v.get("symmetric"))}
    return kinds


def relate(spine_dir, a, b, kind, note=None):
    """設關係 a --kind--> b（例：a pm-of b＝a 是 b 的 PM）。同 (a,b,kind) 冪等（note 取最新）。"""
    kinds = relation_kinds(spine_dir)
    if kind not in kinds:
        raise ValueError(f"未知 kind: {kind}（可用：{', '.join(kinds)}；config relations.kinds 可擴充）")
    if a == b:
        raise ValueError("repo 不能跟自己建關係")
    data = load(spine_dir)
    ids = {r["id"] for r in data["repos"]}
    for x in (a, b):
        if x not in ids:
            raise ValueError(f"repo 不存在: {x}")
    src = next(r for r in data["repos"] if r["id"] == a)
    rels = src.setdefault("relations", [])
    entry = next((x for x in rels if x.get("to") == b and x.get("kind") == kind), None)
    if entry is None:
        entry = {"to": b, "kind": kind}
        rels.append(entry)
    if note:
        entry["note"] = str(note)
    elif "note" in entry and note is None:
        pass  # 沒給 note 不動舊註
    save(spine_dir, data)
    return dict(entry)


def unrelate(spine_dir, a, b, kind=None):
    """刪關係；kind=None＝刪 a→b 全部。回傳刪了幾筆。"""
    data = load(spine_dir)
    src = next((r for r in data["repos"] if r["id"] == a), None)
    if src is None:
        raise ValueError(f"repo 不存在: {a}")
    before = list(src.get("relations") or [])
    keep = [x for x in before
            if not (x.get("to") == b and (kind is None or x.get("kind") == kind))]
    if keep:
        src["relations"] = keep
    else:
        src.pop("relations", None)
    save(spine_dir, data)
    return len(before) - len(keep)


def relations(spine_dir, id=None):
    """全部（或碰到 id 的）有向關係 [{from, to, kind, note?}]，依 registry 順序。"""
    out = []
    for r in load(spine_dir)["repos"]:
        for x in r.get("relations") or []:
            if id is None or r["id"] == id or x.get("to") == id:
                row = {"from": r["id"], "to": x.get("to"), "kind": x.get("kind")}
                if x.get("note"):
                    row["note"] = x["note"]
                out.append(row)
    return out


def relations_of(spine_dir, id):
    """站在 id 的立場讀關係：[{peer, kind, direction: out|in, label, note?}]。
    out＝我指向別人（用 forward 字）；in＝別人指向我（用 inverse 字）。"""
    kinds = relation_kinds(spine_dir)
    out = []
    for rel in relations(spine_dir, id):
        k = kinds.get(rel["kind"], {"forward": rel["kind"], "inverse": rel["kind"]})
        if rel["from"] == id:
            row = {"peer": rel["to"], "kind": rel["kind"], "direction": "out",
                   "label": k["forward"]}
        else:
            row = {"peer": rel["from"], "kind": rel["kind"], "direction": "in",
                   "label": k["inverse"]}
        if rel.get("note"):
            row["note"] = rel["note"]
        out.append(row)
    return out


def related_ids(spine_dir, ids):
    """一跳鄰居（兩向），不含自己——臨時組〔＋相關 repo〕與「挑」的建議模式用。"""
    ids = set(ids)
    out = set()
    for rel in relations(spine_dir):
        if rel["from"] in ids:
            out.add(rel["to"])
        if rel["to"] in ids:
            out.add(rel["from"])
    return out - ids


def relation_lines(spine_dir, ids):
    """人話行（組情境卡用）：組內兩兩關係＋指向組外的關係（標「組外」）。"""
    ids = list(ids)
    kinds = relation_kinds(spine_dir)
    lines = []
    for rel in relations(spine_dir):
        if rel["from"] not in ids and rel["to"] not in ids:
            continue
        k = kinds.get(rel["kind"], {"forward": rel["kind"]})
        note = f"（{rel['note']}）" if rel.get("note") else ""
        outside = ""
        if rel["from"] not in ids:
            outside = f"　←（{rel['from']} 在組外）"
        elif rel["to"] not in ids:
            outside = f"　→（{rel['to']} 在組外）"
        lines.append(f"{rel['from']} {k['forward']}→ {rel['to']}{note}{outside}")
    return lines


def scan_dir(spine_dir, base_dir):
    """mani「init 自動掃描」課（拆機報告 §2.2）：找 base_dir 下含 .git 的資料夾，
    回傳未登記候選 [(path, 建議id)]。id 撞名時加序號（同名資料夾登在別處的情況）。"""
    data = load(spine_dir)
    known_paths = set()
    for r in data["repos"]:
        p = Path(r["path"]).expanduser()
        known_paths.add(p.resolve() if p.exists() else p)
    used_ids = {r["id"] for r in data["repos"]}
    out = []
    base = Path(base_dir).expanduser()
    if not base.is_dir():
        raise ValueError(f"目錄不存在: {base_dir}")
    for child in sorted(base.iterdir()):
        if not (child.is_dir() and (child / ".git").exists()):
            continue
        if child.resolve() in known_paths:
            continue
        cid, n = child.name, 2
        while cid in used_ids:
            cid = f"{child.name}-{n}"
            n += 1
        used_ids.add(cid)
        out.append((child, cid))
    return out


def scan_register(spine_dir, base_dir):
    """混合模式的「自動下半身」：掃描到的 repo 以預設值登記（mine/active/無 tag）；
    上半身（type/tier/tags/血統/關係）留人手寫——狀態欄位機器填，語意我填。"""
    added = []
    for path, cid in scan_dir(spine_dir, base_dir):
        add_repo(spine_dir, cid, path)
        added.append(cid)
    return added


def audit(spine_dir, scan_dirs=None, active_max_days=30, dormant_min_days=7):
    """紅字清單。scan_dirs：額外掃「資料夾在但 registry 沒有」的洞。
    反向漂移（2026-09-08 真資料實錘：MI 線 PM repo 掛 active 沉默 66 天、code repo 掛 dormant
    卻 7 天 4 commits）：dormant 但 dormant_min_days 內有 commit 也算紅字——活動才是真相。"""
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
        if r.get("tier") == "dormant" and r.get("type") == "mine":
            days = last_commit_days(p)
            if days is not None and days <= dormant_min_days:
                reds.append(f"[tier 漂移] {r['id']}: dormant 但 {days:.0f} 天前有 commit（該升回 active？）")
        if r.get("type") == "external" and not r.get("upstream"):
            reds.append(f"[external 無 upstream] {r['id']}：P4 無從查起")
    ids = {r["id"] for r in data["repos"]}
    for r in data["repos"]:
        for x in r.get("relations") or []:
            if x.get("to") not in ids:
                reds.append(f"[關係斷鏈] {r['id']} → {x.get('to')} ({x.get('kind')})：目標未登記")
    for d in (scan_dirs or []):
        base = Path(d).expanduser()
        if not base.is_dir():
            continue
        for child in base.iterdir():
            if child.is_dir() and (child / ".git").exists() \
                    and child.resolve() not in known_paths:
                reds.append(f"[未登記] {child}: 是 git repo 但 registry 沒有")
    return reds
