"""CLI——人的介面（正式版同組原語再暴露成 MCP server 給 agent）。"""
import argparse
import datetime as _dt
import json
import os
import subprocess
import sys
from pathlib import Path

from . import brief as _brief
from . import collect as _collect
from . import collide as _collide
from . import registry as _registry
from . import route as _route
from . import spine as _spine
from . import pack as _pack
from . import config as _config

SCAFFOLD_CONFIG = """\
# config.yaml — 私有設定（公私切分線：路徑、偏好、閾值、provider 全在這）
thresholds:
  dirty_stale_days: 4        # dirty 超過 N 天 → 進未讀
  inactive_days: 21          # active 但 N 天無 commit → 「變化」進未讀
  openloop_default_due_days: 14
provider:
  default: mock              # mock | claude
pack:
  token_budget: 60000
"""

SCAFFOLD_GITIGNORE = ".engine.lock\n.state/\n.agent_logs/\n"


def _spine_dir(args):
    d = args.spine or os.environ.get("REPOENGINE_SPINE") or "."
    d = Path(d).expanduser()
    if not (d / "config.yaml").exists() and not (d / "registry.yaml").exists():
        sys.exit(f"錯誤：{d} 不是 spine repo（找不到 config.yaml；先跑 init）")
    return d


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
    elif args.action == "list":
        for r in _registry.load(d)["repos"]:
            print(f"{r['id']:<24} {r.get('type','mine'):<9} {r.get('tier','?'):<9} {r['path']}")
    elif args.action == "audit":
        reds = _registry.audit(d, scan_dirs=args.scan or [])
        if reds:
            for r in reds:
                print(f"[RED] {r}")
            sys.exit(1)
        print("audit 零紅字 OK")


def cmd_group(args):
    d = _spine_dir(args)
    if args.action == "add":
        _registry.add_group(d, args.name, args.members.split(","))
        print(f"組已建：{args.name}")
    else:
        for g in _registry.load(d)["groups"]:
            print(f"{g['name']:<20} {','.join(g['members'])}")


def cmd_collect(args):
    d = _spine_dir(args)
    _, entries = _registry.resolve_group(d, args.group, args.repos)
    print(_collect.format_table(_collect.collect_group(entries)))


def cmd_pack(args):
    d = _spine_dir(args)
    gname, entries = _registry.resolve_group(d, args.group, args.repos)
    cfg = _config.load(d)
    budget = args.budget or cfg["pack"]["token_budget"]
    text, inc, exc = _pack.pack_group(entries, budget)
    out = d / "groups" / (args.group or "adhoc") / "materials" / \
        f"pack-{_dt.datetime.now():%Y%m%d-%H%M%S}.md"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(text, encoding="utf-8")
    print(f"打包完成：{out}（收錄 {len(inc)}、未納入 {len(exc)}）")


def cmd_brief(args):
    d = _spine_dir(args)
    out, text = _brief.run(d, args.group, args.repos)
    print(text)
    print(f"→ 已寫入 {out} 並落 presented 事件")


def cmd_append(args):
    d = _spine_dir(args)
    ev = _spine.append_event(d, args.type, args.source, args.kv or [],
                             body=args.body or "", dead_letter=args.dead_letter)
    if ev is None:
        print("validator 拒寫 → 原文已落 spine/dead-letter/（想法不丟）")
        sys.exit(1)
    print(f"已寫入：{ev.header()}")


def cmd_query(args):
    d = _spine_dir(args)
    if args.stats:
        s = _spine.stats(d)
        rate = f"{s['hit_rate']:.0%}" if s["hit_rate"] is not None else "n/a（無碰撞）"
        print(f"碰撞 {s['collisions']} 次｜有 outcome 回連 {s['collisions_with_outcome']} 次｜靈感命中率 {rate}")
        return
    date = f"{_dt.date.today():%Y-%m-%d}" if args.today else args.date
    for ev in _spine.query(d, type=args.type, date=date, group=args.group):
        print(f"{ev.date} {ev.header()}")
        if args.verbose and ev.body:
            print("   " + ev.body.replace("\n", "\n   "))


def cmd_loops(args):
    d = _spine_dir(args)
    loops = _spine.open_loops(d)
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
    subprocess.run(["git", "add", "-A"], cwd=d, check=True)
    r = subprocess.run(["git", "commit", "-q", "-m", args.message],
                       cwd=d, capture_output=True, text=True)
    print("已 commit" if r.returncode == 0 else "無變更可 commit")


def cmd_ui(args):
    from . import webui
    d = _spine_dir(args)
    webui.serve(d, port=args.port, open_browser=not args.no_browser)


def build_parser():
    p = argparse.ArgumentParser(prog="repoengine", description="個人 repo 引擎原型")
    p.add_argument("--spine", help="spine repo 路徑（預設 $REPOENGINE_SPINE 或 cwd）")
    sub = p.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("init", help="建 spine repo scaffold")
    s.add_argument("path")
    s.set_defaults(func=cmd_init)

    s = sub.add_parser("registry", help="P1 登記/稽核")
    s.add_argument("action", choices=["add", "set", "list", "audit"])
    s.add_argument("id", nargs="?")
    s.add_argument("path_arg", nargs="?")
    s.add_argument("key", nargs="?")
    s.add_argument("value", nargs="?")
    s.add_argument("--type", default="mine", choices=["mine", "external"])
    s.add_argument("--tier", default="active")
    s.add_argument("--tags")
    s.add_argument("--upstream")
    s.add_argument("--scan", action="append")
    s.set_defaults(func=cmd_registry)

    s = sub.add_parser("group", help="P2 組")
    s.add_argument("action", choices=["add", "list"])
    s.add_argument("name", nargs="?")
    s.add_argument("members", nargs="?")
    s.set_defaults(func=cmd_group)

    for name, fn, hlp in [("collect", cmd_collect, "P3 採集"),
                          ("pack", cmd_pack, "P6 打包"),
                          ("brief", cmd_brief, "早晨簡報（樣貌A）")]:
        s = sub.add_parser(name, help=hlp)
        s.add_argument("--group")
        s.add_argument("--repos")
        if name == "pack":
            s.add_argument("--budget", type=int)
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

    s = sub.add_parser("loops", help="未結 open loops（P11 視圖）")
    s.set_defaults(func=cmd_loops)

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

    s = sub.add_parser("ui", help="S4 監控台雛形（localhost 網頁）")
    s.add_argument("--port", type=int, default=8765)
    s.add_argument("--no-browser", action="store_true")
    s.set_defaults(func=cmd_ui)
    return p


def main(argv=None):
    args = build_parser().parse_args(argv)
    if args.cmd == "collide" and args.action == "run":
        args.cid = args.cid or args.idea  # collide run <cid> 位置參數
    args.func(args)


if __name__ == "__main__":
    main()
