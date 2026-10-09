#!/usr/bin/env python3
"""工作台畫面資料對拍：同一個 spine，Python `webui.state.build_state/build_scan` vs Rust `ctx_core::workbench`。

用法（repo 根目錄）：
    python3 scripts/parity_state.py            # 建 temp spine、兩邊各跑一次、比 JSON
    python3 scripts/parity_state.py --keep     # 留下 temp 目錄（印路徑）
    python3 scripts/parity_state.py --spine D  # 用現成的 spine（唯讀，不寫任何東西）

比的是解析後的 JSON（不比位元組、不比鍵序）。不穩定的欄位先正規化：
- 浮點數四捨五入到 1 位（`last_commit_days`、`dirty_days` 依「現在」算，兩邊跑的時間差幾秒）
- `summary.at`（產生時刻，分鐘精度）
其餘欄位應完全相同；不同就列出路徑與兩邊的值，exit 1。
"""
import argparse
import datetime as dt
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
os.environ.setdefault("REPOCTX_NO_BACKGROUND", "1")

SCENARIOS = ("state_all", "state_g1", "state_adhoc", "scan_all", "scan_g1", "scan_adhoc")


def _git(cwd, *args, when=None):
    env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t",
               GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t")
    if when:
        env.update(GIT_AUTHOR_DATE=when, GIT_COMMITTER_DATE=when)
    subprocess.run(["git", "-C", str(cwd), *args], check=True, env=env,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def make_repo(base, name, days_old, dirty=0):
    p = base / name
    (p / "notes").mkdir(parents=True)
    _git(p, "init", "-q")
    (p / "README.md").write_text(f"# {name}\n這個 repo 談 widget 快取設計。\n", encoding="utf-8")
    (p / "notes" / "plan.md").write_text("## 計劃\n已知結論：快取要用 LRU。\n", encoding="utf-8")
    _git(p, "add", "-A")
    when = (dt.datetime.now() - dt.timedelta(days=days_old)).strftime("%Y-%m-%dT%H:%M:%S")
    _git(p, "commit", "-q", "-m", f"init {name}", when=when)
    old = (dt.datetime.now() - dt.timedelta(days=5)).timestamp()
    for i in range(dirty):
        f = p / f"wip{i}.md"
        f.write_text("未提交草稿\n", encoding="utf-8")
        os.utime(f, (old, old))
    return p


def build_fixture(base):
    """spine＋兩個假 repo＋兩組＋關係＋各種事件（涵蓋收件匣每一種卡、範圍歸屬的各種來源）。"""
    from repo_context import collide, ops, registry, spine
    sp = base / "spine-repo"
    subprocess.run([sys.executable, "-m", "repo_context", "init", str(sp)], check=True,
                   cwd=ROOT, stdout=subprocess.DEVNULL)
    ra = make_repo(base, "repo-a", 1)
    rb = make_repo(base, "repo-b", 6, dirty=2)
    registry.add_repo(sp, "repo-a", str(ra))
    registry.add_repo(sp, "repo-b", str(rb))
    registry.add_group(sp, "g1", ["repo-a", "repo-b"])
    registry.add_group(sp, "g2", ["repo-b"])
    ops.call(sp, "registry_relate", {"a": "repo-a", "b": "repo-b", "kind": "pm-of", "note": "規劃"},
             via="cli")
    ops.call(sp, "registry_tag", {"id": "repo-a", "add": ["PM"]}, via="mcp")
    ops.call(sp, "group_goal", {"name": "g1", "text": "把兩邊說法對齊"}, via="ui")
    now = dt.datetime.now().replace(second=0, microsecond=0)
    t = lambda m: now - dt.timedelta(minutes=m)  # noqa: E731
    spine.append_event(sp, "open-loop", "hotkey", ["#1", "repo:repo-a", "due:2030-01-01"],
                       body="opened →「補驗收條件」", when=t(50))
    spine.append_event(sp, "open-loop", "agent", ["#2", "group:g2"],
                       body="opened →「B 的待辦」", when=t(49))
    c1 = collide.submit(sp, "已判定的想法", group="g1", when=t(45))
    collide.run_judgement(sp, c1, provider="mock", when=t(44))
    collide.submit(sp, "還在等判定", repos=["repo-a"], when=t(40))
    collide.submit(sp, "全域的想法", when=t(39))
    spine.append_event(sp, "suggestion", "engine", ["repo:repo-b"], body="B 的更新\n細節一",
                       when=t(30))
    spine.append_event(sp, "suggestion", "timer", [], body="system-unsure：全域排程失敗",
                       when=t(25))
    spine.append_event(sp, "collision", "engine", ["id:zz-1"],
                       body="判定失敗（system-unsure，准打斷）：boom", when=t(24))
    ev = spine.append_event(sp, "decision", "agent", ["group:g1"], body="比對完驗收條件",
                            when=t(20))
    spine.append_event(sp, "chosen", "monitor", [f"ref:decision:{ev.time}"],
                       body="忽略並記錄：x", when=t(19))
    spine.append_event(sp, "outcome", "manual", [f"ref:collision:{c1}"], body="落地了",
                       when=t(10))
    spine.append_event(sp, "chosen", "monitor", ["repo:repo-b"],
                       body="7 天後再提醒\nsnooze:inactive until:2099-01-01", when=t(5))
    return sp


def python_dump(sp):
    from repo_context.webui.state import build_scan, build_state
    return {
        "state_all": build_state(sp),
        "state_g1": build_state(sp, group="g1"),
        "state_adhoc": build_state(sp, repos=["repo-a"]),
        "scan_all": build_scan(sp),
        "scan_g1": build_scan(sp, "g1"),
        "scan_adhoc": build_scan(sp, repos=["repo-a"]),
    }


# Rust 才有的卡片動作（比對前從 Rust 那邊拿掉）：F-42 未結卡的「用任務包開會話」
RUST_ONLY_ACTIONS = {"brief_session"}


def drop_rust_only(v):
    if isinstance(v, dict):
        if isinstance(v.get("actions"), list):
            v["actions"] = [a for a in v["actions"]
                            if not (isinstance(a, dict) and a.get("action") in RUST_ONLY_ACTIONS)]
        for x in v.values():
            drop_rust_only(x)
    elif isinstance(v, list):
        for x in v:
            drop_rust_only(x)
    return v


def rust_dump(sp):
    out = subprocess.run(["cargo", "run", "-q", "-p", "ctx-core", "--example", "workbench_dump",
                          "--", str(sp)], cwd=ROOT, check=True, capture_output=True, text=True)
    return drop_rust_only(json.loads(out.stdout))


def normalize(v, path=()):
    if isinstance(v, float):
        return round(v, 1)
    if isinstance(v, dict):
        return {k: ("<at>" if path[-1:] == ("summary",) and k == "at" else normalize(x, path + (k,)))
                for k, x in v.items()}
    if isinstance(v, list):
        return [normalize(x, path + (i,)) for i, x in enumerate(v)]
    return v


def diff(a, b, path="$"):
    out = []
    if isinstance(a, dict) and isinstance(b, dict):
        for k in sorted(set(a) | set(b), key=str):
            if k not in a:
                out.append((f"{path}.{k}", "<缺>", b[k]))
            elif k not in b:
                out.append((f"{path}.{k}", a[k], "<缺>"))
            else:
                out += diff(a[k], b[k], f"{path}.{k}")
    elif isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            out.append((f"{path}[len]", len(a), len(b)))
        for i, (x, y) in enumerate(zip(a, b)):
            out += diff(x, y, f"{path}[{i}]")
    elif a != b and not (isinstance(a, (int, float)) and isinstance(b, (int, float)) and a == b):
        out.append((path, a, b))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--spine")
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()
    tmp = None
    if args.spine:
        sp = Path(args.spine)
    else:
        tmp = Path(tempfile.mkdtemp(prefix="parity_state_"))
        sp = build_fixture(tmp)
    try:
        py = normalize(json.loads(json.dumps(python_dump(sp), ensure_ascii=False, default=str)))
        rs = normalize(rust_dump(sp))
        total = 0
        for name in SCENARIOS:
            d = diff(py.get(name), rs.get(name), name)
            total += len(d)
            print(f"{'OK ' if not d else 'DIFF'} {name}" + (f"（{len(d)} 處）" if d else ""))
            for p, a, b in d[:40]:
                print(f"    {p}\n      python: {json.dumps(a, ensure_ascii=False)[:200]}"
                      f"\n      rust:   {json.dumps(b, ensure_ascii=False)[:200]}")
        print(f"spine: {sp}")
        print("全部相同" if not total else f"共 {total} 處不同")
        return 1 if total else 0
    finally:
        if tmp and not args.keep:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
