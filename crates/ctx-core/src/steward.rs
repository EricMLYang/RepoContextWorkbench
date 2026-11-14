//! 總管（F-125，2026-11-14，Rust 才有）：站在組上面的那一場 agent 會話（工作目錄＝spine）的簡報與作業規則。
//!
//! 為什麼要有：二十幾個 repo、五個組，沒有任何東西站在組的上面。總管的工作是讓使用者更快把要決定的事看懂、答完，
//! 並看住全局。第一版權限刻意很小：只讀全部 repo、在卡上留建議（`ctx decide suggest`），不代答、不升級規則、
//! 不改 repo、不替別的 repo 寫交接——總管答錯會被底下的 agent 放大，代答的唯一途徑是使用者同意過的放行規則。
//!
//! 簡報（[`brief`]）≤ [`BRIEF_MAX_CHARS`] 字元，給人在終端看（`ctx steward`），也是 spine 目錄開會話時的開場
//! （`hooks::session_start`）。資料全部沿用既有的：接力 plan（只跑一次，各 repo 的 git 狀態在這裡）、決策佇列、
//! 自動放行、升級建議、policy 的問題、registry 變更、hook 記的「沒交接就結束」。

use std::path::{Path, PathBuf};

use chrono::{Duration, NaiveDateTime};
use serde::{Deserialize, Serialize};

use crate::decide::{self, TrustHint};
use crate::registry_log::{self, MissingPath, RegistryChanges};
use crate::relay::{self, RelayScope, RelayStatus};
use crate::{policy, spine, Result};

/// 簡報上限（字元）。
pub const BRIEF_MAX_CHARS: usize = 2000;
/// 第一次（沒有看過的紀錄）registry 變更看幾天。
pub const FIRST_DAYS: i64 = 7;
/// 要你決定列前幾張。
pub const TOP_CARDS: usize = 5;
/// 自動放行、沒交接就結束看幾天。
pub const RECENT_DAYS: i64 = 7;
/// 總管的作業規則（開場時接在簡報後面）。
pub const RULES_MD: &str = include_str!("assets/STEWARD.md");

const STATE_FILE: &str = "steward.json";

/// `.state/steward.json`：上次總管會話開場的時間。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct State {
    seen: Option<String>,
}

fn state_path(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join(STATE_FILE)
}

/// 上次總管會話開場的時間（`YYYY-MM-DD HH:MM`；沒有＝None）。
pub fn last_seen(spine_dir: &Path) -> Option<String> {
    std::fs::read_to_string(state_path(spine_dir))
        .ok()
        .and_then(|s| serde_json::from_str::<State>(&s).ok())
        .and_then(|s| s.seen)
}

/// 記下「總管看過了」（只有總管會話開場時呼叫；人在終端看簡報不記）。
pub fn mark_seen(spine_dir: &Path, now: NaiveDateTime) -> Result<()> {
    crate::locking::update_json_locked(spine_dir, &state_path(spine_dir), |s: &mut State| {
        s.seen = Some(now.format("%Y-%m-%d %H:%M").to_string());
    })
}

/// 簡報裡的一張卡。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CardLine {
    pub repo: String,
    pub question: String,
    /// 類別的人話；沒帶提問＝None（舊交接）
    pub kind: Option<String>,
    pub days: i64,
    pub hours: i64,
    /// 總管已經在這張卡上留過建議
    pub steward: bool,
}

/// 各組一行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GroupLine {
    /// 沒有組＝None
    pub group: Option<String>,
    pub repos: usize,
    pub waiting: usize,
    pub running: usize,
    pub ready: usize,
    /// 組裡最新一筆交接
    pub last_handoff: Option<String>,
    /// 最新一筆交接到現在幾天
    pub idle_days: Option<i64>,
}

/// 近期沒交接就結束的會話。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoHandoffEnd {
    pub repo: String,
    pub at: String,
}

/// 總管簡報。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Brief {
    pub at: String,
    /// 給人看的文字（≤ [`BRIEF_MAX_CHARS`] 字元）
    pub text: String,
    pub chars: usize,
    /// 為了上限從後面砍掉幾項
    pub cut: usize,
    pub waiting: usize,
    pub cards: Vec<CardLine>,
    pub auto_recent: usize,
    pub suggestions: Vec<TrustHint>,
    pub policy_problems: Vec<String>,
    /// registry 變更從什麼時候算（上次總管開場；第一次＝近 7 天）
    pub registry_since: String,
    pub registry: RegistryChanges,
    pub groups: Vec<GroupLine>,
    pub missing_paths: Vec<MissingPath>,
    pub no_handoff_ends: Vec<NoHandoffEnd>,
}

fn stamp(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%d %H:%M").to_string()
}

