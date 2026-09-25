"""CLI 共用：spine 定位、錯誤出口、--json 輸出、init 的 scaffold。"""
from typing import NoReturn
import datetime as _dt
import json
import sys
from pathlib import Path

from .. import config as _config


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
#   - {task: ruminate, at: "06:30"}   # 反芻：別組新進的知識／判斷對上各組手上工作（ctx fresh 看）
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


def _fail(args, error, message, hint=None, code=EXIT_REJECTED) -> NoReturn:
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
