"""CLI——人的介面（正式版同組原語再暴露成 MCP server 給 agent）。"""
import argparse
import datetime as _dt
import json
import os
import subprocess
import sys
from pathlib import Path

from . import agentapi as _api
from . import brief as _brief
from . import collect as _collect
from . import context as _context
from . import collide as _collide
from . import crossref as _xref
from . import repocard as _repocard
from . import digest as _digest
from . import notify as _notify
from . import registry as _registry
from . import route as _route
from . import session as _session
from . import spawn as _spawn
from . import spine as _spine
from . import summary as _summary
from . import pack as _pack
from . import config as _config
from . import timer as _timer
from . import upstream as _upstream

SCAFFOLD_CONFIG = """\
# config.yaml — 私有設定（公私切分線：路徑、偏好、閾值、provider、排程全在這）
thresholds:
  dirty_stale_days: 4        # dirty 超過 N 天 → 進未讀
  inactive_days: 21          # active 但 N 天無 commit → 「變化」進未讀
  openloop_default_due_days: 14
  upstream:
    kinds: [release]         # 上游預設只報正式 release（commit 洪流不進未讀）
    prerelease: false
provider:
  default: mock              # mock | claude
pack:
  token_budget: 60000
# schedule:                  # P12：timer（`ui`/`timer` 子命令）按時呼叫；當日補課、跨日不補
#   - {task: brief, at: "07:30", group: g1}
#   - {task: upstream, at: "07:25"}
#   - {task: commit, at: "21:00"}
# agents:                     # P16 會話 agent 註冊表——claude 內建（見預設值），其他家照這個形狀加：
#   codex:  {cmd: [codex], mcp: none, prompt_flag: []}   # codex <prompt> 本身就是互動模式
#   gemini: {cmd: [gemini], mcp: none, prompt_flag: ["-i"]}  # -i 帶初始 prompt 進互動模式（未驗證）
#   agy:    {cmd: [agy], mcp: none, prompt_flag: ["-i"]}     # Google Antigravity CLI；-i/--prompt-interactive 已用 --help 核對
#   # mcp: mcp-config＝支援 --mcp-config（v1 只保證 claude）｜cwd＝靠 cwd 設定檔自動掛載｜none＝該家自理
"""

SCAFFOLD_GITIGNORE = ".engine.lock\n.state/\n.agent_logs/\n"


# exit code 約定（agent 靠它分支，不必讀中文）：
#   0 成功｜1 稽核／lint 有紅字或一般失敗｜2 用法錯誤或 agent 請求被拒（--json 時 stdout 有 {ok:false,...}）
#   3 找不到 spine（先 `ctx init` 或 `ctx setup --spine <路徑>`）
EXIT_FINDINGS, EXIT_REJECTED, EXIT_NO_SPINE = 1, 2, 3


def _spine_dir(args):
    """--spine ＞ $REPOENGINE_SPINE ＞ 使用者設定檔（ctx setup 寫的）＞ cwd。"""
    explicit = getattr(args, "spine", None)
    if explicit and not _config.is_spine(Path(explicit).expanduser()):
        _fail(args, "no_spine", f"{explicit} 不是 spine repo（找不到 config.yaml；先跑 init）",
              code=EXIT_NO_SPINE)
    d = _config.find_spine(explicit)
    if d is None:
        _fail(args, "no_spine",
              "找不到 spine repo：用 --spine 指定、設 REPOENGINE_SPINE，"
              "或跑一次 `ctx setup --spine <路徑>` 記住它",
              code=EXIT_NO_SPINE)
    return d


def _fail(args, error, message, hint=None, code=EXIT_REJECTED):
    if getattr(args, "json", False):
        out = {"ok": False, "error": error, "message": message}
        if hint:
            out["hint"] = hint
        print(json.dumps(out, ensure_ascii=False))
        sys.exit(code)
    sys.exit(f"錯誤：{message}" + (f"\n提示：{hint}" if hint else ""))


def _emit(args, obj, human):
    """--json 印 obj；否則印 human(obj) 回傳的文字。"""
    if getattr(args, "json", False):
        print(json.dumps(obj, ensure_ascii=False, indent=1))
    else:
        text = human(obj)
        if text:
            print(text)


def cmd_init(args):
    d = Path(args.path).expanduser()
    (d / "spine" / "events").mkdir(parents=True, exist_ok=True)
    (d / "groups").mkdir(exist_ok=True)
    (d / "incubator").mkdir(exist_ok=True)
    for name, content in [("config.yaml", SCAFFOLD_CONFIG),
                          ("registry.yaml", "repos: []\ngroups: []\n"),
                          (".gitignore", SCAFFOLD_GITIGNORE)]:
        p = d / name
        if not p.exists():
            p.write_text(content, encoding="utf-8")
    if not (d / ".git").exists():
        subprocess.run(["git", "init", "-q"], cwd=d, check=True)
    print(f"spine repo 就緒：{d}")


