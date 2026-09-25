"""管理：init、registry、組。寫入一律走操作表（ops.py）——參數驗證、留痕跟工作台／MCP 同一份。

2026-09-25 架構檢查：registry 的十幾個 action 以前共用同一組位置參數（id/path_arg/key/value），
`registry set <id> <key> <value>` 實際落在 id/path_arg/key 三格；現在每個 action 一個子 parser。
"""
import subprocess
import sys
from pathlib import Path

from .. import context as _context
from .. import crossref as _xref
from .. import ops as _ops
from .. import registry as _registry
from .. import summary as _summary
from .common import SCAFFOLD_CONFIG, SCAFFOLD_GITIGNORE, _emit, _fail, _spine_dir


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


def _op(args, op_name, /, **opargs):
    """跑一個操作：錯誤走 _fail（--json 時是 {ok:false}），成功印人話或 JSON。"""
    d = _spine_dir(args)
    try:
        result, text = _ops.call(d, op_name, opargs, via="cli")
    except ValueError as e:
        _fail(args, "invalid", str(e))
    _emit(args, result, lambda _: text)


# ── registry ────────────────────────────────────────────────────────────

def _registry_list(args):
    _emit(args, _registry.load(_spine_dir(args))["repos"], lambda rows: "\n".join(
        f"{r['id']:<24} {r.get('type','mine'):<9} {r.get('tier','?'):<9} {r['path']}"
        for r in rows))


def _registry_scan(args):
    # mani「init 自動掃描＋手寫補語意」混合模式：預設只列候選，--apply 才登記
    d = _spine_dir(args)
    try:
        if args.apply:
            added = _registry.scan_register(d, args.dir)
            for cid in added:
                print(f"已登記：{cid}（下半身預設 mine/active；"
                      f"上半身 type/tier/tags 用 registry set 補）")
            if not added:
                print("無新 repo 可登記")
            return
        cands = _registry.scan_dir(d, args.dir)
    except ValueError as e:
        _fail(args, "invalid", str(e))
    for path, cid in cands:
        print(f"[候選] {cid}: {path}")
    print(f"共 {len(cands)} 個未登記 git repo（加 --apply 登記）" if cands else "無未登記 repo")


def _registry_audit(args):
    d = _spine_dir(args)
    reds = _registry.audit(d, scan_dirs=args.scan or [])
    reds += [f"[文件路徑失效] {b['repo']}/{b['doc']}:{b['line']} `{b['ref']}`：{b['fix']}"
             for b in _xref.lint_doc_refs(d)]
    if reds:
        for r in reds:
            print(f"[RED] {r}")
        sys.exit(1)
    print("audit 零紅字 OK")


def _registry_relations(args):
    d = _spine_dir(args)
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


def _registry_exports(args):
    rows = [(r["id"], n, v) for r in _registry.load(_spine_dir(args))["repos"]
            if not args.id or r["id"] == args.id
            for n, v in _registry.exports_of(r).items()]
    _emit(args, [{"repo": r, "name": n, **v} for r, n, v in rows], lambda xs: "\n".join(
        f"{x['repo']}:{x['name']:<12} → {x['path']}" + (f"  {x['desc']}" if x.get("desc") else "")
        for x in xs) or "（沒有 export）用 registry export <repo> <名稱> <路徑> 登記")


# ── 組 ──────────────────────────────────────────────────────────────────

def _group_list(args):
    _emit(args, _registry.load(_spine_dir(args))["groups"], lambda gs: "\n".join(
        f"{g['name']:<20} {','.join(g['members'])}" for g in gs))


def _group_goal(args):
    # 目標只有人能寫（機器不替你發明）；不給 --text＝只讀
    if args.text is None:
        try:
            g = _registry.get_group(_spine_dir(args), args.name)
        except ValueError as e:
            _fail(args, "invalid", str(e))
        print(g.get("goal") or "（尚未登記目標）")
    else:
        _op(args, "group_goal", name=args.name, text=args.text)


def _actions(subparsers):
    """子命令宣告簡寫：action(名稱, 函式, 說明, (位置參數, nargs, help)..., 旗標=argparse kwargs...)。"""
    def action(name, fn, hlp, *pos, **flags):
        a = subparsers.add_parser(name, help=hlp)
        for p in pos:
            a.add_argument(p[0], nargs=p[1] if len(p) > 1 else None,
                           help=p[2] if len(p) > 2 else None)
        for f, kw in flags.items():
            a.add_argument("--" + f.replace("_", "-"), dest=f, **kw)
        a.set_defaults(func=fn)
        return a
    return action


