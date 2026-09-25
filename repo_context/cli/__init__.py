"""CLI——人的介面（同一組原語也經 MCP server 給 agent）。

2026-09-25 架構檢查：原本 1110 行一個檔，拆成依領域的模組，各自 `register(sub)` 自己的子命令：
    common（spine 定位／錯誤出口／--json）、admin（init／registry／組，寫入走 ops.py）、
    daily（日常原語）、apps（會話／MCP／工作台）、agent（agentapi 入口）、setup（hook／安裝）。
新增子命令：放進對應模組的 register()；要跨入口共用的管理寫入，先在 ops.py 加 @op。
exit code 約定見 common.py。
"""
import argparse
import sys

from . import admin, agent, apps, daily, setup
from .admin import cmd_init  # noqa: F401（測試與既有呼叫端）

MODULES = (admin, daily, apps, agent, setup)  # 順序＝`ctx --help` 列出的順序


def build_parser():
    p = argparse.ArgumentParser(prog="ctx", description="repo-context 工作台：把一群 repo 分成組，每組組成一份 context")
    p.add_argument("--spine", help="spine repo 路徑（預設 $REPOENGINE_SPINE → ctx setup 記住的 → cwd）")
    p.add_argument("--json", action="store_true",
                   help="結構化輸出（agent 用；錯誤也是 JSON，exit code 見 cli/common.py）")
    sub = p.add_subparsers(dest="cmd", required=True)
    for m in MODULES:
        m.register(sub)
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