def cmd_registry(args):
    d = _spine_dir(args)
    if args.action == "add":
        e = _registry.add_repo(d, args.id, args.path_arg, type=args.type,
                               tags=args.tags.split(",") if args.tags else None,
                               tier=args.tier, upstream=args.upstream)
        print(f"已登記：{e['id']} ({e['type']}/{e['tier']})")
    elif args.action == "set":
        # 位置參數在 argparse 裡與 add 共用同一組 slot（id/path_arg/key/value）；
        # `registry set <id> <key> <value>` 實際落在 id/path_arg/key 三格，不是 id/key/value。
        key, value = args.path_arg, args.key
        e = _registry.set_field(d, args.id, key, value)
        print(f"已更新：{e['id']}.{key} = {value}")
    elif args.action == "remove":
        _registry.remove_repo(d, args.id)
        print(f"已移除：{args.id}（含所有組的 membership）")
    elif args.action == "tag":
        tags = _registry.tag_repo(
            d, args.id,
            add=args.add.split(",") if args.add else None,
            remove=args.remove.split(",") if args.remove else None)
        print(f"{args.id} tags: {', '.join(tags) or '（無）'}")
    elif args.action == "list":
        _emit(args, _registry.load(d)["repos"], lambda rows: "\n".join(
            f"{r['id']:<24} {r.get('type','mine'):<9} {r.get('tier','?'):<9} {r['path']}"
            for r in rows))
    elif args.action == "scan":
        # mani「init 自動掃描＋手寫補語意」混合模式：預設只列候選，--apply 才登記
        base = args.id
        if not base:
            sys.exit("用法：registry scan <目錄> [--apply]")
        if args.apply:
            added = _registry.scan_register(d, base)
            for cid in added:
                print(f"已登記：{cid}（下半身預設 mine/active；"
                      f"上半身 type/tier/tags 用 registry set 補）")
            if not added:
                print("無新 repo 可登記")
        else:
            cands = _registry.scan_dir(d, base)
            for path, cid in cands:
                print(f"[候選] {cid}: {path}")
            print(f"共 {len(cands)} 個未登記 git repo（加 --apply 登記）"
                  if cands else "無未登記 repo")
    elif args.action == "relate":
        # registry relate <A> <B> --kind pm-of [--note ...]（A 是 B 的 PM）
        if not (args.id and args.path_arg and args.kind):
            sys.exit("用法：registry relate <A> <B> --kind <kind> [--note 說明]")
        e = _registry.relate(d, args.id, args.path_arg, args.kind, note=args.note,
                             exports=args.export)
        k = _registry.relation_kinds(d)[args.kind]
        print(f"已設關係：{args.id} {k['forward']}→ {args.path_arg}（{args.kind}）"
              + (f"，用到 export：{'、'.join(e['exports'])}" if e.get("exports") else ""))
    elif args.action == "export":
        # registry export <id> <名稱> <repo 內路徑> [--note 說明]
        if not (args.id and args.path_arg and args.key):
            sys.exit("用法：registry export <repo> <名稱> <repo 內路徑> [--note 說明]")
        e = _registry.set_export(d, args.id, args.path_arg, args.key, desc=args.note)
        print(f"已登記 export：{e['repo']}:{e['name']} → {e['path']}"
              + (f"（{e['desc']}）" if e.get("desc") else ""))
    elif args.action == "unexport":
        if not (args.id and args.path_arg):
            sys.exit("用法：registry unexport <repo> <名稱>")
        ok = _registry.remove_export(d, args.id, args.path_arg)
        print(f"已刪 export：{args.id}:{args.path_arg}" if ok
              else f"{args.id} 沒有 export {args.path_arg}")
    elif args.action == "exports":
        rows = [(r["id"], n, v) for r in _registry.load(d)["repos"]
                if not args.id or r["id"] == args.id
                for n, v in _registry.exports_of(r).items()]
        _emit(args, [{"repo": r, "name": n, **v} for r, n, v in rows], lambda xs: "\n".join(
            f"{x['repo']}:{x['name']:<12} → {x['path']}" + (f"  {x['desc']}" if x.get("desc") else "")
            for x in xs) or "（沒有 export）用 registry export <repo> <名稱> <路徑> 登記")
    elif args.action == "unrelate":
        if not (args.id and args.path_arg):
            sys.exit("用法：registry unrelate <A> <B> [--kind <kind>]")
        n = _registry.unrelate(d, args.id, args.path_arg, args.kind)
        print(f"已刪 {n} 筆關係：{args.id} → {args.path_arg}")
    elif args.action == "relations":
        if args.id:
            rows = _registry.relations_of(d, args.id)
            for r in rows:
                arrow = f"{args.id} {r['label']}→ {r['peer']}" if r["direction"] == "out" \
                    else f"{args.id} 的{r['label']} {r['peer']}"
                print(f"{arrow}（{r['kind']}）" + (f"  {r['note']}" if r.get("note") else ""))
        else:
            kinds = _registry.relation_kinds(d)
            rows = _registry.relations(d)
            for r in rows:
                print(f"{r['from']} {kinds.get(r['kind'], {}).get('forward', r['kind'])}→ "
                      f"{r['to']}（{r['kind']}）" + (f"  {r['note']}" if r.get("note") else ""))
        if not rows:
            print("（無關係）可用 kind：" + ", ".join(_registry.relation_kinds(d)))
    elif args.action == "audit":
        reds = _registry.audit(d, scan_dirs=args.scan or [])
        reds += [f"[文件路徑失效] {b['repo']}/{b['doc']}:{b['line']} `{b['ref']}`：{b['fix']}"
                 for b in _xref.lint_doc_refs(d)]
        if reds:
            for r in reds:
                print(f"[RED] {r}")
            sys.exit(1)
        print("audit 零紅字 OK")


def cmd_lint(args):
    d = _spine_dir(args)
    reds = _spine.lint(d)
    if reds:
        for r in reds:
            print(f"[RED] {r}")
        sys.exit(1)
    print("脊椎 lint 零紅字 OK（衛生迴圈）")


def cmd_group(args):
    d = _spine_dir(args)
    if args.action == "add":
        _registry.add_group(d, args.name, args.members.split(","))
        print(f"組已建：{args.name}")
    elif args.action == "context":
        # 組情境卡（P16 會話開場料同源；agent 用 MCP group_context 拿同一份）
        print(_context.build_context(d, args.name or None, args.members or None))
    elif args.action == "summary":
        # 本組工作摘要（2026-09-12 §3）：工作台首屏同一份資料，每格帶依據
        _emit(args, _summary.build_summary(d, args.name or None, args.members or None),
              _summary.format_summary)
    elif args.action == "goal":
        # 目標只有人能寫（機器不替你發明）；不給 --text＝只讀
        if not args.name:
            sys.exit("group goal 需要組名")
        if args.text is None:
            print(_registry.get_group(d, args.name).get("goal") or "（尚未登記目標）")
        else:
            now = _dt.datetime.now()
            _registry.set_group_field(d, args.name, "goal", args.text)
            _registry.set_group_field(d, args.name, "goal_at",
                                      f"{now:%Y-%m-%d %H:%M}" if args.text else "")
            _spine.append_event(d, "decision", "manual", [f"group:{args.name}"],
                                body=(f"本組目標：{args.text}（從 CLI）" if args.text
                                      else "清除本組目標（從 CLI）"), when=now)
            print(f"目標已記在 {args.name}" if args.text else f"已清除 {args.name} 的目標")
    else:
        _emit(args, _registry.load(d)["groups"], lambda gs: "\n".join(
            f"{g['name']:<20} {','.join(g['members'])}" for g in gs))


