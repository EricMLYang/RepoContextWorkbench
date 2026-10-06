//! 關係層與 exports（repo 間有向關係、repo 對外提供什麼）。對應 `registry/relations.py`。
//!
//! 關係存在**來源 repo** 的 `relations:` 欄（`{to, kind, note?, exports?}`）。詞彙表是起手式，
//! 私有層可用 config `relations.kinds` 擴充（同名以 config 為準）：
//! ```yaml
//! relations:
//!   kinds:
//!     tests: {forward: 驗證, inverse: 被驗證於}
//! ```
//! forward＝從來源 repo 讀的說法（A 規劃（PM）B）、inverse＝從目標 repo 讀的說法（B 的 PM 是 A）；
//! symmetric＝對稱關係兩向同字。
//!
//! exports（2026-09-25 跨 repo 參考輪）：像 repo 的 API——`<repo>:書摘` 指向 `021_Ebook/`。
//! agent 用名字引用、ctx resolve 換路徑，repo 搬家只改 registry 一處；寫死的 `../x/y` 會斷，名字不會。

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;
use serde_yaml::{Mapping, Value};

use super::store::{load, relation_from_mapping, transaction, Relation, Repo};
use crate::{Error, Result};

/// 一種關係的讀法。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelationKind {
    pub forward: String,
    pub inverse: String,
    pub symmetric: bool,
}

/// 關係詞彙表（保順序：內建在前、config 擴充在後）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RelationKinds(pub Vec<(String, RelationKind)>);

impl RelationKinds {
    pub fn get(&self, kind: &str) -> Option<&RelationKind> {
        self.0.iter().find(|(k, _)| k == kind).map(|(_, v)| v)
    }
    pub fn contains(&self, kind: &str) -> bool {
        self.get(kind).is_some()
    }
    pub fn names(&self) -> Vec<&str> {
        self.0.iter().map(|(k, _)| k.as_str()).collect()
    }
    fn upsert(&mut self, k: String, v: RelationKind) {
        match self.0.iter_mut().find(|(n, _)| *n == k) {
            Some(slot) => slot.1 = v,
            None => self.0.push((k, v)),
        }
    }
    /// 正向讀法（不認得的 kind＝原字）。
    pub fn forward<'a>(&'a self, kind: &'a str) -> &'a str {
        self.get(kind).map(|k| k.forward.as_str()).unwrap_or(kind)
    }
    /// 反向讀法（不認得的 kind＝原字）。
    pub fn inverse<'a>(&'a self, kind: &'a str) -> &'a str {
        self.get(kind).map(|k| k.inverse.as_str()).unwrap_or(kind)
    }
}

/// 對應 `registry.RELATION_KINDS`（內建詞彙）。
pub fn builtin_relation_kinds() -> RelationKinds {
    let k = |f: &str, i: &str, s: bool| RelationKind {
        forward: f.into(),
        inverse: i.into(),
        symmetric: s,
    };
    RelationKinds(vec![
        ("pm-of".into(), k("規劃（PM）", "PM 是", false)),
        ("feeds".into(), k("供料給", "吃料自", false)),
        ("derived-from".into(), k("衍生自", "衍生出", false)),
        ("upstream-of".into(), k("是其上游", "上游是", false)),
        ("sibling-topic".into(), k("同主題", "同主題", true)),
    ])
}

/// 對應 `registry.relation_kinds`：內建詞彙 ∪ config `relations.kinds`（私有層擴充；同名以 config 為準）。
pub fn relation_kinds(spine_dir: impl AsRef<Path>) -> Result<RelationKinds> {
    let mut kinds = builtin_relation_kinds();
    let cfg = crate::config::load(spine_dir)?;
    if let Some(serde_json::Value::Object(extra)) = cfg.get("relations.kinds") {
        let s = |v: Option<&serde_json::Value>| -> Option<String> {
            v.filter(|x| match x {
                serde_json::Value::Null => false,
                serde_json::Value::String(s) => !s.is_empty(),
                serde_json::Value::Bool(b) => *b,
                _ => true,
            })
            .map(crate::pystr::py_str)
        };
        for (k, v) in extra {
            if !v.is_object() {
                continue;
            }
            let forward = s(v.get("forward")).unwrap_or_else(|| k.clone());
            let inverse = s(v.get("inverse"))
                .or_else(|| s(v.get("forward")))
                .unwrap_or_else(|| k.clone());
            let symmetric = match v.get("symmetric") {
                Some(serde_json::Value::Bool(b)) => *b,
                Some(serde_json::Value::Null) | None => false,
                Some(serde_json::Value::String(x)) => !x.is_empty(),
                Some(serde_json::Value::Number(n)) => n.as_f64() != Some(0.0),
                Some(_) => true,
            };
            kinds.upsert(
                k.clone(),
                RelationKind {
                    forward,
                    inverse,
                    symmetric,
                },
            );
        }
    }
    Ok(kinds)
}

