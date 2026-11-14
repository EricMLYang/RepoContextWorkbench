//! 決策佇列（F-123 第 1 輪，2026-11-14，Rust 才有）：把各 repo「在等你決定」的事排成一疊卡，一張一張答。
//!
//! 為什麼要有：agent 收尾時常停在「等你決定／確認／驗收」，這些事散在二十幾個 repo 的交接裡；人要一個一個
//! 回想那個 repo 在做什麼、問題是什麼、該怎麼回。佇列把它們收成卡（repo 一行介紹、問題、編號選項、建議、證據、
//! 放了多久），回答時用編號就好，並留一筆痕（答了什麼、有沒有照建議），第 2 輪的放行規則從這些痕算統計。
//!
//! 判斷都沿用既有的：
//! - 每個 repo 現在怎樣（路徑在不在、有沒有會話正在處理、下一步是不是在等你）：[`relay::plan_at`]
//! - 交接的欄位與提問：[`crate::agentapi::parse_handoff`]／[`crate::ask`]
//! - 回覆怎麼送到 agent：[`relay::answer_with`]（只印不開會話）
//!
//! 卡＝範圍內每個 repo 的**最後一筆交接**，還沒被回答、而且：帶了提問（`structured`），或沒帶提問但接力判斷
//! 下一步在等你（[`relay::PlanRow::waits_on_you`]）。路徑不存在、有會話正在處理中的不列。
//! 「已回答」＝有一筆回答留痕（[`spine::DECIDE_TAG`]）指向那筆交接、而且時間不早於它；新的交接自然蓋掉舊的。
//!
//! 回答留痕的格式（`chosen [cli] id:<日>-q<N> repo:<id> ref:<交接 ref>`）：
//! ```text
//! 〔決定〕<repo>：<問題> → <回答>
//! 問題：<問題>
//! 回答：<回答>
//! 建議：<建議>                ← 沒建議不寫這行
//! decide kind:<類別|none> follow:<followed|differed|no_recommendation>
//! ```
//! 最後一行是給程式讀的（[`history`]），不用解析自由文字就算得出「同一個 repo＋類別連續幾次照建議」。
//! 類別與照不照建議放在 body 而不是標頭 token：標頭的 token 文法（`#N`／`group:`／`repo:`／`ref:`／`due:`／`id:`）
//! 是跟 Python 版與舊版 `ctx` 共用的合約，多一種鍵會讓舊版讀整份 spine 失敗。

use std::path::Path;
use std::sync::LazyLock;

use chrono::{Local, NaiveDateTime};
use regex::Regex;
use serde::Serialize;

use crate::ask::{numbered, AskKind};
use crate::registry::{self, RegistryData};
use crate::relay::{self, PlanRow, RelayScope, RelayStatus};
use crate::spine::{self, Event, EventType, DECIDE_TAG};
use crate::{agentapi, Error, RejectCode, Result};

/// 回答留痕的事件來源（使用者在終端下的指令）。
pub const SOURCE: &str = "cli";
/// 回答留痕的 id 前綴（`<日>-q<N>`）。
const ID_PREFIX: &str = "q";
/// 機器讀的那一行的開頭。
const MACHINE_HEAD: &str = "decide ";
/// 留痕第一行的問題、回答各最多幾個字（全文在後面兩行）。
const HEAD_CHARS: usize = 60;

// ---------------------------------------------------------------- 型別

/// 有沒有照建議回答。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowRec {
    /// 回答就是建議的那個
    Followed,
    /// 有建議，但回答不一樣
    Differed,
    /// 卡上沒有建議
    NoRecommendation,
}

impl FollowRec {
    pub fn as_str(self) -> &'static str {
        match self {
            FollowRec::Followed => "followed",
            FollowRec::Differed => "differed",
            FollowRec::NoRecommendation => "no_recommendation",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FollowRec::Followed => "照建議",
            FollowRec::Differed => "跟建議不同",
            FollowRec::NoRecommendation => "沒有建議",
        }
    }

    fn parse(s: &str) -> Option<FollowRec> {
        [
            FollowRec::Followed,
            FollowRec::Differed,
            FollowRec::NoRecommendation,
        ]
        .into_iter()
        .find(|f| f.as_str() == s)
    }

    /// 回答跟建議比（去頭尾空白後完全相同才算照建議）。
    pub fn of(recommend: Option<&str>, answer: &str) -> FollowRec {
        match recommend {
            None => FollowRec::NoRecommendation,
            Some(r) if r.trim() == answer.trim() => FollowRec::Followed,
            Some(_) => FollowRec::Differed,
        }
    }
}

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
}

