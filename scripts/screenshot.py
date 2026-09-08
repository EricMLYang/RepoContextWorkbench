"""工作台截圖（L4 前先自看；2026-09-02 UI 檢討 §2 根因 3：功能測試通得過的畫面不等於體感通得過）。

用法（PowerShell）：
    $env:PYTHONUTF8="1"
    python scripts/screenshot.py <spine 路徑> [--demo] [--out 目錄] [--size 1280x860] [--hash deep|adhoc]

--demo：複製 spine 到暫存目錄，灌入處理中碰撞／判定回程／system-unsure／open loop／suggestion
        各一則，看「有事」時的收件匣長什麼樣（真 spine 不動）。
需要 Microsoft Edge（Windows 內建）；找不到就印出 URL 讓人自己開。
"""
import argparse
import datetime as dt
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from repoengine import spine, webui  # noqa: E402

EDGE = [r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"]


def seed_demo(sp):
    now = dt.datetime.now()
    t = lambda m: now - dt.timedelta(minutes=m)  # noqa: E731
    spine.append_event(sp, "collision", "hotkey", ["id:demo-a", "group:共通"],
                       body="opened\n輸入：把 card_notes 的對撞流程搬到 repo 尺度，用 MCP 讓 agent 直接查脊椎",
                       when=t(9))
    spine.append_event(sp, "collision", "engine", ["id:demo-a", "ref:collision:demo-a"],
                       body="判定：真增量\n理由：組內只有 personal_agent_design 談過鄰近概念，角度是規格不是實作\n"
                            "證據：AgentCodingPM/personal_agent_design/20260830_工具方向_設計思考收斂.md §7\n"
                            "落點建議：repo:AgentCodingPM/01_inbox\n下一步：開 agent 會話把 §7 動詞鏈對到 MCP 工具清單\n"
                            "〔照建議落〕〔改落點〕〔升格 incubator〕〔深撞：開 agent 視窗〕〔丟棄並記錄〕",
                       when=t(7))
    spine.append_event(sp, "collision", "hotkey", ["id:demo-b", "group:共通"],
                       body="opened\n輸入：週復盤能不能直接從脊椎 chosen 事件自動生", when=t(2))
    spine.append_event(sp, "collision", "engine", ["id:demo-c"],
                       body="判定失敗（system-unsure，准打斷）：provider claude 回傳非 JSON", when=t(5))
    spine.append_event(sp, "open-loop", "hotkey", ["#3", f"due:{now:%Y-%m-%d}"],
                       body="opened →「驗證 pywinpty 在 VDI 上能不能開 claude」", when=t(30))
    # 關係層 demo：前兩個登記的 repo 之間設一條 pm-of（牌展開看 ⇄、深看關係段有列）
    from repoengine import registry as _reg
    ids = [r["id"] for r in _reg.load(sp)["repos"]]
    if len(ids) >= 2 and not _reg.relations(sp):
        _reg.relate(sp, ids[0], ids[1], "pm-of", note="demo")
    spine.append_event(sp, "suggestion", "engine", ["repo:repomix"],
                       body="repomix v1.4.0 釋出：MCP 介面變更，tool_experiments 的 clone 落後 2 版", when=t(20))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("spine")
    ap.add_argument("--demo", action="store_true")
    ap.add_argument("--out", default=str(Path(tempfile.gettempdir()) / "repoengine-shots"))
    ap.add_argument("--size", default="1280x860")
    ap.add_argument("--hash", default="", help="頁面 hash：deep＝直接開深看、adhoc＝開臨時組對話框")
    a = ap.parse_args()
    sp = a.spine
    if a.demo:
        tmp = Path(tempfile.mkdtemp(prefix="spine-demo-"))
        shutil.copytree(sp, tmp / "spine", ignore=shutil.ignore_patterns(".git"))
        sp = str(tmp / "spine")
        seed_demo(sp)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    srv = webui.make_server(sp, port=0, token="shot")
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"
    url = f"{base}/?token=shot" + (f"#{a.hash}" if a.hash else "")
    urllib.request.urlopen(f"{base}/api/scan?token=shot", timeout=180).read()  # 先採集，首屏就有料
    edge = next((e for e in EDGE if os.path.exists(e)), None)
    if not edge:
        print(f"找不到 Edge——自己開：{url}（Enter 結束）")
        input()
    else:
        name = out / f"workbench{'-demo' if a.demo else ''}{'-' + a.hash if a.hash else ''}-{a.size}.png"
        w, h = a.size.split("x")
        subprocess.run([edge, "--headless=new", "--disable-gpu", "--hide-scrollbars",
                        f"--window-size={w},{h}", "--virtual-time-budget=15000",
                        f"--screenshot={name}", url], capture_output=True, timeout=120)
        print(name)
    srv.terms.kill_all()
    srv.shutdown()
    srv.server_close()


if __name__ == "__main__":
    main()
