"""Claude Code hooks（2026-09-25 Agent 友善輪）——留痕從「prompt 拜託」變成機制。

ROLE_PROMPT 寫「沒留痕＝沒回報」，但 agent 做完事常常忘了留；工作台的會話回報、
交接草稿、「接續上次工作」就全空。這裡用 hooks 補兩端：

- SessionStart：依 cwd 找出主場，把精簡版情境（目標／上次交接／下一步／未結／工具約定）
  注入開場脈絡；順手記下會話開始時間。cwd 沒登記＝什麼都不輸出（不打擾非本工具的 repo）。
- SessionEnd：會話期間若沒有交接，但有 commit／未提交檔／留痕，就落一筆 suggestion [hook]
  到收件匣——「這次會話沒交接就結束了」，只陳述採得到的事實，不替 agent 編成果。

hook 的輸入是 Claude Code 從 stdin 給的 JSON（session_id／cwd／hook_event_name…）。
任何錯誤都吞掉並回空字串：hook 壞掉不能擋住使用者開 agent。
"""
import datetime as _dt
import json
import re
import subprocess
from pathlib import Path

from . import agentapi as _api
from . import spine as _spine

TOOLS_LINE = ("context_for／next_work／log_decision／add_todo／close_todo／"
              "report_status／handoff／search_knowledge")


def _marker_dir(spine_dir):
    return Path(spine_dir) / ".state" / "hook_sessions"


def _safe(sid):
    return re.sub(r"[^\w\-]", "_", str(sid or "unknown"))[:80]


def render_start(ctx):
    """context_for 的結果 → 開場注入文字（精簡，一屏內）。"""
    w, sc = ctx["where"], ctx["scope"]
    where = w["repo"]["id"] if w.get("repo") else "（未登記的目錄）"
    out = [f"# repo-context：你在 {where}，工作範圍「{sc['label']}」"
           + (f"（{len(sc['repos'])} 個 repo：{'、'.join(sc['repos'])}）" if sc["repos"] else "")]
    if w.get("note"):
        out.append(f"註：{w['note']}")
    out.append("目標：" + (ctx["goal"]["text"] if ctx["goal"] else f"（{ctx['goal_gap']}）"))
    ho = ctx.get("last_handoff")
    if ho:
        out.append(f"上次交接（{ho['at']}，{ho['source']}）：{ho['summary']}")
        if ho.get("next_step"):
            out.append(f"  下次從這裡接：{ho['next_step']}")
    elif ctx.get("last_progress"):
        lp = ctx["last_progress"]
        out.append(f"上次進度（{lp['at']}，{lp['source']}）：{lp['text']}")
    if ctx.get("next"):
        out.append(f"建議下一步：{ctx['next']['text']}（依據：{ctx['next']['source']}）")
    loops = ctx.get("open_loops") or []
    if loops:
        out.append(f"未結（{len(loops)}）：")
        for lp in loops[:6]:
            flag = "，已逾期" if lp["overdue"] else ""
            out.append(f"- {lp['id']} {lp['text']}" + (f"（due {lp['due']}{flag}）" if lp["due"] else ""))
        if len(loops) > 6:
            out.append(f"- …另 {len(loops) - 6} 項（用 next_work 看完整清單）")
    for a in (ctx.get("attention") or [])[:3]:
        out.append(f"注意：{a['text']}")
    st = ctx.get("agent_status")
    if st and st.get("status") in ("waiting", "blocked"):
        out.append(f"上一個 agent 的狀態：{st['status']}（{st['at']}）{st.get('text', '')}")
    out += [
        "",
        "## 工作約定（repo-context）",
        f"- 範圍＝「{sc['label']}」。寫入其他組會被工具拒絕；要跨組先問使用者。",
        f"- 工具（MCP repo-context）：{TOOLS_LINE}。"
        "沒掛 MCP 就用 `ctx <where|context|next|log|todo|status|handoff|search> --json`。",
        "- 做了判斷→log_decision；發現待辦→add_todo；等使用者或卡住→report_status。",
        "- 收尾一定呼叫 handoff（完成了什麼、剩什麼、下次從哪接）；沒交接的會話會被記成未交接。",
    ]
    return "\n".join(out)


