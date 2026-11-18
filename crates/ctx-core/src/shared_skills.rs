//! 工作台 skill（F-133，2026-11-18，Rust 才有）：教 agent 用本工具（搜尋、拿別的 repo 的 context…）的 skill
//! 放在 spine 用 git 版控（`<spine>/skills/<名稱>/SKILL.md`，資料夾裡可以有其他檔與子資料夾）。
//!
//! **只在納管的 repo 裡用得到，任何 repo 與使用者層都不放檔**（使用者 2026-11-18 定）：
//! - 第 1 層（名稱＋一行說明）：開場 hook 在納管 repo 的會話注入 [`opening`]；不在納管 repo＝hook 不輸出，看不到。
//! - 第 2、3 層（SKILL.md 全文、附檔）：agent 要用時呼叫 MCP `skill_get`／`ctx skill <名稱>`（[`read`]），
//!   同樣只在納管 repo 回應（範圍檢查在 [`crate::agentapi::skill_get`]）。
//! - repo 移出 registry：下一場會話起就沒有清單、讀不到——沒有東西要清。
//! - 2026-11-18 早上那版曾發到 `<cli home>/skills/`；[`retire`] 收回帶本 spine 標記（[`MARK_FILE`]）的副本。
//!
//! 不跟隨指到 skill 資料夾外面的 symlink（也不跟指到資料夾的 symlink）；單檔超過 [`MAX_FILE_BYTES`]、
//! 整個 skill 超過 [`MAX_SKILL_BYTES`] 就整個略過。內建的 `repo-context`（[`crate::agentsetup::MCP_NAME`]）名稱保留。

use std::path::{Path, PathBuf};

use serde::Serialize;

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
    skipped: Vec<Skipped>,
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
        let skip = |reason| Skipped {
            name: name.clone(),
            reason,
        };
        if ft.is_symlink() {
            // 指到資料夾的 symlink 才算一個 skill（指到檔的跟 README 一樣忽略）
            if e.path().is_dir() {
                out.skipped.push(skip(SkipReason::Symlink));
            }
            continue;
        }
        if !ft.is_dir() {
            continue; // skills/README.md 之類
        }
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
                    out.skipped.push(skip(SkipReason::NoSkillMd));
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

/// spine 裡沒拿出來用的 skill 與原因（`ctx health` 列出）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// 資料夾名（不合法的名稱也照原樣）。
    pub name: String,
    pub reason: SkipReason,
}

/// 一個可用的工作台 skill（清單那一層）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub name: String,
    /// SKILL.md frontmatter 的 description。
    pub description: Option<String>,
    /// skill 資料夾裡的檔（相對路徑，`/` 分隔；含 SKILL.md）。
    pub files: Vec<String>,
    /// 沒拿出來的檔（symlink 指到外面、指到資料夾、壞掉的連結…）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignored: Vec<String>,
}

/// `<spine>/skills/` 的清單。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    pub skills: Vec<Entry>,
    pub skipped: Vec<Skipped>,
}

/// 掃 `<spine>/skills/`：可用的（只讀 SKILL.md 取說明）＋略過的。沒有 `skills/`＝空。
pub fn index(spine_dir: &Path) -> Result<Index> {
    let listing = list(spine_dir)?;
    let skills = listing
        .sources
        .iter()
        .map(|s| Entry {
            name: s.name.clone(),
            description: std::fs::read_to_string(s.dir.join(SKILL_FILE))
                .ok()
                .and_then(|t| description_of(&t)),
            files: s.files.iter().map(|f| f.rel.clone()).collect(),
            ignored: s.ignored.clone(),
        })
        .collect();
    Ok(Index {
        skills,
        skipped: listing.skipped,
    })
}

/// (名稱, description)：給 spine 的 AGENTS.md 用（registry 每次寫入都會走到；讀不了＝空）。
pub fn listed(spine_dir: &Path) -> Vec<(String, Option<String>)> {
    index(spine_dir)
        .map(|i| {
            i.skills
                .into_iter()
                .map(|e| (e.name, e.description))
                .collect()
        })
        .unwrap_or_default()
}

/// 開場清單最多列幾個 skill（其餘用 `skill_get` 不帶名稱看）。
pub const OPENING_MAX: usize = 12;
/// 開場清單每個說明最多幾個字。
pub const OPENING_DESC_CHARS: usize = 100;

