//! Repo 檔案清單（2026-10-11；Rust 才有，Python 沒有對應模組）。
//!
//! App 的 repo 詳情像 IDE 側邊欄一樣逐層展開，每一列可以複製絕對或相對路徑：
//! - 一次只列一層（展開資料夾才讀下一層），大 repo 也不會一次讀完。
//! - 不列 `.git`；git repo 裡被 .gitignore 忽略的不列（交給 `git check-ignore`，規則跟 git 一致），
//!   不是 git 的資料夾全列。
//! - 只讀，不改任何檔；`dir` 必須在 repo 裡（擋 `..`、絕對路徑、符號連結跳出 repo）。
//!
//! [`read_file`]：App 的檢視器讀一個檔——文字（UTF-8，壞位元組換成 �）、圖片（base64 給 `<img>`）、
//! 其他二進位只回大小。文字超過 [`TEXT_MAX`] 只給前段並標 `truncated`。
//!
//! [`write_file`]（2026-10-13）：檢視器的簡單編輯。只改既有的文字檔（不新增、刪除、改名）；
//! 帶讀檔時的 `version`，磁碟上已經被改過（agent 寫得很快）就不寫、回 [`WriteOutcome::Conflict`]；
//! 先寫暫存檔再改名；保留原檔的 CRLF 與 BOM（textarea 會把 CRLF 變 LF）。
//!
//! [`create_entry`]（2026-10-15）：檢視器／檔案樹新增空檔或資料夾。只新增、不覆蓋（已存在就拒絕）；
//! 上層資料夾沒有就一路建（但最近一個已存在的上層必須在 repo 裡）；不准建在 `.git` 裡。
//! [`rename_entry`]（同日）：改名或搬移，新路徑同一套規則；不刪任何東西。
//!
//! [`changes`]／[`diff`]（2026-10-14）：還沒 commit 的變更與單檔差異（看 agent 改了什麼）。只讀，交給 git。
//!
//! [`find_files`]（2026-10-14）：⌘K 用檔名找檔。git repo 用 `git ls-files`（追蹤中＋沒被忽略的未追蹤檔，
//! 規則跟 git 一致）；不是 git 的往下走（略過隱藏資料夾、node_modules、target），每個 repo 最多看 [`FIND_SCAN_MAX`] 個。

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use crate::{Error, Result};

/// 一層最多列幾項；超過的只回數量（`more`）。
pub const MAX_ENTRIES: usize = 2000;

/// 一列：檔案或資料夾。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// 相對 repo 根目錄（用系統的路徑分隔符）。
    pub rel: PathBuf,
    pub abs: PathBuf,
    /// 符號連結一律當檔案（不跟進去，免得跳出 repo）。
    pub dir: bool,
}

/// 一層的內容：資料夾在前、再依名稱（不分大小寫）排序。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// 超過 [`MAX_ENTRIES`] 沒列出的項數。
    pub more: usize,
}

/// `rel`（相對路徑，空字串＝根目錄）只能往下走。`noun` 只用在錯誤訊息（資料夾／檔案）。
fn checked_rel(rel: &str, noun: &str) -> Result<PathBuf> {
    let p = PathBuf::from(rel.trim());
    for c in p.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(Error::invalid(format!("{noun}必須在 repo 裡：{rel}"))),
        }
    }
    Ok(p)
}

/// `root/rel` 的實體路徑，必須存在且在 repo 裡（符號連結跳出去也擋）。回 (相對路徑, 實體 root, 實體目標)。
fn resolve_in(root: &Path, rel: &str, noun: &str) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let rel_p = checked_rel(rel, noun)?;
    let root_real = root
        .canonicalize()
        .map_err(|_| Error::invalid(format!("repo 路徑不存在：{}", root.display())))?;
    let target_real = root_real
        .join(&rel_p)
        .canonicalize()
        .map_err(|_| Error::invalid(format!("{noun}不存在：{rel}")))?;
    if !target_real.starts_with(&root_real) {
        return Err(Error::invalid(format!("{noun}必須在 repo 裡：{rel}")));
    }
    Ok((rel_p, root_real, target_real))
}

/// `root/rel` 存在且在 repo 裡（符號連結跳出去也擋）就回 `root/rel`（不展開符號連結，跟檔案樹的 `abs` 同一種寫法）。
/// 給「挑幾份檔案交給 agent」驗證路徑用。
pub fn locate(root: &Path, rel: &str) -> Result<PathBuf> {
    let (rel_p, _, _) = resolve_in(root, rel, "檔案")?;
    Ok(root.join(rel_p))
}

/// git repo 裡被忽略的那些（`rels` 是相對 root 的路徑）。不是 git、或 git 不能用＝都不忽略。
fn ignored(root: &Path, rels: &[PathBuf]) -> Vec<bool> {
    let mut out = vec![false; rels.len()];
    if rels.is_empty() || !root.join(".git").exists() {
        return out;
    }
    let Ok(mut child) = Command::new("git")
        .args(["check-ignore", "--stdin", "-z"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return out;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let mut buf = Vec::new();
        for r in rels {
            buf.extend_from_slice(r.to_string_lossy().as_bytes());
            buf.push(0);
        }
        let _ = stdin.write_all(&buf);
    }
    // 有忽略的 exit 0、全部沒忽略 exit 1；兩種都照 stdout 判斷
    let Ok(res) = child.wait_with_output() else {
        return out;
    };
    let hits: Vec<&[u8]> = res
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .collect();
    for (i, r) in rels.iter().enumerate() {
        let key = r.to_string_lossy();
        out[i] = hits.iter().any(|h| *h == key.as_bytes());
    }
    out
}

/// 列出 `root/dir` 這一層。
pub fn list_dir(root: &Path, dir: &str) -> Result<Listing> {
    let (rel_dir, root_real, target_real) = resolve_in(root, dir, "資料夾")?;
    if !target_real.is_dir() {
        return Err(Error::invalid(format!("不是資料夾：{dir}")));
    }

    let mut entries: Vec<Entry> = std::fs::read_dir(&target_real)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                return None;
            }
            let dir = e.file_type().ok().is_some_and(|t| t.is_dir());
            Some(Entry {
                rel: rel_dir.join(&name),
                abs: root.join(&rel_dir).join(&name),
                name,
                dir,
            })
        })
        .collect();
    let rels: Vec<PathBuf> = entries.iter().map(|e| e.rel.clone()).collect();
    let skip = ignored(&root_real, &rels);
    let mut i = 0;
    entries.retain(|_| {
        i += 1;
        !skip[i - 1]
    });
    entries.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    let more = entries.len().saturating_sub(MAX_ENTRIES);
    entries.truncate(MAX_ENTRIES);
    Ok(Listing { entries, more })
}

