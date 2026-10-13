//! 照 Python 行為的字串小工具：`str.splitlines`、`repr(str)`、`str(x)`、`s[:n]`。
//! 路徑（`expanduser`／`resolve`）在 `fsutil`。
//! 給人看的訊息要逐字照抄 Python，有些地方（例：`{line!r}`）就得模仿 Python 的字串表示。

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

/// Python 真值判斷 `bool(x)`／`if x:`（JSON 值）：null、false、0、空字串、空陣列、空物件為假。
pub fn truthy(v: &serde_json::Value) -> bool {
    use serde_json::Value;
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// 同 [`truthy`]（YAML 值；有 tag 的值一律為真）。
pub fn truthy_yaml(v: &serde_yaml::Value) -> bool {
    use serde_yaml::Value;
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Sequence(s) => !s.is_empty(),
        Value::Mapping(m) => !m.is_empty(),
        Value::Tagged(_) => true,
    }
}

/// Python 字串切片 `s[:n]`（以字元計，不是位元組）。
pub fn take_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// 最多 `n` 個字元；截掉了就補「…」（讓讀的人知道不是全文）。Rust 才有。
pub fn clip(s: &str, n: usize) -> String {
    let head = take_chars(s, n);
    if head.len() < s.len() {
        format!("{head}…")
    } else {
        head.to_string()
    }
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

    #[test]
    fn take_chars_counts_chars() {
        assert_eq!(take_chars("中文字", 2), "中文");
        assert_eq!(take_chars("ab", 5), "ab");
    }
}