def cmd_collect(args):
    d = _spine_dir(args)
    _, entries = _registry.resolve_group(d, args.group, args.repos)
    _emit(args, _collect.collect_group(entries, d), _collect.format_table)


def cmd_pulse(args):
    """活動脈動：組的監控專注「活動頻繁度＋近期 commit 內容」（2026-09-08）。"""
    d = _spine_dir(args)
    gname, entries = _registry.resolve_group(d, args.group, args.repos)
    states = _collect.collect_group(entries, d)
    pulse = _collect.group_pulse(states)
    print(f"# 脈動 {gname}")
    print(_collect.pulse_line(pulse))
    print()
    print(f"{'repo':<24} {'7d':>3} {'30d':>3}  最近 commit")
    for s in states:
        if s.get("type", "mine") != "mine" or not s.get("exists"):
            continue
        rc = s.get("recent_commits") or []
        head = f"{rc[0]['date']} {rc[0]['subject']}" if rc else "（無）"
        print(f"{s['id']:<24} {s.get('commits_7d') or 0:>3} {s.get('commits_30d') or 0:>3}  {head}")
    print()
    print(f"# 近期 commit（新→舊，最多 {args.limit} 則）")
    for c in _collect.recent_across(states, limit=args.limit):
        print(f"{c['date']} {c['repo']:<24} {c['hash']} {c['subject']}")


def cmd_pack(args):
    d = _spine_dir(args)
    gname, entries = _registry.resolve_group(d, args.group, args.repos)
    cfg = _config.load(d)
    budget = args.budget or cfg["pack"]["token_budget"]
    if args.estimate:
        # 挑的預算函數：先算再挑，粒度由數字反推（Repomix 課）
        rows = _pack.estimate(entries)
        total = sum(r["tokens"] for r in rows)
        for r in sorted(rows, key=lambda x: -x["tokens"]):
            print(f"{r['id']:<24} {r['files']:>4} 檔 ~{r['tokens']:>7} tokens")
        fit = "裝得下" if total <= budget else f"超預算（挑細一點或加 --budget）"
        print(f"{'合計':<24} {'':>6} ~{total:>7} tokens／預算 {budget} → {fit}")
        return
    text, inc, exc, flagged = _pack.pack_group(entries, budget)
    out = d / "groups" / (args.group or "adhoc") / "materials" / \
        f"pack-{_dt.datetime.now():%Y%m%d-%H%M%S}.md"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(text, encoding="utf-8")
    msg = f"打包完成：{out}（收錄 {len(inc)}、未納入 {len(exc)}"
    msg += f"、疑似機密擋下 {len(flagged)}）" if flagged else "）"
    print(msg)


def cmd_brief(args):
    d = _spine_dir(args)
    out, text = _brief.run(d, args.group, args.repos)
    print(text)
    print(f"→ 已寫入 {out} 並落 presented 事件")


def cmd_append(args):
    d = _spine_dir(args)
    tokens = args.kv or []
    if args.type == "open-loop":
        due_days = _config.load(d)["thresholds"]["openloop_default_due_days"]
        tokens = _spine.default_due_tokens(tokens, args.body, due_days)
    ev = _spine.append_event(d, args.type, args.source, tokens,
                             body=args.body or "", dead_letter=args.dead_letter)
    if ev is None:
        print("validator 拒寫 → 原文已落 spine/dead-letter/（想法不丟）")
        sys.exit(1)
    print(f"已寫入：{ev.header()}")


def cmd_query(args):
    d = _spine_dir(args)
    if args.stats:
        s = _spine.stats(d)
        sv = _registry.survival(d)
        rate = f"{s['hit_rate']:.0%}" if s["hit_rate"] is not None else "n/a（無碰撞）"
        srate = f"{sv['survival_rate']:.0%}" if sv["survival_rate"] is not None else "n/a"
        print(f"碰撞 {s['collisions']} 次｜有 outcome 回連 {s['collisions_with_outcome']} 次｜靈感命中率 {rate}")
        print(f"生出 repo {sv['spawned']} 個｜存活 {sv['alive']}｜存活率 {srate}")
        return
    date = f"{_dt.date.today():%Y-%m-%d}" if args.today else args.date
    evs = _spine.query(d, type=args.type, date=date, group=args.group)
    if args.json:
        _emit(args, [_api.event_dict(ev) for ev in evs], None)
        return
    for ev in evs:
        print(f"{ev.date} {ev.header()}")
        if args.verbose and ev.body:
            print("   " + ev.body.replace("\n", "\n   "))


def cmd_loops(args):
    d = _spine_dir(args)
    loops = _spine.open_loops(d)
    if args.group or args.repos:
        _, entries = _registry.resolve_group(d, args.group, args.repos)
        ids = [e["id"] for e in entries]
        loops = [ev for ev in loops if _spine.in_scope(ev, args.group, ids)]
    if args.json:
        _emit(args, [_api.loop_dict(ev) for ev in loops], None)
        return
    if not loops:
        print("無未結 open loops")
    for ev in loops:
        num = next((t for t in ev.tokens if t.startswith("#")), "#?")
        first = ev.body.splitlines()[0] if ev.body else ""
        print(f"{num} due:{ev.kv('due') or '-'} {first}")


def cmd_collide(args):
    d = _spine_dir(args)
    if args.action == "submit":
        cid = _collide.submit(d, args.idea, group=args.group, repos=args.repos)
        print(f"已丟進碰撞：{cid}（想法已落脊椎）")
        if args.wait:
            j = _collide.run_judgement(d, cid, group=args.group,
                                       repos=args.repos, provider=args.provider)
            print(json.dumps(j, ensure_ascii=False, indent=1) if j
                  else "判定失敗（system-unsure）——已落脊椎浮出")
        elif not args.no_run:
            _collide.spawn_detached(d, cid, group=args.group, provider=args.provider)
            print("判定已丟 detached 進程，結果走脊椎未讀回程")
    else:  # run（段二；detached 進程的入口）
        j = _collide.run_judgement(d, args.cid, group=args.group,
                                   repos=args.repos, provider=args.provider)
        if j is None:
            sys.exit(1)
        print(json.dumps(j, ensure_ascii=False, indent=1))


