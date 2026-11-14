//! 放行規則的人用入口（F-124）：agent 做到一半問（[`request`]）、人升降級（[`trust`]／[`untrust`]）、
//! 推翻一筆自動放行（[`overturn`]）、看規則（[`rules_report`]）與最近的自動放行（[`auto_list`]）、
//! 健康指標的「決策」段（[`health_stats`]）。判定本身在 [`crate::policy::gate`]。

use std::collections::HashMap;

use super::*;
use crate::agentapi::{self, Target};
use crate::ask::AskInput;
use crate::policy::{Decision, Policy};

/// 連續照建議幾次以上出升級建議。
pub const TRUST_STREAK: usize = 3;
/// `decide list --auto` 預設看幾天。
pub const AUTO_LIST_DAYS: i64 = 7;

/// 升級建議：這個（repo、類別）連續照建議回答了 `streak` 次。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrustHint {
    pub repo: String,
    pub kind: AskKind,
    pub streak: usize,
    /// 可直接複製的指令
    pub command: String,
}

impl TrustHint {
    /// 給人看的一句話。
    pub fn line(&self) -> String {
        format!(
            "{}／{}：連續 {} 次照建議回答——以後這類都照建議自動放行？{}",
            self.repo,
            self.kind.label(),
            self.streak,
            self.command
        )
    }
}

/// 這個（repo、類別）現在該不該出升級建議：類別可自動、目前不是 auto、規則 `since` 之後（沒規則＝全部歷史）
/// 人的回答連續照建議 ≥ [`TRUST_STREAK`]。
pub fn suggestion_for(
    history: &[Answered],
    pol: &Policy,
    repo: &str,
    kind: AskKind,
) -> Option<TrustHint> {
    if !kind.auto_approvable() {
        return None;
    }
    let rule = pol.rule(repo, kind);
    if rule.is_some_and(|r| r.level == Level::Auto) {
        return None;
    }
    let n = log::streak(history, repo, kind, rule.map(|r| r.since.as_str()));
    (n >= TRUST_STREAK).then(|| TrustHint {
        repo: repo.to_string(),
        kind,
        streak: n,
        command: format!(
            "ctx decide trust {} {}",
            crate::term::shlex_quote(repo),
            kind.as_str()
        ),
    })
}

/// 所有該出的升級建議（照 repo、類別排）。
pub fn suggestions(history: &[Answered], pol: &Policy) -> Vec<TrustHint> {
    let mut pairs: Vec<(String, AskKind)> = Vec::new();
    for a in history.iter().filter(|a| a.by == AnsweredBy::Human) {
        if let Some(k) = a.kind {
            if !pairs
                .iter()
                .any(|(r, kk)| *kk == k && spine::same_token(r, &a.repo))
            {
                pairs.push((a.repo.clone(), k));
            }
        }
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.as_str().cmp(b.1.as_str())));
    pairs
        .iter()
        .filter_map(|(r, k)| suggestion_for(history, pol, r, *k))
        .collect()
}

// ---------------------------------------------------------------- request（agent 做到一半問）

/// agent 做到一半需要人決定時先問：放行＝留痕、回答案；不放行＝不留痕、回原因（agent 能做的先做完，收尾交接帶同一個提問）。
/// 寫入範圍沿用 agent 寫入工具的檢查（跨組要使用者同意）。
pub fn request(
    spine_dir: &Path,
    t: &Target,
    input: &AskInput,
    cross_scope_ok: bool,
) -> Result<Decision> {
    let repo = agentapi::write_repo(spine_dir, t, cross_scope_ok)?.ok_or_else(|| {
        Error::rejected(
            RejectCode::BadRequest,
            "不在登記的 repo 裡，不知道是哪個 repo 的提問",
            Some("在 repo 目錄裡呼叫，或帶 cwd／repos 指定一個 repo".into()),
        )
    })?;
    let ask = input.validate()?;
    policy::approve(spine_dir, &repo, &ask, None, Local::now().naive_local())
}

// ---------------------------------------------------------------- trust／untrust

/// 這類過去的回答。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PastAnswers {
    /// 人回答過幾次
    pub answers: usize,
    pub followed: usize,
    pub differed: usize,
    /// 規則 since 之後（沒規則＝全部）的連續照建議
    pub streak: usize,
}

fn past(history: &[Answered], pol: &Policy, repo: &str, kind: AskKind) -> PastAnswers {
    let mine: Vec<&Answered> = history
        .iter()
        .filter(|a| {
            a.by == AnsweredBy::Human && a.kind == Some(kind) && spine::same_token(&a.repo, repo)
        })
        .collect();
    PastAnswers {
        answers: mine.len(),
        followed: mine
            .iter()
            .filter(|a| a.follow == FollowRec::Followed)
            .count(),
        differed: mine
            .iter()
            .filter(|a| a.follow == FollowRec::Differed)
            .count(),
        streak: log::streak(
            history,
            repo,
            kind,
            pol.rule(repo, kind).map(|r| r.since.as_str()),
        ),
    }
}

