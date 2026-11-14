//! 放行規則（F-124，2026-11-14，Rust 才有）：同一類問題人連續照建議回答幾次後，工具問人要不要以後自動；
//! 人點頭（`ctx decide trust`）才升級，之後這類照建議自動放行並留痕；推翻一次就降回每次都問。
//!
//! 規則存在 spine 根目錄的 `policy.yaml`（純文字、人會手改、跟著 git）：
//! ```yaml
//! rules:
//! - repo: my-repo
//!   kind: commit
//!   level: auto          # ask＝每次問人｜auto＝照建議自動放行
//!   since: 2026-11-14 16:00   # 最後一次被人改等級的時間
//!   paths:               # 選填：repo 內路徑前綴；有寫時，沒 commit 的檔全在這底下才放行
//!   - docs/
//! ```
//! 讀：檔案不存在＝沒有規則；整份讀不懂、某條寫壞（level 亂寫、repo 沒登記、類別看不懂或不能自動、since 看不懂、
//! paths 不是相對路徑、同一個（repo、類別）寫了兩條）＝**那條當成沒有**，並記進 [`Policy::problems`]——
//! 任何解析失敗的結果都是「問人」，不會變成自動放行。
//! 寫：持 spine 的引擎鎖、暫存檔再 rename；同一份資料寫兩次位元組相同。沒動到的條目原樣保留（包括寫壞的，人才找得到）。
//!
//! 放行判定只有一個函式（[`gate`]）：`request_decision`（MCP／`ctx decide request`）與帶提問的 `handoff` 都走它。

use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;
use serde::Serialize;
use serde_yaml::{Mapping, Value};

use crate::ask::{Ask, AskKind};
use crate::decision_log;
use crate::{registry, spine, Error, RejectCode, Result};

/// 規則檔名（spine 根目錄）。
pub const FILE: &str = "policy.yaml";
/// 每第幾次放行改成問人（抽查）。
pub const SPOT_CHECK_EVERY: usize = 10;

const HEADER: &str = "# 放行規則（F-124）：ctx decide trust／untrust／overturn 會改這份，手改也可以。\n\
# level：ask＝每次問人｜auto＝照建議自動放行。push、publish、delete、send_external、visual_check、manual_test、other\n\
# 永遠不會自動；寫壞的條目一律當成沒有（ctx decide rules 會列出來）。\n";

/// 規則的等級。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// 每次問人
    Ask,
    /// 照建議自動放行
    Auto,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Ask => "ask",
            Level::Auto => "auto",
        }
    }
    fn parse(s: &str) -> Option<Level> {
        match s.trim() {
            "ask" => Some(Level::Ask),
            "auto" => Some(Level::Auto),
            _ => None,
        }
    }
}

/// 一條（讀得懂的）規則。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rule {
    pub repo: String,
    pub kind: AskKind,
    pub level: Level,
    /// `YYYY-MM-DD HH:MM`
    pub since: String,
    /// repo 內路徑前綴；空＝不限
    pub paths: Vec<String>,
}

impl Rule {
    /// `<repo>/<kind>`
    pub fn name(&self) -> String {
        decision_log::rule_name(&self.repo, self.kind)
    }
}

/// 讀進來的規則檔。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Policy {
    /// 生效的規則（寫壞的不在這裡）
    pub rules: Vec<Rule>,
    /// 哪裡寫壞了（給人看的句子）
    pub problems: Vec<String>,
}

impl Policy {
    /// 某個（repo、類別）生效的規則。
    pub fn rule(&self, repo: &str, kind: AskKind) -> Option<&Rule> {
        self.rules
            .iter()
            .find(|r| r.kind == kind && spine::same_token(&r.repo, repo))
    }
}

pub fn path(spine_dir: &Path) -> PathBuf {
    spine_dir.join(FILE)
}

