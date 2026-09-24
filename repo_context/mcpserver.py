"""MCP server——兩層工具（2026-09-25 Agent 友善輪）。

**工作層（預設）**：照 agent 一次工作的節奏切——where_am_i／context_for／next_work／
log_decision／add_todo／close_todo／report_status／handoff／search_knowledge。
依 server 的 cwd（或參數 cwd）推出主場範圍；回傳 JSON（帶可引用的 ref／id）；
錯誤是結構化的 {ok:false, error, message, hint}；寫入主場以外會被拒（cross_scope）。
邏輯全在 agentapi.py，CLI `--json` 與 hooks 同源。

**管理層（`ctx mcp --admin` 或 config `mcp.admin: true` 才開）**：registry／組／採集／
打包／碰撞／升格等維護工具——給人（或人明確授權的 agent）用，預設不塞給 agent。

stdio 傳輸（newline-delimited JSON-RPC 2.0），stdlib 實作零依賴（耗材原則）。
拒寫去向（P10 檢討④）：validator 錯誤**原樣**回給 agent（isError tool result）供自我修正；
dead-letter 是人的入口（hotkey）專屬，MCP 不走。
"""
import datetime as _dt
import json
import os
import sys

from . import agentapi as _api
from . import brief as _brief
from . import collect as _collect
from . import context as _context
from . import collide as _collide
from . import config as _config
from . import digest as _digest
from . import notify as _notify
from . import pack as _pack
from . import registry as _registry
from . import route as _route
from . import spawn as _spawn
from . import spine as _spine
from . import upstream as _upstream

PROTOCOL_VERSION = "2024-11-05"


def _ev_lines(events, verbose=False):
    out = []
    for ev in events:
        out.append(f"{ev.date} {ev.header()}")
        if verbose and ev.body:
            out.append("   " + ev.body.replace("\n", "\n   "))
    return "\n".join(out) if out else "（無）"


# ---- tool handlers（回傳字串；raise＝isError，訊息原樣給 agent）----

def _t_registry_list(d, a):
    rows = [f"{r['id']}\t{r.get('type', 'mine')}\t{r.get('tier', '?')}\t{r['path']}"
            for r in _registry.load(d)["repos"]]
    return "\n".join(rows) or "（registry 空）"


def _t_registry_add(d, a):
    e = _registry.add_repo(d, a["id"], a["path"], type=a.get("type", "mine"),
                           tags=a.get("tags"), tier=a.get("tier", "active"),
                           upstream=a.get("upstream"))
    return f"已登記：{e['id']} ({e['type']}/{e['tier']})"


def _t_registry_audit(d, a):
    reds = _registry.audit(d, scan_dirs=a.get("scan_dirs") or [])
    return "\n".join(f"[RED] {r}" for r in reds) if reds else "audit 零紅字 OK"


def _t_registry_scan(d, a):
    if a.get("apply"):
        added = _registry.scan_register(d, a["dir"])
        return ("已登記：" + ", ".join(added)
                + "（上半身 type/tier/tags 待人補）") if added else "無新 repo 可登記"
    cands = _registry.scan_dir(d, a["dir"])
    return "\n".join(f"[候選] {cid}: {p}" for p, cid in cands) \
        or "無未登記 repo"


def _t_spine_lint(d, a):
    reds = _spine.lint(d)
    return "\n".join(f"[RED] {r}" for r in reds) if reds \
        else "脊椎 lint 零紅字 OK（衛生迴圈）"


def _t_pack_estimate(d, a):
    _, entries = _registry.resolve_group(d, a.get("group"), a.get("repos"))
    rows = _pack.estimate(entries)
    total = sum(r["tokens"] for r in rows)
    budget = _config.load(d)["pack"]["token_budget"]
    lines = [f"{r['id']}\t{r['files']} 檔\t~{r['tokens']} tokens"
             for r in sorted(rows, key=lambda x: -x["tokens"])]
    lines.append(f"合計 ~{total} tokens／預算 {budget} → "
                 + ("裝得下" if total <= budget else "超預算（挑細一點）"))
    return "\n".join(lines)


