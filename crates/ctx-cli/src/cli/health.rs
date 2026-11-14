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
    // 2026-10-21：App 開會話用的 .mcp.json 指到的 ctx 還在不在（10-18 重裝後指到被刪的 bundle，兩天沒人發現）
    let mcp_json = ctx_core::session::mcp_json_status(&d);
    v["setup"] = serde_json::json!({"skill": skill, "mcp_json": mcp_json});
    // 2026-11-10：要人判斷的提醒（未結體檢、判斷回顧、關係建議、何時重看）搬來這裡——以前塞在每場 agent 會話的開場
    let here = ctx_core::agentapi::Target {
        cwd: std::env::current_dir().ok(),
        ..Default::default()
    };
    v["for_you"] = Value::Array(ctx_core::agentapi::human_hints(&d, &here).unwrap_or_default());
    // F-123／F-124：決策佇列與放行規則
    let now = chrono::Local::now().naive_local();
    v["decide"] = serde_json::to_value(ctx_core::decide::health_stats(&d, days, now)?)
        .map_err(ctx_core::Error::from)?;
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
        out.extend(decide_lines(&r["decide"]));
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
        let stale: Vec<String> = arr(p, "stale")
            .iter()
            .map(|x| format!("{}（{} 之後 {} 個 commit）", s(x, "repo"), s(x, "described_at"), s(x, "commits")))
            .collect();
        if !stale.is_empty() {
            line.push_str(&format!("；介紹可能過時：{}", stale.join("、")));
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
        // F-121 對講：agent 之間傳了幾則、有沒有人回、送不送得到（有用過才講）
        let m = &r["mailbox"];
        let reqs = m["requests"].as_u64().unwrap_or(0);
        if reqs + m["replies"].as_u64().unwrap_or(0) > 0 {
            let via: Vec<String> = m["via"]
                .as_object()
                .map(|o| o.iter().map(|(k, v)| format!("{k} {v}")).collect())
                .unwrap_or_default();
            out.push(format!(
                "- 對講（agent 互傳）：請求 {reqs}、有回覆 {}、回覆 {}、還沒送到 {}；送達路：{}",
                s(m, "answered"),
                s(m, "replies"),
                s(m, "undelivered"),
                if via.is_empty() { "—".to_string() } else { via.join("、") }
            ));
        }
        // F-84：程式化呼叫另計額度——先看得見
        let c = &r["agent_calls"];
        if c["calls"].as_u64().unwrap_or(0) > 0 || s(c, "policy") == "off" {
            out.push(format!(
                "- 程式化呼叫（claude -p：碰撞判定、ask_repo、助手、backfill；用訂閱登入時吃同一份額度）：期間 {} 次（今天 {}、平均 {:.1} 秒、失敗 {}）；config provider.programmatic＝{}",
                s(c, "calls"),
                s(c, "today"),
                c["avg_ms"].as_f64().unwrap_or(0.0) / 1000.0,
                s(c, "errors"),
                s(c, "policy")
            ));
        }
        // F-83：額度餘光（Claude＝App 會話的 statusLine 回報；Codex＝Codex 自己的會話紀錄）
        // 寫剩多少＋離重置多久（跟狀態列同一套說法），一眼判斷要多用還是省著用
        let win = |u: &Value, k: &str, label: &str| {
            u[k].as_object().map(|w| {
                if w.get("expired").and_then(Value::as_bool) == Some(true) {
                    return format!("{label} 已重置");
                }
                let left = 100u64.saturating_sub(w.get("used").and_then(Value::as_u64).unwrap_or(0));
                let when = match (
                    w.get("resets_in_min").and_then(Value::as_i64),
                    w.get("resets").and_then(Value::as_str),
                ) {
                    (Some(m), _) => format!("（{}）", after_span(m, "後重置")),
                    (None, Some(r)) => format!("（{r} 重置）"),
                    (None, None) => String::new(),
                };
                format!("{label} 剩 {left}%{when}")
            })
        };
        let usages = [
            ("Claude Code", ctx_core::usage::read(&d)),
            ("Codex", ctx_core::usage::codex()),
        ];
        for (name, u) in usages {
            let Some(u) = u else { continue };
            let parts: Vec<String> = [win(&u, "five_hour", "5h"), win(&u, "seven_day", "週")]
                .into_iter()
                .flatten()
                .collect();
            if !parts.is_empty() {
                out.push(format!(
                    "- {name} 額度：{}（{} 更新{}）",
                    parts.join("、"),
                    s(&u, "at"),
                    if u["warn"] == true { "；剩不到 20%" } else { "" }
                ));
            }
        }
        let tools = arr(r, "tools");
        if tools.is_empty() {
            out.push("- 工具用量：還沒有紀錄（MCP 呼叫才會記，`.state/metrics.jsonl`）".into());
        } else {
            out.push("- 工具用量：".into());
            for t in tools {
                // 失敗原因（錯誤碼→次數）：有才印
                let why = t["error_kinds"]
                    .as_object()
                    .map(|k| {
                        k.iter()
                            .map(|(code, n)| format!("{code} {n}"))
                            .collect::<Vec<_>>()
                            .join("、")
                    })
                    .filter(|w| !w.is_empty())
                    .map(|w| format!("（{w}）"))
                    .unwrap_or_default();
                out.push(format!(
                    "    {}：{} 次，平均 {}ms，失敗 {}{why}",
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
        if r["setup"]["mcp_json"]["status"] == "command_missing" {
            out.push(format!(
                "- App 開會話用的 .mcp.json 指到的 ctx 不在（{}）：那些會話的 MCP 會連不上、agent 退回 CLI；下一次 App 開會話會自動重寫，現在要修就跑 `ctx session --print`",
                s(&r["setup"]["mcp_json"], "command")
            ));
        }
        let for_you = arr(r, "for_you");
        if !for_you.is_empty() {
            out.push("- 要你判斷的（不再進 agent 的開場）：".into());
            for h in for_you {
                out.push(format!("    {}", s(h, "text")));
            }
        }
        out.join("\n")
    });
    Ok(())
}

/// 幾分鐘＋後綴 → 「45 分後重置」「2.5 hr 後重置」「3.2 天後重置」（10 以上不帶小數；跟前端 `spanText` 同一套）。
fn after_span(min: i64, word: &str) -> String {
    let one = |n: f64| {
        if n < 10.0 {
            format!("{}", (n * 10.0).round() / 10.0)
        } else {
            format!("{}", n.round())
        }
    };
    let m = min.max(0) as f64;
    if m < 60.0 {
        format!("{} 分{word}", m.round())
    } else if m < 24.0 * 60.0 {
        format!("{} hr {word}", one(m / 60.0))
    } else {
        format!("{} 天{word}", one(m / (24.0 * 60.0)))
    }
}

/// 「決策」段（F-123／F-124）：沒資料的比例與時間寫「沒資料」，不印 0。
fn decide_lines(d: &Value) -> Vec<String> {
    let n = |k: &str| d[k].as_u64().unwrap_or(0);
    let rate = match d["followed_rate"].as_f64() {
        Some(f) => format!(
            "照建議 {:.0}%（{}／{}）",
            f * 100.0,
            n("followed"),
            n("with_recommend")
        ),
        None => "照建議比例沒資料（還沒有帶建議的回答）".into(),
    };
    let median = match d["median_hours"].as_f64() {
        Some(h) if h < 24.0 => format!("{h:.1} 小時"),
        Some(h) => format!("{:.1} 天", h / 24.0),
        None => "沒資料".into(),
    };
    let mut out = vec![format!(
        "- 決策：現在要你決定 {} 件（ctx decide list --all）；人回答 {} 次，{rate}；自動放行 {} 次、被推翻 {}；從交接到回答的中位時間 {median}",
        n("waiting"),
        n("answered"),
        n("auto"),
        n("overturned")
    )];
    for p in d["policy_problems"].as_array().into_iter().flatten() {
        out.push(format!(
            "  policy.yaml 有問題（這條不生效）：{}",
            p.as_str().unwrap_or("")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::after_span;

    #[test]
    fn span_like_status_bar() {
        assert_eq!(after_span(45, "後重置"), "45 分後重置");
        assert_eq!(after_span(150, "後重置"), "2.5 hr 後重置");
        assert_eq!(after_span(12 * 60 + 20, "後重置"), "12 hr 後重置");
        assert_eq!(after_span(3 * 24 * 60, "後重置"), "3 天後重置");
        assert_eq!(after_span(4608, "後重置"), "3.2 天後重置");
        assert_eq!(after_span(-5, "後重置"), "0 分後重置");
    }
}
