//! 操作表：管理類寫入動作（registry／關係／export／組）只定義一次，CLI／工作台／MCP 共用。
//! 對應 Python `repo_context/ops.py`。
//!
//! 2026-09-25 架構檢查：加一個功能要改 registry → CLI → webui → MCP 四處，連「記一筆 decision」
//! 都各寫一份、措辭還不一樣。這裡每個操作宣告一次：參數（同時是 MCP inputSchema）、實作、
//! 給人看的一行結果、留痕。入口只做轉接：
//!
//! ```ignore
//! ops::call(spine_dir, "group_update", json!({"name": "g1", "add": ["x"]}), Via::Cli)
//! ```
//!
//! - 參數驗證、留痕、錯誤型別（`Error::Invalid`）在這裡統一；入口各自把錯誤轉成自己的形狀
//!   （CLI 印出、工作台回 {"error"}、MCP 回 isError）。
//! - 留痕的 source 依入口：cli → [cli]、工作台 → [monitor]（都是使用者自己出手，不進收件匣）；
//!   MCP → [agent]（agent 改了你的 registry，要讓你看得到）。
//! - 新增管理操作：在 [`ops`] 表加一項，MCP 管理層自動多一個工具；CLI／工作台要不要開入口另外決定。

use std::path::Path;
use std::str::FromStr;

use chrono::Local;
use serde_json::{json, Map, Value};
use std::sync::LazyLock;

use crate::pystr::py_str;
use crate::registry::{self, AddRepo, GroupUpdate};
use crate::{spine, Error, Result};

/// 呼叫入口（決定留痕的 source 與標籤）。對應 `ops.VIA`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Cli,
    Ui,
    Mcp,
}

impl Via {
    /// 留痕事件的 source。
    pub fn source(self) -> &'static str {
        match self {
            Via::Cli => "cli",
            Via::Ui => "monitor",
            Via::Mcp => "agent",
        }
    }
    /// 留痕 body 結尾的標籤。
    pub fn label(self) -> &'static str {
        match self {
            Via::Cli => "從 CLI",
            Via::Ui => "從工作台",
            Via::Mcp => "從 MCP",
        }
    }
}

impl FromStr for Via {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "cli" => Ok(Via::Cli),
            "ui" => Ok(Via::Ui),
            "mcp" => Ok(Via::Mcp),
            _ => Err(Error::invalid(format!("未知入口: {s}"))),
        }
    }
}

/// 參數型別（對應 `ops.STR`／`ARR`／`BOOL`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    Str,
    Arr,
    Bool,
}

/// 一個參數宣告。
#[derive(Debug, Clone)]
pub struct Param {
    pub name: &'static str,
    pub ty: ParamType,
    pub desc: &'static str,
    pub required: bool,
}

/// 已過濾（去掉 null）的參數。
#[derive(Debug, Clone, Default)]
pub struct Args(pub Map<String, Value>);

