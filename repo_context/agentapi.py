"""Agent 介面（2026-09-25 Agent 友善輪）——agent 是主要使用者，不是被請進工作台的客人。

之前的問題（檢討見 docs/20260925_Agent友善輪.md）：
- agent 只有從工作台開、cwd 在 spine 時才拿得到脈絡；在某個 repo 直接開 agent 什麼都沒有。
- MCP 工具照規格編號（P1–P16）切，不照 agent 的工作切；留痕要自己拼 kv 文法。
- 輸出是給人看的中文表格，沒有可引用的 id；範圍只是 prompt 裡的約定。

這一層把「agent 一次工作」需要的動作收成幾個函數，全部回 JSON-able dict：
    where_am_i → context_for → next_work → log_decision / add_todo / close_todo
    → report_status → handoff；另有 search_knowledge（知識 ↔ repo 相關度）。
跨 repo 參考輪（同日第二輪）加：resolve_ref／read_from／ask_repo／repo_status——
讀別的 repo 用名字（`<repo>:<export>/子路徑`），讀了會記引用；搜尋依主場的關係加分。
MCP 工作層、CLI（--json）、Claude Code hooks 都呼叫這裡——同一份邏輯，三個入口。

範圍紀律（從約定變成機制）：
- 沒指定範圍＝依 cwd 推出「主場」（所在 repo 所屬的組；不在任何組＝該 repo 自己）。
- 查詢預設只回主場範圍。
- 寫入主場以外的範圍會被拒（AgentError code=cross_scope），要先問使用者、
  確認後帶 cross_scope_ok=true 才寫得進去。
"""
import datetime as _dt
import json
import re
from pathlib import Path

from . import brief as _brief
from . import collect as _collect
from . import config as _config
from . import context as _context
from . import crossref as _xref
from . import repocard as _repocard
from . import knowledge as _knowledge
from . import registry, spine
from . import summary as _summary


class AgentError(ValueError):
    """給 agent 的結構化錯誤：code 可程式判斷，hint 告訴它下一步怎麼修。"""

    def __init__(self, code, message, hint=None):
        super().__init__(message)
        self.code, self.message, self.hint = code, message, hint

    def to_dict(self):
        out = {"ok": False, "error": self.code, "message": self.message}
        if self.hint:
            out["hint"] = self.hint
        return out


# ---------------------------------------------------------------- 定位與範圍

def _resolve(p):
    try:
        return Path(p).expanduser().resolve()
    except OSError:
        return Path(p).expanduser().absolute()


def where_am_i(spine_dir, cwd=None):
    """cwd → 所在 repo、所屬組、主場範圍。認不出來也回 dict（registered=false＋hint）。"""
    here = _resolve(cwd or Path.cwd())
    data = registry.load(spine_dir)
    best = None
    for r in data["repos"]:
        root = _resolve(r["path"])
        if here == root or root in here.parents:
            if best is None or len(str(root)) > len(str(_resolve(best["path"]))):
                best = r
    groups = [g["name"] for g in data["groups"]
              if best and best["id"] in g["members"]]
    in_spine = here == _resolve(spine_dir) or _resolve(spine_dir) in here.parents
    out = {"cwd": str(here), "spine": str(_resolve(spine_dir)),
           "registered": best is not None, "in_spine": in_spine,
           "repo": None, "groups": groups, "group": groups[0] if groups else None}
    if best:
        out["repo"] = {"id": best["id"], "path": best["path"],
                       "type": best.get("type", "mine"),
                       "tier": best.get("tier", "active")}
        if len(groups) > 1:
            out["note"] = (f"此 repo 屬於多個組（{'、'.join(groups)}），預設用第一個；"
                           "要換組就在呼叫時帶 group。")
        elif not groups:
            out["note"] = "此 repo 還沒編進任何組，範圍只有它自己。"
    elif in_spine:
        out["note"] = "cwd 在 spine repo 裡：沒有主場範圍，查詢預設是全部 repo。"
    else:
        out["hint"] = ("這個目錄沒登記在 registry。可請使用者跑 "
                       f"`ctx registry add <id> {here}`，或在工作台首頁掃描登記。")
    return out