def _t_registry_relate(d, a):
    e = _registry.relate(d, a["a"], a["b"], a["kind"], note=a.get("note"))
    k = _registry.relation_kinds(d)[a["kind"]]
    return f"已設關係：{a['a']} {k['forward']}→ {a['b']}（{e['kind']}）"


def _t_registry_relations(d, a):
    if a.get("id"):
        rows = _registry.relations_of(d, a["id"])
        out = [(f"{a['id']} {r['label']}→ {r['peer']}" if r["direction"] == "out"
                else f"{a['id']} 的{r['label']} {r['peer']}") + f"（{r['kind']}）"
               + (f"  {r['note']}" if r.get("note") else "") for r in rows]
    else:
        kinds = _registry.relation_kinds(d)
        out = [f"{r['from']} {kinds.get(r['kind'], {}).get('forward', r['kind'])}→ {r['to']}"
               f"（{r['kind']}）" + (f"  {r['note']}" if r.get("note") else "")
               for r in _registry.relations(d)]
    return "\n".join(out) or ("（無關係）可用 kind：" + ", ".join(_registry.relation_kinds(d)))


def _t_group_context(d, a):
    return _context.build_context(d, a.get("group"), a.get("repos"))


def _t_group_list(d, a):
    rows = [f"{g['name']}\t{','.join(g['members'])}"
            for g in _registry.load(d)["groups"]]
    return "\n".join(rows) or "（無組）"


def _t_group_add(d, a):
    _registry.add_group(d, a["name"], a["members"])
    return f"組已建：{a['name']}"


def _t_collect(d, a):
    _, entries = _registry.resolve_group(d, a.get("group"), a.get("repos"))
    return _collect.format_table(_collect.collect_group(entries, d))


def _t_pack(d, a):
    _, entries = _registry.resolve_group(d, a.get("group"), a.get("repos"))
    budget = a.get("budget") or _config.load(d)["pack"]["token_budget"]
    text, inc, exc, flagged = _pack.pack_group(entries, budget)
    return (f"<!-- 收錄 {len(inc)}、未納入 {len(exc)}、"
            f"疑似機密擋下 {len(flagged)} -->\n{text}")


def _t_brief(d, a):
    _, text = _brief.run(d, a.get("group"), a.get("repos"))
    return text


def _t_spine_append(d, a):
    tokens = list(a.get("kv") or [])
    if a["type"] == "open-loop":
        due_days = _config.load(d)["thresholds"]["openloop_default_due_days"]
        tokens = _spine.default_due_tokens(tokens, a.get("body"), due_days)
    ev = _spine.append_event(d, a["type"], a["source"], tokens,
                             body=a.get("body", ""))
    return f"已寫入：{ev.header()}"


def _t_spine_query(d, a):
    date = f"{_dt.date.today():%Y-%m-%d}" if a.get("today") else a.get("date")
    evs = _spine.query(d, type=a.get("type"), date=date, group=a.get("group"))
    return _ev_lines(evs, verbose=a.get("verbose", False))


def _t_spine_stats(d, a):
    s = _spine.stats(d)
    sv = _registry.survival(d)
    rate = f"{s['hit_rate']:.0%}" if s["hit_rate"] is not None else "n/a"
    srate = f"{sv['survival_rate']:.0%}" if sv["survival_rate"] is not None else "n/a"
    return (f"碰撞 {s['collisions']} 次｜outcome 回連 {s['collisions_with_outcome']} 次"
            f"｜靈感命中率 {rate}｜生出 repo {sv['spawned']} 個"
            f"｜存活 {sv['alive']}｜存活率 {srate}")


def _t_open_loops(d, a):
    # 範圍跟簡報／情境卡同一份規則（以前這裡回全部組的未結）
    evs = _spine.open_loops(d)
    if a.get("group") or a.get("repos"):
        _, entries = _registry.resolve_group(d, a.get("group"), a.get("repos"))
        ids = [e["id"] for e in entries]
        evs = [ev for ev in evs if _spine.in_scope(ev, a.get("group"), ids)]
    return _ev_lines(evs, verbose=True)


