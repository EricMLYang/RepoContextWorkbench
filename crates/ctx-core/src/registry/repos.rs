//! repo 登記：新增／改欄位／tag／移除／存活率。對應 `registry/repos.py`。

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use serde::Serialize;
use serde_yaml::Value;

use super::store::{load, transaction, Repo};
use crate::{Error, Result};

/// 對應 `registry.TIERS`。
pub const TIERS: [&str; 4] = ["active", "paused", "dormant", "archived"];

/// repo 的 tier（序列化字串同 Python）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    Active,
    Paused,
    Dormant,
    Archived,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Active => "active",
            Tier::Paused => "paused",
            Tier::Dormant => "dormant",
            Tier::Archived => "archived",
        }
    }
}

impl FromStr for Tier {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "active" => Tier::Active,
            "paused" => Tier::Paused,
            "dormant" => Tier::Dormant,
            "archived" => Tier::Archived,
            _ => return Err(Error::invalid(format!("tier 需為 {}", TIERS.join("|")))),
        })
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `add_repo` 的選填參數（預設＝Python 的預設：mine／active／無 tag）。
#[derive(Debug, Clone)]
pub struct AddRepo {
    /// `type`：mine｜external
    pub kind: String,
    pub tier: String,
    pub tags: Vec<String>,
    pub upstream: Option<String>,
    pub origin: Option<String>,
}

impl Default for AddRepo {
    fn default() -> Self {
        AddRepo {
            kind: "mine".into(),
            tier: "active".into(),
            tags: Vec::new(),
            upstream: None,
            origin: None,
        }
    }
}

fn strs(v: &[String]) -> Value {
    Value::Sequence(v.iter().map(|s| Value::String(s.clone())).collect())
}

/// 登記的 repo 是檔案檢視器、編輯、終端的範圍；根目錄或家目錄（以及它們的上層）等於把整台機器開放出去。
/// 刻意差異（2026-10-13，資安）：Python 版不擋。
fn refuse_too_broad(path: &Path) -> Result<()> {
    let real = crate::fsutil::resolve(&crate::fsutil::expanduser(path));
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| crate::fsutil::resolve(Path::new(&h)));
    if real.parent().is_none() || home.is_some_and(|h| h.starts_with(&real)) {
        return Err(Error::invalid(format!(
            "不能把根目錄或家目錄登記成 repo（範圍太大）：{}",
            path.display()
        )));
    }
    Ok(())
}

/// 對應 `registry.add_repo`。回傳新登記項。
pub fn add_repo(
    spine_dir: impl AsRef<Path>,
    id: &str,
    path: impl AsRef<Path>,
    opts: AddRepo,
) -> Result<Repo> {
    if opts.kind != "mine" && opts.kind != "external" {
        return Err(Error::invalid("type 需為 mine|external"));
    }
    if !TIERS.contains(&opts.tier.as_str()) {
        return Err(Error::invalid(format!("tier 需為 {}", TIERS.join("|"))));
    }
    refuse_too_broad(path.as_ref())?;
    let path = path.as_ref().to_string_lossy().into_owned();
    transaction(spine_dir, |data| {
        if data.repo(id).is_some() {
            return Err(Error::invalid(format!("repo id 已存在: {id}")));
        }
        let mut e = Repo::default();
        e.set("id", id);
        e.set("path", path);
        e.set("type", opts.kind.as_str());
        e.set("tags", strs(&opts.tags));
        e.set("tier", opts.tier.as_str());
        if let Some(u) = opts.upstream.filter(|s| !s.is_empty()) {
            e.set("upstream", u);
        }
        if let Some(o) = opts.origin.filter(|s| !s.is_empty()) {
            e.set("origin", o);
        }
        data.repos.push(e.clone());
        Ok(e)
    })
}

/// 對應 `registry.set_field`。回傳更新後的登記項。
pub fn set_field(
    spine_dir: impl AsRef<Path>,
    id: &str,
    key: &str,
    value: impl Into<Value>,
) -> Result<Repo> {
    let value = value.into();
    transaction(spine_dir, |data| match data.repo_mut(id) {
        Some(r) => {
            r.set(key, value);
            Ok(r.clone())
        }
        None => Err(Error::invalid(format!("repo 不存在: {id}"))),
    })
}

/// `describe_repo` 要改哪些名片欄位：None＝不動、空字串／空清單＝清掉。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Describe {
    pub description: Option<String>,
    pub role: Option<String>,
    pub keywords: Option<Vec<String>>,
}

