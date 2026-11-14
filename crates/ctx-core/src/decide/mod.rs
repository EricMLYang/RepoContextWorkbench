//! 決策佇列（F-123／F-124，2026-11-14，Rust 才有）：把各 repo「在等你決定」的事排成一疊卡，一張一張答；
//! 同一類問題人連續照建議回答後，可以升級成放行規則（[`crate::policy`]），之後照建議自動放行。
//!
//! 為什麼要有：agent 收尾時常停在「等你決定／確認／驗收」，這些事散在二十幾個 repo 的交接裡；人要一個一個
//! 回想那個 repo 在做什麼、問題是什麼、該怎麼回。佇列把它們收成卡（repo 一行介紹、問題、編號選項、建議、證據、
//! 放了多久），回答時用編號就好，並留一筆痕（[`crate::decision_log`]）。
//!
//! 判斷都沿用既有的：
//! - 每個 repo 現在怎樣、這列算不算一張卡：[`relay::plan_at`]、[`relay::PlanRow::is_decision_card`]
//!   （接力的「要你決定的共 N 件」同一個算法）
//! - 交接的欄位與提問：[`crate::agentapi::parse_handoff`]／[`crate::ask`]
//! - 回覆怎麼送到 agent：[`relay::answer_with`]（只印不開會話）
//! - 回答、放行、推翻的留痕：[`crate::decision_log`]；放行判定與規則檔：[`crate::policy`]
//!
//! 卡＝範圍內每個 repo 的**最後一筆交接**，還沒被回答（人回答或規則放行、沒被推翻）、而且帶了提問（`structured`），
//! 或沒帶提問但接力判斷下一步在等你。路徑不存在、有會話正在處理中的不列。新的交接自然蓋掉舊的。

use std::path::Path;
use std::sync::LazyLock;

use chrono::{Local, NaiveDateTime};
use regex::Regex;
use serde::Serialize;

use crate::ask::{numbered, AskKind};
use crate::decision_log::{self as log, NewAnswer};
use crate::policy::{self, Level, Rule};
use crate::registry::{self, RegistryData};
use crate::relay::{self, PlanRow, RelayScope, RelayStatus};
use crate::{spine, Error, RejectCode, Result};

pub use crate::decision_log::{answer_of, history, history_of, Answered, AnsweredBy, FollowRec};

mod rules;
pub use rules::*;

// ---------------------------------------------------------------- 型別

/// 卡上的一個選項。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CardOption {
    /// 1 起算的編號（`ctx decide answer <repo> <n>`）
    pub n: usize,
    pub text: String,
    /// 是 agent 建議的那個
    pub recommended: bool,
}

/// 卡上的一筆證據。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Evidence {
    /// 交接裡寫的原文（repo 內相對路徑或 `<repo>:<路徑>`）
    #[serde(rename = "ref")]
    pub reference: String,
    /// 解出來的絕對路徑（repo 沒登記、路徑跳出 repo＝None）
    pub path: Option<String>,
    /// 檔案（或目錄）在不在
    pub exists: bool,
}

/// 一張要你決定的卡。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Card {
    pub repo: String,
    pub group: Option<String>,
    /// 名片的一行介紹（registry 的 description；沒寫＝None）
    pub about: Option<String>,
    /// 名片的協作角色（沒寫＝None）
    pub role: Option<String>,
    pub path: String,
    pub handoff_ref: String,
    /// `YYYY-MM-DD HH:MM`
    pub handoff_at: String,
    /// 交接到現在幾天（滿 24 小時才算 1 天）
    pub days: i64,
    /// 交接到現在幾小時
    pub hours: i64,
    /// 交接帶了提問＝true；沒帶、但接力判斷下一步在等你＝false（這時問題就是交接的下一步）
    pub structured: bool,
    pub question: String,
    /// 沒帶提問＝None
    pub kind: Option<AskKind>,
    pub options: Vec<CardOption>,
    pub recommend: Option<String>,
    /// 沒帶提問＝None
    pub reversible: Option<bool>,
    pub evidence: Vec<Evidence>,
    /// 接力看這個 repo 的狀態（等你／會話開著／交接太舊…）
    pub relay_status: RelayStatus,
    /// 接力的備註（會話開著、交接是幾天前、沒 commit 的檔…）
    pub notes: Vec<String>,
}