fn as_str(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// 路徑前綴合不合格：相對、不跳出 repo。
fn valid_prefix(p: &str) -> bool {
    let p = p.trim();
    !p.is_empty()
        && !p.starts_with('/')
        && !p.starts_with('~')
        && !Path::new(p).is_absolute()
        && !p.split('/').any(|seg| seg == "..")
}

/// 一個條目 → 規則；看不懂＝Err(原因)。
fn rule_of(v: &Value, data: &registry::RegistryData) -> std::result::Result<Rule, String> {
    let Value::Mapping(m) = v else {
        return Err("不是「repo／kind／level／since」的物件".into());
    };
    let get = |k: &str| as_str(m.get(Value::String(k.into())));
    let repo = get("repo").filter(|r| !r.is_empty()).ok_or("缺 repo")?;
    if data.repo(&repo).is_none() {
        return Err(format!("repo「{repo}」沒登記"));
    }
    let kind_s = get("kind").unwrap_or_default();
    let kind = AskKind::parse(&kind_s).ok_or(format!("{repo} 的 kind「{kind_s}」看不懂"))?;
    let level_s = get("level").unwrap_or_default();
    let level = Level::parse(&level_s).ok_or(format!(
        "{repo}/{kind_s} 的 level「{level_s}」看不懂（只能是 ask 或 auto）"
    ))?;
    if level == Level::Auto && !kind.auto_approvable() {
        return Err(format!(
            "{repo}/{kind_s} 寫成 auto，但「{}」類永遠要人回答",
            kind.label()
        ));
    }
    let since = get("since").unwrap_or_default();
    if NaiveDateTime::parse_from_str(&since, "%Y-%m-%d %H:%M").is_err() {
        return Err(format!(
            "{repo}/{kind_s} 的 since「{since}」看不懂（要寫成 YYYY-MM-DD HH:MM）"
        ));
    }
    let paths = match m.get(Value::String("paths".into())) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Sequence(xs)) => {
            let ps: Vec<String> = xs.iter().filter_map(|x| as_str(Some(x))).collect();
            if ps.len() != xs.len() || !ps.iter().all(|p| valid_prefix(p)) {
                return Err(format!(
                    "{repo}/{kind_s} 的 paths 要是 repo 內相對路徑的清單"
                ));
            }
            ps
        }
        Some(_) => return Err(format!("{repo}/{kind_s} 的 paths 要是清單（- docs/）")),
    };
    Ok(Rule {
        repo,
        kind,
        level,
        since,
        paths,
    })
}

/// 原始條目（寫回時保留沒動到的）＋讀懂的結果。整份讀不懂＝Err(原因)。
fn read_raw(spine_dir: &Path) -> std::result::Result<Vec<Value>, String> {
    let p = path(spine_dir);
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{FILE} 讀不了：{e}")),
    };
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let v: Value =
        serde_yaml::from_str(&text).map_err(|e| format!("{FILE} 不是合格的 YAML：{e}"))?;
    match v {
        Value::Null => Ok(Vec::new()),
        Value::Mapping(m) => match m.get(Value::String("rules".into())) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Sequence(xs)) => Ok(xs.clone()),
            Some(_) => Err(format!("{FILE} 的 rules 要是清單")),
        },
        _ => Err(format!("{FILE} 最上層要是「rules:」")),
    }
}

fn parse(raw: &[Value], data: &registry::RegistryData) -> Policy {
    let mut out = Policy::default();
    let mut ok: Vec<Rule> = Vec::new();
    for (i, v) in raw.iter().enumerate() {
        match rule_of(v, data) {
            Ok(r) => ok.push(r),
            Err(e) => out
                .problems
                .push(format!("第 {} 條：{e}（這條不生效）", i + 1)),
        }
    }
    for r in &ok {
        let n = ok
            .iter()
            .filter(|x| x.kind == r.kind && spine::same_token(&x.repo, &r.repo))
            .count();
        if n > 1 {
            let msg = format!(
                "{} 寫了 {n} 條（同一個 repo＋類別只能一條；都不生效）",
                r.name()
            );
            if !out.problems.contains(&msg) {
                out.problems.push(msg);
            }
        } else {
            out.rules.push(r.clone());
        }
    }
    out
}

/// 讀規則檔。不會失敗：讀不懂的都進 `problems`、那些條目不生效。
pub fn load(spine_dir: &Path) -> Policy {
    let data = match registry::load(spine_dir) {
        Ok(d) => d,
        Err(e) => {
            return Policy {
                rules: Vec::new(),
                problems: vec![format!("registry 讀不了，規則都不生效：{e}")],
            }
        }
    };
    match read_raw(spine_dir) {
        Ok(raw) => parse(&raw, &data),
        Err(e) => Policy {
            rules: Vec::new(),
            problems: vec![format!("{e}（規則都不生效）")],
        },
    }
}

