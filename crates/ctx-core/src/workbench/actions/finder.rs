//! 找東西與看變更（2026-10-14，Rust 才有）：⌘K 的 `palette`、`scope_changes`、`repo_changes`、`file_diff`。

use super::*;

/// Rust 才有（2026-10-13）：App 的 ⌘K——打字找 repo（名稱部分字串、縮寫、名片內容）與牌組（名稱、目標）。
/// 排名在 [`crate::profile::quick_find`]；前端只畫。repo 多帶它在哪些牌組。
#[derive(serde::Deserialize)]
struct PaletteArgs {
    #[serde(default)]
    query: String,
    /// 找檔案的範圍（畫面目前的範圍；都沒有＝所有沒封存的 repo）。repo 與牌組不受範圍限制。
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    repos: Option<Vec<String>>,
}

/// ⌘K 一次最多列幾個 repo（打字會再縮小範圍）。
const PALETTE_LIMIT: usize = 30;
/// ⌘K 一次最多列幾個檔案。
const PALETTE_FILES: usize = 20;
/// 打幾個字以上才找檔案（一個字會中一大堆）。
const PALETTE_FILE_MIN_CHARS: usize = 2;

pub(super) fn a_palette(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let PaletteArgs {
        query,
        group,
        repos: scope_repos,
    } = parse("palette", p)?;
    let data = registry::load(d)?;
    let profiles: Vec<_> = data.repos.iter().map(crate::profile::profile).collect();
    let q = query.trim().to_lowercase();
    let repos: Vec<Value> = crate::profile::quick_find(&profiles, &q, PALETTE_LIMIT)
        .into_iter()
        .map(|h| {
            let groups: Vec<String> = data
                .groups
                .iter()
                .filter(|g| g.members().contains(&h.repo))
                .map(|g| g.name())
                .collect();
            let mut v = serde_json::to_value(&h)?;
            v["groups"] = json!(groups);
            Ok(v)
        })
        .collect::<Result<_>>()?;
    let groups: Vec<Value> = data
        .groups
        .iter()
        .filter(|g| {
            let goal = g.get_str("goal").unwrap_or_default();
            q.is_empty() || g.name().to_lowercase().contains(&q) || goal.to_lowercase().contains(&q)
        })
        .map(|g| {
            json!({"name": g.name(), "members": g.members().len(),
                   "goal": g.get_str("goal").unwrap_or_default()})
        })
        .collect();
    let files = if q.chars().count() >= PALETTE_FILE_MIN_CHARS {
        palette_files(&data, &q, group.as_deref(), scope_repos.as_deref())
    } else {
        Vec::new()
    };
    Ok(json!({"ok": true, "query": query, "repos": repos, "groups": groups, "files": files}))
}

/// 畫面範圍 → (repo id, 路徑)：`repos` 優先；`group`＝組員；都沒有＝所有沒封存的 repo。不在 registry 的略過。
fn scope_roots(
    data: &registry::RegistryData,
    group: Option<&str>,
    scope_repos: Option<&[String]>,
) -> Vec<(String, PathBuf)> {
    let ids: Vec<String> = match (scope_repos, group) {
        (Some(r), _) => r.to_vec(),
        (None, Some(g)) => data
            .groups
            .iter()
            .find(|x| x.name() == g)
            .map(|x| x.members())
            .unwrap_or_default(),
        (None, None) => data
            .repos
            .iter()
            .filter(|r| r.tier() != Some(registry::Tier::Archived))
            .map(|r| r.id())
            .collect(),
    };
    ids.iter()
        .filter_map(|id| data.repo(id).map(|r| (id.clone(), r.path())))
        .collect()
}

/// Rust 才有（2026-10-14）：範圍內每個 repo 還沒 commit 的變更（各 repo 平行跑 git status）——
/// 多個 agent 同時在不同 repo 改東西時，一眼看出誰動了什麼。只回有變更的 repo（照範圍順序）；
/// 不是 git repo 的略過，git 失敗的帶 `error`。
#[derive(serde::Deserialize)]
struct ScopeArgs {
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    repos: Option<Vec<String>>,
}

