//! `ctx health`：健康指標（F-36／N-07／F-50）＋skill 版本（F-74）＋程式化呼叫（F-84）＋額度餘光（F-83）。
//! 2026-10-16 從 `agent.rs` 拆出來（那檔超過 1500 行）；行為不變。

use serde_json::Value;

use super::agent::{arr, s};
use super::common::{CmdResult, Ctx};

/// 健康指標（Rust 才有，F-36／N-07）：真用一陣子後，用數字看行為有沒有改變。
pub fn cmd_health(ctx: &Ctx, days: i64) -> CmdResult {
    let d = ctx.spine_dir()?;
    let mut v = ctx_core::health::health(&d, days, None)?;
    // F-74：agent 讀到的 skill 是不是這版的（讀 ~/.claude，所以不放在引擎的 health 裡）
    let skill = ctx_core::agentsetup::skill_status(&ctx_core::agentsetup::claude_home());
    v["setup"] = serde_json::json!({"skill": skill});
    ctx.emit(&v, |r| {
        let pct = |x: &Value| {
            x.as_f64()
                .map(|f| format!("{:.0}%", f * 100.0))
                .unwrap_or_else(|| "—".into())
        };
        let mut out = vec![format!(
            "# 健康指標（近 {} 天，自 {}）",
            s(r, "days"),
            s(r, "since")
        )];
        let h = &r["handoff"];
        out.push(format!(
            "- 交接率 {}：交接 {}、沒交接就結束 {}",
            pct(&h["rate"]),
            s(h, "handoffs"),
            s(h, "missed")
        ));
        let f = &r["fresh"];
        out.push(format!(
            "- 推薦命中率 {}：推了 {}、用上 {}、略過 {}",
            pct(&f["hit_rate"]),
            s(f, "found"),
            s(f, "used"),
            s(f, "dismissed")
        ));
        let dd = &r["decisions"];
        out.push(format!(
            "- 判斷結果記錄率 {}：有結果 {}、待回顧 {}",
            pct(&dd["outcome_rate"]),
            s(dd, "with_outcome"),
            s(dd, "awaiting_retro")
        ));
        let l = &r["loops"];
        out.push(format!(
            "- 未結：期間開 {}、關 {}；現在 {} 項，逾期 {}",
            s(l, "opened"),
            s(l, "closed"),
            s(l, "open_now"),
            s(l, "overdue")
        ));
        let p = &r["profiles"];
        let undescribed: Vec<&str> = arr(p, "undescribed")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let mut line = format!(
            "- 名片：{} 個 repo（不含封存），寫了介紹 {}、有角色 {}",
            s(p, "repos"),
            s(p, "described"),
            s(p, "with_role")
        );
        if !undescribed.is_empty() {
            let more = p["undescribed_more"].as_u64().unwrap_or(0);
            line.push_str(&format!(
                "；還沒寫介紹：{}{}（`ctx registry describe <id> --description …`，或在 App 右側欄寫）",
                undescribed.join("、"),
                if more > 0 { format!("，另 {more} 個") } else { String::new() }
            ));
        }
        out.push(line);
        let a = &r["assist"];
        if a["runs"].as_u64().unwrap_or(0) + a["undone"].as_u64().unwrap_or(0) > 0 {
            out.push(format!(
                "- 助手：用了 {} 次（agent {} 次），有做事 {}、中途失敗 {}、復原 {}",
                s(a, "runs"),
                s(a, "from_agent"),
                s(a, "did_something"),
                s(a, "failed"),
                s(a, "undone")
            ));
        }
        // F-84：程式化呼叫另計額度——先看得見
        let c = &r["agent_calls"];
        if c["calls"].as_u64().unwrap_or(0) > 0 || s(c, "policy") == "off" {
            out.push(format!(
                "- 程式化呼叫（claude -p：碰撞判定、ask_repo、助手、backfill；另計額度）：期間 {} 次（今天 {}、平均 {:.1} 秒、失敗 {}）；config provider.programmatic＝{}",
                s(c, "calls"),
                s(c, "today"),
                c["avg_ms"].as_f64().unwrap_or(0.0) / 1000.0,
                s(c, "errors"),
                s(c, "policy")
            ));
        }
        // F-83：額度餘光（App 會話的 statusLine 回報）
        if let Some(u) = ctx_core::usage::read(&d) {
            let win = |k: &str, label: &str| {
                u[k].as_object().map(|w| {
                    format!(
                        "{label} {}%{}",
                        w["used"].as_u64().unwrap_or(0),
                        w["resets"]
                            .as_str()
                            .map(|r| format!("（{r} 重置）"))
                            .unwrap_or_default()
                    )
                })
            };
            let parts: Vec<String> = [win("five_hour", "5h"), win("seven_day", "7d")]
                .into_iter()
                .flatten()
                .collect();
            if !parts.is_empty() {
                out.push(format!(
                    "- Claude Code 額度：{}（{} 更新{}）",
                    parts.join("、"),
                    s(&u, "at"),
                    if u["warn"] == true { "；已過 80%" } else { "" }
                ));
            }
        }
        let tools = arr(r, "tools");
        if tools.is_empty() {
            out.push("- 工具用量：還沒有紀錄（MCP 呼叫才會記，`.state/metrics.jsonl`）".into());
        } else {
            out.push("- 工具用量：".into());
            for t in tools {
                out.push(format!(
                    "    {}：{} 次，平均 {}ms，失敗 {}",
                    s(t, "tool"),
                    s(t, "calls"),
                    s(t, "avg_ms"),
                    s(t, "errors")
                ));
            }
        }
        // 無事不報：skill 是最新的就不講
        match r["setup"]["skill"].as_str() {
            Some("stale") => out.push(
                "- skill 不是這版的（重裝 App 後還沒重跑）：agent 讀到舊說明——跑 `ctx setup --claude` 更新".into(),
            ),
            Some("missing") => out.push(
                "- 沒裝 repo-context skill：agent 不知道何時該用工具——跑 `ctx setup --claude`".into(),
            ),
            _ => {}
        }
        out.join("\n")
    });
    Ok(())
}