def _t_unread(d, a):
    p = _notify.pending(d)
    text = _notify.render(p) + "\n" + _ev_lines(p["interrupt"] + p["normal"])
    if a.get("ack"):
        _spine.ack_unread(d)
        text += "\n（已標記全部已讀）"
    return text


def _t_collide_submit(d, a):
    cid = _collide.submit(d, a["idea"], group=a.get("group"),
                          repos=a.get("repos"), source="agent")
    if a.get("wait"):
        j = _collide.run_judgement(d, cid, provider=a.get("provider"))
        return f"cid={cid}\n" + (json.dumps(j, ensure_ascii=False, indent=1)
                                 if j else "判定失敗（system-unsure）——已落脊椎浮出")
    _collide.spawn_detached(d, cid, provider=a.get("provider"))
    return f"cid={cid}（判定走 detached，結果看 unread）"


def _t_route(d, a):
    p = _route.route(d, a["text"], a["dest"], title=a.get("title", "idea"))
    return f"已落：{p}"


def _t_upstream_check(d, a):
    n, findings = _upstream.check(d, group=a.get("group"), repos=a.get("repos"))
    if not n:
        return "無 external repo 可查（registry 的 external 需帶 upstream 欄位）"
    lines = [f"{f['repo']}: {f['kind']} {f['id']} {f['title']}" for f in findings]
    return "\n".join(lines) if lines else f"查了 {n} 個 external，上游無新事"


def _t_digest_run(d, a):
    results = _digest.run(d, group=a.get("group"), repos=a.get("repos"),
                          provider=a.get("provider"))
    return "\n".join(f"{r['repo']}: {r['note']}"
                     + (f" → {r['file']}" if r["file"] else "")
                     for r in results) or "（組內無 mine repo）"


def _t_spawn(d, a):
    p = _spawn.spawn(d, a["id"], a["path"],
                     from_incubator=a.get("from_incubator"), origin=a.get("origin"))
    return f"已升格：{a['id']} → {p}"


def _s(**props):
    """inputSchema 簡寫。"""
    return {"type": "object",
            "properties": {k: v for k, v in props.items() if k != "_req"},
            "required": props.get("_req", [])}


def _d(base, desc, **extra):
    return {**base, "description": desc, **extra}


_STR = {"type": "string"}
_BOOL = {"type": "boolean"}
_INT = {"type": "integer"}
_ARR = {"type": "array", "items": {"type": "string"}}
_SCOPE = {"group": _d(_STR, "組名；省略＝依 cwd 推出的主場"),
          "repos": _d(_STR, "逗號分隔 repo id（臨時組合，優先於 group）")}
_CWD = {"cwd": _d(_STR, "用哪個目錄推主場；省略＝server 啟動目錄（通常就是你的專案）")}
_CROSS = {"cross_scope_ok": _d(_BOOL, "寫入主場以外的範圍。只有在使用者明確同意後才設 true")}

# ---- 工作層：agent 一次工作的節奏（全部回 JSON）----


def _w(fn, *keys):
    """把 agentapi 函數包成工作層 handler：只轉交有給的參數，cwd 預設用 server 的。"""
    def handler(d, a, cwd):
        kw = {k: a[k] for k in keys if a.get(k) not in (None, "")}
        if "cwd" in keys:
            kw.setdefault("cwd", cwd)
        return fn(d, **kw)
    return handler