impl Args {
    pub fn get(&self, k: &str) -> Option<&Value> {
        self.0.get(k)
    }
    /// 字串參數（非字串以 Python `str()` 轉）。
    pub fn s(&self, k: &str) -> Option<String> {
        self.0.get(k).map(py_str)
    }
    /// 必填字串參數（`call` 已驗過存在）。
    fn req(&self, k: &str) -> String {
        self.s(k).unwrap_or_default()
    }
    /// 對應 `ops._csv`：CLI 給逗號字串、MCP／工作台給陣列——都收。
    pub fn csv(&self, k: &str) -> Option<Vec<String>> {
        let v = self.0.get(k)?;
        let items: Vec<String> = match v {
            Value::Array(a) => a.iter().map(py_str).collect(),
            other => vec![py_str(other)],
        };
        Some(
            items
                .iter()
                .flat_map(|x| {
                    x.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .collect(),
        )
    }
    /// Python 真值（`if remove:`）。
    pub fn flag(&self, k: &str) -> bool {
        self.0.get(k).is_some_and(crate::pystr::truthy)
    }
}

type RunFn = fn(&Path, &Args) -> Result<Value>;
type SayFn = fn(&Value, &Args) -> String;
type AuditFn = fn(&Value, &Args) -> Option<(Vec<String>, String)>;

/// 一個操作（對應 `ops.Op`）。
pub struct Op {
    pub name: &'static str,
    pub summary: &'static str,
    pub params: Vec<Param>,
    run: RunFn,
    say: SayFn,
    audit: Option<AuditFn>,
}

impl Op {
    /// 對應 `Op.schema`：MCP inputSchema。
    pub fn schema(&self) -> Value {
        let mut props = Map::new();
        for p in &self.params {
            let mut o = Map::new();
            match p.ty {
                ParamType::Str => {
                    o.insert("type".into(), json!("string"));
                }
                ParamType::Arr => {
                    o.insert("type".into(), json!("array"));
                    o.insert("items".into(), json!({"type": "string"}));
                }
                ParamType::Bool => {
                    o.insert("type".into(), json!("boolean"));
                }
            }
            o.insert("description".into(), json!(p.desc));
            props.insert(p.name.into(), Value::Object(o));
        }
        let required: Vec<&str> = self
            .params
            .iter()
            .filter(|p| p.required)
            .map(|p| p.name)
            .collect();
        json!({"type": "object", "properties": props, "required": required})
    }
}

fn p(name: &'static str, ty: ParamType, desc: &'static str, required: bool) -> Param {
    Param {
        name,
        ty,
        desc,
        required,
    }
}

use ParamType::{Arr, Bool, Str};

/// 結果欄位當字串讀（Python `f"{r['x']}"`）。
fn rs(r: &Value, k: &str) -> String {
    r.get(k).map(py_str).unwrap_or_else(|| "None".into())
}

fn rlist(r: &Value, k: &str) -> Vec<String> {
    match r.get(k) {
        Some(Value::Array(a)) => a.iter().map(py_str).collect(),
        _ => Vec::new(),
    }
}

fn rtruthy(r: &Value, k: &str) -> bool {
    Args(r.as_object().cloned().unwrap_or_default()).flag(k)
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<Value> {
    Ok(serde_json::to_value(v)?)
}

// ── repo ────────────────────────────────────────────────────────────────

fn registry_add(d: &Path, a: &Args) -> Result<Value> {
    let opts = AddRepo {
        kind: a.s("type").unwrap_or_else(|| "mine".into()),
        tier: a.s("tier").unwrap_or_else(|| "active".into()),
        tags: a.csv("tags").unwrap_or_default(),
        upstream: a.s("upstream"),
        origin: None,
    };
    Ok(registry::add_repo(d, &a.req("id"), a.req("path"), opts)?.to_json())
}

fn registry_remove(d: &Path, a: &Args) -> Result<Value> {
    let id = a.req("id");
    registry::remove_repo(d, &id)?;
    Ok(json!({"id": id}))
}

fn registry_set(d: &Path, a: &Args) -> Result<Value> {
    let v = crate::config::json_to_yaml(a.get("value").unwrap_or(&Value::Null));
    Ok(registry::set_field(d, &a.req("id"), &a.req("key"), v)?.to_json())
}

fn registry_tier(d: &Path, a: &Args) -> Result<Value> {
    let (id, tier) = (a.req("id"), a.req("tier"));
    if !registry::TIERS.contains(&tier.as_str()) {
        return Err(Error::invalid(format!(
            "tier 需為 {}",
            registry::TIERS.join("|")
        )));
    }
    let old = registry::get_repo(d, &id)?.get_str("tier");
    registry::set_field(d, &id, "tier", tier.as_str())?;
    if tier == "paused" {
        if let Some(rw) = a.s("resume_when").filter(|s| !s.is_empty()) {
            registry::set_field(d, &id, "resume_when", rw)?;
        }
    }
    Ok(json!({"id": id, "tier": tier, "old": old}))
}

fn registry_tag(d: &Path, a: &Args) -> Result<Value> {
    let id = a.req("id");
    let tags = registry::tag_repo(
        d,
        &id,
        &a.csv("add").unwrap_or_default(),
        &a.csv("remove").unwrap_or_default(),
    )?;
    Ok(json!({"id": id, "tags": tags}))
}

fn forward(d: &Path, kind: &str) -> String {
    registry::relation_kinds(d)
        .ok()
        .map(|k| k.forward(kind).to_string())
        .unwrap_or_else(|| kind.to_string())
}

fn registry_relate(d: &Path, a: &Args) -> Result<Value> {
    let kind = a.req("kind");
    let exports = a.csv("exports").unwrap_or_default();
    let e = registry::relate(
        d,
        &a.req("a"),
        &a.req("b"),
        &kind,
        a.s("note").as_deref(),
        &exports,
    )?;
    let mut v = to_json(&e)?;
    if let Some(o) = v.as_object_mut() {
        o.insert("label".into(), json!(forward(d, &kind)));
    }
    Ok(v)
}

fn registry_unrelate(d: &Path, a: &Args) -> Result<Value> {
    let n = registry::unrelate(d, &a.req("a"), &a.req("b"), a.s("kind").as_deref())?;
    Ok(json!({"removed": n}))
}

fn registry_export(d: &Path, a: &Args) -> Result<Value> {
    let (id, name) = (a.req("id"), a.req("name"));
    if a.flag("remove") {
        return Ok(
            json!({"removed_flag": true, "removed": registry::remove_export(d, &id, &name)?}),
        );
    }
    let path = a.s("path").filter(|s| !s.is_empty());
    let Some(path) = path else {
        return Err(Error::invalid("registry_export 登記時需要 path"));
    };
    to_json(&registry::set_export(
        d,
        &id,
        &name,
        &path,
        a.s("desc").as_deref(),
    )?)
}

// ── 組 ──────────────────────────────────────────────────────────────────

fn group_add(d: &Path, a: &Args) -> Result<Value> {
    let name = a.req("name").trim().to_string();
    if name.is_empty() {
        return Err(Error::invalid("組名不可為空"));
    }
    registry::add_group(d, &name, &a.csv("members").unwrap_or_default())?;
    Ok(registry::get_group(d, &name)?.to_json())
}

fn group_update_body(r: &Value) -> String {
    let mut parts = Vec::new();
    if rtruthy(r, "renamed_from") {
        parts.push(format!(
            "改名 {} → {}",
            rs(r, "renamed_from"),
            rs(&r["group"], "name")
        ));
    }
    let added = rlist(r, "added");
    if !added.is_empty() {
        parts.push(format!("加入 {}", added.join(",")));
    }
    let removed = rlist(r, "removed");
    if !removed.is_empty() {
        parts.push(format!("移出 {}（登記保留）", removed.join(",")));
    }
    format!(
        "編輯組：{}",
        if parts.is_empty() {
            "無變更".to_string()
        } else {
            parts.join("；")
        }
    )
}

fn group_update_say(r: &Value, _a: &Args) -> String {
    let g = &r["group"];
    let members = rlist(g, "members");
    let mut out = vec![format!(
        "{}：{}",
        rs(g, "name"),
        if members.is_empty() {
            "（空組）".to_string()
        } else {
            members.join(",")
        }
    )];
    if rtruthy(r, "moved_dir") {
        out.push(format!(
            "groups/{}/ 已搬到 groups/{}/",
            rs(r, "renamed_from"),
            rs(g, "name")
        ));
    }
    let refs = rlist(r, "schedule_refs");
    if !refs.is_empty() {
        out.push(format!(
            "注意：config.yaml 排程還指著「{}」：{}（請自行更新）",
            rs(r, "renamed_from"),
            refs.join(", ")
        ));
    }
    out.join("\n")
}

fn group_update(d: &Path, a: &Args) -> Result<Value> {
    let (add, remove, members, rename) = (
        a.csv("add"),
        a.csv("remove"),
        a.csv("members"),
        a.s("rename"),
    );
    if add.is_none() && remove.is_none() && members.is_none() && rename.is_none() {
        return Err(Error::invalid(
            "沒有要改的：給 add／remove／members／rename 至少一項",
        ));
    }
    if members.as_ref().is_some_and(Vec::is_empty) {
        return Err(Error::invalid(
            "members 不可為空（要整組拿掉請用 group_remove）",
        ));
    }
    let r = registry::update_group(
        d,
        &a.req("name"),
        GroupUpdate {
            add,
            remove,
            members,
            rename,
        },
    )?;
    let mut v = to_json(&r)?;
    v["group"] = r.group.to_json();
    Ok(v)
}

fn group_remove(d: &Path, a: &Args) -> Result<Value> {
    to_json(&registry::remove_group(d, &a.req("name"))?)
}

fn group_goal(d: &Path, a: &Args) -> Result<Value> {
    let name = a.req("name").trim().to_string();
    let text = a.s("text").unwrap_or_default().trim().to_string();
    if text.chars().count() > 300 {
        return Err(Error::invalid("目標請寫一句話（300 字以內）"));
    }
    let now = Local::now().naive_local();
    registry::set_group_field(d, &name, "goal", &text)?;
    let at = if text.is_empty() {
        String::new()
    } else {
        now.format("%Y-%m-%d %H:%M").to_string()
    };
    registry::set_group_field(d, &name, "goal_at", &at)?;
    Ok(json!({"group": name, "goal": text}))
}

static OPS: LazyLock<Vec<Op>> = LazyLock::new(|| {
    vec![
        Op {
            name: "registry_add",
            summary: "登記一個 repo（id＋路徑）",
            params: vec![
                p("id", Str, "repo id", true),
                p("path", Str, "repo 路徑", true),
                p("type", Str, "mine 或 external", false),
                p("tier", Str, "active／paused／dormant／archived", false),
                p("tags", Arr, "tag 清單", false),
                p("upstream", Str, "external 的 GitHub owner/repo", false),
            ],
            run: registry_add,
            say: |r, _| format!("已登記：{} ({}/{})", rs(r, "id"), rs(r, "type"), rs(r, "tier")),
            audit: Some(|r, _| {
                Some((vec![format!("repo:{}", rs(r, "id"))], format!("登記 repo：{}（{}）", rs(r, "id"), rs(r, "path"))))
            }),
        },
        Op {
            name: "registry_remove",
            summary: "把 repo 移出 registry（同時清掉所有組的 membership 與指向它的關係）",
            params: vec![p("id", Str, "repo id", true)],
            run: registry_remove,
            say: |_, a| format!("已移除：{}（含所有組的 membership）", a.req("id")),
            audit: Some(|_, a| Some((vec![], format!("移出 registry：{}", a.req("id"))))),
        },
        Op {
            name: "registry_set",
            summary: "改 repo 的一個欄位（例 note、resume_when）",
            params: vec![p("id", Str, "repo id", true), p("key", Str, "欄位名", true), p("value", Str, "值", true)],
            run: registry_set,
            say: |_, a| format!("已更新：{}.{} = {}", a.req("id"), a.req("key"), a.req("value")),
            audit: Some(|_, a| {
                Some((vec![format!("repo:{}", a.req("id"))], format!("{}.{} = {}", a.req("id"), a.req("key"), a.req("value"))))
            }),
        },
        Op {
            name: "registry_tier",
            summary: "改 repo 的 tier（paused 可一併給復工條件）",
            params: vec![
                p("id", Str, "repo id", true),
                p("tier", Str, "active／paused／dormant／archived", true),
                p("resume_when", Str, "paused 的復工條件", false),
            ],
            run: registry_tier,
            say: |r, _| format!("{}：tier {} → {}", rs(r, "id"), rs(r, "old"), rs(r, "tier")),
            audit: Some(|r, _| {
                Some((vec![format!("repo:{}", rs(r, "id"))], format!("tier {} → {}", rs(r, "old"), rs(r, "tier"))))
            }),
        },
        Op {
            name: "registry_tag",
            summary: "加／移除 repo 的 tag（冪等）",
            params: vec![
                p("id", Str, "repo id", true),
                p("add", Arr, "要加的 tag", false),
                p("remove", Arr, "要移除的 tag", false),
            ],
            run: registry_tag,
            say: |r, _| {
                let tags = rlist(r, "tags");
                format!("{} tags: {}", rs(r, "id"), if tags.is_empty() { "（無）".to_string() } else { tags.join(", ") })
            },
            audit: Some(|r, a| {
                let mut parts: Vec<String> = a.csv("add").unwrap_or_default().iter().map(|t| format!("+{t}")).collect();
                parts.extend(a.csv("remove").unwrap_or_default().iter().map(|t| format!("-{t}")));
                Some((vec![format!("repo:{}", rs(r, "id"))], format!("tag {}", parts.join(" "))))
            }),
        },
        Op {
            name: "registry_relate",
            summary: "設 repo 間關係 a --kind--> b（例 a pm-of b＝a 是 b 的 PM；kind: pm-of/feeds/derived-from/upstream-of/sibling-topic）",
            params: vec![
                p("a", Str, "來源 repo", true),
                p("b", Str, "目標 repo", true),
                p("kind", Str, "關係種類", true),
                p("note", Str, "一句說明", false),
                p("exports", Str, "逗號分隔：這條關係用到 a 的哪些 export", false),
            ],
            run: registry_relate,
            say: |r, a| {
                let ex = rlist(r, "exports");
                format!("已設關係：{} {}→ {}（{}）", a.req("a"), rs(r, "label"), a.req("b"), rs(r, "kind"))
                    + &if ex.is_empty() { String::new() } else { format!("，用到 export：{}", ex.join("、")) }
            },
            audit: Some(|r, a| {
                Some((
                    vec![format!("repo:{}", a.req("a"))],
                    format!("關係：{} {}→ {}（{}）", a.req("a"), rs(r, "label"), a.req("b"), rs(r, "kind")),
                ))
            }),
        },
        Op {
            name: "registry_unrelate",
            summary: "刪 repo 間關係（不給 kind＝刪 a→b 全部）",
            params: vec![
                p("a", Str, "來源 repo", true),
                p("b", Str, "目標 repo", true),
                p("kind", Str, "關係種類", false),
            ],
            run: registry_unrelate,
            say: |r, a| format!("已刪 {} 筆關係：{} → {}", rs(r, "removed"), a.req("a"), a.req("b")),
            audit: Some(|r, a| {
                if !rtruthy(r, "removed") {
                    return None;
                }
                let kind = a.s("kind").filter(|k| !k.is_empty()).map(|k| format!("（{k}）")).unwrap_or_default();
                Some((vec![format!("repo:{}", a.req("a"))], format!("刪關係：{} → {}{kind}", a.req("a"), a.req("b"))))
            }),
        },
        Op {
            name: "registry_export",
            summary: "登記 repo 對外提供的東西（export）：名稱→repo 內路徑；remove=true 刪除。agent 之後用 `<repo>:<名稱>/子路徑` 引用",
            params: vec![
                p("id", Str, "repo id", true),
                p("name", Str, "export 名稱", true),
                p("path", Str, "repo 內路徑（登記時必填）", false),
                p("desc", Str, "一句說明", false),
                p("remove", Bool, "true＝刪除", false),
            ],
            run: registry_export,
            say: |r, a| {
                if !rtruthy(r, "removed_flag") {
                    let desc = if rtruthy(r, "desc") { format!("（{}）", rs(r, "desc")) } else { String::new() };
                    format!("已登記 export：{}:{} → {}{desc}", rs(r, "repo"), rs(r, "name"), rs(r, "path"))
                } else if rtruthy(r, "removed") {
                    format!("已刪 export：{}:{}", a.req("id"), a.req("name"))
                } else {
                    format!("{} 沒有 export {}", a.req("id"), a.req("name"))
                }
            },
            audit: Some(|r, a| {
                let removed_flag = rtruthy(r, "removed_flag");
                if removed_flag && !rtruthy(r, "removed") {
                    return None;
                }
                let body = if !removed_flag {
                    format!("export {}:{} → {}", a.req("id"), a.req("name"), rs(r, "path"))
                } else {
                    format!("刪 export {}:{}", a.req("id"), a.req("name"))
                };
                Some((vec![format!("repo:{}", a.req("id"))], body))
            }),
        },
        Op {
            name: "group_add",
            summary: "建一個組",
            params: vec![p("name", Str, "組名", true), p("members", Arr, "repo id 清單", true)],
            run: group_add,
            say: |r, _| format!("組已建：{}", rs(r, "name")),
            audit: Some(|r, _| Some((vec![format!("group:{}", rs(r, "name"))], format!("建組：{}", rlist(r, "members").join(","))))),
        },
        Op {
            name: "group_update",
            summary: "改組：add／remove 組員（repo 登記不動）、members 整份換掉、rename 改名；可一次給多項，任何一項失敗＝全部不改",
            params: vec![
                p("name", Str, "組名", true),
                p("add", Arr, "要加入的 repo id", false),
                p("remove", Arr, "要移出的 repo id", false),
                p("members", Arr, "新的完整成員名單（取代現有）", false),
                p("rename", Str, "新組名", false),
            ],
            run: group_update,
            say: group_update_say,
            audit: Some(|r, _| Some((vec![format!("group:{}", rs(&r["group"], "name"))], group_update_body(r)))),
        },
        Op {
            name: "group_remove",
            summary: "刪除一個組（只刪組定義；repo 登記與 groups/<組>/ 的料保留）",
            params: vec![p("name", Str, "組名", true)],
            run: group_remove,
            say: |r, _| {
                let mut out = vec![format!("已刪除組：{}（repo 登記保留）", rs(r, "name"))];
                if rtruthy(r, "kept_dir") {
                    out.push(format!("保留了這組的料：{}", rs(r, "kept_dir")));
                }
                let refs = rlist(r, "schedule_refs");
                if !refs.is_empty() {
                    out.push(format!(
                        "注意：config.yaml 排程還指著「{}」：{}（請自行更新）",
                        rs(r, "name"),
                        refs.join(", ")
                    ));
                }
                out.join("\n")
            },
            audit: Some(|r, _| Some((vec![format!("group:{}", rs(r, "name"))], "刪除組（repo 登記保留）".to_string()))),
        },
        Op {
            name: "group_goal",
            summary: "記下（或清除）組的一句話目標——目標只有人能給，agent 別自己編",
            params: vec![p("name", Str, "組名", true), p("text", Str, "一句話目標；空字串＝清除", false)],
            run: group_goal,
            say: |r, _| {
                if rtruthy(r, "goal") {
                    format!("目標已記在 {}", rs(r, "group"))
                } else {
                    format!("已清除 {} 的目標", rs(r, "group"))
                }
            },
            audit: Some(|r, _| {
                let body = if rtruthy(r, "goal") { format!("本組目標：{}", rs(r, "goal")) } else { "清除本組目標".to_string() };
                Some((vec![format!("group:{}", rs(r, "group"))], body))
            }),
        },
    ]
});

/// 對應 `ops.OPS`：全部操作（宣告順序）。
pub fn ops() -> &'static [Op] {
    &OPS
}

/// 依名稱找操作。
pub fn get(name: &str) -> Option<&'static Op> {
    OPS.iter().find(|o| o.name == name)
}

/// 對應 `ops.call`：跑一個操作——驗參數 → 執行 → 留痕。回傳 (結果, 一行人話)。錯誤一律 `Error::Invalid`
/// （registry 的錯也是；IO／鎖逾時等照原型別）。`args` 是 JSON 物件（null 值視為沒給）。
pub fn call(
    spine_dir: impl AsRef<Path>,
    name: &str,
    args: Value,
    via: Via,
) -> Result<(Value, String)> {
    let spine_dir = spine_dir.as_ref();
    let o = get(name).ok_or_else(|| Error::invalid(format!("未知操作: {name}")))?;
    let args = match args {
        Value::Object(m) => Args(m.into_iter().filter(|(_, v)| !v.is_null()).collect()),
        Value::Null => Args::default(),
        _ => return Err(Error::invalid(format!("{name} 的參數必須是物件"))),
    };
    let names: Vec<&str> = o.params.iter().map(|p| p.name).collect();
    let unknown: Vec<&str> = args
        .0
        .keys()
        .map(String::as_str)
        .filter(|k| !names.contains(k))
        .collect();
    if !unknown.is_empty() {
        return Err(Error::invalid(format!(
            "{name} 不認得參數: {}（可用：{}）",
            unknown.join(", "),
            names.join(", ")
        )));
    }
    let missing: Vec<&str> = o
        .params
        .iter()
        .filter(|p| {
            p.required
                && match args.get(p.name) {
                    None => true,
                    Some(Value::String(s)) => s.is_empty(),
                    Some(Value::Array(a)) => a.is_empty(),
                    _ => false,
                }
        })
        .map(|p| p.name)
        .collect();
    if !missing.is_empty() {
        return Err(Error::invalid(format!(
            "{name} 缺參數: {}",
            missing.join(", ")
        )));
    }
    let result = (o.run)(spine_dir, &args)?;
    if let Some(audit) = o.audit {
        if let Some((tokens, body)) = audit(&result, &args) {
            spine::append_event(
                spine_dir,
                "decision",
                via.source(),
                &tokens,
                &format!("{}{body}（{}）", spine::ADMIN_TAG, via.label()),
                None,
            )?;
        }
    }
    let say = (o.say)(&result, &args);
    Ok((result, say))
}
