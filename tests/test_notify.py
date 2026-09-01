"""P13 通知：interrupt 白名單分類（system-unsure 打斷、其餘未讀累積）＋render 文字。"""
from repoengine import notify
from repoengine import spine as spine_mod


def test_classify_interrupt_vs_normal(spine):
    spine_mod.append_event(spine, "suggestion", "timer", [],
                           body="system-unsure：排程任務掛了")
    spine_mod.append_event(spine, "decision", "manual", [], body="一般決策")
    p = notify.pending(spine)
    assert len(p["interrupt"]) == 1 and len(p["normal"]) == 1 and p["count"] == 2
    text = notify.render(p)
    assert "⚠" in text and "[打斷]" in text and "未讀 1 則" in text


def test_all_clear_after_ack(spine):
    spine_mod.append_event(spine, "decision", "manual", [], body="x")
    spine_mod.ack_unread(spine)
    p = notify.pending(spine)
    assert p["count"] == 0
    assert "一切正常" in notify.render(p)  # 無事不報：空狀態一行字
