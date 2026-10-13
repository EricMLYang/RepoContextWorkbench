//! 跨 repo 參考（2026-09-25 跨 repo 參考輪）——agent 在 A repo 工作，要又快又準地拿到 B repo 的東西。
//! 對應 `repo_context/crossref.py`。
//!
//! 之前的做法與問題：AGENTS.md 寫死相對路徑（repo 一搬家就斷、沒人發現）；整批複製進來（上游一改，
//! 副本默默過期）；agent 不知道對方 repo 哪裡有料，只好整份讀或亂 grep。
//!
//! 四層，由便宜到貴（agent 先用便宜的，不夠再往下）：
//! - L0 地圖 [`neighbors`]：開場注入「有關係的 repo 提供什麼（exports）」——只注入鄰居，不是全部
//! - L1 定位 [`code_search`]＋`knowledge::search`（md，標題加權、關係加分）
//! - L2 讀取 [`read`]：用 `<repo>:<export>/子路徑` 讀，回傳附 commit，並記一筆引用
//! - L3 委派 [`ask_repo`]：在對方 repo 開一個唯讀 agent 回答（對方的 AGENTS.md／skill 自動生效）
//!
//! 引用紀錄（cites）讓它越用越準：每次 read／ask_repo 記「誰引用了誰的哪個檔、當時的 commit」，
//! 落在 spine 的 `refs/cites.jsonl`（不是事件檔——引用不是要人處理的事，不進收件匣；但跟 spine
//! 一起被批次 commit，換機器也在）。由此算出 [`drift`]（引用過的上游檔之後又改了）與
//! [`suggest_exports`]（常被引用、卻還沒宣告成 export 的目錄）。
//! 另有 [`lint_doc_refs`]：掃各 repo 的 agent 文件，抓出指向其他 repo 卻已失效的路徑。
//!
//! 錯誤：Python 的 `RefError(code, message, hint)` → [`Error::Rejected`]（同樣的 code／hint 形狀）。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDateTime};
use regex::Regex;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};
use std::sync::LazyLock;

use crate::agents;
use crate::fsutil::{expanduser, read_text_lossy, resolve as py_resolve};
use crate::knowledge::{Boosts, PathBoost, Snippet};
use crate::locking::FileLock;
use crate::pyjson::dumps as py_dumps;
use crate::pystr::{repr, splitlines, take_chars};
use crate::registry::{self, Direction, Export, Repo};
use crate::spine::LOCK_NAME;
use crate::{config, pack, Error, Result};

mod ask;
mod code;
mod doclint;
mod drift;
mod read;

pub use ask::*;
pub use code::*;
pub use doclint::*;
pub use drift::*;
pub use read::*;

/// 對應 `_MAX_READ_CHARS`。
pub const MAX_READ_CHARS: usize = 40_000;
const MAX_DIR_ENTRIES: usize = 200;

fn ref_err(code: crate::RejectCode, message: impl Into<String>, hint: Option<String>) -> Error {
    Error::rejected(code, message, hint)
}

/// 對應 `crossref._git`／`repocard._git`：`git -c core.quotePath=false -C <path> ...`，
/// 回傳 (exit code, stdout 去掉結尾換行)；跑不起來或逾時＝(1, "")。
pub(crate) fn git(path: &Path, args: &[&str], timeout: Duration) -> (i32, String) {
    let child = Command::new("git")
        .args(["-c", "core.quotePath=false", "-C"])
        .arg(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return (1, String::new());
    };
    let Some(mut stdout) = child.stdout.take() else {
        return (1, String::new());
    };
    // 另開執行緒讀 stdout，避免管道塞滿卡住子程序
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + timeout;
    let mut nap = Duration::from_millis(1);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => {
                std::thread::sleep(nap);
                nap = (nap * 2).min(Duration::from_millis(20));
            }
            Err(_) => break None,
        }
    };
    let buf = reader.join().unwrap_or_default();
    match status {
        Some(s) => (
            s.code().unwrap_or(1),
            String::from_utf8_lossy(&buf)
                .trim_end_matches('\n')
                .to_string(),
        ),
        None => (1, String::new()),
    }
}

pub(crate) const GIT_TIMEOUT: Duration = Duration::from_secs(15);

/// 有序 (鍵, 值) 清單序列化成 JSON 物件（Python dict 保插入順序）。
pub(crate) fn ser_pairs<S: Serializer, T: Serialize>(
    v: &[(String, T)],
    s: S,
) -> std::result::Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut m = s.serialize_map(Some(v.len()))?;
    for (k, x) in v {
        m.serialize_entry(k, x)?;
    }
    m.end()
}

// ---------------------------------------------------------------- 名字 → 路徑

