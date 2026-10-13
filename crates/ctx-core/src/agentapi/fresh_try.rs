//! Rust 才有（Python 沒有）：反芻推薦「拿來試」（F-26）。
//! 從 agentapi.rs 搬出來（2026-10-10，ideation N-24），行為不變；`agentapi::` 底下的路徑照舊。

use super::*;

/// 把一筆反芻推薦「拿來試」：在推薦所屬的組開一個帶出處的待辦，推薦標成用上了。
/// 待辦文字：`試用：<標題>（<ref>，反芻 <id>）`——之後 `ctx todo review` 也找得到它的出處。
/// 推薦本來就是推給那一組的，所以寫進那一組（不受 cwd 主場限制）。
pub fn try_fresh(spine_dir: &Path, link_id: &str, source: &str) -> Result<Value> {
    let link = ruminate::load_state(spine_dir)
        .links
        .into_iter()
        .find(|l| l.id.as_deref() == Some(link_id))
        .ok_or_else(|| {
            agent_error(
                crate::RejectCode::NotFound,
                format!("找不到推薦 {link_id}"),
                Some("ctx fresh --all 看全部推薦的 id。"),
            )
        })?;
    // 檔案推薦的標題是相對路徑，取檔名（去副檔名）就好；判斷推薦的標題是判斷第一行
    let title = match link.kind {
        ruminate::LinkKind::File => Path::new(&link.title)
            .file_stem()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_else(|| link.title.clone()),
        ruminate::LinkKind::Decision => link.title.clone(),
    };
    let text = format!(
        "試用：{}（{}，反芻 {link_id}）",
        crate::pystr::take_chars(&title, 60),
        link.reference
    );
    let t = Target {
        cwd: None,
        group: Some(link.group.clone()),
        repos: None,
    };
    let todo = add_todo(spine_dir, &text, None, &t, true, source)?;
    let now = Local::now().format("%Y-%m-%d %H:%M").to_string();
    ruminate::mark_tried(spine_dir, link_id, &now)?;
    Ok(
        json!({"ok": true, "todo": todo.id, "group": link.group, "ref": link.reference, "text": text}),
    )
}
