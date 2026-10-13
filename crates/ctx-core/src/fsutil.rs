//! 檔案／路徑小工具：把 Python `pathlib` 的幾個慣用語義搬成 Rust，讓各模組行為對得上。
//!
//! 字串層面的 Python 語義（`str.splitlines`、`repr`、切片）在 `pystr`；這裡只放路徑與檔案。
//!
//! - `expanduser` ＝ `Path.expanduser()`
//! - `resolve` ＝ `Path.resolve()`（非 strict：路徑不存在也不報錯）
//! - `read_text_lossy` ＝ `read_text(encoding="utf-8", errors="replace")`（含 universal newline）
//! - `mtime_secs` ＝ `stat().st_mtime`（與 CPython 同算法，Python 寫的 mtime 快取才比得上）

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// 對應 `Path.expanduser()`：開頭的 `~`（`~`、`~/…`、`~\\…`）換成家目錄
/// （unix 讀 HOME，Windows 退而讀 USERPROFILE）；`~user` 不展開。
pub fn expanduser(p: impl AsRef<Path>) -> PathBuf {
    let p = p.as_ref();
    let s = p.to_string_lossy();
    if s == "~" || s.starts_with("~/") || s.starts_with("~\\") {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        if let Some(h) = home {
            let rest = s[1..].trim_start_matches(['/', '\\']);
            let base = PathBuf::from(h);
            return if rest.is_empty() {
                base
            } else {
                base.join(rest)
            };
        }
    }
    p.to_path_buf()
}

/// 對應 `Path.resolve()`（非 strict）：存在就 canonicalize；不存在就把最長存在的前綴
/// canonicalize（符號連結照解，例：macOS 的 /var → /private/var）後接上其餘部分，`..` 照字面消去。
/// 與 Python 一樣不報錯。
pub fn resolve(p: &Path) -> PathBuf {
    if let Ok(c) = p.canonicalize() {
        return c;
    }
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    };
    let mut out = PathBuf::new();
    for comp in abs.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => {
                out.push(c.as_os_str());
                if let Ok(canon) = out.canonicalize() {
                    out = canon;
                }
            }
        }
    }
    out
}

/// 對應 `read_text(encoding="utf-8", errors="replace")`：壞位元組換 U+FFFD，
/// 且照 Python universal newline 把 `\r\n`／`\r` 轉成 `\n`（字數估算才對得上）。
pub fn read_text_lossy(p: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(p)?;
    let s = String::from_utf8_lossy(&bytes);
    if s.contains('\r') {
        Ok(s.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Ok(s.into_owned())
    }
}

/// 對應 `os.stat(p).st_mtime`：CPython 以 `sec + nsec * 1e-9` 算浮點，這裡同算法，
/// 讓兩邊寫進同一份快取的 mtime 能用 `==` 比對。
pub fn mtime_secs(p: &Path) -> std::io::Result<f64> {
    let m = std::fs::metadata(p)?.modified()?;
    Ok(match m.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as f64 + f64::from(d.subsec_nanos()) * 1e-9,
        Err(e) => -(e.duration().as_secs_f64()),
    })
}

/// 原子寫檔（DEVELOPMENT.md §8）：同資料夾的暫存檔（隱藏、名稱不可預測、`create_new` 不跟隨既有的檔或
/// 符號連結）→ fsync → 原檔在就沿用它的權限 → rename。讀的人不會看到寫一半的檔；失敗不留暫存檔。
/// `p` 是符號連結時改寫的是連結本身（要寫實體檔請先 canonicalize）。
pub fn write_atomic(p: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = p.parent().unwrap_or_else(|| Path::new("."));
    let name = p.file_name().unwrap_or_default().to_string_lossy();
    let nonce = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(
        ".{name}.ctx-tmp-{}-{nonce:08x}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let perms = std::fs::metadata(p).ok().map(|m| m.permissions());
    let res = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        if let Some(perms) = perms {
            std::fs::set_permissions(&tmp, perms)?;
        }
        std::fs::rename(&tmp, p)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// 現在的 Unix 秒數（浮點）——對應 `time.time()`。
pub fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanduser_like_python() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("測試環境有 HOME"));
        assert_eq!(expanduser("~"), home);
        assert_eq!(expanduser("~/a/b"), home.join("a/b"));
        assert_eq!(expanduser("~x/a"), PathBuf::from("~x/a"));
        assert_eq!(expanduser("/abs"), PathBuf::from("/abs"));
    }

    #[test]
    fn resolve_nonexistent_tail() {
        let d = std::env::temp_dir();
        let canon = d.canonicalize().expect("temp dir 存在");
        assert_eq!(resolve(&d.join("nope/../x")), canon.join("x"));
    }

    #[test]
    fn write_atomic_replaces_keeps_permissions_and_leaves_no_temp() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("a.json");
        write_atomic(&p, b"{}").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
            write_atomic(&p, b"{\"x\":1}").unwrap();
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert!(std::fs::read_to_string(&p).unwrap().contains('x') || cfg!(not(unix)));
        let names: Vec<String> = std::fs::read_dir(t.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.json"]);
        // 上層資料夾不存在：失敗、不留東西
        assert!(write_atomic(&t.path().join("nope").join("b.json"), b"x").is_err());
    }
}