/// 文字檔最多讀幾個位元組（超過只給前段）。
pub const TEXT_MAX: u64 = 1024 * 1024;
/// 圖片最大幾個位元組（超過不載入）。
pub const IMAGE_MAX: u64 = 8 * 1024 * 1024;

/// 檢視器讀到的內容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileView {
    Text {
        text: String,
        truncated: bool,
        /// 能不能在 App 裡編輯：完整讀到（沒截斷）而且是合法 UTF-8（壞位元組存回去會變 �）。
        editable: bool,
        /// 內容版本（大小＋雜湊）；[`write_file`] 用它判斷讀檔之後有沒有被別人改過。
        version: String,
    },
    Image {
        mime: &'static str,
        base64: String,
    },
    /// 不是文字也不是認得的圖片。
    Binary,
    /// 圖片太大，不載入。
    TooLarge,
}

/// 讀到的檔：`size` 是整個檔的位元組數。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRead {
    pub rel: PathBuf,
    pub abs: PathBuf,
    pub size: u64,
    pub view: FileView,
}

fn image_mime(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        _ => return None,
    })
}

/// 標準 base64（有 `=` 補齊）。只給圖片預覽用，不值得為它加依賴。
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= c.len() {
                T[((n >> shift) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

/// 內容版本：只在同一個執行檔裡比對（讀檔和存檔都經過引擎），不寫進任何檔案。
fn version_of(bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{}-{:016x}", bytes.len(), h.finish())
}

/// 讀 `root/rel` 一個檔。
pub fn read_file(root: &Path, rel: &str) -> Result<FileRead> {
    use std::io::Read;
    let (rel_p, _root_real, target) = resolve_in(root, rel, "檔案")?;
    if !target.is_file() {
        return Err(Error::invalid(format!("不是檔案：{rel}")));
    }
    let size = std::fs::metadata(&target)?.len();
    let abs = root.join(&rel_p);
    let view = if let Some(mime) = image_mime(&target) {
        if size > IMAGE_MAX {
            FileView::TooLarge
        } else {
            FileView::Image {
                mime,
                base64: base64(&std::fs::read(&target)?),
            }
        }
    } else {
        let mut buf = Vec::new();
        std::fs::File::open(&target)?
            .take(TEXT_MAX)
            .read_to_end(&mut buf)?;
        // 前 8KB 有 NUL＝二進位（git 也是這樣判斷）
        if buf.iter().take(8000).any(|b| *b == 0) {
            FileView::Binary
        } else {
            let truncated = size > TEXT_MAX;
            let version = version_of(&buf);
            let (text, valid) = match String::from_utf8(buf) {
                Ok(t) => (t, true),
                Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), false),
            };
            FileView::Text {
                text,
                truncated,
                editable: valid && !truncated,
                version,
            }
        }
    };
    Ok(FileRead {
        rel: rel_p,
        abs,
        size,
        view,
    })
}

/// [`write_file`] 的結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// 寫好了；附上寫完重讀的內容（含新 `version`）。
    Saved(FileRead),
    /// 讀檔之後磁碟上的檔已經變了，沒有寫。
    Conflict,
}

/// 把 `text` 寫回 `root/rel`（既有的文字檔）。`base`＝讀檔時的 `version`。
pub fn write_file(root: &Path, rel: &str, text: &str, base: &str) -> Result<WriteOutcome> {
    let (_rel_p, _root_real, target) = resolve_in(root, rel, "檔案")?;
    if !target.is_file() {
        return Err(Error::invalid(format!("不是檔案：{rel}")));
    }
    if image_mime(&target).is_some() {
        return Err(Error::invalid(format!("圖片不能在 App 裡編輯：{rel}")));
    }
    let meta = std::fs::metadata(&target)?;
    if meta.len() > TEXT_MAX {
        return Err(Error::invalid(format!(
            "檔案超過 1 MB，不能在 App 裡編輯：{rel}"
        )));
    }
    let cur = std::fs::read(&target)?;
    if cur.iter().take(8000).any(|b| *b == 0) || std::str::from_utf8(&cur).is_err() {
        return Err(Error::invalid(format!(
            "不是 UTF-8 文字檔，不能在 App 裡編輯：{rel}"
        )));
    }
    if version_of(&cur) != base {
        return Ok(WriteOutcome::Conflict);
    }

    const BOM: &str = "\u{feff}";
    let crlf = cur.windows(2).any(|w| w == b"\r\n");
    let mut out = text.strip_prefix(BOM).unwrap_or(text).replace("\r\n", "\n");
    if crlf {
        out = out.replace('\n', "\r\n");
    }
    if cur.starts_with(BOM.as_bytes()) {
        out.insert_str(0, BOM);
    }

    // 寫到符號連結指向的實體檔（target 已 canonicalize），連結本身不動；暫存檔與權限見 write_atomic
    crate::fsutil::write_atomic(&target, out.as_bytes())?;
    Ok(WriteOutcome::Saved(read_file(root, rel)?))
}

/// [`create_entry`] 要建的是什麼。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NewEntry {
    File,
    Dir,
}