/// 開場注入的那一段（漸進揭露第 1 層）；spine 沒有可用的 skill＝None（開場跟以前一字不差）。
pub fn opening(spine_dir: &Path) -> Option<String> {
    let idx = index(spine_dir).ok()?;
    if idx.skills.is_empty() {
        return None;
    }
    let mut lines = vec![
        "## 工作台 skill（教你用 repo-context 做事；要用時 skill_get 讀全文，沒掛 MCP 用 `ctx skill <名稱>`）"
            .to_string(),
    ];
    for e in idx.skills.iter().take(OPENING_MAX) {
        let desc = e.description.as_deref().unwrap_or("（沒寫說明）");
        lines.push(format!("- {}：{}", e.name, clip(desc, OPENING_DESC_CHARS)));
    }
    if idx.skills.len() > OPENING_MAX {
        lines.push(format!(
            "- …還有 {} 個：skill_get 不帶名稱看全部",
            idx.skills.len() - OPENING_MAX
        ));
    }
    Some(lines.join("\n"))
}

fn clip(s: &str, n: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= n {
        s
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

/// 讀到的一個 skill 檔（漸進揭露第 2、3 層）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Doc {
    pub name: String,
    /// 讀的是哪個檔（相對 skill 資料夾）。
    pub file: String,
    pub text: String,
    /// 這個 skill 的所有檔（要附檔就再帶 file 讀）。
    pub files: Vec<String>,
    /// skill 資料夾的絕對路徑（SKILL.md 叫你跑裡面的腳本時用）。
    pub dir: String,
}

fn not_found(msg: String, hint: &str) -> crate::Error {
    crate::Error::rejected(crate::RejectCode::NotFound, msg, Some(hint.to_string()))
}

/// 讀 `<spine>/skills/<name>/<file>`（file 省略＝SKILL.md）。只讀清單裡的檔：不跟 symlink 出去、不讀 `..`。
pub fn read(spine_dir: &Path, name: &str, file: Option<&str>) -> Result<Doc> {
    let listing = list(spine_dir)?;
    let name = name.trim();
    let Some(src) = listing.sources.iter().find(|s| s.name == name) else {
        if let Some(sk) = listing.skipped.iter().find(|s| s.name == name) {
            return Err(not_found(
                format!("工作台 skill {name} 沒有啟用：{}", sk.reason.text()),
                "請使用者修正 spine/skills/ 裡的那個資料夾；ctx health 也會列出。",
            ));
        }
        return Err(not_found(
            format!("沒有這個工作台 skill：{name}"),
            "skill_get 不帶名稱看有哪些。",
        ));
    };
    let rel = file
        .map(|f| f.trim().replace('\\', "/"))
        .map(|f| f.trim_start_matches("./").to_string())
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| SKILL_FILE.to_string());
    let files: Vec<String> = src.files.iter().map(|f| f.rel.clone()).collect();
    let Some(f) = src.files.iter().find(|f| f.rel == rel) else {
        return Err(not_found(
            format!("{name} 裡沒有 {rel}"),
            &format!("有的檔：{}", files.join("、")),
        ));
    };
    let bytes = std::fs::read(&f.path)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(not_found(
            format!("{name}/{rel} 超過單檔上限 {}", human(MAX_FILE_BYTES)),
            "請使用者把它拆小。",
        ));
    }
    let dir = crate::fsutil::resolve(&src.dir)
        .to_string_lossy()
        .into_owned();
    let Ok(text) = String::from_utf8(bytes) else {
        return Err(not_found(
            format!("{name}/{rel} 不是文字檔"),
            &format!("要執行就用路徑 {dir}/{rel}"),
        ));
    };
    Ok(Doc {
        name: src.name.clone(),
        file: rel,
        text,
        files,
        dir,
    })
}

/// 收回 2026-11-18 早上那版發到 `<cli_home>/skills/` 的副本：帶本 spine 標記的資料夾才收
/// （使用者自己的、別的 spine 的不動）。`write=false` 只回會收哪些。依名稱排序。
pub fn retire(spine_dir: &Path, cli_home: &Path, write: bool) -> Result<Vec<String>> {
    let spine = crate::fsutil::resolve(spine_dir)
        .to_string_lossy()
        .into_owned();
    let Ok(rd) = std::fs::read_dir(cli_home.join(SKILLS_DIR)) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        let is_dir = std::fs::symlink_metadata(e.path()).is_ok_and(|m| m.is_dir());
        if !is_dir || marker_spine(&e.path()).as_deref() != Some(spine.as_str()) {
            continue;
        }
        if write {
            std::fs::remove_dir_all(e.path())?;
        }
        out.push(name);
    }
    out.sort();
    Ok(out)
}

/// 標記檔裡的來源 spine（讀不到、不是 JSON＝None）。
fn marker_spine(dir: &Path) -> Option<String> {
    let t = std::fs::read_to_string(dir.join(MARK_FILE)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&t).ok()?;
    v["spine"].as_str().map(str::to_string)
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
