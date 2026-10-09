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
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::agents;
use crate::fsutil::{expanduser, read_text_lossy, resolve as py_resolve};
use crate::knowledge::{Boosts, PathBoost, Snippet};
use crate::locking::FileLock;
use crate::pyjson::dumps as py_dumps;
use crate::pystr::{repr, splitlines, take_chars};
use crate::registry::{self, Direction, Export, Repo};
use crate::spine::LOCK_NAME;
use crate::{config, pack, Error, Result};

/// 對應 `_MAX_READ_CHARS`。
pub const MAX_READ_CHARS: usize = 40_000;
const MAX_DIR_ENTRIES: usize = 200;

fn ref_err(code: &str, message: impl Into<String>, hint: Option<String>) -> Error {
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
            "bad_request",
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
        "not_found",
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
            "bad_request",
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

// ---------------------------------------------------------------- 讀 ＋ 引用紀錄

fn cites_path(spine_dir: &Path) -> PathBuf {
    spine_dir.join("refs").join("cites.jsonl")
}

/// `refs/cites.jsonl` 的一筆引用。欄位順序同 Python 寫出的順序；不認得的鍵保留在 `extra`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Cite {
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub consumer: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub rel: String,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub via: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// 對應 `crossref.load_cites`：壞行略過。
pub fn load_cites(spine_dir: &Path) -> Vec<Cite> {
    let Ok(text) = std::fs::read_to_string(cites_path(spine_dir)) else {
        return Vec::new();
    };
    splitlines(&text)
        .into_iter()
        .filter_map(|ln| serde_json::from_str::<Cite>(ln).ok())
        .collect()
}

/// 對應 `crossref.record_cite`：記一筆引用。同一天同 (consumer, provider, rel, via) 只記一次
/// （讀十次不是十個訊號）。consumer 是 None（在 spine 或未登記目錄）或等於 provider（自己讀自己）時不記。
#[allow(clippy::too_many_arguments)]
pub fn record_cite(
    spine_dir: &Path,
    consumer: Option<&str>,
    provider: &str,
    rel: &str,
    commit: Option<&str>,
    via: &str,
    note: Option<&str>,
    when: Option<NaiveDateTime>,
) -> Result<Option<Cite>> {
    let Some(consumer) = consumer.filter(|c| !c.is_empty() && *c != provider) else {
        return Ok(None);
    };
    let now = when.unwrap_or_else(|| Local::now().naive_local());
    let rec = Cite {
        at: now.format("%Y-%m-%d %H:%M").to_string(),
        consumer: consumer.to_string(),
        provider: provider.to_string(),
        rel: rel.to_string(),
        commit: commit.map(str::to_string),
        via: via.to_string(),
        note: note.filter(|n| !n.is_empty()).map(str::to_string),
        extra: Map::new(),
    };
    let p = cites_path(spine_dir);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _g = FileLock::new(spine_dir.join(LOCK_NAME)).acquire()?;
    let day = &rec.at[..10];
    for c in load_cites(spine_dir) {
        if take_chars(&c.at, 10) == day
            && c.consumer == rec.consumer
            && c.provider == rec.provider
            && c.rel == rec.rel
            && c.via == rec.via
        {
            return Ok(None);
        }
    }
    let line = py_dumps(&serde_json::to_value(&rec)?);
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)?;
    f.write_all(format!("{line}\n").as_bytes())?;
    Ok(Some(rec))
}

/// `git rev-parse --short HEAD`（拿不到＝None）。
pub(crate) fn head(root: &Path) -> Option<String> {
    let (rc, out) = git(root, &["rev-parse", "--short", "HEAD"], GIT_TIMEOUT);
    (rc == 0 && !out.is_empty()).then_some(out)
}

static LINES_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*(\d+)\s*(?:-\s*(\d+))?\s*$").expect("寫死的 regex"));

fn parse_lines(lines: Option<&str>) -> Result<Option<(usize, usize)>> {
    let Some(s) = lines.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let bad = || {
        ref_err(
            "bad_request",
            format!("lines 格式不對：{}", repr(s)),
            Some("用 120 或 120-180。".into()),
        )
    };
    let m = LINES_RE.captures(s).ok_or_else(bad)?;
    let a: usize = m[1].parse().map_err(|_| bad())?;
    let b: usize = match m.get(2) {
        Some(x) => x.as_str().parse().map_err(|_| bad())?,
        None => a,
    };
    Ok(Some((a.min(b), a.max(b))))
}

