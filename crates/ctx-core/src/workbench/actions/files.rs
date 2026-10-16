//! 檔案樹與檢視器的 action：列目錄、讀檔、寫檔、新增、改名、貼圖、反向連結（都走 [`crate::filetree`]，限制在登記的 repo 裡）。
//! 2026-10-16 從 `mod.rs` 拆出來（檔案超過 800 行依職責拆，見 `docs/DEVELOPMENT.md` §2）；行為不變。

use super::*;

/// Rust 才有（2026-10-11）：repo 詳情的檔案清單，一次一層（見 [`crate::filetree`]）。
#[derive(serde::Deserialize)]
struct RepoTreeArgs {
    id: String,
    /// 相對 repo 根目錄；空＝根。
    #[serde(default)]
    dir: String,
}

pub(super) fn a_repo_tree(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let RepoTreeArgs { id, dir } = parse("repo_tree", p)?;
    let root = registry::get_repo(d, &id)?.path();
    let l = crate::filetree::list_dir(&root, &dir)?;
    let entries: Vec<Value> = l
        .entries
        .iter()
        .map(|e| {
            json!({"name": e.name, "path": e.rel.to_string_lossy(),
                   "abs": e.abs.to_string_lossy(), "dir": e.dir})
        })
        .collect();
    Ok(
        json!({"ok": true, "id": id, "root": root.to_string_lossy(), "dir": dir,
              "entries": entries, "more": l.more}),
    )
}

/// Rust 才有（2026-10-11）：唯讀檢視器讀一個檔（見 [`crate::filetree::read_file`]）。
#[derive(serde::Deserialize)]
pub(super) struct ReadFileArgs {
    pub(super) id: String,
    pub(super) path: String,
}

pub(super) fn a_read_file(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let ReadFileArgs { id, path } = parse("read_file", p)?;
    let root = registry::get_repo(d, &id)?.path();
    Ok(file_json(&id, crate::filetree::read_file(&root, &path)?))
}

fn file_json(id: &str, f: crate::filetree::FileRead) -> Value {
    let mut out = json!({"ok": true, "id": id, "path": f.rel.to_string_lossy(),
                         "abs": f.abs.to_string_lossy(), "size": f.size});
    let extra = match f.view {
        crate::filetree::FileView::Text {
            text,
            truncated,
            editable,
            version,
        } => {
            json!({"kind": "text", "text": text, "truncated": truncated,
                   "editable": editable, "version": version})
        }
        crate::filetree::FileView::Image { mime, base64 } => {
            json!({"kind": "image", "mime": mime, "base64": base64})
        }
        crate::filetree::FileView::Binary => json!({"kind": "binary"}),
        crate::filetree::FileView::TooLarge => json!({"kind": "too_large"}),
    };
    if let (Value::Object(o), Value::Object(e)) = (&mut out, extra) {
        o.extend(e);
    }
    out
}

/// Rust 才有（2026-10-13）：檢視器簡單編輯（見 [`crate::filetree::write_file`]）。
/// 不記 spine 事件——改檔的痕跡在 git。磁碟上已被改過回 `{ok: false, conflict: true}`。
#[derive(serde::Deserialize)]
struct WriteFileArgs {
    id: String,
    path: String,
    text: String,
    /// 讀檔時的 `version`。
    base: String,
}

pub(super) fn a_write_file(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let WriteFileArgs {
        id,
        path,
        text,
        base,
    } = parse("write_file", p)?;
    let root = registry::get_repo(d, &id)?.path();
    Ok(
        match crate::filetree::write_file(&root, &path, &text, &base)? {
            crate::filetree::WriteOutcome::Saved(f) => file_json(&id, f),
            crate::filetree::WriteOutcome::Conflict => {
                json!({"ok": false, "conflict": true, "id": id, "path": path})
            }
        },
    )
}

/// Rust 才有（2026-10-15）：檢視器／檔案樹新增空檔或資料夾（見 [`crate::filetree::create_entry`]）。
/// 不記 spine 事件——跟改檔一樣，痕跡在 git。
#[derive(serde::Deserialize)]
struct CreateEntryArgs {
    id: String,
    path: String,
    kind: crate::filetree::NewEntry,
}

pub(super) fn a_create_entry(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let CreateEntryArgs { id, path, kind } = parse("create_entry", p)?;
    let root = registry::get_repo(d, &id)?.path();
    let abs = crate::filetree::create_entry(&root, &path, kind)?;
    let rel = abs
        .strip_prefix(&root)
        .unwrap_or(&abs)
        .to_string_lossy()
        .into_owned();
    Ok(
        json!({"ok": true, "id": id, "path": rel, "abs": abs.to_string_lossy(),
              "dir": kind == crate::filetree::NewEntry::Dir}),
    )
}

/// Rust 才有（F-66）：檔案樹改名或搬移（見 [`crate::filetree::rename_entry`]）；不覆蓋、不刪；不記 spine 事件。
#[derive(serde::Deserialize)]
struct RenameEntryArgs {
    id: String,
    from: String,
    to: String,
}

pub(super) fn a_rename_entry(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let RenameEntryArgs { id, from, to } = parse("rename_entry", p)?;
    let root = registry::get_repo(d, &id)?.path();
    let (abs, dir) = crate::filetree::rename_entry(&root, &from, &to)?;
    let rel = abs
        .strip_prefix(&root)
        .unwrap_or(&abs)
        .to_string_lossy()
        .into_owned();
    Ok(json!({"ok": true, "id": id, "from": from, "path": rel,
              "abs": abs.to_string_lossy(), "dir": dir}))
}

/// Rust 才有（F-64）：md 編輯區貼上圖片存成新檔（見 [`crate::filetree::save_image`]）；不覆蓋、不記 spine 事件。
#[derive(serde::Deserialize)]
struct SaveImageArgs {
    id: String,
    path: String,
    base64: String,
}

pub(super) fn a_save_image(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let SaveImageArgs { id, path, base64 } = parse("save_image", p)?;
    let root = registry::get_repo(d, &id)?.path();
    let abs = crate::filetree::save_image(&root, &path, &base64)?;
    let rel = abs
        .strip_prefix(&root)
        .unwrap_or(&abs)
        .to_string_lossy()
        .into_owned();
    Ok(json!({"ok": true, "id": id, "path": rel, "abs": abs.to_string_lossy()}))
}

/// Rust 才有（F-69）：repo 裡哪些 md 連到這個檔（唯讀）。
pub(super) fn a_backlinks(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let id = raw(p, "id").unwrap_or_default();
    let path = raw(p, "path").unwrap_or_default();
    let root = registry::get_repo(d, &id)?.path();
    let (hits, more) = crate::filetree::backlinks(&root, &path)?;
    let items: Vec<Value> = hits
        .iter()
        .map(|h| json!({"path": h.rel, "line": h.line, "text": h.text}))
        .collect();
    Ok(json!({"ok": true, "id": id, "path": path, "items": items, "more": more}))
}