/// 改等級的結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LevelChanged {
    pub rule: Rule,
    /// 改之前的等級（沒有規則＝None）
    pub previous: Option<Level>,
    /// 這類過去的回答（改之前算的）
    pub past: PastAnswers,
}

fn registered(spine_dir: &Path, repo: &str) -> Result<String> {
    let data = registry::load(spine_dir)?;
    data.repo(repo).map(|r| r.id()).ok_or_else(|| {
        Error::rejected(
            RejectCode::NotFound,
            format!("repo 沒登記：{repo}"),
            Some("用 ctx registry list 看登記的 repo id".into()),
        )
    })
}

fn kind_of(kind: &str) -> Result<AskKind> {
    AskKind::parse(kind).ok_or_else(|| {
        Error::rejected(
            RejectCode::BadRequest,
            format!("類別看不懂：「{kind}」"),
            Some(format!("用這幾個之一：{}", AskKind::NAMES.join("、"))),
        )
    })
}

fn change(
    spine_dir: &Path,
    repo: &str,
    kind: AskKind,
    level: Level,
    paths: Option<Vec<String>>,
) -> Result<LevelChanged> {
    let h = history(spine_dir)?;
    let pol = policy::load(spine_dir);
    let previous = pol.rule(repo, kind).map(|r| r.level);
    let past = past(&h, &pol, repo, kind);
    let rule = policy::set_level(
        spine_dir,
        repo,
        kind,
        level,
        paths,
        Local::now().naive_local(),
    )?;
    Ok(LevelChanged {
        rule,
        previous,
        past,
    })
}

/// 人明確同意：這個（repo、類別）以後照建議自動放行。不要求先有連續紀錄（人下的指令就是同意）。
/// 類別不能自動時拒絕。`paths`＝None 保留原本的、Some(空)＝清掉。
pub fn trust(
    spine_dir: &Path,
    repo: &str,
    kind: &str,
    paths: Option<Vec<String>>,
) -> Result<LevelChanged> {
    let id = registered(spine_dir, repo)?;
    let k = kind_of(kind)?;
    if !k.auto_approvable() {
        return Err(Error::rejected(
            RejectCode::BadRequest,
            format!(
                "「{}」類永遠要人回答，不能設成自動（寫死在引擎：push、publish、delete、send_external、visual_check、manual_test、other）",
                k.label()
            ),
            None,
        ));
    }
    if let Some(ps) = &paths {
        if let Some(bad) = ps.iter().find(|p| {
            let p = p.trim();
            p.is_empty()
                || p.starts_with('/')
                || p.starts_with('~')
                || p.split('/').any(|s| s == "..")
        }) {
            return Err(Error::rejected(
                RejectCode::BadRequest,
                format!("paths 要是 repo 內的相對路徑：「{bad}」"),
                Some("例：--paths docs/,notes/".into()),
            ));
        }
    }
    change(spine_dir, &id, k, Level::Auto, paths)
}

/// 降回每次都問。沒有規則時也寫一條 ask（之後的連續計數從現在算）。
pub fn untrust(spine_dir: &Path, repo: &str, kind: &str) -> Result<LevelChanged> {
    let id = registered(spine_dir, repo)?;
    let k = kind_of(kind)?;
    change(spine_dir, &id, k, Level::Ask, None)
}

// ---------------------------------------------------------------- overturn

/// 推翻的結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Overturned {
    /// 推翻這筆留痕的 ref
    #[serde(rename = "ref")]
    pub reference: String,
    /// 被推翻的那筆放行
    pub approval: Answered,
    /// 降回 ask 的規則（規則檔讀不懂、沒降成＝None，看 `note`）
    pub rule: Option<Rule>,
    pub note: Option<String>,
}

