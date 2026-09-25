"""工作台的畫面資料：/api/state（輕量輪詢）、/api/scan（跑採集）、收件匣卡片、會話清冊。"""
import datetime as _dt
from pathlib import Path

from .. import agentapi as _agentapi
from .. import brief as _brief
from .. import collect as _collect
from .. import config as _config
from .. import notify as _notify
from .. import registry as _registry
from .. import spine as _spine
from .. import summary as _summary


STATIC_DIR = Path(__file__).parent.parent / "static"
PAGE_PATH = Path(__file__).parent.parent / "workbench.html"

# 自己出手的留痕：不是要處理的事，不進收件匣（深看的事件表仍看得到）
_OWN_RECORD_TYPES = ("presented", "chosen")
# cli＝使用者自己從 CLI 改 registry／組（ops.py 留痕）；agent 改的（[agent]）照常進收件匣
_OWN_RECORD_SOURCES = ("monitor", "cli", "route", "spawn", "morning-brief")


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
            actions.append(_act("與 Agent 深入討論", "term_create", kind="agent", task=task,
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


# 會話回報（2026-09-12 檢討 §5）：程序存活只代表程序活著。
# 「工作是否推進」只認脊椎留痕；presented/chosen 是工作台自己的簿記，不算回報。
_REPORT_SKIP_TYPES = ("presented", "chosen")


def _session_scope(term, data):
    """會話範圍 → (group, ids)。認不出來就回 (None, None)＝不限範圍（會照實說明依據）。"""
    scope = term.get("scope")
    groups = {g["name"]: g["members"] for g in data["groups"]}
    if scope in groups:
        return scope, list(groups[scope])
    if scope in {r["id"] for r in data["repos"]}:
        return None, [scope]
    members = list(term.get("scope_repos") or [])
    return None, members or None


def session_report(term, events, data):
    """會話開始後、範圍內的最後一筆脊椎留痕；沒有就 None。

    回傳帶 source／type／時間——使用者要看得出這筆是 agent 寫的還是工作台寫的，
    才判斷得了「現在輪到誰」。不做任何由程序狀態推進度的推論。"""
    started = term.get("started")
    if not started:
        return None
    group, ids = _session_scope(term, data)
    best = None
    for ev in events:
        if ev.type in _REPORT_SKIP_TYPES:
            continue
        if f"{ev.date} {ev.time}" < started:
            continue
        if not _spine.in_scope(ev, group, ids):
            continue
        if best is None or (ev.date, ev.time) >= (best.date, best.time):
            best = ev
    if best is None:
        return None
    first = (best.body or "").splitlines()[0] if best.body else " ".join(best.tokens)
    return {"text": first, "type": best.type, "source": best.source,
            "at": f"{best.date} {best.time}"}


def annotate_terms(spine_dir, terms, events=None, data=None):
    """替會話清冊補上回報狀態（清冊本身由 TermManager 給，這裡只加脊椎那一半）。"""
    events = list(_spine.iter_events(spine_dir)) if events is None else events
    data = data or _registry.load(spine_dir)
    out = []
    for t in terms:
        rec = dict(t)
        if t.get("kind") != "agent":
            rec["report"], rec["report_basis"] = None, "shell 會話不回報"
        else:
            group, ids = _session_scope(t, data)
            rec["report"] = session_report(t, events, data)
            # agent 自己回報的階段（report_status）——只認會話開始後的，舊的不算
            st = _agentapi.get_status(spine_dir, {"group": group, "repos": ids})
            rec["agent_status"] = st if st and st.get("at", "") >= (t.get("started") or "~") else None
            # 依據要說得出口：範圍認得出來就講範圍，認不出來就說沒再過濾
            rec["report_basis"] = (
                f"會話開始後、範圍內（{group or '、'.join(ids)}）的脊椎留痕"
                if (group or ids) else
                "會話開始後的脊椎留痕（此會話範圍是全部 repo，未再依組過濾）")
        out.append(rec)
    return out


def session_draft(spine_dir, term, events=None, data=None):
    """交接草稿＝會話期間的脊椎留痕彙整。是草稿不是成果，尚未寫入任何事件。

    2026-09-12 檢討 §5：「記錄結果」原本是空白表單，使用者得自己翻終端整理。
    這裡只把已經存在的留痕排好給人確認——沒有留痕就直說沒有，不編造成果。"""
    events = list(_spine.iter_events(spine_dir)) if events is None else events
    data = data or _registry.load(spine_dir)
    started = term.get("started")
    group, ids = _session_scope(term, data)
    traces = [ev for ev in events
              if ev.type not in _REPORT_SKIP_TYPES
              and (not started or f"{ev.date} {ev.time}" >= started)
              and _spine.in_scope(ev, group, ids)]
    loops = [ev for ev in _spine.open_loops(spine_dir)
             if _spine.in_scope(ev, group, ids)]
    lines = [f"會話：{term.get('title') or term.get('sid')}"
             f"（範圍 {term.get('scope') or '本機 Shell'}"
             + (f"，開始於 {started}" if started else "") + "）", ""]
    lines.append("完成了什麼（以下是會話期間的脊椎留痕，請確認後再存）：")
    if traces:
        for ev in traces[-8:]:
            first = (ev.body or "").splitlines()[0] if ev.body else " ".join(ev.tokens)
            lines.append(f"- {ev.date} {ev.time} {ev.type} [{ev.source}] {first}")
    else:
        lines.append("- （會話期間沒有留痕，請自己補；下次可請 agent 收尾時呼叫 handoff）")
    lines += ["", "還沒結束的事："]
    if loops:
        for ev in loops[:6]:
            num = next((t for t in ev.tokens if t.startswith("#")), "#?")
            first = (ev.body or "").splitlines()[0] if ev.body else ""
            due = ev.kv("due")
            lines.append(f"- {num}「{first}」" + (f"（due {due}）" if due else ""))
    else:
        lines.append("- （本範圍目前沒有未結事項）")
    lines += ["", "下次從哪裡接回：", "- （請補上）"]
    return {"text": "\n".join(lines), "traces": len(traces), "loops": len(loops),
            "note": "草稿由脊椎留痕組成，尚未儲存；存檔會落一筆 decision，不代表成果已被採納。"}


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
        "groups": [{"name": g["name"], "members": g["members"],
                    "goal": g.get("goal") or "", "goal_at": g.get("goal_at") or ""}
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


# defer 的七天提醒過濾搬到 brief.snoozed（agent 介面 next_work 也要同一份規則）
_snoozed = _brief.snoozed


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
        # 本組工作摘要（§3）：目標／上次進度／變化／可接續的一步，每格帶來源
        "summary": _summary.build_summary(spine_dir, group=group, repos=repos,
                                          states=states, cards=cards),
        # 活動脈動（2026-09-08）：組的監控焦點＝活動頻繁度＋近期 commit 內容
        "pulse": _collect.group_pulse(states),
        "recent_commits": _collect.recent_across(states, limit=30),
        "question_cards": cards,
        "questions": [_brief.card_text(c) for c in cards],
        "quiet": max(quiet, 0),
        # registry 稽核＋脊椎衛生迴圈併同一個紅字面（second-brain lint 課）
        "audit": _registry.audit(spine_dir) + _spine.lint(spine_dir),
    }
