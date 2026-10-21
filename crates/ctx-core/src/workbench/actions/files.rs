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
                   "abs": e.abs.to_string_lossy(), "dir": e.dir, "ignored": e.ignored})
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

/// 終端裡的路徑字樣拆成 (路徑, 行, 欄)：`src/a.rs:12:5`、`a.rs(12,5)`、`"src/a.rs",`。
/// 前後的引號、括號、句讀先拿掉。Rust 才有（F-95）。
pub fn split_location(text: &str) -> (String, Option<u32>, Option<u32>) {
    let t = text
        .trim()
        .trim_start_matches(['"', '\'', '(', '[', '<', '`'])
        .trim_end_matches(['"', '\'', ')', ']', '>', '`', ',', '.', ';', ':']);
    // a.rs(12,5) / a.rs(12)
    if let Some((p, rest)) = t.split_once('(') {
        let nums: Vec<Option<u32>> = rest
            .trim_end_matches(')')
            .split(',')
            .map(|n| n.trim().parse().ok())
            .collect();
        if let [Some(l), rest @ ..] = nums.as_slice() {
            return (p.to_string(), Some(*l), rest.first().copied().flatten());
        }
    }
    let mut parts: Vec<&str> = t.rsplitn(3, ':').collect();
    parts.reverse();
    let num = |s: &str| s.parse::<u32>().ok().filter(|n| *n > 0);
    match parts.as_slice() {
        [p, l, c] if num(l).is_some() && num(c).is_some() => (p.to_string(), num(l), num(c)),
        [a, b, l] if num(l).is_some() => (format!("{a}:{b}"), num(l), None),
        [p, l] if num(l).is_some() => (p.to_string(), num(l), None),
        _ => (t.to_string(), None, None),
    }
}

#[derive(serde::Deserialize)]
struct LocateArgs {
    /// 終端上看到的那段字（可帶 `:行:欄`）。
    text: String,
    /// 哪個會話印的（相對路徑照它的目錄解析）。
    #[serde(default)]
    sid: Option<String>,
}

/// F-95（Rust 才有）：終端輸出裡的路徑 → 登記的哪個 repo、repo 內的相對路徑、第幾行。
/// 相對路徑先照 agent 現在的目錄（hook 記的 `cwd`，開在 worktree 時是 worktree），再照會話開的目錄解析。
/// 不在任何登記的 repo 裡、不是檔案：回錯（說明原因），不開任意路徑。
pub(super) fn a_locate_path(d: &Path, p: &Value, t: Option<&dyn Terms>) -> Result<Value> {
    let LocateArgs { text, sid } = parse("locate_path", p)?;
    let (raw_path, line, col) = split_location(&text);
    if raw_path.is_empty() {
        return Err(Error::invalid("沒有路徑"));
    }
    let given = crate::fsutil::expanduser(&raw_path);
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Some(sid) = sid.as_deref().filter(|s| !s.is_empty()) {
        if let Some(c) = crate::session_status::read(d, sid)
            .as_ref()
            .and_then(|v| v.get("cwd"))
            .and_then(Value::as_str)
        {
            bases.push(PathBuf::from(c));
        }
        if let Some(c) = t.and_then(|t| t.cwd(sid)) {
            bases.push(c);
        }
    }
    let candidates: Vec<PathBuf> = if given.is_absolute() {
        vec![given]
    } else {
        bases.iter().map(|b| b.join(&given)).collect()
    };
    let Some(abs) = candidates
        .iter()
        .find(|c| c.is_file())
        .and_then(|c| std::fs::canonicalize(c).ok())
    else {
        return Err(Error::invalid(format!("找不到這個檔：{raw_path}")));
    };
    let mut best: Option<(String, PathBuf)> = None;
    for e in &registry::load(d)?.repos {
        let Ok(root) = std::fs::canonicalize(e.path()) else {
            continue;
        };
        if abs.starts_with(&root)
            && best
                .as_ref()
                .is_none_or(|(_, r)| root.as_os_str().len() > r.as_os_str().len())
        {
            best = Some((e.id(), root));
        }
    }
    let Some((id, root)) = best else {
        return Err(Error::invalid(format!(
            "這個檔不在任何登記的 repo 裡：{}",
            abs.display()
        )));
    };
    let rel = abs
        .strip_prefix(&root)
        .map_err(|_| Error::invalid("路徑不在 repo 裡"))?
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    Ok(json!({"ok": true, "repo": id, "path": rel, "line": line, "col": col}))
}

