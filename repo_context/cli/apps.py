"""開窗：agent 會話、MCP server、工作台（app／瀏覽器）。"""
import datetime as _dt
import subprocess

from .. import session as _session

from .common import _spine_dir


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
    from .. import mcpserver
    mcpserver.serve(_spine_dir(args), admin=args.admin)


def cmd_ui(args):
    from .. import webui
    d = _spine_dir(args)
    webui.serve(d, port=args.port, open_browser=not args.no_browser)


def cmd_app(args):
    from .. import webui
    webui.serve_window(_spine_dir(args), port=args.port)


def register(sub):
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
