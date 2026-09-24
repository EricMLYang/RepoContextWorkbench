"""本組工作摘要（2026-09-12 UX 檢討 §3）。

首屏原本只回答「有幾個 repo、幾件待處理」，卡片只回答「哪些規則被觸發」；
使用者仍要自己還原「上次做到哪裡、哪些變化影響原本的工作、現在值得接著做什麼」。
摘要把四件事壓成一屏：目標／上次進度／自上次紀錄以來的變化／可接續的一步。

紀律（這輪最重要的部分，不是版面）：
- 每一格都帶 `source`（依據）與 `at`（時間）；讀得到依據才敢照著做。
- 沒有依據就明講沒有（`gaps`），不用推測補位。
- commit 數只當「活動」陳述，不當進度也不當價值——目標由使用者一句話提供，
  上次進度只認脊椎裡的 decision／outcome 留痕。
- 範圍判定走 `spine.in_scope`，跟情境卡、簡報同一份規則（§6.2）。
"""
import datetime as _dt

from . import brief as _brief
from . import collect as _collect
from . import config as _config
from . import registry, spine

_PROGRESS_TYPES = ("decision", "outcome")


def _fact(text, source, at=None):
    return {"text": text, "source": source, "at": at}


def _stamp(ev):
    return f"{ev.date} {ev.time}"


def _first_line(ev):
    return (ev.body or "").splitlines()[0] if ev.body else " ".join(ev.tokens)


def _goal(spine_dir, group, gname):
    """目標只有使用者能給（一句話），機器不替他發明。"""
    if not group:
        return None, ("「全部」是跨組總覽，沒有單一目標；選一個牌組才看得到目標。"
                      if gname == "全部" else
                      "臨時範圍沒有登記目標；存成牌組後才能記下這組在做什麼。")
    try:
        g = registry.get_group(spine_dir, group)
    except ValueError:
        return None, f"registry 找不到「{group}」這組。"
    text = (g.get("goal") or "").strip()
    if not text:
        return None, "尚未登記這組的目標——寫一句話，摘要才說得出這組在做什麼。"
    return _fact(text, f"registry.yaml · groups.{group}.goal",
                 g.get("goal_at")), None


def _progress(spine_dir, gkey, ids, scoped):
    """上次進度＝脊椎最後一筆本組 decision／outcome。沒有就說沒有，不拿 commit 充數。"""
    best = None
    for ev in spine.iter_events(spine_dir):
        if ev.type not in _PROGRESS_TYPES:
            continue
        if scoped and not spine.in_scope(ev, gkey, ids):
            continue
        if best is None or (ev.date, ev.time) >= (best.date, best.time):
            best = ev
    if best is None:
        return None, "脊椎沒有本組的進度紀錄（decision／outcome）；會話結束時用「記錄結果」留一筆。"
    return _fact(_first_line(best),
                 f"脊椎 {best.type} [{best.source}]", _stamp(best)), None


def _changes(states, spine_dir, gkey, ids, scoped, since, limit=4):
    """自上次紀錄以來的事實。commit 只陳述為活動，不解讀成進度或價值。"""
    out = []
    since_date = (since or "")[:10]
    for c in _collect.recent_across(states, limit=30):
        if since_date and c["date"] < since_date:
            continue
        out.append(_fact(f"{c['repo']}「{c['subject']}」",
                         f"git log · {c['repo']} {c['hash']}", c["date"]))
        if len(out) >= limit:
            break
    for s in states:
        if len(out) >= limit + 2:
            break
        if s.get("dirty"):
            days = s.get("dirty_days")
            age = f"，最舊 {days:.0f} 天" if days is not None else ""
            out.append(_fact(f"{s['id']} 有 {s['dirty']} 個未提交檔{age}",
                             f"git status · {s['id']}"))
        elif not s.get("exists"):
            out.append(_fact(f"{s['id']} 路徑不存在，本次採集不到",
                             f"registry.yaml · {s['id']}.path"))
    for ev in spine.open_loops(spine_dir):
        if len(out) >= limit + 4:
            break
        if scoped and not spine.in_scope(ev, gkey, ids):
            continue
        if since and _stamp(ev) <= since:
            continue
        num = next((t for t in ev.tokens if t.startswith("#")), "#?")
        out.append(_fact(f"新的未結 {num}「{_first_line(ev)}」",
                         f"脊椎 open-loop [{ev.source}]", _stamp(ev)))
    return out


_HANDOFF_NEXT = "下次從這裡接："


def _handoff_next(spine_dir, gkey, ids, scoped):
    """本組最後一筆交接 decision 的「下次從這裡接」；沒有就 None。"""
    best = None
    for ev in spine.iter_events(spine_dir):
        if ev.type != "decision" or not (ev.body or "").startswith("交接"):
            continue
        if scoped and not spine.in_scope(ev, gkey, ids):
            continue
        best = ev
    if best is None:
        return None
    line = next((ln for ln in best.body.splitlines() if ln.startswith(_HANDOFF_NEXT)), "")
    text = line[len(_HANDOFF_NEXT):].strip()
    if not text:
        return None
    return _fact(text, f"脊椎 交接 decision [{best.source}]", _stamp(best))