def _scope_of(spine_dir, where, group=None, repos=None):
    """回傳 scope dict：{group, repos, label, explicit}。group/repos 都沒給＝主場。"""
    if isinstance(repos, str):
        repos = [r.strip() for r in repos.split(",") if r.strip()]
    if repos:
        gname, entries = registry.resolve_group(spine_dir, None, repos)
        return {"group": None, "repos": [e["id"] for e in entries],
                "label": gname, "explicit": True}
    if group:
        _, entries = registry.resolve_group(spine_dir, group)
        return {"group": group, "repos": [e["id"] for e in entries],
                "label": group, "explicit": True}
    if where.get("group"):
        g = where["group"]
        _, entries = registry.resolve_group(spine_dir, g)
        return {"group": g, "repos": [e["id"] for e in entries],
                "label": g, "explicit": False}
    if where.get("repo"):
        rid = where["repo"]["id"]
        return {"group": None, "repos": [rid], "label": rid, "explicit": False}
    return {"group": None, "repos": None, "label": "全部", "explicit": False}


def _home_ids(spine_dir, where):
    """主場涵蓋的 repo id：所在 repo 所屬的全部組的成員（沒組＝它自己）。"""
    if not where.get("repo"):
        return None
    ids = {where["repo"]["id"]}
    data = registry.load(spine_dir)
    for g in data["groups"]:
        if g["name"] in where["groups"]:
            ids.update(g["members"])
    return ids


def _check_write_scope(spine_dir, where, scope, cross_scope_ok):
    """寫入主場以外＝拒寫（除非使用者已確認）。主場不明（spine 內／未登記）時不擋。"""
    if cross_scope_ok or not scope["explicit"]:
        return
    home = _home_ids(spine_dir, where)
    if home is None:
        return
    if scope["group"] and scope["group"] in where["groups"]:
        return
    if scope["repos"] and set(scope["repos"]) <= home:
        return
    raise AgentError(
        "cross_scope",
        f"要寫入的範圍「{scope['label']}」不在你的主場"
        f"（{where['repo']['id']}／{'、'.join(where['groups']) or '無組'}）。",
        "先問使用者是否要跨組留痕；使用者同意後再帶 cross_scope_ok=true 重試。")


def _tokens_for(scope, where):
    toks = []
    if scope["group"]:
        toks.append(f"group:{scope['group']}")
    rid = (where.get("repo") or {}).get("id")
    if rid and (scope["repos"] is None or rid in scope["repos"]):
        toks.append(f"repo:{rid}")
    elif scope["repos"] and len(scope["repos"]) == 1:
        toks.append(f"repo:{scope['repos'][0]}")
    return toks


def _entries(spine_dir, scope):
    if scope["repos"] is None:
        return list(registry.load(spine_dir)["repos"])
    return [registry.get_repo(spine_dir, i) for i in scope["repos"]]


def _in(ev, scope):
    if scope["group"] is None and scope["repos"] is None:
        return True
    return spine.in_scope(ev, scope["group"], scope["repos"])


# ---------------------------------------------------------------- 事件的 JSON 形狀

def _first(ev):
    return (ev.body or "").splitlines()[0] if ev.body else " ".join(ev.tokens)


def _num(ev):
    return next((t for t in ev.tokens if t.startswith("#")), None)


def event_ref(ev):
    """可放進 ref: token 的鍵（lint 認得）：有 id 用 id，沒有用 HH:MM。"""
    return f"{ev.type}:{ev.kv('id') or ev.time}"


def event_dict(ev):
    return {"ref": event_ref(ev), "date": ev.date, "time": ev.time,
            "type": ev.type, "source": ev.source, "group": ev.kv("group"),
            "repo": ev.kv("repo"), "text": _first(ev), "body": ev.body}


def loop_dict(ev, today=None):
    today = today or f"{_dt.date.today():%Y-%m-%d}"
    due = ev.kv("due")
    return {"id": _num(ev), "text": _first(ev), "due": due,
            "overdue": bool(due and due < today), "group": ev.kv("group"),
            "repo": ev.kv("repo"), "opened": f"{ev.date} {ev.time}",
            "source": ev.source}


def open_loops(spine_dir, scope):
    return [ev for ev in spine.open_loops(spine_dir) if _in(ev, scope)]


def _last_handoff(spine_dir, scope):
    best = None
    for ev in spine.iter_events(spine_dir):
        if ev.type == "decision" and (ev.body or "").startswith("交接") and _in(ev, scope):
            best = ev
    if best is None:
        return None
    fields = {}
    for ln in best.body.splitlines()[1:]:
        if "：" in ln:
            k, v = ln.split("：", 1)
            fields[k.strip()] = v.strip()
    return {"ref": event_ref(best), "at": f"{best.date} {best.time}",
            "source": best.source, "summary": _first(best)[3:].strip(),
            "done": fields.get("完成"), "remaining": fields.get("剩下"),
            "next_step": fields.get("下次從這裡接")}


