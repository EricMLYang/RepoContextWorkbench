"""S4 工作台（三個一級物件：牌／收件匣／會話；2026-09-02 UI 檢討後重排）。

2026-09-01 裁定：要工作台不要玩具監控頁，內嵌 terminal 直接與 agent 對話。
2026-09-02 檢討（`personal_agent_design/20260902_工作台UI_UX檢討.md`）：畫面曾是
「監控頁黏 terminal」——行動出口是字不是按鈕、三種待辦格式疊加、會話無主。本輪重排：
- 牌（sidebar）：組是一張卡；臨時組合＝未命名卡；repo 微標 icon 化
- 收件匣（main）：統一卡片 {key, kind, title, lines, actions}，出口是真按鈕→呼叫引擎原語
  →落 chosen 後卡消失；自己丟的碰撞＝「處理中」卡，回程原地換判定卡；
  自己出手的留痕（presented/chosen/monitor decision）不進收件匣
- 會話（sidebar 下段＋terminal 面板）：每個會話有主（origin＝哪張卡／哪次碰撞），title＝任務摘要
- 早晨簡報＝收件匣的三段折疊視圖；「存成今日簡報」落 presented

殼原則（v2 §1）不變：
- /api/state  輕量輪詢（純脊椎/registry 檔案讀）＝「殼只 poll 脊椎未讀」
- /api/scan   才跑 P3 採集（開頁/切組/勾選/手動），高頻輪詢不打 git subprocess
- 所有行動按鈕都只是呼叫引擎原語再落脊椎；terminal 會話生命週期獨立於視窗

安全（檢討④ VDI multi-session：localhost 埠跨 session 共享）：
- 只綁 127.0.0.1 ＋ 每次啟動隨機 token——URL 帶 token 才服務（/static 除外）；
  pywebview 視窗拿完整 URL，其他本機使用者猜不到
- WS（terminal）同樣驗 token；WS 實作為 stdlib 手寫 RFC6455 最小子集（零依賴）
"""
import base64
import datetime as _dt
import hashlib
import json
import queue
import secrets
import struct
import threading
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

from . import brief as _brief
from . import collect as _collect
from . import collide as _collide
from . import config as _config
from . import notify as _notify
from . import registry as _registry
from . import route as _route
from . import spine as _spine
from . import term as _term
from . import timer as _timer

STATIC_DIR = Path(__file__).parent / "static"
PAGE_PATH = Path(__file__).parent / "workbench.html"

# 自己出手的留痕：不是要處理的事，不進收件匣（深看的事件表仍看得到）
_OWN_RECORD_TYPES = ("presented", "chosen")
_OWN_RECORD_SOURCES = ("monitor", "route", "spawn", "morning-brief")


def _ev_dict(ev):
    return {"date": ev.date, "time": ev.time, "type": ev.type,
            "source": ev.source, "tokens": ev.tokens, "body": ev.body,
            "header": ev.header()}


def _act(label, action, **payload):
    return {"label": label, "action": action, "payload": payload}


def _parse_judgement(body):
    """判定卡 body（collide.run_judgement 寫的五欄位）→ dict；不是判定 body 回 None。"""
    if not body.startswith("判定："):
        return None
    j = {}
    for ln in body.splitlines():
        if "：" in ln and not ln.startswith("〔"):
            k, v = ln.split("：", 1)
            j[k.strip()] = v.strip()
    return j


def normalize_dest(text):
    """落點建議字串 → route dest（'repo:<id>' | 'group:<g>' | 'incubator'）。解不出回 None。"""
    t = (text or "").strip()
    if not t:
        return None
    if t.startswith("incubator"):
        return "incubator"
    for prefix in ("repo:", "group:"):
        if t.startswith(prefix):
            ident = t[len(prefix):].split("/", 1)[0].split(" ", 1)[0].strip()
            return f"{prefix}{ident}" if ident else None
    return None


def _handled_keys(events):
    """被 chosen ref 到的事件 key（'collision:<cid>' 或 '<type>:<HH:MM>'）＝已處理。"""
    keys = set()
    for ev in events:
        if ev.type == "chosen":
            ref = ev.kv("ref")
            if ref:
                keys.add(ref)
    return keys


