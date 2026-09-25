"""工作台的行動按鈕 → 引擎原語（每個 action 一個函式，ACTIONS 表分派）。

registry／組的寫入走操作表（ops.py）：參數驗證、留痕跟 CLI／MCP 同一份，這裡只轉回傳形狀。
新增 action：寫一個 `_a_<名稱>(spine_dir, payload, terms)`，加進 ACTIONS。
"""
import datetime as _dt

from .. import brief as _brief
from .. import collide as _collide
from .. import ops as _ops
from .. import registry as _registry
from .. import route as _route
from .. import spine as _spine
from .state import _collision_index, normalize_dest, session_draft


def _via_op(spine_dir, name, args, shape):
    """跑操作表的操作；ValueError → {"error"}（工作台的錯誤形狀）。"""
    try:
        result, _ = _ops.call(spine_dir, name, args, via="ui")
    except ValueError as e:
        return {"error": str(e)}
    return {"ok": True, **shape(result)}


def _a_ack(spine_dir, payload, terms):
    _spine.ack_unread(spine_dir)
    return {"ok": True}


def _a_relate(spine_dir, payload, terms):
    return _via_op(spine_dir, "registry_relate",
                   {"a": payload.get("a"), "b": payload.get("b"), "kind": payload.get("kind"),
                    "note": payload.get("note") or None},
                   lambda r: {"relation": {k: v for k, v in r.items() if k != "label"}})


def _a_unrelate(spine_dir, payload, terms):
    return _via_op(spine_dir, "registry_unrelate",
                   {"a": payload.get("a"), "b": payload.get("b"), "kind": payload.get("kind") or None},
                   lambda r: r)


def _a_collide(spine_dir, payload, terms):
    idea = (payload.get("idea") or "").strip()
    if not idea:
        return {"error": "想法不可為空"}
    repos = payload.get("repos") or None
    group = payload.get("group") or None
    cid = _collide.submit(spine_dir, idea, group=group, repos=repos,
                          source="monitor")
    if payload.get("wait"):
        j = _collide.run_judgement(spine_dir, cid)
        return {"ok": True, "cid": cid, "judgement": j}
    _collide.spawn_detached(spine_dir, cid)
    return {"ok": True, "cid": cid}


def _a_collide_rerun(spine_dir, payload, terms):
    cid = payload.get("cid") or ""
    if not cid:
        return {"error": "缺 cid"}
    _collide.spawn_detached(spine_dir, cid)
    return {"ok": True, "cid": cid}


def _a_ignore(spine_dir, payload, terms):
    etype, etime = payload.get("type"), payload.get("time")
    if etype not in _spine.EVENT_TYPES:
        return {"error": f"未知事件型別: {etype}"}
    ref = f"collision:{payload['cid']}" if payload.get("cid") else f"{etype}:{etime}"
    _spine.append_event(spine_dir, "chosen", "monitor", [f"ref:{ref}"],
                        body=f"忽略並記錄：{payload.get('header', '')}")
    return {"ok": True}


def _a_close_loop(spine_dir, payload, terms):
    num = payload.get("num", "")
    if not (num.startswith("#") and num[1:].isdigit()):
        return {"error": f"loop 編號不合法: {num}"}
    cur = next((e for e in _spine.open_loops(spine_dir) if num in e.tokens), None)
    tokens = [t for t in (cur.tokens if cur else []) if t.startswith(("repo:", "group:"))]
    _spine.append_event(spine_dir, "open-loop", "monitor", [num, *tokens],
                        body="closed → 從工作台關閉")
    return {"ok": True}


def _a_loop_defer(spine_dir, payload, terms):
    num = payload.get("num", "")
    if not (num.startswith("#") and num[1:].isdigit()):
        return {"error": f"loop 編號不合法: {num}"}
    days = int(payload.get("days") or 7)
    cur = next((e for e in _spine.open_loops(spine_dir) if num in e.tokens), None)
    if cur is None:
        return {"error": f"loop 不存在或已關: {num}"}
    due = _dt.date.today() + _dt.timedelta(days=days)
    first = (cur.body or "").splitlines()[0] if cur.body else ""
    _spine.append_event(spine_dir, "open-loop", "monitor",
                        [num, f"due:{due:%Y-%m-%d}",
                         *[t for t in cur.tokens if t.startswith(("repo:", "group:"))]],
                        body=f"{first}\n延期 {days} 天 → 從工作台")
    return {"ok": True, "due": f"{due:%Y-%m-%d}"}