/// 推翻一筆自動放行：留一筆痕指向它、把那條規則降回 ask。那筆放行之後不算回答（交接的卡會回到佇列）。
pub fn overturn(spine_dir: &Path, reference: &str, why: Option<&str>) -> Result<Overturned> {
    let h = history(spine_dir)?;
    let Some(a) = h.iter().find(|a| a.reference == reference.trim()) else {
        return Err(Error::rejected(
            RejectCode::NotFound,
            format!("找不到這筆放行：{reference}"),
            Some("用 ctx decide list --auto 看最近的自動放行與它們的 ref".into()),
        ));
    };
    if a.by != AnsweredBy::Rule {
        return Err(Error::rejected(
            RejectCode::BadRequest,
            format!("{reference} 是人回答的，不是自動放行"),
            Some("要改答案就再交接一次、或直接跟那個 repo 的 agent 說".into()),
        ));
    }
    if let Some(by) = &a.overturned_by {
        return Err(Error::rejected(
            RejectCode::BadRequest,
            format!("{reference} 已經被推翻過了（{by}）"),
            None,
        ));
    }
    let now = Local::now().naive_local();
    let r = log::write_overturn(spine_dir, a, why, now)?;
    let (rule, note) = match a.kind {
        None => (None, Some("這筆沒有類別，沒有規則可降".to_string())),
        Some(k) => match policy::set_level(spine_dir, &a.repo, k, Level::Ask, None, now) {
            Ok(rule) => (Some(rule), None),
            Err(e) => (None, Some(format!("規則沒降成：{e}"))),
        },
    };
    let mut approval = a.clone();
    approval.overturned_by = Some(r.clone());
    Ok(Overturned {
        reference: r,
        approval,
        rule,
        note,
    })
}

// ---------------------------------------------------------------- rules／auto list

/// `decide rules` 的一列。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleRow {
    #[serde(flatten)]
    pub rule: Rule,
    /// since 之後自動放行幾次
    pub approvals: usize,
    /// since 之後的放行裡被推翻幾次
    pub overturned: usize,
}

/// `decide rules` 的結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RulesReport {
    /// policy.yaml 的路徑
    pub file: String,
    pub exists: bool,
    pub rules: Vec<RuleRow>,
    pub suggestions: Vec<TrustHint>,
    /// policy.yaml 哪裡寫壞了
    pub problems: Vec<String>,
}

fn approvals_of<'a>(
    h: &'a [Answered],
    repo: &str,
    kind: AskKind,
    since: &str,
) -> Vec<&'a Answered> {
    h.iter()
        .filter(|a| {
            a.by == AnsweredBy::Rule
                && a.kind == Some(kind)
                && spine::same_token(&a.repo, repo)
                && a.at.as_str() >= since
        })
        .collect()
}

/// 規則一覽：每條規則（等級、since、paths、since 之後放行與被推翻幾次）＋升級建議＋policy.yaml 的問題。
pub fn rules_report(spine_dir: &Path) -> Result<RulesReport> {
    let h = history(spine_dir)?;
    let pol = policy::load(spine_dir);
    let rows = pol
        .rules
        .iter()
        .map(|r| {
            let ap = approvals_of(&h, &r.repo, r.kind, &r.since);
            RuleRow {
                rule: r.clone(),
                approvals: ap.len(),
                overturned: ap.iter().filter(|a| a.overturned_by.is_some()).count(),
            }
        })
        .collect();
    let p = policy::path(spine_dir);
    Ok(RulesReport {
        file: p.to_string_lossy().into_owned(),
        exists: p.exists(),
        rules: rows,
        suggestions: suggestions(&h, &pol),
        problems: pol.problems,
    })
}

/// 最近的一筆自動放行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AutoItem {
    #[serde(flatten)]
    pub approval: Answered,
    /// 推翻用的指令（已推翻＝None）
    pub command: Option<String>,
}

/// `decide list --auto` 的結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AutoList {
    /// `YYYY-MM-DD HH:MM`
    pub since: String,
    /// 新的在前
    pub items: Vec<AutoItem>,
}

/// `since` 之後的自動放行（None＝最近 [`AUTO_LIST_DAYS`] 天）。
pub fn auto_list(spine_dir: &Path, since: Option<&str>) -> Result<AutoList> {
    let since = match since {
        Some(s) => relay::parse_since(s)?,
        None => (Local::now().naive_local() - chrono::Duration::days(AUTO_LIST_DAYS))
            .format("%Y-%m-%d %H:%M")
            .to_string(),
    };
    let items = history(spine_dir)?
        .into_iter()
        .rev()
        .filter(|a| a.by == AnsweredBy::Rule && a.at >= since)
        .map(|a| AutoItem {
            command: a
                .overturned_by
                .is_none()
                .then(|| format!("ctx decide overturn {} --why \"…\"", a.reference)),
            approval: a,
        })
        .collect();
    Ok(AutoList { since, items })
}

// ---------------------------------------------------------------- health

/// `ctx health` 的「決策」段。比例與時間沒資料＝None（不要印 0）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecideHealth {
    pub days: i64,
    /// 現在要你決定幾件（同 `ctx decide list --all`）
    pub waiting: usize,
    /// 期間人回答幾次
    pub answered: usize,
    /// 其中有建議的幾次、照建議幾次
    pub with_recommend: usize,
    pub followed: usize,
    pub followed_rate: Option<f64>,
    /// 期間自動放行幾次、其中被推翻幾次
    pub auto: usize,
    pub overturned: usize,
    /// 從交接到人回答的中位時間（小時）
    pub median_hours: Option<f64>,
    /// policy.yaml 的問題
    pub policy_problems: Vec<String>,
}