/// 目錄清單的一項：子目錄（帶檔數）或檔案。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirEntry {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<usize>,
}

/// [`read`] 的結果（鍵名、順序同 Python；目錄與檔案各有自己的欄位）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReadOut {
    pub ok: bool,
    pub repo: String,
    pub export: Option<String>,
    pub rel: String,
    pub path: String,
    pub commit: Option<String>,
    pub kind: PathKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<DirEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_entries: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_lines: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_commit: Option<String>,
}

fn walk_files(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_files(&p, root, out);
        } else if p.is_file() {
            out.push(posix_rel(&p, root));
        }
    }
}

/// Python 切片 `xs[start:stop]`（start 可為 -1：`a=0` 時 Python 會算成 `[-1:b]`）。
fn py_slice<'a>(xs: &[&'a str], start: i64, stop: i64) -> Vec<&'a str> {
    let n = xs.len() as i64;
    let norm = |i: i64| if i < 0 { (n + i).max(0) } else { i.min(n) };
    let (s, e) = (norm(start), norm(stop));
    if s >= e {
        return Vec::new();
    }
    xs[s as usize..e as usize].to_vec()
}

/// 對應 `crossref.read`（L2）：讀 `<repo>:<export>/子路徑`。檔案回內容（可指定行段），目錄回清單。
/// 一律附 commit（引用時說得出版本）；長得像憑證的檔拒讀（`secret`，洩密哨兵同 pack）。
/// Python 預設 `max_chars=MAX_READ_CHARS`。
pub fn read(
    spine_dir: &Path,
    reference: &str,
    consumer: Option<&str>,
    lines: Option<&str>,
    max_chars: usize,
) -> Result<ReadOut> {
    let r = resolve(spine_dir, reference)?;
    if !r.exists {
        return Err(ref_err(
            "not_found",
            format!("不存在：{}/{}", r.repo, r.rel),
            r.hint.clone(),
        ));
    }
    let root = PathBuf::from(&r.root);
    let target = PathBuf::from(&r.path);
    let mut out = ReadOut {
        ok: true,
        repo: r.repo.clone(),
        export: r.export.clone(),
        rel: r.rel.clone(),
        path: r.path.clone(),
        commit: head(&root),
        kind: r.kind,
        entries: None,
        total_entries: None,
        lines: None,
        total_lines: None,
        truncated: None,
        content: None,
        hint: None,
        file_commit: None,
    };
    if r.kind == PathKind::Dir {
        let spec = if r.rel == "." { "." } else { r.rel.as_str() };
        let (rc, listed) = git(
            &root,
            &["ls-files", "-z", "-co", "--exclude-standard", "--", spec],
            GIT_TIMEOUT,
        );
        let names: Vec<String> = if rc == 0 {
            listed
                .split('\0')
                .filter(|x| !x.is_empty())
                .map(str::to_string)
                .collect()
        } else {
            let mut v = Vec::new();
            walk_files(&target, &root, &mut v);
            v
        };
        let prefix = if r.rel == "." {
            String::new()
        } else {
            format!("{}/", r.rel.trim_end_matches('/'))
        };
        let mut children: BTreeMap<String, usize> = BTreeMap::new();
        let mut files = Vec::new();
        for n in &names {
            let sub = n.strip_prefix(prefix.as_str()).unwrap_or(n);
            match sub.split_once('/') {
                Some((first, _)) => *children.entry(format!("{first}/")).or_default() += 1,
                None => files.push(sub.to_string()),
            }
        }
        files.sort();
        let entries: Vec<DirEntry> = children
            .into_iter()
            .map(|(name, n)| DirEntry {
                name,
                files: Some(n),
            })
            .chain(files.into_iter().map(|name| DirEntry { name, files: None }))
            .collect();
        out.total_entries = Some(entries.len());
        out.entries = Some(entries.into_iter().take(MAX_DIR_ENTRIES).collect());
        out.hint = Some("挑檔案再 read `<repo>:<路徑>`；不確定哪份相關就先用 find。".into());
    } else {
        let text = read_text_lossy(&target)
            .map_err(|e| ref_err("failed", format!("讀不到：{e}"), None))?;
        let hits = pack::secret_hits(&text);
        if !hits.is_empty() {
            return Err(ref_err(
                "secret",
                format!(
                    "{}/{} 疑似含憑證（{}），不讀出",
                    r.repo,
                    r.rel,
                    hits.join(", ")
                ),
                Some("這是洩密哨兵；真的需要請使用者自己看。".into()),
            ));
        }
        let all_lines = splitlines(&text);
        let body = match parse_lines(lines)? {
            Some((a, b)) => {
                out.lines = Some(format!("{a}-{}", b.min(all_lines.len())));
                py_slice(&all_lines, a as i64 - 1, b as i64).join("\n")
            }
            None => text.clone(),
        };
        let truncated = body.chars().count() > max_chars;
        out.total_lines = Some(all_lines.len());
        out.truncated = Some(truncated);
        out.content = Some(take_chars(&body, max_chars).to_string());
        if truncated {
            out.hint = Some(format!(
                "內容超過 {max_chars} 字被截斷；用 lines 指定段落（總共 {} 行）。",
                all_lines.len()
            ));
        }
        let (rc, fc) = git(
            &root,
            &["log", "-1", "--format=%h", "--", &r.rel],
            GIT_TIMEOUT,
        );
        if rc == 0 && !fc.is_empty() {
            out.file_commit = Some(fc);
        }
    }
    record_cite(
        spine_dir,
        consumer,
        &r.repo,
        &r.rel,
        out.commit.as_deref(),
        "read",
        None,
        None,
    )?;
    Ok(out)
}

