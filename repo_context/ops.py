"""操作表：管理類寫入動作（registry／關係／export／組）只定義一次，CLI／工作台／MCP 共用。

2026-09-25 架構檢查：加一個功能要改 registry → CLI → webui → MCP 四處，連「記一筆 decision」
都各寫一份、措辭還不一樣。這裡每個操作宣告一次：參數（同時是 MCP inputSchema）、實作、
給人看的一行結果、留痕。入口只做轉接：

    ops.call(spine_dir, "group_update", {"name": "g1", "add": ["x"]}, via="cli")

- 參數驗證、留痕、錯誤型別（ValueError）在這裡統一；入口各自把錯誤轉成自己的形狀
  （CLI 印出、工作台回 {"error"}、MCP 回 isError）。
- 留痕的 source 依入口：cli → [cli]、工作台 → [monitor]（都是使用者自己出手，不進收件匣）；
  MCP → [agent]（agent 改了你的 registry，要讓你看得到）。
- 新增管理操作：在下面加一個 @op，MCP 管理層自動多一個工具；CLI／工作台要不要開入口另外決定。
"""
import datetime as _dt

from . import registry as _registry
from . import spine as _spine

VIA = {"cli": ("cli", "從 CLI"), "ui": ("monitor", "從工作台"), "mcp": ("agent", "從 MCP")}

STR = {"type": "string"}
ARR = {"type": "array", "items": {"type": "string"}}
BOOL = {"type": "boolean"}


def _p(base, desc, required=False):
    return {**base, "description": desc, "_required": required}


class Op:
    def __init__(self, name, summary, params, fn, say, audit):
        self.name, self.summary, self.params = name, summary, params
        self.fn, self.say, self.audit = fn, say, audit

    def schema(self):
        return {"type": "object",
                "properties": {k: {x: y for x, y in v.items() if x != "_required"}
                               for k, v in self.params.items()},
                "required": [k for k, v in self.params.items() if v.get("_required")]}


OPS = {}


def op(op_name, summary, /, say, audit=None, **params):
    """註冊一個操作。fn(spine_dir, **args) → dict；say(result, args) → 一行人話；
    audit(result, args) → (tokens, body) 或 None（不留痕）。"""
    def deco(fn):
        OPS[op_name] = Op(op_name, summary, params, fn, say, audit)
        return fn
    return deco


def call(spine_dir, name, args, via):
    """跑一個操作：驗參數 → 執行 → 留痕。回傳 (result, 一行人話)。錯誤一律 ValueError。"""
    o = OPS.get(name)
    if o is None:
        raise ValueError(f"未知操作: {name}")
    args = {k: v for k, v in (args or {}).items() if v is not None}
    unknown = [k for k in args if k not in o.params]
    if unknown:
        raise ValueError(f"{name} 不認得參數: {', '.join(unknown)}（可用：{', '.join(o.params)}）")
    missing = [k for k, v in o.params.items()
               if v.get("_required") and args.get(k) in (None, "", [])]
    if missing:
        raise ValueError(f"{name} 缺參數: {', '.join(missing)}")
    result = o.fn(spine_dir, **args)
    if o.audit:
        rec = o.audit(result, args)
        if rec:
            source, label = VIA[via]
            tokens, body = rec
            _spine.append_event(spine_dir, "decision", source, tokens,
                                body=f"{_spine.ADMIN_TAG}{body}（{label}）")
    return result, o.say(result, args)


def _csv(v):
    """CLI 給逗號字串、MCP／工作台給陣列——都收。"""
    if v is None:
        return None
    items = [v] if isinstance(v, str) else list(v)
    return [s.strip() for x in items for s in str(x).split(",") if s.strip()]


# ── repo ────────────────────────────────────────────────────────────────

@op("registry_add", "登記一個 repo（id＋路徑）",
    say=lambda r, a: f"已登記：{r['id']} ({r['type']}/{r['tier']})",
    audit=lambda r, a: ([f"repo:{r['id']}"], f"登記 repo：{r['id']}（{r['path']}）"),
    id=_p(STR, "repo id", True), path=_p(STR, "repo 路徑", True),
    type=_p(STR, "mine 或 external"), tier=_p(STR, "active／paused／dormant／archived"),
    tags=_p(ARR, "tag 清單"), upstream=_p(STR, "external 的 GitHub owner/repo"))
def _registry_add(d, id, path, type="mine", tier="active", tags=None, upstream=None):
    return _registry.add_repo(d, id, path, type=type, tags=_csv(tags), tier=tier,
                              upstream=upstream)


@op("registry_remove", "把 repo 移出 registry（同時清掉所有組的 membership 與指向它的關係）",
    say=lambda r, a: f"已移除：{a['id']}（含所有組的 membership）",
    audit=lambda r, a: ([], f"移出 registry：{a['id']}"),
    id=_p(STR, "repo id", True))