pub(super) fn a_scope_changes(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let ScopeArgs { group, repos } = parse("scope_changes", p)?;
    let data = registry::load(d)?;
    let roots = scope_roots(&data, group.as_deref(), repos.as_deref());
    let rows: Vec<Value> = std::thread::scope(|sc| {
        let jobs: Vec<_> = roots
            .iter()
            .map(|(id, root)| {
                sc.spawn(move || {
                    (
                        id,
                        root.join(".git")
                            .exists()
                            .then(|| crate::filetree::changes(root)),
                    )
                })
            })
            .collect();
        jobs.into_iter()
            .filter_map(|j| j.join().ok())
            .filter_map(|(id, res)| match res? {
                Ok((cs, _)) if cs.is_empty() => None,
                Ok((cs, more)) => Some(json!({"id": id, "more": more, "changes": cs.iter()
                    .map(|c| json!({"path": c.rel, "status": c.kind})).collect::<Vec<_>>()})),
                Err(e) => Some(json!({"id": id, "error": e.to_string(), "changes": [], "more": 0})),
            })
            .collect()
    });
    Ok(json!({"ok": true, "repos": rows}))
}

/// ⌘K 的檔案結果：範圍內每個 repo 用檔名找（各 repo 平行跑 `git ls-files`），合起來照分數排。
fn palette_files(
    data: &registry::RegistryData,
    q: &str,
    group: Option<&str>,
    scope_repos: Option<&[String]>,
) -> Vec<Value> {
    let roots = scope_roots(data, group, scope_repos);
    let mut hits: Vec<(String, PathBuf, crate::filetree::FileHit)> = std::thread::scope(|sc| {
        let jobs: Vec<_> = roots
            .iter()
            .map(|(id, root)| {
                sc.spawn(move || {
                    crate::filetree::find_files(root, q, PALETTE_FILES)
                        .into_iter()
                        .map(|h| (id.clone(), root.clone(), h))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        jobs.into_iter()
            .flat_map(|j| j.join().unwrap_or_default())
            .collect()
    });
    hits.sort_by(|a, b| {
        b.2.score
            .cmp(&a.2.score)
            .then(a.2.rel.len().cmp(&b.2.rel.len()))
    });
    hits.truncate(PALETTE_FILES);
    hits.into_iter()
        .map(|(id, root, h)| {
            json!({"repo": id, "path": h.rel, "name": h.name,
                   "abs": root.join(&h.rel).to_string_lossy()})
        })
        .collect()
}

/// Rust 才有（2026-10-14）：還沒 commit 的變更（看 agent 改了什麼；見 [`crate::filetree::changes`]）；
/// 2026-10-19 起多回 `unpushed`（已 commit 沒 push 的檔，見 [`crate::filetree::unpushed`]）。
pub(super) fn a_repo_changes(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let ProfileArgs { id } = parse("repo_changes", p)?;
    let root = registry::get_repo(d, &id)?.path();
    let (cs, more) = crate::filetree::changes(&root)?;
    let changes: Vec<Value> = cs
        .iter()
        .map(|c| json!({"path": c.rel, "status": c.kind}))
        .collect();
    // 已 commit 但還沒 push 的檔（檔案樹標 ↑）；沒上游就是空
    let unpushed = crate::filetree::unpushed(&root)?;
    Ok(json!({"ok": true, "id": id, "changes": changes, "more": more, "unpushed": unpushed}))
}

/// Rust 才有（2026-10-14）：單檔差異（見 [`crate::filetree::diff`]）。
pub(super) fn a_file_diff(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let files::ReadFileArgs { id, path } = parse("file_diff", p)?;
    let root = registry::get_repo(d, &id)?.path();
    let (diff, truncated) = crate::filetree::diff(&root, &path)?;
    Ok(json!({"ok": true, "id": id, "path": path, "diff": diff, "truncated": truncated}))
}

#[derive(serde::Deserialize)]
struct PastArgs {
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    repos: Option<Vec<String>>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    query: Option<String>,
}

/// F-72（Rust 才有）：目前範圍過去的 Claude Code 互動會話（最近的在前），給「過去的會話」清單。
pub(super) fn a_past_sessions(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let PastArgs {
        group,
        repos,
        limit,
        query,
    } = parse("past_sessions", p)?;
    let list = crate::history::search_sessions(
        d,
        group.as_deref(),
        repos.as_deref(),
        query.as_deref(),
        limit.unwrap_or(20).min(100),
    )?;
    Ok(json!({"ok": true, "sessions": list}))
}

#[derive(serde::Deserialize)]
struct DiffNote {
    /// 畫面算好的位置（「第 12 行（新增）」）。
    #[serde(rename = "where")]
    at: String,
    /// 那一行的 diff 文字（含 +／- 前綴）。
    #[serde(default)]
    code: String,
    note: String,
}

#[derive(serde::Deserialize)]
struct DiffNotesArgs {
    id: String,
    path: String,
    notes: Vec<DiffNote>,
}

/// 一則留言最多帶幾個字元的程式碼（一行 diff 可能很長）。
const NOTE_CODE_MAX: usize = 160;

/// F-75（Rust 才有）：差異上的逐行留言 → 一段給 agent 的話（一次改完再回報，不要一則一則來回）。
/// 前端只負責收留言與貼進會話；文字怎麼寫在這裡（參考 Orca Annotate AI Diff 的整批送回）。
pub(super) fn a_diff_notes_brief(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let DiffNotesArgs { id, path, notes } = parse("diff_notes_brief", p)?;
    let notes: Vec<&DiffNote> = notes.iter().filter(|n| !n.note.trim().is_empty()).collect();
    if notes.is_empty() {
        return Err(Error::invalid("還沒有留言"));
    }
    let abs = registry::get_repo(d, &id)?.path().join(&path);
    let mut lines = vec![format!(
        "我看了 {} 還沒 commit 的差異，留了 {} 則意見。請一次全部處理完再回報（哪則改了什麼、哪則不改和理由）：",
        abs.display(),
        notes.len()
    )];
    for (i, n) in notes.iter().enumerate() {
        let code = crate::transcript::one_line(n.code.trim_end(), NOTE_CODE_MAX);
        lines.push(String::new());
        lines.push(format!("{}. {}：`{}`", i + 1, n.at.trim(), code));
        for l in n.note.trim().lines() {
            lines.push(format!("   {l}"));
        }
    }
    Ok(json!({"ok": true, "text": lines.join("\n"), "count": notes.len()}))
}

#[derive(serde::Deserialize)]
struct TranscriptArgs {
    /// App 會話（從它的狀態檔拿 Claude Code 會話 id 與目錄）。
    #[serde(default)]
    sid: Option<String>,
    /// 或直接給 Claude Code 會話 id（過去的會話）。
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}

/// F-94（Rust 才有）：一場會話的逐字稿時間軸（你說的、回覆、思考、工具＋結果）。只讀。
/// 給 `sid` 時，會話還沒觸發過任何 hook（剛開、還沒送出第一句）就沒有 Claude Code 會話 id，回錯說明。
pub(super) fn a_session_transcript(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let TranscriptArgs { sid, id, cwd } = parse("session_transcript", p)?;
    let (id, cwd) = match (sid, id) {
        (_, Some(id)) => (id, cwd),
        (Some(sid), None) => {
            let st = crate::session_status::read(d, &sid).unwrap_or_default();
            let field = |k: &str| st.get(k).and_then(Value::as_str).map(str::to_string);
            let Some(id) = field("claude_session") else {
                return Err(Error::invalid(
                    "這個會話還沒有逐字稿：送出第一句之後才會有（或它不是 Claude Code 會話）",
                ));
            };
            (id, cwd.or_else(|| field("cwd")))
        }
        (None, None) => return Err(Error::invalid("缺 sid 或 id")),
    };
    let path = crate::timeline::locate(&id, cwd.as_deref(), None)
        .ok_or_else(|| Error::invalid(format!("找不到這場會話的逐字稿：{id}")))?;
    let tl = crate::timeline::read(&path)
        .ok_or_else(|| Error::invalid(format!("讀不了逐字稿：{}", path.display())))?;
    let mut out = serde_json::to_value(tl)?;
    out["ok"] = json!(true);
    out["id"] = json!(id);
    Ok(out)
}
