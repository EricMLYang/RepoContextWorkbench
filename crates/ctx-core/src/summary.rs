//! 本組工作摘要（2026-09-12 UX 檢討 §3）。對應 Python `repo_context/summary.py`。
//!
//! 首屏原本只回答「有幾個 repo、幾件待處理」，卡片只回答「哪些規則被觸發」；
//! 使用者仍要自己還原「上次做到哪裡、哪些變化影響原本的工作、現在值得接著做什麼」。
//! 摘要把四件事壓成一屏：目標／上次進度／自上次紀錄以來的變化／可接續的一步。
//!
//! 紀律（這輪最重要的部分，不是版面）：
//! - 每一格都帶 `source`（依據）與 `at`（時間）；讀得到依據才敢照著做。
//! - 沒有依據就明講沒有（`gaps`），不用推測補位。
//! - commit 數只當「活動」陳述，不當進度也不當價值——目標由使用者一句話提供，
//!   上次進度只認脊椎裡的 decision／outcome 留痕（`spine::ADMIN_TAG` 開頭的改設定不算）。
//! - 範圍判定走 `spine::in_scope`，跟情境卡、簡報同一份規則（§6.2）。

use std::path::Path;

use chrono::{Local, NaiveDateTime};
use serde::Serialize;

use crate::brief::{self, QuestionCard};
use crate::collect::{self, RepoState};
use crate::spine::{self, Event, EventType};
use crate::{config, registry, Error, Result};

/// 一格事實：文字＋依據＋時間（對應 `summary._fact` 的 dict）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fact {
    pub text: String,
    pub source: String,
    pub at: Option<String>,
}

fn fact(text: impl Into<String>, source: impl Into<String>, at: Option<String>) -> Fact {
    Fact {
        text: text.into(),
        source: source.into(),
        at,
    }
}

/// 對應 `build_summary` 回傳的 dict（序列化鍵名、順序同 Python）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub group: String,
    pub saved_group: Option<String>,
    pub repos: Vec<String>,
    pub at: String,
    // 每格都配一句「沒有依據時該說的話」——前端直接顯示，不自己編話
    pub goal: Option<Fact>,
    pub goal_gap: Option<String>,
    pub progress: Option<Fact>,
    pub progress_gap: Option<String>,
    pub since: Option<String>,
    pub changes: Vec<Fact>,
    pub changes_window: String,
    pub next: Option<Fact>,
    pub next_gap: Option<String>,
    pub changes_gap: String,
    pub gaps: Vec<String>,
    pub resume_task: String,
}

fn first_line(ev: &Event) -> String {
    if ev.body.is_empty() {
        ev.tokens.join(" ")
    } else {
        ev.first_line().to_string()
    }
}

/// 目標只有使用者能給（一句話），機器不替他發明。
fn goal(
    spine_dir: &Path,
    group: Option<&str>,
    gname: &str,
) -> Result<(Option<Fact>, Option<String>)> {
    let Some(group) = group else {
        let gap = if gname == "全部" {
            "「全部」是跨組總覽，沒有單一目標；選一個牌組才看得到目標。"
        } else {
            "臨時範圍沒有登記目標；存成牌組後才能記下這組在做什麼。"
        };
        return Ok((None, Some(gap.into())));
    };
    let g = match registry::get_group(spine_dir, group) {
        Ok(g) => g,
        Err(Error::Invalid(_)) => {
            return Ok((None, Some(format!("registry 找不到「{group}」這組。"))))
        }
        Err(e) => return Err(e),
    };
    let text = g.get_str("goal").unwrap_or_default().trim().to_string();
    if text.is_empty() {
        return Ok((
            None,
            Some("尚未登記這組的目標——寫一句話，摘要才說得出這組在做什麼。".into()),
        ));
    }
    Ok((
        Some(fact(
            text,
            format!("registry.yaml · groups.{group}.goal"),
            g.get_str("goal_at"),
        )),
        None,
    ))
}

