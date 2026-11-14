//! 接力（F-122，2026-11-14，Rust 才有）：一次看出哪些 repo 能直接從上次交接的下一步接著做（`plan`），
//! 事後一次看出各 repo 做了什麼（`report`）。
//!
//! 為什麼要有：repo 一多（二十幾個），每個 repo 的 agent 收尾都留了交接，人卻要一個一個開會話說「接著做」，
//! 結束後再一個一個看做了什麼。接力把這兩件事各壓成一張表。
//!
//! 第 1 輪只有純讀取：讀 spine 事件、registry、`.state/session_status`，對 repo 只跑唯讀的 git
//! （`--no-optional-locks`）；不寫事件、不開會話。用到的都是既有的判斷：
//! - 交接的欄位：[`agentapi::parse_handoff`]（跟 `context_for` 的「上次交接」同一份解析）
//! - 誰在跑：[`mailbox::peers`]（App 會話的狀態檔）
//! - 會話結束沒交接：hook 寫的 suggestion（`hooks` 的 SessionEnd，第一行「agent 會話結束但沒有交接」）
//! - 沒 commit 的檔：[`collect::collect_group`]；之後的 commit：[`gitlog::since`]
//! - 參考類、暫停／封存的 repo 不列（registry 的 class／tier）
//!
//! CLI：`ctx relay plan`／`ctx relay report`。

use std::path::Path;
use std::sync::LazyLock;

use chrono::{Local, NaiveDateTime};
use regex::Regex;
use serde::Serialize;

use crate::agentapi::{self, mailbox, Handoff};
use crate::ask::Ask;
use crate::collect::{self, RepoSpec, RepoState};
use crate::registry::{self, Repo, Tier};
use crate::spine::{self, Event, EventType};
use crate::status::AgentStatus;
use crate::{gitlog, Error, RejectCode, Result};

/// 交接超過幾天就不直接接（可能過時，要先看現況）。
pub const STALE_DAYS: i64 = 14;
/// report 每個 repo 列前幾個 commit 標題。
pub const REPORT_COMMITS: usize = 3;

/// plan 列的備註：下一步在等你（決策卡不重印這句，卡片本身就是在等你）。
pub const NOTE_WAITS: &str = "下一步在等你";

/// hook 在會話結束、期間沒交接時寫的 suggestion 第一行開頭（見 `hooks.rs` 的 SessionEnd）。
const NO_HANDOFF_MARK: &str = "agent 會話結束但沒有交接";

/// 範圍：一個組、全部 repo、或指定幾個 repo（只有 report 用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayScope {
    Group(String),
    All,
    Repos(Vec<String>),
}

// ---------------------------------------------------------------- 「在等你」的判斷

/// next_step 看起來是在等人（不是 agent 能自己動手的）。只用通用詞，不認任何人名：
/// - 子句開頭的「等／等待」＋幾個字內的決定類動作（「等使用者挑…」「等 X 看過」「等你決定」）
/// - 「請／問／等」＋使用者／user／你（「請使用者戴耳機看一輪」「問使用者要不要…」「等使用者口頭補…」），
///   或整句以使用者／user 開頭（「使用者開新會話試一輪」）
/// - 使用者／user＋決定類動作（「使用者確認後…」「使用者審閱…」）
/// - 開頭是一個英文字的人或角色＋決定類動作（「X 確認後 commit」「X 挑定要讀的書後」）
/// - 拍板、人工驗收／測試、目視／肉眼檢查
/// - 外加未結清理用的 [`agentapi::asks_user`]（「要不要」「是否要」「待確認」…）
static WAITS_ON_YOU: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?:^|[，。；：、\s（(*])等(?:待)?[^，。；]{0,12}?(?:決定|確認|回覆|回應|看過|看完|審閱|挑|選|拍板|同意|核准|驗收|提供)",
        r"|(?:^|[，。；：、\s（(*])(?:請|問|等待|等)\s*(?:使用者|用戶|(?i:user)|你|您)",
        r"|^[\s*]*(?:使用者|用戶|(?i:user)\s)",
        r"|(?:使用者|用戶|(?i:user))\s*(?:決定|確認|挑|選|審閱|回覆|拍板|同意|看過|驗收)",
        r"|^[\s*]*[A-Za-z][A-Za-z.\-]*\s*(?:確認|決定|拍板|挑定|選定|審閱|同意|核准|看過)",
        r"|拍板|人工驗收|人工測試|手動驗收|目視|肉眼",
    ))
    .expect("寫死的 regex")
});

/// 這一步是不是在等人決定／確認／驗收。
pub fn waits_on_you(next_step: &str) -> bool {
    WAITS_ON_YOU.is_match(next_step) || agentapi::asks_user(next_step)
}

// ---------------------------------------------------------------- plan