# ---------------------------------------------------------------- 讀

def context_for(spine_dir, cwd=None, group=None, repos=None, card=True):
    """開場一次拿齊：我在哪、範圍、目標、上次交接、下一步、未結、最近事件、agent 狀態。"""
    where = where_am_i(spine_dir, cwd)
    scope = _scope_of(spine_dir, where, group, repos)
    entries = _entries(spine_dir, scope)
    states = _collect.collect_group(entries, spine_dir)
    snoozed = _brief.snoozed(spine_dir)
    cards = [c for c in _brief.build_question_cards(
        states, _config.load(spine_dir)["thresholds"])
        if (c["repo"], c["qkind"]) not in snoozed]
    summ = _summary.build_summary(spine_dir, group=scope["group"],
                                  repos=None if scope["group"] else scope["repos"],
                                  states=states, cards=cards)
    evs = [ev for ev in spine.iter_events(spine_dir)
           if ev.type != "presented" and _in(ev, scope)]
    out = {
        "ok": True,
        "where": where,
        "scope": {k: scope[k] for k in ("group", "repos", "label")},
        "goal": summ["goal"], "goal_gap": summ["goal_gap"],
        "last_progress": summ["progress"],
        "last_handoff": _last_handoff(spine_dir, scope),
        "next": summ["next"], "next_gap": summ["next_gap"],
        "changes": summ["changes"],
        "open_loops": [loop_dict(ev) for ev in open_loops(spine_dir, scope)],
        "recent_events": [event_dict(ev) for ev in evs[-8:][::-1]],
        "relations": registry.relation_lines(spine_dir, [e["id"] for e in entries]),
        "pulse": _collect.pulse_line(_collect.group_pulse(states)),
        "agent_status": get_status(spine_dir, scope),
        "attention": [{"repo": c["repo"], "kind": _brief.qkind_name(c["qkind"]),
                       "text": c["title"]} for c in cards],
    }
    rid = (where.get("repo") or {}).get("id")
    out["neighbors"] = _xref.neighbors(spine_dir, rid) if rid else []
    out["drift"] = _xref.drift(spine_dir, consumer=rid) if rid else []
    if card:
        out["card"] = _context.build_context(
            spine_dir, scope["group"],
            None if scope["group"] else scope["repos"], states=states)
    return out


def next_work(spine_dir, cwd=None, group=None, repos=None, limit=5):
    """接下來值得做的事，依序：逾期未結 → 上次交接的下一步 → 有 due 的未結 → 其他未結 → 需要注意的 repo。
    每項都帶 why（依據），agent 要說得出為什麼是這件。"""
    where = where_am_i(spine_dir, cwd)
    scope = _scope_of(spine_dir, where, group, repos)
    today = f"{_dt.date.today():%Y-%m-%d}"
    loops = [loop_dict(ev, today) for ev in open_loops(spine_dir, scope)]
    items = []
    for lp in sorted((lp for lp in loops if lp["overdue"]), key=lambda x: x["due"]):
        items.append({"kind": "todo", "id": lp["id"], "text": lp["text"],
                      "due": lp["due"], "why": f"未結事項已逾期（due {lp['due']}）"})
    ho = _last_handoff(spine_dir, scope)
    if ho and ho.get("next_step"):
        items.append({"kind": "handoff", "ref": ho["ref"], "text": ho["next_step"],
                      "why": f"上次交接（{ho['at']}，{ho['source']}）留下的下一步"})
    rest = [lp for lp in loops if not lp["overdue"]]
    for lp in sorted(rest, key=lambda x: (x["due"] is None, x["due"] or "")):
        why = f"未結事項，due {lp['due']}" if lp["due"] else "未結事項（沒有 due）"
        items.append({"kind": "todo", "id": lp["id"], "text": lp["text"],
                      "due": lp["due"], "why": why})
    if len(items) < limit:
        states = _collect.collect_group(_entries(spine_dir, scope), spine_dir)
        snoozed = _brief.snoozed(spine_dir)
        for c in _brief.build_question_cards(states, _config.load(spine_dir)["thresholds"]):
            if (c["repo"], c["qkind"]) in snoozed:
                continue
            items.append({"kind": "attention", "repo": c["repo"], "text": c["title"],
                          "why": f"採集閾值：{_brief.qkind_name(c['qkind'])}"})
    goal = None
    if scope["group"]:
        try:
            goal = registry.get_group(spine_dir, scope["group"]).get("goal")
        except ValueError:
            goal = None
    out = {"ok": True, "scope": {k: scope[k] for k in ("group", "repos", "label")},
           "goal": goal, "items": items[:limit]}
    if not items:
        out["note"] = "沒有逾期或未結事項，也沒有觸發提醒的 repo；下一步由使用者決定。"
    return out


