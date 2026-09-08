"""組情境卡（2026-09-08 組為單位輪）——預設 agent 以「一組 repo」為工作單位的開場料。

P16 會話料原本只有文件地圖（路徑＋行數）；agent 不知道這組是誰、誰管誰、最近在動什麼、
有什麼未結，開場就只能問「要做什麼」。情境卡把 registry（成員、tier、關係）＋ P3 脈動
＋ 脊椎（本組 loops／最近事件）壓成一屏，外加**組管家角色說明**（可由 config `session.role_prompt` 覆寫）。
純函數：build_context(spine, group|repos) → str；CLI `group context`、MCP `group_context` 同源。
"""
import datetime as _dt

from . import collect as _collect
from . import config as _config
from . import registry, spine

ROLE_PROMPT = """你是這組 repo 的**組管家**（group steward）——工作單位是「這一組」，不是單一 repo。
- 先讀完本情境卡再動手；需要細節再用文件地圖挑檔讀，不要整包吞。
- 彙整、比對、追蹤都以組為範圍：講結論時標明來自哪個 repo 哪份檔；跨 repo 的彙整產物落 spine 的 groups/<組>/materials/。
- 尊重關係：PM repo 管方向、code repo 管實作；不越組改別組的東西，要動組外 repo 先問。
- 出手留痕：做了判斷寫 `spine_append`（type=decision）、發現待辦寫 open-loop（帶 group:）、有靈感用 `collide_submit`；
  隨時可用 `group_context` 重抓本卡、`collect` 看最新 git 狀態、`open_loops` 看未結。
- 無事不報：沒有異常就一句話帶過，不要把正常狀態講成待辦。
"""


def _member_line(entry, state, rels_of):
    rid = entry["id"]
    bits = [entry.get("tier", "active")]
    if entry.get("tags"):
        bits.append("/".join(entry["tags"]))
    if entry.get("type") == "external":
        bits.append("external" + (f" ↑{entry['upstream']}" if entry.get("upstream") else ""))
    rel_txt = "；".join(
        (f"{r['label']}→ {r['peer']}" if r["direction"] == "out" else f"{r['label']} {r['peer']}")
        for r in rels_of)
    act = ""
    if state and state.get("exists") and state.get("type", "mine") == "mine":
        act = f"近 7 天 {state.get('commits_7d') or 0} commits"
        if state.get("dirty"):
            act += f"，未提交 {state['dirty']} 檔"
        rc = state.get("recent_commits") or []
        if rc:
            act += f"｜最新 {rc[0]['date']}「{rc[0]['subject']}」"
        elif state.get("last_commit_days") is not None:
            act += f"｜末次 commit {state['last_commit_days']:.0f} 天前"
    elif state and not state.get("exists"):
        act = "路徑不存在"
    line = f"- **{rid}**（{'｜'.join(bits)}）"
    if rel_txt:
        line += f"：{rel_txt}"
    if act:
        line += f"\n  {act}"
    return line


def _in_scope(ev, scoped, gkey, ids):
    if not scoped:
        return True
    return ev.kv("group") == gkey or ev.kv("repo") in ids


def build_context(spine_dir, group=None, repos=None, states=None, when=None):
    now = when or _dt.datetime.now()
    cfg = _config.load(spine_dir)
    gname, entries = registry.resolve_group(spine_dir, group, repos)
    ids = [e["id"] for e in entries]
    scoped = bool(group or repos)
    gkey = group or gname
    if states is None:
        states = _collect.collect_group(entries, spine_dir)
    smap = {s["id"]: s for s in states}
    out = [f"# 組情境卡 {gname}（{now:%Y-%m-%d %H:%M}）", ""]
    out.append("## 成員與角色")
    for e in entries:
        out.append(_member_line(e, smap.get(e["id"]),
                                [r for r in registry.relations_of(spine_dir, e["id"])
                                 if r["peer"] in ids]))
    outside = [ln for ln in registry.relation_lines(spine_dir, ids) if "組外" in ln]
    if outside:
        out.append("- 指向組外的關係：" + "；".join(outside))
    out += ["", "## 脈動", _collect.pulse_line(_collect.group_pulse(states))]
    loops = [ev for ev in spine.open_loops(spine_dir) if _in_scope(ev, scoped, gkey, ids)]
    out += ["", "## 未結（open loops，本組）"]
    if loops:
        for ev in loops:
            num = next((t for t in ev.tokens if t.startswith("#")), "#?")
            due = ev.kv("due")
            first = (ev.body or "").splitlines()[0] if ev.body else ""
            out.append(f"- {num} {first}" + (f"（due {due}）" if due else ""))
    else:
        out.append("（無）")
    out += ["", "## 最近事件（本組，最多 8 則）"]
    evs = [ev for ev in spine.iter_events(spine_dir)
           if ev.type != "presented" and _in_scope(ev, scoped, gkey, ids)]
    if evs:
        for ev in evs[-8:][::-1]:
            first = (ev.body or "").splitlines()[0] if ev.body else " ".join(ev.tokens)
            out.append(f"- {ev.date} {ev.time} {ev.type} [{ev.source}] {first}")
    else:
        out.append("（無）")
    role = (cfg.get("session") or {}).get("role_prompt") or ROLE_PROMPT
    out += ["", "## 你的角色", role.rstrip(), ""]
    return "\n".join(out)