fn scoped_in(ev: &Event, scoped: bool, gkey: &str, ids: &[String]) -> bool {
    !scoped || spine::in_scope(ev, Some(gkey), ids)
}

/// 上次進度＝脊椎最後一筆本組 decision／outcome。沒有就說沒有，不拿 commit 充數。
fn progress(
    events: &[Event],
    gkey: &str,
    ids: &[String],
    scoped: bool,
) -> (Option<Fact>, Option<String>) {
    let mut best: Option<&Event> = None;
    for ev in events {
        if !matches!(ev.etype, EventType::Decision | EventType::Outcome)
            || ev.body.starts_with(spine::ADMIN_TAG)
        {
            continue; // 改設定（建組、改目標、改 tier…）不是工作進度
        }
        if !scoped_in(ev, scoped, gkey, ids) {
            continue;
        }
        if best.is_none_or(|b| (&ev.date, &ev.time) >= (&b.date, &b.time)) {
            best = Some(ev);
        }
    }
    match best {
        None => (
            None,
            Some(
                "脊椎沒有本組的進度紀錄（decision／outcome）；會話結束時用「記錄結果」留一筆。"
                    .into(),
            ),
        ),
        Some(b) => (
            Some(fact(
                first_line(b),
                format!("脊椎 {} [{}]", b.etype, b.source),
                Some(b.stamp()),
            )),
            None,
        ),
    }
}

/// 自上次紀錄以來的事實。commit 只陳述為活動，不解讀成進度或價值。
fn changes(
    states: &[RepoState],
    loops: &[Event],
    gkey: &str,
    ids: &[String],
    scoped: bool,
    since: Option<&str>,
    limit: usize,
) -> Vec<Fact> {
    let mut out = Vec::new();
    let since_date = crate::pystr::take_chars(since.unwrap_or(""), 10);
    for c in collect::recent_across(states, 30) {
        if !since_date.is_empty() && c.date.as_str() < since_date {
            continue;
        }
        out.push(fact(
            format!("{}「{}」", c.repo, c.subject),
            format!("git log · {} {}", c.repo, c.hash),
            Some(c.date.clone()),
        ));
        if out.len() >= limit {
            break;
        }
    }
    for s in states {
        if out.len() >= limit + 2 {
            break;
        }
        if s.dirty > 0 {
            let age = s
                .dirty_days
                .map(|d| format!("，最舊 {d:.0} 天"))
                .unwrap_or_default();
            out.push(fact(
                format!("{} 有 {} 個未提交檔{age}", s.id, s.dirty),
                format!("git status · {}", s.id),
                None,
            ));
        } else if !s.exists {
            out.push(fact(
                format!("{} 路徑不存在，本次採集不到", s.id),
                format!("registry.yaml · {}.path", s.id),
                None,
            ));
        }
    }
    for ev in loops {
        if out.len() >= limit + 4 {
            break;
        }
        if !scoped_in(ev, scoped, gkey, ids) {
            continue;
        }
        if since.is_some_and(|s| ev.stamp().as_str() <= s) {
            continue;
        }
        let num = ev.num().unwrap_or("#?");
        out.push(fact(
            format!("新的未結 {num}「{}」", first_line(ev)),
            format!("脊椎 open-loop [{}]", ev.source),
            Some(ev.stamp()),
        ));
    }
    out
}

const HANDOFF_NEXT: &str = "下次從這裡接：";

/// 本組最後一筆交接 decision 的「下次從這裡接」；沒有就 None。
fn handoff_next(events: &[Event], gkey: &str, ids: &[String], scoped: bool) -> Option<Fact> {
    let best = events.iter().rev().find(|ev| {
        ev.etype == EventType::Decision
            && ev.body.starts_with("交接")
            && scoped_in(ev, scoped, gkey, ids)
    })?;
    let line = crate::pystr::splitlines(&best.body)
        .into_iter()
        .find(|ln| ln.starts_with(HANDOFF_NEXT))
        .unwrap_or("");
    let text = line.get(HANDOFF_NEXT.len()..).unwrap_or("").trim();
    if text.is_empty() {
        return None;
    }
    Some(fact(
        text,
        format!("脊椎 交接 decision [{}]", best.source),
        Some(best.stamp()),
    ))
}