// ---------------------------------------------------------------- 程式碼定位（L1 的 code 半邊）

static ASCII_TERM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_\-\.]{2,}").expect("寫死的 regex"));
static CJK_TERM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[\x{3400}-\x{9FFF}\x{F900}-\x{FAFF}]{2,8}").expect("寫死的 regex"));
const CODE_STOP: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "what", "how", "where", "which", "does",
    "into", "about", "when", "use", "are",
];

/// 對應 `crossref.query_terms`：查詢 → git grep 用的字面詞：英數識別字（≥3 字）＋中文詞段（2～8 字）。
/// Python 預設 `limit=8`。
pub fn query_terms(query: &str, limit: usize) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for m in ASCII_TERM.find_iter(query) {
        let t = m.as_str();
        if !CODE_STOP.contains(&t.to_lowercase().as_str()) && !terms.iter().any(|x| x == t) {
            terms.push(t.to_string());
        }
    }
    for m in CJK_TERM.find_iter(query) {
        let t = m.as_str();
        if !terms.iter().any(|x| x == t) {
            terms.push(t.to_string());
        }
    }
    terms.truncate(limit);
    terms
}

/// 程式碼命中一份檔（對應 `code_search` 回傳的 dict）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CodeHit {
    pub repo: String,
    pub file: String,
    pub path: String,
    pub score: f64,
    pub why: Vec<String>,
    pub snippets: Vec<Snippet>,
}