fn split_names<S: AsRef<str>>(names: &[S]) -> Vec<String> {
    names
        .iter()
        .flat_map(|x| {
            x.as_ref()
                .split(',')
                .map(|n| n.trim().to_string())
                .collect::<Vec<_>>()
        })
        .filter(|n| !n.is_empty())
        .collect()
}

/// 對應 `registry.relate`：設關係 a --kind--> b（例：a pm-of b＝a 是 b 的 PM）。同 (a,b,kind) 冪等
/// （note 取最新；沒給 note 不動舊註）。`exports`＝這條關係實際用到 a 的哪幾個 export
/// （每項可再以逗號分隔；空＝不動）；給了就覆寫。
pub fn relate<S: AsRef<str>>(
    spine_dir: impl AsRef<Path>,
    a: &str,
    b: &str,
    kind: &str,
    note: Option<&str>,
    exports: &[S],
) -> Result<Relation> {
    let spine_dir = spine_dir.as_ref();
    let kinds = relation_kinds(spine_dir)?;
    if !kinds.contains(kind) {
        return Err(Error::invalid(format!(
            "未知 kind: {kind}（可用：{}；config relations.kinds 可擴充）",
            kinds.names().join(", ")
        )));
    }
    if a == b {
        return Err(Error::invalid("repo 不能跟自己建關係"));
    }
    let names = split_names(exports);
    transaction(spine_dir, |data| {
        for x in [a, b] {
            if data.repo(x).is_none() {
                return Err(Error::invalid(format!("repo 不存在: {x}")));
            }
        }
        let src = data.repo_mut(a).expect("上面檢查過存在");
        let known: Vec<String> = src.exports().into_iter().map(|(k, _)| k).collect();
        let raw = src.raw_mut();
        if !matches!(raw.get("relations"), Some(Value::Sequence(_))) {
            raw.insert("relations".into(), Value::Sequence(Vec::new()));
        }
        let Some(Value::Sequence(rels)) = raw.get_mut("relations") else {
            unreachable!("剛設成 sequence")
        };
        let idx = rels.iter().position(|x| {
            x.get("to").and_then(Value::as_str) == Some(b)
                && x.get("kind").and_then(Value::as_str) == Some(kind)
        });
        let idx = match idx {
            Some(i) => i,
            None => {
                let mut m = Mapping::new();
                m.insert("to".into(), b.into());
                m.insert("kind".into(), kind.into());
                rels.push(Value::Mapping(m));
                rels.len() - 1
            }
        };
        let Some(entry) = rels[idx].as_mapping_mut() else {
            unreachable!("關係項是 mapping")
        };
        if let Some(n) = note.filter(|n| !n.is_empty()) {
            entry.insert("note".into(), n.into());
        }
        if !names.is_empty() {
            let unknown: Vec<&str> = names
                .iter()
                .filter(|n| !known.contains(n))
                .map(String::as_str)
                .collect();
            if !unknown.is_empty() {
                let now = if known.is_empty() {
                    "無".to_string()
                } else {
                    known.join(", ")
                };
                return Err(Error::invalid(format!(
                    "{a} 沒有 export: {}（先 `registry export {a} <名稱> <路徑>`；現有：{now}）",
                    unknown.join(", ")
                )));
            }
            entry.insert(
                "exports".into(),
                Value::Sequence(names.iter().map(|n| n.as_str().into()).collect()),
            );
        }
        Ok(relation_from_mapping(entry))
    })
}

/// `set_export` 的結果（對應 `{"repo", "name", "path", "desc"?}`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportSet {
    pub repo: String,
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
}