def _a_defer(spine_dir, payload, terms):
    rid = payload.get("id") or ""
    qkind = payload.get("qkind") or "question"
    days = int(payload.get("days") or 7)
    if not rid:
        return {"error": "缺 repo id"}
    until = _dt.date.today() + _dt.timedelta(days=days)
    # 按鈕說「七天後再提醒」，做的就是延後提醒——留痕第一行也要這樣寫，
    # 使用者翻活動記錄時才說得出系統究竟做了什麼（2026-09-12 檢討 §6.1）
    _spine.append_event(
        spine_dir, "chosen", "monitor", [f"repo:{rid}"],
        body=f"{days} 天後再提醒（{rid} 的「{_brief.qkind_name(qkind)}」"
             f"提醒延到 {until:%Y-%m-%d}；"
             f"沒有安排任何進度）\n"
             f"snooze:{qkind} until:{until:%Y-%m-%d}\n"
             f"{payload.get('text', '')}".rstrip())
    return {"ok": True, "until": f"{until:%Y-%m-%d}", "days": days}


def _a_tier(spine_dir, payload, terms):
    return _via_op(spine_dir, "registry_tier",
                   {"id": payload.get("id"), "tier": payload.get("tier"),
                    "resume_when": payload.get("resume_when") or None},
                   lambda r: {"id": r["id"], "tier": r["tier"]})


def _a_remove_repo(spine_dir, payload, terms):
    return _via_op(spine_dir, "registry_remove", {"id": payload.get("id")}, lambda r: r)


def _a_route_collision(spine_dir, payload, terms):
    cid, dest = payload.get("cid") or "", payload.get("dest") or ""
    dest = normalize_dest(dest)
    if not cid or not dest:
        return {"error": "dest 不合法（repo:<id> | group:<g> | incubator）"}
    events = list(_spine.iter_events(spine_dir))
    rec = _collision_index(events).get(cid)
    if not rec:
        return {"error": f"找不到碰撞: {cid}"}
    judged = next((e for e in events if e.type == "collision"
                   and e.kv("id") == cid and e.body.startswith("判定：")), None)
    text = (f"# 碰撞 {cid}\n\n## 想法\n{rec['idea']}\n\n## 判定\n"
            f"{judged.body if judged else '（尚無判定）'}\n\n"
            f"（來源：{rec['opened'].date} {rec['opened'].time}｜範圍 {rec['group'] or '全部'}）\n")
    target = _route.route(spine_dir, text, dest, title=f"collision-{cid}")
    _spine.append_event(spine_dir, "chosen", "monitor", [f"ref:collision:{cid}"],
                        body=f"落點：{dest} → {target.name}")
    return {"ok": True, "cid": cid, "dest": dest, "file": str(target)}


def _a_tag(spine_dir, payload, terms):
    return _via_op(spine_dir, "registry_tag",
                   {"id": payload.get("id"), "add": payload.get("add") or None,
                    "remove": payload.get("remove") or None}, lambda r: r)


def _a_save_group(spine_dir, payload, terms):
    if not payload.get("repos"):
        return {"error": "至少勾選一個 repo"}
    return _via_op(spine_dir, "group_add",
                   {"name": payload.get("name") or "", "members": payload.get("repos")},
                   lambda r: {"name": r["name"]})


def _a_update_group(spine_dir, payload, terms):
    # 編輯牌組：勾選結果＝新名單；new_name 不同才改名——一個 transaction，失敗不留半套
    if not payload.get("repos"):
        return {"error": "至少勾選一個 repo（要整組拿掉請用刪除）"}
    return _via_op(spine_dir, "group_update",
                   {"name": (payload.get("name") or "").strip(), "members": payload.get("repos"),
                    "rename": (payload.get("new_name") or "").strip() or None},
                   lambda r: {"name": r["group"]["name"], "members": r["group"]["members"]})


def _a_remove_group(spine_dir, payload, terms):
    return _via_op(spine_dir, "group_remove", {"name": (payload.get("name") or "").strip()},
                   lambda r: r)


def _a_set_goal(spine_dir, payload, terms):
    if not (payload.get("group") or "").strip():
        return {"error": "目標只能記在已儲存的牌組上；先把這個範圍存成牌組"}
    return _via_op(spine_dir, "group_goal",
                   {"name": payload.get("group"), "text": payload.get("text") or ""}, lambda r: r)


def _a_session_draft(spine_dir, payload, terms):
    session = terms.get(payload.get("sid", "")) if terms else None
    if not session:
        return {"error": f"未知會話: {payload.get('sid')}"}
    info = {"sid": session.sid, "title": session.title,
            "kind": getattr(session, "kind", "shell"),
            "scope": getattr(session, "scope", None),
            "scope_repos": getattr(session, "scope_repos", None),
            "started": getattr(session, "started", None)}
    return {"ok": True, **session_draft(spine_dir, info)}