fn age(days: i64, hours: i64) -> String {
    if days > 0 {
        format!("{days} 天")
    } else if hours > 0 {
        format!("{hours} 小時")
    } else {
        "剛交接".into()
    }
}

/// 一段：標題（含下一步的指令）＋項目；砍的時候從最後一段的最後一項砍起。
struct Section {
    head: String,
    cmd: String,
    items: Vec<String>,
}

/// 組簡報（純讀取）。`now` 給測試用。
pub fn brief(spine_dir: &Path, now: NaiveDateTime) -> Result<Brief> {
    let plan = relay::plan_at(spine_dir, &RelayScope::All, now)?;
    let queue = decide::queue_of(spine_dir, &plan, now)?;
    let recent = stamp(now - Duration::days(RECENT_DAYS));
    let auto_recent = decide::auto_list(spine_dir, Some(&recent))?.items.len();
    let pol = policy::load(spine_dir);
    let registry_since =
        last_seen(spine_dir).unwrap_or_else(|| stamp(now - Duration::days(FIRST_DAYS)));
    let events = spine::iter_events(spine_dir)?;
    let registry = registry_log::changes_of(spine_dir, &events, &registry_since, now)?;

    let cards: Vec<CardLine> = queue
        .cards
        .iter()
        .map(|c| CardLine {
            repo: c.repo.clone(),
            question: c.question.clone(),
            kind: c.kind.map(|k| k.label().to_string()),
            days: c.days,
            hours: c.hours,
            steward: c.steward.is_some(),
        })
        .collect();

    // 各組：沿用 plan 的列（全部範圍時 group＝registry 裡第一個有它的組）
    let mut groups: Vec<GroupLine> = Vec::new();
    for r in &plan.rows {
        let g = match groups.iter_mut().find(|g| g.group == r.group) {
            Some(g) => g,
            None => {
                groups.push(GroupLine {
                    group: r.group.clone(),
                    repos: 0,
                    waiting: 0,
                    running: 0,
                    ready: 0,
                    last_handoff: None,
                    idle_days: None,
                });
                groups.last_mut().expect("剛放進去")
            }
        };
        g.repos += 1;
        g.waiting += usize::from(r.is_decision_card(false));
        g.running += usize::from(r.status == RelayStatus::Busy);
        g.ready += usize::from(r.status == RelayStatus::Ready);
        if r.handoff_at > g.last_handoff {
            g.last_handoff = r.handoff_at.clone();
        }
    }
    for g in &mut groups {
        g.idle_days = g
            .last_handoff
            .as_deref()
            .and_then(|a| NaiveDateTime::parse_from_str(a, "%Y-%m-%d %H:%M").ok())
            .map(|a| (now - a).num_days());
    }
    groups.sort_by(|a, b| (a.group.is_none(), &a.group).cmp(&(b.group.is_none(), &b.group)));

    let mut no_handoff_ends: Vec<NoHandoffEnd> = events
        .iter()
        .filter(|e| relay::is_no_handoff(e) && e.stamp() >= recent)
        .map(|e| NoHandoffEnd {
            repo: e.kv("repo").unwrap_or("（沒有 repo）").to_string(),
            at: e.stamp(),
        })
        .collect();
    no_handoff_ends.reverse();

    let mut b = Brief {
        at: stamp(now),
        text: String::new(),
        chars: 0,
        cut: 0,
        waiting: queue.cards.len(),
        cards,
        auto_recent,
        suggestions: queue.suggestions.clone(),
        policy_problems: pol.problems,
        registry_since,
        missing_paths: registry.missing_paths.clone(),
        registry,
        groups,
        no_handoff_ends,
    };
    let (text, cut) = render_text(&b);
    b.chars = text.chars().count();
    b.text = text;
    b.cut = cut;
    Ok(b)
}

