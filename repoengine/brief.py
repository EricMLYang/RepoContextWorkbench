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


def _act(label, action, **payload):
    return {"label": label, "action": action, "payload": payload}


def build_question_cards(states, thresholds):
    """問句＝結構化卡（2026-09-02 UI 檢討 P0：出口是按鈕不是字）。
    每張卡：repo／qkind／title（問句）／actions（label＋引擎原語＋payload）。
    文字版（簡報 md、CLI）一律由 card_text 渲染而來，兩邊不會分岔。"""
    cards = []
    for s in states:
        rid = s["id"]
        if s["dirty_days"] is not None and s["dirty_days"] >= thresholds["dirty_stale_days"]:
            cards.append({
                "kind": "question", "qkind": "dirty", "repo": rid,
                "key": f"q:dirty:{rid}",
                "title": f"{rid} dirty 已 {s['dirty_days']:.0f} 天（{s['dirty']} 檔），本週要動它嗎？",
                "actions": [
                    _act("開 agent 收尾", "term_create", kind="agent", repo=rid,
                         repos=[rid], task=f"收尾 {rid} 的 {s['dirty']} 個未提交檔：先看 git status 與 diff，"
                                          f"判斷該 commit、丟棄或拆開，跟我確認後執行",
                         origin=f"q:dirty:{rid}"),
                    _act("標記本週不動", "defer", id=rid, qkind="dirty", days=7),
                ]})
        elif (s["tier"] == "active" and s["last_commit_days"] is not None
              and s["last_commit_days"] >= thresholds["inactive_days"]):
            cards.append({
                "kind": "question", "qkind": "inactive", "repo": rid,
                "key": f"q:inactive:{rid}",
                "title": f"{rid} 掛 active 但已 {s['last_commit_days']:.0f} 天無 commit，還算進行中嗎？",
                "actions": [
                    _act("降 tier", "tier", id=rid, tier="dormant"),
                    _act("本週排進度", "defer", id=rid, qkind="inactive", days=7),
                ]})
        if not s["exists"] or s["note"] == "非 git repo":
            cards.append({
                "kind": "question", "qkind": "broken", "repo": rid,
                "key": f"q:broken:{rid}",
                "title": f"{rid} 採集失敗（{s['note'] or '路徑不存在'}），registry 要修嗎？",
                "actions": [
                    _act("修 registry", "term_create", kind="shell", origin=f"q:broken:{rid}"),
                    _act("移出 registry", "remove_repo", id=rid),
                    _act("忽略並記錄", "defer", id=rid, qkind="broken", days=7),
                ]})
    return cards


def card_text(card):
    """卡 → 一行文字（問句 → 〔出口〕〔出口〕），簡報 md 與 CLI 用。"""
    outs = "".join(f"〔{a['label']}〕" for a in card["actions"])
    return f"{card['title']} → {outs}" if outs else card["title"]


def build_questions(states, thresholds):
    return [card_text(c) for c in build_question_cards(states, thresholds)]


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