def _collision_index(events):
    """cid → {'idea', 'group', 'opened': ev, 'answered': bool}"""
    idx = {}
    for ev in events:
        if ev.type != "collision":
            continue
        cid = ev.kv("id")
        if not cid:
            continue
        rec = idx.setdefault(cid, {"idea": "", "group": None, "opened": None,
                                   "answered": False})
        if ev.body.startswith("opened"):
            rec["opened"] = ev
            rec["idea"] = ev.body.split("輸入：", 1)[-1].strip()
            rec["group"] = ev.kv("group")
        else:
            rec["answered"] = True
    return idx


def _collision_scope(group):
    """opened 事件的 group token → term_create 的 group/repos payload。"""
    if not group:
        return {}
    if group.startswith("臨時(") and group.endswith(")"):
        return {"repos": group[3:-1].split(",")}
    return {"group": group}


def event_scopes(events, registry):
    """Read-only provenance for cards/activity; never infer ownership from body text.

    Collision replies inherit their opening scope; decisions inherit referenced
    events. Unclassified records remain visible in their own explicitly named lane.
    """
    groups = {g["name"]: g["members"] for g in registry["groups"]}
    openings = {e.kv("id"): e for e in events
                if e.type == "collision" and e.body.startswith("opened")}
    refs = {}
    for e in events:
        refs.setdefault(f"{e.type}:{e.time}", []).append(e)
    cache = {}

    def resolve(e, seen=None):
        if id(e) in cache:
            return cache[id(e)]
        seen = set() if seen is None else set(seen)
        if id(e) in seen:
            return {"scope_kind": "unclassified", "scope_label": "未分類", "scope_repos": []}
        seen.add(id(e))
        rid, group = e.kv("repo"), e.kv("group")
        if rid:
            result = {"scope_kind": "repo", "scope_label": rid, "scope_repos": [rid]}
        elif group:
            if group.startswith("臨時(") and group.endswith(")"):
                members = group[3:-1].split(",")
            else:
                members = groups.get(group, [])
            result = {"scope_kind": "group", "scope_label": group, "scope_repos": members}
        else:
            ref = e.kv("ref") or ""
            cid = e.kv("id") if e.type == "collision" else None
            if ref.startswith("collision:"):
                cid = ref[len("collision:"):]
            parent = openings.get(cid) if cid else None
            if parent is e:
                result = {"scope_kind": "global", "scope_label": "所有 repo", "scope_repos": []}
            else:
                candidates = [x for x in refs.get(ref, []) if x.date == e.date and x is not e]
                if parent is None and len(candidates) == 1:
                    parent = candidates[0]
                result = resolve(parent, seen) if parent else {
                    "scope_kind": "unclassified", "scope_label": "未分類", "scope_repos": []}
        cache[id(e)] = result
        return result

    return {id(e): resolve(e) for e in events}


