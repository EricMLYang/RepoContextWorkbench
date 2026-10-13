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

    // 暫存檔放同一個資料夾，改名才是原子的；寫到符號連結指向的實體檔，連結本身不動
    let dir = target
        .parent()
        .ok_or_else(|| Error::invalid(format!("找不到上層資料夾：{rel}")))?;
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    // 暫存檔名不可預測、create_new 不跟隨既有的檔或符號連結（同資料夾有人先放好連結也寫不出去）
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(
        ".{name}.ctx-save-{}-{nonce:08x}-{seq}",
        std::process::id()
    ));
    let res = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(out.as_bytes())?;
        f.sync_all()?;
        std::fs::set_permissions(&tmp, meta.permissions())?;
        std::fs::rename(&tmp, &target)
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(WriteOutcome::Saved(read_file(root, rel)?))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
