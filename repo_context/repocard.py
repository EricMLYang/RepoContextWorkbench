"""repo 狀態卡（2026-09-25 跨 repo 參考輪）——「這個 repo 現在怎樣」一張卡看完。

pulse 只給 commit 主旨，看不出 repo 的狀況。這張卡依「對使用者多有用」排：
1. 你上次看過之後的變化（水位線記在 .state/seen/<id>.json；沒看過＝最近 5 則）
2. 放著沒 commit 的改動與放了幾天（忘了收尾的工作藏在這）
3. 變化集中在哪些目錄（numstat 依頂層目錄彙總，比主旨更說得出在忙什麼）
4. 正在跑的 dev server／程序，以及它啟動後有沒有檔案又改過（「要不要重開」）
5. agent 最後一次交接／沒交接就結束的會話
6. agent 文件裡指向別的 repo 卻失效的路徑、引用過的上游有沒有變
7. 還沒 push 的 commit

水位線只有人看卡（CLI 不帶 --peek）才前進；agent 透過 MCP 讀不動它——那是人的「已讀」。
"""
import datetime as _dt
import json
import os
import re
import subprocess
from collections import defaultdict
from pathlib import Path

from . import collect as _collect
from . import crossref as _xref
from . import registry
from . import spine as _spine

_DEV_NAMES = {"node", "python", "python3", "bun", "deno", "ruby", "java", "uvicorn",
              "gunicorn", "php", "dotnet", "go", "cargo", "flask", "rails", "hugo"}
_SELF_MARKERS = ("repo_context", " ctx ")


def _git(path, *args, timeout=15):
    try:
        r = subprocess.run(["git", "-c", "core.quotePath=false", "-C", str(path), *args],
                           capture_output=True, text=True, encoding="utf-8",
                           errors="replace", timeout=timeout)
    except (OSError, subprocess.SubprocessError):
        return 1, ""
    return r.returncode, (r.stdout or "").rstrip("\n")


# ---------------------------------------------------------------- 水位線

def _seen_path(spine_dir, repo_id):
    safe = re.sub(r"[^\w\-]", "_", repo_id)
    return Path(spine_dir) / ".state" / "seen" / f"{safe}.json"


def get_seen(spine_dir, repo_id):
    try:
        return json.loads(_seen_path(spine_dir, repo_id).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None


def mark_seen(spine_dir, repo_id, head, now=None):
    now = now or _dt.datetime.now()
    p = _seen_path(spine_dir, repo_id)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps({"head": head, "at": f"{now:%Y-%m-%d %H:%M}"}), encoding="utf-8")


# ---------------------------------------------------------------- 各段

def _commits(root, rng=None, limit=30):
    args = ["log", f"-{limit}", "--format=%h%x09%ad%x09%s", "--date=short"]
    if rng:
        args.append(rng)
    rc, out = _git(root, *args)
    if rc != 0:
        return None
    rows = []
    for ln in out.splitlines():
        p = ln.split("\t", 2)
        if len(p) == 3:
            rows.append({"hash": p[0], "date": p[1], "subject": p[2]})
    return rows


def areas(root, rng):
    """rng（例 abc123..HEAD 或 --since=7.days）內的改動依頂層目錄彙總：
    [{area, files, added, deleted}]，改動量大的在前。"""
    args = ["log", "--numstat", "--format="] + (rng if isinstance(rng, list) else [rng])
    rc, out = _git(root, *args)
    if rc != 0 or not out:
        return []
    agg = defaultdict(lambda: {"files": set(), "added": 0, "deleted": 0})
    for ln in out.splitlines():
        p = ln.split("\t")
        if len(p) != 3:
            continue
        a, d, f = p
        f = f.split(" => ")[-1].strip("{}")
        top = f.split("/", 1)[0] + "/" if "/" in f else "（根目錄）"
        agg[top]["files"].add(f)
        agg[top]["added"] += int(a) if a.isdigit() else 0
        agg[top]["deleted"] += int(d) if d.isdigit() else 0
    rows = [{"area": k, "files": len(v["files"]), "added": v["added"], "deleted": v["deleted"]}
            for k, v in agg.items()]
    rows.sort(key=lambda x: -(x["added"] + x["deleted"] + x["files"]))
    return rows


def dirty_files(root, now=None):
    """[{file, status, days}]，放最久的在前。"""
    import time
    now = now or time.time()
    rc, out = _git(root, "status", "--porcelain")
    if rc != 0:
        return []
    rows = []
    for ln in out.splitlines():
        if len(ln) < 4:
            continue
        status, f = ln[:2].strip() or "?", ln[3:].split(" -> ")[-1].strip('"')
        fp = root / f
        days = None
        try:
            days = (now - fp.stat().st_mtime) / 86400.0
        except OSError:
            pass
        rows.append({"file": f, "status": status, "days": round(days, 1) if days is not None else None})
    rows.sort(key=lambda x: -(x["days"] or 0))
    return rows