/// `list` 的結果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Queue {
    /// 範圍標籤（組名／「全部」）
    pub scope: String,
    /// 產生時間（`YYYY-MM-DD HH:MM`）
    pub at: String,
    /// 帶提問的在前；同一段內交接新的在前
    pub cards: Vec<Card>,
    /// 其中帶提問的幾張
    pub structured: usize,
    /// 連續照建議夠多次、可以升級成放行規則的（repo、類別）
    pub suggestions: Vec<TrustHint>,
}

/// `answer` 的結果：留下的痕＋怎麼把回答送到 agent。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Decided {
    #[serde(flatten)]
    pub answered: Answered,
    /// 回答是第幾個選項（自由文字且不等於任何選項＝None）
    pub option: Option<usize>,
    /// 送達方式（同 `ctx relay answer`）
    pub relay: relay::Answer,
    /// 這類有 auto 規則、這次卻跟建議不同 → 規則降回 ask（降級後的規則）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downgraded: Option<Rule>,
    /// 答完這次，這類連續照建議夠多次了：可以升級
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<TrustHint>,
}

// ---------------------------------------------------------------- list

/// `src/a.rs:12`、`src/a.rs:12-30` 的行號不算進路徑。
static LINE_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(.+?):\d+(?:-\d+)?$").expect("寫死的 regex"));

fn evidence_of(spine_dir: &Path, data: &RegistryData, repo: &str, raw: &str) -> Evidence {
    let bare = LINE_SUFFIX
        .captures(raw)
        .and_then(|c| c.get(1))
        .map_or(raw, |m| m.as_str());
    let reference = match bare.split_once(':') {
        Some((r, _)) if data.repo(r).is_some() => bare.to_string(),
        _ => format!("{repo}:{bare}"),
    };
    let (path, exists) = match crate::crossref::resolve(spine_dir, &reference) {
        Ok(r) => (Some(r.path), r.exists),
        Err(_) => (None, false),
    };
    Evidence {
        reference: raw.to_string(),
        path,
        exists,
    }
}

fn parse_at(at: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(at, "%Y-%m-%d %H:%M").ok()
}

/// 這筆交接回答過了沒（同一個 repo、指向它、時間不早於它——沒有 id 的 ref 只有時間，不同天會撞）。
/// plan 的一列 → 卡；不是卡＝None。`include_blocked`＝路徑不存在、會話處理中的也出卡
/// （`answer` 用：讓 [`relay::answer_with`] 說出為什麼送不出去，而不是說「沒有卡」）。
fn card_of(
    spine_dir: &Path,
    row: &PlanRow,
    data: &RegistryData,
    now: NaiveDateTime,
    include_blocked: bool,
) -> Option<Card> {
    if !row.is_decision_card(include_blocked) {
        return None;
    }
    let (handoff_ref, handoff_at) = (row.handoff_ref.as_ref()?, row.handoff_at.as_ref()?);
    let entry = data.repo(&row.repo);
    let age = parse_at(handoff_at).map(|at| now - at);
    let (days, hours) = age.map_or((0, 0), |a| (a.num_days(), a.num_hours()));
    let notes: Vec<String> = row
        .notes
        .iter()
        .filter(|n| n.as_str() != relay::NOTE_WAITS)
        .cloned()
        .collect();
    let mut card = Card {
        repo: row.repo.clone(),
        group: row.group.clone(),
        about: entry.and_then(|e| e.description()),
        role: entry.and_then(|e| e.role()),
        path: row.path.clone(),
        handoff_ref: handoff_ref.clone(),
        handoff_at: handoff_at.clone(),
        days,
        hours,
        structured: false,
        question: row.next_step.clone().unwrap_or_default(),
        kind: None,
        options: Vec::new(),
        recommend: None,
        reversible: None,
        evidence: Vec::new(),
        relay_status: row.status,
        notes,
    };
    if let Some(a) = &row.ask {
        let rec = a.recommend_index();
        card.structured = true;
        card.question = a.question.clone();
        card.kind = Some(a.kind);
        card.options = a
            .options
            .iter()
            .enumerate()
            .map(|(i, o)| CardOption {
                n: i + 1,
                text: o.clone(),
                recommended: rec == Some(i + 1),
            })
            .collect();
        card.recommend = a.recommend.clone();
        card.reversible = Some(a.reversible);
        card.evidence = a
            .evidence
            .iter()
            .map(|e| evidence_of(spine_dir, data, &row.repo, e))
            .collect();
    }
    Some(card)
}

