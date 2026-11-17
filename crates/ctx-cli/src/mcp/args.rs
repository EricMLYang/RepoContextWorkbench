//! 管理層共用的小工具。

use super::*;

pub(super) fn ev_lines(events: &[spine::Event], verbose: bool) -> String {
    let mut out = Vec::new();
    for ev in events {
        out.push(format!("{} {}", ev.date, ev.header()));
        if verbose && !ev.body.is_empty() {
            out.push(format!("   {}", ev.body.replace('\n', "\n   ")));
        }
    }
    if out.is_empty() {
        "（無）".into()
    } else {
        out.join("\n")
    }
}