def _ps_rows():
    """[(pid, start datetime, command)]；沒有 ps（Windows）回空。"""
    try:
        r = subprocess.run(["ps", "-axo", "pid=,lstart=,command="], capture_output=True,
                           text=True, encoding="utf-8", errors="replace", timeout=5)
    except (OSError, subprocess.SubprocessError):
        return []
    rows = []
    for ln in r.stdout.splitlines():
        p = ln.split(None, 6)
        if len(p) < 7 or not p[0].isdigit():
            continue
        try:
            start = _dt.datetime.strptime(" ".join(p[1:6]), "%a %b %d %H:%M:%S %Y")
        except ValueError:
            continue
        rows.append((int(p[0]), start, p[6]))
    return rows


def _cwd_pids(root):
    """cwd 在 root 底下的程序 pid（lsof；沒有 lsof 回空集合）。"""
    try:
        r = subprocess.run(["lsof", "-a", "-d", "cwd", "-Fpn"], capture_output=True,
                           text=True, encoding="utf-8", errors="replace", timeout=5)
    except (OSError, subprocess.SubprocessError):
        return set()
    pids, pid = set(), None
    rs = str(root)
    for ln in r.stdout.splitlines():
        if ln.startswith("p"):
            pid = int(ln[1:]) if ln[1:].isdigit() else None
        elif ln.startswith("n") and pid and (ln[1:] == rs or ln[1:].startswith(rs + os.sep)):
            pids.add(pid)
    return pids


def running(root, ps_rows=None, cwd_pids=None):
    """跟這個 repo 有關的 dev 程序：指令列含 repo 路徑，或 cwd 在 repo 內且是常見 runtime。
    每個帶 started 與「啟動後改過的 tracked 檔」——回答「改了要不要重開」。"""
    root = Path(root).resolve()
    rows = _ps_rows() if ps_rows is None else ps_rows
    cwd_pids = _cwd_pids(root) if cwd_pids is None else cwd_pids
    me = os.getpid()
    found = []
    for pid, start, cmd in rows:
        if pid == me or any(m in f" {cmd} " for m in _SELF_MARKERS):
            continue
        first = cmd.split()[0] if cmd.split() else ""
        if "/node_modules/" in first:   # esbuild 之類的子程序：主程序（node …/vite）已代表它
            continue
        exe = Path(first).name.lower()
        by_cmd = str(root) + os.sep in cmd or cmd.endswith(str(root))
        by_cwd = pid in cwd_pids and (exe in _DEV_NAMES or any(
            f"/{n} " in f"{cmd} " or cmd.startswith(n + " ") for n in _DEV_NAMES))
        if not (by_cmd or by_cwd):
            continue
        found.append({"pid": pid, "started": f"{start:%Y-%m-%d %H:%M}", "_start": start,
                      "command": cmd[:160]})
    if not found:
        return []
    rc, out = _git(root, "ls-files", "-z")
    tracked = [f for f in out.split("\0") if f] if rc == 0 else []
    mtimes = []
    for f in tracked:
        try:
            mtimes.append((os.path.getmtime(root / f), f))
        except OSError:
            continue
    for p in found:
        ts = p.pop("_start").timestamp()
        changed = sorted((m, f) for m, f in mtimes if m > ts)
        p["changed_since_start"] = len(changed)
        p["changed_files"] = [f for _, f in changed[-5:][::-1]]
    return found


def _last_agent(spine_dir, repo_id):
    handoff, unhanded = None, None
    for ev in _spine.iter_events(spine_dir):
        if ev.kv("repo") != repo_id:
            continue
        first = (ev.body or "").splitlines()[0] if ev.body else ""
        if ev.type == "decision" and first.startswith("交接"):
            handoff = {"at": f"{ev.date} {ev.time}", "source": ev.source, "text": first[3:].strip()}
        elif ev.type == "suggestion" and ev.source == "hook":
            unhanded = {"at": f"{ev.date} {ev.time}", "text": first}
    return handoff, unhanded


# ---------------------------------------------------------------- 組卡

