//! 檔案／路徑小工具：把 Python `pathlib` 的幾個慣用語義搬成 Rust，讓各模組行為對得上。
//!
//! - `expand_user` ＝ `Path.expanduser()`
//! - `resolve_lenient` ＝ `Path.resolve()`（非 strict：路徑不存在也不報錯）
//! - `read_text_lossy` ＝ `read_text(encoding="utf-8", errors="replace")`（含 universal newline）
//! - `splitlines` ＝ `str.splitlines()`
//! - `mtime_secs` ＝ `stat().st_mtime`（與 CPython 同算法，Python 寫的 mtime 快取才比得上）

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// 對應 `Path.expanduser()`：開頭的 `~` 換成家目錄（unix 讀 HOME，Windows 讀 USERPROFILE）。
pub fn expand_user(p: impl AsRef<Path>) -> PathBuf {
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

/// 對應 `Path.resolve()`（非 strict）：能 canonicalize 就 canonicalize；
/// 不存在時退成「絕對路徑」——與 Python 一樣不報錯。
pub fn resolve_lenient(p: &Path) -> PathBuf {
    match std::fs::canonicalize(p) {
        Ok(c) => c,
        Err(_) => std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()),
    }
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

/// 對應 `str.splitlines()`：Python 認得的所有行界都切，結尾行界不產生空行。
pub fn splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let is_break = matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if !is_break {
            continue;
        }
        out.push(&s[start..i]);
        let mut end = i + c.len_utf8();
        if c == '\r' {
            if let Some(&(j, '\n')) = it.peek() {
                it.next();
                end = j + 1;
            }
        }
        start = end;
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
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

/// 現在的 Unix 秒數（浮點）——對應 `time.time()`。
pub fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Python 字串切片 `s[:n]`（以字元計，不是位元組）。
pub fn take_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitlines_matches_python() {
        assert_eq!(splitlines("a\nb\r\nc\rd\n"), vec!["a", "b", "c", "d"]);
        assert_eq!(splitlines(""), Vec::<&str>::new());
        assert_eq!(splitlines("\n\n"), vec!["", ""]);
        assert_eq!(splitlines("x"), vec!["x"]);
    }

    #[test]
    fn take_chars_counts_chars() {
        assert_eq!(take_chars("中文字", 2), "中文");
        assert_eq!(take_chars("ab", 5), "ab");
    }
}
