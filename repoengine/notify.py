"""P13 通知——內容產生＋分類；殼只負責顯示（v2：通知以顯示為本分，第一出口＝tray/監控台）。

分類規則（§3.4 interrupt 白名單）：
- system-unsure：引擎各處失敗事件的統一標記（P15 agent 失敗、P7 判定失敗、P12 任務失敗
  的 body 都以/含 'system-unsure'）→ 即時打斷
- irreversible-confirm / costly-anomaly：原型尚無產生者，規則先立在這裡等事件來
其餘一律未讀累積，儀式時刻集中呈現（打斷要掙得）。
"""
from . import spine


def classify(events):
    """未讀事件 → (interrupt, normal)。"""
    interrupt = [e for e in events if "system-unsure" in (e.body or "")]
    normal = [e for e in events if "system-unsure" not in (e.body or "")]
    return interrupt, normal


def pending(spine_dir):
    """殼 poll 的單一來源：{interrupt, normal, count}。"""
    interrupt, normal = classify(spine.get_unread(spine_dir))
    return {"interrupt": interrupt, "normal": normal,
            "count": len(interrupt) + len(normal)}


def render(p):
    """CLI/殼可直接印的通知文字。"""
    lines = []
    for ev in p["interrupt"]:
        first = (ev.body or "").splitlines()[0]
        lines.append(f"⚠ [打斷] {ev.date} {ev.time} {ev.type}：{first}")
    if p["normal"]:
        lines.append(f"未讀 {len(p['normal'])} 則（儀式時刻集中看；`unread` 列全量）")
    if not lines:
        lines.append("一切正常，無事不報。")
    return "\n".join(lines)
