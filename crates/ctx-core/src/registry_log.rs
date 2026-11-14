//! registry 變更（F-125，2026-11-14，Rust 才有）：一段時間內 registry 發生了什麼——**結果**，不是流水帳。
//!
//! 來源是 spine 裡 `ops::call` 留的管理類留痕（body 以 [`spine::ADMIN_TAG`] 開頭的 decision，格式是 `ops.rs` 各操作的
//! `audit` 寫死的句子）。同一個對象（repo、組、關係、export）在期間內的多筆合成一筆：登記後又移出＝不列；
//! 最後是移出＝列在「移除」；有登記（或建組、加關係、加 export）＝列在「新增」，期間的其他改動併成它的細節；
//! 其他＝「變更」，同一面向（名片、類別、tier、tag、欄位、組員、目標…）只留最後的樣子（tier 保留最早的舊值）。
//! 另附「現在的異常」：登記的路徑不存在的 repo（封存的不算）。
//!
//! 已知沒有留這種痕的寫入（不在這裡）：`ctx registry scan --register`（`registry::scan_register` 直接登記、不留痕）、
//! App 的「加入資料夾」與「登記 repo」（`workbench::actions` 留的是沒有 ADMIN_TAG 的 decision）、`ctx spawn` 的出生登記。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use chrono::NaiveDateTime;
use regex::Regex;
use serde::Serialize;

use crate::registry::{self, Tier};
use crate::spine::{self, Event, EventType, ADMIN_TAG};
use crate::Result;

/// 變更的對象種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    Repo,
    Group,
    Relation,
    Export,
}

impl Area {
    pub fn label(self) -> &'static str {
        match self {
            Area::Repo => "repo",
            Area::Group => "組",
            Area::Relation => "關係",
            Area::Export => "export",
        }
    }
}

/// 一個對象在期間內的結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegChange {
    pub area: Area,
    /// repo id、組名、`a → b（kind）`、`repo:name`
    pub name: String,
    /// 最後的樣子（一個面向一句，舊到新）
    pub details: Vec<String>,
    /// 最後一筆的時間（`YYYY-MM-DD HH:MM`）
    pub at: String,
    /// 期間內幾筆留痕合成的
    pub events: usize,
}

/// 登記的路徑不存在的 repo。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MissingPath {
    pub repo: String,
    pub path: String,
}

/// `ctx registry changes` 的結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegistryChanges {
    pub since: String,
    pub at: String,
    pub added: Vec<RegChange>,
    pub removed: Vec<RegChange>,
    pub changed: Vec<RegChange>,
    /// 現在的異常：路徑不存在
    pub missing_paths: Vec<MissingPath>,
    /// 期間內讀不懂的管理類留痕筆數（人手寫的、舊格式）
    pub unrecognized: usize,
}

impl RegistryChanges {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Add,
    Remove,
    Change,
}

/// 一個對象的一步：(動作, 面向, 一句話, 時間)。
type Step = (Op, String, String, String);

/// 一筆留痕拆出來的：對象、動作、面向（同面向後蓋前）、一句話。
struct Parsed {
    area: Area,
    name: String,
    op: Op,
    aspect: String,
    text: String,
}

static VIA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"（從[^（）]*）$").expect("寫死的 regex"));
static REL_ADD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^關係：(?P<a>\S+) (?P<label>.*?)→ (?P<b>\S+)（(?P<kind>[^（）]+)）$")
        .expect("寫死的 regex")
});
static REL_DEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^刪關係：(?P<a>\S+) → (?P<b>[^（\s]+)(?:（(?P<kind>[^（）]+)）)?$")
        .expect("寫死的 regex")
});