/// 範圍內要你決定的卡。純讀取（沿用 [`relay::plan`] 的讀法：spine、registry、會話狀態檔、唯讀 git）。
pub fn list(spine_dir: &Path, scope: &RelayScope) -> Result<Queue> {
    list_at(spine_dir, scope, Local::now().naive_local())
}

/// 同 [`list`]，指定「現在」（測試用）。
pub fn list_at(spine_dir: &Path, scope: &RelayScope, now: NaiveDateTime) -> Result<Queue> {
    let plan = relay::plan_at(spine_dir, scope, now)?;
    let data = registry::load(spine_dir)?;
    let mut cards: Vec<Card> = plan
        .rows
        .iter()
        .filter_map(|r| card_of(spine_dir, r, &data, now, false))
        .collect();
    cards.sort_by(|a, b| {
        b.structured
            .cmp(&a.structured)
            .then_with(|| b.handoff_at.cmp(&a.handoff_at))
            .then_with(|| a.repo.cmp(&b.repo))
    });
    Ok(Queue {
        scope: plan.scope,
        at: plan.at,
        structured: cards.iter().filter(|c| c.structured).count(),
        cards,
        suggestions: suggestions(&history(spine_dir)?, &policy::load(spine_dir)),
    })
}

// ---------------------------------------------------------------- answer

fn rejected(code: RejectCode, msg: String, hint: String) -> Error {
    Error::rejected(code, msg, Some(hint))
}

