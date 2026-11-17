//! 共享 skill（F-133，2026-11-18，Rust 才有）：跨 repo 共用的 skill 放在 spine 用 git 版控
//! （`<spine>/skills/<名稱>/SKILL.md`，資料夾裡可以有其他檔與子資料夾），`ctx setup --claude`／`--codex`
//! 發到該家 CLI 的使用者層 skill 目錄（`<cli home>/skills/<名稱>/`）。
//!
//! - 整個資料夾同步：新增、更新、刪掉來源已沒有的檔。每個由我們發的資料夾放 [`MARK_FILE`]
//!   （記來源 spine 與內容雜湊）。目的地同名資料夾沒有標記＝使用者自己的，不覆蓋、回報衝突；
//!   標記是別的 spine 的也不動。spine 裡刪掉的 skill，目的地帶本 spine 標記的才移除。
//! - 內建的 `repo-context`（[`crate::agentsetup::MCP_NAME`]）名稱保留：spine 裡有同名資料夾就略過。
//! - 不跟隨指到 skill 資料夾外面的 symlink（也不跟指到資料夾的 symlink，免得繞圈）；單檔超過
//!   [`MAX_FILE_BYTES`]、整個 skill 超過 [`MAX_SKILL_BYTES`] 就整個略過——這些檔會進 agent 的脈絡目錄。
//! - [`sync`] 冪等：`write=false` 只算計畫（`--dry-run`、`ctx health` 用），結果跟真的跑一樣。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::Result;

/// spine 底下放共享 skill 的資料夾。
pub const SKILLS_DIR: &str = "skills";
/// 我們發的資料夾裡的標記檔。
pub const MARK_FILE: &str = ".ctx-managed";
/// skill 的入口檔。
pub const SKILL_FILE: &str = "SKILL.md";
/// 單檔上限。
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// 一個 skill 的總量上限。
pub const MAX_SKILL_BYTES: u64 = 5 * 1024 * 1024;
/// 一個 skill 最多幾個檔、幾層資料夾（防呆，正常的 skill 遠低於此）。
const MAX_FILES: usize = 1000;
const MAX_DEPTH: usize = 16;
/// 名稱長度上限（Claude Code 的 skill name 上限是 64）。
const MAX_NAME_LEN: usize = 64;
/// 來源裡不發的東西。
const IGNORED_NAMES: [&str; 2] = [".DS_Store", ".git"];

/// spine 裡這個 skill 為什麼沒發。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// 資料夾名不是小寫英數與 `-`。
    BadName,
    /// 資料夾裡沒有 `SKILL.md`（或它不是一般檔）。
    NoSkillMd,
    /// 跟內建的 `repo-context` 同名。
    Reserved,
    /// `skills/<名稱>` 本身是 symlink。
    Symlink,
    /// 有一個檔超過 [`MAX_FILE_BYTES`]。
    FileTooLarge { file: String, bytes: u64 },
    /// 整個 skill 超過 [`MAX_SKILL_BYTES`]。
    TooLarge { bytes: u64 },
    /// 檔案數超過上限。
    TooManyFiles,
    /// 讀不了。
    Unreadable(String),
}

impl SkipReason {
    pub fn text(&self) -> String {
        match self {
            SkipReason::BadName => "資料夾名只能用小寫英數與 -".into(),
            SkipReason::NoSkillMd => format!("裡面沒有 {SKILL_FILE}"),
            SkipReason::Reserved => format!("{} 是內建 skill 的名字", crate::agentsetup::MCP_NAME),
            SkipReason::Symlink => "資料夾本身是 symlink，不跟".into(),
            SkipReason::FileTooLarge { file, bytes } => format!(
                "{file} 有 {}，單檔上限 {}",
                human(*bytes),
                human(MAX_FILE_BYTES)
            ),
            SkipReason::TooLarge { bytes } => {
                format!("共 {}，上限 {}", human(*bytes), human(MAX_SKILL_BYTES))
            }
            SkipReason::TooManyFiles => format!("超過 {MAX_FILES} 個檔"),
            SkipReason::Unreadable(e) => format!("讀不了（{e}）"),
        }
    }
}

fn human(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KB", (bytes as f64 / 1024.0).ceil())
    }
}

/// 目的地同名資料夾為什麼不能動。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    /// 沒有標記：使用者自己的。
    UserOwned,
    /// 別的 spine 發的。
    OtherSpine(String),
}

impl Conflict {
    pub fn text(&self) -> String {
        match self {
            Conflict::UserOwned => "同名資料夾不是 ctx 發的，不覆蓋".into(),
            Conflict::OtherSpine(s) => format!("同名資料夾是別的 spine（{s}）發的，不動"),
        }
    }
}

