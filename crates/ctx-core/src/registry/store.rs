//! registry.yaml 的讀寫：load、原子寫入、transaction（讀–改–寫同一把 lock）。對應 `registry/store.py`。
//!
//! 資料模型：registry.yaml 是人也會手改的檔，Python 版寫什麼 Rust 要讀得回、反之亦然，
//! 所以 repo／組都以 `serde_yaml::Mapping`（保鍵序、保留不認得的鍵）為底，外面包 typed 存取器
//! （[`Repo`]、[`Group`]）。頂層不認得的鍵也照原順序寫回。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

use crate::locking::FileLock;
use crate::pystr::py_str_yaml;
use crate::spine::LOCK_NAME;
use crate::Result;

fn key(k: &str) -> Value {
    Value::String(k.to_string())
}

/// YAML 值當字串看（非字串以 Python `str()` 轉；null＝None）。
fn as_string(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(py_str_yaml(other)),
    }
}

fn seq_strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Sequence(s)) => s.iter().filter_map(as_string).collect(),
        _ => Vec::new(),
    }
}

/// Python 的真值判斷（`if r.get(k):`；沒有這個鍵＝假）。
pub(crate) fn truthy(v: Option<&Value>) -> bool {
    v.is_some_and(crate::pystr::truthy_yaml)
}

macro_rules! mapping_wrapper {
    ($name:ident) => {
        impl $name {
            /// 原始 mapping（保鍵序）。
            pub fn raw(&self) -> &Mapping {
                &self.0
            }
            pub fn raw_mut(&mut self) -> &mut Mapping {
                &mut self.0
            }
            pub fn get(&self, k: &str) -> Option<&Value> {
                self.0.get(k)
            }
            /// 字串欄位（null／不存在＝None；非字串以 Python `str()` 轉）。
            pub fn get_str(&self, k: &str) -> Option<String> {
                self.0.get(k).and_then(as_string)
            }
            /// Python `if entry.get(k):`。
            pub fn has(&self, k: &str) -> bool {
                truthy(self.0.get(k))
            }
            /// 設欄位：既有鍵原位更新，新鍵接在最後（同 Python dict）。
            pub fn set(&mut self, k: &str, v: impl Into<Value>) {
                self.0.insert(key(k), v.into());
            }
            /// 刪欄位（保其餘鍵序）。
            pub fn remove(&mut self, k: &str) -> Option<Value> {
                self.0.shift_remove(k)
            }
            /// 轉 JSON（給 MCP／工作台／`--json`）。
            pub fn to_json(&self) -> serde_json::Value {
                crate::config::yaml_to_json(&Value::Mapping(self.0.clone()))
            }
        }
    };
}

/// registry 的一個 repo 登記（`repos:` 底下一項）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Repo(pub Mapping);
mapping_wrapper!(Repo);

/// registry 的一個組（`groups:` 底下一項）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Group(pub Mapping);
mapping_wrapper!(Group);

/// 關係（repo 的 `relations:` 一項）的 typed 視圖。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Relation {
    pub to: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<String>,
}

/// export（repo 的 `exports:` 一項）的 typed 視圖。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Export {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
}

/// 關係項 mapping → typed 視圖（缺 to／kind 的以空字串表示）。
pub(crate) fn relation_from_mapping(m: &Mapping) -> Relation {
    Relation {
        to: m.get("to").and_then(as_string).unwrap_or_default(),
        kind: m.get("kind").and_then(as_string).unwrap_or_default(),
        note: m
            .get("note")
            .filter(|v| truthy(Some(v)))
            .and_then(as_string),
        exports: seq_strings(m.get("exports")),
    }
}