def _next_step(spine_dir, gkey, ids, scoped, cards, today):
    """可接續的一步＝有依據的一件事：先看到期未結，再看觸發閾值的問句。"""
    due_best = None
    for ev in spine.open_loops(spine_dir):
        if scoped and not spine.in_scope(ev, gkey, ids):
            continue
        due = ev.kv("due")
        if not due:
            continue
        if due_best is None or due < due_best[0]:
            due_best = (due, ev)
    overdue = due_best and due_best[0] < today
    if not overdue:
        # 上次交接留下的下一步（agentapi.handoff 寫的 decision）排在逾期之後、一般到期之前
        ho = _handoff_next(spine_dir, gkey, ids, scoped)
        if ho:
            return ho, None
    if due_best:
        due, ev = due_best
        num = next((t for t in ev.tokens if t.startswith("#")), "#?")
        when = "已逾期" if due < today else ("今天到期" if due == today else f"{due} 到期")
        return _fact(f"接續 {num}「{_first_line(ev)}」（{when}）",
                     f"脊椎 open-loop due:{due}", _stamp(ev)), None
    if cards:
        c = cards[0]
        return _fact(c["title"],
                     f"採集閾值 · {_brief.qkind_name(c['qkind'])} · {c['repo']}"), None
    return None, "目前沒有到期未結，也沒有觸發提醒條件的 repo；下一步要做什麼由你決定。"


def _resume_task(gname, goal, progress, nxt):
    """「接續上次工作」帶進會話的任務——只複述有依據的內容，不替使用者宣稱進度。"""
    bits = [f"接續「{gname}」上次的工作。"]
    if goal:
        bits.append(f"這組的目標（使用者登記）：{goal['text']}。")
    if progress:
        bits.append(f"脊椎最後一筆紀錄（{progress['at']}，{progress['source']}）："
                    f"{progress['text']}。")
    else:
        bits.append("脊椎沒有本組的進度紀錄，請先從情境卡的最近事件推斷現況，"
                    "並說明你的依據。")
    if nxt:
        bits.append(f"工作台建議的下一步（依據：{nxt['source']}）：{nxt['text']}。")
    bits.append("請先確認這件事現在的實際狀態與依據，再提出接下來一步；"
                "不確定或需要動到組外的東西就先問我。")
    return "".join(bits)


def build_summary(spine_dir, group=None, repos=None, states=None, cards=None,
                  when=None):
    """回傳工作摘要 dict。states／cards 可由呼叫端（build_scan）餵進來避免重採集。"""
    now = when or _dt.datetime.now()
    today = f"{now:%Y-%m-%d}"
    gname, entries = registry.resolve_group(spine_dir, group, repos)
    ids = [e["id"] for e in entries]
    scoped = bool(group or repos)
    gkey = group or gname
    if states is None:
        states = _collect.collect_group(entries, spine_dir)
    if cards is None:
        cards = _brief.build_question_cards(
            states, _config.load(spine_dir)["thresholds"])
    goal, goal_gap = _goal(spine_dir, group, gname)
    progress, progress_gap = _progress(spine_dir, gkey, ids, scoped)
    since = progress["at"] if progress else None
    changes = _changes(states, spine_dir, gkey, ids, scoped, since)
    nxt, next_gap = _next_step(spine_dir, gkey, ids, scoped, cards, today)
    gaps = [g for g in (goal_gap, progress_gap, next_gap) if g]
    return {
        "group": gname,
        "saved_group": group,
        "repos": ids,
        "at": f"{now:%Y-%m-%d %H:%M}",
        # 每格都配一句「沒有依據時該說的話」——前端直接顯示，不自己編話
        "goal": goal, "goal_gap": goal_gap,
        "progress": progress, "progress_gap": progress_gap,
        "since": since,
        "changes": changes,
        "changes_window": ("自上次紀錄（" + since + "）以來" if since else "近期（沒有上次紀錄可比，改列最近活動）"),
        "next": nxt, "next_gap": next_gap,
        "changes_gap": "本次採集沒有看到新的 commit、未提交檔或新未結。",
        "gaps": gaps,
        "resume_task": _resume_task(gname, goal, progress, nxt),
    }


def format_summary(s):
    """摘要 → 純文字（CLI 用；跟工作台同一份資料，兩邊不會分岔）。"""
    def line(label, item, gap):
        if not item:
            return [f"## {label}", f"（{gap}）"]
        src = f"  依據：{item['source']}" + (f" · {item['at']}" if item.get("at") else "")
        return [f"## {label}", item["text"], src]

    out = [f"# 本組工作摘要 {s['group']}（{s['at']}）", ""]
    out += line("目前目標", s["goal"], s["goal_gap"]) + [""]
    out += line("上次進度", s["progress"], s["progress_gap"]) + [""]
    out.append(f"## 變化（{s['changes_window']}）")
    if s["changes"]:
        for c in s["changes"]:
            out.append(f"- {c['text']}")
            out.append(f"  依據：{c['source']}" + (f" · {c['at']}" if c.get("at") else ""))
    else:
        out.append(f"（{s['changes_gap']}）")
    out += [""] + line("可接續的一步", s["next"], s["next_gap"]) + [""]
    return "\n".join(out)