/// 新路徑的檢查（[`create_entry`]／[`rename_entry`] 共用）：不能是根目錄、不能跳出 repo、不能碰 `.git`、
/// 不能已存在；最近一個已存在的上層要是資料夾且在 repo 裡（符號連結跳出去也擋）。
/// 通過就把缺的上層資料夾建好，回（實體目標路徑, `root/rel` 寫法）。
fn new_target(root: &Path, rel: &str) -> Result<(PathBuf, PathBuf)> {
    let rel_p = checked_rel(rel, "路徑")?;
    let parts: Vec<&std::ffi::OsStr> = rel_p
        .components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        return Err(Error::invalid("請給檔名或資料夾名稱"));
    }
    if parts.iter().any(|n| *n == ".git") {
        return Err(Error::invalid(format!("不能建在 .git 裡：{rel}")));
    }
    let mut have = parts.len() - 1;
    let root_real = root
        .canonicalize()
        .map_err(|_| Error::invalid(format!("repo 路徑不存在：{}", root.display())))?;
    while have > 0
        && !root_real
            .join(parts[..have].iter().collect::<PathBuf>())
            .exists()
    {
        have -= 1;
    }
    let base: PathBuf = parts[..have].iter().collect();
    let (_, _, base_real) = resolve_in(root, &base.to_string_lossy(), "資料夾")?;
    if !base_real.is_dir() {
        return Err(Error::invalid(format!(
            "不是資料夾：{}",
            base.to_string_lossy()
        )));
    }
    let target = base_real.join(parts[have..].iter().collect::<PathBuf>());
    if target.symlink_metadata().is_ok() {
        return Err(Error::invalid(format!("已經有同名的檔案或資料夾：{rel}")));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok((target, root.join(parts.iter().collect::<PathBuf>())))
}

/// 在 repo 裡新增空檔或資料夾，回 `root/rel`（跟檔案樹的 `abs` 同一種寫法）。
/// `rel` 不能是根目錄、不能跳出 repo、不能碰 `.git`；已存在（檔或資料夾都算）就拒絕，不覆蓋。
pub fn create_entry(root: &Path, rel: &str, kind: NewEntry) -> Result<PathBuf> {
    let (target, abs) = new_target(root, rel)?;
    match kind {
        NewEntry::Dir => std::fs::create_dir(&target)?,
        NewEntry::File => {
            // create_new：剛好有人同時建了同名檔也不會蓋掉
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
        }
    }
    Ok(abs)
}

/// 存一張新圖片（2026-10-15，F-64：md 編輯區貼上截圖）。`rel` 要是圖片副檔名（svg 不收：可以帶腳本）、
/// 內容不超過 [`IMAGE_MAX`]；路徑規則同 [`create_entry`]（不覆蓋、上層一路建）。回 `root/rel`。
pub fn save_image(root: &Path, rel: &str, b64: &str) -> Result<PathBuf> {
    let mime = image_mime(Path::new(rel));
    if mime.is_none() || mime == Some("image/svg+xml") {
        return Err(Error::invalid(format!(
            "只收 png／jpg／gif／webp／bmp 圖片：{rel}"
        )));
    }
    let bytes = unbase64(b64).ok_or_else(|| Error::invalid("圖片資料壞了（不是 base64）"))?;
    if bytes.is_empty() || bytes.len() as u64 > IMAGE_MAX {
        return Err(Error::invalid("圖片是空的或超過 8 MB"));
    }
    let (target, abs) = new_target(root, rel)?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)?;
    f.write_all(&bytes)?;
    Ok(abs)
}

/// [`base64`] 的反向（忽略空白；壞字元回 None）。
fn unbase64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let body = clean
        .strip_suffix(b"==")
        .or_else(|| clean.strip_suffix(b"="))
        .unwrap_or(&clean);
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    for chunk in body.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= val(*c)? << (18 - 6 * i);
        }
        let take = match chunk.len() {
            4 => 3,
            3 => 2,
            2 => 1,
            _ => return None,
        };
        for i in 0..take {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

/// 改名或搬移（2026-10-15，F-66）：`from` 是 repo 裡已存在的檔或資料夾（符號連結改的是連結本身），
/// `to` 照 [`create_entry`] 的規則（不覆蓋、上層一路建、不碰 .git）。回 (`root/to`, 是不是資料夾)。
pub fn rename_entry(root: &Path, from: &str, to: &str) -> Result<(PathBuf, bool)> {
    let from_p = checked_rel(from, "路徑")?;
    let name = from_p
        .file_name()
        .ok_or_else(|| Error::invalid("不能改 repo 根目錄的名字"))?;
    if from_p.components().any(|c| c.as_os_str() == ".git") {
        return Err(Error::invalid(format!("不能改 .git 裡的東西：{from}")));
    }
    let parent = from_p.parent().unwrap_or(Path::new(""));
    let (_, _, parent_real) = resolve_in(root, &parent.to_string_lossy(), "資料夾")?;
    let src = parent_real.join(name);
    let meta = src
        .symlink_metadata()
        .map_err(|_| Error::invalid(format!("檔案不存在：{from}")))?;
    // 資料夾不能搬進自己底下（先擋，免得 new_target 已經建了空的上層資料夾才失敗）
    let to_p = checked_rel(to, "路徑")?;
    if meta.is_dir() && to_p.starts_with(&from_p) {
        return Err(Error::invalid(format!(
            "不能把資料夾搬進自己底下：{from} → {to}"
        )));
    }
    let (target, abs) = new_target(root, to)?;
    std::fs::rename(&src, &target)?;
    Ok((abs, meta.is_dir()))
}

/// 變更清單最多列幾個檔（其餘只給數量）。
pub const CHANGES_MAX: usize = 500;
/// 單檔差異最多給多少位元組（再多只給前段，標 `truncated`）。
pub const DIFF_MAX: usize = 512 * 1024;
/// git status／diff 最多等多久。
const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// 一個還沒 commit 的變更。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// 相對 repo 根目錄（`/` 分隔，git 給的樣子）。
    pub rel: String,
    pub kind: ChangeKind,
}

