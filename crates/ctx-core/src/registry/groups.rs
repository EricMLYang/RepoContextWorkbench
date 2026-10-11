//! 組（P2）：建組、改組（加減組員／改名／刪組）、組層欄位、範圍解析。對應 `registry/groups.py`。
//!
//! 組的維護（2026-09-25 使用者回報：組只能建、不能改）：組員異動只動 registry；repo 登記本身不受影響
//! （整個移出 registry 是 remove_repo 的事）。改名／刪組牽涉 spine 的 `groups/<組>/`（brief、materials、
//! digest）：改名跟著搬，刪組保留（那是產出的料，不是設定）。config 排程指到這組的只回報、不代改（人寫的檔）。

use std::path::Path;

use serde::Serialize;

use super::store::{load, transaction, Group, RegistryData, Repo};
use crate::{Error, Result};

/// 對應 `registry.add_group`（landing＝"default"）。
pub fn add_group<S: AsRef<str>>(
    spine_dir: impl AsRef<Path>,
    name: &str,
    members: &[S],
) -> Result<()> {
    add_group_with_landing(spine_dir, name, members, "default")
}

/// 對應 `registry.add_group(..., landing=)`。
pub fn add_group_with_landing<S: AsRef<str>>(
    spine_dir: impl AsRef<Path>,
    name: &str,
    members: &[S],
    landing: &str,
) -> Result<()> {
    let members: Vec<String> = members.iter().map(|m| m.as_ref().to_string()).collect();
    transaction(spine_dir, |data| {
        if data.group(name).is_some() {
            return Err(Error::invalid(format!("組已存在: {name}")));
        }
        check_members(data, &members)?;
        let mut g = Group::default();
        g.set("name", name);
        g.set_members(&members);
        g.set("landing", landing);
        data.groups.push(g);
        Ok(())
    })
}

/// 對應 `registry.get_group`。
pub fn get_group(spine_dir: impl AsRef<Path>, name: &str) -> Result<Group> {
    load(spine_dir)?
        .group(name)
        .cloned()
        .ok_or_else(|| Error::invalid(format!("組不存在: {name}")))
}

/// 對應 `registry.set_group_field`：組層欄位就地更新（目前用於 `goal`：使用者一句話的工作目標）。
/// 值為空＝刪掉這個欄位（2026-09-12 §3：目標只有人能給，沒給就誠實說沒有）。
pub fn set_group_field(
    spine_dir: impl AsRef<Path>,
    name: &str,
    key: &str,
    value: &str,
) -> Result<Group> {
    if !["goal", "goal_at", "landing"].contains(&key) {
        return Err(Error::invalid(format!("不支援的組欄位: {key}")));
    }
    transaction(spine_dir, |data| {
        let g = find_group(data, name)?;
        if value.is_empty() {
            g.remove(key);
        } else {
            g.set(key, value);
        }
        Ok(g.clone())
    })
}

fn find_group<'a>(data: &'a mut RegistryData, name: &str) -> Result<&'a mut Group> {
    data.group_mut(name)
        .ok_or_else(|| Error::invalid(format!("組不存在: {name}")))
}

fn check_members(data: &RegistryData, members: &[String]) -> Result<()> {
    let unknown: Vec<&str> = members
        .iter()
        .filter(|m| data.repo(m).is_none())
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        return Err(Error::invalid(format!(
            "組員未登記: {}",
            unknown.join(", ")
        )));
    }
    Ok(())
}

fn dedupe<S: AsRef<str>>(ids: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for i in ids {
        let i = i.as_ref().trim();
        if !i.is_empty() && !out.iter().any(|x| x == i) {
            out.push(i.to_string());
        }
    }
    out
}

/// `update_group` 的參數（`None`＝不動）。
#[derive(Debug, Clone, Default)]
pub struct GroupUpdate {
    pub add: Option<Vec<String>>,
    pub remove: Option<Vec<String>>,
    /// 整份換掉（工作台勾選結果）。
    pub members: Option<Vec<String>>,
    pub rename: Option<String>,
}

/// `update_group` 的結果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupUpdated {
    pub group: Group,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub renamed_from: Option<String>,
    pub moved_dir: bool,
    pub schedule_refs: Vec<String>,
}