def _a_scan_dir(spine_dir, payload, terms):
    base = (payload.get("dir") or "").strip()
    if not base:
        return {"error": "請填要掃描的資料夾路徑"}
    try:
        found = _registry.scan_dir(spine_dir, base)
    except ValueError as err:
        return {"error": str(err)}
    return {"ok": True, "dir": base,
            "candidates": [{"path": str(p), "id": cid} for p, cid in found]}


def _a_register_repos(spine_dir, payload, terms):
    items = payload.get("repos") or []
    if not items:
        return {"error": "至少選一個 repo 才建得起工作範圍"}
    name = (payload.get("group") or "").strip()
    added = []
    try:
        for it in items:
            rid = (it.get("id") or "").strip()
            path = (it.get("path") or "").strip()
            if not rid or not path:
                raise ValueError("每個 repo 都需要 id 與路徑")
            _registry.add_repo(spine_dir, rid, path)
            added.append(rid)
        if name:
            _registry.add_group(spine_dir, name, added)
    except ValueError as err:
        return {"error": str(err), "added": added}
    _spine.append_event(spine_dir, "decision", "monitor",
                        ([f"group:{name}"] if name else []),
                        body=f"登記 {len(added)} 個 repo："
                             f"{'、'.join(added)}"
                             + (f"，建立牌組「{name}」" if name else "")
                             + "（從工作台）")
    return {"ok": True, "added": added, "group": name or None}


def _a_brief(spine_dir, payload, terms):
    out, text = _brief.run(spine_dir, payload.get("group") or None,
                           payload.get("repos") or None)
    return {"ok": True, "text": text, "file": str(out)}


def _a_term_create(spine_dir, payload, terms):
    if terms is None:
        return {"error": "本 server 未啟用 terminal"}
    kind = payload.get("kind", "shell")
    if kind == "agent":
        s = terms.create_agent(group=payload.get("group") or None,
                               repos=payload.get("repos") or None,
                               repo=payload.get("repo") or None,
                               task=payload.get("task") or None,
                               agent=payload.get("agent") or "claude",
                               origin=payload.get("origin") or None)
    else:
        s = terms.create_shell(cwd=payload.get("cwd") or None,
                               origin=payload.get("origin") or None)
    return {"ok": True, "sid": s.sid, "title": s.title}


def _a_term_kill(spine_dir, payload, terms):
    if terms is None or not terms.kill(payload.get("sid", "")):
        return {"error": f"未知會話: {payload.get('sid')}"}
    return {"ok": True}


def _a_session_result(spine_dir, payload, terms):
    session = terms.get(payload.get("sid", "")) if terms else None
    text = (payload.get("text") or "").strip()
    if not session or not text:
        return {"error": "請選擇會話並填寫結果"}
    origin = getattr(session, "origin", "") or ""
    tokens = [f"ref:{origin}"] if origin.startswith("collision:") else []
    scope = getattr(session, "scope", "")
    members = getattr(session, "scope_repos", None)
    registry = _registry.load(spine_dir)
    if scope in {g["name"] for g in registry["groups"]}:
        tokens.append(f"group:{scope}")
    elif scope in {r["id"] for r in registry["repos"]}:
        tokens.append(f"repo:{scope}")
    elif members:
        tokens.append("group:臨時(" + ",".join(members) + ")")
    # A session note is a decision record, not evidence of an adopted idea.
    ev = _spine.append_event(spine_dir, "decision", "monitor", tokens,
                             body=f"會話結果：{session.title}\n{text}")
    return {"ok": True, "date": ev.date, "time": ev.time}


ACTIONS = {
    "ack": _a_ack,
    "relate": _a_relate,
    "unrelate": _a_unrelate,
    "collide": _a_collide,
    "collide_rerun": _a_collide_rerun,
    "ignore": _a_ignore,
    "close_loop": _a_close_loop,
    "loop_defer": _a_loop_defer,
    "defer": _a_defer,
    "tier": _a_tier,
    "remove_repo": _a_remove_repo,
    "route_collision": _a_route_collision,
    "tag": _a_tag,
    "save_group": _a_save_group,
    "update_group": _a_update_group,
    "remove_group": _a_remove_group,
    "set_goal": _a_set_goal,
    "session_draft": _a_session_draft,
    "scan_dir": _a_scan_dir,
    "register_repos": _a_register_repos,
    "brief": _a_brief,
    "term_create": _a_term_create,
    "term_kill": _a_term_kill,
    "session_result": _a_session_result,
}


def handle_action(spine_dir, action, payload, terms=None):
    if "repos" in payload and payload["repos"] == []:
        return {"error": "至少選擇一個 repo；空範圍不代表全部"}
    fn = ACTIONS.get(action)
    if fn is None:
        return {"error": f"未知 action: {action}"}
    return fn(spine_dir, payload, terms)