def cmd_route(args):
    d = _spine_dir(args)
    text = Path(args.file).read_text(encoding="utf-8") if args.file else args.text
    p = _route.route(d, text, args.dest, title=args.title)
    print(f"已落：{p}")


def cmd_unread(args):
    d = _spine_dir(args)
    evs = _spine.get_unread(d)
    print(f"未讀 {len(evs)} 則")
    for ev in evs:
        print(f"{ev.date} {ev.header()}")
    if args.ack:
        _spine.ack_unread(d)
        print("已標記全部已讀")


def cmd_commit(args):
    d = _spine_dir(args)
    print("已 commit" if _spine.batch_commit(d, args.message) else "無變更可 commit")


def cmd_upstream(args):
    d = _spine_dir(args)
    n, findings = _upstream.check(d, group=args.group, repos=args.repos)
    if not n:
        print("無 external repo 可查（registry 的 external 需帶 upstream 欄位）")
        return
    if not findings:
        print(f"查了 {n} 個 external，上游無新事（無事不報）")
        return
    for f in findings:
        print(f"{f['repo']}: {f['kind']} {f['id']} {f['title']}（已落 suggestion 事件）")


def cmd_digest(args):
    d = _spine_dir(args)
    for r in _digest.run(d, group=args.group, repos=args.repos,
                         provider=args.provider):
        print(f"{r['repo']}: {r['note']}" + (f" → {r['file']}" if r["file"] else ""))


def cmd_spawn(args):
    d = _spine_dir(args)
    p = _spawn.spawn(d, args.id, args.path, from_incubator=args.from_incubator,
                     origin=args.origin)
    print(f"已升格：{args.id} → {p}（出生登記＋血統已落脊椎）")


def cmd_timer(args):
    d = _spine_dir(args)
    if args.once:
        ran = _timer.tick(d)
        if not ran:
            print("無到期任務")
        for key, ok in ran:
            print(f"{'✔' if ok else '✘（已浮出 system-unsure）'} {key}")
        return
    print(f"timer 啟動（每 {args.interval}s tick；Ctrl+C 結束）")
    try:
        _timer.run_loop(d, interval=args.interval)
    except KeyboardInterrupt:
        print("\ntimer 已停")


def cmd_notify(args):
    d = _spine_dir(args)
    print(_notify.render(_notify.pending(d)))


def cmd_session(args):
    d = _spine_dir(args)
    cwd, cmd, pack_path = _session.build(d, group=args.group, repos=args.repos,
                                         repo=args.repo, task=args.task,
                                         agent=args.agent)
    print(f"料：{pack_path}")
    if args.print_only:
        print(f"cd {cwd}")
        print(subprocess.list2cmdline(cmd))
        return
    _session.open_terminal(cwd, cmd)


def cmd_mcp(args):
    from . import mcpserver
    mcpserver.serve(_spine_dir(args), admin=args.admin)


def cmd_ui(args):
    from . import webui
    d = _spine_dir(args)
    webui.serve(d, port=args.port, open_browser=not args.no_browser)


def cmd_app(args):
    from . import webui
    webui.serve_window(_spine_dir(args), port=args.port)


# ---- agent 介面（2026-09-25 Agent 友善輪）：agentapi 的 CLI 入口，--json 回結構化結果 ----

def _agent_call(args, fn, human, **kw):
    d = _spine_dir(args)
    try:
        res = fn(d, **{k: v for k, v in kw.items() if v not in (None, "")})
    except _api.AgentError as e:
        _fail(args, e.code, e.message, e.hint)
    except ValueError as e:  # registry 找不到組／repo 等
        _fail(args, "bad_request", str(e))
    _emit(args, res, human)


def _h_where(r):
    if r.get("repo"):
        g = "、".join(r["groups"]) or "（不在任何組）"
        out = f"repo：{r['repo']['id']}（{r['repo']['path']}）\n組：{g}"
    else:
        out = "這個目錄沒有登記" + ("（在 spine 裡）" if r["in_spine"] else "")
    for k in ("note", "hint"):
        if r.get(k):
            out += f"\n{r[k]}"
    return out


def _h_next(r):
    out = [f"# 下一步（{r['scope']['label']}）"]
    if r.get("goal"):
        out.append(f"目標：{r['goal']}")
    for i, it in enumerate(r["items"], 1):
        tag = it.get("id") or it.get("ref") or it.get("repo") or ""
        out.append(f"{i}. [{it['kind']}] {tag} {it['text']}".replace("  ", " "))
        out.append(f"   為什麼：{it['why']}")
    if r.get("note"):
        out.append(r["note"])
    return "\n".join(out)


def _h_search(r):
    out = [f"# 相關度（{r['scope']}，索引 {r.get('indexed_files', 0)} 檔）：{r['query'][:40]}"]
    if r.get("note"):
        out.append(r["note"])
    if r["repos"]:
        out.append("repo 排名：" + "、".join(f"{x['repo']}({x['score']})" for x in r["repos"][:6]))
    for f in r["files"]:
        out.append(f"- {f['repo']}/{f['file']}  {f['score']}"
                   + (f"（{'；'.join(f['why'])}）" if f.get("why") else ""))
        for sn in f["snippets"]:
            out.append(f"    L{sn['line']}: {sn['text']}")
    return "\n".join(out)


def _h_search_full(r):
    out = [_h_search(r)]
    if r.get("code"):
        out.append("程式碼：")
        for c in r["code"]:
            out.append(f"- {c['repo']}/{c['file']}  （{'；'.join(c['why'])}）")
            for sn in c["snippets"]:
                out.append(f"    L{sn['line']}: {sn['text']}")
    return "\n".join(out)


def _h_resolve(r):
    out = f"{r['path']}" + ("" if r["exists"] else "（不存在）")
    if r.get("export"):
        out += f"\n= {r['repo']}:{r['export']}" + (f"（{r['desc']}）" if r.get("desc") else "")
    if r.get("hint"):
        out += f"\n{r['hint']}"
    return out