#[derive(serde::Deserialize)]
struct WorktreeArgs {
    id: String,
    #[serde(default)]
    path: String,
}

/// F-107（Rust 才有）：repo 掛出去的 worktree（分支、沒 commit 的檔數、只在這裡的 commit、能不能移除）。只讀。
pub(super) fn a_repo_worktrees(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let WorktreeArgs { id, .. } = parse("repo_worktrees", p)?;
    let root = registry::get_repo(d, &id)?.path();
    let list: Vec<Value> = crate::worktrees::list(&root)?
        .into_iter()
        .map(|w| {
            let removable = w.removable();
            let mut v = serde_json::to_value(w).unwrap_or_default();
            v["removable"] = json!(removable);
            v
        })
        .collect();
    Ok(json!({"ok": true, "id": id, "worktrees": list}))
}

/// F-107（Rust 才有）：移除一個乾淨的 worktree（`git worktree remove`，不加 --force；分支留著）。
pub(super) fn a_worktree_remove(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let WorktreeArgs { id, path } = parse("worktree_remove", p)?;
    let root = registry::get_repo(d, &id)?.path();
    crate::worktrees::remove(&root, &path)?;
    Ok(json!({"ok": true}))
}

#[derive(serde::Deserialize)]
struct LogArgs {
    id: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    sha: String,
}

/// F-109（Rust 才有）：repo 最近的 commit（目前分支，新的在前；預設 20 個）。只讀。
pub(super) fn a_repo_log(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let LogArgs { id, limit, .. } = parse("repo_log", p)?;
    let root = registry::get_repo(d, &id)?.path();
    let commits = crate::gitlog::log(&root, limit.unwrap_or(20));
    Ok(json!({"ok": true, "id": id, "commits": commits}))
}

/// F-109（Rust 才有）：一個 commit 的說明、統計與差異（`git show`）。只讀。
pub(super) fn a_commit_diff(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let LogArgs { id, sha, .. } = parse("commit_diff", p)?;
    let root = registry::get_repo(d, &id)?.path();
    let (diff, truncated) = crate::gitlog::show(&root, &sha)?;
    Ok(json!({"ok": true, "id": id, "sha": sha, "diff": diff, "truncated": truncated}))
}

/// F-112（Rust 才有）：repo 在 GitHub 上開著的 PR 與 issue（用使用者自己的 `gh`）。只讀。
pub(super) fn a_repo_github(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let WorktreeArgs { id, .. } = parse("repo_github", p)?;
    let root = registry::get_repo(d, &id)?.path();
    let mut out = crate::github::open_items(&root)?;
    out["ok"] = json!(true);
    out["id"] = json!(id);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::split_location;

    #[test]
    fn split_location_reads_line_and_column_suffixes() {
        let s = |t: &str| split_location(t);
        assert_eq!(s("src/a.rs:12:5"), ("src/a.rs".into(), Some(12), Some(5)));
        assert_eq!(s("src/a.rs:12"), ("src/a.rs".into(), Some(12), None));
        assert_eq!(s("(src/a.rs:12)."), ("src/a.rs".into(), Some(12), None));
        assert_eq!(s("a.ts(30,2)"), ("a.ts".into(), Some(30), Some(2)));
        assert_eq!(s("\"docs/x.md\","), ("docs/x.md".into(), None, None));
        assert_eq!(s("C:/r/a.rs:7"), ("C:/r/a.rs".into(), Some(7), None));
        assert_eq!(s("src/a.rs:"), ("src/a.rs".into(), None, None));
    }
}