def build_cards(spine_dir, events, pending, loops, scopes=None):
    """收件匣＝一種卡。順序：interrupt → 處理中 → 碰撞回程 → 其他未讀 → open loops。"""
    handled = _handled_keys(events)
    cidx = _collision_index(events)
    cards = []
    seen_cids = set()
    scopes = scopes or event_scopes(events, _registry.load(spine_dir))
    by_record = {(e.date, e.header(), e.body): scopes[id(e)] for e in events}

    def provenance(ev):
        return by_record.get((ev.date, ev.header(), ev.body), {
            "scope_kind": "unclassified", "scope_label": "未分類", "scope_repos": []})

    def event_card(ev, interrupt):
        cid = ev.kv("id") if ev.type == "collision" else None
        if ev.type in _OWN_RECORD_TYPES or ev.source in _OWN_RECORD_SOURCES:
            return None
        if cid and ev.body.startswith("opened"):
            return None                     # 自己的提問由 pending 卡呈現
        if ev.type == "open-loop":
            return None                     # loop 由 loop 卡呈現（不重複成一則未讀）
        j = _parse_judgement(ev.body) if cid else None
        key = f"collision:{cid}" if (cid and j) else f"{ev.type}:{ev.time}"
        if key in handled or (cid and j and cid in seen_cids):
            return None
        if cid and j:
            seen_cids.add(cid)
            info = cidx.get(cid, {})
            dest = normalize_dest(j.get("落點建議"))
            scope = _collision_scope(info.get("group"))
            task = (f"深撞碰撞 {cid}：想法「{info.get('idea', '')}」；判定 {j.get('判定', '')}，"
                    f"理由：{j.get('理由', '')}。請對照組內文件驗證判定並提出落地步驟")
            actions = []
            if dest:
                actions.append(_act(f"照建議落 {dest}", "route_collision", cid=cid, dest=dest))
            actions.append(_act("改落點…", "route_pick", cid=cid))
            if dest != "incubator":
                actions.append(_act("升格 incubator", "route_collision", cid=cid, dest="incubator"))
            actions.append(_act("深撞：開會話", "term_create", kind="agent", task=task,
                                origin=f"collision:{cid}", **scope))
            actions.append(_act("丟棄並記錄", "ignore", type=ev.type, time=ev.time,
                                header=ev.header(), cid=cid))
            return {"key": key, "kind": "collision", "cid": cid,
                    **provenance(ev),
                    "time": ev.time, "date": ev.date,
                    "title": info.get("idea") or f"碰撞 {cid}",
                    "verdict": j.get("判定", ""), "judgement": j,
                    "lines": [f"{k}：{v}" for k, v in j.items() if k != "判定"],
                    "actions": actions}
        first = (ev.body or "").splitlines()
        title = first[0] if first else " ".join(ev.tokens)
        lines = first[1:6]
        actions = [_act("忽略並記錄", "ignore", type=ev.type, time=ev.time,
                        header=ev.header())]
        if interrupt and cid:
            actions.insert(0, _act("重跑判定", "collide_rerun", cid=cid))
        return {"key": key, "kind": "interrupt" if interrupt else "event",
                **provenance(ev),
                "time": ev.time, "date": ev.date, "etype": ev.type,
                "source": ev.source, "title": title, "lines": lines,
                "actions": actions}

    for ev in pending["interrupt"]:
        c = event_card(ev, True)
        if c:
            cards.append(c)
    # 處理中：opened 未回程（不看 last_seen——沒回來就一直是處理中）
    for cid, rec in sorted(cidx.items()):
        if not rec["answered"] and rec["opened"] is not None \
                and f"collision:{cid}" not in handled:
            ev = rec["opened"]
            cards.append({"key": f"pending:{cid}", "kind": "pending", "cid": cid,
                          **provenance(ev),
                          "time": ev.time, "date": ev.date, "title": rec["idea"],
                          "lines": [f"送出於 {ev.date} {ev.time}｜範圍 {rec['group'] or '全部'}"],
                          "actions": []})
    normal_cards = []
    for ev in pending["normal"]:
        c = event_card(ev, False)
        if c:
            normal_cards.append(c)
    normal_cards.sort(key=lambda c: 0 if c["kind"] == "collision" else 1)
    cards += normal_cards
    for ev in loops:
        num = next((t for t in ev.tokens if t.startswith("#")), "#?")
        first = (ev.body or "").splitlines()
        title = first[0] if first else num
        for pre in ("opened →", "opened"):
            if title.startswith(pre):
                title = title[len(pre):].strip()
        cards.append({"key": f"loop:{num}", "kind": "loop", "num": num,
                      **provenance(ev),
                      "due": ev.kv("due"), "time": ev.time, "date": ev.date,
                      "title": title.strip("「」"), "lines": [],
                      "actions": [_act("開碰撞", "collide_prefill", text=title.strip("「」")),
                                  _act("延 7 天", "loop_defer", num=num, days=7),
                                  _act("關閉", "close_loop", num=num)]})
    return cards