fn entry_of(r: &Rule) -> Value {
    let mut m = Mapping::new();
    let s = |x: &str| Value::String(x.to_string());
    m.insert(s("repo"), s(&r.repo));
    m.insert(s("kind"), s(r.kind.as_str()));
    m.insert(s("level"), s(r.level.as_str()));
    m.insert(s("since"), s(&r.since));
    if !r.paths.is_empty() {
        m.insert(
            s("paths"),
            Value::Sequence(r.paths.iter().map(|p| s(p)).collect()),
        );
    }
    Value::Mapping(m)
}

/// 條目指的是不是這個（repo、類別）——寫壞的條目也認（改等級時一起換掉）。
fn entry_is(v: &Value, repo: &str, kind: AskKind) -> bool {
    let Value::Mapping(m) = v else { return false };
    let get = |k: &str| as_str(m.get(Value::String(k.into())));
    get("repo").is_some_and(|r| spine::same_token(&r, repo))
        && get("kind").as_deref() == Some(kind.as_str())
}

/// 條目清單 → 檔案內容（穩定：同一份資料位元組相同）。
fn render(entries: &[Value]) -> Result<String> {
    let mut m = Mapping::new();
    m.insert(
        Value::String("rules".into()),
        Value::Sequence(entries.to_vec()),
    );
    Ok(format!(
        "{HEADER}{}",
        serde_yaml::to_string(&Value::Mapping(m))?
    ))
}

/// 設定一個（repo、類別）的等級（`since`＝now）。`paths`＝None 時保留原本的 paths（降級時用）。
/// 持引擎鎖讀→改→寫，暫存檔再 rename。整份讀不懂時拒絕（不覆蓋人手寫的東西）。
pub fn set_level(
    spine_dir: &Path,
    repo: &str,
    kind: AskKind,
    level: Level,
    paths: Option<Vec<String>>,
    now: NaiveDateTime,
) -> Result<Rule> {
    crate::locking::with_spine_lock(spine_dir, || {
        let mut raw = read_raw(spine_dir).map_err(|e| {
            Error::rejected(
                RejectCode::Failed,
                format!("{e}，沒有改規則"),
                Some(format!(
                    "先修好 {}（或刪掉重來）",
                    path(spine_dir).display()
                )),
            )
        })?;
        let data = registry::load(spine_dir)?;
        let old = parse(&raw, &data).rule(repo, kind).cloned();
        let rule = Rule {
            repo: repo.to_string(),
            kind,
            level,
            since: now.format("%Y-%m-%d %H:%M").to_string(),
            paths: paths.unwrap_or_else(|| old.map(|r| r.paths).unwrap_or_default()),
        };
        match raw.iter().position(|v| entry_is(v, repo, kind)) {
            Some(i) => {
                raw[i] = entry_of(&rule);
                let mut j = i + 1;
                while j < raw.len() {
                    if entry_is(&raw[j], repo, kind) {
                        raw.remove(j);
                    } else {
                        j += 1;
                    }
                }
            }
            None => raw.push(entry_of(&rule)),
        }
        crate::fsutil::write_atomic(&path(spine_dir), render(&raw)?.as_bytes())?;
        Ok(rule)
    })
}

// ---------------------------------------------------------------- 放行判定

/// 不放行的原因（每個原因一個變體；`code` 是 MCP 合約）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitReason {
    /// 這類問題永遠要人回答
    KindNotAutomatable(AskKind),
    /// 做了收不回來
    Irreversible,
    /// 沒有建議（放行的答案就是建議）
    NoRecommendation,
    /// repo 沒登記
    UnknownRepo,
    /// 沒有這個（repo、類別）的規則；policy.yaml 裡這條寫壞了時附上哪裡壞
    NoRule { problem: Option<String> },
    /// 規則是每次都問
    RuleIsAsk,
    /// 登記的路徑不存在
    RepoMissing,
    /// 查不到 git 狀態
    GitUnavailable(String),
    /// 有沒 commit 的檔在規則的 paths 外面
    OutsidePaths {
        files: Vec<String>,
        paths: Vec<String>,
    },
    /// 抽查：這條規則的第 N 次照樣問人
    SpotCheck { nth: usize },
    /// spine 讀不了（算不出抽查的次數）
    SpineUnreadable(String),
}