/// 一個共享 skill 在某一家 CLI 的結果（`write=false` 時是「會怎樣」）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Installed,
    Updated,
    Unchanged,
    Removed,
    Conflict(Conflict),
    Skipped(SkipReason),
}

/// 一個 skill 的報告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// 資料夾名（不合法的名稱也照原樣）。
    pub name: String,
    pub outcome: Outcome,
    /// 沒發的個別檔（symlink 指到外面、指到資料夾、壞掉的連結），相對 skill 資料夾。
    pub ignored: Vec<String>,
}

/// 名稱合法：小寫英數與 `-`，不以 `-` 開頭或結尾，不超過 64 字。
pub fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

// ── 來源 ────────────────────────────────────────────────────────────────

/// 掃到的一個檔：相對路徑（`/` 分隔）＋實際讀哪裡。
#[derive(Debug, Clone)]
struct SrcFile {
    rel: String,
    path: PathBuf,
}

/// spine 裡一個合法的 skill（還沒讀內容）。
#[derive(Debug, Clone)]
struct Source {
    name: String,
    dir: PathBuf,
    files: Vec<SrcFile>,
    ignored: Vec<String>,
}

/// `<spine>/skills/` 掃完的樣子。
#[derive(Debug, Default)]
struct Listing {
    sources: Vec<Source>,
    skipped: Vec<Report>,
    /// `skills/` 下所有資料夾名（含不合法、略過的）：這些名稱在目的地不會被當成「已刪」移除。
    present: BTreeSet<String>,
}

fn list(spine_dir: &Path) -> Result<Listing> {
    let root = spine_dir.join(SKILLS_DIR);
    let rd = match std::fs::read_dir(&root) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Listing::default()),
        Err(e) => return Err(e.into()),
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    let mut out = Listing::default();
    for e in entries {
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        if name.starts_with('.') {
            continue; // .git、.DS_Store 之類
        }
        let Ok(ft) = e.file_type() else { continue };
        let skip = |reason| Report {
            name: name.clone(),
            outcome: Outcome::Skipped(reason),
            ignored: Vec::new(),
        };
        if ft.is_symlink() {
            // 指到資料夾的 symlink 才算一個 skill（指到檔的跟 README 一樣忽略）
            if e.path().is_dir() {
                out.present.insert(name.clone());
                out.skipped.push(skip(SkipReason::Symlink));
            }
            continue;
        }
        if !ft.is_dir() {
            continue; // skills/README.md 之類
        }
        out.present.insert(name.clone());
        if !name_ok(&name) {
            out.skipped.push(skip(SkipReason::BadName));
            continue;
        }
        if name == crate::agentsetup::MCP_NAME {
            out.skipped.push(skip(SkipReason::Reserved));
            continue;
        }
        match scan(&e.path()) {
            Ok((files, ignored)) => {
                if !files.iter().any(|f| f.rel == SKILL_FILE) {
                    out.skipped.push(Report {
                        name,
                        outcome: Outcome::Skipped(SkipReason::NoSkillMd),
                        ignored,
                    });
                } else {
                    out.sources.push(Source {
                        name,
                        dir: e.path(),
                        files,
                        ignored,
                    });
                }
            }
            Err(r) => out.skipped.push(skip(r)),
        }
    }
    Ok(out)
}

