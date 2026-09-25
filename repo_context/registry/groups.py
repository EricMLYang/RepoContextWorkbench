"""組（P2）：建組、改組（加減組員／改名／刪組）、組層欄位、範圍解析。"""
from pathlib import Path

from .. import config as _config
from .repos import get_repo
from .store import load, transaction


def add_group(spine_dir, name, members, landing="default"):
    with transaction(spine_dir) as data:
        if any(g["name"] == name for g in data["groups"]):
            raise ValueError(f"組已存在: {name}")
        known = {r["id"] for r in data["repos"]}
        unknown = [m for m in members if m not in known]
        if unknown:
            raise ValueError(f"組員未登記: {', '.join(unknown)}")
        data["groups"].append({"name": name, "members": members, "landing": landing})


def get_group(spine_dir, name):
    for g in load(spine_dir)["groups"]:
        if g["name"] == name:
            return g
    raise ValueError(f"組不存在: {name}")


def set_group_field(spine_dir, name, key, value):
    """組層欄位就地更新（目前用於 `goal`：使用者一句話的工作目標）。
    值為空＝刪掉這個欄位（2026-09-12 §3：目標只有人能給，沒給就誠實說沒有）。"""
    if key not in ("goal", "goal_at", "landing"):
        raise ValueError(f"不支援的組欄位: {key}")
    with transaction(spine_dir) as data:
        for g in data["groups"]:
            if g["name"] == name:
                if value in (None, ""):
                    g.pop(key, None)
                else:
                    g[key] = value
                return g
        raise ValueError(f"組不存在: {name}")


# ---- 組的維護（2026-09-25 使用者回報：組只能建、不能改）----
# 組員異動只動 registry；repo 登記本身不受影響（整個移出 registry 是 remove_repo 的事）。
# 改名／刪組牽涉 spine 的 groups/<組>/（brief、materials、digest）：改名跟著搬，
# 刪組保留（那是產出的料，不是設定）。config 排程指到這組的只回報、不代改（人寫的檔）。

def _find_group(data, name):
    for g in data["groups"]:
        if g["name"] == name:
            return g
    raise ValueError(f"組不存在: {name}")


def _check_members(data, members):
    known = {r["id"] for r in data["repos"]}
    unknown = [m for m in members if m not in known]
    if unknown:
        raise ValueError(f"組員未登記: {', '.join(unknown)}")


def _dedupe(ids):
    out = []
    for i in ids:
        i = str(i).strip()
        if i and i not in out:
            out.append(i)
    return out


def update_group(spine_dir, name, add=None, remove=None, members=None, rename=None):
    """一次改組（同一個 transaction：任何一步失敗＝全部不寫）。
    members＝整份換掉（工作台勾選結果）；add／remove＝增減；rename＝改名（groups/<組>/ 跟著搬）。
    回傳 {group, added, removed, renamed_from, moved_dir, schedule_refs}。"""
    with transaction(spine_dir) as data:
        g = _find_group(data, name)
        before = list(g["members"])
        if members is not None:
            members = _dedupe(members)
            _check_members(data, members)
            g["members"] = members
        if add:
            add = _dedupe(add)
            _check_members(data, add)
            g["members"] += [i for i in add if i not in g["members"]]
        if remove:
            remove = _dedupe(remove)
            missing = [i for i in remove if i not in g["members"]]
            if missing:
                raise ValueError(f"不在組「{name}」裡: {', '.join(missing)}")
            g["members"] = [m for m in g["members"] if m not in remove]
        moved, refs, new = False, [], name
        if rename is not None and _check_group_name(rename) != name:
            new = _check_group_name(rename)
            if any(x["name"] == new for x in data["groups"]):
                raise ValueError(f"組已存在: {new}")
            src, dst = Path(spine_dir) / "groups" / name, Path(spine_dir) / "groups" / new
            moved = src.is_dir()
            if moved and dst.exists():
                raise ValueError(f"資料夾已存在: groups/{new}/（先自行合併或移走，不代為覆蓋）")
            refs = _schedule_refs(spine_dir, name)
            g["name"] = new
            if moved:
                src.rename(dst)
        return {"group": dict(g, members=list(g["members"])),
                "added": [m for m in g["members"] if m not in before],
                "removed": [m for m in before if m not in g["members"]],
                "renamed_from": name if new != name else None,
                "moved_dir": moved, "schedule_refs": refs}


def add_group_members(spine_dir, name, ids):
    return update_group(spine_dir, name, add=ids)["group"]


def remove_group_members(spine_dir, name, ids):
    """移出組（登記保留）；允許移到空組。"""
    return update_group(spine_dir, name, remove=ids)["group"]


def set_group_members(spine_dir, name, ids):
    """整份換掉（工作台編輯視窗用：勾選結果就是新名單）。"""
    return update_group(spine_dir, name, members=ids)["group"]


def _check_group_name(name):
    name = (name or "").strip()
    if not name:
        raise ValueError("組名不可為空")
    if "/" in name or "\\" in name or name in (".", ".."):
        raise ValueError(f"組名不可含路徑字元: {name}（會變成 groups/<組名>/ 資料夾）")
    return name


def _schedule_refs(spine_dir, name):
    return [f"{e.get('task')}@{e.get('at', '')}"
            for e in _config.load(spine_dir).get("schedule") or []
            if e.get("group") == name]


def rename_group(spine_dir, old, new):
    """改名：registry 欄位（目標等）原樣帶過去；spine 的 groups/<舊>/ 搬到 groups/<新>/。
    回傳 {group, moved_dir, schedule_refs}。舊事件裡的 group:<舊> 不改（脊椎 append-only）。"""
    r = update_group(spine_dir, old, rename=_check_group_name(new))
    return {"group": r["group"], "moved_dir": r["moved_dir"], "schedule_refs": r["schedule_refs"]}


def remove_group(spine_dir, name):
    """刪組：只刪 registry 的組定義；repo 登記與 groups/<組>/ 的料都保留。
    回傳 {name, kept_dir（保留的資料夾或 None）, schedule_refs}。"""
    with transaction(spine_dir) as data:
        _find_group(data, name)
        data["groups"] = [g for g in data["groups"] if g["name"] != name]
        d = Path(spine_dir) / "groups" / name
        return {"name": name, "kept_dir": str(d) if d.is_dir() else None,
                "schedule_refs": _schedule_refs(spine_dir, name)}


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