def session_start(spine_dir, payload, now=None):
    """回傳要印到 stdout 的字串（Claude Code SessionStart hook 的 JSON 輸出）；不適用就回空字串。"""
    try:
        cwd = payload.get("cwd") or str(Path.cwd())
        where = _api.where_am_i(spine_dir, cwd)
        if not where["registered"]:
            return ""
        ctx = _api.context_for(spine_dir, cwd=cwd, card=False)
        now = now or _dt.datetime.now()
        d = _marker_dir(spine_dir)
        d.mkdir(parents=True, exist_ok=True)
        (d / f"{_safe(payload.get('session_id'))}.json").write_text(json.dumps({
            "session_id": payload.get("session_id"), "cwd": cwd,
            "started": f"{now:%Y-%m-%d %H:%M}", "started_iso": now.isoformat(timespec="seconds"),
            "repo": where["repo"]["id"], "repo_path": where["repo"]["path"],
            # 記下開場時的 HEAD：結束時只算這之後的 commit（用時間窗會把既有 commit 算進來）
            "head": next(iter(_git_lines(where["repo"]["path"], "rev-parse", "HEAD")), None),
            "group": ctx["scope"]["group"], "repos": ctx["scope"]["repos"],
        }, ensure_ascii=False), encoding="utf-8")
        return json.dumps({"hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": render_start(ctx)}}, ensure_ascii=False)
    except Exception:  # hook 壞掉不能擋住開 agent
        return ""


def _git_lines(path, *args):
    try:
        r = subprocess.run(["git", "-C", str(Path(path).expanduser()), *args],
                           capture_output=True, text=True, encoding="utf-8", timeout=10)
    except (OSError, subprocess.SubprocessError):
        return []
    return [ln for ln in r.stdout.splitlines() if ln.strip()] if r.returncode == 0 else []


def session_end(spine_dir, payload, now=None):
    """會話結束：沒交接但有事發生 → suggestion [hook]。回傳寫入的事件 dict 或 None。"""
    try:
        p = _marker_dir(spine_dir) / f"{_safe(payload.get('session_id'))}.json"
        if not p.exists():
            return None
        m = json.loads(p.read_text(encoding="utf-8"))
        p.unlink()
        scope = {"group": m.get("group"), "repos": m.get("repos"),
                 "label": m.get("group") or ",".join(m.get("repos") or [])}
        started = m["started"]
        traces, handed_off = 0, False
        for ev in _spine.iter_events(spine_dir):
            if f"{ev.date} {ev.time}" < started or ev.source in ("hook", "monitor"):
                continue
            if not _api._in(ev, scope):
                continue
            traces += 1
            if ev.type == "decision" and (ev.body or "").startswith("交接"):
                handed_off = True
        if handed_off:
            return None
        commits = (_git_lines(m["repo_path"], "log", f"{m['head']}..HEAD", "--format=%h %s")
                   if m.get("head") else
                   _git_lines(m["repo_path"], "log", f"--since={m['started_iso']}",
                              "--format=%h %s"))
        dirty = _git_lines(m["repo_path"], "status", "--porcelain")
        if not (commits or dirty or traces):
            return None  # 無事不報：什麼都沒發生的會話不留痕
        facts = [f"期間 {len(commits)} 個 commit"]
        if commits:
            facts[-1] += "（" + "；".join(c.split(" ", 1)[-1] for c in commits[:3]) \
                + ("…" if len(commits) > 3 else "") + "）"
        facts.append(f"{m['repo']} 目前未提交 {len(dirty)} 個檔")
        facts.append(f"範圍內留痕 {traces} 筆")
        where = {"repo": {"id": m["repo"]}, "groups": [m["group"]] if m.get("group") else []}
        toks = _api._tokens_for(scope, where)
        ev = _spine.append_event(
            spine_dir, "suggestion", "hook", toks,
            body=f"agent 會話結束但沒有交接（{started} 開始，{payload.get('reason') or '結束'}）\n"
                 + "；".join(facts) + "\n下次開場先補交接：做到哪、剩什麼、下次從哪接。",
            when=now)
        return _api.event_dict(ev)
    except Exception:
        return None
