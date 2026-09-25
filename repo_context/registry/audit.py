"""掃描與稽核：找未登記 repo、tier 漂移、路徑失效、關係斷鏈。"""
import subprocess
import time
from pathlib import Path

from .repos import add_repo
from .store import load


def _git(repo_path, *args):
    r = subprocess.run(["git", "-C", str(repo_path)] + list(args),
                       capture_output=True, text=True, encoding="utf-8")
    return r.returncode, (r.stdout or "").strip()


def last_commit_days(repo_path):
    rc, out = _git(repo_path, "log", "-1", "--format=%ct")
    if rc != 0 or not out:
        return None
    return (time.time() - int(out)) / 86400.0


def scan_dir(spine_dir, base_dir):
    """mani「init 自動掃描」課（拆機報告 §2.2）：找 base_dir 下含 .git 的資料夾，
    回傳未登記候選 [(path, 建議id)]。id 撞名時加序號（同名資料夾登在別處的情況）。"""
    data = load(spine_dir)
    known_paths = set()
    for r in data["repos"]:
        p = Path(r["path"]).expanduser()
        known_paths.add(p.resolve() if p.exists() else p)
    used_ids = {r["id"] for r in data["repos"]}
    out = []
    base = Path(base_dir).expanduser()
    if not base.is_dir():
        raise ValueError(f"目錄不存在: {base_dir}")
    for child in sorted(base.iterdir()):
        if not (child.is_dir() and (child / ".git").exists()):
            continue
        if child.resolve() in known_paths:
            continue
        cid, n = child.name, 2
        while cid in used_ids:
            cid = f"{child.name}-{n}"
            n += 1
        used_ids.add(cid)
        out.append((child, cid))
    return out


def scan_register(spine_dir, base_dir):
    """混合模式的「自動下半身」：掃描到的 repo 以預設值登記（mine/active/無 tag）；
    上半身（type/tier/tags/血統/關係）留人手寫——狀態欄位機器填，語意我填。"""
    added = []
    for path, cid in scan_dir(spine_dir, base_dir):
        add_repo(spine_dir, cid, path)
        added.append(cid)
    return added


def audit(spine_dir, scan_dirs=None, active_max_days=30, dormant_min_days=7):
    """紅字清單。scan_dirs：額外掃「資料夾在但 registry 沒有」的洞。
    反向漂移（2026-09-08 真資料實錘：MI 線 PM repo 掛 active 沉默 66 天、code repo 掛 dormant
    卻 7 天 4 commits）：dormant 但 dormant_min_days 內有 commit 也算紅字——活動才是真相。"""
    data = load(spine_dir)
    reds = []
    known_paths = set()
    for r in data["repos"]:
        p = Path(r["path"]).expanduser()
        known_paths.add(p.resolve() if p.exists() else p)
        if not p.is_dir():
            reds.append(f"[路徑失效] {r['id']}: {r['path']} 不存在")
            continue
        if r.get("tier") == "paused" and not r.get("resume_when"):
            reds.append(f"[paused 無 resume_when] {r['id']}：說不出復工條件就不准掛 paused")
        if r.get("tier") == "active" and r.get("type") == "mine":
            days = last_commit_days(p)
            if days is None:
                reds.append(f"[非 git repo] {r['id']}: {r['path']}")
            elif days > active_max_days:
                reds.append(f"[tier 漂移] {r['id']}: active 但 {days:.0f} 天無 commit")
        if r.get("tier") == "dormant" and r.get("type") == "mine":
            days = last_commit_days(p)
            if days is not None and days <= dormant_min_days:
                reds.append(f"[tier 漂移] {r['id']}: dormant 但 {days:.0f} 天前有 commit（該升回 active？）")
        if r.get("type") == "external" and not r.get("upstream"):
            reds.append(f"[external 無 upstream] {r['id']}：P4 無從查起")
    ids = {r["id"] for r in data["repos"]}
    for r in data["repos"]:
        for x in r.get("relations") or []:
            if x.get("to") not in ids:
                reds.append(f"[關係斷鏈] {r['id']} → {x.get('to')} ({x.get('kind')})：目標未登記")
    for d in (scan_dirs or []):
        base = Path(d).expanduser()
        if not base.is_dir():
            continue
        for child in base.iterdir():
            if child.is_dir() and (child / ".git").exists() \
                    and child.resolve() not in known_paths:
                reds.append(f"[未登記] {child}: 是 git repo 但 registry 沒有")
    return reds
