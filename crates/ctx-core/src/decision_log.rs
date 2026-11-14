//! 決定的留痕（F-123／F-124，Rust 才有）：人回答決策卡、規則自動放行、人推翻自動放行，三種都是 spine 裡的一筆
//! `chosen` 事件，第一行以 [`spine::DECIDE_TAG`] 開頭，最後一行是給程式讀的機器行。
//!
//! 這一層只依賴 spine 與 ask（不碰 relay、decide、agentapi），所以接力（`relay`）、交接（`agentapi::handoff`）、
//! 放行規則（`policy`）、決策佇列（`decide`）都能用它而不互相依賴。
//!
//! 格式（`chosen [<來源>] id:<日>-q<N> repo:<id> ref:<指向>`）：
//! ```text
//! 〔決定〕<repo>：<問題> → <回答>               ← 規則放行在後面加「（自動放行：<repo>/<kind>）」
//! 問題：<問題>
//! 回答：<回答>
//! 建議：<建議>                                  ← 沒建議不寫
//! decide kind:<kind|none> follow:<followed|differed|no_recommendation> by:<human|rule> rev:<yes|no>[ rule:<repo>/<kind>]
//! ```
//! 人回答：來源 `cli`、`ref:` 指向交接；規則放行：來源 `policy`、`ref:` 指向交接（agent 做到一半問的、還沒交接＝沒有 `ref:`）。
//! 推翻：來源 `cli`、`ref:` 指向那筆放行，第一行 `〔決定〕<repo>：推翻自動放行…`，機器行 `decide overturn kind:<kind> rule:<repo>/<kind>`。
//! 撤回（F-127）：同一套推翻留痕，`ref:` 指向那筆**人的**回答，第一行 `〔決定〕<repo>：撤回回答…`，
//! 機器行 `decide overturn of:human kind:<kind>`（`of:human`＝撤回人的回答；沒有＝推翻放行，舊紀錄都是這種）。
//! 撤回的回答不算數：不算回答（卡回到佇列）、不算連續、不算抽查、不算 health。
//!
//! 讀回寬鬆而保守：缺 `by:`＝人（第 1 輪只寫過人的回答）、缺 `rev:`＝不可逆（不算進連續照建議）、
//! 缺 `follow:`＝沒有建議、看不懂的類別＝None。

use std::collections::HashMap;
use std::path::Path;

use chrono::NaiveDateTime;
use serde::Serialize;

use crate::ask::AskKind;
use crate::spine::{self, Event, EventType, DECIDE_TAG};
use crate::Result;

/// 人回答的來源。
pub const SOURCE_HUMAN: &str = "cli";
/// 規則放行的來源。
pub const SOURCE_RULE: &str = "policy";
/// 留痕 id 的前綴（`<日>-q<N>`）。
const ID_PREFIX: &str = "q";
/// 機器行的開頭。
const MACHINE_HEAD: &str = "decide";
/// 第一行的問題、回答各最多幾個字（全文在後面兩行）。
const HEAD_CHARS: usize = 60;

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

/// 誰給的答案。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnsweredBy {
    /// 人用 `ctx decide answer` 回答
    Human,
    /// 放行規則自動照建議放行
    Rule,
}

impl AnsweredBy {
    pub fn as_str(self) -> &'static str {
        match self {
            AnsweredBy::Human => "human",
            AnsweredBy::Rule => "rule",
        }
    }
}

/// 一筆回答（人或規則）讀回來的樣子。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Answered {
    /// 留痕的 ref（`chosen:<日>-q<N>`）
    #[serde(rename = "ref")]
    pub reference: String,
    /// `YYYY-MM-DD HH:MM`
    pub at: String,
    pub repo: String,
    /// 回答的是哪筆交接（agent 做到一半問、規則當場放行的沒有）
    pub handoff_ref: Option<String>,
    /// 沒帶提問的卡＝None
    pub kind: Option<AskKind>,
    pub follow: FollowRec,
    pub by: AnsweredBy,
    /// 提問可逆（舊紀錄沒寫＝false）
    pub reversible: bool,
    /// 規則放行時是哪條規則（`<repo>/<kind>`）
    pub rule: Option<String>,
    pub question: String,
    pub answer: String,
    pub recommend: Option<String>,
    /// 被推翻了（推翻那筆留痕的 ref）；人的回答＝被撤回了
    pub overturned_by: Option<String>,
}

impl Answered {
    /// 人撤回了自己的回答：之後這筆不算數（連續、抽查、health 都不算；推翻的規則放行仍照原本算）。
    pub fn withdrawn(&self) -> bool {
        self.by == AnsweredBy::Human && self.overturned_by.is_some()
    }
}

