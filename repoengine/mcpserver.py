"""MCP server——agent 的介面＝人的介面（同組原語，v2 §1）。

stdio 傳輸（newline-delimited JSON-RPC 2.0），stdlib 實作零依賴（耗材原則）。
拒寫去向（P10 檢討④）：validator 錯誤**原樣**回給 agent（isError tool result）供自我修正；
dead-letter 是人的入口（hotkey）專屬，MCP 不走。
掛載：spine repo 的 .mcp.json（session.ensure_mcp_json 產生）或 claude --mcp-config。
"""
import datetime as _dt
import json
import sys

from . import brief as _brief
from . import collect as _collect
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
    return _ev_lines(_spine.open_loops(d), verbose=True)


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


_STR = {"type": "string"}
_BOOL = {"type": "boolean"}
_INT = {"type": "integer"}
_ARR = {"type": "array", "items": {"type": "string"}}
_SCOPE = {"group": _STR, "repos": {**_STR, "description": "逗號分隔 repo id（臨時組合，優先於 group）"}}

TOOLS = {
    "registry_list": ("P1 列出已登記 repo", _s(), _t_registry_list),
    "registry_add": ("P1 登記 repo", _s(id=_STR, path=_STR, type=_STR, tier=_STR,
                                        upstream=_STR, _req=["id", "path"]), _t_registry_add),
    "registry_audit": ("P1 稽核（tier 漂移/路徑失效/未登記）", _s(scan_dirs=_ARR), _t_registry_audit),
    "registry_scan": ("P1 掃目錄找未登記 git repo（apply=true 才登記；mani 混合模式）",
                      _s(dir=_STR, apply=_BOOL, _req=["dir"]), _t_registry_scan),
    "group_list": ("P2 列組", _s(), _t_group_list),
    "group_add": ("P2 建組", _s(name=_STR, members=_ARR, _req=["name", "members"]), _t_group_add),
    "collect": ("P3 採集 git 狀態", _s(**_SCOPE), _t_collect),
    "pack": ("P6 打包文件層選料（含洩密哨兵）", _s(**_SCOPE, budget=_INT), _t_pack),
    "pack_estimate": ("P6 只算各 repo token 成本不打包（挑的預算函數）",
                      _s(**_SCOPE), _t_pack_estimate),
    "brief": ("早晨簡報（樣貌A；落 presented 事件）", _s(**_SCOPE), _t_brief),
    "spine_append": ("P10 寫脊椎事件（validator 拒寫時錯誤原樣回傳，修正後重試）",
                     _s(type=_STR, source=_STR, kv=_ARR, body=_STR,
                        _req=["type", "source"]), _t_spine_append),
    "spine_query": ("P11 查事件", _s(type=_STR, date=_STR, today=_BOOL, group=_STR,
                                    verbose=_BOOL), _t_spine_query),
    "spine_stats": ("P11 品味量測（靈感命中率＋repo 存活率）", _s(), _t_spine_stats),
    "open_loops": ("P11 未結 open loops", _s(), _t_open_loops),
    "spine_lint": ("P11 脊椎衛生迴圈（ref 斷鏈/逾期 loop/碰撞無回程/dead-letter）",
                   _s(), _t_spine_lint),
    "unread": ("P13 未讀（interrupt 優先；ack=true 全部標已讀）", _s(ack=_BOOL), _t_unread),
    "collide_submit": ("P7 撞（兩段式；wait=true 同步判定）",
                       _s(idea=_STR, **_SCOPE, wait=_BOOL, provider=_STR,
                          _req=["idea"]), _t_collide_submit),
    "route": ("P8 落（dest: group:<g> | repo:<id> | incubator）",
              _s(text=_STR, dest=_STR, title=_STR, _req=["text", "dest"]), _t_route),
    "upstream_check": ("P4 查上游 release/commit（無事不報）", _s(**_SCOPE), _t_upstream_check),
    "digest_run": ("P5 增量 digest（便宜模型）", _s(**_SCOPE, provider=_STR), _t_digest_run),
    "spawn": ("P9 升格成新 repo（出生登記＋血統）",
              _s(id=_STR, path=_STR, from_incubator=_STR, origin=_STR,
                 _req=["id", "path"]), _t_spawn),
}


def handle_message(spine_dir, msg):
    """單則 JSON-RPC → 回應 dict（notification 回 None）。"""
    mid = msg.get("id")
    method = msg.get("method", "")
    if method.startswith("notifications/"):
        return None

    def ok(result):
        return {"jsonrpc": "2.0", "id": mid, "result": result}

    if method == "initialize":
        return ok({"protocolVersion": PROTOCOL_VERSION,
                   "capabilities": {"tools": {}},
                   "serverInfo": {"name": "repoengine", "version": "0.1.0"}})
    if method == "ping":
        return ok({})
    if method == "tools/list":
        return ok({"tools": [
            {"name": n, "description": desc, "inputSchema": schema}
            for n, (desc, schema, _) in TOOLS.items()]})
    if method == "tools/call":
        name = msg.get("params", {}).get("name")
        args = msg.get("params", {}).get("arguments") or {}
        if name not in TOOLS:
            return {"jsonrpc": "2.0", "id": mid,
                    "error": {"code": -32602, "message": f"未知 tool: {name}"}}
        try:
            text = TOOLS[name][2](spine_dir, args)
            return ok({"content": [{"type": "text", "text": text}],
                       "isError": False})
        except Exception as e:  # 錯誤原樣回 agent 自我修正（含 validator 拒寫）
            return ok({"content": [{"type": "text", "text": str(e)}],
                       "isError": True})
    return {"jsonrpc": "2.0", "id": mid,
            "error": {"code": -32601, "message": f"未知 method: {method}"}}


def serve(spine_dir):
    """stdio loop：一行一則 JSON-RPC。"""
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        resp = handle_message(spine_dir, msg)
        if resp is not None:
            sys.stdout.write(json.dumps(resp, ensure_ascii=False) + "\n")
            sys.stdout.flush()
