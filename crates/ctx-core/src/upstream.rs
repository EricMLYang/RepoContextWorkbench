//! P4 上游 upstream——pull 式查 external repo 的 release/commit（`gh api`），只偵測與抓取。
//! 對應 Python `repo_context/upstream.py`。
//!
//! 無事不報在這裡的落實：
//! - 預設只報正式 release（config `thresholds.upstream.kinds`／`prerelease`，§3.4）
//! - 已見過的 release/commit 記在 `.state/upstream.json`，同一筆不重報（洪流不進未讀）
//!
//! 新發現落 `suggestion [upstream] repo:<id>` 事件 → 走未讀回程，儀式時刻集中呈現。

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::NaiveDateTime;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::{config, registry, spine, Error, Result};

/// 上游最新一筆（對應 `gh_fetch` 回傳的 dict）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Latest {
    /// "release" | "commit"
    pub kind: String,
    pub id: String,
    pub title: String,
    pub url: String,
}

/// 一筆新發現（對應 Python `{"repo": id, **latest}`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub repo: String,
    pub kind: String,
    pub id: String,
    pub title: String,
    pub url: String,
}

fn state_path(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("upstream.json")
}

fn load_state(spine_dir: &Path) -> Result<Map<String, Value>> {
    let p = state_path(spine_dir);
    if !p.exists() {
        return Ok(Map::new());
    }
    match serde_json::from_str(&std::fs::read_to_string(&p)?)? {
        Value::Object(m) => Ok(m),
        _ => Ok(Map::new()),
    }
}

fn save_state(spine_dir: &Path, state: &Map<String, Value>) -> Result<()> {
    let p = state_path(spine_dir);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // json.dumps(ensure_ascii=False, indent=1)
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    state.serialize(&mut ser)?;
    std::fs::write(p, buf)?;
    Ok(())
}

/// 對應 `upstream.normalize`：upstream 欄位 → owner/repo（容忍完整 URL 與 .git 尾綴）。
pub fn normalize(upstream: &str) -> String {
    let mut u = upstream.trim().trim_end_matches('/');
    for prefix in ["https://github.com/", "http://github.com/", "github.com/"] {
        if let Some(rest) = u.strip_prefix(prefix) {
            u = rest;
        }
    }
    u.strip_suffix(".git").unwrap_or(u).to_string()
}

fn gh_api(path: &str) -> Result<Option<Value>> {
    let o = Command::new("gh").args(["api", path]).output()?;
    if !o.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&o.stdout);
    let text = if text.trim().is_empty() {
        "[]"
    } else {
        text.as_ref()
    };
    Ok(Some(serde_json::from_str(text)?))
}

fn str_field(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => crate::pystr::py_str(other),
    }
}

/// 對應 `upstream.gh_fetch`：`gh api` 抓最新一筆。查無／gh 失敗＝`Ok(None)`；未知 kind＝`Error::Invalid`。
pub fn gh_fetch(upstream: &str, kind: &str, prerelease: bool) -> Result<Option<Latest>> {
    let up = normalize(upstream);
    match kind {
        "release" => {
            let Some(Value::Array(rels)) = gh_api(&format!("repos/{up}/releases?per_page=10"))?
            else {
                return Ok(None);
            };
            for rel in &rels {
                if rel.get("draft").and_then(Value::as_bool).unwrap_or(false) {
                    continue;
                }
                if rel
                    .get("prerelease")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    && !prerelease
                {
                    continue;
                }
                let tag = str_field(rel, "tag_name");
                let name = str_field(rel, "name");
                return Ok(Some(Latest {
                    kind: "release".into(),
                    title: if name.is_empty() { tag.clone() } else { name },
                    id: tag,
                    url: str_field(rel, "html_url"),
                }));
            }
            Ok(None)
        }
        "commit" => {
            let Some(Value::Array(commits)) = gh_api(&format!("repos/{up}/commits?per_page=1"))?
            else {
                return Ok(None);
            };
            let Some(c) = commits.first() else {
                return Ok(None);
            };
            let sha = str_field(c, "sha");
            let msg = c
                .get("commit")
                .map(|m| str_field(m, "message"))
                .unwrap_or_default();
            Ok(Some(Latest {
                kind: "commit".into(),
                id: crate::pystr::take_chars(&sha, 7).to_string(),
                title: crate::pystr::splitlines(&msg)
                    .first()
                    .copied()
                    .unwrap_or("")
                    .to_string(),
                url: str_field(c, "html_url"),
            }))
        }
        other => Err(Error::invalid(format!(
            "未知 upstream kind: {other}（release|commit）"
        ))),
    }
}

/// 對應 `upstream.check`（fetch＝真的 `gh api`）。回傳 (external_checked 數, 新發現)。
pub fn check(
    spine_dir: impl AsRef<Path>,
    group: Option<&str>,
    repos: Option<&[String]>,
    when: Option<NaiveDateTime>,
) -> Result<(usize, Vec<Finding>)> {
    check_with(spine_dir, group, repos, &mut gh_fetch, when)
}

/// 注入的 fetch：(upstream, kind, prerelease) → 最新一筆（簽名同 [`gh_fetch`]）。
pub type Fetcher<'a> = dyn FnMut(&str, &str, bool) -> Result<Option<Latest>> + 'a;

/// `check` 的本體，fetch 可注入（測試用；簽名同 [`gh_fetch`]：(upstream, kind, prerelease)）。
pub fn check_with(
    spine_dir: impl AsRef<Path>,
    group: Option<&str>,
    repos: Option<&[String]>,
    fetch: &mut Fetcher<'_>,
    when: Option<NaiveDateTime>,
) -> Result<(usize, Vec<Finding>)> {
    let spine_dir = spine_dir.as_ref();
    let th = config::load(spine_dir)?.thresholds();
    let group = group.filter(|g| !g.is_empty());
    let repos = repos.filter(|r| !r.is_empty());
    let entries = if group.is_some() || repos.is_some() {
        registry::resolve_group(spine_dir, group, repos)?.1
    } else {
        registry::load(spine_dir)?.repos
    };
    let externals: Vec<_> = entries
        .iter()
        .filter(|e| e.kind().as_deref() == Some("external"))
        .filter_map(|e| e.upstream().map(|u| (e.id(), u)))
        .collect();
    let mut state = load_state(spine_dir)?;
    let mut findings = Vec::new();
    for (rid, up) in &externals {
        let seen = state
            .entry(rid.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        if !seen.is_object() {
            *seen = Value::Object(Map::new());
        }
        for kind in &th.upstream_kinds {
            let Some(latest) = fetch(up, kind, th.upstream_prerelease)? else {
                continue;
            };
            if seen.get(kind.as_str()).and_then(Value::as_str) == Some(latest.id.as_str()) {
                continue;
            }
            if let Some(m) = seen.as_object_mut() {
                m.insert(kind.clone(), Value::String(latest.id.clone()));
            }
            let mut body = format!("上游 {kind}：{} {}", latest.id, latest.title);
            if !latest.url.is_empty() {
                body.push_str(&format!("\n{}", latest.url));
            }
            spine::append_event(
                spine_dir,
                "suggestion",
                "upstream",
                &[format!("repo:{rid}")],
                &body,
                when,
            )?;
            findings.push(Finding {
                repo: rid.clone(),
                kind: latest.kind,
                id: latest.id,
                title: latest.title,
                url: latest.url,
            });
        }
    }
    save_state(spine_dir, &state)?;
    Ok((externals.len(), findings))
}
