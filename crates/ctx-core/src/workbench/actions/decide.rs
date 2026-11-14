//! 決策佇列與放行規則的 App 入口（F-127，Rust 才有）：判斷都在 `crate::decide`／`crate::policy`，這裡只轉參數與回傳。
//!
//! App 是使用者本人在按：回答、設為自動、降回、推翻都直接呼叫引擎（CLI 那道 `CLAUDECODE` 檢查只在 `ctx` 的 CLI 層，
//! 引擎不看環境變數）。讀的（佇列、自動放行、規則、總管簡報）列在 `READ_ONLY`。

use std::path::Path;

use chrono::Local;
use serde::Deserialize;
use serde_json::Value;

use super::{parse, Terms};
use crate::relay::RelayScope;
use crate::Result;

/// 範圍（同 get_state：repos ＞ group ＞ 全部）。
#[derive(Deserialize, Default)]
struct ScopeArgs {
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    repos: Option<Vec<String>>,
}

impl ScopeArgs {
    fn relay(self) -> RelayScope {
        match (self.repos, self.group.filter(|g| !g.is_empty())) {
            (Some(r), _) => RelayScope::Repos(r),
            (None, Some(g)) => RelayScope::Group(g),
            (None, None) => RelayScope::All,
        }
    }
}

fn json<T: serde::Serialize>(v: T) -> Result<Value> {
    Ok(serde_json::to_value(v)?)
}

/// 要你決定的卡（不跑 git）。
pub(super) fn a_decide_queue(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let a: ScopeArgs = parse("decide_queue", p)?;
    json(crate::decide::list_light(
        d,
        &a.relay(),
        Local::now().naive_local(),
    )?)
}

#[derive(Deserialize)]
struct AnswerArgs {
    repo: String,
    reply: String,
}

/// 回答一張卡（留痕＋送達方式；不開會話——送不送、怎麼送由人在畫面上按）。
pub(super) fn a_decide_answer(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let a: AnswerArgs = parse("decide_answer", p)?;
    json(crate::decide::answer(d, &a.repo, &a.reply)?)
}

#[derive(Deserialize, Default)]
struct SinceArgs {
    #[serde(default)]
    since: Option<String>,
}

/// 最近的自動放行（預設近 7 天）。
pub(super) fn a_decide_auto(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let a: SinceArgs = parse("decide_auto", p)?;
    json(crate::decide::auto_list(d, a.since.as_deref())?)
}

/// 人最近的回答（近 7 天、最多 10 筆；撤回用）。
pub(super) fn a_decide_mine(d: &Path, _p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    json(crate::decide::my_answers(d, Local::now().naive_local())?)
}

/// 放行規則一覽＋升級建議＋policy.yaml 的問題。
pub(super) fn a_decide_rules(d: &Path, _p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    json(crate::decide::rules_report(d)?)
}

#[derive(Deserialize)]
struct RuleArgs {
    repo: String,
    kind: String,
}

/// 設為自動（只從回答後的升級建議那一步進來；不提供 paths）。
pub(super) fn a_decide_trust(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let a: RuleArgs = parse("decide_trust", p)?;
    json(crate::decide::trust(d, &a.repo, &a.kind, None)?)
}

/// 降回每次都問。
pub(super) fn a_decide_untrust(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let a: RuleArgs = parse("decide_untrust", p)?;
    json(crate::decide::untrust(d, &a.repo, &a.kind)?)
}

#[derive(Deserialize)]
struct OverturnArgs {
    #[serde(rename = "ref")]
    reference: String,
    #[serde(default)]
    why: Option<String>,
}

/// 推翻一筆自動放行（規則降回 ask）或撤回一筆人的回答（規則不動）；那筆交接回到佇列。
pub(super) fn a_decide_overturn(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let a: OverturnArgs = parse("decide_overturn", p)?;
    json(crate::decide::overturn(d, &a.reference, a.why.as_deref())?)
}

/// 總管簡報（不記「看過了」——只有總管會話開場才記）。
pub(super) fn a_steward_brief(d: &Path, _p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    json(crate::steward::brief(d, Local::now().naive_local())?)
}