/// 對應 `registry.update_group`：一次改組（同一個 transaction：任何一步失敗＝全部不寫）。
/// members＝整份換掉；add／remove＝增減；rename＝改名（`groups/<組>/` 跟著搬）。
pub fn update_group(
    spine_dir: impl AsRef<Path>,
    name: &str,
    upd: GroupUpdate,
) -> Result<GroupUpdated> {
    let spine_dir = spine_dir.as_ref();
    transaction(spine_dir, |data| {
        let before = find_group(data, name)?.members();
        let mut cur = before.clone();
        if let Some(members) = &upd.members {
            let members = dedupe(members);
            check_members(data, &members)?;
            cur = members;
        }
        if let Some(add) = upd.add.as_ref().filter(|a| !a.is_empty()) {
            let add = dedupe(add);
            check_members(data, &add)?;
            for i in add {
                if !cur.contains(&i) {
                    cur.push(i);
                }
            }
        }
        if let Some(remove) = upd.remove.as_ref().filter(|a| !a.is_empty()) {
            let remove = dedupe(remove);
            let missing: Vec<&str> = remove
                .iter()
                .filter(|i| !cur.contains(i))
                .map(String::as_str)
                .collect();
            if !missing.is_empty() {
                return Err(Error::invalid(format!(
                    "不在組「{name}」裡: {}",
                    missing.join(", ")
                )));
            }
            cur.retain(|m| !remove.contains(m));
        }
        let (mut moved, mut refs, mut new) = (false, Vec::new(), name.to_string());
        let mut rename_dirs = None;
        if let Some(rename) = &upd.rename {
            let checked = check_group_name(rename)?;
            if checked != name {
                new = checked;
                if data.group(&new).is_some() {
                    return Err(Error::invalid(format!("組已存在: {new}")));
                }
                let src = spine_dir.join("groups").join(name);
                let dst = spine_dir.join("groups").join(&new);
                moved = src.is_dir();
                if moved && dst.exists() {
                    return Err(Error::invalid(format!(
                        "資料夾已存在: groups/{new}/（先自行合併或移走，不代為覆蓋）"
                    )));
                }
                refs = schedule_refs(spine_dir, name)?;
                rename_dirs = moved.then_some((src, dst));
            }
        }
        let g = find_group(data, name)?;
        g.set_members(&cur);
        if new != name {
            g.set("name", new.as_str());
        }
        let group = g.clone();
        if let Some((src, dst)) = rename_dirs {
            std::fs::rename(src, dst)?;
        }
        Ok(GroupUpdated {
            added: cur
                .iter()
                .filter(|m| !before.contains(m))
                .cloned()
                .collect(),
            removed: before
                .iter()
                .filter(|m| !cur.contains(m))
                .cloned()
                .collect(),
            renamed_from: (new != name).then(|| name.to_string()),
            moved_dir: moved,
            schedule_refs: refs,
            group,
        })
    })
}

/// 對應 `registry.add_group_members`。
pub fn add_group_members<S: AsRef<str>>(
    spine_dir: impl AsRef<Path>,
    name: &str,
    ids: &[S],
) -> Result<Group> {
    let add = ids.iter().map(|s| s.as_ref().to_string()).collect();
    Ok(update_group(
        spine_dir,
        name,
        GroupUpdate {
            add: Some(add),
            ..Default::default()
        },
    )?
    .group)
}

/// 對應 `registry.remove_group_members`：移出組（登記保留）；允許移到空組。
pub fn remove_group_members<S: AsRef<str>>(
    spine_dir: impl AsRef<Path>,
    name: &str,
    ids: &[S],
) -> Result<Group> {
    let remove = ids.iter().map(|s| s.as_ref().to_string()).collect();
    Ok(update_group(
        spine_dir,
        name,
        GroupUpdate {
            remove: Some(remove),
            ..Default::default()
        },
    )?
    .group)
}

/// 對應 `registry.set_group_members`：整份換掉（工作台編輯視窗用：勾選結果就是新名單）。
pub fn set_group_members<S: AsRef<str>>(
    spine_dir: impl AsRef<Path>,
    name: &str,
    ids: &[S],
) -> Result<Group> {
    let members = ids.iter().map(|s| s.as_ref().to_string()).collect();
    Ok(update_group(
        spine_dir,
        name,
        GroupUpdate {
            members: Some(members),
            ..Default::default()
        },
    )?
    .group)
}

/// 對應 `groups._check_group_name`。
pub fn check_group_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::invalid("組名不可為空"));
    }
    if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Err(Error::invalid(format!(
            "組名不可含路徑字元: {name}（會變成 groups/<組名>/ 資料夾）"
        )));
    }
    Ok(name.to_string())
}

/// 對應 `groups._schedule_refs`：config 排程裡指到這組的項（`task@at`）。
pub fn schedule_refs(spine_dir: &Path, name: &str) -> Result<Vec<String>> {
    Ok(crate::config::load(spine_dir)?
        .schedule()
        .into_iter()
        .filter(|e| e.group.as_deref() == Some(name))
        .map(|e| format!("{}@{}", e.task, e.at))
        .collect())
}

/// `rename_group` 的結果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupRenamed {
    pub group: Group,
    pub moved_dir: bool,
    pub schedule_refs: Vec<String>,
}

/// 對應 `registry.rename_group`：registry 欄位（目標等）原樣帶過去；spine 的 `groups/<舊>/` 搬到
/// `groups/<新>/`。舊事件裡的 `group:<舊>` 不改（脊椎 append-only）。
pub fn rename_group(spine_dir: impl AsRef<Path>, old: &str, new: &str) -> Result<GroupRenamed> {
    let new = check_group_name(new)?;
    let r = update_group(
        spine_dir,
        old,
        GroupUpdate {
            rename: Some(new),
            ..Default::default()
        },
    )?;
    Ok(GroupRenamed {
        group: r.group,
        moved_dir: r.moved_dir,
        schedule_refs: r.schedule_refs,
    })
}