/// 可接續的一步＝有依據的一件事：先看到期未結，再看觸發閾值的問句。
#[allow(clippy::too_many_arguments)]
fn next_step(
    events: &[Event],
    loops: &[Event],
    gkey: &str,
    ids: &[String],
    scoped: bool,
    cards: &[QuestionCard],
    today: &str,
) -> (Option<Fact>, Option<String>) {
    let mut due_best: Option<(&str, &Event)> = None;
    for ev in loops {
        if !scoped_in(ev, scoped, gkey, ids) {
            continue;
        }
        let Some(due) = ev.kv("due").filter(|d| !d.is_empty()) else {
            continue;
        };
        if due_best.is_none_or(|(d, _)| due < d) {
            due_best = Some((due, ev));
        }
    }
    let overdue = due_best.is_some_and(|(d, _)| d < today);
    if !overdue {
        // 上次交接留下的下一步（agentapi.handoff 寫的 decision）排在逾期之後、一般到期之前
        if let Some(ho) = handoff_next(events, gkey, ids, scoped) {
            return (Some(ho), None);
        }
    }
    if let Some((due, ev)) = due_best {
        let num = ev.num().unwrap_or("#?");
        let when = if due < today {
            "已逾期".to_string()
        } else if due == today {
            "今天到期".to_string()
        } else {
            format!("{due} 到期")
        };
        return (
            Some(fact(
                format!("接續 {num}「{}」（{when}）", first_line(ev)),
                format!("脊椎 open-loop due:{due}"),
                Some(ev.stamp()),
            )),
            None,
        );
    }
    if let Some(c) = cards.first() {
        return (
            Some(fact(
                c.title.clone(),
                format!("採集閾值 · {} · {}", c.qkind.name(), c.repo),
                None,
            )),
            None,
        );
    }
    (
        None,
        Some("目前沒有到期未結，也沒有觸發提醒條件的 repo；下一步要做什麼由你決定。".into()),
    )
}

/// 「接續上次工作」帶進會話的任務——只複述有依據的內容，不替使用者宣稱進度。
fn resume_task(
    gname: &str,
    goal: Option<&Fact>,
    progress: Option<&Fact>,
    nxt: Option<&Fact>,
) -> String {
    let mut bits = vec![format!("接續「{gname}」上次的工作。")];
    if let Some(g) = goal {
        bits.push(format!("這組的目標（使用者登記）：{}。", g.text));
    }
    match progress {
        Some(p) => bits.push(format!(
            "脊椎最後一筆紀錄（{}，{}）：{}。",
            p.at.as_deref().unwrap_or("None"),
            p.source,
            p.text
        )),
        None => bits.push(
            "脊椎沒有本組的進度紀錄，請先從情境卡的最近事件推斷現況，並說明你的依據。".into(),
        ),
    }
    if let Some(n) = nxt {
        bits.push(format!(
            "工作台建議的下一步（依據：{}）：{}。",
            n.source, n.text
        ));
    }
    bits.push(
        "請先確認這件事現在的實際狀態與依據，再提出接下來一步；不確定或需要動到組外的東西就先問我。"
            .into(),
    );
    bits.concat()
}