fn parse(ev: &Event) -> Option<Parsed> {
    let rest = ev.body.lines().next()?.strip_prefix(ADMIN_TAG)?;
    let rest = VIA.replace(rest, "").to_string();
    let repo = ev.kv("repo").map(str::to_string);
    let group = ev.kv("group").map(str::to_string);
    let p = |area, name: String, op, aspect: &str, text: String| {
        Some(Parsed {
            area,
            name,
            op,
            aspect: aspect.into(),
            text,
        })
    };
    if let Some(x) = rest.strip_prefix("登記 repo：") {
        let (id, path) = x.split_once('（').unwrap_or((x, ""));
        return p(
            Area::Repo,
            id.to_string(),
            Op::Add,
            "登記",
            format!("登記（{}）", path.trim_end_matches('）')),
        );
    }
    if let Some(id) = rest.strip_prefix("移出 registry：") {
        return p(
            Area::Repo,
            id.trim().into(),
            Op::Remove,
            "移出",
            "移出 registry".into(),
        );
    }
    if let Some(c) = REL_ADD.captures(&rest) {
        let name = format!("{} → {}（{}）", &c["a"], &c["b"], &c["kind"]);
        let label = c["label"].trim().to_string();
        let text = if label.is_empty() {
            "加關係".to_string()
        } else {
            format!("加關係：{label}")
        };
        return p(Area::Relation, name, Op::Add, "關係", text);
    }
    if let Some(c) = REL_DEL.captures(&rest) {
        let name = match c.name("kind") {
            Some(k) => format!("{} → {}（{}）", &c["a"], &c["b"], k.as_str()),
            None => format!("{} → {}", &c["a"], &c["b"]),
        };
        return p(Area::Relation, name, Op::Remove, "關係", "刪關係".into());
    }
    if let Some(x) = rest.strip_prefix("刪 export ") {
        return p(
            Area::Export,
            x.trim().into(),
            Op::Remove,
            "export",
            "刪 export".into(),
        );
    }
    if let Some(x) = rest.strip_prefix("export ") {
        let (name, path) = x.split_once(" → ").unwrap_or((x, ""));
        return p(
            Area::Export,
            name.trim().into(),
            Op::Add,
            "export",
            format!("→ {path}"),
        );
    }
    if let Some(m) = rest.strip_prefix("建組：") {
        return p(
            Area::Group,
            group?,
            Op::Add,
            "建組",
            format!("建組，成員 {m}"),
        );
    }
    if rest.starts_with("刪除組") {
        return p(Area::Group, group?, Op::Remove, "刪除", "刪除組".into());
    }
    if let Some(x) = rest.strip_prefix("編輯組：") {
        return p(Area::Group, group?, Op::Change, "組員", x.to_string());
    }
    if let Some(x) = rest.strip_prefix("本組目標：") {
        return p(
            Area::Group,
            group?,
            Op::Change,
            "目標",
            format!("目標：{x}"),
        );
    }
    if rest == "清除本組目標" {
        return p(Area::Group, group?, Op::Change, "目標", "清除目標".into());
    }
    let id = repo?;
    if let Some(x) = rest.strip_prefix(&format!("{id} 名片：")) {
        return p(
            Area::Repo,
            id.clone(),
            Op::Change,
            "名片",
            format!("名片：{x}"),
        );
    }
    if let Some(x) = rest.strip_prefix(&format!("{id} 類別：")) {
        return p(
            Area::Repo,
            id.clone(),
            Op::Change,
            "類別",
            format!("類別：{x}"),
        );
    }
    if let Some(x) = rest.strip_prefix("tier ") {
        return p(
            Area::Repo,
            id.clone(),
            Op::Change,
            "tier",
            format!("tier {x}"),
        );
    }
    if let Some(x) = rest.strip_prefix("tag ") {
        return p(Area::Repo, id.clone(), Op::Change, "tag", x.to_string());
    }
    if let Some(x) = rest.strip_prefix(&format!("{id}.")) {
        let key = x.split_once(" = ").map_or(x, |(k, _)| k).to_string();
        return p(
            Area::Repo,
            id.clone(),
            Op::Change,
            &format!("欄位 {key}"),
            x.to_string(),
        );
    }
    None
}

/// 同一面向的多筆合成最後的樣子（tier 保留最早的舊值；tag 把增減併起來）。
fn merge_aspect(aspect: &str, texts: &[String]) -> String {
    let last = texts.last().cloned().unwrap_or_default();
    match aspect {
        "tier" if texts.len() > 1 => {
            let first_old = texts[0]
                .strip_prefix("tier ")
                .and_then(|x| x.split_once(" → "))
                .map(|(o, _)| o.to_string());
            let last_new = last.split_once(" → ").map(|(_, n)| n.to_string());
            match (first_old, last_new) {
                (Some(o), Some(n)) => format!("tier {o} → {n}"),
                _ => last,
            }
        }
        "tag" => {
            let mut net: BTreeMap<String, char> = BTreeMap::new();
            for t in texts {
                for tok in t.split_whitespace() {
                    let (sign, name) = tok.split_at(tok.chars().next().map_or(0, char::len_utf8));
                    if let Some(prev) = net.get(name) {
                        if prev.to_string() != sign {
                            net.remove(name);
                            continue;
                        }
                    }
                    if let Some(c) = sign.chars().next() {
                        net.insert(name.to_string(), c);
                    }
                }
            }
            let parts: Vec<String> = net.iter().map(|(n, s)| format!("{s}{n}")).collect();
            if parts.is_empty() {
                "tag 改了又改回".into()
            } else {
                format!("tag {}", parts.join(" "))
            }
        }
        _ => last,
    }
}

/// `since`（`YYYY-MM-DD HH:MM`）之後 registry 的變更結果＋現在的異常。純讀取。
pub fn changes(spine_dir: &Path, since: &str, now: NaiveDateTime) -> Result<RegistryChanges> {
    changes_of(spine_dir, &spine::iter_events(spine_dir)?, since, now)
}

