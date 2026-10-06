//! 照 Python 行為的小工具：`str.splitlines`、`repr(str)`、`str(x)`、`Path.expanduser`。
//! 給人看的訊息要逐字照抄 Python，有些地方（例：`{line!r}`）就得模仿 Python 的字串表示。

use std::path::{Path, PathBuf};

/// 對應 Python `str.splitlines()`：認 `\n`、`\r\n`、`\r`、`\v`、`\f`、`\x1c-\x1e`、`\x85`、` `、` `。
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
        let mut next = i + c.len_utf8();
        if c == '\r' {
            if let Some(&(j, '\n')) = it.peek() {
                it.next();
                next = j + 1;
            }
        }
        start = next;
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// 對應 Python `repr(str)`（錯誤訊息裡的 `{x!r}`）。
pub fn repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// 對應 Python `f"{x}"`（JSON 值）：字串原樣、null→`None`、布林→`True`/`False`，其餘照 JSON。
pub fn py_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "None".into(),
        serde_json::Value::Bool(true) => "True".into(),
        serde_json::Value::Bool(false) => "False".into(),
        other => other.to_string(),
    }
}

/// 對應 Python `f"{x}"`（YAML 值）。
pub fn py_str_yaml(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Null => "None".into(),
        serde_yaml::Value::Bool(true) => "True".into(),
        serde_yaml::Value::Bool(false) => "False".into(),
        serde_yaml::Value::Number(n) => n.to_string(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// 對應 Python `Path(p).expanduser()`：開頭的 `~` 換成 `$HOME`。
pub fn expanduser(p: impl AsRef<Path>) -> PathBuf {
    let p = p.as_ref();
    if let Ok(rest) = p.strip_prefix("~") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    p.to_path_buf()
}

/// 對應 Python `Path.resolve()`（非 strict）：存在就 canonicalize；不存在就把最長存在的前綴
/// canonicalize 後接上其餘部分（`..` 照字面消去）。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitlines_like_python() {
        assert_eq!(splitlines("a\nb\r\nc\rd"), vec!["a", "b", "c", "d"]);
        assert_eq!(splitlines("a\n"), vec!["a"]);
        assert_eq!(splitlines(""), Vec::<&str>::new());
        assert_eq!(splitlines("\n\nx"), vec!["", "", "x"]);
    }

    #[test]
    fn repr_like_python() {
        assert_eq!(repr("ab"), "'ab'");
        assert_eq!(repr("a'b"), "\"a'b\"");
        assert_eq!(repr("a'b\""), "'a\\'b\"'");
        assert_eq!(repr("中\n"), "'中\\n'");
    }
}
