"""Agent 介面（2026-09-25 Agent 友善輪）：agentapi 的 CLI 入口，--json 回結構化結果。"""
import datetime as _dt
import json
import subprocess
import sys
from pathlib import Path

from .. import agentapi as _api
from .. import crossref as _xref
from .. import repocard as _repocard
from .. import ruminate as _ruminate

from .common import EXIT_FINDINGS, _emit, _fail, _spine_dir


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


def _h_fresh(r):
    if r.get("note"):
        return r["note"]
    head = f"# 反芻推薦（{r['scope']}；上次反芻 {r.get('last_run') or '從未'}）"
    if not r["links"]:
        return head + "\n目前沒有新推薦。" + ("" if r.get("last_run") else "先跑 `ctx fresh --run`。")
    out = [head]
    for l in r["links"]:
        tag = "" if l["status"] == "new" else f"〔{ {'used': '用上了', 'dismissed': '已略過'}[l['status']] }〕"
        out += [ln.replace(f"[{l['id']}]", f"[{l['id']}]{tag}", 1) if i == 0 else ln
                for i, ln in enumerate(_ruminate.link_lines([l], limit=1))]
        out.append(f"    依據：{'、'.join(l['basis'])}；分數 {l['score']}")
    return "\n".join(out)


def cmd_fresh(args):
    d = _spine_dir(args)
    if args.run:
        res = _ruminate.run(d, window_days=args.days)
        n = sum(len(v) for v in res["groups"].values())
        _emit(args, res, lambda r: f"反芻完成：自 {r['since']} 起 {r['candidates']} 個新／改過的檔，"
              f"{n} 筆新推薦" + "".join(f"\n  {g}：{len(v)}" for g, v in r["groups"].items()))
        return
    if args.stats:
        _emit(args, _ruminate.stats(d), lambda s: (
            f"推薦 {s['total']} 筆：未處理 {s['new']}、用上了 {s['used']}、略過 {s['dismissed']}；"
            f"命中率 {s['hit_rate'] if s['hit_rate'] is not None else '—'}（上次反芻 {s['last_run'] or '從未'}）"))
        return
    if args.dismiss:
        try:
            l = _ruminate.dismiss(d, args.dismiss)
        except ValueError as e:
            _fail(args, "not_found", str(e))
        _emit(args, {"ok": True, "dismissed": l}, lambda r: f"已略過 {l['id']} {l['ref']}")
        return
    _agent_call(args, _api.fresh_links, _h_fresh, cwd=args.cwd, group=args.group,
                repos=args.repos, include_seen=args.all or None)


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


def register(sub):
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

    s = sub.add_parser("fresh", help="反芻推薦：別組新進的知識／判斷中，跟主場手上工作強相關的")
    scoped(s)
    s.add_argument("--run", action="store_true", help="現在跑一輪反芻（排程任務 ruminate 做同一件事）")
    s.add_argument("--days", type=int, help="配 --run：回看幾天（預設＝上次反芻之後）")
    s.add_argument("--all", action="store_true", help="連用過／略過的也列")
    s.add_argument("--dismiss", metavar="ID", help="略過一筆推薦，例 r3")
    s.add_argument("--stats", action="store_true", help="命中率：推薦後來有多少被讀了")
    s.set_defaults(func=cmd_fresh)

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
