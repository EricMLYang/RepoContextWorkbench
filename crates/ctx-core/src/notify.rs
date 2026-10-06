//! P13 通知——內容產生＋分類；殼只負責顯示（v2：通知以顯示為本分，第一出口＝tray/監控台）。
//! 對應 Python `repo_context/notify.py`。
//!
//! 分類規則（§3.4 interrupt 白名單）：
//! - system-unsure：引擎各處失敗事件的統一標記（P15 agent 失敗、P7 判定失敗、P12 任務失敗
//!   的 body 都以/含 'system-unsure'）→ 即時打斷
//! - irreversible-confirm / costly-anomaly：原型尚無產生者，規則先立在這裡等事件來
//!
//! 其餘一律未讀累積，儀式時刻集中呈現（打斷要掙得）。

use std::path::Path;

use serde::Serialize;

use crate::spine::{self, Event};
use crate::Result;

/// 對應 `notify.classify`：未讀事件 → (interrupt, normal)。
pub fn classify(events: Vec<Event>) -> (Vec<Event>, Vec<Event>) {
    events
        .into_iter()
        .partition(|e| e.body.contains("system-unsure"))
}

/// `pending` 的結果（殼 poll 的單一來源）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pending {
    pub interrupt: Vec<Event>,
    pub normal: Vec<Event>,
    pub count: usize,
}

/// 對應 `notify.pending`：殼 poll 的單一來源 {interrupt, normal, count}。
pub fn pending(spine_dir: impl AsRef<Path>) -> Result<Pending> {
    let (interrupt, normal) = classify(spine::get_unread(spine_dir)?);
    let count = interrupt.len() + normal.len();
    Ok(Pending {
        interrupt,
        normal,
        count,
    })
}

/// 對應 `notify.render`：CLI／殼可直接印的通知文字。
pub fn render(p: &Pending) -> String {
    let mut lines: Vec<String> = p
        .interrupt
        .iter()
        .map(|ev| {
            format!(
                "⚠ [打斷] {} {} {}：{}",
                ev.date,
                ev.time,
                ev.etype,
                ev.first_line()
            )
        })
        .collect();
    if !p.normal.is_empty() {
        lines.push(format!(
            "未讀 {} 則（儀式時刻集中看；`unread` 列全量）",
            p.normal.len()
        ));
    }
    if lines.is_empty() {
        lines.push("一切正常，無事不報。".into());
    }
    lines.join("\n")
}