/// 走一個 skill 資料夾（只看 metadata，不讀內容）：回要發的檔與沒跟的 symlink。
fn scan(dir: &Path) -> std::result::Result<(Vec<SrcFile>, Vec<String>), SkipReason> {
    let unreadable = |e: std::io::Error| SkipReason::Unreadable(e.to_string());
    let root = std::fs::canonicalize(dir).map_err(unreadable)?;
    let mut files = Vec::new();
    let mut ignored = Vec::new();
    let mut total = 0u64;
    let mut stack = vec![(String::new(), dir.to_path_buf(), 0usize)];
    while let Some((prefix, d, depth)) = stack.pop() {
        for e in std::fs::read_dir(&d).map_err(unreadable)? {
            let e = e.map_err(unreadable)?;
            let Ok(name) = e.file_name().into_string() else {
                ignored.push(format!(
                    "{prefix}{}（檔名不是 UTF-8）",
                    e.file_name().to_string_lossy()
                ));
                continue;
            };
            if IGNORED_NAMES.contains(&name.as_str()) || (prefix.is_empty() && name == MARK_FILE) {
                continue;
            }
            let rel = format!("{prefix}{name}");
            let p = e.path();
            let meta = std::fs::symlink_metadata(&p).map_err(unreadable)?;
            let (path, meta) = if meta.file_type().is_symlink() {
                let Ok(target) = std::fs::canonicalize(&p) else {
                    ignored.push(format!("{rel}（壞掉的 symlink）"));
                    continue;
                };
                if !target.starts_with(&root) {
                    ignored.push(format!("{rel}（symlink 指到 skill 資料夾外，不跟）"));
                    continue;
                }
                let m = std::fs::metadata(&target).map_err(unreadable)?;
                if !m.is_file() {
                    ignored.push(format!("{rel}（symlink 指到資料夾，不跟）"));
                    continue;
                }
                (target, m)
            } else {
                (p, meta)
            };
            if meta.is_dir() {
                if depth + 1 >= MAX_DEPTH {
                    ignored.push(format!("{rel}（資料夾太深）"));
                } else {
                    stack.push((format!("{rel}/"), path, depth + 1));
                }
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let bytes = meta.len();
            if bytes > MAX_FILE_BYTES {
                return Err(SkipReason::FileTooLarge { file: rel, bytes });
            }
            total += bytes;
            if total > MAX_SKILL_BYTES {
                return Err(SkipReason::TooLarge { bytes: total });
            }
            files.push(SrcFile { rel, path });
            if files.len() > MAX_FILES {
                return Err(SkipReason::TooManyFiles);
            }
        }
    }
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    ignored.sort();
    Ok((files, ignored))
}

/// 讀內容（掃描之後檔案又變大也擋）。
fn load(src: &Source) -> std::result::Result<BTreeMap<String, Vec<u8>>, SkipReason> {
    let mut out = BTreeMap::new();
    let mut total = 0u64;
    for f in &src.files {
        let b = std::fs::read(&f.path).map_err(|e| SkipReason::Unreadable(e.to_string()))?;
        let n = b.len() as u64;
        if n > MAX_FILE_BYTES {
            return Err(SkipReason::FileTooLarge {
                file: f.rel.clone(),
                bytes: n,
            });
        }
        total += n;
        if total > MAX_SKILL_BYTES {
            return Err(SkipReason::TooLarge { bytes: total });
        }
        out.insert(f.rel.clone(), b);
    }
    Ok(out)
}

/// 內容雜湊（FNV-1a 64；只拿來認「是不是同一份」，不是安全用途）。
fn content_hash(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for (rel, b) in files {
        feed(rel.as_bytes());
        feed(&[0]);
        feed(&(b.len() as u64).to_le_bytes());
        feed(b);
    }
    format!("{h:016x}")
}

// ── 目的地 ──────────────────────────────────────────────────────────────

/// 目的地資料夾的內容（不跟 symlink；symlink 記成一個不可能跟來源相同的值，同步時會被換掉）。
fn read_dest(dir: &Path) -> std::io::Result<BTreeMap<String, Vec<u8>>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![(String::new(), dir.to_path_buf())];
    while let Some((prefix, d)) = stack.pop() {
        for e in std::fs::read_dir(&d)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if prefix.is_empty() && name == MARK_FILE {
                continue;
            }
            let rel = format!("{prefix}{name}");
            let ft = e.file_type()?;
            if ft.is_dir() {
                stack.push((format!("{rel}/"), e.path()));
            } else if ft.is_file() {
                out.insert(rel, std::fs::read(e.path())?);
            } else {
                out.insert(rel, b"\0ctx:not-a-file\0".to_vec());
            }
        }
    }
    Ok(out)
}

/// 標記檔裡的來源 spine（讀不到、不是 JSON＝None）。
fn marker_spine(dir: &Path) -> Option<String> {
    let t = std::fs::read_to_string(dir.join(MARK_FILE)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&t).ok()?;
    v["spine"].as_str().map(str::to_string)
}

fn marker_text(spine: &str, hash: &str) -> String {
    let mut s = serde_json::to_string_pretty(&json!({
        "managed_by": "ctx setup（repo-context 共享 skill；來源改了會覆蓋，這裡的修改不會留下）",
        "spine": spine,
        "hash": hash,
    }))
    .unwrap_or_default();
    s.push('\n');
    s
}

/// 目的地同名路徑的狀態。
enum Dest {
    Missing,
    Ours,
    Foreign(Conflict),
}

fn dest_state(dir: &Path, spine: &str) -> Dest {
    match std::fs::symlink_metadata(dir) {
        Err(_) => Dest::Missing,
        Ok(m) if !m.is_dir() => Dest::Foreign(Conflict::UserOwned),
        Ok(_) => match marker_spine(dir) {
            None => Dest::Foreign(Conflict::UserOwned),
            Some(s) if s == spine => Dest::Ours,
            Some(s) => Dest::Foreign(Conflict::OtherSpine(s)),
        },
    }
}