def _h_read(r):
    head = f"# {r['repo']}:{r['rel']}（commit {r.get('file_commit') or r.get('commit') or '-'}）"
    if r["kind"] == "dir":
        lines = [head] + [f"- {e['name']}" + (f"（{e['files']} 檔）" if e.get("files") else "")
                          for e in r["entries"]]
        if r["total_entries"] > len(r["entries"]):
            lines.append(f"- …共 {r['total_entries']} 項")
        return "\n".join(lines)
    tail = f"\n\n（{r['hint']}）" if r.get("hint") else ""
    return f"{head}\n{r['content']}{tail}"


def _h_ask(r):
    out = [f"# {r['repo']} 的回答（{r['provider']}，commit {r.get('commit') or '-'}）", r["answer"]]
    if r.get("citations"):
        out.append("\n引用：")
        out += [f"- {r['repo']}:{c['file']}" + (f" L{c['lines']}" if c.get("lines") else "")
                + (f"——{c['why']}" if c.get("why") else "") for c in r["citations"]]
    if r.get("note"):
        out.append(f"\n（{r['note']}）")
    return "\n".join(out)


def _h_written(r):
    ev = r.get("event") or {}
    bits = [r.get("ref") or r.get("id") or ""]
    if r.get("todos"):
        bits.append("新未結 " + " ".join(r["todos"]))
    if ev.get("text"):
        bits.append(ev["text"])
    return "已寫入：" + "｜".join(b for b in bits if b)


def cmd_where(args):
    _agent_call(args, _api.where_am_i, _h_where, cwd=args.cwd)


def cmd_context(args):
    _agent_call(args, _api.context_for, lambda r: r.get("card") or "",
                cwd=args.cwd, group=args.group, repos=args.repos)


def cmd_next(args):
    _agent_call(args, _api.next_work, _h_next, cwd=args.cwd, group=args.group,
                repos=args.repos, limit=args.limit)


def cmd_log(args):
    _agent_call(args, _api.log_decision, _h_written, text=args.text, ref=args.ref,
                cwd=args.cwd, group=args.group, repos=args.repos,
                cross_scope_ok=args.cross_scope_ok, source=args.source)


def cmd_todo(args):
    if args.action == "add":
        _agent_call(args, _api.add_todo, _h_written, text=args.text, due=args.due,
                    cwd=args.cwd, group=args.group, repos=args.repos,
                    cross_scope_ok=args.cross_scope_ok, source=args.source)
    else:
        _agent_call(args, _api.close_todo,
                    lambda r: f"已關閉 {r['id']}：{r['closed']}",
                    id=args.text, note=args.note, cwd=args.cwd,
                    cross_scope_ok=args.cross_scope_ok, source=args.source)


def cmd_status(args):
    _agent_call(args, _api.report_status,
                lambda r: f"狀態：{r['status']['status']}（{r['status']['scope']}）",
                status=args.status, text=args.text, cwd=args.cwd,
                group=args.group, repos=args.repos)


def cmd_handoff(args):
    _agent_call(args, _api.handoff, _h_written, done=args.done,
                next_step=args.next_step, remaining=args.remaining, cwd=args.cwd,
                group=args.group, repos=args.repos,
                cross_scope_ok=args.cross_scope_ok, source=args.source)


def cmd_search(args):
    _agent_call(args, _api.search_knowledge, _h_search_full,
                query=" ".join(args.query) if args.query else
                (Path(args.file).read_text(encoding="utf-8") if args.file else ""),
                cwd=args.cwd, group=args.group, repos=args.repos,
                everywhere=not args.here, limit=args.limit, code=not args.no_code,
                exclude_paths=[args.file] if args.file else None)


def cmd_resolve(args):
    _agent_call(args, _api.resolve_ref, _h_resolve, ref=args.ref)


def cmd_read(args):
    _agent_call(args, _api.read_from, _h_read, ref=args.ref, lines=args.lines, cwd=args.cwd)


def cmd_ask(args):
    _agent_call(args, _api.ask_repo, _h_ask, repo=args.repo,
                question=" ".join(args.question), cwd=args.cwd, provider=args.provider)


def cmd_repo(args):
    """狀態卡。人看（不帶 --peek、不是 --json）才推已讀水位線。"""
    d = _spine_dir(args)
    rid = args.id
    if not rid:
        w = _api.where_am_i(d, args.cwd)
        if not w.get("repo"):
            _fail(args, "bad_request", "目前目錄不在已登記的 repo 裡，請指定 repo id",
                  w.get("hint") or w.get("note"))
        rid = w["repo"]["id"]
    try:
        card = _repocard.build(d, rid, since=args.since,
                               mark=not (args.peek or args.json))
    except ValueError as e:
        _fail(args, "bad_request", str(e))
    _emit(args, card, _repocard.render)