impl WaitReason {
    pub fn code(&self) -> &'static str {
        match self {
            WaitReason::KindNotAutomatable(_) => "kind_not_automatable",
            WaitReason::Irreversible => "irreversible",
            WaitReason::NoRecommendation => "no_recommendation",
            WaitReason::UnknownRepo => "unknown_repo",
            WaitReason::NoRule { .. } => "no_rule",
            WaitReason::RuleIsAsk => "rule_is_ask",
            WaitReason::RepoMissing => "repo_missing",
            WaitReason::GitUnavailable(_) => "git_unavailable",
            WaitReason::OutsidePaths { .. } => "outside_paths",
            WaitReason::SpotCheck { .. } => "spot_check",
            WaitReason::SpineUnreadable(_) => "spine_unreadable",
        }
    }

    /// 給人（與 agent）看的一句話。
    pub fn message(&self, repo: &str, kind: AskKind) -> String {
        let rule = decision_log::rule_name(repo, kind);
        match self {
            WaitReason::KindNotAutomatable(k) => {
                format!("「{}」類永遠要人回答（寫死在引擎，不是設定）", k.label())
            }
            WaitReason::Irreversible => "做了收不回來（reversible=false），要人回答".into(),
            WaitReason::NoRecommendation => "沒有建議，沒有可以自動放行的答案".into(),
            WaitReason::UnknownRepo => format!("{repo} 沒登記"),
            WaitReason::NoRule { problem: None } => {
                format!("沒有 {rule} 的放行規則（使用者還沒同意這類自動）")
            }
            WaitReason::NoRule { problem: Some(p) } => {
                format!("{rule} 的放行規則寫壞了，當成沒有：{p}")
            }
            WaitReason::RuleIsAsk => format!("{rule} 的規則是每次都問"),
            WaitReason::RepoMissing => format!("{repo} 登記的路徑不存在，查不了沒 commit 的檔"),
            WaitReason::GitUnavailable(e) => format!("查不了沒 commit 的檔：{e}"),
            WaitReason::OutsidePaths { files, paths } => {
                let shown: Vec<&str> = files.iter().take(5).map(String::as_str).collect();
                format!(
                    "有 {} 個沒 commit 的檔不在規則的範圍（{}）：{}{}",
                    files.len(),
                    paths.join("、"),
                    shown.join("、"),
                    if files.len() > shown.len() { "…" } else { "" }
                )
            }
            WaitReason::SpotCheck { nth } => {
                format!("抽查：{rule} 的第 {nth} 次照樣問人（每 {SPOT_CHECK_EVERY} 次一次）")
            }
            WaitReason::SpineUnreadable(e) => format!("spine 讀不了，算不出抽查次數：{e}"),
        }
    }
}

/// 判定結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    /// 放行：答案就是建議
    Auto { answer: String, rule: Rule },
    /// 要人回答
    Wait(WaitReason),
}

/// 沒 commit 的檔是不是全在某個前綴底下（`docs`、`docs/` 都認 `docs/x`，不認 `docsx`）。
fn under_any(file: &str, paths: &[String]) -> bool {
    paths.iter().any(|p| {
        let p = p.trim().trim_start_matches("./").trim_end_matches('/');
        p.is_empty() || file == p || file.starts_with(&format!("{p}/"))
    })
}