/// 變更的種類（看 git status 的兩欄：暫存區與工作區，有一邊變就算）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

impl ChangeKind {
    fn from_xy(x: char, y: char) -> ChangeKind {
        match (x, y) {
            ('?', '?') => ChangeKind::Untracked,
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => ChangeKind::Conflicted,
            ('R', _) | ('C', _) => ChangeKind::Renamed,
            ('A', _) => ChangeKind::Added,
            ('D', _) | (_, 'D') => ChangeKind::Deleted,
            _ => ChangeKind::Modified,
        }
    }
}

/// 還沒 commit 的變更（`git status --porcelain=v1 -z --untracked-files=all`，git 的忽略規則照算）。
/// 回 (變更, 超過 [`CHANGES_MAX`] 沒列的數量)。不是 git repo、或 git 跑不起來＝錯誤。
pub fn changes(root: &Path) -> Result<(Vec<Change>, usize)> {
    if !root.join(".git").exists() {
        return Err(Error::invalid(format!("不是 git repo：{}", root.display())));
    }
    let (code, out) = crate::crossref::git(
        root,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
        ],
        GIT_TIMEOUT,
    );
    if code != 0 {
        return Err(Error::invalid(format!(
            "git status 失敗：{}",
            root.display()
        )));
    }
    let mut all = Vec::new();
    let mut it = out.split('\0').filter(|s| !s.is_empty());
    while let Some(rec) = it.next() {
        let mut cs = rec.chars();
        let (Some(x), Some(y)) = (cs.next(), cs.next()) else {
            continue;
        };
        let rel = rec.get(3..).unwrap_or("").to_string();
        if x == 'R' || x == 'C' {
            it.next(); // 改名：下一筆是原本的路徑
        }
        all.push(Change {
            rel,
            kind: ChangeKind::from_xy(x, y),
        });
    }
    let more = all.len().saturating_sub(CHANGES_MAX);
    all.truncate(CHANGES_MAX);
    Ok((all, more))
}

/// 單檔的差異：工作區對上一個 commit（`git diff HEAD`）；還沒追蹤的檔整份算新增；repo 還沒有任何 commit
/// 就對暫存區。回 (差異文字, 有沒有截斷)。沒有差異＝空字串。`rel` 只能往下走（檔案可能已刪，所以不要求存在）。
pub fn diff(root: &Path, rel: &str) -> Result<(String, bool)> {
    let rel_p = checked_rel(rel, "檔案")?;
    if rel_p.as_os_str().is_empty() {
        return Err(Error::invalid("要給一個檔案"));
    }
    if !root.join(".git").exists() {
        return Err(Error::invalid(format!("不是 git repo：{}", root.display())));
    }
    let rel_s = rel_p.to_string_lossy().replace('\\', "/");
    let git = |args: &[&str]| crate::crossref::git(root, args, GIT_TIMEOUT);
    let base = ["diff", "--no-color", "--no-ext-diff"];
    let (code, mut text) = git(&[&base[..], &["HEAD", "--", &rel_s]].concat());
    if code != 0 {
        // 還沒有任何 commit：對暫存區
        let (_, t) = git(&[&base[..], &["--", &rel_s]].concat());
        text = t;
    }
    if text.is_empty() {
        let (_, tracked) = git(&["ls-files", "--", &rel_s]);
        if tracked.is_empty() && root.join(&rel_p).is_file() {
            // 還沒追蹤：跟空檔比（--no-index 有差異時 exit 1，照樣拿 stdout）
            let (_, t) = git(&[&base[..], &["--no-index", "--", "/dev/null", &rel_s]].concat());
            text = t;
        }
    }
    let truncated = text.len() > DIFF_MAX;
    if truncated {
        let mut cut = DIFF_MAX;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    Ok((text, truncated))
}

/// ⌘K 找檔時每個 repo 最多看幾個路徑（超大 repo 也不會卡住）。
pub const FIND_SCAN_MAX: usize = 50_000;
/// `git ls-files` 最多等多久。
const FIND_GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
/// 不是 git 的資料夾往下走時跳過的資料夾（隱藏資料夾另外跳過）。
const WALK_SKIP: [&str; 3] = ["node_modules", "target", "__pycache__"];

/// repo 裡的檔案（相對路徑，`/` 分隔）。git：`ls-files` 追蹤中＋沒被忽略的未追蹤檔；不是 git 或 git 失敗：往下走。
pub fn all_files(root: &Path) -> Vec<String> {
    if root.join(".git").exists() {
        let (code, out) = crate::crossref::git(
            root,
            &[
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
            ],
            FIND_GIT_TIMEOUT,
        );
        if code == 0 {
            let mut v: Vec<String> = out
                .split('\0')
                .filter(|s| !s.is_empty())
                .take(FIND_SCAN_MAX)
                .map(str::to_string)
                .collect();
            v.dedup();
            return v;
        }
    }
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(root.join(&dir)) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let rel = dir.join(&name);
            match e.file_type() {
                Ok(t) if t.is_dir() => {
                    if !name.starts_with('.') && !WALK_SKIP.contains(&name.as_str()) {
                        stack.push(rel);
                    }
                }
                Ok(t) if t.is_file() => {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                    if out.len() >= FIND_SCAN_MAX {
                        return out;
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// 一個找到的檔。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHit {
    /// 相對 repo 根目錄（`/` 分隔）。
    pub rel: String,
    pub name: String,
    pub score: u32,
}

/// `name` 裡有沒有一段 `q` 從「詞的開頭」開始（開頭、非英數字元之後、camelCase 的大寫、數字與字母交界）。
/// `00_product_vision.md`、`productVision.md` 算；`revision.md`、`provisioning.png` 不算。
fn word_start_match(orig_name: &str, q: &str) -> bool {
    let chars: Vec<char> = orig_name.chars().collect();
    let qn = q.chars().count();
    (0..chars.len()).any(|i| {
        let starts = i == 0 || {
            let (p, c) = (chars[i - 1], chars[i]);
            !p.is_alphanumeric()
                || (p.is_lowercase() && c.is_uppercase())
                || (p.is_ascii_digit() != c.is_ascii_digit())
        };
        starts
            && i + qn <= chars.len()
            && chars[i..i + qn].iter().collect::<String>().to_lowercase() == q
    })
}

/// 檔名／路徑的吻合程度（不分大小寫）：檔名相同＞檔名開頭＞檔名裡某個詞的開頭＞檔名中間＞路徑包含＞
/// 每個詞都在路徑裡（`docs vision` 找 `docs/VISION.md`）。沒中＝None。
pub fn file_score(rel: &str, q: &str) -> Option<u32> {
    let path = rel.to_lowercase();
    let name = path.rsplit('/').next().unwrap_or(&path);
    let orig_name = rel.rsplit('/').next().unwrap_or(rel);
    let stem = name.split('.').next().unwrap_or(name);
    if name == q || stem == q {
        Some(100)
    } else if name.starts_with(q) {
        Some(60)
    } else if word_start_match(orig_name, q) {
        Some(50)
    } else if name.contains(q) {
        Some(40)
    } else if path.contains(q) {
        Some(20)
    } else {
        let words: Vec<&str> = q.split_whitespace().collect();
        (words.len() > 1 && words.iter().all(|w| path.contains(w))).then_some(15)
    }
}

/// 在 repo 裡用檔名找檔：分數高的在前，同分路徑短的在前。`query` 空白＝不找。
pub fn find_files(root: &Path, query: &str, limit: usize) -> Vec<FileHit> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<FileHit> = all_files(root)
        .into_iter()
        .filter_map(|rel| {
            let score = file_score(&rel, &q)?;
            let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
            Some(FileHit { rel, name, score })
        })
        .collect();
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.rel.len().cmp(&b.rel.len())));
    hits.truncate(limit);
    hits
}

/// 反向連結（2026-10-15，F-69）：一則「誰連到這份」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlink {
    /// 連過來的那份 md（相對 repo 根目錄，`/` 分隔）。
    pub rel: String,
    /// 第幾行（1 起算）。
    pub line: usize,
    /// 那一行（去頭尾空白，最多 [`BACKLINK_TEXT`] 個字）。
    pub text: String,
}