/// 同 [`changes`]，從已讀好的事件算。
pub fn changes_of(
    spine_dir: &Path,
    events: &[Event],
    since: &str,
    now: NaiveDateTime,
) -> Result<RegistryChanges> {
    // (area, name) → [(op, aspect, text, at)]
    let mut by: BTreeMap<(Area, String), Vec<Step>> = BTreeMap::new();
    let mut order: Vec<(Area, String)> = Vec::new();
    let mut unrecognized = 0;
    for ev in events.iter().filter(|e| {
        e.etype == EventType::Decision
            && e.body.starts_with(ADMIN_TAG)
            && e.stamp().as_str() > since
    }) {
        match parse(ev) {
            Some(x) => {
                let key = (x.area, x.name);
                if !by.contains_key(&key) {
                    order.push(key.clone());
                }
                by.entry(key)
                    .or_default()
                    .push((x.op, x.aspect, x.text, ev.stamp()));
            }
            None => unrecognized += 1,
        }
    }
    let (mut added, mut removed, mut changed) = (Vec::new(), Vec::new(), Vec::new());
    // 期間內登記又移出的：結果是沒有——除非現在又在 registry（登記回來那一步沒留痕），下面對帳
    let mut transient: Vec<RegChange> = Vec::new();
    for key in order {
        let ops = &by[&key];
        let first = ops.first().map(|o| o.0);
        let last = ops.last().map(|o| o.0);
        let at = ops.last().map(|o| o.3.clone()).unwrap_or_default();
        // 面向 → 這個面向的句子（舊到新），面向照第一次出現的順序
        let mut aspects: Vec<(String, Vec<String>)> = Vec::new();
        for (op, aspect, text, _) in ops {
            if *op == Op::Change {
                match aspects.iter_mut().find(|(a, _)| a == aspect) {
                    Some((_, v)) => v.push(text.clone()),
                    None => aspects.push((aspect.clone(), vec![text.clone()])),
                }
            }
        }
        let change_lines: Vec<String> = aspects.iter().map(|(a, ts)| merge_aspect(a, ts)).collect();
        let entry = |details: Vec<String>| RegChange {
            area: key.0,
            name: key.1.clone(),
            details,
            at: at.clone(),
            events: ops.len(),
        };
        match (first, last) {
            (Some(Op::Add), Some(Op::Remove)) => transient.push(entry(vec![ops
                .iter()
                .rfind(|o| o.0 == Op::Add)
                .map(|o| o.2.clone())
                .unwrap_or_default()])),
            (_, Some(Op::Remove)) => removed.push(entry(vec![ops
                .last()
                .map(|o| o.2.clone())
                .unwrap_or_default()])),
            _ if ops.iter().any(|o| o.0 == Op::Add) => {
                let add = ops
                    .iter()
                    .rfind(|o| o.0 == Op::Add)
                    .map(|o| o.2.clone())
                    .unwrap_or_default();
                let mut d = vec![add];
                d.extend(change_lines);
                added.push(entry(d));
            }
            _ => changed.push(entry(change_lines)),
        }
    }
    // 用現在的 registry 對帳：有些寫入沒留痕（App 加入資料夾、scan --register…），留痕說移出、現在卻還在的，
    // 改列在「變更」並講清楚；變更過、現在已不在的標出來
    let data = registry::load(spine_dir)?;
    let exists = |c: &RegChange| match c.area {
        Area::Repo => data.repo(&c.name).is_some(),
        Area::Group => data
            .groups
            .iter()
            .any(|g| spine::same_token(&c.name, &g.name())),
        Area::Relation | Area::Export => true,
    };
    let (back, removed): (Vec<RegChange>, Vec<RegChange>) =
        removed.into_iter().partition(|c| exists(c));
    for mut c in back {
        c.details = vec![
            "移出後又回到 registry（登記回來那一步沒留痕：App 加入資料夾、registry scan --register 或手改）".into(),
        ];
        changed.push(c);
    }
    for mut c in transient.into_iter().filter(|c| exists(c)) {
        c.details.push(
            "之後移出、又回到 registry（登記回來那一步沒留痕：App 加入資料夾、registry scan --register 或手改）"
                .into(),
        );
        added.push(c);
    }
    for c in added.iter_mut().chain(changed.iter_mut()) {
        if !exists(c) {
            c.details
                .push("現在已不在 registry（之後的移出或改名沒留痕）".into());
        }
    }
    let missing_paths = data
        .repos
        .iter()
        .filter(|r| r.tier() != Some(Tier::Archived))
        .filter(|r| !crate::fsutil::expanduser(r.path()).is_dir())
        .map(|r| MissingPath {
            repo: r.id(),
            path: r.path_str(),
        })
        .collect();
    Ok(RegistryChanges {
        since: since.to_string(),
        at: now.format("%Y-%m-%d %H:%M").to_string(),
        added,
        removed,
        changed,
        missing_paths,
        unrecognized,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_aspects() {
        assert_eq!(
            merge_aspect(
                "tier",
                &[
                    "tier active → paused".into(),
                    "tier paused → archived".into()
                ]
            ),
            "tier active → archived"
        );
        assert_eq!(
            merge_aspect("tag", &["+a +b".into(), "-a +c".into()]),
            "tag +b +c"
        );
        assert_eq!(
            merge_aspect("tag", &["+a".into(), "-a".into()]),
            "tag 改了又改回"
        );
        assert_eq!(
            merge_aspect("名片", &["名片：x".into(), "名片：y".into()]),
            "名片：y"
        );
    }
}