WORK_TOOLS = {
    "where_am_i": (
        "我在哪：cwd 屬於哪個已登記 repo、哪些組、預設工作範圍是什麼。沒登記會告訴你怎麼登記。",
        _s(**_CWD), _w(_api.where_am_i, "cwd")),
    "context_for": (
        "開場一次拿齊這個範圍的脈絡：目標、上次交接、建議下一步、未結事項（帶 id）、"
        "最近事件（帶 ref）、關係、需要注意的 repo、agent 狀態；card＝可直接閱讀的情境卡。"
        "開工前先呼叫一次。",
        _s(**_CWD, **_SCOPE, card=_d(_BOOL, "是否附 markdown 情境卡（預設 true）")),
        _w(_api.context_for, "cwd", "group", "repos", "card")),
    "next_work": (
        "接下來值得做什麼（依序：逾期未結 → 上次交接的下一步 → 其他未結 → 需要注意的 repo），"
        "每項都附 why。",
        _s(**_CWD, **_SCOPE, limit=_d(_INT, "最多幾項，預設 5")),
        _w(_api.next_work, "cwd", "group", "repos", "limit")),
    "log_decision": (
        "記下一個判斷或結論（為什麼這樣做、依據哪份檔）。回傳 ref，之後可引用。",
        _s(text=_d(_STR, "判斷內容；第一行寫結論，後面寫依據"),
           ref=_d(_STR, "相關事件的 ref（例 decision:2026-09-25-d1、collision:2026-09-25-a）"),
           **_CWD, **_SCOPE, **_CROSS, _req=["text"]),
        _w(_api.log_decision, "text", "ref", "cwd", "group", "repos", "cross_scope_ok")),
    "add_todo": (
        "開一個未結事項（之後要有人處理的事）。編號自動配，沒給 due 用預設天數。",
        _s(text=_d(_STR, "要做的事，一句話"), due=_d(_STR, "YYYY-MM-DD，可省略"),
           **_CWD, **_SCOPE, **_CROSS, _req=["text"]),
        _w(_api.add_todo, "text", "due", "cwd", "group", "repos", "cross_scope_ok")),
    "close_todo": (
        "關掉一個未結事項（用 context_for／next_work 回傳的 id，例 #3）。",
        _s(id=_d(_STR, "未結事項編號，例 #3"), note=_d(_STR, "怎麼結的，一句話"),
           **_CWD, **_CROSS, _req=["id"]),
        _w(_api.close_todo, "id", "note", "cwd", "cross_scope_ok")),
    "report_status": (
        "回報你現在的階段，讓使用者在工作台一眼看出輪到誰：working／waiting（等使用者回應）／"
        "blocked（卡住）／done。waiting 與 blocked 會進使用者的收件匣。",
        _s(status=_d(_STR, "目前階段", enum=list(_api.STATUSES)),
           text=_d(_STR, "在做什麼／在等什麼／卡在哪"), **_CWD, **_SCOPE,
           _req=["status"]),
        _w(_api.report_status, "status", "text", "cwd", "group", "repos")),
    "handoff": (
        "收尾交接（每次會話結束前必呼叫）：完成了什麼、下次從哪接；remaining 每項各開一個未結事項"
        "（只放新的，已經有 id 的別重複）。下一個 agent 的 context_for／next_work 會讀到它。",
        _s(done=_d(_STR, "這次完成了什麼（成果、改了哪些檔）"),
           next_step=_d(_STR, "下次從哪裡接，一句話"),
           remaining=_d(_ARR, "還沒做完、需要追蹤的新事項"),
           **_CWD, **_SCOPE, **_CROSS, _req=["done", "next_step"]),
        _w(_api.handoff, "done", "next_step", "remaining", "cwd", "group", "repos",
           "cross_scope_ok")),
    "search_knowledge": (
        "知識 ↔ repo 相關度：給一段文字（想法、卡片內容、問題），找出各 repo 裡最相關的 md 檔"
        "（附行號片段）與最相關的 repo 排名。預設搜全部已登記 repo。",
        _s(query=_d(_STR, "要比對的文字"),
           everywhere=_d(_BOOL, "true（預設）＝搜全部 repo；false＝只搜主場範圍"),
           limit=_d(_INT, "最多回幾個檔，預設 8"), **_CWD, **_SCOPE, _req=["query"]),
        _w(_api.search_knowledge, "query", "everywhere", "limit", "cwd", "group", "repos")),
}