/// 放行判定（依序，第一個不成立的就是原因）：類別可自動 → 可逆 → 有建議 → repo 有登記 →
/// 有生效的 auto 規則 → 規則有 paths 時沒 commit 的檔全在裡面（引擎自己用 git 查）→ 不是抽查的那一次。
/// 純讀取；不會因為讀不到東西而放行。
pub fn gate(spine_dir: &Path, repo: &str, ask: &Ask) -> Gate {
    let wait = Gate::Wait;
    if !ask.kind.auto_approvable() {
        return wait(WaitReason::KindNotAutomatable(ask.kind));
    }
    if !ask.reversible {
        return wait(WaitReason::Irreversible);
    }
    let Some(answer) = ask.recommend.clone() else {
        return wait(WaitReason::NoRecommendation);
    };
    let Ok(data) = registry::load(spine_dir) else {
        return wait(WaitReason::UnknownRepo);
    };
    let Some(entry) = data.repo(repo) else {
        return wait(WaitReason::UnknownRepo);
    };
    let policy = load(spine_dir);
    let Some(rule) = policy.rule(repo, ask.kind).cloned() else {
        let mark = format!("{repo}/{}", ask.kind.as_str());
        let problem = policy
            .problems
            .iter()
            .find(|p| p.contains(&mark) || p.contains("規則都不生效"))
            .cloned();
        return wait(WaitReason::NoRule { problem });
    };
    if rule.level != Level::Auto {
        return wait(WaitReason::RuleIsAsk);
    }
    if !rule.paths.is_empty() {
        let root = crate::fsutil::expanduser(entry.path());
        if !root.is_dir() {
            return wait(WaitReason::RepoMissing);
        }
        match crate::gitlog::dirty_files(&root) {
            Err(e) => return wait(WaitReason::GitUnavailable(e.to_string())),
            Ok(files) => {
                let outside: Vec<String> = files
                    .into_iter()
                    .filter(|f| !under_any(f, &rule.paths))
                    .collect();
                if !outside.is_empty() {
                    return wait(WaitReason::OutsidePaths {
                        files: outside,
                        paths: rule.paths.clone(),
                    });
                }
            }
        }
    }
    // 抽查：自 since 起這條規則的放行＋人的回答（含被抽查到、人答掉的那次）算第幾次；決定性，不用亂數
    let history = match decision_log::history(spine_dir) {
        Ok(h) => h,
        Err(e) => return wait(WaitReason::SpineUnreadable(e.to_string())),
    };
    let done = history
        .iter()
        .filter(|a| {
            a.kind == Some(ask.kind)
                && spine::same_token(&a.repo, repo)
                && a.at.as_str() >= rule.since.as_str()
        })
        .count();
    let nth = done + 1;
    if nth % SPOT_CHECK_EVERY == 0 {
        return wait(WaitReason::SpotCheck { nth });
    }
    Gate::Auto { answer, rule }
}

/// 放行時告訴 agent 的。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AutoApproved {
    /// 照這個答案接著做（就是提問的建議）
    pub answer: String,
    /// 哪條規則放行的（`<repo>/<kind>`）
    pub rule: String,
    /// 放行留痕的 ref（使用者要推翻：`ctx decide overturn <ref>`）
    #[serde(rename = "ref")]
    pub reference: String,
}

/// 不放行的原因（`code` 是合約）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaitInfo {
    pub code: &'static str,
    pub message: String,
}

/// `request_decision`／`ctx decide request` 的回傳：`{decision:"auto", answer, rule, ref}` 或 `{decision:"wait", reason}`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    Auto(AutoApproved),
    Wait { reason: WaitInfo },
}

/// 判定＋放行時留痕（來源 `policy`，指向交接 `handoff_ref`；agent 做到一半問的沒有）。不放行不留痕。
/// `request_decision` 與帶提問的 `handoff` 共用。
pub fn approve(
    spine_dir: &Path,
    repo: &str,
    ask: &Ask,
    handoff_ref: Option<&str>,
    now: NaiveDateTime,
) -> Result<Decision> {
    match gate(spine_dir, repo, ask) {
        Gate::Wait(r) => Ok(Decision::Wait {
            reason: WaitInfo {
                code: r.code(),
                message: r.message(repo, ask.kind),
            },
        }),
        Gate::Auto { answer, rule } => {
            let a = decision_log::write_answer(
                spine_dir,
                &decision_log::NewAnswer {
                    repo,
                    handoff_ref,
                    question: &ask.question,
                    answer: &answer,
                    recommend: ask.recommend.as_deref(),
                    kind: Some(ask.kind),
                    reversible: ask.reversible,
                    by: decision_log::AnsweredBy::Rule,
                },
                now,
            )?;
            Ok(Decision::Auto(AutoApproved {
                answer,
                rule: rule.name(),
                reference: a.reference,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes() {
        let ps = vec!["docs/".to_string(), "./notes".to_string()];
        assert!(under_any("docs/a.md", &ps));
        assert!(under_any("notes/x/y.md", &ps));
        assert!(under_any("notes", &ps));
        assert!(!under_any("docsx/a.md", &ps));
        assert!(!under_any("src/a.rs", &ps));
        assert!(valid_prefix("docs/"));
        assert!(!valid_prefix("/etc"));
        assert!(!valid_prefix("../x"));
        assert!(!valid_prefix(" "));
    }

    #[test]
    fn level_parse() {
        assert_eq!(Level::parse(" auto "), Some(Level::Auto));
        assert_eq!(Level::parse("Auto"), None);
        assert_eq!(Level::parse("yes"), None);
    }
}