/// 對應 `summary.build_summary`：回傳工作摘要。`states`／`cards` 可由呼叫端（build_scan）餵進來避免重採集。
/// `repos` 給定（非空）＝臨時組合；group、repos 都沒給＝全部。
pub fn build_summary(
    spine_dir: impl AsRef<Path>,
    group: Option<&str>,
    repos: Option<&[String]>,
    states: Option<Vec<RepoState>>,
    cards: Option<Vec<QuestionCard>>,
    when: Option<NaiveDateTime>,
) -> Result<Summary> {
    let spine_dir = spine_dir.as_ref();
    let group = group.filter(|g| !g.is_empty());
    let repos = repos.filter(|r| !r.is_empty());
    let now = when.unwrap_or_else(|| Local::now().naive_local());
    let today = now.format("%Y-%m-%d").to_string();
    let (gname, entries) = registry::resolve_group(spine_dir, group, repos)?;
    let ids: Vec<String> = entries.iter().map(|e| e.id()).collect();
    let scoped = group.is_some() || repos.is_some();
    let gkey = group.unwrap_or(&gname).to_string();
    let states = match states {
        Some(s) => s,
        None => collect::collect_group(&brief::repo_specs(&entries), Some(spine_dir)),
    };
    let cards = match cards {
        Some(c) => c,
        None => brief::build_question_cards(&states, &config::load(spine_dir)?.thresholds()),
    };
    let events = spine::iter_events(spine_dir)?;
    let loops = spine::open_loops_of(&events);
    let (goal, goal_gap) = goal(spine_dir, group, &gname)?;
    let (progress, progress_gap) = progress(&events, &gkey, &ids, scoped);
    let since = progress.as_ref().and_then(|p| p.at.clone());
    let changes = changes(&states, &loops, &gkey, &ids, scoped, since.as_deref(), 4);
    let (nxt, next_gap) = next_step(&events, &loops, &gkey, &ids, scoped, &cards, &today);
    let gaps: Vec<String> = [&goal_gap, &progress_gap, &next_gap]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    let resume = resume_task(&gname, goal.as_ref(), progress.as_ref(), nxt.as_ref());
    Ok(Summary {
        group: gname,
        saved_group: group.map(str::to_string),
        repos: ids,
        at: now.format("%Y-%m-%d %H:%M").to_string(),
        goal,
        goal_gap,
        progress,
        progress_gap,
        changes_window: match &since {
            Some(s) => format!("自上次紀錄（{s}）以來"),
            None => "近期（沒有上次紀錄可比，改列最近活動）".into(),
        },
        since,
        changes,
        next: nxt,
        next_gap,
        changes_gap: "本次採集沒有看到新的 commit、未提交檔或新未結。".into(),
        gaps,
        resume_task: resume,
    })
}

/// 對應 `summary.format_summary`：摘要 → 純文字（CLI 用；跟工作台同一份資料，兩邊不會分岔）。
pub fn format_summary(s: &Summary) -> String {
    fn src_line(item: &Fact) -> String {
        let mut src = format!("  依據：{}", item.source);
        if let Some(at) = item.at.as_deref().filter(|a| !a.is_empty()) {
            src.push_str(&format!(" · {at}"));
        }
        src
    }
    fn line(label: &str, item: Option<&Fact>, gap: Option<&str>) -> Vec<String> {
        match item {
            None => vec![
                format!("## {label}"),
                format!("（{}）", gap.unwrap_or("None")),
            ],
            Some(it) => vec![format!("## {label}"), it.text.clone(), src_line(it)],
        }
    }
    let mut out = vec![
        format!("# 本組工作摘要 {}（{}）", s.group, s.at),
        String::new(),
    ];
    out.extend(line("目前目標", s.goal.as_ref(), s.goal_gap.as_deref()));
    out.push(String::new());
    out.extend(line(
        "上次進度",
        s.progress.as_ref(),
        s.progress_gap.as_deref(),
    ));
    out.push(String::new());
    out.push(format!("## 變化（{}）", s.changes_window));
    if s.changes.is_empty() {
        out.push(format!("（{}）", s.changes_gap));
    } else {
        for c in &s.changes {
            out.push(format!("- {}", c.text));
            out.push(src_line(c));
        }
    }
    out.push(String::new());
    out.extend(line("可接續的一步", s.next.as_ref(), s.next_gap.as_deref()));
    out.push(String::new());
    out.join("\n")
}