def cmd_refs(args):
    d = _spine_dir(args)
    if args.action == "check":
        broken = _xref.lint_doc_refs(d, [args.repo] if args.repo else None)
        dr = _xref.drift(d, consumer=args.repo)
        sug = _xref.suggest_exports(d, min_count=args.min)
        res = {"ok": True, "broken": broken, "drift": dr, "suggest": sug}

        def human(r):
            out = []
            if r["broken"]:
                out.append(f"## agent 文件裡失效的跨 repo 路徑（{len(r['broken'])}）")
                out += [f"- {b['repo']}/{b['doc']}:{b['line']} `{b['ref']}`\n  → {b['fix']}"
                        for b in r["broken"]]
            if r["drift"]:
                out.append(f"## 引用過的上游有變（{len(r['drift'])}）")
                out += _xref.drift_lines(r["drift"], limit=50)
            if r["suggest"]:
                out.append("## 常被引用、建議宣告成 export")
                out += [f"- {x['repo']}:{x['path']} 被引用 {x['cites']} 次（{', '.join(x['consumers'])}）"
                        f"\n  → {x['command']}" for x in r["suggest"]]
            return "\n".join(out) or "跨 repo 參考零紅字 OK"
        _emit(args, res, human)
        if (broken or dr) and not args.json:
            sys.exit(EXIT_FINDINGS)
    elif args.action == "drift":
        dr = _xref.drift(d, consumer=args.repo)
        _emit(args, dr, lambda x: "\n".join(_xref.drift_lines(x, limit=100)) or "引用過的上游都沒變")
    elif args.action == "suggest":
        _emit(args, _xref.suggest_exports(d, min_count=args.min), lambda xs: "\n".join(
            f"{x['repo']}:{x['path']} 被引用 {x['cites']} 次 → {x['command']}" for x in xs)
            or f"沒有被引用 ≥{args.min} 次卻沒宣告的目錄")
    elif args.action == "cite":
        # 手動記一筆引用：例如把上游檔複製進本 repo 時，記下當時的版本（之後上游改了會提醒）
        if not args.ref:
            sys.exit("用法：refs cite <repo>:<路徑> [--note 說明] [--cwd 引用方 repo 目錄]")
        try:
            r = _xref.resolve(d, args.ref)
        except _xref.RefError as e:
            _fail(args, e.code, e.message, e.hint)
        w = _api.where_am_i(d, args.cwd)
        consumer = args.repo or (w.get("repo") or {}).get("id")
        if not consumer:
            _fail(args, "bad_request", "認不出引用方 repo：在該 repo 目錄下執行，或帶 --repo")
        rc = subprocess.run(["git", "-C", r["root"], "rev-parse", "--short", "HEAD"],
                            capture_output=True, text=True)
        rec = _xref.record_cite(d, consumer, r["repo"], r["rel"],
                                rc.stdout.strip() or None, "manual", note=args.note)
        _emit(args, {"ok": True, "cite": rec}, lambda x: (
            f"已記引用：{consumer} ← {r['repo']}:{r['rel']}（{x['cite']['commit']}）" if x["cite"]
            else "今天已經記過這筆引用"))


def cmd_hook(args):
    """Claude Code hook 入口：stdin 是 hook JSON。永遠 exit 0——hook 壞掉不能擋住開 agent。"""
    from . import hooks as _hooks
    try:
        payload = json.loads(sys.stdin.read() or "{}")
    except ValueError:
        payload = {}
    d = _config.find_spine(args.spine)
    if d is None:
        return
    if args.event == "session-start":
        out = _hooks.session_start(d, payload)
        if out:
            print(out)
    else:
        _hooks.session_end(d, payload)


def cmd_instructions(args):
    """給 agent 的使用說明（skill 本體）——可貼進 AGENTS.md／GEMINI.md 給沒有 skill 機制的 agent。"""
    from . import agentsetup as _setup
    print(_setup.skill_text(frontmatter=False))


def cmd_setup(args):
    from . import agentsetup as _setup
    spine_arg = args.spine_path or args.spine
    if spine_arg:
        d = Path(spine_arg).expanduser().resolve()
        if not _config.is_spine(d):
            _fail(args, "no_spine", f"{d} 不是 spine repo（先跑 `ctx init {d}`）",
                  code=EXIT_NO_SPINE)
    else:
        d = _spine_dir(args)
    steps = _setup.plan(d, claude=args.claude)
    for st in steps:
        print(("（試跑）" if args.dry_run else "") + st["desc"])
        if not args.dry_run:
            msg = st["run"]()
            if msg:
                print(f"  → {msg}")
    if args.dry_run:
        print("加上不帶 --dry-run 再跑一次才會真的寫入。")