/// 對應 `crossref.parse_ref`：`<repo>`｜`<repo>:<export>[/<子路徑>]`｜`<repo>:<相對路徑>`｜`<repo>/<相對路徑>` → (repo, rest)。
pub fn parse_ref(reference: &str) -> Result<(String, String)> {
    let r = reference.trim();
    if r.is_empty() {
        return Err(ref_err(
            crate::RejectCode::BadRequest,
            "ref 不能是空的",
            Some(
                "格式：<repo>、<repo>:<export>、<repo>:<export>/<子路徑> 或 <repo>/<相對路徑>"
                    .into(),
            ),
        ));
    }
    let (repo, rest) = if let Some((a, b)) = r.split_once(':') {
        (a, b)
    } else if let Some((a, b)) = r.split_once('/') {
        (a, b)
    } else {
        (r, "")
    };
    Ok((
        repo.trim().to_string(),
        rest.trim().trim_start_matches('/').to_string(),
    ))
}

fn repo_entry(spine_dir: &Path, repo_id: &str) -> Result<Repo> {
    let data = registry::load(spine_dir)?;
    if let Some(r) = data.repo(repo_id) {
        return Ok(r.clone());
    }
    let low = repo_id.to_lowercase();
    let close: Vec<String> = data
        .repos
        .iter()
        .map(Repo::id)
        .filter(|i| {
            let il = i.to_lowercase();
            il.contains(&low) || low.contains(&il)
        })
        .take(5)
        .collect();
    Err(ref_err(
        crate::RejectCode::NotFound,
        format!("沒有登記這個 repo：{repo_id}"),
        Some(if close.is_empty() {
            "用 registry list 看已登記的 repo id。".into()
        } else {
            format!("你是不是要找：{}", close.join("、"))
        }),
    ))
}

/// 路徑種類（`kind` 欄）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PathKind {
    Dir,
    File,
    Missing,
}

/// [`resolve`] 的結果（鍵名、順序同 Python dict）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Resolved {
    pub ok: bool,
    #[serde(rename = "ref")]
    pub reference: String,
    pub repo: String,
    pub export: Option<String>,
    pub rel: String,
    pub path: String,
    pub root: String,
    pub exists: bool,
    pub kind: PathKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