/// 對應 `registry.set_export`：登記（或覆寫）一個 export。path 相對 repo 根目錄，必須存在、不能跳出 repo。
pub fn set_export(
    spine_dir: impl AsRef<Path>,
    id: &str,
    name: &str,
    path: &str,
    desc: Option<&str>,
) -> Result<ExportSet> {
    let name = name.trim();
    if name.is_empty() || name.contains([':', '/', ' ']) {
        return Err(Error::invalid(format!(
            "export 名稱不能空白、不能含 ':' '/' 或空白：{}",
            crate::pystr::repr(name)
        )));
    }
    transaction(spine_dir, |data| {
        let r = data
            .repo_mut(id)
            .ok_or_else(|| Error::invalid(format!("repo 不存在: {id}")))?;
        let root = crate::pystr::resolve(&r.path());
        let rel = path.trim().trim_matches('/');
        let rel = if rel.is_empty() { "." } else { rel };
        let target = crate::pystr::resolve(&root.join(rel));
        if target != root && !target.starts_with(&root) {
            return Err(Error::invalid(format!("export 路徑跳出 repo：{path}")));
        }
        if !target.exists() {
            return Err(Error::invalid(format!(
                "export 路徑不存在：{}",
                root.join(rel).display()
            )));
        }
        let stored = format!(
            "{rel}{}",
            if target.is_dir() && rel != "." {
                "/"
            } else {
                ""
            }
        );
        let mut entry = Mapping::new();
        entry.insert("path".into(), stored.as_str().into());
        let desc = desc.filter(|d| !d.is_empty()).map(str::to_string);
        if let Some(d) = &desc {
            entry.insert("desc".into(), d.as_str().into());
        }
        let raw = r.raw_mut();
        if !matches!(raw.get("exports"), Some(Value::Mapping(_))) {
            raw.insert("exports".into(), Value::Mapping(Mapping::new()));
        }
        if let Some(Value::Mapping(ex)) = raw.get_mut("exports") {
            ex.insert(name.into(), Value::Mapping(entry));
        }
        Ok(ExportSet {
            repo: id.to_string(),
            name: name.to_string(),
            path: stored,
            desc,
        })
    })
}

/// 對應 `registry.remove_export`：刪 export；同時從引用它的關係 exports 清單拿掉。回傳是否真的刪了。
pub fn remove_export(spine_dir: impl AsRef<Path>, id: &str, name: &str) -> Result<bool> {
    transaction(spine_dir, |data| {
        let r = data
            .repo_mut(id)
            .ok_or_else(|| Error::invalid(format!("repo 不存在: {id}")))?;
        let raw = r.raw_mut();
        let Some(Value::Mapping(ex)) = raw.get_mut("exports") else {
            return Ok(false);
        };
        if ex.shift_remove(name).is_none() {
            return Ok(false);
        }
        if ex.is_empty() {
            raw.shift_remove("exports");
        }
        if let Some(Value::Sequence(rels)) = raw.get_mut("relations") {
            for x in rels.iter_mut().filter_map(Value::as_mapping_mut) {
                let Some(Value::Sequence(names)) = x.get_mut("exports") else {
                    continue;
                };
                if names.iter().any(|n| n.as_str() == Some(name)) {
                    names.retain(|n| n.as_str() != Some(name));
                    if names.is_empty() {
                        x.shift_remove("exports");
                    }
                }
            }
        }
        Ok(true)
    })
}

/// 對應 `registry.exports_of`：registry entry → {名稱: {path, desc?}}（沒有＝空）。
pub fn exports_of(entry: &Repo) -> Vec<(String, super::store::Export)> {
    entry.exports()
}

/// 對應 `registry.unrelate`：刪關係；kind=None＝刪 a→b 全部。回傳刪了幾筆。
pub fn unrelate(
    spine_dir: impl AsRef<Path>,
    a: &str,
    b: &str,
    kind: Option<&str>,
) -> Result<usize> {
    transaction(spine_dir, |data| {
        let src = data
            .repo_mut(a)
            .ok_or_else(|| Error::invalid(format!("repo 不存在: {a}")))?;
        let before: Vec<Value> = match src.get("relations") {
            Some(Value::Sequence(s)) => s.clone(),
            _ => Vec::new(),
        };
        let keep: Vec<Value> = before
            .iter()
            .filter(|x| {
                let hit = x.get("to").and_then(Value::as_str) == Some(b)
                    && kind.is_none_or(|k| x.get("kind").and_then(Value::as_str) == Some(k));
                !hit
            })
            .cloned()
            .collect();
        let removed = before.len() - keep.len();
        if keep.is_empty() {
            src.remove("relations");
        } else {
            src.set("relations", Value::Sequence(keep));
        }
        Ok(removed)
    })
}