/// `remove_group` 的結果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GroupRemoved {
    pub name: String,
    /// 保留的資料夾（沒有＝None）。
    pub kept_dir: Option<String>,
    pub schedule_refs: Vec<String>,
}

/// 對應 `registry.remove_group`：只刪 registry 的組定義；repo 登記與 `groups/<組>/` 的料都保留。
pub fn remove_group(spine_dir: impl AsRef<Path>, name: &str) -> Result<GroupRemoved> {
    let spine_dir = spine_dir.as_ref();
    transaction(spine_dir, |data| {
        find_group(data, name)?;
        data.groups.retain(|g| g.name() != name);
        let d = spine_dir.join("groups").join(name);
        Ok(GroupRemoved {
            name: name.to_string(),
            kept_dir: d.is_dir().then(|| d.to_string_lossy().into_owned()),
            schedule_refs: schedule_refs(spine_dir, name)?,
        })
    })
}

/// 對應 `registry.resolve_group`：回 (組名, repo 登記項)。`repos` 給定（非空）＝臨時組合免建組（P2）；
/// `name=None` 且無 repos＝全部（工作台的（全部）範圍——L4 bug：以前這裡直接炸）。
/// Python 收逗號字串的地方，呼叫端先用 [`split_ids`] 拆。
pub fn resolve_group(
    spine_dir: impl AsRef<Path>,
    name: Option<&str>,
    repos: Option<&[String]>,
) -> Result<(String, Vec<Repo>)> {
    let data = load(spine_dir)?;
    let get = |id: &str| {
        data.repo(id)
            .cloned()
            .ok_or_else(|| Error::invalid(format!("repo 不存在: {id}")))
    };
    if let Some(ids) = repos.filter(|r| !r.is_empty()) {
        let entries = ids.iter().map(|i| get(i)).collect::<Result<Vec<_>>>()?;
        return Ok((format!("臨時({})", ids.join(",")), entries));
    }
    let Some(name) = name else {
        return Ok(("全部".into(), data.repos.clone()));
    };
    match data.group(name) {
        Some(g) => Ok((
            name.to_string(),
            g.members()
                .iter()
                .map(|m| get(m))
                .collect::<Result<Vec<_>>>()?,
        )),
        None => Err(Error::invalid(format!("組不存在: {name}"))),
    }
}

/// Python `[r.strip() for r in repos.split(",")]`（CLI 給的逗號字串）。
pub fn split_ids(s: &str) -> Vec<String> {
    s.split(',').map(|x| x.trim().to_string()).collect()
}

/// 新順序必須跟現在是同一批（只換順序）；不是就拒絕——畫面拿的是舊資料時不會默默丟掉或多出項目。
fn same_set(cur: &[String], next: &[String], what: &str) -> Result<()> {
    let mut a = cur.to_vec();
    let mut b = next.to_vec();
    a.sort();
    b.sort();
    if a != b {
        return Err(Error::invalid(format!("{what}已經變動，請重新整理後再排")));
    }
    Ok(())
}

/// Rust 才有（2026-10-11，App 拖曳排序）：牌組的順序（registry.yaml 的 groups 順序）。不記 spine 事件——排序是偏好。
pub fn reorder_groups<S: AsRef<str>>(spine_dir: impl AsRef<Path>, names: &[S]) -> Result<()> {
    let names: Vec<String> = names.iter().map(|s| s.as_ref().to_string()).collect();
    transaction(spine_dir, |data| {
        let cur: Vec<String> = data.groups.iter().map(Group::name).collect();
        same_set(&cur, &names, "牌組")?;
        let mut groups = std::mem::take(&mut data.groups);
        groups.sort_by_key(|g| names.iter().position(|n| *n == g.name()));
        data.groups = groups;
        Ok(())
    })
}

/// Rust 才有（2026-10-11）：組員的順序（只換順序）。
pub fn reorder_members<S: AsRef<str>>(
    spine_dir: impl AsRef<Path>,
    name: &str,
    ids: &[S],
) -> Result<()> {
    let ids: Vec<String> = ids.iter().map(|s| s.as_ref().to_string()).collect();
    transaction(spine_dir, |data| {
        let g = find_group(data, name)?;
        same_set(&g.members(), &ids, "組員")?;
        g.set_members(&ids);
        Ok(())
    })
}

/// Rust 才有（2026-10-11）：「所有 repo」的順序＝registry.yaml 的 repos 順序（只換順序）。
pub fn reorder_repos<S: AsRef<str>>(spine_dir: impl AsRef<Path>, ids: &[S]) -> Result<()> {
    let ids: Vec<String> = ids.iter().map(|s| s.as_ref().to_string()).collect();
    transaction(spine_dir, |data| {
        let cur: Vec<String> = data.repos.iter().map(Repo::id).collect();
        same_set(&cur, &ids, "repo 清單")?;
        let mut repos = std::mem::take(&mut data.repos);
        repos.sort_by_key(|r| ids.iter().position(|i| *i == r.id()));
        data.repos = repos;
        Ok(())
    })
}
