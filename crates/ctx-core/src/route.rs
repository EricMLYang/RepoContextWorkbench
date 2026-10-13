//! P8 落 route：結果路由到 spine `groups/<g>/materials`、某 repo `01_inbox/`、或 incubator。
//! 對應 Python `repo_context/route.py`。

use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime};

use crate::{registry, spine, Error, Result};

/// 對應 `route.route`：dest＝`group:<g>` | `repo:<id>` | `incubator`。回傳寫入的檔案路徑。
/// 落點同時記一筆 `decision [route]`。`title` 預設（Python）是 "idea"。
pub fn route(
    spine_dir: impl AsRef<Path>,
    text: &str,
    dest: &str,
    title: &str,
    when: Option<NaiveDateTime>,
) -> Result<PathBuf> {
    let spine_dir = spine_dir.as_ref();
    let now = when.unwrap_or_else(|| Local::now().naive_local());
    let fname = format!("{}_{title}.md", now.format("%Y%m%d-%H%M%S"));
    let target = if let Some(g) = dest.strip_prefix("group:") {
        // 事件標頭裡的組名可能換寫過（PM_Group）：資料夾用登記的原名，跟筆記放在同一個組資料夾
        let name = registry::load(spine_dir)
            .ok()
            .and_then(|d| d.group(g).map(|x| x.name()))
            .unwrap_or_else(|| g.to_string());
        spine_dir
            .join("groups")
            .join(name)
            .join("materials")
            .join(&fname)
    } else if let Some(id) = dest.strip_prefix("repo:") {
        registry::get_repo(spine_dir, id)?
            .path()
            .join("01_inbox")
            .join(&fname)
    } else if dest == "incubator" {
        spine_dir.join("incubator").join(&fname)
    } else {
        return Err(Error::invalid(format!(
            "dest 不合法: {dest}（group:<g> | repo:<id> | incubator）"
        )));
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, text)?;
    let tokens: Vec<String> = dest
        .strip_prefix("repo:")
        .map(|id| vec![format!("repo:{id}")])
        .unwrap_or_default();
    let name = target
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    spine::append_event(
        spine_dir,
        "decision",
        "route",
        &tokens,
        &format!("落點：{dest} → {name}"),
        Some(now),
    )?;
    Ok(target)
}