/// 對應 `crossref.code_search`：程式碼不建索引（改得快、索引跟不上），直接 `git grep -F`，排除 *.md
/// （md 走 BM25）。分數＝命中幾個不同的查詢詞，同分看命中行數。Python 預設 `limit=8, per_repo=40`。
pub fn code_search(entries: &[Repo], query: &str, limit: usize, per_repo: usize) -> Vec<CodeHit> {
    let terms = query_terms(query, 8);
    if terms.is_empty() {
        return Vec::new();
    }
    let lows: Vec<String> = terms.iter().map(|t| t.to_lowercase()).collect();
    let mut args: Vec<&str> = vec!["grep", "-n", "-I", "-i", "-F", "--max-count=20"];
    for t in &terms {
        args.push("-e");
        args.push(t);
    }
    args.extend([
        "--",
        ".",
        ":(exclude)*.md",
        ":(exclude)*.lock",
        ":(exclude)*-lock.json",
        ":(exclude)*.min.js",
        ":(exclude)*.svg",
    ]);
    // 各 repo 的 git grep 同時跑（Rust 才有：依序跑時 15 個 repo 要 0.6 秒以上）；結果照 repo 順序合併
    let outs: Vec<Option<(i32, String)>> = std::thread::scope(|sc| {
        let hs: Vec<_> = entries
            .iter()
            .map(|e| {
                let args = &args;
                sc.spawn(move || {
                    let root = e.path();
                    (root.is_dir() && e.kind().as_deref() != Some("external"))
                        .then(|| git(&root, args, Duration::from_secs(10)))
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().ok().flatten()).collect()
    });
    let mut rows: Vec<CodeHit> = Vec::new();
    for (e, res) in entries.iter().zip(outs) {
        let Some((rc, out)) = res else {
            continue;
        };
        if rc != 0 || out.is_empty() {
            continue;
        }
        let root = e.path();
        let mut per_file: Vec<(String, Vec<(usize, String)>)> = Vec::new();
        let mut idx: HashMap<String, usize> = HashMap::new();
        for ln in out.lines().take(2000) {
            let parts: Vec<&str> = ln.splitn(3, ':').collect();
            if parts.len() != 3
                || parts[1].is_empty()
                || !parts[1].chars().all(|c| c.is_ascii_digit())
            {
                continue;
            }
            let Ok(n) = parts[1].parse::<usize>() else {
                continue;
            };
            let i = *idx.entry(parts[0].to_string()).or_insert_with(|| {
                per_file.push((parts[0].to_string(), Vec::new()));
                per_file.len() - 1
            });
            per_file[i].1.push((n, parts[2].to_string()));
        }
        for (f, hits) in per_file.into_iter().take(per_repo) {
            let low = hits
                .iter()
                .map(|h| h.1.to_lowercase())
                .collect::<Vec<_>>()
                .join(" ");
            let matched: Vec<&String> = terms
                .iter()
                .zip(&lows)
                .filter(|(_, l)| low.contains(l.as_str()))
                .map(|(t, _)| t)
                .collect();
            let hit_count = |h: &(usize, String)| {
                let hl = h.1.to_lowercase();
                lows.iter().filter(|l| hl.contains(l.as_str())).count()
            };
            let mut best: Vec<&(usize, String)> = hits.iter().collect();
            best.sort_by_key(|h| std::cmp::Reverse(hit_count(h)));
            best.truncate(2);
            best.sort();
            rows.push(CodeHit {
                repo: e.id(),
                file: f.clone(),
                path: root.join(&f).to_string_lossy().into_owned(),
                score: matched.len() as f64 + hits.len().min(20) as f64 / 100.0,
                why: vec![format!(
                    "程式碼含 {}",
                    matched
                        .iter()
                        .take(4)
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join("、")
                )],
                snippets: best
                    .iter()
                    .map(|(n, t)| Snippet {
                        line: *n,
                        text: take_chars(t.trim(), 160).to_string(),
                    })
                    .collect(),
            });
        }
    }
    rows.sort_by(|a, b| b.score.total_cmp(&a.score));
    for r in &mut rows {
        r.score = crate::knowledge::round2(r.score);
    }
    rows.truncate(limit);
    rows
}

// ---------------------------------------------------------------- 委派（L3）

/// 對應 `crossref.ASK_PROMPT`（`{repo}`／`{consumer}`／`{question}` 由 [`ask_prompt`] 代入）。
pub const ASK_PROMPT: &str = "你在 repo「{repo}」裡，被另一個 repo（{consumer}）的 agent 請來回答一個問題。
只能讀檔（Read／Grep／Glob），不要改任何東西。先照這個 repo 的 AGENTS.md／CLAUDE.md 慣例找資料。

【問題】
{question}

回答規則：
- 只根據這個 repo 裡的檔案回答；找不到就直說找不到，不要編。
- 結論精簡（300 字以內），每個重點標出來源檔（repo 內相對路徑，能給行號就給）。
- 最後只輸出一個 JSON 物件，不要其他文字：
{\"answer\": \"<回答>\", \"citations\": [{\"file\": \"<相對路徑>\", \"lines\": \"<例 12-40，可省略>\", \"why\": \"<這份檔支持哪個重點>\"}], \"not_found\": false}
";

/// 組 ask_repo 的 prompt（Python `ASK_PROMPT.format(...)`）。
pub fn ask_prompt(repo: &str, consumer: &str, question: &str) -> String {
    ASK_PROMPT
        .replacen("{repo}", repo, 1)
        .replacen("{consumer}", consumer, 1)
        .replacen("{question}", question, 1)
}

/// ask_repo 回答解析後的形狀（對應 `_parse_ask`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AskParsed {
    pub answer: String,
    pub citations: Vec<Value>,
    pub not_found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// 對應 `crossref._parse_ask`：取第一個 `{` 到最後一個 `}` 解析；不是預期的 JSON 就原文照回。
pub fn parse_ask(text: &str) -> AskParsed {
    if let (Some(a), Some(b)) = (text.find('{'), text.rfind('}')) {
        if a < b {
            if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&text[a..=b]) {
                if let Some(ans) = obj.get("answer") {
                    let citations = match obj.get("citations") {
                        Some(Value::Array(xs)) => xs
                            .iter()
                            .filter(|c| {
                                c.as_object().is_some_and(|o| {
                                    o.get("file").is_some_and(crate::pystr::truthy)
                                })
                            })
                            .cloned()
                            .collect(),
                        _ => Vec::new(),
                    };
                    return AskParsed {
                        answer: crate::pystr::py_str(ans),
                        citations,
                        not_found: obj.get("not_found").is_some_and(crate::pystr::truthy),
                        note: None,
                    };
                }
            }
        }
    }
    AskParsed {
        answer: text.trim().to_string(),
        citations: Vec::new(),
        not_found: false,
        note: Some("回答不是預期的 JSON，原文照回；沒有結構化引用。".into()),
    }
}

/// [`ask_repo`] 的結果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AskResult {
    pub ok: bool,
    pub repo: String,
    pub provider: String,
    pub commit: Option<String>,
    #[serde(flatten)]
    pub parsed: AskParsed,
}

/// 文字 agent 的呼叫介面（同 `agents::run_text` 的參數：provider 名, prompt, spine, model, cwd, read_only）。
/// 測試注入假的用（Python 是 monkeypatch `agents.run_text`）。
pub type RunText<'a> =
    dyn Fn(&str, &str, &Path, Option<&str>, Option<&Path>, bool) -> Result<String> + 'a;

/// 對應 `crossref.ask_repo`（L3）：在對方 repo 開一個唯讀 agent 回答問題，只回結論＋引用，
/// 不把一堆檔案塞進你的 context。provider 預設 config `provider.ask`（沒設＝claude）。
pub fn ask_repo(
    spine_dir: &Path,
    repo_id: &str,
    question: &str,
    consumer: Option<&str>,
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<AskResult> {
    ask_repo_with(
        spine_dir,
        repo_id,
        question,
        consumer,
        provider,
        model,
        &agents::run_text,
    )
}

/// [`ask_repo`] 的本體，文字 agent 由呼叫端給。
#[allow(clippy::too_many_arguments)]
pub fn ask_repo_with(
    spine_dir: &Path,
    repo_id: &str,
    question: &str,
    consumer: Option<&str>,
    provider: Option<&str>,
    model: Option<&str>,
    run_text: &RunText<'_>,
) -> Result<AskResult> {
    if question.trim().is_empty() {
        return Err(ref_err("bad_request", "question 不能是空的", None));
    }
    let entry = repo_entry(spine_dir, repo_id)?;
    let root = py_resolve(&entry.path());
    if !root.is_dir() {
        return Err(ref_err(
            "not_found",
            format!("{repo_id} 的路徑不存在：{}", root.display()),
            None,
        ));
    }
    let cfg = config::load(spine_dir)?;
    let provider = provider
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .or_else(|| {
            cfg.str("provider.ask")
                .filter(|p| !p.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "claude".into());
    let prompt = ask_prompt(
        repo_id,
        consumer.filter(|c| !c.is_empty()).unwrap_or("未登記的目錄"),
        question.trim(),
    );
    let raw = match run_text(&provider, &prompt, spine_dir, model, Some(&root), true) {
        Ok(s) => s,
        Err(Error::External(e)) => {
            return Err(ref_err(
                "failed",
                format!("ask_repo 失敗：{e}"),
                Some("改用 find＋read 自己找；或確認 claude CLI 已登入。".into()),
            ))
        }
        Err(e) => return Err(e),
    };
    let parsed = parse_ask(&raw);
    let commit = head(&root);
    for c in &parsed.citations {
        let file = c.get("file").map(crate::pystr::py_str).unwrap_or_default();
        let rel = file.trim_start_matches(['.', '/']);
        record_cite(
            spine_dir,
            consumer,
            repo_id,
            rel,
            commit.as_deref(),
            "ask",
            None,
            None,
        )?;
    }
    Ok(AskResult {
        ok: true,
        repo: repo_id.to_string(),
        provider,
        commit,
        parsed,
    })
}

// ---------------------------------------------------------------- drift ＋ 建議 exports

/// 上游最新一則改動。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LatestChange {
    pub hash: String,
    pub date: String,
    pub subject: String,
}

/// [`drift`] 的一筆（鍵名、順序同 Python）。`changes=None`＝引用時的 commit 已不在歷史裡（附 note）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Drift {
    pub consumer: String,
    pub provider: String,
    pub rel: String,
    pub cited_commit: String,
    pub cited_at: String,
    pub changes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<LatestChange>,
}

/// 對應 `crossref.drift`：引用過的上游檔，在引用當時的 commit 之後又改過 → 下游可能過期。
/// 每個 (consumer, provider, rel) 只看最近一次引用。
pub fn drift(
    spine_dir: &Path,
    consumer: Option<&str>,
    provider: Option<&str>,
) -> Result<Vec<Drift>> {
    let mut latest: BTreeMap<(String, String, String), Cite> = BTreeMap::new();
    for c in load_cites(spine_dir) {
        if consumer.is_some_and(|x| !x.is_empty() && c.consumer != x)
            || provider.is_some_and(|x| !x.is_empty() && c.provider != x)
        {
            continue;
        }
        let key = (c.consumer.clone(), c.provider.clone(), c.rel.clone());
        if latest.get(&key).is_none_or(|old| c.at >= old.at) {
            latest.insert(key, c);
        }
    }
    let roots: HashMap<String, PathBuf> = registry::load(spine_dir)?
        .repos
        .iter()
        .map(|r| (r.id(), r.path()))
        .collect();
    let mut out = Vec::new();
    for ((cons, prov, rel), c) in latest {
        let Some(root) = roots.get(&prov).filter(|r| r.is_dir()) else {
            continue;
        };
        let Some(commit) = c.commit.clone().filter(|s| !s.is_empty()) else {
            continue;
        };
        let range = format!("{commit}..HEAD");
        let (rc, log) = git(
            root,
            &[
                "log",
                "--format=%h%x09%ad%x09%s",
                "--date=short",
                &range,
                "--",
                &rel,
            ],
            GIT_TIMEOUT,
        );
        if rc != 0 {
            out.push(Drift {
                consumer: cons,
                provider: prov,
                rel,
                cited_commit: commit,
                cited_at: c.at,
                changes: None,
                note: Some("引用時的 commit 已不在歷史裡（rebase？），無法比對".into()),
                latest: None,
            });
            continue;
        }
        let commits: Vec<Vec<&str>> = log
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.splitn(3, '\t').collect())
            .collect();
        if let Some(first) = commits.first() {
            let part = |i: usize| first.get(i).copied().unwrap_or("").to_string();
            out.push(Drift {
                consumer: cons,
                provider: prov,
                rel,
                cited_commit: commit,
                cited_at: c.at,
                changes: Some(commits.len()),
                note: None,
                latest: Some(LatestChange {
                    hash: part(0),
                    date: part(1),
                    subject: part(2),
                }),
            });
        }
    }
    Ok(out)
}

/// 對應 `crossref.drift_lines`。Python 預設 `limit=5`。
pub fn drift_lines(items: &[Drift], limit: usize) -> Vec<String> {
    let mut lines: Vec<String> = items
        .iter()
        .take(limit)
        .map(|d| match (&d.changes, &d.latest) {
            (Some(n), Some(l)) => format!(
                "- {} 引用的 {}:{}（{}）之後改了 {n} 次；最新 {}「{}」",
                d.consumer,
                d.provider,
                d.rel,
                take_chars(&d.cited_at, 10),
                l.date,
                l.subject
            ),
            _ => format!(
                "- {} 引用的 {}:{}：{}",
                d.consumer,
                d.provider,
                d.rel,
                d.note.as_deref().unwrap_or("")
            ),
        })
        .collect();
    if items.len() > limit {
        lines.push(format!(
            "- …另 {} 筆（ctx refs drift 看全部）",
            items.len() - limit
        ));
    }
    lines
}

/// [`suggest_exports`] 的一筆。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportSuggestion {
    pub repo: String,
    pub path: String,
    pub cites: usize,
    pub consumers: Vec<String>,
    pub command: String,
}

/// 對應 `crossref.suggest_exports`：常被別的 repo 引用、卻沒被任何 export 涵蓋的目錄 → 建議宣告成 export。
/// depth＝取引用路徑的前幾層目錄當候選。Python 預設 `min_count=3, depth=1`。
pub fn suggest_exports(
    spine_dir: &Path,
    min_count: usize,
    depth: usize,
) -> Result<Vec<ExportSuggestion>> {
    let data = registry::load(spine_dir)?;
    let exp: HashMap<String, Vec<String>> = data
        .repos
        .iter()
        .map(|r| {
            (
                r.id(),
                registry::exports_of(r)
                    .into_iter()
                    .map(|(_, v)| v.path.trim_end_matches('/').to_string())
                    .collect(),
            )
        })
        .collect();
    // Counter＋most_common：依次數降冪，同數照首次出現順序
    let mut counts: Vec<((String, String), usize, BTreeSet<String>)> = Vec::new();
    for c in load_cites(spine_dir) {
        let covered = exp.get(&c.provider).is_some_and(|ps| {
            ps.iter().any(|p| {
                p == "." || p.is_empty() || c.rel == *p || c.rel.starts_with(&format!("{p}/"))
            })
        });
        if covered {
            continue;
        }
        let parts: Vec<&str> = c.rel.split('/').collect();
        let cand = if parts.len() > depth {
            parts[..depth].join("/")
        } else if parts.len() > 1 {
            parts[0].to_string()
        } else {
            c.rel.clone()
        };
        let key = (c.provider.clone(), cand);
        match counts.iter_mut().find(|x| x.0 == key) {
            Some(x) => {
                x.1 += 1;
                x.2.insert(c.consumer.clone());
            }
            None => counts.push((key, 1, BTreeSet::from([c.consumer.clone()]))),
        }
    }
    counts.sort_by_key(|x| std::cmp::Reverse(x.1));
    Ok(counts
        .into_iter()
        .filter(|x| x.1 >= min_count)
        .map(|((prov, cand), n, cons)| ExportSuggestion {
            command: format!("ctx registry export {prov} <名稱> {cand}"),
            repo: prov,
            path: cand,
            cites: n,
            consumers: cons.into_iter().collect(),
        })
        .collect())
}

// ---------------------------------------------------------------- agent 文件裡的跨 repo 路徑

/// 對應 `crossref.AGENT_DOCS`。
pub const AGENT_DOCS: [&str; 3] = ["AGENTS.md", "CLAUDE.md", "GEMINI.md"];
/// 對應 `crossref.SKILL_GLOBS`（`<dir>/*/SKILL.md`）。
pub const SKILL_DIRS: [&str; 2] = [".claude/skills", ".agents/skills"];

static CODE_SPAN: Lazy<Regex> = Lazy::new(|| Regex::new(r"`([^`\n]+)`").expect("寫死的 regex"));
static MD_LINK: Lazy<Regex> = Lazy::new(|| Regex::new(r"\]\(([^)\s]+)\)").expect("寫死的 regex"));
/// `_BARE_UP` 去掉 lookbehind 的部分（regex crate 不支援 lookbehind，由 [`bare_ups`] 補判）。
static BARE_UP: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^\.\./[^\s`)\]'"，。、；：）]+"#).expect("寫死的 regex"));
static GLOB_CUT: Lazy<Regex> = Lazy::new(|| Regex::new(r"[*<{?]").expect("寫死的 regex"));
static LINE_SUFFIX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r":\d+(?:-\d+)?$").expect("寫死的 regex"));
const TRAIL: &[char] = &[
    '，', '。', '、', '；', '：', '）', ')', ']', '}', '>', '\'', '"', '.', ',', ';', ':',
];

fn doc_files(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = AGENT_DOCS
        .iter()
        .map(|n| root.join(n))
        .filter(|p| p.is_file())
        .collect();
    for d in SKILL_DIRS {
        let mut found: Vec<PathBuf> = std::fs::read_dir(root.join(d))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path().join("SKILL.md"))
                    .filter(|p| p.is_file())
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        out.extend(found);
    }
    out
}

/// Python `(?<![\w/.`])(\.\./...)` 的 findall：逐字元掃，前一字元不是字元／`/`／`.`／反引號才算。
fn bare_ups(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut prev: Option<char> = None;
    let mut i = 0;
    while i < line.len() {
        let ok_before =
            prev.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || "/.`".contains(c)));
        if ok_before {
            if let Some(m) = BARE_UP.find(&line[i..]) {
                out.push(m.as_str().to_string());
                let end = i + m.end();
                prev = line[..end].chars().next_back();
                i = end;
                continue;
            }
        }
        let c = line[i..].chars().next().expect("i 在字元邊界上");
        prev = Some(c);
        i += c.len_utf8();
    }
    out
}

fn path_candidates(line: &str) -> Vec<String> {
    let cands = CODE_SPAN
        .captures_iter(line)
        .map(|c| c[1].to_string())
        .chain(MD_LINK.captures_iter(line).map(|c| c[1].to_string()))
        .chain(bare_ups(line));
    let mut out: Vec<String> = Vec::new();
    for c in cands {
        let c = c.trim().trim_end_matches(TRAIL);
        if !c.contains('/')
            || c.contains(' ')
            || c.contains("://")
            || c.starts_with(['-', '@', '#'])
        {
            continue;
        }
        if c.contains("...") || c.contains('…') {
            continue; // 省略號＝示意路徑，不驗
        }
        let c = GLOB_CUT.split(c).next().unwrap_or(""); // glob／佔位符之後不驗
        let c = LINE_SUFFIX.replace(c, "").into_owned(); // 檔案:行號
        if !c.is_empty() && !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// [`lint_doc_refs`] 的一筆失效引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BrokenRef {
    pub repo: String,
    pub doc: String,
    pub line: usize,
    #[serde(rename = "ref")]
    pub reference: String,
    pub target: String,
    pub target_path: String,
    pub fix: String,
}

/// 對應 `crossref.lint_doc_refs`：掃已登記 repo 的 agent 文件（AGENTS／CLAUDE／GEMINI.md、skills 的 SKILL.md），
/// 找出「指向其他已登記 repo、但路徑已失效」的引用。只報有把握的：路徑裡要出現某個其他 repo 的 id
/// 或資料夾名，且依文件所在位置、repo 根目錄都解析不到；以 repo 名開頭、子路徑在該 repo 裡確實存在的
/// （`<repo>/子路徑`）視為用名字引用，不報。`repo_ids=None`＝全部。
pub fn lint_doc_refs(spine_dir: &Path, repo_ids: Option<&[String]>) -> Result<Vec<BrokenRef>> {
    let data = registry::load(spine_dir)?;
    let mut names: HashMap<String, &Repo> = HashMap::new();
    for r in &data.repos {
        names.insert(r.id(), r);
    }
    for r in &data.repos {
        if let Some(n) = r.path().file_name() {
            names.entry(n.to_string_lossy().into_owned()).or_insert(r);
        }
    }
    let mut out = Vec::new();
    for r in data
        .repos
        .iter()
        .filter(|r| repo_ids.is_none_or(|ids| ids.contains(&r.id())))
    {
        let root = r.path();
        if !root.is_dir() {
            continue;
        }
        let own: [String; 2] = [
            r.id(),
            root.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ];
        for doc in doc_files(&root) {
            let Ok(text) = read_text_lossy(&doc) else {
                continue;
            };
            let mut in_fence = false;
            for (i, line) in splitlines(&text).into_iter().enumerate() {
                if line.trim_start().starts_with("```") {
                    in_fence = !in_fence;
                    continue;
                }
                if in_fence || line.contains("~~") {
                    continue; // 程式碼區塊是範例；刪除線是已作廢的舊路徑
                }
                for c in path_candidates(line) {
                    let segs: Vec<&str> = c
                        .split('/')
                        .filter(|s| !["", ".", "..", "~"].contains(s))
                        .collect();
                    let Some(seg) = segs
                        .iter()
                        .copied()
                        .find(|s| names.contains_key(*s) && !own.iter().any(|o| o == s))
                    else {
                        continue;
                    };
                    let target = names[seg];
                    let tries: Vec<PathBuf> = if c.starts_with('~') || c.starts_with('/') {
                        vec![expanduser(&c)]
                    } else {
                        vec![doc.parent().unwrap_or(&root).join(&c), root.join(&c)]
                    };
                    if tries.iter().any(|t| t.exists()) {
                        continue;
                    }
                    let after = c
                        .split_once(seg)
                        .map(|(_, a)| a.trim_matches('/'))
                        .unwrap_or("")
                        .to_string();
                    let tpath = target.path();
                    if segs[0] == seg && tpath.is_dir() && tpath.join(&after).exists() {
                        continue; // `<repo名>/子路徑` 且子路徑在該 repo 裡存在＝用名字引用，不算失效
                    }
                    let tid = target.id();
                    let reference = if after.is_empty() {
                        tid.clone()
                    } else {
                        format!("{tid}:{after}")
                    };
                    out.push(BrokenRef {
                        repo: r.id(),
                        doc: posix_rel(&doc, &root),
                        line: i + 1,
                        reference: c.clone(),
                        target: tid.clone(),
                        target_path: target.path_str(),
                        fix: format!(
                            "{tid} 實際在 {}；改寫成 `{reference}` 並用 `ctx resolve {reference}` 取路徑",
                            target.path_str()
                        ),
                    });
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
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