def build_state(spine_dir):
    """輕量狀態（純檔案讀，供 5 秒輪詢）。interrupt 先於 normal（打斷要掙得，其餘累積）。"""
    data = _registry.load(spine_dir)
    p = _notify.pending(spine_dir)
    events = list(_spine.iter_events(spine_dir))
    loops = _spine.open_loops(spine_dir)
    scopes = event_scopes(events, data)
    vocab = list(_config.load(spine_dir)["tags"]["suggestions"])
    for t in _registry.all_tags(spine_dir):
        if t not in vocab:
            vocab.append(t)
    ordered = sorted(enumerate(events), key=lambda item: (item[1].date, item[1].time, item[0]), reverse=True)
    recent = [dict(_ev_dict(e), **scopes[id(e)]) for _, e in ordered[:200]]
    return {
        "groups": [{"name": g["name"], "members": g["members"]}
                   for g in data["groups"]],
        "repos": [r["id"] for r in data["repos"]],
        "tiers": {r["id"]: r.get("tier", "active") for r in data["repos"]},
        "tags": {r["id"]: (r.get("tags") or []) for r in data["repos"]},
        "tag_vocab": vocab,
        # 關係層（2026-09-08）：前端算鄰居、畫 ⇄、深看關係段都從這兩個欄位來
        "relations": _registry.relations(spine_dir),
        "rel_kinds": _registry.relation_kinds(spine_dir),
        "unread": [dict(_ev_dict(e), interrupt=True) for e in p["interrupt"]]
                  + [_ev_dict(e) for e in p["normal"]],
        "loops": [_ev_dict(e) for e in loops],
        "cards": build_cards(spine_dir, events, p, loops, scopes),
        "recent": recent,
        "stats": {**_spine.stats(spine_dir), **_registry.survival(spine_dir)},
    }


def _snoozed(spine_dir, today=None):
    """defer 留痕（chosen [monitor] repo:<id> body 'snooze:<qkind> until:<date>'）→ {(repo, qkind)}。"""
    today = today or f"{_dt.date.today():%Y-%m-%d}"
    out = set()
    for ev in _spine.query(spine_dir, type="chosen"):
        body = ev.body or ""
        if not body.startswith("snooze:"):
            continue
        head = body.splitlines()[0].split()
        qkind = head[0][len("snooze:"):]
        until = next((h[len("until:"):] for h in head if h.startswith("until:")), "")
        if until >= today:
            out.add((ev.kv("repo"), qkind))
    return out


def build_scan(spine_dir, group=None, repos=None):
    """P3 採集＋閾值問句卡＋audit（開頁/切組/勾選/手動才跑，因為會打 git）。
    repos 給定＝臨時組合（P2 免建組），優先於 group。"""
    cfg = _config.load(spine_dir)
    if repos is not None and not repos:
        raise ValueError("至少選擇一個 repo；空範圍不代表全部")
    if repos:
        gname, entries = _registry.resolve_group(spine_dir, None, repos)
    elif group:
        gname, entries = _registry.resolve_group(spine_dir, group)
    else:
        entries = _registry.load(spine_dir)["repos"]
        gname = "全部"
    states = _collect.collect_group(entries, spine_dir)
    snoozed = _snoozed(spine_dir)
    cards = [c for c in _brief.build_question_cards(states, cfg["thresholds"])
             if (c["repo"], c["qkind"]) not in snoozed]
    quiet = len(states) - len({c["repo"] for c in cards})
    return {
        "group": gname,
        "states": states,
        # 活動脈動（2026-09-08）：組的監控焦點＝活動頻繁度＋近期 commit 內容
        "pulse": _collect.group_pulse(states),
        "recent_commits": _collect.recent_across(states, limit=30),
        "question_cards": cards,
        "questions": [_brief.card_text(c) for c in cards],
        "quiet": max(quiet, 0),
        # registry 稽核＋脊椎衛生迴圈併同一個紅字面（second-brain lint 課）
        "audit": _registry.audit(spine_dir) + _spine.lint(spine_dir),
    }


