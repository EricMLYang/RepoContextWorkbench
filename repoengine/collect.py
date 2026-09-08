"""P3 採集：本地 git 狀態。欄位規格抄 gitpane（拆機報告 §2.1）：
分支／dirty（＝變更檔案數）／ahead-behind／worktree 數／agent 偵測。

external repo 不看 dirty（監控語意不同，v1 §3.1）；上游查詢屬 P4，原型留 stub。
dirty_days 以「dirty 檔案中最舊的 mtime」近似（夠用；正式版可改脊椎留痕推算）。
agent 偵測讀 agentmark 的存活標記（只感知引擎自己 spawn 的內嵌會話）。

活動脈動（2026-09-08 組為單位輪）：組的監控專注在「活動頻繁度＋近期 commit 內容」——
每 repo 帶 commits_7d／commits_30d／recent_commits（最新 5 則主旨）；group_pulse 做組級彙總，
recent_across 把組內近期 commit 攤平成一條時間線。全是資訊不是問句（無事不報仍守）。
"""
import os
import subprocess
import time
from pathlib import Path

from . import agentmark


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
          "branch": None, "last_subject": "", "worktrees": 0,
          "commits_7d": None, "commits_30d": None, "recent_commits": [],
          "agents": [], "note": ""}
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
    for key, since in (("commits_7d", "7.days"), ("commits_30d", "30.days")):
        rc, out = _git(p, "rev-list", "--count", f"--since={since}", "HEAD")
        st[key] = int(out) if rc == 0 and out.isdigit() else 0
    rc, out = _git(p, "log", "-5", "--format=%ad%x09%h%x09%s", "--date=short")
    if rc == 0 and out:
        for ln in out.splitlines():
            parts = ln.split("\t", 2)
            if len(parts) == 3:
                st["recent_commits"].append(
                    {"date": parts[0], "hash": parts[1], "subject": parts[2]})
    rc, out = _git(p, "worktree", "list", "--porcelain")
    if rc == 0 and out:  # 主 worktree 不算，只數掛出去的（gitpane 的 agent 並行場景）
        st["worktrees"] = max(
            sum(1 for ln in out.splitlines() if ln.startswith("worktree ")) - 1, 0)
    return st


def _annotate_agents(states, spine_dir):
    """gitpane 課的 agent 偵測：存活 agent 會話的 cwd 落在哪個 repo 底下就標誰。"""
    sessions = agentmark.live(spine_dir)
    if not sessions:
        return
    for s in states:
        try:
            rp = Path(s["path"]).resolve()
        except OSError:
            continue
        for m in sessions:
            try:
                Path(m["cwd"]).resolve().relative_to(rp)
            except (ValueError, OSError):
                continue
            s["agents"].append(m.get("agent") or "agent")


def collect_group(entries, spine_dir=None):
    states = [collect_repo(e) for e in entries]
    if spine_dir:
        _annotate_agents(states, spine_dir)
    return states


def group_pulse(states):
    """組級活動脈動：7d/30d 總 commit、每 repo 7d 數、最活躍、30 天沉默者、最新一則 commit。"""
    mine = [s for s in states if s.get("type", "mine") == "mine" and s.get("exists")]
    per = {s["id"]: (s.get("commits_7d") or 0) for s in mine}
    latest = None
    for s in mine:
        rc = s.get("recent_commits") or []
        if rc:  # recent_commits 已是新→舊，只看第一則
            c = rc[0]
            if latest is None or (c["date"], s["id"]) > (latest["date"], latest["repo"]):
                latest = {"repo": s["id"], **c}
    most = max(per, key=per.get) if per and max(per.values()) > 0 else None
    return {
        "repos": len(mine),
        "commits_7d": sum(per.values()),
        "commits_30d": sum((s.get("commits_30d") or 0) for s in mine),
        "per_repo_7d": per,
        "active_7d": [k for k, v in per.items() if v > 0],
        "quiet_30d": [s["id"] for s in mine if not (s.get("commits_30d") or 0)],
        "most_active": most,
        "latest": latest,
    }


def recent_across(states, limit=20):
    """組內近期 commit 攤平成一條時間線（新→舊）：[{repo, date, hash, subject}]。"""
    rows = []
    for s in states:
        for c in s.get("recent_commits") or []:
            rows.append({"repo": s["id"], **c})
    rows.sort(key=lambda r: (r["date"], r["repo"]), reverse=True)
    return rows[:limit]


def pulse_line(pulse):
    """脈動一行（簡報／CLI／情境卡用）。"""
    if not pulse["repos"]:
        return "（沒有可採集的 mine repo）"
    tops = sorted(pulse["per_repo_7d"].items(), key=lambda kv: -kv[1])
    tops = [f"{k} {v}" for k, v in tops if v > 0][:4]
    line = (f"近 7 天 {pulse['commits_7d']} commits／30 天 {pulse['commits_30d']}"
            + (f"（{'、'.join(tops)}）" if tops else "（全組無新 commit）"))
    if pulse["latest"]:
        line += (f"；最新：{pulse['latest']['repo']} {pulse['latest']['date']}"
                 f"「{pulse['latest']['subject']}」")
    if pulse["quiet_30d"]:
        line += f"；30 天沉默：{'、'.join(pulse['quiet_30d'])}"
    return line


def format_table(states):
    lines = [f"{'repo':<24} {'tier':<8} {'branch':<12} {'dirty':>5} {'dirty天':>7} "
             f"{'末commit天':>9} {'7d':>3} {'30d':>3} {'↑未push':>7} {'↓落後':>6} {'wt':>3} {'agent':<10} note"]
    for s in states:
        dd = f"{s['dirty_days']:.1f}" if s["dirty_days"] is not None else "-"
        lc = f"{s['last_commit_days']:.1f}" if s["last_commit_days"] is not None else "-"
        ah = str(s["ahead"]) if s.get("ahead") is not None else "-"
        bh = str(s["behind"]) if s.get("behind") is not None else "-"
        br = (s.get("branch") or "-")[:12]
        wt = str(s.get("worktrees") or 0)
        ag = ",".join(s.get("agents") or []) or "-"
        c7 = str(s.get("commits_7d")) if s.get("commits_7d") is not None else "-"
        c30 = str(s.get("commits_30d")) if s.get("commits_30d") is not None else "-"
        lines.append(f"{s['id']:<24} {s['tier']:<8} {br:<12} {s['dirty']:>5} {dd:>7} "
                     f"{lc:>9} {c7:>3} {c30:>3} {ah:>7} {bh:>6} {wt:>3} {ag:<10} {s['note']}")
    return "\n".join(lines)
