"""關係層與 exports（repo 間有向關係、repo 對外提供什麼）。"""
from pathlib import Path

from .. import config as _config
from .store import load, transaction


RELATION_KINDS = {
    "pm-of":        {"forward": "規劃（PM）", "inverse": "PM 是"},
    "feeds":        {"forward": "供料給", "inverse": "吃料自"},
    "derived-from": {"forward": "衍生自", "inverse": "衍生出"},
    "upstream-of":  {"forward": "是其上游", "inverse": "上游是"},
    "sibling-topic": {"forward": "同主題", "inverse": "同主題", "symmetric": True},
}


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


def relate(spine_dir, a, b, kind, note=None, exports=None):
    """設關係 a --kind--> b（例：a pm-of b＝a 是 b 的 PM）。同 (a,b,kind) 冪等（note 取最新）。
    exports＝這條關係實際用到 a 的哪幾個 export（例 feeds 關係只吃「書摘」）；給了就覆寫。"""
    kinds = relation_kinds(spine_dir)
    if kind not in kinds:
        raise ValueError(f"未知 kind: {kind}（可用：{', '.join(kinds)}；config relations.kinds 可擴充）")
    if a == b:
        raise ValueError("repo 不能跟自己建關係")
    with transaction(spine_dir) as data:
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
        if exports:
            names = [exports] if isinstance(exports, str) else list(exports)
            names = [n.strip() for x in names for n in str(x).split(",") if n.strip()]
            known = src.get("exports") or {}
            unknown = [n for n in names if n not in known]
            if unknown:
                raise ValueError(f"{a} 沒有 export: {', '.join(unknown)}"
                                 f"（先 `registry export {a} <名稱> <路徑>`；現有：{', '.join(known) or '無'}）")
            entry["exports"] = names
        return dict(entry)


# ── exports：repo 對外提供什麼（2026-09-25 跨 repo 參考輪）─────────────────
# 像 repo 的 API：`<repo>:書摘` 指向 `021_Ebook/`。agent 用名字引用、ctx resolve 換路徑，
# repo 搬家只改 registry 一處；AGENTS.md 裡寫死的 `../x/y` 會斷，名字不會。

def set_export(spine_dir, id, name, path, desc=None):
    """登記（或覆寫）一個 export。path 相對 repo 根目錄，必須存在、不能跳出 repo。"""
    name = str(name).strip()
    if not name or any(c in name for c in ":/ "):
        raise ValueError(f"export 名稱不能空白、不能含 ':' '/' 或空白：{name!r}")
    with transaction(spine_dir) as data:
        r = next((x for x in data["repos"] if x["id"] == id), None)
        if r is None:
            raise ValueError(f"repo 不存在: {id}")
        root = Path(r["path"]).expanduser().resolve()
        rel = str(path).strip().strip("/") or "."
        target = (root / rel).resolve()
        if target != root and root not in target.parents:
            raise ValueError(f"export 路徑跳出 repo：{path}")
        if not target.exists():
            raise ValueError(f"export 路徑不存在：{root / rel}")
        entry = {"path": rel + ("/" if target.is_dir() and rel != "." else "")}
        if desc:
            entry["desc"] = str(desc)
        r.setdefault("exports", {})[name] = entry
        return {"repo": id, "name": name, **entry}


def remove_export(spine_dir, id, name):
    """刪 export；同時從引用它的關係 exports 清單拿掉。回傳是否真的刪了。"""
    with transaction(spine_dir) as data:
        r = next((x for x in data["repos"] if x["id"] == id), None)
        if r is None:
            raise ValueError(f"repo 不存在: {id}")
        ex = r.get("exports") or {}
        if name not in ex:
            return False
        del ex[name]
        if not ex:
            r.pop("exports", None)
        for x in r.get("relations") or []:
            if name in (x.get("exports") or []):
                x["exports"] = [n for n in x["exports"] if n != name]
                if not x["exports"]:
                    del x["exports"]
        return True


def exports_of(entry):
    """registry entry → {名稱: {path, desc?}}（沒有＝空 dict）。"""
    return dict(entry.get("exports") or {})


def unrelate(spine_dir, a, b, kind=None):
    """刪關係；kind=None＝刪 a→b 全部。回傳刪了幾筆。"""
    with transaction(spine_dir) as data:
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
                if x.get("exports"):
                    row["exports"] = list(x["exports"])
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
        if rel.get("exports"):
            row["exports"] = rel["exports"]
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