impl Repo {
    pub fn id(&self) -> String {
        self.get_str("id").unwrap_or_default()
    }
    /// 登記的路徑字串（原樣，未展開 `~`）。
    pub fn path_str(&self) -> String {
        self.get_str("path").unwrap_or_default()
    }
    /// 對應 `Path(r["path"]).expanduser()`。
    pub fn path(&self) -> PathBuf {
        crate::fsutil::expanduser(self.path_str())
    }
    /// `type` 欄（mine｜external）。
    pub fn kind(&self) -> Option<String> {
        self.get_str("type")
    }
    /// `tier` 欄字串（可能是手寫的未知值）。
    pub fn tier_str(&self) -> Option<String> {
        self.get_str("tier")
    }
    /// `tier` 欄解析成 [`Tier`]（未知值＝None）。
    pub fn tier(&self) -> Option<super::Tier> {
        self.tier_str().and_then(|t| t.parse().ok())
    }
    pub fn tags(&self) -> Vec<String> {
        seq_strings(self.get("tags"))
    }
    pub fn upstream(&self) -> Option<String> {
        self.get_str("upstream").filter(|s| !s.is_empty())
    }
    /// Rust 才有（2026-10-13）：一句話介紹（給名冊與搜尋；沒填時 [`crate::profile`] 從 README 推）。
    pub fn description(&self) -> Option<String> {
        self.get_str("description").filter(|s| !s.trim().is_empty())
    }
    /// Rust 才有：協作角色（一個短詞，例「知識庫」「規格來源」）。
    pub fn role(&self) -> Option<String> {
        self.get_str("role").filter(|s| !s.trim().is_empty())
    }
    /// Rust 才有：搜尋用關鍵字（自由寫；tags 是給人分類篩選的）。
    pub fn keywords(&self) -> Vec<String> {
        seq_strings(self.get("keywords"))
    }
    pub fn origin(&self) -> Option<String> {
        self.get_str("origin").filter(|s| !s.is_empty())
    }
    /// `relations:` 的 typed 視圖（缺 to／kind 的以空字串表示）。
    pub fn relations(&self) -> Vec<Relation> {
        let Some(Value::Sequence(seq)) = self.get("relations") else {
            return Vec::new();
        };
        seq.iter()
            .filter_map(|x| x.as_mapping())
            .map(relation_from_mapping)
            .collect()
    }
    /// 對應 `registry.exports_of`：{名稱: {path, desc?}}（依登記順序）。
    pub fn exports(&self) -> Vec<(String, Export)> {
        let Some(Value::Mapping(m)) = self.get("exports") else {
            return Vec::new();
        };
        m.iter()
            .filter_map(|(k, v)| {
                let name = as_string(k)?;
                let em = v.as_mapping();
                Some((
                    name,
                    Export {
                        path: em
                            .and_then(|e| e.get("path"))
                            .and_then(as_string)
                            .unwrap_or_default(),
                        desc: em
                            .and_then(|e| e.get("desc"))
                            .filter(|v| truthy(Some(v)))
                            .and_then(as_string),
                    },
                ))
            })
            .collect()
    }
}

impl Group {
    pub fn name(&self) -> String {
        self.get_str("name").unwrap_or_default()
    }
    pub fn members(&self) -> Vec<String> {
        seq_strings(self.get("members"))
    }
    pub fn set_members(&mut self, members: &[String]) {
        self.set(
            "members",
            Value::Sequence(members.iter().map(|m| key(m)).collect()),
        );
    }
    pub fn goal(&self) -> Option<String> {
        self.get_str("goal").filter(|s| !s.is_empty())
    }
    pub fn landing(&self) -> Option<String> {
        self.get_str("landing")
    }
}

/// registry.yaml 整份。`repos`／`groups` 以外的頂層鍵保留在 `rest`（含原順序）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RegistryData {
    pub repos: Vec<Repo>,
    pub groups: Vec<Group>,
    /// 頂層原始 mapping；`repos`／`groups` 的位置以佔位值保留，寫回時換成上面兩個欄位。
    rest: Mapping,
}