/// 反向連結最多列幾則（其餘只給數量）。
pub const BACKLINKS_MAX: usize = 50;
/// 最多看幾份 md、每份最多讀多大（大 repo 也不會卡）。
const BACKLINK_SCAN: usize = 3000;
const BACKLINK_TEXT: usize = 120;

/// md 連結的目標：`[字](目標)` 與 `![字](目標)`（可帶 `<…>`、標題）。
static MD_LINK: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#"\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)"#).expect("固定 regex")
});

/// md 裡的相對連結 → repo 內路徑（同前端 `resolveRel`：`/` 開頭＝repo 根、去掉 #錨點與 ?查詢、解 %20；跳出 repo＝None）。
fn resolve_link(from: &str, href: &str) -> Option<String> {
    if href.contains("://") || href.starts_with("mailto:") || href.starts_with('#') {
        return None;
    }
    let clean = href
        .split(['#', '?'])
        .next()
        .unwrap_or("")
        .replace("%20", " ");
    if clean.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = if clean.starts_with('/') {
        Vec::new()
    } else {
        from.split('/').collect::<Vec<_>>()[..from.split('/').count() - 1].to_vec()
    };
    for seg in clean.trim_start_matches('/').split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// 列出 repo 裡連到 `target`（相對路徑）的 md：照相對連結解析、跳過程式碼區塊與自己。回 (前 [`BACKLINKS_MAX`] 則, 其餘數量)。
pub fn backlinks(root: &Path, target: &str) -> Result<(Vec<Backlink>, usize)> {
    let target = target.replace('\\', "/");
    let target = target.trim_start_matches("./");
    let (_, root_real, _) = resolve_in(root, target, "檔案")?;
    let name = target.rsplit('/').next().unwrap_or(target).to_string();
    let mut out: Vec<Backlink> = Vec::new();
    let mut more = 0usize;
    let md = all_files(root)
        .into_iter()
        .filter(|f| {
            let low = f.to_lowercase();
            (low.ends_with(".md") || low.ends_with(".markdown") || low.ends_with(".mdx"))
                && f != target
        })
        .take(BACKLINK_SCAN);
    for rel in md {
        let p = root_real.join(&rel);
        let Ok(meta) = std::fs::metadata(&p) else {
            continue;
        };
        if meta.len() > TEXT_MAX {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        // 先用檔名粗篩：連結裡一定有檔名（空白可能寫成 %20）
        if !text.contains(&name) && !text.contains(&name.replace(' ', "%20")) {
            continue;
        }
        let mut fence: Option<char> = None;
        for (i, line) in text.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("```") || t.starts_with("~~~") {
                let c = t.chars().next().unwrap_or('`');
                fence = match fence {
                    None => Some(c),
                    Some(f) if f == c => None,
                    f => f,
                };
                continue;
            }
            if fence.is_some() {
                continue;
            }
            let hit = MD_LINK
                .captures_iter(line)
                .any(|c| resolve_link(&rel, &c[1]).as_deref() == Some(target));
            if !hit {
                continue;
            }
            if out.len() >= BACKLINKS_MAX {
                more += 1;
            } else {
                out.push(Backlink {
                    rel: rel.clone(),
                    line: i + 1,
                    text: line.trim().chars().take(BACKLINK_TEXT).collect(),
                });
            }
        }
    }
    Ok((out, more))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backlinks_resolve_relative_links_and_skip_code() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        std::fs::create_dir_all(r.join("docs").join("sub")).unwrap();
        std::fs::write(r.join("docs").join("VISION.md"), "# 願景").unwrap();
        std::fs::write(
            r.join("README.md"),
            "看 [願景](docs/VISION.md#5-走到哪了)\n```\n[不算](docs/VISION.md)\n```\n外部 [x](https://a/VISION.md)",
        )
        .unwrap();
        std::fs::write(
            r.join("docs").join("sub").join("a.md"),
            "上層的 [V](../VISION.md \"t\")",
        )
        .unwrap();
        std::fs::write(
            r.join("docs").join("b.md"),
            "同名但別處 [V](sub/VISION.md)；根目錄 [V](/docs/VISION.md)",
        )
        .unwrap();
        std::fs::write(r.join("notes.txt"), "[V](docs/VISION.md)").unwrap();
        let (hits, more) = backlinks(r, "docs/VISION.md").unwrap();
        let got: Vec<(String, usize)> = hits.iter().map(|h| (h.rel.clone(), h.line)).collect();
        assert_eq!(
            got,
            vec![
                ("README.md".to_string(), 1),
                ("docs/b.md".to_string(), 1),
                ("docs/sub/a.md".to_string(), 1)
            ]
        );
        assert_eq!(more, 0);
        assert!(hits[0].text.starts_with("看 [願景]"));
        assert!(backlinks(r, "../x.md").is_err());
    }

    fn names(l: &Listing) -> Vec<(String, bool)> {
        l.entries.iter().map(|e| (e.name.clone(), e.dir)).collect()
    }

    #[test]
    fn plain_folder_lists_dirs_first_case_insensitive() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        std::fs::create_dir(r.join("src")).unwrap();
        std::fs::write(r.join("b.md"), "").unwrap();
        std::fs::write(r.join("A.md"), "").unwrap();
        std::fs::write(r.join("src").join("main.rs"), "").unwrap();
        let l = list_dir(r, "").unwrap();
        assert_eq!(
            names(&l),
            vec![
                ("src".into(), true),
                ("A.md".into(), false),
                ("b.md".into(), false)
            ]
        );
        let sub = list_dir(r, "src").unwrap();
        assert_eq!(sub.entries[0].rel, PathBuf::from("src").join("main.rs"));
        assert_eq!(sub.entries[0].abs, r.join("src").join("main.rs"));
    }

    #[test]
    fn git_repo_hides_dot_git_and_ignored() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        let ok = Command::new("git")
            .args(["init", "-q"])
            .current_dir(r)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return; // 沒有 git 的環境跳過
        }
        std::fs::write(r.join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::create_dir(r.join("target")).unwrap();
        std::fs::write(r.join("run.log"), "").unwrap();
        std::fs::write(r.join("README.md"), "").unwrap();
        let l = list_dir(r, "").unwrap();
        assert_eq!(
            names(&l),
            vec![(".gitignore".into(), false), ("README.md".into(), false)]
        );
    }

    #[test]
    fn rejects_paths_outside_repo() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path().join("repo");
        std::fs::create_dir_all(r.join("a")).unwrap();
        for bad in ["..", "a/../..", "/etc"] {
            assert!(list_dir(&r, bad).is_err(), "{bad} 應被拒");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(t.path(), r.join("out")).unwrap();
            assert!(list_dir(&r, "out").is_err(), "符號連結跳出 repo 應被拒");
            let l = list_dir(&r, "").unwrap();
            let out = l.entries.iter().find(|e| e.name == "out").unwrap();
            assert!(!out.dir, "符號連結當檔案");
        }
        assert!(list_dir(&r, "nope").is_err());
    }

    #[test]
    fn base64_matches_rfc4648() {
        for (raw, enc) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(raw.as_bytes()), enc);
        }
    }

    #[test]
    fn read_file_text_image_binary_and_guards() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path().join("repo");
        std::fs::create_dir_all(r.join("docs")).unwrap();
        std::fs::write(r.join("docs").join("a.md"), "# 標題\n內容\n").unwrap();
        std::fs::write(r.join("logo.png"), [0x89u8, b'P', b'N', b'G']).unwrap();
        std::fs::write(r.join("blob.bin"), [1u8, 0, 2]).unwrap();
        std::fs::write(t.path().join("secret.txt"), "x").unwrap();

        let f = read_file(&r, "docs/a.md").unwrap();
        assert!(matches!(
            &f.view,
            FileView::Text { text, truncated: false, editable: true, .. } if text == "# 標題\n內容\n"
        ));
        assert_eq!(f.abs, r.join("docs").join("a.md"));
        assert!(matches!(
            read_file(&r, "logo.png").unwrap().view,
            FileView::Image {
                mime: "image/png",
                ..
            }
        ));
        assert_eq!(read_file(&r, "blob.bin").unwrap().view, FileView::Binary);
        assert!(read_file(&r, "../secret.txt").is_err());
        assert!(read_file(&r, "docs").is_err(), "資料夾不是檔案");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(t.path().join("secret.txt"), r.join("link.txt")).unwrap();
            assert!(
                read_file(&r, "link.txt").is_err(),
                "符號連結跳出 repo 應被拒"
            );
        }
    }

    #[test]
    fn read_file_truncates_big_text() {
        let t = tempfile::tempdir().unwrap();
        let big = "a".repeat(TEXT_MAX as usize + 10);
        std::fs::write(t.path().join("big.txt"), &big).unwrap();
        let FileView::Text {
            text,
            truncated,
            editable,
            ..
        } = read_file(t.path(), "big.txt").unwrap().view
        else {
            panic!("要是文字");
        };
        assert!(truncated);
        assert!(!editable, "截斷的檔存回去會掉後半段");
        assert_eq!(text.len(), TEXT_MAX as usize);
        assert!(write_file(t.path(), "big.txt", "x", "whatever").is_err());
    }

    fn version(root: &Path, rel: &str) -> String {
        match read_file(root, rel).unwrap().view {
            FileView::Text { version, .. } => version,
            v => panic!("要是文字：{v:?}"),
        }
    }

    #[test]
    fn write_file_saves_and_detects_conflict() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        std::fs::write(r.join("a.md"), "舊\n").unwrap();
        let v1 = version(r, "a.md");

        let WriteOutcome::Saved(f) = write_file(r, "a.md", "新\n", &v1).unwrap() else {
            panic!("要寫成功");
        };
        assert_eq!(std::fs::read_to_string(r.join("a.md")).unwrap(), "新\n");
        let FileView::Text { version: v2, .. } = f.view else {
            panic!("要是文字");
        };
        assert_ne!(v1, v2);

        // agent 在使用者編輯時改了檔：拿舊 version 存＝衝突，不寫
        std::fs::write(r.join("a.md"), "agent 寫的\n").unwrap();
        assert_eq!(
            write_file(r, "a.md", "我的\n", &v2).unwrap(),
            WriteOutcome::Conflict
        );
        assert_eq!(
            std::fs::read_to_string(r.join("a.md")).unwrap(),
            "agent 寫的\n"
        );
        // 沒留下暫存檔
        let names: Vec<_> = std::fs::read_dir(r)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
    }

    #[test]
    fn write_file_keeps_crlf_and_bom() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        std::fs::write(r.join("w.txt"), "\u{feff}a\r\nb\r\n").unwrap();
        let v = version(r, "w.txt");
        // textarea 送回來是 LF、可能沒有 BOM
        write_file(r, "w.txt", "a\nb\nc\n", &v).unwrap();
        assert_eq!(
            std::fs::read(r.join("w.txt")).unwrap(),
            "\u{feff}a\r\nb\r\nc\r\n".as_bytes()
        );
    }

    #[test]
    fn write_file_guards() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path().join("repo");
        std::fs::create_dir_all(&r).unwrap();
        std::fs::write(t.path().join("secret.txt"), "x").unwrap();
        std::fs::write(r.join("bad.txt"), [b'a', 0xff, b'b']).unwrap();
        std::fs::write(r.join("logo.png"), [0x89u8, b'P']).unwrap();

        assert!(write_file(&r, "../secret.txt", "y", "").is_err());
        assert!(write_file(&r, "new.txt", "y", "").is_err(), "不新增檔案");
        assert!(write_file(&r, "logo.png", "y", "").is_err());
        let FileView::Text { editable, .. } = read_file(&r, "bad.txt").unwrap().view else {
            panic!("要是文字");
        };
        assert!(!editable, "壞 UTF-8 存回去會變 �");
        let v = version(&r, "bad.txt");
        assert!(write_file(&r, "bad.txt", "y", &v).is_err());
        assert_eq!(
            std::fs::read_to_string(t.path().join("secret.txt")).unwrap(),
            "x"
        );
    }

    #[test]
    fn create_entry_makes_files_and_dirs_without_overwriting() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path().join("repo");
        std::fs::create_dir_all(r.join(".git")).unwrap();
        std::fs::write(r.join("a.md"), "keep").unwrap();

        let abs = create_entry(&r, "notes/2026/new.md", NewEntry::File).unwrap();
        assert_eq!(abs, r.join("notes").join("2026").join("new.md"));
        assert_eq!(std::fs::read_to_string(&abs).unwrap(), "");
        create_entry(&r, "docs", NewEntry::Dir).unwrap();
        assert!(r.join("docs").is_dir());

        // 已存在不覆蓋；檔案當上層不行；跳出 repo、.git、根目錄都擋
        assert!(create_entry(&r, "a.md", NewEntry::File).is_err());
        assert_eq!(std::fs::read_to_string(r.join("a.md")).unwrap(), "keep");
        assert!(create_entry(&r, "docs", NewEntry::Dir).is_err());
        assert!(create_entry(&r, "a.md/x.md", NewEntry::File).is_err());
        assert!(create_entry(&r, "../out.md", NewEntry::File).is_err());
        assert!(create_entry(&r, ".git/hooks/x", NewEntry::File).is_err());
        assert!(create_entry(&r, "", NewEntry::Dir).is_err());
        assert!(!t.path().join("out.md").exists());
    }

    #[test]
    fn save_image_roundtrips_and_guards() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path().join("repo");
        std::fs::create_dir_all(&r).unwrap();
        for raw in [
            &b""[..],
            b"a",
            b"ab",
            b"abc",
            &[0x89, b'P', b'N', b'G', 0, 255],
        ] {
            assert_eq!(unbase64(&base64(raw)).unwrap(), raw);
        }
        let png = [0x89u8, b'P', b'N', b'G', 1, 2, 3];
        let abs = save_image(&r, "assets/shot.png", &base64(&png)).unwrap();
        assert_eq!(std::fs::read(&abs).unwrap(), png);
        assert!(
            save_image(&r, "assets/shot.png", &base64(&png)).is_err(),
            "不覆蓋"
        );
        assert!(save_image(&r, "x.svg", &base64(b"<svg/>")).is_err());
        assert!(save_image(&r, "x.txt", &base64(b"hi")).is_err());
        assert!(save_image(&r, "y.png", "不是base64!").is_err());
        assert!(save_image(&r, "../y.png", &base64(&png)).is_err());
    }

    #[test]
    fn rename_entry_moves_without_overwriting() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path().join("repo");
        std::fs::create_dir_all(r.join("docs").join(".git")).unwrap();
        std::fs::write(r.join("a.md"), "A").unwrap();
        std::fs::write(r.join("b.md"), "B").unwrap();

        let (abs, dir) = rename_entry(&r, "a.md", "notes/a2.md").unwrap();
        assert_eq!(abs, r.join("notes").join("a2.md"));
        assert!(!dir && !r.join("a.md").exists());
        assert_eq!(std::fs::read_to_string(&abs).unwrap(), "A");
        let (_, dir) = rename_entry(&r, "notes", "archive").unwrap();
        assert!(dir && r.join("archive").join("a2.md").exists());

        // 不覆蓋、來源要存在、根目錄與 .git 擋、跳出 repo 擋
        assert!(rename_entry(&r, "b.md", "archive/a2.md").is_err());
        assert_eq!(std::fs::read_to_string(r.join("b.md")).unwrap(), "B");
        assert!(rename_entry(&r, "nope.md", "x.md").is_err());
        assert!(rename_entry(&r, "", "x").is_err());
        assert!(rename_entry(&r, "docs/.git", "g").is_err());
        assert!(rename_entry(&r, "b.md", "../b.md").is_err());
        // 資料夾搬進自己底下：擋下，而且不留下空資料夾
        assert!(rename_entry(&r, "archive", "archive/sub/x").is_err());
        assert!(!r.join("archive").join("sub").exists());
        assert!(rename_entry(&r, "../repo/b.md", "c.md").is_err());
        assert!(r.join("b.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn create_entry_blocks_symlink_escape() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path().join("repo");
        let outside = t.path().join("outside");
        std::fs::create_dir_all(&r).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, r.join("link")).unwrap();
        assert!(create_entry(&r, "link/x.md", NewEntry::File).is_err());
        assert!(!outside.join("x.md").exists());
    }

    #[test]
    fn file_score_prefers_name_then_path() {
        assert_eq!(file_score("docs/VISION.md", "vision.md"), Some(100));
        assert_eq!(file_score("docs/VISION.md", "vision"), Some(100));
        assert_eq!(file_score("docs/VISION.md", "vis"), Some(60));
        assert_eq!(file_score("docs/VISION.md", "sion"), Some(40));
        // 詞的開頭比詞中間好：vision 找 00_product_vision 先於 revision
        assert_eq!(file_score("a/00_product_vision.md", "vision"), Some(50));
        assert_eq!(file_score("a/productVision.md", "vision"), Some(50));
        assert_eq!(file_score("a/SupervisionParadox.md", "vision"), Some(40));
        assert_eq!(
            file_score("a/slide_05_version_revision.yaml", "vision"),
            Some(40)
        );
        assert_eq!(file_score("a/report2026.md", "2026"), Some(50));
        assert_eq!(file_score("docs/VISION.md", "docs/"), Some(20));
        assert_eq!(file_score("docs/VISION.md", "docs vision"), Some(15));
        assert_eq!(file_score("docs/VISION.md", "zzz"), None);
    }

    #[test]
    fn find_files_in_git_repo_skips_ignored_and_in_plain_folder_skips_hidden() {
        let t = tempfile::tempdir().unwrap();
        let g = crate::testkit::make_git_repo(t.path(), "g", 1, 0, true);
        std::fs::write(g.join(".gitignore"), "build/\n").unwrap();
        std::fs::create_dir_all(g.join("build")).unwrap();
        std::fs::write(g.join("build").join("plan.md"), "").unwrap();
        std::fs::write(g.join("plan-untracked.md"), "").unwrap();
        let rels: Vec<String> = find_files(&g, "plan", 10)
            .into_iter()
            .map(|h| h.rel)
            .collect();
        assert_eq!(rels, ["notes/plan.md", "plan-untracked.md"]);
        assert!(find_files(&g, "  ", 10).is_empty());

        let p = t.path().join("plain");
        std::fs::create_dir_all(p.join(".hidden")).unwrap();
        std::fs::create_dir_all(p.join("node_modules")).unwrap();
        std::fs::create_dir_all(p.join("sub")).unwrap();
        std::fs::write(p.join(".hidden").join("plan.md"), "").unwrap();
        std::fs::write(p.join("node_modules").join("plan.md"), "").unwrap();
        std::fs::write(p.join("sub").join("Plan.txt"), "").unwrap();
        let hits = find_files(&p, "PLAN", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            (hits[0].rel.as_str(), hits[0].name.as_str()),
            ("sub/Plan.txt", "Plan.txt")
        );
    }

    #[test]
    fn changes_and_diff_show_what_was_touched() {
        let t = tempfile::tempdir().unwrap();
        let g = crate::testkit::make_git_repo(t.path(), "g", 1, 0, true);
        std::fs::write(g.join("README.md"), "# g\n改過的一行\n").unwrap();
        std::fs::write(g.join("new.md"), "新檔第一行\n").unwrap();
        std::fs::remove_file(g.join("notes").join("plan.md")).unwrap();
        let (cs, more) = changes(&g).unwrap();
        assert_eq!(more, 0);
        let mut got: Vec<(String, ChangeKind)> = cs.into_iter().map(|c| (c.rel, c.kind)).collect();
        got.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            got,
            vec![
                ("README.md".into(), ChangeKind::Modified),
                ("new.md".into(), ChangeKind::Untracked),
                ("notes/plan.md".into(), ChangeKind::Deleted),
            ]
        );
        let (d, cut) = diff(&g, "README.md").unwrap();
        assert!(!cut);
        assert!(
            d.contains("-這個 repo 談 widget 快取設計。") && d.contains("+改過的一行"),
            "{d}"
        );
        let (d, _) = diff(&g, "new.md").unwrap();
        assert!(d.contains("+新檔第一行"), "{d}");
        let (d, _) = diff(&g, "notes/plan.md").unwrap();
        assert!(d.contains("-## 計劃"), "{d}");
        // 沒改的檔：空字串
        std::fs::write(g.join("README.md"), "# g\n這個 repo 談 widget 快取設計。\n").unwrap();
        assert_eq!(diff(&g, "README.md").unwrap().0, "");
        assert!(diff(&g, "../x").is_err());
        assert!(diff(&g, "").is_err());
        let plain = t.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(changes(&plain)
            .unwrap_err()
            .to_string()
            .contains("不是 git repo"));
    }

    #[test]
    fn changes_lists_renames_once_by_new_path() {
        let t = tempfile::tempdir().unwrap();
        let g = crate::testkit::make_git_repo(t.path(), "g", 1, 0, true);
        crate::testkit::git(&g, &["mv", "notes/plan.md", "notes/renamed.md"], &[]);
        let (cs, _) = changes(&g).unwrap();
        // -z 格式：新路徑在前、原路徑在後；原路徑不另列一筆
        assert_eq!(
            cs,
            vec![Change {
                rel: "notes/renamed.md".into(),
                kind: ChangeKind::Renamed
            }]
        );
    }
}