/// 一個 repo 這次能不能接力。判斷照下面的**優先序**，先符合的算數：
///
/// `Missing` ＞ `Busy` ＞ `SessionOpen` ＞ `Unhandoffed` ＞ `NoHandoff` ＞ `Stale` ＞ `NeedsYou` ＞ `Ready`
///
/// 理由：路徑不在什麼都做不了；有會話正在處理就別打斷；會話開著（閒著、等你）就在那場接著做、別再開一個；交接之後又有沒交接的會話，交接的下一步可能已經過時
/// （沒交接過、卻有沒交接的會話也算這類——先補交接最有用）；沒交接就沒有可接的；
/// 太舊的交接即使寫著「等你」也要先看現況；最後才看下一步是不是在等你。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayStatus {
    /// 有交接、下一步看起來 agent 能自己動手。
    Ready,
    /// 下一步在等你（決定、確認、驗收）。
    NeedsYou,
    /// 交接超過 [`STALE_DAYS`] 天。
    Stale,
    /// 沒有交接（或交接沒寫下一步）。
    NoHandoff,
    /// 上次交接之後又有會話結束時沒交接（最近 [`STALE_DAYS`] 天內）：下一步可能過時，先補交接。
    Unhandoffed,
    /// 這個 repo 有 App 會話正在處理中（不要打斷）。
    Busy,
    /// 這個 repo 有 App 會話開著、閒著或在等你（第 2 輪）：其實是在等人回一句，用 `relay answer` 送到那場會話。
    SessionOpen,
    /// 登記的路徑不存在。
    Missing,
}

impl RelayStatus {
    /// 表上列出的順序（也是 `counts` 的順序）。
    pub const ORDER: [RelayStatus; 8] = [
        RelayStatus::Ready,
        RelayStatus::NeedsYou,
        RelayStatus::SessionOpen,
        RelayStatus::Unhandoffed,
        RelayStatus::Stale,
        RelayStatus::Busy,
        RelayStatus::NoHandoff,
        RelayStatus::Missing,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RelayStatus::Ready => "ready",
            RelayStatus::NeedsYou => "needs_you",
            RelayStatus::Stale => "stale",
            RelayStatus::NoHandoff => "no_handoff",
            RelayStatus::Unhandoffed => "unhandoffed",
            RelayStatus::Busy => "busy",
            RelayStatus::SessionOpen => "session_open",
            RelayStatus::Missing => "missing",
        }
    }

    /// 人看的名稱。
    pub fn label(self) -> &'static str {
        match self {
            RelayStatus::Ready => "可接",
            RelayStatus::NeedsYou => "等你",
            RelayStatus::Stale => "交接太舊",
            RelayStatus::NoHandoff => "沒有交接",
            RelayStatus::Unhandoffed => "先補交接",
            RelayStatus::Busy => "會話處理中",
            RelayStatus::SessionOpen => "會話開著（閒著／等你）",
            RelayStatus::Missing => "路徑不存在",
        }
    }

    fn rank(self) -> usize {
        Self::ORDER
            .iter()
            .position(|s| *s == self)
            .unwrap_or(Self::ORDER.len())
    }
}

/// plan 的一列。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanRow {
    pub repo: String,
    /// 所屬的組（範圍是組＝那個組；全部＝registry 裡第一個有它的組）
    pub group: Option<String>,
    pub path: String,
    pub status: RelayStatus,
    /// 上次交接的時間（`YYYY-MM-DD HH:MM`）
    pub handoff_at: Option<String>,
    /// 上次交接的 ref（`decision:<id>`）
    pub handoff_ref: Option<String>,
    /// 上次交接「下次從這裡接」原文
    pub next_step: Option<String>,
    /// 建議送給 agent 的一句任務（等你、在跑、沒交接、路徑不在時沒有）
    pub prompt: Option<String>,
    /// 備註（沒 commit 的檔、在跑的會話、沒交接的會話…）
    pub notes: Vec<String>,
    /// 這列在等你回一句（NeedsYou、會話開著且下一步在等你、太舊且下一步在等你）——`relay answer` 的對象
    pub waits_on_you: bool,
    /// 上次交接帶的提問（問題、選項、建議…；F-123）。沒有＝不出現這個鍵
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask: Option<Ask>,
    /// 這筆交接已經有人（或放行規則）回答了：回答留痕的 ref（F-123／F-124）。沒有＝不出現這個鍵
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answered: Option<String>,
}

impl PlanRow {
    /// 這列是不是一張「要你決定」的卡（`ctx decide list` 與 plan 的 `waiting_on_you` 同一個算法）：
    /// 有交接、還沒回答、帶了提問或接力判斷在等你；路徑不存在、會話處理中的不算——
    /// `include_blocked`＝這兩種也算（`decide answer` 用，好讓送達那一步說出為什麼送不出去）。
    pub fn is_decision_card(&self, include_blocked: bool) -> bool {
        let blocked = matches!(self.status, RelayStatus::Missing | RelayStatus::Busy);
        if blocked && !include_blocked {
            return false;
        }
        self.handoff_ref.is_some()
            && self.answered.is_none()
            && (self.ask.is_some()
                || self.waits_on_you
                || (blocked && self.next_step.as_deref().is_some_and(waits_on_you)))
    }
}

/// 一種狀態有幾個。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusCount {
    pub status: RelayStatus,
    pub count: usize,
}