/// 一筆回答留痕讀回來的樣子（[`history`]）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Answered {
    /// 留痕的 ref（`chosen:<日>-q<N>`）
    #[serde(rename = "ref")]
    pub reference: String,
    /// `YYYY-MM-DD HH:MM`
    pub at: String,
    pub repo: String,
    /// 回答的是哪筆交接
    pub handoff_ref: String,
    /// 沒帶提問的卡＝None
    pub kind: Option<AskKind>,
    pub follow: FollowRec,
    pub question: String,
    pub answer: String,
    pub recommend: Option<String>,
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
}

// ---------------------------------------------------------------- 讀留痕

fn line_value<'a>(ev: &'a Event, key: &str) -> Option<&'a str> {
    crate::pystr::splitlines(&ev.body)
        .into_iter()
        .find_map(|ln| ln.strip_prefix(key))
        .map(str::trim)
}

/// 一筆事件 → 回答留痕；不是留痕、或沒有指向交接的 ref＝None。
/// 寬鬆：機器行缺了或看不懂時，類別＝None、照不照建議＝沒有建議（第 2 輪的連續計數因此會斷，不會多算）。
fn answered_of(ev: &Event) -> Option<Answered> {
    if !spine::is_decide_record(ev) {
        return None;
    }
    let handoff_ref = ev.kv("ref").filter(|r| !r.is_empty())?.to_string();
    let machine = line_value(ev, MACHINE_HEAD.trim_end()).unwrap_or("");
    let tok = |k: &str| {
        machine
            .split_whitespace()
            .find_map(|t| t.strip_prefix(k).map(str::to_string))
    };
    Some(Answered {
        reference: agentapi::event_ref(ev),
        at: ev.stamp(),
        repo: ev.kv("repo").unwrap_or("").to_string(),
        handoff_ref,
        kind: tok("kind:").and_then(|k| AskKind::parse(&k)),
        follow: tok("follow:")
            .and_then(|f| FollowRec::parse(&f))
            .unwrap_or(FollowRec::NoRecommendation),
        question: line_value(ev, "問題：").unwrap_or("").to_string(),
        answer: line_value(ev, "回答：").unwrap_or("").to_string(),
        recommend: line_value(ev, "建議：")
            .filter(|r| !r.is_empty())
            .map(str::to_string),
    })
}

/// 全部回答留痕（舊到新）。
pub fn history(spine_dir: &Path) -> Result<Vec<Answered>> {
    Ok(history_of(&spine::iter_events(spine_dir)?))
}

/// 同 [`history`]，從已讀好的事件算。
pub fn history_of(events: &[Event]) -> Vec<Answered> {
    events.iter().filter_map(answered_of).collect()
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
/// 這筆交接的回答（同一個 repo、指向它、時間不早於它——沒有 id 的 ref 只有時間，不同天會撞）；
/// 答過不只一次取最後一次。接力（[`relay::plan_at`]）也用它：答過的交接不再算「等你」。
pub fn answer_of<'a>(
    history: &'a [Answered],
    repo: &str,
    handoff_ref: &str,
    handoff_at: &str,
) -> Option<&'a Answered> {
    history.iter().rfind(|a| {
        spine::same_token(&a.repo, repo)
            && a.handoff_ref == handoff_ref
            && a.at.as_str() >= handoff_at
    })
}