/// `relations` 的一列：{from, to, kind, note?, exports?}。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelationRow {
    pub from: String,
    pub to: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<String>,
}

/// 對應 `registry.relations`：全部（或碰到 id 的）有向關係，依 registry 順序。
pub fn relations(spine_dir: impl AsRef<Path>, id: Option<&str>) -> Result<Vec<RelationRow>> {
    let mut out = Vec::new();
    for r in load(spine_dir)?.repos {
        let rid = r.id();
        for x in r.relations() {
            if id.is_none_or(|i| rid == i || x.to == i) {
                out.push(RelationRow {
                    from: rid.clone(),
                    to: x.to,
                    kind: x.kind,
                    note: x.note,
                    exports: x.exports,
                });
            }
        }
    }
    Ok(out)
}

/// 關係方向（站在某 repo 的立場）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// 我指向別人（用 forward 字）
    Out,
    /// 別人指向我（用 inverse 字）
    In,
}

/// `relations_of` 的一列：{peer, kind, direction, label, note?, exports?}。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelationView {
    pub peer: String,
    pub kind: String,
    pub direction: Direction,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<String>,
}

/// 對應 `registry.relations_of`：站在 id 的立場讀關係。
pub fn relations_of(spine_dir: impl AsRef<Path>, id: &str) -> Result<Vec<RelationView>> {
    let spine_dir = spine_dir.as_ref();
    let kinds = relation_kinds(spine_dir)?;
    Ok(relations(spine_dir, Some(id))?
        .into_iter()
        .map(|rel| {
            let (peer, direction, label) = if rel.from == id {
                (rel.to, Direction::Out, kinds.forward(&rel.kind).to_string())
            } else {
                (
                    rel.from,
                    Direction::In,
                    kinds.inverse(&rel.kind).to_string(),
                )
            };
            RelationView {
                peer,
                kind: rel.kind,
                direction,
                label,
                note: rel.note,
                exports: rel.exports,
            }
        })
        .collect())
}

/// 對應 `registry.related_ids`：一跳鄰居（兩向），不含自己——臨時組〔＋相關 repo〕與「挑」的建議模式用。
pub fn related_ids<S: AsRef<str>>(
    spine_dir: impl AsRef<Path>,
    ids: &[S],
) -> Result<BTreeSet<String>> {
    let ids: BTreeSet<&str> = ids.iter().map(AsRef::as_ref).collect();
    let mut out = BTreeSet::new();
    for rel in relations(spine_dir, None)? {
        if ids.contains(rel.from.as_str()) {
            out.insert(rel.to.clone());
        }
        if ids.contains(rel.to.as_str()) {
            out.insert(rel.from.clone());
        }
    }
    Ok(out
        .into_iter()
        .filter(|x| !ids.contains(x.as_str()))
        .collect())
}

/// 對應 `registry.relation_lines`：人話行（組情境卡用）——組內兩兩關係＋指向組外的關係（標「組外」）。
pub fn relation_lines<S: AsRef<str>>(
    spine_dir: impl AsRef<Path>,
    ids: &[S],
) -> Result<Vec<String>> {
    let spine_dir = spine_dir.as_ref();
    let ids: Vec<&str> = ids.iter().map(AsRef::as_ref).collect();
    let kinds = relation_kinds(spine_dir)?;
    let mut lines = Vec::new();
    for rel in relations(spine_dir, None)? {
        let (fin, tin) = (
            ids.contains(&rel.from.as_str()),
            ids.contains(&rel.to.as_str()),
        );
        if !fin && !tin {
            continue;
        }
        let note = rel
            .note
            .as_ref()
            .map(|n| format!("（{n}）"))
            .unwrap_or_default();
        let outside = if !fin {
            format!("　←（{} 在組外）", rel.from)
        } else if !tin {
            format!("　→（{} 在組外）", rel.to)
        } else {
            String::new()
        };
        lines.push(format!(
            "{} {}→ {}{note}{outside}",
            rel.from,
            kinds.forward(&rel.kind),
            rel.to
        ));
    }
    Ok(lines)
}