/// `plan` 的結果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    /// 範圍標籤（組名／「全部」）
    pub scope: String,
    /// 產生時間（`YYYY-MM-DD HH:MM`）
    pub at: String,
    /// 可接在前、交接新的在前
    pub rows: Vec<PlanRow>,
    /// 各狀態數量（照 [`RelayStatus::ORDER`]，0 的不列）
    pub counts: Vec<StatusCount>,
    /// 沒列的 repo 數（參考類、暫停、封存）
    pub skipped: usize,
    /// 要你決定的共幾件（[`PlanRow::is_decision_card`] 的列數，跟 `ctx decide list` 一樣）
    pub waiting_on_you: usize,
}

/// 現在的會話（App 開的，看狀態檔）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionNow {
    /// 處理中
    Running,
    /// 要你允許／回答，或 agent 說在等你
    NeedsYou,
    /// 開著、這一輪做完了（閒著）
    Idle,
    /// 沒有會話
    None,
}

impl SessionNow {
    pub fn label(self) -> &'static str {
        match self {
            SessionNow::Running => "在跑",
            SessionNow::NeedsYou => "等你",
            SessionNow::Idle => "閒著",
            SessionNow::None => "沒有",
        }
    }
}

/// 每個 repo 現在的會話（同一個 repo 有好幾場時：在跑＞等你＞閒著）。
fn sessions_by_repo(spine_dir: &Path) -> Vec<(String, SessionNow, Option<String>)> {
    let mut out: Vec<(String, SessionNow, Option<String>)> = Vec::new();
    for p in mailbox::peers(spine_dir) {
        let Some(repo) = p["repo"].as_str() else {
            continue;
        };
        let now = match p["status"].as_str().and_then(AgentStatus::parse) {
            Some(AgentStatus::Working) => SessionNow::Running,
            Some(s) if s.needs_you() => SessionNow::NeedsYou,
            Some(AgentStatus::Exited) => continue,
            _ => SessionNow::Idle,
        };
        let sid = p["sid"].as_str().map(str::to_string);
        let weight = |s: SessionNow| match s {
            SessionNow::Running => 3,
            SessionNow::NeedsYou => 2,
            SessionNow::Idle => 1,
            SessionNow::None => 0,
        };
        match out.iter_mut().find(|(r, _, _)| r == repo) {
            Some(slot) if weight(now) > weight(slot.1) => {
                slot.1 = now;
                slot.2 = sid;
            }
            Some(_) => {}
            None => out.push((repo.to_string(), now, sid)),
        }
    }
    out
}

fn session_of(
    sessions: &[(String, SessionNow, Option<String>)],
    id: &str,
) -> (SessionNow, Option<String>) {
    sessions
        .iter()
        .find(|(r, _, _)| r == id)
        .map(|(_, s, sid)| (*s, sid.clone()))
        .unwrap_or((SessionNow::None, None))
}

fn of_repo(ev: &Event, id: &str) -> bool {
    ev.kv("repo")
        .is_some_and(|r| !r.is_empty() && spine::same_token(r, id))
}

/// hook 記的「agent 會話結束但沒有交接」（總管簡報的異常段也用）。
pub fn is_no_handoff(ev: &Event) -> bool {
    ev.etype == EventType::Suggestion && ev.source == "hook" && ev.body.starts_with(NO_HANDOFF_MARK)
}

fn parse_at(at: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(at, "%Y-%m-%d %H:%M").ok()
}

/// 範圍內的一個 repo 與它所屬的組。
type Member = (Repo, Option<String>);

/// 範圍內要看的 repo（參考類、暫停、封存的拿掉，回傳拿掉幾個）＋各自的組。
/// 指定 repo（[`RelayScope::Repos`]）時照給的列，不拿掉。
fn members(spine_dir: &Path, scope: &RelayScope) -> Result<(String, Vec<Member>, usize)> {
    let data = registry::load(spine_dir)?;
    let (label, entries, explicit) = match scope {
        RelayScope::Group(g) => {
            let (name, e) = registry::resolve_group(spine_dir, Some(g), None)?;
            (name, e, false)
        }
        RelayScope::All => {
            let (name, e) = registry::resolve_group(spine_dir, None, None)?;
            (name, e, false)
        }
        RelayScope::Repos(ids) => {
            if ids.is_empty() {
                return Err(Error::invalid("--repos 要給至少一個 repo id（逗號分開）"));
            }
            let (name, e) = registry::resolve_group(spine_dir, None, Some(ids))?;
            (name, e, true)
        }
    };
    let group_of = |id: &str| -> Option<String> {
        if let RelayScope::Group(_) = scope {
            return Some(label.clone());
        }
        data.groups
            .iter()
            .find(|g| g.members().iter().any(|m| m == id))
            .map(|g| g.name())
    };
    let mut kept = Vec::new();
    let mut skipped = 0;
    for e in entries {
        let skip = !explicit
            && (e.is_reference() || matches!(e.tier(), Some(Tier::Paused | Tier::Archived)));
        if skip {
            skipped += 1;
            continue;
        }
        let g = group_of(&e.id());
        kept.push((e, g));
    }
    Ok((label, kept, skipped))
}