/// plan 的一列 → 卡；不是卡＝None。`include_blocked`＝路徑不存在、會話處理中的也出卡
/// （`answer` 用：讓 [`relay::answer_with`] 說出為什麼送不出去，而不是說「沒有卡」）。
fn card_of(
    spine_dir: &Path,
    row: &PlanRow,
    data: &RegistryData,
    history: &[Answered],
    now: NaiveDateTime,
    include_blocked: bool,
) -> Option<Card> {
    let blocked = matches!(row.status, RelayStatus::Missing | RelayStatus::Busy);
    if blocked && !include_blocked {
        return None;
    }
    let (handoff_ref, handoff_at) = (row.handoff_ref.as_ref()?, row.handoff_at.as_ref()?);
    let waiting = row.ask.is_some()
        || row.waits_on_you
        || (blocked && row.next_step.as_deref().is_some_and(relay::waits_on_you));
    if !waiting || answer_of(history, &row.repo, handoff_ref, handoff_at).is_some() {
        return None;
    }
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
    let history = history(spine_dir)?;
    let data = registry::load(spine_dir)?;
    let mut cards: Vec<Card> = plan
        .rows
        .iter()
        .filter_map(|r| card_of(spine_dir, r, &data, &history, now, false))
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

fn clip(s: &str) -> String {
    crate::pystr::clip(s, HEAD_CHARS)
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
    let events = spine::iter_events(spine_dir)?;
    let history = history_of(&events);
    let Some(card) = plan
        .rows
        .iter()
        .find(|r| r.repo == id)
        .and_then(|r| card_of(spine_dir, r, &data, &history, now, true))
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
    let follow = FollowRec::of(card.recommend.as_deref(), &answer_text);
    let mut body = vec![
        format!(
            "{DECIDE_TAG}{id}：{} → {}",
            clip(&card.question),
            clip(&answer_text)
        ),
        format!("問題：{}", one_line(&card.question)),
        format!("回答：{answer_text}"),
    ];
    if let Some(r) = &card.recommend {
        body.push(format!("建議：{r}"));
    }
    body.push(format!(
        "{MACHINE_HEAD}kind:{} follow:{}",
        card.kind.map_or("none", AskKind::as_str),
        follow.as_str()
    ));
    let eid = agentapi::next_id(&events, &now.format("%Y-%m-%d").to_string(), ID_PREFIX);
    let tokens = [
        format!("id:{eid}"),
        format!("repo:{id}"),
        format!("ref:{}", card.handoff_ref),
    ];
    let ev = spine::append_event(
        spine_dir,
        EventType::Chosen.as_str(),
        SOURCE,
        &tokens,
        &body.join("\n"),
        None,
    )?;
    let answered = answered_of(&ev).ok_or_else(|| Error::invalid("回答留痕寫入後讀不回來"))?;
    Ok(Decided {
        answered,
        option,
        relay: relayed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_rec_of() {
        assert_eq!(FollowRec::of(None, "x"), FollowRec::NoRecommendation);
        assert_eq!(FollowRec::of(Some(" A "), "A"), FollowRec::Followed);
        assert_eq!(FollowRec::of(Some("A"), "B"), FollowRec::Differed);
        for f in [
            FollowRec::Followed,
            FollowRec::Differed,
            FollowRec::NoRecommendation,
        ] {
            assert_eq!(FollowRec::parse(f.as_str()), Some(f));
            assert_eq!(serde_json::to_value(f).unwrap(), f.as_str());
        }
    }

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

    #[test]
    fn answered_of_is_lenient() {
        let mk = |body: &str, toks: &[&str]| Event {
            date: "2026-11-14".into(),
            time: "10:00".into(),
            etype: EventType::Chosen,
            source: "cli".into(),
            tokens: toks.iter().map(|t| t.to_string()).collect(),
            body: body.into(),
        };
        // 不是留痕
        assert!(answered_of(&mk("忽略並記錄：x", &["ref:decision:09:00"])).is_none());
        // 沒有 ref
        assert!(answered_of(&mk("〔決定〕r：q → a", &["repo:r"])).is_none());
        // 機器行不見了（人手改過）
        let a = answered_of(&mk(
            "〔決定〕r：q → a\n回答：a",
            &["repo:r", "ref:decision:09:00"],
        ))
        .unwrap();
        assert_eq!(a.kind, None);
        assert_eq!(a.follow, FollowRec::NoRecommendation);
        assert_eq!(a.answer, "a");
        // 完整
        let b = answered_of(&mk(
            "〔決定〕r：q → B\n問題：q\n回答：B\n建議：A\ndecide kind:commit follow:differed",
            &["id:2026-11-14-q1", "repo:r", "ref:decision:2026-11-14-d1"],
        ))
        .unwrap();
        assert_eq!(b.reference, "chosen:2026-11-14-q1");
        assert_eq!(b.kind, Some(AskKind::Commit));
        assert_eq!(b.follow, FollowRec::Differed);
        assert_eq!(b.recommend.as_deref(), Some("A"));
        assert_eq!(b.handoff_ref, "decision:2026-11-14-d1");
    }
}