def _handle_action(spine_dir, action, payload, terms=None):
    if "repos" in payload and payload["repos"] == []:
        return {"error": "至少選擇一個 repo；空範圍不代表全部"}
    if action == "ack":
        _spine.ack_unread(spine_dir)
        return {"ok": True}
    if action == "relate":
        try:
            e = _registry.relate(spine_dir, payload.get("a", ""), payload.get("b", ""),
                                 payload.get("kind", ""), note=payload.get("note") or None)
        except ValueError as err:
            return {"error": str(err)}
        return {"ok": True, "relation": e}
    if action == "unrelate":
        try:
            n = _registry.unrelate(spine_dir, payload.get("a", ""), payload.get("b", ""),
                                   payload.get("kind") or None)
        except ValueError as err:
            return {"error": str(err)}
        return {"ok": True, "removed": n}
    if action == "collide":
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
    if action == "collide_rerun":
        cid = payload.get("cid") or ""
        if not cid:
            return {"error": "缺 cid"}
        _collide.spawn_detached(spine_dir, cid)
        return {"ok": True, "cid": cid}
    if action == "ignore":
        etype, etime = payload.get("type"), payload.get("time")
        if etype not in _spine.EVENT_TYPES:
            return {"error": f"未知事件型別: {etype}"}
        ref = f"collision:{payload['cid']}" if payload.get("cid") else f"{etype}:{etime}"
        _spine.append_event(spine_dir, "chosen", "monitor", [f"ref:{ref}"],
                            body=f"忽略並記錄：{payload.get('header', '')}")
        return {"ok": True}
    if action == "close_loop":
        num = payload.get("num", "")
        if not (num.startswith("#") and num[1:].isdigit()):
            return {"error": f"loop 編號不合法: {num}"}
        cur = next((e for e in _spine.open_loops(spine_dir) if num in e.tokens), None)
        tokens = [t for t in (cur.tokens if cur else []) if t.startswith(("repo:", "group:"))]
        _spine.append_event(spine_dir, "open-loop", "monitor", [num, *tokens],
                            body="closed → 從工作台關閉")
        return {"ok": True}
    if action == "loop_defer":
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
    if action == "defer":
        rid = payload.get("id") or ""
        qkind = payload.get("qkind") or "question"
        days = int(payload.get("days") or 7)
        if not rid:
            return {"error": "缺 repo id"}
        until = _dt.date.today() + _dt.timedelta(days=days)
        _spine.append_event(spine_dir, "chosen", "monitor", [f"repo:{rid}"],
                            body=f"snooze:{qkind} until:{until:%Y-%m-%d}\n"
                                 f"{payload.get('text', '')}".rstrip())
        return {"ok": True, "until": f"{until:%Y-%m-%d}"}
    if action == "tier":
        rid, tier = payload.get("id") or "", payload.get("tier") or ""
        if tier not in _registry.TIERS:
            return {"error": f"tier 需為 {'|'.join(_registry.TIERS)}"}
        old = _registry.get_repo(spine_dir, rid).get("tier")
        _registry.set_field(spine_dir, rid, "tier", tier)
        if tier == "paused" and payload.get("resume_when"):
            _registry.set_field(spine_dir, rid, "resume_when", payload["resume_when"])
        _spine.append_event(spine_dir, "decision", "monitor", [f"repo:{rid}"],
                            body=f"tier {old} → {tier}（從工作台）")
        return {"ok": True, "id": rid, "tier": tier}
    if action == "remove_repo":
        rid = payload.get("id") or ""
        _registry.remove_repo(spine_dir, rid)
        _spine.append_event(spine_dir, "decision", "monitor", [],
                            body=f"移出 registry：{rid}（從工作台）")
        return {"ok": True, "id": rid}
    if action == "route_collision":
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
    if action == "tag":
        rid = payload.get("id") or ""
        tags = _registry.tag_repo(spine_dir, rid,
                                  add=payload.get("add") or None,
                                  remove=payload.get("remove") or None)
        return {"ok": True, "id": rid, "tags": tags}
    if action == "save_group":
        name = (payload.get("name") or "").strip()
        members = payload.get("repos") or []
        if not name:
            return {"error": "組名不可為空"}
        if not members:
            return {"error": "至少勾選一個 repo"}
        _registry.add_group(spine_dir, name, members)
        return {"ok": True, "name": name}
    if action == "brief":
        out, text = _brief.run(spine_dir, payload.get("group") or None,
                               payload.get("repos") or None)
        return {"ok": True, "text": text, "file": str(out)}
    if action == "term_create":
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
    if action == "term_kill":
        if terms is None or not terms.kill(payload.get("sid", "")):
            return {"error": f"未知會話: {payload.get('sid')}"}
        return {"ok": True}
    if action == "session_result":
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
    return {"error": f"未知 action: {action}"}


