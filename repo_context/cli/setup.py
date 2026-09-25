"""安裝與掛載：Claude Code hook 入口、給 agent 的使用說明、ctx setup。"""
import datetime as _dt
import json
import sys
from pathlib import Path

from .. import config as _config

from .common import EXIT_NO_SPINE, _fail, _spine_dir


def cmd_hook(args):
    """Claude Code hook 入口：stdin 是 hook JSON。永遠 exit 0——hook 壞掉不能擋住開 agent。"""
    from .. import hooks as _hooks
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
    from .. import agentsetup as _setup
    print(_setup.skill_text(frontmatter=False))


def cmd_setup(args):
    from .. import agentsetup as _setup
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


def register(sub):
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