fn states_of(spine_dir: &Path, entries: &[Member]) -> Vec<RepoState> {
    let specs: Vec<RepoSpec> = entries.iter().map(|(r, _)| RepoSpec::from(r)).collect();
    collect::collect_group(&specs, Some(spine_dir))
}

fn trim_end_punct(s: &str) -> &str {
    s.trim().trim_end_matches(['。', '.', '；', ';'])
}

/// 給 agent 的一句任務。
fn prompt_for(
    status: RelayStatus,
    h: Option<&Handoff>,
    days: i64,
    gap_at: Option<&str>,
) -> Option<String> {
    let next = h.and_then(|h| h.next_step.as_deref()).map(trim_end_punct);
    match (status, h, next) {
        (RelayStatus::Ready, Some(h), Some(n)) => Some(format!(
            "接續上次交接（{}）：{n}。先確認這一步還成立再動手；做完 handoff。",
            h.at
        )),
        (RelayStatus::Stale, Some(h), Some(n)) => Some(format!(
            "上次交接是 {}（{days} 天前）：{n}。先看這段時間的 commit 與沒 commit 的檔，確認這一步還成立再動手；做完 handoff。",
            h.at
        )),
        (RelayStatus::Unhandoffed, _, _) => {
            let tail = match next {
                Some(n) => format!("，再看上次交接的下一步「{n}」還成不成立"),
                None => String::new(),
            };
            Some(format!(
                "{} 的會話結束時沒有交接。先看 git log 與沒 commit 的檔補一筆 handoff（做到哪、剩什麼、下次從哪接）{tail}；不確定的先問我。",
                gap_at.unwrap_or("上次")
            ))
        }
        _ => None,
    }
}

/// 接力前看一眼：範圍內每個 repo 能不能直接從上次交接接著做。純讀取。
pub fn plan(spine_dir: &Path, scope: &RelayScope) -> Result<Plan> {
    plan_at(spine_dir, scope, Local::now().naive_local())
}

/// 同 [`plan`]，指定「現在」（測試用）。
pub fn plan_at(spine_dir: &Path, scope: &RelayScope, now: NaiveDateTime) -> Result<Plan> {
    let (label, entries, skipped) = members(spine_dir, scope)?;
    let states = states_of(spine_dir, &entries);
    let events = spine::iter_events(spine_dir)?;
    let history = crate::decision_log::history_of(&events);
    let sessions = sessions_by_repo(spine_dir);
    let mut rows: Vec<PlanRow> = Vec::with_capacity(entries.len());
    for ((entry, group), st) in entries.iter().zip(&states) {
        let id = entry.id();
        let ev = events
            .iter()
            .rfind(|ev| agentapi::is_handoff(ev) && of_repo(ev, &id));
        let h = ev.map(agentapi::parse_handoff);
        let h_stamp = ev.map(Event::stamp);
        let next = h
            .as_ref()
            .and_then(|h| h.next_step.clone())
            .filter(|n| !n.trim().is_empty() && n.trim() != "無");
        let age_days = h
            .as_ref()
            .and_then(|h| parse_at(&h.at))
            .map(|at| (now - at).num_days());
        let horizon = (now - chrono::Duration::days(STALE_DAYS))
            .format("%Y-%m-%d %H:%M")
            .to_string();
        // 交接之後（沒交接＝最近 STALE_DAYS 天內）又有會話結束沒交接
        let gap = events.iter().rfind(|e| {
            is_no_handoff(e)
                && of_repo(e, &id)
                && e.stamp() >= horizon
                && h_stamp.as_ref().is_none_or(|s| e.stamp() > *s)
        });
        let (session, sid) = session_of(&sessions, &id);
        // 交接帶了提問（F-123）就是在等你，不用再猜下一步的文字
        let ask = h.as_ref().and_then(|h| h.ask.clone());
        // 已經用 `ctx decide answer` 回答過的交接不再等你：任務帶著回答，可以接（F-123）
        let answered = h
            .as_ref()
            .and_then(|h| crate::decision_log::answer_of(&history, &id, &h.reference, &h.at));
        let waits =
            answered.is_none() && (ask.is_some() || next.as_deref().is_some_and(waits_on_you));
        let mut notes = Vec::new();
        if let Some(a) = answered {
            notes.push(format!(
                "已回答（{}）：{}",
                a.reference,
                crate::pystr::clip(&a.answer, 60)
            ));
        }
        let status = if !st.exists {
            notes.push(format!("登記的路徑不存在：{}", st.path));
            RelayStatus::Missing
        } else if session != SessionNow::None {
            notes.push(format!(
                "會話{}：{}",
                sid.map(|s| format!(" {s}")).unwrap_or_default(),
                session.label()
            ));
            if session == SessionNow::Running {
                RelayStatus::Busy
            } else {
                RelayStatus::SessionOpen
            }
        } else if let Some(g) = gap {
            notes.push(format!("{} 的會話結束時沒有交接", g.stamp()));
            RelayStatus::Unhandoffed
        } else if h.is_none() || next.is_none() {
            if h.is_some() {
                notes.push("上次交接沒寫下一步".into());
            }
            RelayStatus::NoHandoff
        } else if age_days.is_some_and(|d| d > STALE_DAYS) {
            notes.push(format!("交接是 {} 天前", age_days.unwrap_or_default()));
            if waits {
                notes.push(NOTE_WAITS.into());
            }
            RelayStatus::Stale
        } else if waits {
            notes.push(NOTE_WAITS.into());
            RelayStatus::NeedsYou
        } else {
            RelayStatus::Ready
        };
        if st.exists && st.dirty > 0 {
            notes.push(format!("{} 個檔沒 commit", st.dirty));
        }
        // 太舊又在等你的不給任務：agent 接了也只能回頭問你
        let prompt = match (answered, &h) {
            (Some(a), Some(h)) if matches!(status, RelayStatus::Ready | RelayStatus::Stale) => {
                Some(answer_task(&id, h, &a.answer))
            }
            _ => prompt_for(
                status,
                h.as_ref()
                    .filter(|_| next.is_some() && !(status == RelayStatus::Stale && waits)),
                age_days.unwrap_or_default(),
                gap.map(Event::stamp).as_deref(),
            ),
        };
        rows.push(PlanRow {
            repo: id,
            group: group.clone(),
            path: st.path.clone(),
            status,
            handoff_at: h.as_ref().map(|h| h.at.clone()),
            handoff_ref: h.as_ref().map(|h| h.reference.clone()),
            next_step: next,
            prompt,
            notes,
            waits_on_you: match status {
                RelayStatus::NeedsYou => true,
                RelayStatus::SessionOpen | RelayStatus::Stale => waits,
                _ => false,
            },
            ask,
            answered: answered.map(|a| a.reference.clone()),
        });
    }
    rows.sort_by(|a, b| {
        a.status
            .rank()
            .cmp(&b.status.rank())
            .then_with(|| b.handoff_at.cmp(&a.handoff_at))
            .then_with(|| a.repo.cmp(&b.repo))
    });
    let counts = RelayStatus::ORDER
        .iter()
        .map(|s| StatusCount {
            status: *s,
            count: rows.iter().filter(|r| r.status == *s).count(),
        })
        .filter(|c| c.count > 0)
        .collect();
    Ok(Plan {
        scope: label,
        at: now.format("%Y-%m-%d %H:%M").to_string(),
        waiting_on_you: rows.iter().filter(|r| r.is_decision_card(false)).count(),
        rows,
        counts,
        skipped,
    })
}

