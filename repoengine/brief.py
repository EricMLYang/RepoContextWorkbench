"""早晨簡報產生器（v2 §4.5 樣貌 A，純事實版、無 LLM）。

三段固定結構：需要你判斷的（≤3 條問句＋出口）／未結 open loops／沉默摘要一行。
超過 3 條問句＝過濾閾值錯了（照樣印，但標警語——這是免費校準資料）。
產出同時寫 groups/<g>/briefs/ 並落 presented 事件（教練規格：出手即留痕）。
"""
import datetime as _dt
from pathlib import Path

from . import collect as _collect
from . import config as _config
from . import registry, spine


def build_questions(states, thresholds):
    qs = []
    for s in states:
        if s["dirty_days"] is not None and s["dirty_days"] >= thresholds["dirty_stale_days"]:
            qs.append(f"{s['id']} dirty 已 {s['dirty_days']:.0f} 天（{s['dirty']} 檔），本週要動它嗎？"
                      f" → 〔開 agent 收尾〕〔標記本週不動〕")
        elif (s["tier"] == "active" and s["last_commit_days"] is not None
              and s["last_commit_days"] >= thresholds["inactive_days"]):
            qs.append(f"{s['id']} 掛 active 但已 {s['last_commit_days']:.0f} 天無 commit，"
                      f"還算進行中嗎？ → 〔降 tier〕〔本週排進度〕")
        if not s["exists"] or s["note"] == "非 git repo":
            qs.append(f"{s['id']} 採集失敗（{s['note'] or '路徑不存在'}），registry 要修嗎？ → 〔修 registry〕〔忽略並記錄〕")
    return qs


def build_brief(spine_dir, group_name, states, when=None):
    cfg = _config.load(spine_dir)
    th = cfg["thresholds"]
    now = when or _dt.datetime.now()
    qs = build_questions(states, th)
    loops = spine.open_loops(spine_dir)
    today = f"{now:%Y-%m-%d}"
    loop_lines = []
    for ev in loops:
        num = next((t for t in ev.tokens if t.startswith("#")), "#?")
        due = ev.kv("due")
        flag = ""
        if due and due <= today:
            flag = "今天到期" if due == today else f"已逾期（{due}）"
        elif ev.date == today:
            flag = "今日新增"
        if flag:
            first = ev.body.splitlines()[0] if ev.body else ""
            loop_lines.append(f"- {num}「{first}」{flag} → 〔開碰撞〕〔延期〕〔關閉〕")
    quiet = len(states) - len({q.split(" ")[0] for q in qs})
    lines = [f"# 早晨簡報 {today}（{group_name} 組）", ""]
    lines.append("## 需要你判斷的（每條一個問句）")
    if qs:
        lines += [f"{i}. {q}" for i, q in enumerate(qs, 1)]
        if len(qs) > 3:
            lines.append("")
            lines.append(f"> ⚠ 問句 {len(qs)} 條 > 3——過濾閾值該修了（config.yaml thresholds）")
    else:
        lines.append("（今天沒有需要判斷的事）")
    lines += ["", "## 未結（open loops，只列到期與新增）"]
    lines += loop_lines if loop_lines else ["（無到期或新增）"]
    lines += ["", "## 沉默摘要（一行）",
              f"其餘 {max(quiet, 0)} 個 repo 無異常、無變化。", ""]
    return "\n".join(lines)


def run(spine_dir, group=None, repos=None, when=None):
    now = when or _dt.datetime.now()
    gname, entries = registry.resolve_group(spine_dir, group, repos)
    states = _collect.collect_group(entries)
    text = build_brief(spine_dir, gname, states, now)
    out = Path(spine_dir) / "groups" / (group or "adhoc") / "briefs" / f"{now:%Y-%m-%d}.md"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(text, encoding="utf-8")
    n_q = text.count("\n1. ") and len([l for l in text.splitlines() if l[:2].rstrip(".").isdigit()])
    spine.append_event(
        spine_dir, "presented", "morning-brief",
        (["group:" + group] if group else []),
        body=f"呈現了：判斷 {n_q} 條、open loops {len(spine.open_loops(spine_dir))} 條 → {out.name}",
        when=now)
    return out, text