def build_parser():
    p = argparse.ArgumentParser(prog="ctx", description="repo-context 工作台：把一群 repo 分成組，每組組成一份 context")
    p.add_argument("--spine", help="spine repo 路徑（預設 $REPOENGINE_SPINE → ctx setup 記住的 → cwd）")
    p.add_argument("--json", action="store_true",
                   help="結構化輸出（agent 用；錯誤也是 JSON，exit code 見 cli.py 開頭）")
    sub = p.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("init", help="建 spine repo scaffold")
    s.add_argument("path")
    s.set_defaults(func=cmd_init)

    s = sub.add_parser("registry", help="P1 登記/掃描/稽核/tag")
    s.add_argument("action",
                   choices=["add", "set", "remove", "tag", "list", "scan", "audit",
                            "relate", "unrelate", "relations", "export", "unexport", "exports"])
    s.add_argument("id", nargs="?", help="scan 時＝要掃的目錄；relate 時＝來源 repo A")
    s.add_argument("path_arg", nargs="?", help="relate 時＝目標 repo B")
    s.add_argument("--kind", help="relate/unrelate：關係種類（pm-of/feeds/derived-from/"
                                  "upstream-of/sibling-topic；config relations.kinds 可擴充）")
    s.add_argument("--note", help="relate／export：一句說明")
    s.add_argument("--export", help="relate：這條關係用到 A 的哪些 export（逗號分隔）")
    s.add_argument("key", nargs="?")
    s.add_argument("value", nargs="?")
    s.add_argument("--type", default="mine", choices=["mine", "external"])
    s.add_argument("--tier", default="active")
    s.add_argument("--tags")
    s.add_argument("--upstream")
    s.add_argument("--scan", action="append")
    s.add_argument("--apply", action="store_true",
                   help="scan：把候選實際登記（預設只列出）")
    s.add_argument("--add", help="tag：加（逗號分隔）")
    s.add_argument("--remove", help="tag：移除（逗號分隔）")
    s.set_defaults(func=cmd_registry)

    s = sub.add_parser("group", help="P2 組（context＝組情境卡；summary＝工作摘要；goal＝一句話目標）")
    s.add_argument("action", choices=["add", "list", "context", "summary", "goal"])
    s.add_argument("name", nargs="?", help="context／summary 時可省略＝全部")
    s.add_argument("members", nargs="?", help="context／summary 時＝逗號分隔 repo id（臨時組合）")
    s.add_argument("--text", help="goal：這組要達成什麼（一句話）；不給＝只讀目前目標")
    s.set_defaults(func=cmd_group)

    s = sub.add_parser("pulse", help="活動脈動：7d/30d commit 數＋近期 commit 主旨（組的監控焦點）")
    s.add_argument("--group")
    s.add_argument("--repos")
    s.add_argument("--limit", type=int, default=20)
    s.set_defaults(func=cmd_pulse)

    for name, fn, hlp in [("collect", cmd_collect, "P3 採集"),
                          ("pack", cmd_pack, "P6 打包"),
                          ("brief", cmd_brief, "早晨簡報（樣貌A）")]:
        s = sub.add_parser(name, help=hlp)
        s.add_argument("--group")
        s.add_argument("--repos")
        if name == "pack":
            s.add_argument("--budget", type=int)
            s.add_argument("--estimate", action="store_true",
                           help="只算各 repo token 成本不打包（挑的預算函數）")
        s.set_defaults(func=fn)

    s = sub.add_parser("append", help="P10 寫事件")
    s.add_argument("--type", required=True)
    s.add_argument("--source", required=True)
    s.add_argument("--body")
    s.add_argument("--kv", action="append")
    s.add_argument("--dead-letter", action="store_true",
                   help="人的入口：拒寫時原文落 dead-letter 不丟")
    s.set_defaults(func=cmd_append)

    s = sub.add_parser("query", help="P11 查詢")
    s.add_argument("--type")
    s.add_argument("--date")
    s.add_argument("--group")
    s.add_argument("--today", action="store_true")
    s.add_argument("--stats", action="store_true")
    s.add_argument("-v", "--verbose", action="store_true")
    s.set_defaults(func=cmd_query)

    s = sub.add_parser("loops", help="未結 open loops（P11 視圖；可依組過濾）")
    s.add_argument("--group")
    s.add_argument("--repos")
    s.set_defaults(func=cmd_loops)

    s = sub.add_parser("lint", help="脊椎衛生迴圈（ref 斷鏈/逾期 loop/碰撞無回程/dead-letter）")
    s.set_defaults(func=cmd_lint)

    s = sub.add_parser("collide", help="P7 撞（兩段式）")
    s.add_argument("action", choices=["submit", "run"])
    s.add_argument("idea", nargs="?")
    s.add_argument("--cid")
    s.add_argument("--group")
    s.add_argument("--repos")
    s.add_argument("--provider")
    s.add_argument("--wait", action="store_true", help="同步跑段二")
    s.add_argument("--no-run", action="store_true", help="只落想法不判定")
    s.set_defaults(func=cmd_collide)

    s = sub.add_parser("route", help="P8 落")
    s.add_argument("--text")
    s.add_argument("--file")
    s.add_argument("--dest", required=True)
    s.add_argument("--title", default="idea")
    s.set_defaults(func=cmd_route)

    s = sub.add_parser("unread", help="未讀事件（殼 poll 的同一來源）")
    s.add_argument("--ack", action="store_true")
    s.set_defaults(func=cmd_unread)

    s = sub.add_parser("commit", help="git 單一提交者（批次 commit）")
    s.add_argument("-m", "--message", default="spine: batch commit")
    s.set_defaults(func=cmd_commit)

    for name, fn, hlp in [("upstream", cmd_upstream, "P4 查上游 release/commit"),
                          ("digest", cmd_digest, "P5 增量 digest（便宜模型）")]:
        s = sub.add_parser(name, help=hlp)
        s.add_argument("--group")
        s.add_argument("--repos")
        if name == "digest":
            s.add_argument("--provider")
        s.set_defaults(func=fn)

    s = sub.add_parser("spawn", help="P9 生（升格＋出生登記＋血統）")
    s.add_argument("id")
    s.add_argument("path")
    s.add_argument("--from-incubator", dest="from_incubator",
                   help="incubator 裡的素材檔名（會搬進新 repo）")
    s.add_argument("--origin", help="血統，如 collision:2026-09-01-a（會落 outcome 回連）")
    s.set_defaults(func=cmd_spawn)

    s = sub.add_parser("timer", help="P12 排程 tick（S6 引信；schedule 見 config.yaml）")
    s.add_argument("--once", action="store_true", help="跑一輪到期任務就退出")
    s.add_argument("--interval", type=int, default=30)
    s.set_defaults(func=cmd_timer)

    s = sub.add_parser("notify", help="P13 通知內容（interrupt 白名單優先）")
    s.set_defaults(func=cmd_notify)

    s = sub.add_parser("session", help="P16 帶料開 agent 終端視窗（含 MCP 掛載）")
    s.add_argument("--group")
    s.add_argument("--repos")
    s.add_argument("--repo", help="會話 cwd 設為此 repo（改用 --mcp-config 掛載）")
    s.add_argument("--task")
    s.add_argument("--agent", default="claude",
                   help="開哪家 agent CLI（config agents: 註冊表；預設 claude）")
    s.add_argument("--print", dest="print_only", action="store_true",
                   help="只印指令不開視窗")
    s.set_defaults(func=cmd_session)

    s = sub.add_parser("mcp", help="MCP server（stdio）；預設只開工作層工具，--admin 另開維護工具")
    s.add_argument("--admin", action="store_true", default=None,
                   help="另開管理層工具（registry／組／打包／碰撞／升格）")
    s.set_defaults(func=cmd_mcp)

    s = sub.add_parser("app", help="S4 工作台桌面視窗（IDE 風格＋內嵌 terminal；"
                                   "pywebview，不開瀏覽器；有 schedule 時內建 timer）")
    s.add_argument("--port", type=int, default=0)
    s.set_defaults(func=cmd_app)

    s = sub.add_parser("ui", help="S4 工作台瀏覽器模式（app 的過渡替代）")
    s.add_argument("--port", type=int, default=8765)
    s.add_argument("--no-browser", action="store_true")
    s.set_defaults(func=cmd_ui)

    # ---- agent 介面 ----
    def scoped(sp, write=False):
        sp.add_argument("--cwd", help="用哪個目錄推主場（預設目前目錄）")
        sp.add_argument("--group", help="組名（省略＝主場）")
        sp.add_argument("--repos", help="逗號分隔 repo id（臨時組合）")
        if write:
            sp.add_argument("--cross-scope-ok", dest="cross_scope_ok", action="store_true",
                            help="寫入主場以外（使用者同意後才用）")
            sp.add_argument("--source", default="agent",
                            help="留痕來源（預設 agent；人手動記用 manual）")

    s = sub.add_parser("where", help="我在哪：cwd 屬於哪個 repo／組（agent 開場用）")
    s.add_argument("--cwd")
    s.set_defaults(func=cmd_where)

    s = sub.add_parser("context", help="主場的完整脈絡（--json：目標／交接／下一步／未結／事件）")
    scoped(s)
    s.set_defaults(func=cmd_context)

    s = sub.add_parser("next", help="接下來值得做什麼（每項附依據）")
    scoped(s)
    s.add_argument("--limit", type=int, default=5)
    s.set_defaults(func=cmd_next)

    s = sub.add_parser("log", help="記一筆判斷（decision），回傳可引用的 ref")
    s.add_argument("text")
    s.add_argument("--ref", help="相關事件 ref，例 decision:2026-09-25-d1")
    scoped(s, write=True)
    s.set_defaults(func=cmd_log)

    s = sub.add_parser("todo", help="未結事項：add <文字> [--due] ／ close <#N> [--note]")
    s.add_argument("action", choices=["add", "close"])
    s.add_argument("text", help="add：要做的事；close：編號（#3 或 3）")
    s.add_argument("--due", help="YYYY-MM-DD")
    s.add_argument("--note", help="close 時：怎麼結的")
    scoped(s, write=True)
    s.set_defaults(func=cmd_todo)

    s = sub.add_parser("status", help="回報 agent 階段：working／waiting／blocked／done")
    s.add_argument("status", choices=list(_api.STATUSES))
    s.add_argument("text", nargs="?", default="")
    scoped(s)
    s.set_defaults(func=cmd_status)

    s = sub.add_parser("handoff", help="收尾交接：--done 完成了什麼 --next 下次從哪接 [--remaining 新事項 ...]")
    s.add_argument("--done", required=True)
    s.add_argument("--next", dest="next_step", required=True)
    s.add_argument("--remaining", action="append", help="還沒做完的新事項（可重複；各開一個未結）")
    scoped(s, write=True)
    s.set_defaults(func=cmd_handoff)

    s = sub.add_parser("search", help="知識 ↔ repo 相關度：這段文字跟哪些 repo 的哪些檔最相關")
    s.add_argument("query", nargs="*")
    s.add_argument("--file", help="拿一個檔的內容當查詢（例：一張新卡片）")
    s.add_argument("--here", action="store_true", help="只搜主場範圍（預設搜全部 repo）")
    s.add_argument("--no-code", dest="no_code", action="store_true", help="不搜程式碼（只搜 md）")
    s.add_argument("--limit", type=int, default=8)
    scoped(s)
    s.set_defaults(func=cmd_search)

    # ---- 跨 repo 參考（2026-09-25 跨 repo 參考輪）----
    s = sub.add_parser("resolve", help="名字 → 路徑：<repo>、<repo>:<export>[/子路徑]、<repo>:<相對路徑>")
    s.add_argument("ref")
    s.set_defaults(func=cmd_resolve)

    s = sub.add_parser("read", help="讀別的 repo 的檔案或目錄（附 commit；會記一筆引用）")
    s.add_argument("ref")
    s.add_argument("--lines", help="只讀某段，例 120-180")
    s.add_argument("--cwd", help="引用方目錄（預設目前目錄）")
    s.set_defaults(func=cmd_read)

    s = sub.add_parser("ask", help="在另一個 repo 開唯讀 agent 回答問題（慢、花錢；回結論＋引用）")
    s.add_argument("repo")
    s.add_argument("question", nargs="+")
    s.add_argument("--provider", help="claude（預設看 config provider.ask）或 mock")
    s.add_argument("--cwd")
    s.set_defaults(func=cmd_ask)

    s = sub.add_parser("repo", help="repo 狀態卡：上次看過後的變化、沒 commit 的檔、正在跑的程序、交接、失效路徑")
    s.add_argument("id", nargs="?", help="repo id（省略＝目前目錄所在 repo）")
    s.add_argument("--since", help="7d／30d／commit hash；省略＝你上次看過之後")
    s.add_argument("--peek", action="store_true", help="看但不記已讀（下次仍從同一點列）")
    s.add_argument("--cwd")
    s.set_defaults(func=cmd_repo)

    s = sub.add_parser("refs", help="跨 repo 參考：check（失效路徑＋上游變動＋建議 export）／drift／suggest／cite")
    s.add_argument("action", choices=["check", "drift", "suggest", "cite"])
    s.add_argument("ref", nargs="?", help="cite：被引用的 <repo>:<路徑>")
    s.add_argument("--repo", help="只看這個 repo（check／drift）；cite 時＝引用方")
    s.add_argument("--note", help="cite：說明（例：複製進 20_reference/）")
    s.add_argument("--cwd")
    s.add_argument("--min", type=int, default=3, help="suggest：被引用幾次以上才建議")
    s.set_defaults(func=cmd_refs)

    s = sub.add_parser("hook", help="Claude Code hook 入口（stdin 讀 hook JSON；ctx setup --claude 會掛好）")
    s.add_argument("event", choices=["session-start", "session-end"])
    s.set_defaults(func=cmd_hook)

    s = sub.add_parser("instructions", help="印出給 agent 的使用說明（可貼進 AGENTS.md）")
    s.set_defaults(func=cmd_instructions)

    s = sub.add_parser("setup", help="記住 spine 位置；--claude 另外掛 MCP（user scope）＋hooks＋skill")
    s.add_argument("--spine", dest="spine_path", help="spine repo 路徑")
    s.add_argument("--claude", action="store_true",
                   help="註冊 Claude Code：MCP server、SessionStart/SessionEnd hooks、skill")
    s.add_argument("--dry-run", action="store_true", help="只列出會做什麼")
    s.set_defaults(func=cmd_setup)
    return p


def main(argv=None):
    # 不再需要先設 PYTHONUTF8：輸出一律 UTF-8（Windows cp950 主控台也不會炸）
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8")
        except (AttributeError, ValueError):
            pass
    # --json 放在子命令前後都認（agent 自然會寫 `ctx where --json`）
    argv = list(sys.argv[1:] if argv is None else argv)
    as_json = "--json" in argv
    argv = [a for a in argv if a != "--json"]
    args = build_parser().parse_args(argv)
    args.json = as_json
    if args.cmd == "collide" and args.action == "run":
        args.cid = args.cid or args.idea  # collide run <cid> 位置參數
    args.func(args)


if __name__ == "__main__":
    main()
