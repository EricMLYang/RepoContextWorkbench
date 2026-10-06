//! P13 通知：interrupt 白名單分類（system-unsure 打斷、其餘未讀累積）＋render 文字。對應 tests/test_notify.py。

use ctx_core::notify;
use ctx_core::spine::{self, NO_TOKENS};
use ctx_core::testkit::spine;

#[test]
fn test_classify_interrupt_vs_normal() {
    let (_t, d) = spine();
    spine::append_event(
        &d,
        "suggestion",
        "timer",
        NO_TOKENS,
        "system-unsure：排程任務掛了",
        None,
    )
    .unwrap();
    spine::append_event(&d, "decision", "manual", NO_TOKENS, "一般決策", None).unwrap();
    let p = notify::pending(&d).unwrap();
    assert!(p.interrupt.len() == 1 && p.normal.len() == 1 && p.count == 2);
    let text = notify::render(&p);
    assert!(text.contains('⚠') && text.contains("[打斷]") && text.contains("未讀 1 則"));
}

#[test]
fn test_all_clear_after_ack() {
    let (_t, d) = spine();
    spine::append_event(&d, "decision", "manual", NO_TOKENS, "x", None).unwrap();
    spine::ack_unread(&d, None).unwrap();
    let p = notify::pending(&d).unwrap();
    assert_eq!(p.count, 0);
    assert!(notify::render(&p).contains("一切正常")); // 無事不報：空狀態一行字
}