// ---------------------------------------------------------------- report

/// 一個 repo 這段時間的結論。判斷順序：在跑 ＞ 等你 ＞ 有交接 ＞ 有 commit 沒交接 ＞ 沒動靜。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// 有交接（下一步不是在等你）
    Done,
    /// 會話在等你，或最後一筆交接的下一步在等你
    Waiting,
    /// 會話還在處理中
    Running,
    /// 有 commit 但沒交接
    NoHandoff,
    /// 沒動靜
    Idle,
}

impl Verdict {
    pub const ORDER: [Verdict; 5] = [
        Verdict::Done,
        Verdict::Waiting,
        Verdict::Running,
        Verdict::NoHandoff,
        Verdict::Idle,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Done => "done",
            Verdict::Waiting => "waiting",
            Verdict::Running => "running",
            Verdict::NoHandoff => "no_handoff",
            Verdict::Idle => "idle",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Verdict::Done => "有交接",
            Verdict::Waiting => "等你",
            Verdict::Running => "在跑",
            Verdict::NoHandoff => "有 commit 沒交接",
            Verdict::Idle => "沒動靜",
        }
    }
}

/// since 之後的一筆交接。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HandoffBrief {
    pub at: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub done: Option<String>,
    pub next_step: Option<String>,
}

/// since 之後的一個 commit。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitBrief {
    pub short: String,
    pub subject: String,
    pub at: String,
}

/// report 的一列。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReportRow {
    pub repo: String,
    pub group: Option<String>,
    pub path: String,
    pub verdict: Verdict,
    /// since 之後的交接（舊到新）
    pub handoffs: Vec<HandoffBrief>,
    /// since 之後的 commit 數
    pub commits: usize,
    /// 最新的幾個 commit（新到舊，最多 [`REPORT_COMMITS`] 個）
    pub recent_commits: Vec<CommitBrief>,
    /// 目前沒 commit 的檔數
    pub dirty: usize,
    pub session: SessionNow,
    /// 登記的路徑在不在
    pub exists: bool,
}

/// 各結論的數量＋一行總計。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportTotal {
    pub repos: usize,
    pub done: usize,
    pub waiting: usize,
    pub running: usize,
    pub no_handoff: usize,
    pub idle: usize,
    pub commits: usize,
    pub handoffs: usize,
    /// 人看的一行
    pub line: String,
}

/// `report` 的結果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub scope: String,
    pub since: String,
    /// 照結論排（有交接、等你、在跑、有 commit 沒交接、沒動靜），同結論照 repo id
    pub rows: Vec<ReportRow>,
    pub total: ReportTotal,
    /// 沒列的 repo 數（參考類、暫停、封存；指定 repo 時是 0）
    pub skipped: usize,
}

