//! 健康指標（2026-10-10，ideation F-36／N-07；Rust 才有，Python 沒有對應模組）。
//!
//! 功能一個個加上去之後，要能用數字回答「真用幾天下來有沒有用」——量的是**行為有沒有改變**，
//! 不是功能有沒有跑起來：
//! - 交接率：會話結束時有沒有交接（交接 vs hook 記的「沒交接」）
//! - 推薦命中率：反芻推的新料有沒有被讀（引用）或拿來試
//! - 判斷結果記錄率：滿回顧天數的判斷，有幾成記了結果（outcome）
//! - 未結：期間開了幾項、關了幾項、現在逾期幾項
//! - 工具用量：MCP 每次呼叫記在 `.state/metrics.jsonl`（[`record_call`]），算次數、平均毫秒、失敗數
//!
//! 全部只讀檔，不改任何東西。

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{Duration, Local, NaiveDateTime};
use serde_json::{json, Value};

use crate::agentapi::{self, event_ref};
use crate::ruminate::{self, LinkStatus};
use crate::spine::{self, EventType};
use crate::Result;

fn metrics_path(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("metrics.jsonl")
}

/// MCP 每次工具呼叫記一行（`{"at","tool","ms","ok"}`）。寫不進去就算了——紀錄不能擋住工具。
pub fn record_call(spine_dir: &Path, tool: &str, ms: u128, ok: bool) {
    let p = metrics_path(spine_dir);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let line = json!({
        "at": Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        "tool": tool, "ms": ms as u64, "ok": ok,
    });
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        let _ = writeln!(f, "{line}");
    }
}

fn ratio(n: usize, d: usize) -> Value {
    if d == 0 {
        Value::Null
    } else {
        json!(((n as f64 / d as f64) * 100.0).round() / 100.0)
    }
}

/// 近 `days` 天的健康指標。`now`＝None 用現在（測試可指定）。
pub fn health(spine_dir: &Path, days: i64, now: Option<NaiveDateTime>) -> Result<Value> {
    let now = now.unwrap_or_else(|| Local::now().naive_local());
    let since = (now - Duration::days(days))
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let today = now.format("%Y-%m-%d").to_string();
    let events = spine::iter_events(spine_dir)?;
    let recent: Vec<&spine::Event> = events.iter().filter(|e| e.stamp() >= since).collect();

    // 交接率
    let handoffs = recent
        .iter()
        .filter(|e| e.etype == EventType::Decision && e.body.starts_with("交接"))
        .count();
    let missed = recent
        .iter()
        .filter(|e| e.source == "hook" && e.body.contains("沒有交接"))
        .count();

    // 推薦命中率（期間找到的推薦）
    let links: Vec<ruminate::Link> = ruminate::load_state(spine_dir)
        .links
        .into_iter()
        .filter(|l| l.found >= since)
        .collect();
    let cites = crate::crossref::load_cites(spine_dir);
    let used = links
        .iter()
        .filter(|l| {
            l.status == LinkStatus::Used
                || cites
                    .iter()
                    .any(|c| format!("{}:{}", c.provider, c.rel) == l.reference && c.at >= l.found)
        })
        .count();
    let dismissed = links
        .iter()
        .filter(|l| l.status == LinkStatus::Dismissed)
        .count();

    // 判斷結果記錄率：滿回顧天數、人／agent／回填寫的判斷（同 retro_candidates 的口徑）
    let due = agentapi::retro_candidates(spine_dir, &events, None, now).len();
    let outcomes: Vec<&str> = events
        .iter()
        .filter(|e| e.etype == EventType::Outcome)
        .filter_map(|e| e.kv("ref"))
        .collect();
    let with_outcome = events
        .iter()
        .filter(|e| e.etype == EventType::Decision && outcomes.contains(&event_ref(e).as_str()))
        .count();

    // 未結
    let opened = recent
        .iter()
        .filter(|e| e.etype == EventType::OpenLoop && !e.body.starts_with("closed"))
        .count();
    let closed = recent
        .iter()
        .filter(|e| e.etype == EventType::OpenLoop && e.body.starts_with("closed"))
        .count();
    let open_now = spine::open_loops_of(&events);
    let overdue = open_now
        .iter()
        .filter(|e| {
            e.kv("due")
                .is_some_and(|d| !d.is_empty() && d < today.as_str())
        })
        .count();

    // 工具用量
    let mut tools: BTreeMap<String, (usize, u64, usize)> = BTreeMap::new();
    if let Ok(text) = std::fs::read_to_string(metrics_path(spine_dir)) {
        for v in text
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        {
            if v["at"].as_str().is_none_or(|a| a < since.as_str()) {
                continue;
            }
            let e = tools
                .entry(v["tool"].as_str().unwrap_or("?").to_string())
                .or_default();
            e.0 += 1;
            e.1 += v["ms"].as_u64().unwrap_or(0);
            if v["ok"] != true {
                e.2 += 1;
            }
        }
    }
    let tool_rows: Vec<Value> = tools
        .iter()
        .map(|(t, (n, ms, err))| json!({"tool": t, "calls": n, "avg_ms": ms / (*n as u64).max(1), "errors": err}))
        .collect();

    Ok(json!({
        "ok": true, "days": days, "since": since,
        "handoff": {"handoffs": handoffs, "missed": missed, "rate": ratio(handoffs, handoffs + missed)},
        "fresh": {"found": links.len(), "used": used, "dismissed": dismissed, "hit_rate": ratio(used, links.len())},
        "decisions": {"with_outcome": with_outcome, "awaiting_retro": due,
                      "outcome_rate": ratio(with_outcome, with_outcome + due)},
        "loops": {"opened": opened, "closed": closed, "open_now": open_now.len(), "overdue": overdue},
        "tools": tool_rows,
    }))
}