def register(sub):
    s = sub.add_parser("init", help="建 spine repo scaffold")
    s.add_argument("path")
    s.set_defaults(func=cmd_init)

    reg = sub.add_parser("registry", help="P1 登記/掃描/稽核/tag/關係/export")
    action = _actions(reg.add_subparsers(dest="action", required=True))

    action("add", lambda a: _op(a, "registry_add", id=a.id, path=a.path, type=a.type, tier=a.tier,
                                tags=a.tags, upstream=a.upstream),
           "登記一個 repo", ("id",), ("path",),
           type={"default": "mine", "choices": ["mine", "external"]},
           tier={"default": "active"}, tags={"help": "逗號分隔"}, upstream={})
    action("set", lambda a: _op(a, "registry_set", id=a.id, key=a.key, value=a.value),
           "改 repo 的一個欄位", ("id",), ("key",), ("value",))
    action("tier", lambda a: _op(a, "registry_tier", id=a.id, tier=a.tier,
                                 resume_when=a.resume_when),
           "改 tier（paused 可帶 --resume-when）", ("id",), ("tier",),
           resume_when={"help": "paused 的復工條件"})
    action("remove", lambda a: _op(a, "registry_remove", id=a.id),
           "移出 registry（含所有組的 membership）", ("id",))
    action("tag", lambda a: _op(a, "registry_tag", id=a.id, add=a.add, remove=a.remove),
           "加／移除 tag", ("id",), add={"help": "加（逗號分隔）"}, remove={"help": "移除（逗號分隔）"})
    action("list", _registry_list, "列出 repo")
    action("scan", _registry_scan, "掃目錄找未登記的 git repo", ("dir",),
           apply={"action": "store_true", "help": "把候選實際登記（預設只列出）"})
    action("audit", _registry_audit, "稽核：tier 漂移、路徑失效、未登記、文件路徑失效",
           scan={"action": "append", "help": "額外掃這個目錄找未登記 repo（可重複）"})
    action("relate", lambda a: _op(a, "registry_relate", a=a.a, b=a.b, kind=a.kind, note=a.note,
                                   exports=a.export),
           "設關係 A --kind--> B（例 A pm-of B＝A 是 B 的 PM）", ("a",), ("b",),
           kind={"required": True, "help": "pm-of/feeds/derived-from/upstream-of/sibling-topic"
                                           "（config relations.kinds 可擴充）"},
           note={"help": "一句說明"}, export={"help": "這條關係用到 A 的哪些 export（逗號分隔）"})
    action("unrelate", lambda a: _op(a, "registry_unrelate", a=a.a, b=a.b, kind=a.kind),
           "刪關係（不給 --kind＝刪 A→B 全部）", ("a",), ("b",), kind={})
    action("relations", _registry_relations, "讀關係（給 id＝站在該 repo 兩向讀）", ("id", "?"))
    action("export", lambda a: _op(a, "registry_export", id=a.id, name=a.name, path=a.path,
                                   desc=a.note),
           "登記 export：<repo>:<名稱> → repo 內路徑", ("id",), ("name",), ("path",),
           note={"help": "一句說明"})
    action("unexport", lambda a: _op(a, "registry_export", id=a.id, name=a.name, remove=True),
           "刪 export", ("id",), ("name",))
    action("exports", _registry_exports, "列出 export", ("id", "?"))

    grp = sub.add_parser("group", help="P2 組（add／add-member／remove-member／rename／remove；"
                                       "context＝組情境卡；summary＝工作摘要；goal＝一句話目標）")
    action = _actions(grp.add_subparsers(dest="action", required=True))

    action("add", lambda a: _op(a, "group_add", name=a.name, members=a.members),
           "建組", ("name",), ("members", None, "逗號分隔 repo id"))
    action("list", _group_list, "列出組")
    action("add-member", lambda a: _op(a, "group_update", name=a.name, add=a.members),
           "加入組員（repo 需已登記）", ("name",), ("members", None, "逗號分隔 repo id"))
    action("remove-member", lambda a: _op(a, "group_update", name=a.name, remove=a.members),
           "移出組（repo 登記保留）", ("name",), ("members", None, "逗號分隔 repo id"))
    action("rename", lambda a: _op(a, "group_update", name=a.name, rename=a.new_name),
           "改名（groups/<組>/ 的料跟著搬）", ("name",), ("new_name",))
    action("remove", lambda a: _op(a, "group_remove", name=a.name),
           "刪組（repo 登記與 groups/<組>/ 的料保留）", ("name",))
    action("goal", _group_goal, "一句話目標（不給 --text＝只讀）", ("name",),
           text={"help": "這組要達成什麼（一句話）；空字串＝清除"})
    action("context", lambda a: print(_context.build_context(
        _spine_dir(a), a.name or None, a.members or None)),
           "組情境卡（會話開場料同源；省略組名＝全部）", ("name", "?"),
           ("members", "?", "逗號分隔 repo id（臨時組合）"))
    action("summary", lambda a: _emit(a, _summary.build_summary(
        _spine_dir(a), a.name or None, a.members or None), _summary.format_summary),
           "本組工作摘要（工作台首屏同一份，每格帶依據）", ("name", "?"),
           ("members", "?", "逗號分隔 repo id（臨時組合）"))
