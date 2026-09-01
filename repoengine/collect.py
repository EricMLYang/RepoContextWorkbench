"""P3 採集：本地 git 狀態（dirty / dirty 幾天 / 最後 commit 幾天 / ahead-behind）。

external repo 不看 dirty（監控語意不同，v1 §3.1）；上游查詢屬 P4，原型留 stub。
dirty_days 以「dirty 檔案中最舊的 mtime」近似（夠用；正式版可改脊椎留痕推算）。
"""
import os
import subprocess
import time
from pathlib import Path


def _git(repo_path, *args):
    r = subprocess.run(["git", "-C", str(repo_path)] + list(args),
                       capture_output=True, text=True, encoding="utf-8")
    return r.returncode, (r.stdout or "").strip()


def collect_repo(entry):
    """entry＝registry repo dict → 狀態 dict。"""
    p = Path(entry["path"]).expanduser()
    st = {"id": entry["id"], "type": entry.get("type", "mine"),
          "tier": entry.get("tier", "active"), "path": str(p),
          "exists": p.is_dir(), "dirty": 0, "dirty_days": None,
          "last_commit_days": None, "ahead": None, "behind": None,
          "branch": None, "last_subject": "", "note": ""}
    if not st["exists"]:
        st["note"] = "路徑不存在"
        return st
    if st["type"] == "external":
        st["note"] = "external：上游查詢待 P4"
        return st
    rc, out = _git(p, "status", "--porcelain")
    if rc != 0:
        st["note"] = "非 git repo"
        return st
    dirty_files = [ln[3:].strip().strip('"') for ln in out.splitlines() if ln.strip()]
    st["dirty"] = len(dirty_files)
    if dirty_files:
        mtimes = []
        for f in dirty_files:
            fp = p / f.split(" -> ")[-1]
            if fp.exists():
                mtimes.append(os.path.getmtime(fp))
        if mtimes:
            st["dirty_days"] = (time.time() - min(mtimes)) / 86400.0
    rc, out = _git(p, "log", "-1", "--format=%ct")
    if rc == 0 and out:
        st["last_commit_days"] = (time.time() - int(out)) / 86400.0
    rc, out = _git(p, "rev-list", "--left-right", "--count", "@{u}...HEAD")
    if rc == 0 and out:
        behind, ahead = out.split()
        st["ahead"], st["behind"] = int(ahead), int(behind)
    rc, out = _git(p, "rev-parse", "--abbrev-ref", "HEAD")
    st["branch"] = out if rc == 0 and out else None
    rc, out = _git(p, "log", "-1", "--format=%s")
    st["last_subject"] = out if rc == 0 else ""
    return st


def collect_group(entries):
    return [collect_repo(e) for e in entries]


def format_table(states):
    lines = [f"{'repo':<24} {'tier':<8} {'branch':<12} {'dirty':>5} {'dirty天':>7} "
             f"{'末commit天':>9} {'↑未push':>7} {'↓落後':>6}  note"]
    for s in states:
        dd = f"{s['dirty_days']:.1f}" if s["dirty_days"] is not None else "-"
        lc = f"{s['last_commit_days']:.1f}" if s["last_commit_days"] is not None else "-"
        ah = str(s["ahead"]) if s.get("ahead") is not None else "-"
        bh = str(s["behind"]) if s.get("behind") is not None else "-"
        br = (s.get("branch") or "-")[:12]
        lines.append(f"{s['id']:<24} {s['tier']:<8} {br:<12} {s['dirty']:>5} {dd:>7} "
                     f"{lc:>9} {ah:>7} {bh:>6}  {s['note']}")
    return "\n".join(lines)