/// 要寫的一筆回答。
#[derive(Debug, Clone)]
pub struct NewAnswer<'a> {
    pub repo: &'a str,
    pub handoff_ref: Option<&'a str>,
    pub question: &'a str,
    pub answer: &'a str,
    pub recommend: Option<&'a str>,
    pub kind: Option<AskKind>,
    pub reversible: bool,
    pub by: AnsweredBy,
}

/// 規則的名字（`<repo>/<kind>`；repo id 有空白時換成 `_`，機器行才切得開）。
pub fn rule_name(repo: &str, kind: AskKind) -> String {
    format!("{}/{}", spine::token_value(repo), kind.as_str())
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip(s: &str) -> String {
    crate::pystr::clip(s, HEAD_CHARS)
}

fn line_value<'a>(ev: &'a Event, key: &str) -> Option<&'a str> {
    crate::pystr::splitlines(&ev.body)
        .into_iter()
        .find_map(|ln| ln.strip_prefix(key))
        .map(str::trim)
}

/// 機器行的 token（`decide` 之後、空白分開）。
fn machine(ev: &Event) -> Vec<&str> {
    crate::pystr::splitlines(&ev.body)
        .into_iter()
        .find_map(|ln| ln.strip_prefix(MACHINE_HEAD).filter(|r| r.starts_with(' ')))
        .map(|r| r.split_whitespace().collect())
        .unwrap_or_default()
}

fn tok<'a>(m: &[&'a str], k: &str) -> Option<&'a str> {
    m.iter().find_map(|t| t.strip_prefix(k))
}

fn is_overturn(m: &[&str]) -> bool {
    m.first() == Some(&"overturn")
}

fn is_suggest(m: &[&str]) -> bool {
    m.first() == Some(&"suggest")
}

/// 一筆事件 → 回答；不是回答（不是留痕、是推翻）＝None。
fn answered_of(ev: &Event) -> Option<Answered> {
    if !spine::is_decide_record(ev) {
        return None;
    }
    let m = machine(ev);
    if is_overturn(&m) || is_suggest(&m) {
        return None;
    }
    let by = match tok(&m, "by:") {
        Some("rule") => AnsweredBy::Rule,
        _ => AnsweredBy::Human,
    };
    Some(Answered {
        reference: ev.reference(),
        at: ev.stamp(),
        repo: ev.kv("repo").unwrap_or("").to_string(),
        handoff_ref: ev.kv("ref").filter(|r| !r.is_empty()).map(str::to_string),
        kind: tok(&m, "kind:").and_then(AskKind::parse),
        follow: tok(&m, "follow:")
            .and_then(FollowRec::parse)
            .unwrap_or(FollowRec::NoRecommendation),
        by,
        reversible: tok(&m, "rev:") == Some("yes"),
        rule: tok(&m, "rule:").map(str::to_string),
        question: line_value(ev, "問題：").unwrap_or("").to_string(),
        answer: line_value(ev, "回答：").unwrap_or("").to_string(),
        recommend: line_value(ev, "建議：")
            .filter(|r| !r.is_empty())
            .map(str::to_string),
        overturned_by: None,
    })
}

/// 全部回答（人與規則，舊到新），被推翻的標出 `overturned_by`。
pub fn history(spine_dir: &Path) -> Result<Vec<Answered>> {
    Ok(history_of(&spine::iter_events(spine_dir)?))
}

/// 同 [`history`]，從已讀好的事件算。
pub fn history_of(events: &[Event]) -> Vec<Answered> {
    let overturns: HashMap<&str, String> = events
        .iter()
        .filter(|e| spine::is_decide_record(e) && is_overturn(&machine(e)))
        .filter_map(|e| e.kv("ref").map(|r| (r, e.reference())))
        .collect();
    events
        .iter()
        .filter_map(answered_of)
        .map(|mut a| {
            a.overturned_by = overturns.get(a.reference.as_str()).cloned();
            a
        })
        .collect()
}

/// 這筆交接的有效回答（同一個 repo、指向它、時間不早於它、沒被推翻——沒有 id 的 ref 只有時間，不同天會撞）；
/// 不只一筆取最後一筆。
pub fn answer_of<'a>(
    history: &'a [Answered],
    repo: &str,
    handoff_ref: &str,
    handoff_at: &str,
) -> Option<&'a Answered> {
    history.iter().rfind(|a| {
        a.overturned_by.is_none()
            && spine::same_token(&a.repo, repo)
            && a.handoff_ref.as_deref() == Some(handoff_ref)
            && a.at.as_str() >= handoff_at
    })
}