fn posix_rel(p: &Path, root: &Path) -> String {
    p.strip_prefix(root)
        .map(|r| {
            r.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_else(|_| p.to_string_lossy().into_owned())
}

/// 對應 `crossref.resolve`：名字 → 實際路徑。rest 的第一段若是 export 名稱就換成 export 路徑，
/// 否則當成 repo 內相對路徑。路徑跳出 repo＝`bad_request`；沒登記＝`not_found`（hint 給相近 id）。
pub fn resolve(spine_dir: &Path, reference: &str) -> Result<Resolved> {
    let (repo_id, mut rest) = parse_ref(reference)?;
    let entry = repo_entry(spine_dir, &repo_id)?;
    let root = py_resolve(&entry.path());
    let exports = registry::exports_of(&entry);
    let (head, tail) = match rest.split_once('/') {
        Some((h, t)) => (h.to_string(), t.to_string()),
        None => (rest.clone(), String::new()),
    };
    let mut export = None;
    let mut desc = None;
    if !head.is_empty() {
        if let Some((_, ex)) = exports.iter().find(|(n, _)| *n == head) {
            export = Some(head.clone());
            desc = ex.desc.clone();
            let base = ex.path.trim_end_matches('/');
            rest = if tail.is_empty() {
                base.to_string()
            } else {
                format!("{base}/{tail}")
            };
        }
    }
    let rel = rest.trim_matches('/');
    let rel = if rel.is_empty() { "." } else { rel };
    let target = py_resolve(&root.join(rel));
    if target != root && !target.starts_with(&root) {
        return Err(ref_err(
            crate::RejectCode::BadRequest,
            format!("路徑跳出 repo：{reference}"),
            Some("ref 只能指向 repo 內的檔案。".into()),
        ));
    }
    let exists = target.exists();
    let kind = if target.is_dir() {
        PathKind::Dir
    } else if target.is_file() {
        PathKind::File
    } else {
        PathKind::Missing
    };
    let hint = (!exists).then(|| {
        let tail = if exports.is_empty() {
            format!("{repo_id} 還沒宣告 exports，用 find 找檔案。")
        } else {
            format!(
                "{repo_id} 的 exports：{}",
                exports
                    .iter()
                    .map(|(n, v)| format!("{n}（{}）", v.path))
                    .collect::<Vec<_>>()
                    .join("、")
            )
        };
        format!("路徑不存在。{tail}")
    });
    Ok(Resolved {
        ok: true,
        reference: reference.to_string(),
        repo: repo_id,
        export,
        rel: if target == root {
            ".".into()
        } else {
            posix_rel(&target, &root)
        },
        path: target.to_string_lossy().into_owned(),
        root: root.to_string_lossy().into_owned(),
        exists,
        kind,
        desc,
        hint,
    })
}

/// L0 地圖的一列（對應 `neighbors` 回傳的 dict）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Neighbor {
    pub peer: String,
    pub kind: String,
    pub label: String,
    pub direction: Direction,
    pub path: String,
    #[serde(serialize_with = "ser_pairs")]
    pub exports: Vec<(String, Export)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// 對方 feeds 我、而且只給這幾個 export。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<String>,
}

/// 對應 `crossref.neighbors`：跟 repo_id 有關係的 repo，各自提供什麼。關係上有標 exports
/// （例 feeds 只吃「書摘」）時，uses 只列那幾個；exports 仍列對方全部。
pub fn neighbors(spine_dir: &Path, repo_id: &str) -> Result<Vec<Neighbor>> {
    let data = registry::load(spine_dir)?;
    let mut out = Vec::new();
    for rel in registry::relations_of(spine_dir, repo_id)? {
        let Some(peer) = data.repo(&rel.peer) else {
            continue;
        };
        let uses = if !rel.exports.is_empty() && rel.direction == Direction::In {
            rel.exports.clone()
        } else {
            Vec::new()
        };
        out.push(Neighbor {
            peer: rel.peer.clone(),
            kind: rel.kind,
            label: rel.label,
            direction: rel.direction,
            path: peer.path_str(),
            exports: registry::exports_of(peer),
            note: rel.note,
            uses,
        });
    }
    Ok(out)
}

/// 對應 `crossref.neighbor_lines`：鄰居地圖 → 人話行（開場注入用）。Python 預設 `max_exports=4`。
pub fn neighbor_lines(nbs: &[Neighbor], max_exports: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for n in nbs {
        let arrow = format!(
            "{}（{}{}）",
            n.peer,
            n.label,
            if n.direction == Direction::In {
                ""
            } else {
                " 你→它"
            }
        );
        let ex: Vec<&(String, Export)> = n
            .exports
            .iter()
            .filter(|(k, _)| n.uses.is_empty() || n.uses.contains(k))
            .collect();
        if ex.is_empty() {
            lines.push(format!(
                "- {arrow}：沒宣告 exports（用 find 找，或 ask_repo 問它）"
            ));
            continue;
        }
        let items: Vec<String> = ex
            .iter()
            .take(max_exports)
            .map(|(k, v)| {
                format!(
                    "`{}:{k}`→{}{}",
                    n.peer,
                    v.path,
                    v.desc
                        .as_ref()
                        .map(|d| format!("（{d}）"))
                        .unwrap_or_default()
                )
            })
            .collect();
        let more = if ex.len() > max_exports {
            format!(" …另 {} 個", ex.len() - max_exports)
        } else {
            String::new()
        };
        lines.push(format!("- {arrow}：{}{more}", items.join("；")));
    }
    lines
}

/// 對應 agentapi `_search_boosts`：主場 repo 的鄰居加分（×1.5）、鄰居 export 內的檔再加分（×1.2）
/// ——「上游的書摘」該排在不相關 repo 前面。repo_id=None（不在已登記 repo）＝不加分。
pub fn search_boosts(spine_dir: &Path, repo_id: Option<&str>) -> Result<Boosts> {
    let mut b = Boosts::default();
    let Some(rid) = repo_id else {
        return Ok(b);
    };
    for n in neighbors(spine_dir, rid)? {
        b.repos.insert(
            n.peer.clone(),
            (1.5, format!("跟 {rid} 有關係（{} {}）", n.peer, n.label)),
        );
        for (name, v) in &n.exports {
            if !n.uses.is_empty() && !n.uses.contains(name) {
                continue;
            }
            let prefix = v.path.trim_end_matches('/');
            b.paths.push(PathBoost {
                repo: n.peer.clone(),
                prefix: if prefix == "." {
                    String::new()
                } else {
                    format!("{prefix}/")
                },
                factor: 1.2,
                reason: format!("在 {}:{name} 內", n.peer),
            });
        }
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::doclint::bare_ups;
    use super::read::py_slice;
    use super::*;

    #[test]
    fn bare_ups_respects_lookbehind() {
        assert_eq!(bare_ups("看 ../a/b。"), vec!["../a/b"]);
        assert!(bare_ups("x../a").is_empty());
        assert!(bare_ups("`../a`").is_empty());
    }

    #[test]
    fn parse_ref_forms() {
        assert_eq!(parse_ref("a:b/c").unwrap(), ("a".into(), "b/c".into()));
        assert_eq!(parse_ref("a/b").unwrap(), ("a".into(), "b".into()));
        assert_eq!(parse_ref(" a ").unwrap(), ("a".into(), "".into()));
        assert!(parse_ref("  ").is_err());
    }

    #[test]
    fn py_slice_like_python() {
        let xs = ["a", "b", "c"];
        assert_eq!(py_slice(&xs, 1, 2), vec!["b"]);
        assert_eq!(py_slice(&xs, -1, 5), vec!["c"]);
        assert!(py_slice(&xs, 2, 1).is_empty());
    }
}