def _search_boosts(spine_dir, where):
    """主場 repo 的鄰居加分、鄰居 export 內的檔再加分——「上游的書摘」該排在不相關 repo 前面。"""
    rid = (where.get("repo") or {}).get("id")
    if not rid:
        return {}
    repos, paths = {}, []
    for n in _xref.neighbors(spine_dir, rid):
        repos[n["peer"]] = (1.5, f"跟 {rid} 有關係（{n['peer']} {n['label']}）")
        for name, v in n["exports"].items():
            if n.get("uses") and name not in n["uses"]:
                continue
            prefix = v["path"].rstrip("/")
            paths.append((n["peer"], "" if prefix == "." else prefix + "/", 1.2,
                          f"在 {n['peer']}:{name} 內"))
    return {"repos": repos, "paths": paths}


def search_knowledge(spine_dir, query, cwd=None, group=None, repos=None,
                     everywhere=True, limit=8, exclude_paths=(), code=True):
    """知識 ↔ repo 相關度（L1 定位）。預設搜全部已登記 repo（知識本來就跨組）；everywhere=false 只搜主場。
    md 走 BM25（標題加權、依主場關係加分），程式碼走 git grep（code=false 可關）。
    每筆結果帶 why；要讀內容用 read_from（ref 已附在結果裡）。"""
    if not (query or "").strip():
        raise AgentError("bad_request", "query 不能是空的", "給一段文字、一個想法或一段卡片內容。")
    where = where_am_i(spine_dir, cwd)
    if everywhere and not (group or repos):
        entries = registry.load(spine_dir)["repos"]
        label = "全部"
    else:
        scope = _scope_of(spine_dir, where, group, repos)
        label = scope["label"]
        entries = _entries(spine_dir, scope)
    res = _knowledge.search(entries, query, limit=limit, spine_dir=spine_dir,
                            exclude_paths=exclude_paths, boosts=_search_boosts(spine_dir, where))
    for f in res["files"]:
        f["ref"] = f"{f['repo']}:{f['file']}"
    out = {"ok": True, "scope": label, **res}
    if code:
        hits = _xref.code_search(entries, query, limit=max(3, limit // 2))
        for h in hits:
            h["ref"] = f"{h['repo']}:{h['file']}"
        out["code"] = hits
    return out


def _ref_call(fn, *a, **kw):
    try:
        return fn(*a, **kw)
    except _xref.RefError as e:
        raise AgentError(e.code, e.message, e.hint) from e
    except ValueError as e:
        raise AgentError("bad_request", str(e)) from e


def resolve_ref(spine_dir, ref):
    """名字 → 實際路徑（`<repo>`、`<repo>:<export>[/子路徑]`、`<repo>:<相對路徑>`）。不記引用。"""
    return _ref_call(_xref.resolve, spine_dir, ref)


def read_from(spine_dir, ref, lines=None, cwd=None):
    """讀別的 repo 的檔案或目錄（L2）。回傳附 commit；讀了會記一筆引用（主場 repo → 對方）。
    讀跨組 repo 不用先問——只讀、留紀錄；寫入才受範圍限制。"""
    where = where_am_i(spine_dir, cwd)
    consumer = (where.get("repo") or {}).get("id")
    return _ref_call(_xref.read, spine_dir, ref, consumer=consumer, lines=lines)


def ask_repo(spine_dir, repo, question, cwd=None, provider=None):
    """在對方 repo 開一個唯讀 agent 回答（L3，幾十秒、花錢）：需要整理歸納的問題才用，
    找單一檔案用 search_knowledge＋read_from。回傳結論＋引用（repo 內路徑），引用會記下來。"""
    where = where_am_i(spine_dir, cwd)
    consumer = (where.get("repo") or {}).get("id")
    return _ref_call(_xref.ask_repo, spine_dir, repo, question, consumer=consumer,
                     provider=provider)


def repo_status(spine_dir, repo=None, cwd=None, since=None):
    """repo 狀態卡（agent 版：不推使用者的已讀水位線）。repo 省略＝cwd 所在 repo。"""
    if not repo:
        where = where_am_i(spine_dir, cwd)
        if not where.get("repo"):
            raise AgentError("bad_request", "cwd 不在已登記的 repo 裡，請指定 repo",
                             where.get("hint") or where.get("note"))
        repo = where["repo"]["id"]
    return _ref_call(_repocard.build, spine_dir, repo, since=since or "recent", mark=False)


# ---------------------------------------------------------------- 寫

def _next_id(spine_dir, date_str, prefix):
    used = {ev.kv("id") for ev in spine.iter_events(spine_dir) if ev.kv("id")}
    n = 1
    while f"{date_str}-{prefix}{n}" in used:
        n += 1
    return f"{date_str}-{prefix}{n}"


def _next_loop_num(spine_dir):
    nums = [int(t[1:]) for ev in spine.iter_events(spine_dir) if ev.type == "open-loop"
            for t in ev.tokens if re.fullmatch(r"#\d+", t)]
    return f"#{max(nums, default=0) + 1}"


def _clean(text, field="text"):
    text = (text or "").strip()
    if not text:
        raise AgentError("bad_request", f"{field} 不能是空的")
    # 事件邊界是 '## ' 開頭行——內文裡的 markdown 標題降一級，不讓 validator 拒寫
    return "\n".join(("#" + ln if ln.startswith("## ") else ln) for ln in text.splitlines())


def _append(spine_dir, etype, source, tokens, body):
    try:
        return spine.append_event(spine_dir, etype, source, tokens, body=body)
    except spine.ValidationError as e:
        raise AgentError("invalid_event", str(e), "修正內容後重試。") from e


def log_decision(spine_dir, text, cwd=None, group=None, repos=None, ref=None,
                 cross_scope_ok=False, source="agent"):
    """記一筆判斷（decision）。回傳 ref，之後可以用它引用這筆。"""
    where = where_am_i(spine_dir, cwd)
    scope = _scope_of(spine_dir, where, group, repos)
    _check_write_scope(spine_dir, where, scope, cross_scope_ok)
    now = _dt.datetime.now()
    eid = _next_id(spine_dir, f"{now:%Y-%m-%d}", "d")
    toks = [f"id:{eid}"] + _tokens_for(scope, where)
    if ref:
        if not re.fullmatch(r"[a-z-]+:\S+", ref):
            raise AgentError("bad_request", f"ref 格式不對：{ref!r}",
                             "用其他工具回傳的 ref，例如 decision:2026-09-25-d1 或 collision:2026-09-25-a。")
        toks.append(f"ref:{ref}")
    ev = _append(spine_dir, "decision", source, toks, _clean(text))
    return {"ok": True, "ref": event_ref(ev), "event": event_dict(ev)}


def add_todo(spine_dir, text, due=None, cwd=None, group=None, repos=None,
             cross_scope_ok=False, source="agent"):
    """開一個未結事項（open-loop）。編號自動配；沒給 due 用 config 的預設天數。"""
    where = where_am_i(spine_dir, cwd)
    scope = _scope_of(spine_dir, where, group, repos)
    _check_write_scope(spine_dir, where, scope, cross_scope_ok)
    if due and not re.fullmatch(r"\d{4}-\d{2}-\d{2}", due):
        raise AgentError("bad_request", f"due 需為 YYYY-MM-DD：{due!r}")
    num = _next_loop_num(spine_dir)
    toks = [num] + ([f"due:{due}"] if due else []) + _tokens_for(scope, where)
    body = _clean(text)
    toks = spine.default_due_tokens(
        toks, body, _config.load(spine_dir)["thresholds"]["openloop_default_due_days"])
    ev = _append(spine_dir, "open-loop", source, toks, body)
    return {"ok": True, "id": num, "todo": loop_dict(ev)}


def close_todo(spine_dir, id, note="", cwd=None, cross_scope_ok=False, source="agent"):
    """關掉一個未結事項。只能關主場範圍內的（跨範圍要使用者確認）。"""
    id = id if str(id).startswith("#") else f"#{id}"
    cur = next((ev for ev in spine.open_loops(spine_dir) if id in ev.tokens), None)
    if cur is None:
        raise AgentError("not_found", f"找不到未結事項 {id}（不存在或已關）",
                         "用 next_work 或 context_for 看目前的未結清單。")
    where = where_am_i(spine_dir, cwd)
    home = _home_ids(spine_dir, where)
    if home is not None and not cross_scope_ok:
        g, r = cur.kv("group"), cur.kv("repo")
        if not ((g and g in where["groups"]) or (r and r in home)):
            raise AgentError("cross_scope", f"{id} 屬於「{g or r or '全域'}」，不在你的主場。",
                             "先問使用者；同意後帶 cross_scope_ok=true 重試。")
    toks = [id] + [t for t in cur.tokens if t.startswith(("repo:", "group:"))]
    body = "closed → " + (_clean(note, "note") if note else "完成")
    ev = _append(spine_dir, "open-loop", source, toks, body)
    return {"ok": True, "id": id, "closed": _first(cur), "event": event_dict(ev)}


STATUSES = ("working", "waiting", "blocked", "done")


def _status_dir(spine_dir):
    return Path(spine_dir) / ".state" / "agent_status"


def _status_key(scope):
    raw = scope["group"] or ",".join(scope["repos"] or []) or "全部"
    return re.sub(r"[^\w\-㐀-鿿]+", "_", raw)


def report_status(spine_dir, status, text="", cwd=None, group=None, repos=None):
    """回報目前階段：working／waiting（等使用者回應）／blocked（卡住）／done。
    這是會一直被覆寫的「現況」，不是歷史；waiting 與 blocked 另留一筆事件讓使用者在收件匣看到。"""
    if status not in STATUSES:
        raise AgentError("bad_request", f"status 需為 {'／'.join(STATUSES)}：{status!r}")
    where = where_am_i(spine_dir, cwd)
    scope = _scope_of(spine_dir, where, group, repos)
    now = _dt.datetime.now()
    rec = {"status": status, "text": (text or "").strip(), "at": f"{now:%Y-%m-%d %H:%M}",
           "scope": scope["label"], "group": scope["group"], "repos": scope["repos"],
           "repo": (where.get("repo") or {}).get("id")}
    d = _status_dir(spine_dir)
    d.mkdir(parents=True, exist_ok=True)
    (d / f"{_status_key(scope)}.json").write_text(
        json.dumps(rec, ensure_ascii=False), encoding="utf-8")
    out = {"ok": True, "status": rec}
    if status in ("waiting", "blocked"):
        label = "等你回應" if status == "waiting" else "卡住了"
        ev = _append(spine_dir, "suggestion", "agent", _tokens_for(scope, where),
                     f"agent {label}：{_clean(text or label)}")
        out["event"] = event_dict(ev)
    return out


def get_status(spine_dir, scope):
    try:
        return json.loads((_status_dir(spine_dir) / f"{_status_key(scope)}.json")
                          .read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None


def all_statuses(spine_dir):
    d = _status_dir(spine_dir)
    out = []
    for p in sorted(d.glob("*.json")) if d.is_dir() else []:
        try:
            out.append(json.loads(p.read_text(encoding="utf-8")))
        except (OSError, ValueError):
            continue
    return out


def handoff(spine_dir, done, next_step, remaining=None, cwd=None, group=None,
            repos=None, cross_scope_ok=False, source="agent"):
    """收尾交接：完成了什麼、下次從哪接；remaining 每一項各開一個未結事項（只放新的）。
    落一筆 decision（第一行「交接：…」）——工作摘要的「上次進度」與 next_work 都讀它。"""
    where = where_am_i(spine_dir, cwd)
    scope = _scope_of(spine_dir, where, group, repos)
    _check_write_scope(spine_dir, where, scope, cross_scope_ok)
    done = _clean(done, "done")
    next_step = _clean(next_step, "next_step")
    remaining = [r for r in (remaining or []) if (r or "").strip()]
    todos = [add_todo(spine_dir, r, cwd=cwd, group=group, repos=repos,
                      cross_scope_ok=True, source=source)["id"] for r in remaining]
    now = _dt.datetime.now()
    eid = _next_id(spine_dir, f"{now:%Y-%m-%d}", "d")
    head = done.splitlines()[0][:80]
    body = "\n".join([
        f"交接：{head}",
        f"完成：{' '.join(done.splitlines())}",
        "剩下：" + ("；".join(f"{n} {r.strip()}" for n, r in zip(todos, remaining))
                   if remaining else "無"),
        f"下次從這裡接：{' '.join(next_step.splitlines())}",
    ])
    ev = _append(spine_dir, "decision", source,
                 [f"id:{eid}"] + _tokens_for(scope, where), body)
    report_status(spine_dir, "done", head, cwd=cwd, group=group, repos=repos)
    return {"ok": True, "ref": event_ref(ev), "todos": todos, "event": event_dict(ev)}