/// 同一個（repo、類別）最近連續照建議的次數：只看**人的**、沒撤回的回答、`since` 之後（嚴格晚於；None＝全部歷史）；
/// 只算可逆且有建議的；遇到跟建議不同就歸零。時間只到分鐘：跟 `since` 同一分鐘的不算（寧可少算）。
pub fn streak(history: &[Answered], repo: &str, kind: AskKind, since: Option<&str>) -> usize {
    let mut n = 0;
    for a in history.iter().filter(|a| {
        a.by == AnsweredBy::Human
            && !a.withdrawn()
            && a.kind == Some(kind)
            && spine::same_token(&a.repo, repo)
            && since.is_none_or(|s| a.at.as_str() > s)
    }) {
        match a.follow {
            FollowRec::Differed => n = 0,
            FollowRec::Followed if a.reversible => n += 1,
            _ => {}
        }
    }
    n
}

/// 一筆回答寫進 spine，回傳讀回來的樣子。人的回答來源 `cli`、規則放行來源 `policy`。
pub fn write_answer(spine_dir: &Path, a: &NewAnswer, now: NaiveDateTime) -> Result<Answered> {
    let question = one_line(a.question);
    let answer = one_line(a.answer);
    let follow = FollowRec::of(a.recommend, &answer);
    let rule = match (a.by, a.kind) {
        (AnsweredBy::Rule, Some(k)) => Some(rule_name(a.repo, k)),
        _ => None,
    };
    let mut head = format!(
        "{DECIDE_TAG}{}：{} → {}",
        a.repo,
        clip(&question),
        clip(&answer)
    );
    if let Some(r) = &rule {
        head.push_str(&format!("（自動放行：{r}）"));
    }
    let mut body = vec![head, format!("問題：{question}"), format!("回答：{answer}")];
    if let Some(r) = a.recommend {
        body.push(format!("建議：{}", one_line(r)));
    }
    let mut m = format!(
        "{MACHINE_HEAD} kind:{} follow:{} by:{} rev:{}",
        a.kind.map_or("none", AskKind::as_str),
        follow.as_str(),
        a.by.as_str(),
        if a.reversible { "yes" } else { "no" }
    );
    if let Some(r) = &rule {
        m.push_str(&format!(" rule:{r}"));
    }
    body.push(m);
    let mut tokens = vec![format!("repo:{}", a.repo)];
    if let Some(h) = a.handoff_ref {
        tokens.push(format!("ref:{h}"));
    }
    let source = match a.by {
        AnsweredBy::Human => SOURCE_HUMAN,
        AnsweredBy::Rule => SOURCE_RULE,
    };
    let ev = append(spine_dir, source, tokens, &body.join("\n"), now)?;
    answered_of(&ev).ok_or_else(|| crate::Error::invalid("回答留痕寫入後讀不回來"))
}

/// 卡上的建議是誰留的（F-125／F-126）：總管會話（cwd 在 spine），或某個登記 repo 的 agent（對別的 repo 的卡）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdviceFrom {
    Steward,
    Repo { repo: String },
}

impl AdviceFrom {
    /// 人看的名稱：「總管」／「<repo> 的 agent」。
    pub fn label(&self) -> String {
        match self {
            AdviceFrom::Steward => "總管".into(),
            AdviceFrom::Repo { repo } => format!("{repo} 的 agent"),
        }
    }
    /// 「總管建議」／「<repo> 的 agent 建議」。
    pub fn advice_label(&self) -> String {
        match self {
            AdviceFrom::Steward => "總管建議".into(),
            AdviceFrom::Repo { repo } => format!("{repo} 的 agent 建議"),
        }
    }
    fn token(&self) -> String {
        match self {
            AdviceFrom::Steward => "from:steward".into(),
            AdviceFrom::Repo { repo } => format!("from:repo:{}", spine::token_value(repo)),
        }
    }
    fn source(&self) -> &'static str {
        match self {
            AdviceFrom::Steward => SOURCE_STEWARD,
            AdviceFrom::Repo { .. } => "agent",
        }
    }
}

/// 卡上留的建議：不是回答、不算連續、不算抽查，只給人回答時參考。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Advice {
    /// 留痕的 ref
    #[serde(rename = "ref")]
    pub reference: String,
    pub at: String,
    pub repo: String,
    /// 指向哪筆交接
    pub handoff_ref: String,
    /// 誰留的
    pub from: AdviceFrom,
    /// 建議的答案（選項編號已換成選項文字）
    pub text: String,
    /// 建議的是第幾個選項
    pub option: Option<usize>,
    pub why: Option<String>,
}

/// 總管會話留的建議的事件來源。
pub const SOURCE_STEWARD: &str = "steward";