def _registry_remove(d, id):
    _registry.remove_repo(d, id)
    return {"id": id}


@op("registry_set", "改 repo 的一個欄位（例 note、resume_when）",
    say=lambda r, a: f"已更新：{a['id']}.{a['key']} = {a['value']}",
    audit=lambda r, a: ([f"repo:{a['id']}"], f"{a['id']}.{a['key']} = {a['value']}"),
    id=_p(STR, "repo id", True), key=_p(STR, "欄位名", True), value=_p(STR, "值", True))
def _registry_set(d, id, key, value):
    return _registry.set_field(d, id, key, value)


@op("registry_tier", "改 repo 的 tier（paused 可一併給復工條件）",
    say=lambda r, a: f"{r['id']}：tier {r['old']} → {r['tier']}",
    audit=lambda r, a: ([f"repo:{r['id']}"], f"tier {r['old']} → {r['tier']}"),
    id=_p(STR, "repo id", True), tier=_p(STR, "active／paused／dormant／archived", True),
    resume_when=_p(STR, "paused 的復工條件"))
def _registry_tier(d, id, tier, resume_when=None):
    if tier not in _registry.TIERS:
        raise ValueError(f"tier 需為 {'|'.join(_registry.TIERS)}")
    old = _registry.get_repo(d, id).get("tier")
    _registry.set_field(d, id, "tier", tier)
    if tier == "paused" and resume_when:
        _registry.set_field(d, id, "resume_when", resume_when)
    return {"id": id, "tier": tier, "old": old}


@op("registry_tag", "加／移除 repo 的 tag（冪等）",
    say=lambda r, a: f"{r['id']} tags: {', '.join(r['tags']) or '（無）'}",
    audit=lambda r, a: ([f"repo:{r['id']}"], "tag " + " ".join(
        [f"+{t}" for t in _csv(a.get("add")) or []] + [f"-{t}" for t in _csv(a.get("remove")) or []])),
    id=_p(STR, "repo id", True), add=_p(ARR, "要加的 tag"), remove=_p(ARR, "要移除的 tag"))
def _registry_tag(d, id, add=None, remove=None):
    return {"id": id, "tags": _registry.tag_repo(d, id, add=_csv(add), remove=_csv(remove))}


# ── 關係與 export ──────────────────────────────────────────────────────

def _forward(d, kind):
    return _registry.relation_kinds(d).get(kind, {}).get("forward", kind)


@op("registry_relate", "設 repo 間關係 a --kind--> b（例 a pm-of b＝a 是 b 的 PM；"
    "kind: pm-of/feeds/derived-from/upstream-of/sibling-topic）",
    say=lambda r, a: f"已設關係：{a['a']} {r['label']}→ {a['b']}（{r['kind']}）"
    + (f"，用到 export：{'、'.join(r['exports'])}" if r.get("exports") else ""),
    audit=lambda r, a: ([f"repo:{a['a']}"], f"關係：{a['a']} {r['label']}→ {a['b']}（{r['kind']}）"),
    a=_p(STR, "來源 repo", True), b=_p(STR, "目標 repo", True), kind=_p(STR, "關係種類", True),
    note=_p(STR, "一句說明"), exports=_p(STR, "逗號分隔：這條關係用到 a 的哪些 export"))
def _registry_relate(d, a, b, kind, note=None, exports=None):
    e = _registry.relate(d, a, b, kind, note=note, exports=exports)
    return {**e, "label": _forward(d, kind)}


@op("registry_unrelate", "刪 repo 間關係（不給 kind＝刪 a→b 全部）",
    say=lambda r, a: f"已刪 {r['removed']} 筆關係：{a['a']} → {a['b']}",
    audit=lambda r, a: ([f"repo:{a['a']}"], f"刪關係：{a['a']} → {a['b']}"
                        + (f"（{a['kind']}）" if a.get("kind") else "")) if r["removed"] else None,
    a=_p(STR, "來源 repo", True), b=_p(STR, "目標 repo", True), kind=_p(STR, "關係種類"))
def _registry_unrelate(d, a, b, kind=None):
    return {"removed": _registry.unrelate(d, a, b, kind)}


@op("registry_export", "登記 repo 對外提供的東西（export）：名稱→repo 內路徑；remove=true 刪除。"
    "agent 之後用 `<repo>:<名稱>/子路徑` 引用",
    say=lambda r, a: (f"已登記 export：{r['repo']}:{r['name']} → {r['path']}"
                      + (f"（{r['desc']}）" if r.get("desc") else "") if not r.get("removed_flag")
                      else f"已刪 export：{a['id']}:{a['name']}" if r["removed"]
                      else f"{a['id']} 沒有 export {a['name']}"),
    audit=lambda r, a: ([f"repo:{a['id']}"], (f"export {a['id']}:{a['name']} → {r['path']}"
                                              if not r.get("removed_flag")
                                              else f"刪 export {a['id']}:{a['name']}"))
    if not r.get("removed_flag") or r["removed"] else None,
    id=_p(STR, "repo id", True), name=_p(STR, "export 名稱", True),
    path=_p(STR, "repo 內路徑（登記時必填）"), desc=_p(STR, "一句說明"), remove=_p(BOOL, "true＝刪除"))