/// Rust 才有（2026-10-13）：改 repo 名片（介紹／角色／關鍵字），存 registry——不寫進 repo（最小打擾）。
/// 回傳更新後的登記項。三個都沒給＝錯。
pub fn describe_repo(spine_dir: impl AsRef<Path>, id: &str, d: &Describe) -> Result<Repo> {
    if d.description.is_none() && d.role.is_none() && d.keywords.is_none() {
        return Err(Error::invalid("description／role／keywords 至少給一個"));
    }
    transaction(spine_dir, |data| {
        let r = data
            .repo_mut(id)
            .ok_or_else(|| Error::invalid(format!("repo 不存在: {id}")))?;
        for (k, v) in [("description", &d.description), ("role", &d.role)] {
            match v.as_deref().map(str::trim) {
                Some("") => {
                    r.remove(k);
                }
                Some(t) => r.set(k, t),
                None => {}
            }
        }
        match &d.keywords {
            Some(k) if k.is_empty() => {
                r.remove("keywords");
            }
            Some(k) => r.set("keywords", strs(k)),
            None => {}
        }
        Ok(r.clone())
    })
}

/// 對應 `registry.tag_repo`：P1 tag 操作（2026-09-01 使用者需求：分群以外要能很容易下 tag）。
/// 回傳更新後的 tags；重複 add 冪等、remove 不存在的 tag 靜默略過。
pub fn tag_repo<A: AsRef<str>, R: AsRef<str>>(
    spine_dir: impl AsRef<Path>,
    id: &str,
    add: &[A],
    remove: &[R],
) -> Result<Vec<String>> {
    transaction(spine_dir, |data| {
        let r = data
            .repo_mut(id)
            .ok_or_else(|| Error::invalid(format!("repo 不存在: {id}")))?;
        let mut tags = r.tags();
        for t in add {
            let t = t.as_ref().trim();
            if !t.is_empty() && !tags.iter().any(|x| x == t) {
                tags.push(t.to_string());
            }
        }
        for t in remove {
            if let Some(i) = tags.iter().position(|x| x == t.as_ref()) {
                tags.remove(i);
            }
        }
        r.set("tags", strs(&tags));
        Ok(tags)
    })
}

/// 對應 `registry.all_tags`：registry 內既有 tag 的去重清單（保持首次出現順序）。
pub fn all_tags(spine_dir: impl AsRef<Path>) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for r in load(spine_dir)?.repos {
        for t in r.tags() {
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    Ok(out)
}

/// 對應 `registry.remove_repo`：移除 repo 並清掉所有組的 membership（組保留，允許空組）與指向它的關係。
pub fn remove_repo(spine_dir: impl AsRef<Path>, id: &str) -> Result<()> {
    transaction(spine_dir, |data| {
        if data.repo(id).is_none() {
            return Err(Error::invalid(format!("repo 不存在: {id}")));
        }
        data.repos.retain(|r| r.id() != id);
        for r in &mut data.repos {
            if !r.has("relations") {
                continue;
            }
            if let Some(Value::Sequence(seq)) = r.raw_mut().get_mut("relations") {
                seq.retain(|x| x.get("to").and_then(Value::as_str) != Some(id));
                if seq.is_empty() {
                    r.remove("relations");
                }
            }
        }
        for g in &mut data.groups {
            let mut m = g.members();
            if let Some(i) = m.iter().position(|x| x == id) {
                m.remove(i);
                g.set_members(&m);
            }
        }
        Ok(())
    })
}

/// 對應 `registry.get_repo`。
pub fn get_repo(spine_dir: impl AsRef<Path>, id: &str) -> Result<Repo> {
    load(spine_dir)?
        .repo(id)
        .cloned()
        .ok_or_else(|| Error::invalid(format!("repo 不存在: {id}")))
}

/// `survival` 的結果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Survival {
    pub spawned: usize,
    pub alive: usize,
    pub survival_rate: Option<f64>,
}

/// 對應 `registry.survival`：品味量測第二視圖（P11）——碰撞生出來的 repo（有 origin 血統）還活著幾個。
pub fn survival(spine_dir: impl AsRef<Path>) -> Result<Survival> {
    let data = load(spine_dir)?;
    let spawned: Vec<&Repo> = data.repos.iter().filter(|r| r.has("origin")).collect();
    let alive = spawned
        .iter()
        .filter(|r| matches!(r.tier(), Some(Tier::Active | Tier::Paused)))
        .count();
    let n = spawned.len();
    Ok(Survival {
        spawned: n,
        alive,
        survival_rate: (n > 0).then(|| alive as f64 / n as f64),
    })
}