/// 全部建議（舊到新）。機器行缺 `from:`（第 3 輪的舊紀錄）＝總管。
pub fn advice_of(events: &[Event]) -> Vec<Advice> {
    events
        .iter()
        .filter(|e| spine::is_decide_record(e))
        .filter_map(|e| {
            let m = machine(e);
            if !is_suggest(&m) {
                return None;
            }
            let from = match tok(&m, "from:") {
                Some(x) => match x.strip_prefix("repo:") {
                    Some(r) if !r.is_empty() => AdviceFrom::Repo {
                        repo: r.to_string(),
                    },
                    _ => AdviceFrom::Steward,
                },
                None => AdviceFrom::Steward,
            };
            Some(Advice {
                reference: e.reference(),
                at: e.stamp(),
                repo: e.kv("repo").unwrap_or("").to_string(),
                handoff_ref: e.kv("ref").filter(|r| !r.is_empty())?.to_string(),
                from,
                text: line_value(e, "建議：").unwrap_or("").to_string(),
                option: tok(&m, "option:").and_then(|n| n.parse().ok()),
                why: line_value(e, "理由：")
                    .filter(|w| !w.is_empty())
                    .map(str::to_string),
            })
        })
        .collect()
}

/// 這筆交接最新的建議（時間不早於交接）。
pub fn advice_for<'a>(
    all: &'a [Advice],
    repo: &str,
    handoff_ref: &str,
    handoff_at: &str,
) -> Option<&'a Advice> {
    all.iter().rfind(|n| {
        spine::same_token(&n.repo, repo)
            && n.handoff_ref == handoff_ref
            && n.at.as_str() >= handoff_at
    })
}

/// 寫一筆建議（指向交接）。回傳留痕的 ref。
#[allow(clippy::too_many_arguments)]
pub fn write_advice(
    spine_dir: &Path,
    repo: &str,
    handoff_ref: &str,
    from: &AdviceFrom,
    text: &str,
    option: Option<usize>,
    why: Option<&str>,
    now: NaiveDateTime,
) -> Result<String> {
    let text = one_line(text);
    let mut body = vec![
        format!(
            "{DECIDE_TAG}{repo}：{}「{}」",
            from.advice_label(),
            clip(&text)
        ),
        format!("建議：{text}"),
    ];
    if let Some(w) = why.map(one_line).filter(|w| !w.is_empty()) {
        body.push(format!("理由：{w}"));
    }
    let mut m = format!("{MACHINE_HEAD} suggest {}", from.token());
    if let Some(n) = option {
        m.push_str(&format!(" option:{n}"));
    }
    body.push(m);
    let tokens = vec![format!("repo:{repo}"), format!("ref:{handoff_ref}")];
    Ok(append(spine_dir, from.source(), tokens, &body.join("\n"), now)?.reference())
}

/// 推翻一筆規則放行的留痕（指向那筆放行）。回傳推翻那筆的 ref。
pub fn write_overturn(
    spine_dir: &Path,
    approval: &Answered,
    why: Option<&str>,
    now: NaiveDateTime,
) -> Result<String> {
    let mut body = vec![format!(
        "{DECIDE_TAG}{}：推翻自動放行「{} → {}」",
        approval.repo,
        clip(&approval.question),
        clip(&approval.answer)
    )];
    if let Some(w) = why.map(one_line).filter(|w| !w.is_empty()) {
        body.push(format!("理由：{w}"));
    }
    let mut m = format!(
        "{MACHINE_HEAD} overturn kind:{}",
        approval.kind.map_or("none", AskKind::as_str)
    );
    if let Some(r) = &approval.rule {
        m.push_str(&format!(" rule:{r}"));
    }
    body.push(m);
    let tokens = vec![
        format!("repo:{}", approval.repo),
        format!("ref:{}", approval.reference),
    ];
    Ok(append(spine_dir, SOURCE_HUMAN, tokens, &body.join("\n"), now)?.reference())
}

/// 撤回一筆人的回答（同推翻的留痕，機器行帶 `of:human`）。回傳撤回那筆的 ref。
pub fn write_withdraw(
    spine_dir: &Path,
    answer: &Answered,
    why: Option<&str>,
    now: NaiveDateTime,
) -> Result<String> {
    let mut body = vec![format!(
        "{DECIDE_TAG}{}：撤回回答「{} → {}」",
        answer.repo,
        clip(&answer.question),
        clip(&answer.answer)
    )];
    if let Some(w) = why.map(one_line).filter(|w| !w.is_empty()) {
        body.push(format!("理由：{w}"));
    }
    body.push(format!(
        "{MACHINE_HEAD} overturn of:human kind:{}",
        answer.kind.map_or("none", AskKind::as_str)
    ));
    let tokens = vec![
        format!("repo:{}", answer.repo),
        format!("ref:{}", answer.reference),
    ];
    Ok(append(spine_dir, SOURCE_HUMAN, tokens, &body.join("\n"), now)?.reference())
}