# ── WebSocket 最小子集（RFC6455；不做 fragmentation/extension——瀏覽器端
#    terminal 訊息都是小 frame，prototype 夠用）─────────────────────────

_WS_MAGIC = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


def ws_accept_key(key):
    return base64.b64encode(
        hashlib.sha1((key + _WS_MAGIC).encode("ascii")).digest()).decode("ascii")


def ws_encode(data, opcode=2, mask=False):
    """單一 frame。server→client 不 mask；mask=True 給測試當 client 用。"""
    if isinstance(data, str):
        data = data.encode("utf-8")
    head = bytes([0x80 | opcode])
    ln = len(data)
    mbit = 0x80 if mask else 0
    if ln < 126:
        head += bytes([mbit | ln])
    elif ln < 65536:
        head += bytes([mbit | 126]) + struct.pack(">H", ln)
    else:
        head += bytes([mbit | 127]) + struct.pack(">Q", ln)
    if mask:
        key = secrets.token_bytes(4)
        return head + key + bytes(c ^ key[i % 4] for i, c in enumerate(data))
    return head + data


def ws_read_frame(rfile):
    """回傳 (opcode, payload bytes)。連線斷 → ConnectionError。"""
    h = rfile.read(2)
    if len(h) < 2:
        raise ConnectionError("ws closed")
    opcode = h[0] & 0x0F
    masked = h[1] & 0x80
    ln = h[1] & 0x7F
    if ln == 126:
        ln = struct.unpack(">H", rfile.read(2))[0]
    elif ln == 127:
        ln = struct.unpack(">Q", rfile.read(8))[0]
    key = rfile.read(4) if masked else b""
    payload = rfile.read(ln)
    if masked:
        payload = bytes(c ^ key[i % 4] for i, c in enumerate(payload))
    return opcode, payload


def _make_handler(spine_dir):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *a):  # 安靜，不刷終端
            pass

        def _authed(self):
            tok = getattr(self.server, "token", "")
            if not tok:
                return True
            q = parse_qs(urlparse(self.path).query)
            return (self.headers.get("X-Auth") == tok
                    or (q.get("token") or [None])[0] == tok)

        def _json(self, obj, code=200):
            body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
            self.send_response(code)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def _bytes(self, body, ctype):
            self.send_response(200)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            u = urlparse(self.path)
            try:
                if u.path.startswith("/static/"):
                    return self._static(u.path[len("/static/"):])
                if not self._authed():
                    return self._json({"error": "token 不對（重開工作台拿新網址）"}, 403)
                if u.path == "/":
                    self._bytes(PAGE.encode("utf-8"), "text/html; charset=utf-8")
                elif u.path == "/api/state":
                    st = build_state(spine_dir)
                    st["terms"] = self.server.terms.list()
                    st["agents"] = list(_config.load(spine_dir)["agents"])
                    self._json(st)
                elif u.path == "/api/scan":
                    q = parse_qs(u.query, keep_blank_values=True)
                    self._json(build_scan(
                        spine_dir,
                        group=(q.get("group") or [None])[0],
                        repos=(q.get("repos") or [None])[0]))
                elif u.path.startswith("/ws/term/"):
                    self._handle_ws(u.path[len("/ws/term/"):])
                else:
                    self._json({"error": "not found"}, 404)
            except Exception as e:  # 失敗必須浮出，不准白屏
                try:
                    self._json({"error": str(e)}, 500)
                except OSError:
                    pass

        def do_POST(self):
            u = urlparse(self.path)
            try:
                if not self._authed():
                    return self._json({"error": "token 不對"}, 403)
                n = int(self.headers.get("Content-Length") or 0)
                payload = json.loads(self.rfile.read(n) or b"{}")
                if not u.path.startswith("/api/"):
                    return self._json({"error": "not found"}, 404)
                result = _handle_action(spine_dir, u.path[len("/api/"):],
                                        payload, terms=self.server.terms)
                self._json(result, 400 if "error" in result else 200)
            except Exception as e:
                self._json({"error": str(e)}, 500)

        def _static(self, name):
            p = STATIC_DIR / name
            if "/" in name or ".." in name or not p.is_file():
                return self._json({"error": "not found"}, 404)
            ctype = ("text/css" if name.endswith(".css")
                     else "application/javascript; charset=utf-8")
            self._bytes(p.read_bytes(), ctype)

        def _handle_ws(self, sid):
            key = self.headers.get("Sec-WebSocket-Key")
            sess = self.server.terms.get(sid)
            if not key or sess is None:
                return self._json({"error": f"未知會話或非 ws: {sid}"}, 404)
            self.wfile.write((
                "HTTP/1.1 101 Switching Protocols\r\n"
                "Upgrade: websocket\r\nConnection: Upgrade\r\n"
                f"Sec-WebSocket-Accept: {ws_accept_key(key)}\r\n\r\n"
            ).encode("ascii"))
            self.wfile.flush()
            sendlock = threading.Lock()

            def send(data, opcode=2):
                with sendlock:
                    self.connection.sendall(ws_encode(data, opcode))

            # replay 先送、live 輸出經 queue 排在後面——順序不亂
            q = queue.Queue()
            replay = sess.attach(q.put)
            try:
                send(replay)

                def _pump():
                    while True:
                        chunk = q.get()
                        if chunk is None:
                            return
                        try:
                            send(chunk)
                        except OSError:
                            return
                pump = threading.Thread(target=_pump, daemon=True)
                pump.start()
                while True:
                    opcode, payload = ws_read_frame(self.rfile)
                    if opcode == 8:      # close
                        break
                    if opcode == 9:      # ping → pong
                        send(payload, 0xA)
                        continue
                    if opcode in (1, 2) and payload:
                        msg = json.loads(payload)
                        if msg.get("t") == "i":
                            sess.write(msg["d"].encode("utf-8"))
                        elif msg.get("t") == "r":
                            sess.resize(int(msg["c"]), int(msg["r"]))
            except Exception:   # 101 之後只能斷線，不能再回 HTTP 錯誤
                pass
            finally:
                sess.detach(q.put)
                q.put(None)
                self.close_connection = True

    return Handler