/// 把 `--since` 正規成 `YYYY-MM-DD HH:MM`（也收只有日期的，當 00:00）。
pub fn parse_since(since: &str) -> Result<String> {
    let s = since.trim();
    if let Ok(t) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M") {
        return Ok(t.format("%Y-%m-%d %H:%M").to_string());
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(format!("{} 00:00", d.format("%Y-%m-%d")));
    }
    Err(Error::invalid(format!(
        "--since 看不懂：「{since}」——請寫成 \"YYYY-MM-DD HH:MM\"（例：\"2026-11-14 09:00\"）"
    )))
}

/// 接力之後看一眼：範圍內每個 repo 在 `since` 之後做了什麼。純讀取。
pub fn report(spine_dir: &Path, scope: &RelayScope, since: &str) -> Result<Report> {
    let since = parse_since(since)?;
    let (label, entries, skipped) = members(spine_dir, scope)?;
    let states = states_of(spine_dir, &entries);
    let events = spine::iter_events(spine_dir)?;
    let history = crate::decision_log::history_of(&events);
    let sessions = sessions_by_repo(spine_dir);
    let mut rows = Vec::with_capacity(entries.len());
    for ((entry, group), st) in entries.iter().zip(&states) {
        let id = entry.id();
        // 最後一筆交接在等你：帶了提問或下一步的文字在等人，而且還沒用 decide answer 回答（F-123，同 plan）
        let mut last_waits = false;
        let handoffs: Vec<HandoffBrief> = events
            .iter()
            .filter(|ev| agentapi::is_handoff(ev) && of_repo(ev, &id) && ev.stamp() >= since)
            .map(|ev| {
                let h = agentapi::parse_handoff(ev);
                last_waits = (h.ask.is_some() || h.next_step.as_deref().is_some_and(waits_on_you))
                    && crate::decision_log::answer_of(&history, &id, &h.reference, &h.at).is_none();
                HandoffBrief {
                    at: h.at,
                    reference: h.reference,
                    done: h.done,
                    next_step: h.next_step,
                }
            })
            .collect();
        let (commits, recent) = if st.exists && !entry.is_reference() {
            gitlog::since(Path::new(&st.path), &since, REPORT_COMMITS)
        } else {
            (0, Vec::new())
        };
        let (session, _) = session_of(&sessions, &id);
        let verdict = if session == SessionNow::Running {
            Verdict::Running
        } else if session == SessionNow::NeedsYou || last_waits {
            Verdict::Waiting
        } else if !handoffs.is_empty() {
            Verdict::Done
        } else if commits > 0 {
            Verdict::NoHandoff
        } else {
            Verdict::Idle
        };
        rows.push(ReportRow {
            repo: id,
            group: group.clone(),
            path: st.path.clone(),
            verdict,
            handoffs,
            commits,
            recent_commits: recent
                .into_iter()
                .map(|c| CommitBrief {
                    short: c.short,
                    subject: c.subject,
                    at: c.at,
                })
                .collect(),
            dirty: st.dirty,
            session,
            exists: st.exists,
        });
    }
    let rank = |v: Verdict| Verdict::ORDER.iter().position(|x| *x == v).unwrap_or(0);
    rows.sort_by(|a, b| {
        rank(a.verdict)
            .cmp(&rank(b.verdict))
            .then_with(|| a.repo.cmp(&b.repo))
    });
    let n = |v: Verdict| rows.iter().filter(|r| r.verdict == v).count();
    let commits: usize = rows.iter().map(|r| r.commits).sum();
    let handoffs: usize = rows.iter().map(|r| r.handoffs.len()).sum();
    let parts: Vec<String> = Verdict::ORDER
        .iter()
        .map(|v| (v, n(*v)))
        .filter(|(_, c)| *c > 0)
        .map(|(v, c)| format!("{} {c}", v.label()))
        .collect();
    let line = format!(
        "{} 個 repo：{}；共 {handoffs} 筆 handoff、{commits} 個 commit",
        rows.len(),
        if parts.is_empty() {
            "（無）".to_string()
        } else {
            parts.join("、")
        }
    );
    let total = ReportTotal {
        repos: rows.len(),
        done: n(Verdict::Done),
        waiting: n(Verdict::Waiting),
        running: n(Verdict::Running),
        no_handoff: n(Verdict::NoHandoff),
        idle: n(Verdict::Idle),
        commits,
        handoffs,
        line,
    };
    Ok(Report {
        scope: label,
        since,
        rows,
        total,
        skipped,
    })
}

// ---------------------------------------------------------------- answer（第 2 輪）

/// 送給 agent 的任務文字大約多長（超過就截交接摘要與下一步；使用者的回覆不截）。
pub const ANSWER_CHARS: usize = 800;