def build(spine_dir, repo_id, since=None, mark=False, now=None, ps_rows=None, cwd_pids=None):
    """一個 repo 的狀態卡 dict。since＝'seen'（預設：上次看過之後）｜'recent'（最近 5 則）｜'7d' 之類｜commit hash。
    mark=True：看完把水位線推到目前 HEAD（人看卡才推；agent 不推）。"""
    entry = registry.get_repo(spine_dir, repo_id)
    st = _collect.collect_repo(entry)
    root = Path(entry["path"]).expanduser()
    card = {"ok": True, "repo": repo_id, "path": str(root), "tier": entry.get("tier", "active"),
            "exists": st["exists"], "branch": st.get("branch"), "note": st.get("note") or "",
            "ahead": st.get("ahead"), "behind": st.get("behind"),
            "commits_7d": st.get("commits_7d"), "commits_30d": st.get("commits_30d"),
            "last_commit_days": st.get("last_commit_days")}
    if not st["exists"] or st.get("note") == "非 git repo" or entry.get("type") == "external":
        return card
    rc, head = _git(root, "rev-parse", "--short", "HEAD")
    head = head if rc == 0 else None
    seen = get_seen(spine_dir, repo_id)
    since = since or "seen"
    if since == "seen" and seen and seen.get("head"):
        commits = _commits(root, f"{seen['head']}..HEAD")
        if commits is None:  # 水位線 commit 不見了（rebase）→ 退回最近 5 則
            window = {"kind": "recent", "label": "上次看過的 commit 已不在歷史裡；列最近 5 則"}
            commits = _commits(root, limit=5) or []
            rng = ["-5"]
        else:
            window = {"kind": "seen", "label": f"你上次看是 {seen['at']}（{seen['head']}）",
                      "since": seen["at"]}
            rng = [f"{seen['head']}..HEAD"]
    elif since in ("seen", "recent"):
        window = {"kind": "recent", "label": "第一次看這個 repo；列最近 5 則"
                  if since == "seen" else "最近 5 則"}
        commits = _commits(root, limit=5) or []
        rng = ["-5"]
    elif re.fullmatch(r"\d+d", since):
        window = {"kind": "days", "label": f"近 {since[:-1]} 天"}
        commits = _commits(root, f"--since={since[:-1]}.days") or []
        rng = [f"--since={since[:-1]}.days"]
    else:
        window = {"kind": "commit", "label": f"{since} 之後"}
        commits = _commits(root, f"{since}..HEAD")
        if commits is None:
            raise ValueError(f"找不到 commit：{since}")
        rng = [f"{since}..HEAD"]
    card.update({
        "head": head, "window": window, "new_commits": commits,
        "areas": areas(root, rng) if commits else [],
        "dirty": dirty_files(root),
        "running": running(root, ps_rows=ps_rows, cwd_pids=cwd_pids),
    })
    handoff, unhanded = _last_agent(spine_dir, repo_id)
    card["last_handoff"], card["last_unhanded"] = handoff, unhanded
    card["broken_refs"] = _xref.lint_doc_refs(spine_dir, [repo_id])
    card["drift"] = _xref.drift(spine_dir, consumer=repo_id)
    if mark and head:
        mark_seen(spine_dir, repo_id, head, now)
        card["marked_seen"] = head
    return card


def render(card):
    """狀態卡 → 一屏內的人話。沒事的段落不印（無事不報）。"""
    if not card.get("exists"):
        return f"# {card['repo']}\n路徑不存在：{card['path']}"
    out = [f"# {card['repo']}（{card.get('branch') or '-'}，{card['tier']}）  {card['path']}"]
    if card.get("note"):
        out.append(card["note"])
    if "window" not in card:
        return "\n".join(out)
    nc = card["new_commits"] or []
    out.append(f"\n## 變化：{card['window']['label']}")
    if not nc:
        out.append("沒有新 commit。")
    for c in nc[:10]:
        out.append(f"- {c['date']} {c['hash']} {c['subject']}")
    if len(nc) > 10:
        out.append(f"- …另 {len(nc) - 10} 則")
    if card["areas"]:
        out.append("改動集中在：" + "、".join(
            f"{a['area']} {a['files']} 檔 +{a['added']}/-{a['deleted']}" for a in card["areas"][:5]))
    if card["dirty"]:
        old = [d for d in card["dirty"] if (d["days"] or 0) >= 1]
        out.append(f"\n## 沒 commit 的改動：{len(card['dirty'])} 個檔"
                   + (f"（放最久 {card['dirty'][0]['days']:.0f} 天）" if old else ""))
        for d in card["dirty"][:6]:
            age = f"{d['days']:.1f} 天" if d["days"] is not None else "?"
            out.append(f"- [{d['status']}] {d['file']}（{age}）")
        if len(card["dirty"]) > 6:
            out.append(f"- …另 {len(card['dirty']) - 6} 個")
    if card["running"]:
        out.append("\n## 正在跑")
        for p in card["running"]:
            line = f"- pid {p['pid']}，{p['started']} 啟動：{p['command']}"
            if p["changed_since_start"]:
                line += (f"\n  啟動後改過 {p['changed_since_start']} 個檔（最近：{'、'.join(p['changed_files'][:3])}）"
                         "——不會自動重載的部分要重開")
            else:
                line += "\n  啟動後沒有 tracked 檔改過"
            out.append(line)
    if card.get("ahead"):
        out.append(f"\n還沒 push：{card['ahead']} 個 commit")
    lh, lu = card.get("last_handoff"), card.get("last_unhanded")
    if lh or lu:
        out.append("\n## agent")
        if lh:
            out.append(f"- 最後交接 {lh['at']}（{lh['source']}）：{lh['text']}")
        if lu and (not lh or lu["at"] > lh["at"]):
            out.append(f"- {lu['at']} {lu['text']}")
    if card["broken_refs"]:
        out.append(f"\n## agent 文件裡失效的跨 repo 路徑：{len(card['broken_refs'])} 處")
        for b in card["broken_refs"][:5]:
            out.append(f"- {b['doc']}:{b['line']} `{b['ref']}` → {b['fix']}")
    if card["drift"]:
        out.append("\n## 引用過的上游有變")
        out += _xref.drift_lines(card["drift"])
    if card.get("marked_seen"):
        out.append(f"\n（已記下看到 {card['marked_seen']}；下次只列之後的變化。--peek 不記）")
    return "\n".join(out)