# ---- 管理層：維護 registry／組／採集／打包／碰撞（預設不開）----
ADMIN_TOOLS = {
    "registry_list": ("列出所有已登記 repo（id／類型／tier／路徑）", _s(), _t_registry_list),
    "registry_add": ("登記一個 repo", _s(id=_STR, path=_STR,
                                        type=_d(_STR, "mine 或 external"),
                                        tier=_d(_STR, "active／paused／dormant"),
                                        upstream=_d(_STR, "external 的 GitHub owner/repo"),
                                        _req=["id", "path"]), _t_registry_add),
    "registry_audit": ("稽核 registry：tier 與實際活動不符、路徑失效、目錄裡有未登記 repo",
                       _s(scan_dirs=_ARR), _t_registry_audit),
    "registry_scan": ("掃一個目錄找未登記的 git repo（apply=true 才真的登記）",
                      _s(dir=_STR, apply=_BOOL, _req=["dir"]), _t_registry_scan),
    "registry_relate": ("設 repo 間關係 a --kind--> b（例 a pm-of b＝a 是 b 的 PM；"
                        "kind: pm-of/feeds/derived-from/upstream-of/sibling-topic）",
                        _s(a=_STR, b=_STR, kind=_STR, note=_STR, _req=["a", "b", "kind"]),
                        _t_registry_relate),
    "registry_relations": ("讀 repo 間關係（給 id＝站在該 repo 兩向讀；省略＝全部）",
                           _s(id=_STR), _t_registry_relations),
    "group_context": ("組情境卡（markdown）：成員與角色、脈動、本組未結、最近事件、角色說明",
                      _s(**_SCOPE), _t_group_context),
    "group_list": ("列出所有組與成員", _s(), _t_group_list),
    "group_add": ("建一個組", _s(name=_STR, members=_ARR, _req=["name", "members"]), _t_group_add),
    "collect": ("採集 git 狀態（dirty、末次 commit、ahead/behind、worktree）", _s(**_SCOPE), _t_collect),
    "pack": ("把範圍內的 md 文件打包成一份全文（含洩密過濾）", _s(**_SCOPE, budget=_INT), _t_pack),
    "pack_estimate": ("只估算範圍內 md 文件的 token 成本，不打包", _s(**_SCOPE), _t_pack_estimate),
    "brief": ("產生早晨簡報並存檔（會落一筆 presented 事件）", _s(**_SCOPE), _t_brief),
    "spine_append": ("直接寫一筆原始脊椎事件（進階；一般請用 log_decision／add_todo）。"
                     "type: presented/chosen/decision/suggestion/collision/open-loop/outcome；"
                     "kv: #編號 或 group:/repo:/ref:/due:/id: token",
                     _s(type=_STR, source=_STR, kv=_ARR, body=_STR,
                        _req=["type", "source"]), _t_spine_append),
    "spine_query": ("查脊椎事件（依型別／日期／組）", _s(type=_STR, date=_STR, today=_BOOL,
                                                   group=_STR, verbose=_BOOL), _t_spine_query),
    "spine_stats": ("量測：靈感命中率＋repo 存活率", _s(), _t_spine_stats),
    "open_loops": ("未結事項（可依 group／repos 過濾）", _s(**_SCOPE), _t_open_loops),
    "spine_lint": ("脊椎衛生檢查：ref 斷鏈、逾期未結、碰撞沒回程、dead-letter 積壓",
                   _s(), _t_spine_lint),
    "unread": ("使用者的未讀事件（ack=true 會把使用者的收件匣全部標已讀，別隨便用）",
               _s(ack=_BOOL), _t_unread),
    "collide_submit": ("把一個想法丟去跟範圍內的 repo 比對（wait=true 同步等判定）",
                       _s(idea=_STR, **_SCOPE, wait=_BOOL, provider=_STR,
                          _req=["idea"]), _t_collide_submit),
    "route": ("把一段文字落檔（dest: group:<g> | repo:<id> | incubator）",
              _s(text=_STR, dest=_STR, title=_STR, _req=["text", "dest"]), _t_route),
    "upstream_check": ("查 external repo 的上游新 release／commit", _s(**_SCOPE), _t_upstream_check),
    "digest_run": ("跑增量摘要（便宜模型）", _s(**_SCOPE, provider=_STR), _t_digest_run),
    "spawn": ("把 incubator 的想法升格成新 repo（登記＋血統）",
              _s(id=_STR, path=_STR, from_incubator=_STR, origin=_STR,
                 _req=["id", "path"]), _t_spawn),
}