/// 回覆要怎麼送到 agent。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Delivery {
    /// 這個 repo 有 App 會話開著（閒著或在等你）：在那場會話貼上任務文字。
    ExistingSession { sid: String, state: SessionNow },
    /// 沒有開著的會話，但找得到留下這筆交接的 Claude Code 會話：`claude --resume` 接回去。
    Resume {
        claude_session_id: String,
        cwd: String,
    },
    /// 開一場新的會話。
    NewSession { cwd: String },
}

/// `answer` 的結果：要送的任務文字＋怎麼送。純計算，不寫事件、不開會話。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Answer {
    pub repo: String,
    pub path: String,
    pub handoff_at: String,
    pub handoff_ref: String,
    pub next_step: Option<String>,
    /// 要送給 agent 的任務文字
    pub task: String,
    pub delivery: Delivery,
    /// 可以直接貼到終端機的指令（ExistingSession 沒有——在 App 的那場會話貼上任務文字）
    pub commands: Vec<String>,
    /// 回覆裡遮掉幾處像憑證的字
    pub redacted: usize,
}

fn rejected(code: RejectCode, msg: String, hint: &str) -> Error {
    Error::rejected(code, msg, Some(hint.to_string()))
}

/// 組任務文字：上次交接（時間、ref、完成摘要、下一步）＋使用者的回覆＋怎麼接。
fn answer_task(repo: &str, h: &Handoff, reply: &str) -> String {
    let head = format!("接續 {repo} 上次的交接（{}，{}）。", h.at, h.reference);
    let tail = "照回覆接著做；回覆沒涵蓋到、仍要人決定的部分不要自己猜，做完能做的就 handoff 並寫清楚還在等什麼。";
    // 交接帶了提問（F-123）：講清楚這是在回答哪一題
    let reply_line = match &h.ask {
        Some(a) => format!("使用者對「{}」的回答是：{reply}", a.question),
        None => format!("使用者的回覆：{reply}"),
    };
    let fixed = head.chars().count() + tail.chars().count() + reply_line.chars().count() + 20;
    let room = ANSWER_CHARS.saturating_sub(fixed);
    // 下一步比完成摘要重要：先給下一步，剩下的給摘要；兩個都至少留一點
    let summary_src = h
        .done
        .as_deref()
        .filter(|d| !d.trim().is_empty())
        .unwrap_or(&h.summary);
    let next_src = h.next_step.as_deref().unwrap_or("（沒寫）");
    let next_room = room.saturating_mul(2) / 3;
    let next = crate::pystr::clip(next_src.trim(), next_room.max(60));
    let summary_room = room.saturating_sub(next.chars().count()).max(40);
    let summary = crate::pystr::clip(summary_src.trim(), summary_room.min(160));
    [
        head,
        format!("完成：{summary}"),
        format!("下次從這裡接：{next}"),
        reply_line,
        tail.to_string(),
    ]
    .join("\n")
}

/// 留下這筆交接的 Claude Code 會話：範圍是這個 repo 的過去會話裡，時間涵蓋交接時間（開始 ≤ 交接 ≤ 最後活動），
/// 而且逐字稿裡真的有這筆交接的 ref（`handoff` 回傳的 `decision:<日>-dN`）。
/// 交接沒有 id（ref 只是 `decision:HH:MM`，不唯一）就不猜；兩分鐘內還在寫的（可能還開著）也不接。
fn resume_source(
    spine_dir: &Path,
    repo: &str,
    h: &Handoff,
    projects_root: Option<&Path>,
) -> Option<(String, String)> {
    let has_id = h
        .reference
        .split_once(':')
        .is_some_and(|(_, k)| k.contains('-'));
    if !has_id {
        return None;
    }
    let at = parse_at(&h.at)?;
    let ids = [repo.to_string()];
    let past =
        crate::history::past_sessions(spine_dir, None, Some(&ids), usize::MAX, projects_root)
            .ok()?;
    past.into_iter()
        .filter(|ps| !ps.live)
        .filter(|ps| parse_at(&ps.updated).is_some_and(|u| u >= at))
        .filter(|ps| {
            ps.started
                .as_deref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.with_timezone(&Local).naive_local())
                .is_none_or(|t| t <= at + chrono::Duration::minutes(1))
        })
        .find(|ps| {
            std::fs::read(&ps.path).is_ok_and(|b| {
                let needle = h.reference.as_bytes();
                b.windows(needle.len()).any(|w| w == needle)
            })
        })
        .map(|ps| (ps.id, ps.cwd))
}

/// 人回了一句，組出要送給這個 repo 的 agent 的任務與送達方式。純計算。
///
/// 錯誤（都是 [`Error::Rejected`]）：repo 沒登記／路徑不存在／沒有交接（`not_found`）、
/// 回覆是空的（`bad_request`）、有會話正在處理中（`busy`——不要打斷，等它這輪做完）。
pub fn answer(spine_dir: &Path, repo: &str, reply: &str) -> Result<Answer> {
    answer_with(spine_dir, repo, reply, None)
}