/// 近 `days` 天的決策指標。
pub fn health_stats(spine_dir: &Path, days: i64, now: NaiveDateTime) -> Result<DecideHealth> {
    let since = (now - chrono::Duration::days(days))
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let events = spine::iter_events(spine_dir)?;
    let h = history_of(&events);
    let recent: Vec<&Answered> = h.iter().filter(|a| a.at >= since).collect();
    let human: Vec<&&Answered> = recent
        .iter()
        .filter(|a| a.by == AnsweredBy::Human)
        .collect();
    let with_rec = human
        .iter()
        .filter(|a| a.follow != FollowRec::NoRecommendation)
        .count();
    let followed = human
        .iter()
        .filter(|a| a.follow == FollowRec::Followed)
        .count();
    let auto: Vec<&&Answered> = recent.iter().filter(|a| a.by == AnsweredBy::Rule).collect();
    let stamps: HashMap<String, String> = events
        .iter()
        .filter(|e| agentapi::is_handoff(e))
        .map(|e| (e.reference(), e.stamp()))
        .collect();
    let parse = |s: &str| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").ok();
    let mut hours: Vec<f64> = human
        .iter()
        .filter_map(|a| {
            let at = parse(stamps.get(a.handoff_ref.as_ref()?)?)?;
            let done = parse(&a.at)?;
            (done >= at).then(|| (done - at).num_minutes() as f64 / 60.0)
        })
        .collect();
    hours.sort_by(f64::total_cmp);
    let median = match hours.len() {
        0 => None,
        n if n % 2 == 1 => Some(hours[n / 2]),
        n => Some((hours[n / 2 - 1] + hours[n / 2]) / 2.0),
    };
    let waiting = list_at(spine_dir, &RelayScope::All, now)
        .map(|q| q.cards.len())
        .unwrap_or(0);
    Ok(DecideHealth {
        days,
        waiting,
        answered: human.len(),
        with_recommend: with_rec,
        followed,
        followed_rate: (with_rec > 0).then(|| followed as f64 / with_rec as f64),
        auto: auto.len(),
        overturned: auto.iter().filter(|a| a.overturned_by.is_some()).count(),
        median_hours: median,
        policy_problems: policy::load(spine_dir).problems,
    })
}

// ---------------------------------------------------------------- suggest（總管建議，F-125）

/// `decide suggest` 的結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suggested {
    #[serde(flatten)]
    pub note: StewardNote,
    /// 卡上的問題
    pub question: String,
}

/// 總管在一張卡上留建議（選項編號或一句話＋理由）。不是回答：不算連續、不算抽查、不讓卡消失；
/// 人回答時看得到。沒有這張卡（沒在等、已回答）＝`not_found`。agent 可以下。
pub fn suggest(spine_dir: &Path, repo: &str, text: &str, why: Option<&str>) -> Result<Suggested> {
    let text = one_line(text);
    if text.is_empty() {
        return Err(rejected(
            RejectCode::BadRequest,
            "建議是空的".into(),
            "寫選項編號或一句話，例：ctx decide suggest <repo> 2 --why \"…\"".into(),
        ));
    }
    let id = registered(spine_dir, repo)?;
    let now = Local::now().naive_local();
    let plan = relay::plan_at(spine_dir, &RelayScope::Repos(vec![id.clone()]), now)?;
    let data = registry::load(spine_dir)?;
    let notes = log::steward_notes_of(&spine::iter_events(spine_dir)?);
    let Some(card) = plan
        .rows
        .iter()
        .find(|r| r.repo == id)
        .and_then(|r| card_of(spine_dir, r, &data, &notes, now, true))
    else {
        return Err(rejected(
            RejectCode::NotFound,
            format!(
                "{id} 沒有在等使用者決定的卡（最後一筆交接沒在等，或已經回答過），建議無處可留"
            ),
            "用 ctx decide list --all 看現在有哪些卡".into(),
        ));
    };
    let (answer, option) = resolve_reply(&card, &text)?;
    let r = log::write_steward_note(spine_dir, &id, &card.handoff_ref, &answer, option, why, now)?;
    let note = log::steward_notes_of(&spine::iter_events(spine_dir)?)
        .into_iter()
        .find(|n| n.reference == r)
        .ok_or_else(|| Error::invalid("總管建議寫入後讀不回來"))?;
    Ok(Suggested {
        note,
        question: card.question,
    })
}