# 舊名相容（測試與既有呼叫端）：TOOLS＝全部工具
TOOLS = {**{k: (v[0], v[1], v[2]) for k, v in WORK_TOOLS.items()}, **ADMIN_TOOLS}


def _admin_enabled(spine_dir, admin):
    if admin is not None:
        return admin
    return bool((_config.load(spine_dir).get("mcp") or {}).get("admin"))


def tool_table(spine_dir, admin=None):
    out = dict(WORK_TOOLS)
    if _admin_enabled(spine_dir, admin):
        out.update(ADMIN_TOOLS)
    return out


def _text(obj):
    return json.dumps(obj, ensure_ascii=False, indent=1)


def handle_message(spine_dir, msg, admin=None, cwd=None):
    """單則 JSON-RPC → 回應 dict（notification 回 None）。
    admin=None＝看 config `mcp.admin`；cwd＝推主場用的目錄（預設 server 行程的 cwd）。"""
    mid = msg.get("id")
    method = msg.get("method", "")
    if method.startswith("notifications/"):
        return None

    def ok(result):
        return {"jsonrpc": "2.0", "id": mid, "result": result}

    if method == "initialize":
        return ok({"protocolVersion": PROTOCOL_VERSION,
                   "capabilities": {"tools": {}},
                   "serverInfo": {"name": "repo_context", "version": "0.2.0"},
                   "instructions": (
                       "repo-context：跨 repo 的工作脈絡與留痕。開工先 context_for，"
                       "做了判斷 log_decision、發現待辦 add_todo、等人或卡住 report_status，"
                       "收尾一定 handoff。範圍依 cwd 推出，寫到別組會被拒，要先問使用者。")})
    if method == "ping":
        return ok({})
    table = tool_table(spine_dir, admin)
    if method == "tools/list":
        return ok({"tools": [
            {"name": n, "description": t[0], "inputSchema": t[1]}
            for n, t in table.items()]})
    if method == "tools/call":
        name = msg.get("params", {}).get("name")
        args = msg.get("params", {}).get("arguments") or {}
        if name not in table:
            hint = "（這是管理層工具，需用 `ctx mcp --admin` 啟動）" if name in ADMIN_TOOLS else ""
            return {"jsonrpc": "2.0", "id": mid,
                    "error": {"code": -32602, "message": f"未知 tool: {name}{hint}"}}
        here = cwd or os.environ.get("CTX_CWD") or os.getcwd()
        try:
            if name in WORK_TOOLS:
                text = _text(WORK_TOOLS[name][2](spine_dir, args, here))
            else:
                text = table[name][2](spine_dir, args)
            return ok({"content": [{"type": "text", "text": text}],
                       "isError": False})
        except _api.AgentError as e:
            return ok({"content": [{"type": "text", "text": _text(e.to_dict())}],
                       "isError": True})
        except Exception as e:  # 錯誤原樣回 agent 自我修正（含 validator 拒寫）
            text = (_text({"ok": False, "error": "failed", "message": str(e)})
                    if name in WORK_TOOLS else str(e))
            return ok({"content": [{"type": "text", "text": text}],
                       "isError": True})
    return {"jsonrpc": "2.0", "id": mid,
            "error": {"code": -32601, "message": f"未知 method: {method}"}}


def serve(spine_dir, admin=None):
    """stdio loop：一行一則 JSON-RPC。"""
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        resp = handle_message(spine_dir, msg, admin=admin)
        if resp is not None:
            sys.stdout.write(json.dumps(resp, ensure_ascii=False) + "\n")
            sys.stdout.flush()