/// 同 [`answer`]，指定 Claude Code 逐字稿的根目錄（測試用；`None`＝`~/.claude/projects`）。
pub fn answer_with(
    spine_dir: &Path,
    repo: &str,
    reply: &str,
    projects_root: Option<&Path>,
) -> Result<Answer> {
    let reply = reply.trim();
    if reply.is_empty() {
        return Err(rejected(
            RejectCode::BadRequest,
            "回覆是空的".into(),
            "寫一句要給 agent 的回覆，例：ctx relay answer <repo> \"選方案 B\"",
        ));
    }
    let (reply, redacted) = crate::redact::redact_secrets(reply);
    let data = registry::load(spine_dir)?;
    let Some(entry) = data.repo(repo) else {
        return Err(rejected(
            RejectCode::NotFound,
            format!("repo 沒登記：{repo}"),
            "用 ctx relay plan --all 看登記的 repo id",
        ));
    };
    let id = entry.id();
    let path = crate::fsutil::expanduser(entry.path());
    if !path.is_dir() {
        return Err(rejected(
            RejectCode::NotFound,
            format!("{id} 登記的路徑不存在：{}", path.display()),
            "先把 repo 放回原處，或用 ctx registry 改登記的路徑",
        ));
    }
    let events = spine::iter_events(spine_dir)?;
    let Some(ev) = events
        .iter()
        .rfind(|ev| agentapi::is_handoff(ev) && of_repo(ev, &id))
    else {
        return Err(rejected(
            RejectCode::NotFound,
            format!("{id} 沒有交接，沒有可接的下一步"),
            "直接開會話交代任務：ctx session --repos <repo> --repo <repo> --task \"…\"",
        ));
    };
    let h = agentapi::parse_handoff(ev);
    let (session, sid) = session_of(&sessions_by_repo(spine_dir), &id);
    if session == SessionNow::Running {
        return Err(rejected(
            RejectCode::Busy,
            format!(
                "{id} 的會話{}正在處理中，不要打斷",
                sid.map(|s| format!(" {s} ")).unwrap_or_default()
            ),
            "等它這一輪做完（ctx relay plan 會顯示「會話開著」）再回覆",
        ));
    }
    let task = answer_task(&id, &h, &reply);
    let cwd = path.to_string_lossy().into_owned();
    let q = crate::term::shlex_quote;
    let (delivery, commands) = match (session, sid) {
        (SessionNow::NeedsYou | SessionNow::Idle, Some(sid)) => (
            Delivery::ExistingSession {
                sid,
                state: session,
            },
            Vec::new(),
        ),
        _ => match resume_source(spine_dir, &id, &h, projects_root) {
            Some((cid, dir)) => {
                let cmd = format!("cd {} && claude --resume {cid} {}", q(&dir), q(&task));
                (
                    Delivery::Resume {
                        claude_session_id: cid,
                        cwd: dir,
                    },
                    vec![cmd],
                )
            }
            None => {
                // 範圍只有這個 repo（只給 --repo 的話情境卡會是全部 repo）
                let cmd = format!(
                    "ctx session --repos {0} --repo {0} --task {1}",
                    q(&id),
                    q(&task)
                );
                (Delivery::NewSession { cwd }, vec![cmd])
            }
        },
    };
    Ok(Answer {
        repo: id,
        path: path.to_string_lossy().into_owned(),
        handoff_at: h.at.clone(),
        handoff_ref: h.reference.clone(),
        next_step: h.next_step.clone(),
        task,
        delivery,
        commands,
        redacted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_on_you_positive() {
        for s in [
            "等使用者挑要讀的書後，再走入庫流程",
            "等你決定要不要 commit",
            "使用者確認後 commit/push",
            "請使用者戴耳機從首頁看三集連播，決定要不要 push",
            "問使用者要不要把路徑改掉",
            "Someone 確認後 commit 兩檔",
            "PM 挑定要讀的書後，寫進讀法段",
            "總召拍板出入表 #1–3",
            "照 TESTING.md 16 項人工驗收",
            "等 user 決定是否 commit",
            "使用者審閱交接草稿 → commit",
            "等待回覆後再改用語",
            "等使用者口頭補各段續辦事項後改正文",
            "使用者開新會話，確認第一段是菜單",
            "在簡報軟體打開 opening.pptx 目視檢查版面",
        ] {
            assert!(waits_on_you(s), "應該是等你：{s}");
        }
    }

    #[test]
    fn waits_on_you_negative() {
        for s in [
            "重 build App 並裝到 /Applications，重跑 ctx setup --claude",
            "VM agent 依 HANDOFF.md 第 0 步升級 certifi 並回報版本",
            "先確認測試通過後再 commit",
            "跑完 eval，確認分數沒掉後 commit",
            "補 README 的安裝段；選單、按鈕等元件照舊",
            "把迷你選單改成下拉",
            "README 選讀段補例子",
            "username 欄位改名後跑測試",
            "§8d 其餘：優先 8d-7、8d-9",
        ] {
            assert!(!waits_on_you(s), "不該是等你：{s}");
        }
    }

    #[test]
    fn parse_since_formats() {
        assert_eq!(parse_since("2026-11-14 09:05").unwrap(), "2026-11-14 09:05");
        assert_eq!(parse_since(" 2026-11-14 ").unwrap(), "2026-11-14 00:00");
        assert!(parse_since("昨天").is_err());
    }
}
