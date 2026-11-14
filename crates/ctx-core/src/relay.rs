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
use crate::collect::{self, RepoSpec, RepoState};
use crate::registry::{self, Repo, Tier};
use crate::spine::{self, Event, EventType};
use crate::status::AgentStatus;
use crate::{gitlog, Error, Result};

/// 交接超過幾天就不直接接（可能過時，要先看現況）。
pub const STALE_DAYS: i64 = 14;
/// report 每個 repo 列前幾個 commit 標題。
pub const REPORT_COMMITS: usize = 3;

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
/// `Missing` ＞ `Busy` ＞ `Unhandoffed` ＞ `NoHandoff` ＞ `Stale` ＞ `NeedsYou` ＞ `Ready`
///
/// 理由：路徑不在什麼都做不了；有會話在跑就別再開一個；交接之後又有沒交接的會話，交接的下一步可能已經過時
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
    /// 這個 repo 現在有 App 會話在跑。
    Busy,
    /// 登記的路徑不存在。
    Missing,
}

impl RelayStatus {
    /// 表上列出的順序（也是 `counts` 的順序）。
    pub const ORDER: [RelayStatus; 7] = [
        RelayStatus::Ready,
        RelayStatus::NeedsYou,
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
            RelayStatus::Busy => "會話在跑",
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

fn is_no_handoff(ev: &Event) -> bool {
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
        let waits = next.as_deref().is_some_and(waits_on_you);
        let mut notes = Vec::new();
        let status = if !st.exists {
            notes.push(format!("登記的路徑不存在：{}", st.path));
            RelayStatus::Missing
        } else if session != SessionNow::None {
            notes.push(format!(
                "會話{}：{}",
                sid.map(|s| format!(" {s}")).unwrap_or_default(),
                session.label()
            ));
            RelayStatus::Busy
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
                notes.push("下一步在等你".into());
            }
            RelayStatus::Stale
        } else if waits {
            notes.push("下一步在等你".into());
            RelayStatus::NeedsYou
        } else {
            RelayStatus::Ready
        };
        if st.exists && st.dirty > 0 {
            notes.push(format!("{} 個檔沒 commit", st.dirty));
        }
        // 太舊又在等你的不給任務：agent 接了也只能回頭問你
        let prompt = prompt_for(
            status,
            h.as_ref()
                .filter(|_| next.is_some() && !(status == RelayStatus::Stale && waits)),
            age_days.unwrap_or_default(),
            gap.map(Event::stamp).as_deref(),
        );
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
    let sessions = sessions_by_repo(spine_dir);
    let mut rows = Vec::with_capacity(entries.len());
    for ((entry, group), st) in entries.iter().zip(&states) {
        let id = entry.id();
        let handoffs: Vec<HandoffBrief> = events
            .iter()
            .filter(|ev| agentapi::is_handoff(ev) && of_repo(ev, &id) && ev.stamp() >= since)
            .map(|ev| {
                let h = agentapi::parse_handoff(ev);
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
        let last_waits = handoffs
            .last()
            .and_then(|h| h.next_step.as_deref())
            .is_some_and(waits_on_you);
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