fn remove_any(p: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn abs(dir: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(dir.to_path_buf(), |p, c| p.join(c))
}

/// 由下往上收掉空資料夾（skill 根目錄不收）。
fn prune_empty(dir: &Path, is_root: bool) -> std::io::Result<bool> {
    let mut empty = true;
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            if !prune_empty(&e.path(), false)? {
                empty = false;
            }
        } else {
            empty = false;
        }
    }
    if empty && !is_root {
        std::fs::remove_dir(dir)?;
    }
    Ok(empty)
}

/// 把目的地換成來源的樣子：標記先寫（中途失敗留下的也認得是我們的），刪多的、寫不同的，最後更新標記的雜湊。
fn write_skill(
    dest: &Path,
    want: &BTreeMap<String, Vec<u8>>,
    have: &BTreeMap<String, Vec<u8>>,
    spine: &str,
    hash: &str,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    if !dest.join(MARK_FILE).exists() {
        crate::fsutil::write_atomic(&dest.join(MARK_FILE), marker_text(spine, "").as_bytes())?;
    }
    for rel in have.keys().filter(|k| !want.contains_key(*k)) {
        remove_any(&abs(dest, rel))?;
    }
    prune_empty(dest, true)?;
    for (rel, bytes) in want {
        if have.get(rel) == Some(bytes) {
            continue;
        }
        let p = abs(dest, rel);
        // 路徑上有同名的檔（來源把檔換成資料夾）或這裡是資料夾（反過來）：先清掉
        let mut cur = dest.to_path_buf();
        for c in rel.split('/') {
            cur = cur.join(c);
            if cur == p {
                if std::fs::symlink_metadata(&cur).is_ok_and(|m| !m.is_file()) {
                    remove_any(&cur)?;
                }
            } else if std::fs::symlink_metadata(&cur).is_ok_and(|m| !m.is_dir()) {
                remove_any(&cur)?;
            }
        }
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::fsutil::write_atomic(&p, bytes)?;
    }
    crate::fsutil::write_atomic(&dest.join(MARK_FILE), marker_text(spine, hash).as_bytes())
}

// ── 對外 ────────────────────────────────────────────────────────────────

/// 把 `<spine>/skills/` 發到 `<cli_home>/skills/`；`write=false` 只回計畫。回報依名稱排序：先來源的
/// （發了／沒變／衝突／略過），再目的地要移除的。沒有 `skills/`、也沒有本 spine 發過的＝空。
pub fn sync(spine_dir: &Path, cli_home: &Path, write: bool) -> Result<Vec<Report>> {
    let spine = crate::fsutil::resolve(spine_dir)
        .to_string_lossy()
        .into_owned();
    let listing = list(spine_dir)?;
    let target_root = cli_home.join(SKILLS_DIR);
    let mut out = listing.skipped;
    for src in &listing.sources {
        let dest = target_root.join(&src.name);
        let report = |outcome| Report {
            name: src.name.clone(),
            outcome,
            ignored: src.ignored.clone(),
        };
        let want = match load(src) {
            Ok(w) => w,
            Err(r) => {
                out.push(report(Outcome::Skipped(r)));
                continue;
            }
        };
        let (outcome, have) = match dest_state(&dest, &spine) {
            Dest::Foreign(c) => {
                out.push(report(Outcome::Conflict(c)));
                continue;
            }
            Dest::Missing => (Outcome::Installed, BTreeMap::new()),
            Dest::Ours => {
                let have = read_dest(&dest)?;
                if have == want {
                    out.push(report(Outcome::Unchanged));
                    continue;
                }
                (Outcome::Updated, have)
            }
        };
        if write {
            write_skill(&dest, &want, &have, &spine, &content_hash(&want))?;
        }
        out.push(report(outcome));
    }
    // spine 裡已經沒有的：目的地帶本 spine 標記的才移除
    if let Ok(rd) = std::fs::read_dir(&target_root) {
        for e in rd.flatten() {
            let Ok(name) = e.file_name().into_string() else {
                continue;
            };
            if listing.present.contains(&name) || !e.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            if marker_spine(&e.path()).as_deref() != Some(spine.as_str()) {
                continue;
            }
            if write {
                std::fs::remove_dir_all(e.path())?;
            }
            out.push(Report {
                name,
                outcome: Outcome::Removed,
                ignored: Vec::new(),
            });
        }
    }
    out.sort_by(|a, b| {
        (matches!(a.outcome, Outcome::Removed), &a.name)
            .cmp(&(matches!(b.outcome, Outcome::Removed), &b.name))
    });
    Ok(out)
}