def make_server(spine_dir, port=0, token=None):
    """只綁 127.0.0.1（不變式：不對外暴露）＋啟動 token（VDI 多使用者防護）。"""
    srv = ThreadingHTTPServer(("127.0.0.1", port), _make_handler(spine_dir))
    srv.token = secrets.token_hex(16) if token is None else token
    srv.terms = _term.TermManager(spine_dir)
    return srv


def _url(srv):
    return f"http://127.0.0.1:{srv.server_address[1]}/?token={srv.token}"


def _start_timer_if_scheduled(spine_dir):
    if _config.load(spine_dir).get("schedule"):
        threading.Thread(target=_timer.run_loop, args=(spine_dir,),
                         daemon=True).start()
        return True
    return False


def serve_window(spine_dir, port=0, width=1280, height=860):
    """工作台桌面視窗（pywebview 包 OS 內建 webview；預設入口，不開瀏覽器）。"""
    try:
        import webview  # pywebview（optional dependency：pip install pywebview）
    except ImportError:
        raise SystemExit("缺 pywebview——桌面視窗模式需要它：pip install pywebview\n"
                         "（過渡替代：`ui` 子命令走瀏覽器模式）")
    srv = make_server(spine_dir, port)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    if _start_timer_if_scheduled(spine_dir):
        print("timer 已隨工作台啟動（schedule 見 config.yaml）")
    try:
        webview.create_window("repo 工作台", _url(srv),
                              width=width, height=height)
        webview.start()  # 阻塞在主執行緒直到視窗關閉（pywebview 的要求）
    finally:
        srv.terms.kill_all()
        srv.shutdown()
        srv.server_close()


def serve(spine_dir, port=8765, open_browser=True):
    srv = make_server(spine_dir, port)
    url = _url(srv)
    print(f"工作台（瀏覽器模式）：{url}（Ctrl+C 結束）")
    if _start_timer_if_scheduled(spine_dir):
        print("timer 已隨工作台啟動（schedule 見 config.yaml）")
    if open_browser:
        threading.Timer(0.3, lambda: webbrowser.open(url)).start()
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        print("\n工作台已關閉")
    finally:
        srv.terms.kill_all()
        srv.server_close()


PAGE = PAGE_PATH.read_text(encoding="utf-8")
