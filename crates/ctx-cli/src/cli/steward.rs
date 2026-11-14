//! `ctx steward`（總管簡報）與 `ctx registry changes`（F-125，Rust 才有）。判斷都在 `ctx_core::steward`／`registry_log`；
//! 這裡只轉參數、排版。人在終端看簡報不會記「總管看過了」（只有總管會話開場才記）。

use ctx_core::registry_log::{RegChange, RegistryChanges};

use super::agent::agent_err;
use super::common::{CmdResult, Ctx};

pub fn cmd_steward(ctx: &Ctx) -> CmdResult {
    let d = ctx.spine_dir()?;
    let b = ctx_core::steward::brief(&d, chrono::Local::now().naive_local()).map_err(agent_err)?;
    ctx.emit(&b, |b| b.text.clone());
    Ok(())
}

pub fn cmd_registry_changes(ctx: &Ctx, since: Option<&str>) -> CmdResult {
    let d = ctx.spine_dir()?;
    let now = chrono::Local::now().naive_local();
    let since = match since {
        Some(s) => ctx_core::relay::parse_since(s).map_err(agent_err)?,
        None => (now - chrono::Duration::days(7))
            .format("%Y-%m-%d %H:%M")
            .to_string(),
    };
    let r = ctx_core::registry_log::changes(&d, &since, now).map_err(agent_err)?;
    ctx.emit(&r, render_changes);
    Ok(())
}

fn section(out: &mut Vec<String>, title: &str, xs: &[RegChange]) {
    if xs.is_empty() {
        return;
    }
    out.push(String::new());
    out.push(format!("## {title}（{}）", xs.len()));
    for c in xs {
        let times = if c.events > 1 {
            format!("（{} 筆合成）", c.events)
        } else {
            String::new()
        };
        out.push(format!("- {} {}　{}{times}", c.area.label(), c.name, c.at));
        for d in &c.details {
            out.push(format!("  {d}"));
        }
    }
}

fn render_changes(r: &RegistryChanges) -> String {
    let mut out = vec![format!("# registry 變更（{} 之後，到 {}）", r.since, r.at)];
    if r.is_empty() {
        out.push("這段時間 registry 沒有變更。".into());
    }
    section(&mut out, "新增", &r.added);
    section(&mut out, "移除", &r.removed);
    section(&mut out, "變更", &r.changed);
    if r.unrecognized > 0 {
        out.push(format!(
            "（另有 {} 筆管理類留痕讀不懂，用 ctx query 看原文）",
            r.unrecognized
        ));
    }
    if !r.missing_paths.is_empty() {
        out.push(String::new());
        out.push(format!(
            "## 現在的異常：路徑不存在（{}）",
            r.missing_paths.len()
        ));
        for m in &r.missing_paths {
            out.push(format!("- {}：{}", m.repo, m.path));
        }
        out.push(
            "放回原處，或 ctx registry set <id> path <新路徑>；不用了就 ctx registry remove <id>"
                .into(),
        );
    }
    out.join("\n")
}