fn sections(b: &Brief) -> Vec<Section> {
    let mut out = Vec::new();
    // 1. 要你決定
    out.push(Section {
        head: if b.waiting == 0 {
            "## 要你決定：沒有".into()
        } else {
            format!("## 要你決定 {} 件", b.waiting)
        },
        cmd: "ctx decide list --all".into(),
        items: b
            .cards
            .iter()
            .take(TOP_CARDS)
            .map(|c| {
                format!(
                    "- {}｜{}｜{}｜{}{}",
                    c.repo,
                    crate::pystr::clip(&c.question, 40),
                    c.kind.as_deref().unwrap_or("舊交接"),
                    age(c.days, c.hours),
                    if c.steward {
                        "｜已留總管建議"
                    } else {
                        ""
                    }
                )
            })
            .collect(),
    });
    // 2. 自動放行
    if b.auto_recent > 0 {
        out.push(Section {
            head: format!("## 自動放行：近 {RECENT_DAYS} 天 {} 筆", b.auto_recent),
            cmd: "ctx decide list --auto".into(),
            items: Vec::new(),
        });
    }
    // 3. 可以升級的類型、policy 的問題
    if !b.suggestions.is_empty() || !b.policy_problems.is_empty() {
        let mut items: Vec<String> = b
            .suggestions
            .iter()
            .map(|h| {
                format!(
                    "- {}／{} 連續 {} 次照建議（使用者同意才下：{}）",
                    h.repo,
                    h.kind.label(),
                    h.streak,
                    h.command
                )
            })
            .collect();
        items.extend(
            b.policy_problems
                .iter()
                .map(|p| format!("- policy.yaml：{}", crate::pystr::clip(p, 80))),
        );
        out.push(Section {
            head: "## 放行規則".into(),
            cmd: "ctx decide rules".into(),
            items,
        });
    }
    // 4. registry 變更
    let r = &b.registry;
    if !r.is_empty() {
        let line = |label: &str, xs: &[registry_log::RegChange]| -> Option<String> {
            (!xs.is_empty()).then(|| {
                let names: Vec<String> = xs
                    .iter()
                    .map(|c| format!("{} {}", c.area.label(), c.name))
                    .collect();
                format!("- {label}：{}", crate::pystr::clip(&names.join("、"), 120))
            })
        };
        out.push(Section {
            head: format!("## registry 變更（自 {}）", b.registry_since),
            cmd: format!("ctx registry changes --since \"{}\"", b.registry_since),
            items: [
                line("新增", &r.added),
                line("移除", &r.removed),
                line("變更", &r.changed),
            ]
            .into_iter()
            .flatten()
            .collect(),
        });
    }
    // 5. 各組
    out.push(Section {
        head: "## 各組".into(),
        cmd: "ctx relay plan --group <組>".into(),
        items: b
            .groups
            .iter()
            .map(|g| {
                let idle = match g.idle_days {
                    Some(0) => "今天有交接".to_string(),
                    Some(d) => format!("最後交接 {d} 天前"),
                    None => "沒有交接".to_string(),
                };
                format!(
                    "- {}（{}）：等你 {}｜在跑 {}｜可接 {}｜{idle}",
                    g.group.as_deref().unwrap_or("沒有組"),
                    g.repos,
                    g.waiting,
                    g.running,
                    g.ready
                )
            })
            .collect(),
    });
    // 6. 異常（無事不報）
    let mut items = Vec::new();
    if !b.missing_paths.is_empty() {
        let names: Vec<&str> = b.missing_paths.iter().map(|m| m.repo.as_str()).collect();
        items.push(format!(
            "- 路徑不存在 {} 個：{}（放回原處或 ctx registry set <id> path <新路徑>）",
            names.len(),
            crate::pystr::clip(&names.join("、"), 100)
        ));
    }
    if !b.no_handoff_ends.is_empty() {
        let mut repos: Vec<&str> = Vec::new();
        for e in &b.no_handoff_ends {
            if !repos.contains(&e.repo.as_str()) {
                repos.push(&e.repo);
            }
        }
        items.push(format!(
            "- 近 {RECENT_DAYS} 天有 {} 場會話沒交接就結束：{}",
            b.no_handoff_ends.len(),
            crate::pystr::clip(&repos.join("、"), 100)
        ));
    }
    if !items.is_empty() {
        out.push(Section {
            head: "## 異常".into(),
            cmd: "ctx relay plan --all".into(),
            items,
        });
    }
    out
}

/// 文字＋砍掉幾項。超過上限時從最後一段的最後一項砍起（一段的項目砍光就連標題一起拿掉），
/// 最後留一行「還有 N 項：<指令>」。
pub fn render_text(b: &Brief) -> (String, usize) {
    let mut secs = sections(b);
    let title = format!("# 總管簡報（{}）", b.at);
    let draw = |secs: &[Section], cut: usize, cmds: &[String]| -> String {
        let mut lines = vec![title.clone()];
        for s in secs {
            lines.push(format!("{} → {}", s.head, s.cmd));
            lines.extend(s.items.iter().cloned());
        }
        if cut > 0 {
            lines.push(format!("還有 {cut} 項：{}", cmds.join("；")));
        }
        lines.join("\n")
    };
    let (mut cut, mut cmds): (usize, Vec<String>) = (0, Vec::new());
    loop {
        let text = draw(&secs, cut, &cmds);
        if text.chars().count() <= BRIEF_MAX_CHARS || secs.is_empty() {
            return (text, cut);
        }
        let last = secs.last_mut().expect("上面檢查過不是空的");
        if !cmds.contains(&last.cmd) {
            cmds.insert(0, last.cmd.clone());
        }
        if last.items.pop().is_none() {
            secs.pop();
        }
        cut += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages() {
        assert_eq!(age(2, 50), "2 天");
        assert_eq!(age(0, 5), "5 小時");
        assert_eq!(age(0, 0), "剛交接");
    }
}