impl RegistryData {
    pub fn repo(&self, id: &str) -> Option<&Repo> {
        self.repos.iter().find(|r| r.id() == id)
    }
    pub fn repo_mut(&mut self, id: &str) -> Option<&mut Repo> {
        self.repos.iter_mut().find(|r| r.id() == id)
    }
    pub fn group(&self, name: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.name() == name)
    }
    pub fn group_mut(&mut self, name: &str) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.name() == name)
    }
    /// 頂層其他鍵（不含 repos／groups）。
    pub fn extra(&self, k: &str) -> Option<&Value> {
        if k == "repos" || k == "groups" {
            return None;
        }
        self.rest.get(k)
    }

    fn from_value(v: Value) -> Result<Self> {
        let mut rest = match v {
            Value::Mapping(m) => m,
            Value::Null => Mapping::new(),
            _ => return Err(crate::Error::invalid("registry.yaml 頂層必須是 mapping")),
        };
        let mut take = |k: &str| -> Vec<Mapping> {
            // setdefault：沒有就接在最後（同 Python）
            let v = rest.insert(key(k), Value::Null).unwrap_or(Value::Null);
            match v {
                Value::Sequence(s) => s
                    .into_iter()
                    .filter_map(|x| x.as_mapping().cloned())
                    .collect(),
                _ => Vec::new(),
            }
        };
        let repos = take("repos").into_iter().map(Repo).collect();
        let groups = take("groups").into_iter().map(Group).collect();
        Ok(RegistryData {
            repos,
            groups,
            rest,
        })
    }

    /// 轉回 YAML 值（保頂層鍵序）。
    pub fn to_value(&self) -> Value {
        let mut out = Mapping::new();
        let put = |m: &mut Mapping, k: &Value, v: &Value| {
            let v = match k.as_str() {
                Some("repos") => Value::Sequence(
                    self.repos
                        .iter()
                        .map(|r| Value::Mapping(r.0.clone()))
                        .collect(),
                ),
                Some("groups") => Value::Sequence(
                    self.groups
                        .iter()
                        .map(|g| Value::Mapping(g.0.clone()))
                        .collect(),
                ),
                _ => v.clone(),
            };
            m.insert(k.clone(), v);
        };
        for (k, v) in &self.rest {
            put(&mut out, k, v);
        }
        for k in ["repos", "groups"] {
            if !out.contains_key(k) {
                put(&mut out, &key(k), &Value::Null);
            }
        }
        Value::Mapping(out)
    }
}

fn path_of(spine_dir: &Path) -> PathBuf {
    spine_dir.join("registry.yaml")
}

/// 對應 `registry.load`（不拿 lock；讀到的永遠是完整的一份，因為寫入是原子的）。
pub fn load(spine_dir: impl AsRef<Path>) -> Result<RegistryData> {
    let p = path_of(spine_dir.as_ref());
    if !p.exists() {
        return RegistryData::from_value(Value::Null);
    }
    let text = std::fs::read_to_string(&p)?;
    let v: Value = if text.trim().is_empty() {
        Value::Null
    } else {
        serde_yaml::from_str(&text)?
    };
    RegistryData::from_value(v)
}

/// 對應 `store._write`：原子寫入——先寫暫存檔 `.registry.yaml.<pid>.tmp` 再 rename，
/// 讀的人（不拿 lock）永遠讀到完整的一份。
fn write(spine_dir: &Path, data: &RegistryData) -> Result<()> {
    let p = path_of(spine_dir);
    let text = crate::yamlout::to_string(&data.to_value());
    let tmp = spine_dir.join(format!(".registry.yaml.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

/// 對應 `registry.save`：整份覆寫（不讀舊檔）。改既有資料請用 [`transaction`]，否則會蓋掉別人同時寫的。
pub fn save(spine_dir: impl AsRef<Path>, data: &RegistryData) -> Result<()> {
    let d = spine_dir.as_ref();
    let _g = FileLock::new(d.join(LOCK_NAME)).acquire()?;
    write(d, data)
}

/// 對應 `registry.transaction`：讀–改–寫在同一把 lock 裡（2026-09-25 架構檢查實測：只鎖寫入那一刻，
/// 2 進程各寫 40 筆只剩 41）。`f` 回錯＝不寫回。lock 不可重入：`f` 裡別再呼叫會寫入的函式。
pub fn transaction<T>(
    spine_dir: impl AsRef<Path>,
    f: impl FnOnce(&mut RegistryData) -> Result<T>,
) -> Result<T> {
    let d = spine_dir.as_ref();
    let _g = FileLock::new(d.join(LOCK_NAME)).acquire()?;
    let mut data = load(d)?;
    let out = f(&mut data)?;
    write(d, &data)?;
    Ok(out)
}