/// 回覆 → (回答文字, 第幾個選項)。純數字且卡上有選項＝那個選項（超出範圍拒絕並列出選項）；其餘當自由文字。
fn resolve_reply(card: &Card, reply: &str) -> Result<(String, Option<usize>)> {
    let texts: Vec<String> = card.options.iter().map(|o| o.text.clone()).collect();
    if !texts.is_empty() && reply.chars().all(|c| c.is_ascii_digit()) {
        let n: usize = reply.parse().unwrap_or(0);
        return match texts.get(n.wrapping_sub(1)) {
            Some(t) => Ok((t.clone(), Some(n))),
            None => Err(rejected(
                RejectCode::BadRequest,
                format!("{} 的卡沒有第 {reply} 個選項", card.repo),
                format!("選項：{}；或直接寫一句話回答", numbered(&texts).join("；")),
            )),
        };
    }
    let n = texts.iter().position(|t| t == reply).map(|i| i + 1);
    Ok((reply.to_string(), n))
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 回答一張卡：留一筆痕（答了什麼、有沒有照建議），回傳怎麼把回答送到那個 repo 的 agent（只印不開會話）。
///
/// 送不出去的情況（路徑不存在、會話正在處理中…，見 [`relay::answer`]）先擋下，不會留了痕才發現。
/// 錯誤（都是 [`Error::Rejected`]）：回覆是空的、編號超出範圍（`bad_request`）；repo 沒登記、沒有要你決定的卡
/// （`not_found`）；會話正在處理中（`busy`）；剛好有新的交接蓋掉這張卡（`failed`，重看一次卡）。
pub fn answer(spine_dir: &Path, repo: &str, reply: &str) -> Result<Decided> {
    answer_with(spine_dir, repo, reply, None)
}

/// 同 [`answer`]，指定 Claude Code 逐字稿的根目錄（測試用；`None`＝`~/.claude/projects`）。
pub fn answer_with(
    spine_dir: &Path,
    repo: &str,
    reply: &str,
    projects_root: Option<&Path>,
) -> Result<Decided> {
    let reply = one_line(reply);
    if reply.is_empty() {
        return Err(rejected(
            RejectCode::BadRequest,
            "回覆是空的".into(),
            "寫選項編號或一句話，例：ctx decide answer <repo> 2".into(),
        ));
    }
    let data = registry::load(spine_dir)?;
    let Some(entry) = data.repo(repo) else {
        return Err(rejected(
            RejectCode::NotFound,
            format!("repo 沒登記：{repo}"),
            "用 ctx decide list --all 看要你決定的卡".into(),
        ));
    };
    let id = entry.id();
    let now = Local::now().naive_local();
    let plan = relay::plan_at(spine_dir, &RelayScope::Repos(vec![id.clone()]), now)?;
    let Some(card) = plan
        .rows
        .iter()
        .find(|r| r.repo == id)
        .and_then(|r| card_of(spine_dir, r, &data, now, true))
    else {
        return Err(rejected(
            RejectCode::NotFound,
            format!("{id} 沒有在等你決定的事（最後一筆交接沒在等你，或已經回答過）"),
            "用 ctx decide list --all 看要你決定的卡".into(),
        ));
    };
    let (answer_text, option) = resolve_reply(&card, &reply)?;
    // 先組送達方式：送不出去（busy、路徑不在…）就在這裡停下，還沒留痕
    let relayed = relay::answer_with(spine_dir, &id, &answer_text, projects_root)?;
    if relayed.handoff_ref != card.handoff_ref {
        return Err(rejected(
            RejectCode::Failed,
            format!(
                "{id} 剛有新的交接（{}），這張卡已經過時",
                relayed.handoff_ref
            ),
            "重跑 ctx decide list 看新的卡再回答".into(),
        ));
    }
    let answered = log::write_answer(
        spine_dir,
        &NewAnswer {
            repo: &id,
            handoff_ref: Some(&card.handoff_ref),
            question: &card.question,
            answer: &answer_text,
            recommend: card.recommend.as_deref(),
            kind: card.kind,
            reversible: card.reversible.unwrap_or(false),
            by: AnsweredBy::Human,
        },
        now,
    )?;
    // 這類有 auto 規則、人卻答得跟建議不同：規則降回每次都問（since 更新，連續計數從頭算）
    let mut downgraded = None;
    if let Some(k) = card.kind {
        let pol = policy::load(spine_dir);
        let is_auto = pol.rule(&id, k).is_some_and(|r| r.level == Level::Auto);
        if is_auto && answered.follow == FollowRec::Differed {
            downgraded = Some(policy::set_level(spine_dir, &id, k, Level::Ask, None, now)?);
        }
    }
    let suggestion = card.kind.and_then(|k| {
        let h = history(spine_dir).ok()?;
        suggestion_for(&h, &policy::load(spine_dir), &id, k)
    });
    Ok(Decided {
        answered,
        option,
        relay: relayed,
        downgraded,
        suggestion,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_suffix_strips_line_numbers() {
        let cap = |s: &str| {
            LINE_SUFFIX
                .captures(s)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_string())
        };
        assert_eq!(cap("src/a.rs:12").as_deref(), Some("src/a.rs"));
        assert_eq!(cap("r2:src/a.rs:12-30").as_deref(), Some("r2:src/a.rs"));
        assert_eq!(cap("r2:src/a.rs"), None);
        assert_eq!(cap("docs/plan.md"), None);
    }
}
