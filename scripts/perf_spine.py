#!/usr/bin/env python3
"""規模測試用的合成 spine（ideation N-12）：複製一份 spine，灌入 N 筆合成事件，再量熱路徑。

用法：
  python3 scripts/perf_spine.py <來源 spine> <輸出目錄> [--events 10000] [--days 200]
  # 之後：
  REPOCTX_NO_BACKGROUND=1 target/release/ctx --spine <輸出目錄> todo review --json
  echo '{"session_id":"perf","cwd":"'$PWD'"}' | target/release/ctx --spine <輸出目錄> hook session-start

事件組成（固定亂數種子，每次一樣）：判斷 45%（10% 帶「何時重看」、5% 帶「取代」）、未結 15%（七成之後關掉）、
monitor 建議 20%、簡報 20%；組與 repo 取自來源 spine 的 registry。只寫輸出目錄，不碰來源。
預算（2026-10-10 定）：開場 hook < 500ms、build_state < 300ms、todo review < 300ms。
"""
import argparse
import datetime
import os
import random
import shutil

import yaml

WORDS = ("快取 索引 重建 增量 水位線 部署 驗收 排程 推送 反芻 交接 判斷 知識 關係 會話 終端 前端 後端 "
         "資料 模型 評估 品質 測試 文件 規格 介面 效能 記憶 搜尋 建議").split()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("src")
    ap.add_argument("out")
    ap.add_argument("--events", type=int, default=10000)
    ap.add_argument("--days", type=int, default=200)
    a = ap.parse_args()
    if os.path.exists(a.out):
        raise SystemExit(f"輸出目錄已存在：{a.out}（先刪掉或換一個）")
    shutil.copytree(a.src, a.out, ignore=shutil.ignore_patterns(".git", "hook_sessions"))
    rnd = random.Random(7)
    reg = yaml.safe_load(open(os.path.join(a.out, "registry.yaml"), encoding="utf-8"))
    groups = [g["name"] for g in reg.get("groups") or []] or ["g"]
    repos = [r["id"] for r in reg.get("repos") or []] or ["r"]

    def text(n):
        return "".join(rnd.choice(WORDS) for _ in range(n))

    start = datetime.date.today() - datetime.timedelta(days=a.days)
    evs, dn, loopn = {}, {}, 100000
    for _ in range(a.events):
        d = start + datetime.timedelta(days=rnd.randint(0, a.days - 1))
        ds, hm = d.isoformat(), f"{rnd.randint(8, 23):02d}:{rnd.randint(0, 59):02d}"
        g, r, k = rnd.choice(groups), rnd.choice(repos), rnd.random()
        if k < 0.45:
            dn[ds] = dn.get(ds, 0) + 1
            body = f"{text(6)}：{text(10)}"
            if rnd.random() < 0.1:
                when = (d + datetime.timedelta(days=rnd.randint(1, 120))).isoformat()
                body += f"\n何時重看：{when} 或 {text(3)}"
            if rnd.random() < 0.05 and dn[ds] > 1:
                body += f"\n取代：decision:{ds}-d{dn[ds] + 99}"
            line = f"## {hm} decision [agent] id:{ds}-d{dn[ds] + 100} group:{g}\n{body}\n"
        elif k < 0.6:
            loopn += 1
            due = (d + datetime.timedelta(days=14)).isoformat()
            line = f"## {hm} open-loop [agent] #{loopn} group:{g} due:{due}\nopened → {text(8)}\n"
            if rnd.random() < 0.7:
                d2 = (d + datetime.timedelta(days=rnd.randint(0, 30))).isoformat()
                evs.setdefault(d2, []).append(
                    f"## 23:59 open-loop [agent] #{loopn} group:{g}\nclosed → 做完了\n")
        elif k < 0.8:
            line = f"## {hm} suggestion [monitor] repo:{r}\n{text(8)}\n"
        else:
            line = f"## {hm} presented [morning-brief] group:{g}\n{text(5)}\n"
        evs.setdefault(ds, []).append(line)
    edir = os.path.join(a.out, "spine", "events")
    os.makedirs(edir, exist_ok=True)
    n = 0
    for ds, lines in evs.items():
        lines.sort(key=lambda ln: ln[3:8])
        with open(os.path.join(edir, f"{ds}.md"), "a", encoding="utf-8") as f:
            f.write("".join(lines))
        n += len(lines)
    print(f"合成 spine：{a.out}（{n} 筆事件，{len(evs)} 個事件檔）")


if __name__ == "__main__":
    main()