def _registry_export(d, id, name, path=None, desc=None, remove=False):
    if remove:
        return {"removed_flag": True, "removed": _registry.remove_export(d, id, name)}
    if not path:
        raise ValueError("registry_export 登記時需要 path")
    return _registry.set_export(d, id, name, path, desc=desc)


# ── 組 ──────────────────────────────────────────────────────────────────

@op("group_add", "建一個組",
    say=lambda r, a: f"組已建：{r['name']}",
    audit=lambda r, a: ([f"group:{r['name']}"], f"建組：{','.join(r['members'])}"),
    name=_p(STR, "組名", True), members=_p(ARR, "repo id 清單", True))
def _group_add(d, name, members):
    name = name.strip()
    if not name:
        raise ValueError("組名不可為空")
    _registry.add_group(d, name, _csv(members))
    return _registry.get_group(d, name)


def _group_update_body(r):
    parts = ([f"改名 {r['renamed_from']} → {r['group']['name']}"] if r["renamed_from"] else []) \
        + ([f"加入 {','.join(r['added'])}"] if r["added"] else []) \
        + ([f"移出 {','.join(r['removed'])}（登記保留）"] if r["removed"] else [])
    return "編輯組：" + ("；".join(parts) or "無變更")


def _group_update_say(r, a):
    g = r["group"]
    out = [f"{g['name']}：{','.join(g['members']) or '（空組）'}"]
    if r["moved_dir"]:
        out.append(f"groups/{r['renamed_from']}/ 已搬到 groups/{g['name']}/")
    if r["schedule_refs"]:
        out.append(f"注意：config.yaml 排程還指著「{r['renamed_from']}」："
                   f"{', '.join(r['schedule_refs'])}（請自行更新）")
    return "\n".join(out)


@op("group_update", "改組：add／remove 組員（repo 登記不動）、members 整份換掉、rename 改名；"
    "可一次給多項，任何一項失敗＝全部不改",
    say=_group_update_say,
    audit=lambda r, a: ([f"group:{r['group']['name']}"], _group_update_body(r)),
    name=_p(STR, "組名", True), add=_p(ARR, "要加入的 repo id"), remove=_p(ARR, "要移出的 repo id"),
    members=_p(ARR, "新的完整成員名單（取代現有）"), rename=_p(STR, "新組名"))
def _group_update(d, name, add=None, remove=None, members=None, rename=None):
    if add is None and remove is None and members is None and rename is None:
        raise ValueError("沒有要改的：給 add／remove／members／rename 至少一項")
    if members is not None and not _csv(members):
        raise ValueError("members 不可為空（要整組拿掉請用 group_remove）")
    return _registry.update_group(d, name, add=_csv(add), remove=_csv(remove),
                                  members=_csv(members), rename=rename)


@op("group_remove", "刪除一個組（只刪組定義；repo 登記與 groups/<組>/ 的料保留）",
    say=lambda r, a: "\n".join([f"已刪除組：{r['name']}（repo 登記保留）"]
                               + ([f"保留了這組的料：{r['kept_dir']}"] if r["kept_dir"] else [])
                               + ([f"注意：config.yaml 排程還指著「{r['name']}」："
                                   f"{', '.join(r['schedule_refs'])}（請自行更新）"]
                                  if r["schedule_refs"] else [])),
    audit=lambda r, a: ([f"group:{r['name']}"], "刪除組（repo 登記保留）"),
    name=_p(STR, "組名", True))
def _group_remove(d, name):
    return _registry.remove_group(d, name)


@op("group_goal", "記下（或清除）組的一句話目標——目標只有人能給，agent 別自己編",
    say=lambda r, a: f"目標已記在 {r['group']}" if r["goal"] else f"已清除 {r['group']} 的目標",
    audit=lambda r, a: ([f"group:{r['group']}"],
                        f"本組目標：{r['goal']}" if r["goal"] else "清除本組目標"),
    name=_p(STR, "組名", True), text=_p(STR, "一句話目標；空字串＝清除"))
def _group_goal(d, name, text=""):
    name, text = name.strip(), (text or "").strip()
    if len(text) > 300:
        raise ValueError("目標請寫一句話（300 字以內）")
    now = _dt.datetime.now()
    _registry.set_group_field(d, name, "goal", text)
    _registry.set_group_field(d, name, "goal_at", f"{now:%Y-%m-%d %H:%M}" if text else "")
    return {"group": name, "goal": text}