/// setup 要不要多一步發共享 skill（沒有 `skills/`、目的地也沒有本 spine 發過的＝不用，輸出跟以前一樣）。
/// 讀不了就當要（讓那一步把錯誤講出來）。
pub fn has_work(spine_dir: &Path, cli_home: &Path) -> bool {
    sync(spine_dir, cli_home, false).map_or(true, |r| !r.is_empty())
}

/// 一行摘要：「新增 a、b；更新 c；…」（setup 的計畫與結果都用這個）。
pub fn summary(reports: &[Report]) -> String {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    let mut push = |k: &str, v: String| match groups.iter_mut().find(|(g, _)| g == k) {
        Some((_, list)) => list.push(v),
        None => groups.push((k.to_string(), vec![v])),
    };
    for r in reports {
        match &r.outcome {
            Outcome::Installed => push("新增", r.name.clone()),
            Outcome::Updated => push("更新", r.name.clone()),
            Outcome::Unchanged => push("已是最新", r.name.clone()),
            Outcome::Removed => push("移除（spine 裡已刪）", r.name.clone()),
            Outcome::Conflict(c) => push("衝突", format!("{}（{}）", r.name, c.text())),
            Outcome::Skipped(s) => push("略過", format!("{}（{}）", r.name, s.text())),
        }
    }
    let ignored: Vec<String> = reports
        .iter()
        .flat_map(|r| r.ignored.iter().map(move |i| format!("{}/{i}", r.name)))
        .collect();
    if !ignored.is_empty() {
        push("沒發的檔", ignored.join("、"));
    }
    if groups.is_empty() {
        return "沒有共享 skill".into();
    }
    groups
        .into_iter()
        .map(|(k, v)| format!("{k} {}", v.join("、")))
        .collect::<Vec<_>>()
        .join("；")
}

/// 某家 CLI 的共享 skill 哪些不是最新（`ctx health` 用）：未安裝、過時（含 spine 裡已刪但還留著的）、衝突。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pending {
    pub missing: Vec<String>,
    pub stale: Vec<String>,
    pub conflict: Vec<String>,
}

impl Pending {
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.stale.is_empty() && self.conflict.is_empty()
    }
}

pub fn pending(spine_dir: &Path, cli_home: &Path) -> Result<Pending> {
    let mut p = Pending::default();
    for r in sync(spine_dir, cli_home, false)? {
        match r.outcome {
            Outcome::Installed => p.missing.push(r.name),
            Outcome::Updated | Outcome::Removed => p.stale.push(r.name),
            Outcome::Conflict(_) => p.conflict.push(r.name),
            Outcome::Unchanged | Outcome::Skipped(_) => {}
        }
    }
    Ok(p)
}

/// spine 裡會發的 skill：(名稱, SKILL.md frontmatter 的 description)。給 spine 的 AGENTS.md 用——
/// 只看 metadata 與 SKILL.md，不讀其他檔（registry 每次寫入都會走到）。
pub fn listed(spine_dir: &Path) -> Vec<(String, Option<String>)> {
    let Ok(listing) = list(spine_dir) else {
        return Vec::new();
    };
    listing
        .sources
        .iter()
        .map(|s| {
            let desc = std::fs::read_to_string(s.dir.join(SKILL_FILE))
                .ok()
                .and_then(|t| description_of(&t));
            (s.name.clone(), desc)
        })
        .collect()
}

/// SKILL.md 開頭 `---…---` 裡的 `description`。
pub fn description_of(text: &str) -> Option<String> {
    let t = text.trim_start_matches('\u{feff}');
    let mut lines = t.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let fm: Vec<&str> = lines.take_while(|l| l.trim_end() != "---").collect();
    let v: serde_yaml::Value = serde_yaml::from_str(&fm.join("\n")).ok()?;
    let d = v.get("description")?.as_str()?.trim().to_string();
    (!d.is_empty()).then_some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(name_ok("pdf-tools"));
        assert!(name_ok("a1"));
        for bad in ["", "-a", "a-", "Pdf", "a_b", "a b", "中文"] {
            assert!(!name_ok(bad), "{bad}");
        }
    }

    #[test]
    fn frontmatter_description() {
        assert_eq!(
            description_of("---\nname: x\ndescription: 做 PDF\n---\n# x\n").as_deref(),
            Some("做 PDF")
        );
        assert_eq!(
            description_of("---\ndescription: >\n  多行\n  說明\n---\n").as_deref(),
            Some("多行 說明")
        );
        assert_eq!(description_of("# 沒有 frontmatter\n"), None);
    }
}
