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

/// 先寫同目錄的暫存檔（檔名帶行程 id，同時寫不互踩）再改名：讀的人不會看到寫一半的檔。
pub fn write_atomic(p: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = p.as_os_str().to_owned();
    tmp.push(format!(".tmp{}", std::process::id()));
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, p).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
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
}