fn append(
    spine_dir: &Path,
    source: &str,
    mut tokens: Vec<String>,
    body: &str,
    now: NaiveDateTime,
) -> Result<Event> {
    let events = spine::iter_events(spine_dir)?;
    let eid = spine::next_event_id(&events, &now.format("%Y-%m-%d").to_string(), ID_PREFIX);
    tokens.insert(0, format!("id:{eid}"));
    spine::append_event(
        spine_dir,
        EventType::Chosen.as_str(),
        source,
        &tokens,
        body,
        Some(now),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(body: &str, toks: &[&str]) -> Event {
        Event {
            date: "2026-11-14".into(),
            time: "10:00".into(),
            etype: EventType::Chosen,
            source: "cli".into(),
            tokens: toks.iter().map(|t| t.to_string()).collect(),
            body: body.into(),
        }
    }

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
    fn answered_of_is_lenient_and_conservative() {
        assert!(answered_of(&mk("忽略並記錄：x", &["ref:decision:09:00"])).is_none());
        // 第 1 輪的舊紀錄：沒有 by／rev → 人、不可逆
        let a = answered_of(&mk(
            "〔決定〕r：q → B\n問題：q\n回答：B\n建議：A\ndecide kind:commit follow:differed",
            &["id:2026-11-14-q1", "repo:r", "ref:decision:2026-11-14-d1"],
        ))
        .unwrap();
        assert_eq!(a.reference, "chosen:2026-11-14-q1");
        assert_eq!(a.by, AnsweredBy::Human);
        assert!(!a.reversible);
        assert_eq!(a.kind, Some(AskKind::Commit));
        assert_eq!(a.follow, FollowRec::Differed);
        assert_eq!(a.handoff_ref.as_deref(), Some("decision:2026-11-14-d1"));
        // 機器行不見了
        let b = answered_of(&mk("〔決定〕r：q → a\n回答：a", &["repo:r"])).unwrap();
        assert_eq!(
            (b.kind, b.follow, b.handoff_ref),
            (None, FollowRec::NoRecommendation, None)
        );
        // 規則放行
        let c = answered_of(&mk(
            "〔決定〕r：q → A（自動放行：r/commit）\ndecide kind:commit follow:followed by:rule rev:yes rule:r/commit",
            &["repo:r"],
        ))
        .unwrap();
        assert_eq!(c.by, AnsweredBy::Rule);
        assert_eq!(c.rule.as_deref(), Some("r/commit"));
        // 推翻不是回答；「decided」這種字開頭的行不是機器行
        assert!(answered_of(&mk(
            "〔決定〕r：推翻\ndecide overturn kind:commit",
            &["repo:r"]
        ))
        .is_none());
        let d = answered_of(&mk("〔決定〕r：q → a\ndecided by:rule", &["repo:r"])).unwrap();
        assert_eq!(d.by, AnsweredBy::Human);
    }

    #[test]
    fn streak_counts_human_followed_and_resets() {
        let a = |at: &str, follow: FollowRec, by: AnsweredBy, rev: bool| Answered {
            reference: format!("chosen:{at}"),
            at: at.into(),
            repo: "r".into(),
            handoff_ref: None,
            kind: Some(AskKind::Commit),
            follow,
            by,
            reversible: rev,
            rule: None,
            question: String::new(),
            answer: String::new(),
            recommend: None,
            overturned_by: None,
        };
        use AnsweredBy::*;
        use FollowRec::*;
        let h = vec![
            a("2026-11-01 10:00", Followed, Human, true),
            a("2026-11-01 10:01", Differed, Human, true),
            a("2026-11-01 10:02", Followed, Human, true),
            a("2026-11-01 10:03", Followed, Rule, true),
            a("2026-11-01 10:04", Followed, Human, false),
            a("2026-11-01 10:05", NoRecommendation, Human, true),
            a("2026-11-01 10:06", Followed, Human, true),
        ];
        assert_eq!(streak(&h, "r", AskKind::Commit, None), 2);
        assert_eq!(
            streak(&h, "r", AskKind::Commit, Some("2026-11-01 10:02")),
            1
        );
        assert_eq!(streak(&h, "r", AskKind::Push, None), 0);
        assert_eq!(streak(&h, "other", AskKind::Commit, None), 0);
    }
}
