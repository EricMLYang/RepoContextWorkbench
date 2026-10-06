//! 寫 YAML（registry.yaml 等人也會手改、Python 也會讀的檔）。
//!
//! 為什麼不直接用 `serde_yaml::to_string`：serde_yaml 依 YAML 1.2 判斷要不要加引號，
//! `2026-09-25`、`yes`、`on`、`1_000`、`190:20:30` 這類字串會寫成裸字；Python 的 PyYAML 是 YAML 1.1，
//! 讀回來會變成日期／布林／整數——資料格式合約就破了。這裡用保守規則：任一版 YAML 可能解成非字串的
//! 一律加引號。版面照 PyYAML `safe_dump(allow_unicode=True, sort_keys=False)` 的 block 風格
//! （序列項與父鍵同縮排、空容器寫 `[]`／`{}`），讓兩邊寫出的檔 diff 小。

use serde_yaml::{Mapping, Value};

/// 把 YAML 值寫成 block 風格文字（結尾有換行）。
pub fn to_string(v: &Value) -> String {
    let mut out = String::new();
    match v {
        Value::Mapping(m) if !m.is_empty() => write_map(&mut out, m, 0),
        Value::Sequence(s) if !s.is_empty() => write_seq(&mut out, s, 0),
        Value::Tagged(t) => return to_string(&t.value),
        other => {
            out.push_str(&scalar(other));
            out.push('\n');
        }
    }
    out
}

fn is_container(v: &Value) -> bool {
    match v {
        Value::Mapping(m) => !m.is_empty(),
        Value::Sequence(s) => !s.is_empty(),
        Value::Tagged(t) => is_container(&t.value),
        _ => false,
    }
}

fn untag(v: &Value) -> &Value {
    match v {
        Value::Tagged(t) => untag(&t.value),
        other => other,
    }
}

fn write_map(out: &mut String, m: &Mapping, indent: usize) {
    for (i, (k, v)) in m.iter().enumerate() {
        if i > 0 {
            out.push_str(&" ".repeat(indent));
        }
        out.push_str(&scalar(k));
        out.push(':');
        match untag(v) {
            Value::Mapping(c) if !c.is_empty() => {
                out.push('\n');
                out.push_str(&" ".repeat(indent + 2));
                write_map(out, c, indent + 2);
            }
            Value::Sequence(s) if !s.is_empty() => {
                out.push('\n');
                out.push_str(&" ".repeat(indent));
                write_seq(out, s, indent);
            }
            other => {
                out.push(' ');
                out.push_str(&scalar(other));
                out.push('\n');
            }
        }
    }
}

fn write_seq(out: &mut String, s: &[Value], indent: usize) {
    for (i, v) in s.iter().enumerate() {
        if i > 0 {
            out.push_str(&" ".repeat(indent));
        }
        out.push_str("- ");
        match untag(v) {
            Value::Mapping(c) if is_container(v) => write_map(out, c, indent + 2),
            Value::Sequence(c) if is_container(v) => write_seq(out, c, indent + 2),
            other => {
                out.push_str(&scalar(other));
                out.push('\n');
            }
        }
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => if *b { "true" } else { "false" }.into(),
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                n.to_string()
            } else {
                let f = n.as_f64().unwrap_or(0.0);
                if f.is_nan() {
                    ".nan".into()
                } else if f.is_infinite() {
                    if f > 0.0 { ".inf" } else { "-.inf" }.into()
                } else {
                    // PyYAML 的 float 需要小數點（`1e-07` 會被讀成字串）
                    let s = format!("{f:?}");
                    if s.contains('.') {
                        s
                    } else if let Some((m, e)) = s.split_once('e') {
                        format!("{m}.0e{}{e}", if e.starts_with('-') { "" } else { "+" })
                    } else {
                        format!("{s}.0")
                    }
                }
            }
        }
        Value::String(s) => string(s),
        Value::Sequence(_) => "[]".into(),
        Value::Mapping(_) => "{}".into(),
        Value::Tagged(t) => scalar(&t.value),
    }
}

/// 任一版 YAML 都會當字串讀的才寫裸字，否則雙引號。
fn plain_ok(s: &str) -> bool {
    let Some(first) = s.chars().next() else {
        return false;
    };
    if s.starts_with(' ') || s.ends_with(' ') || s.ends_with(':') {
        return false;
    }
    if "-?:,[]{}#&*!|>'\"%@`".contains(first) {
        return false;
    }
    if s.contains(": ") || s.contains(" #") || s.contains('\t') {
        return false;
    }
    if s.chars().any(|c| {
        (c as u32) < 0x20
            || matches!(
                c,
                '\u{7f}' | '\u{85}' | '\u{2028}' | '\u{2029}' | '\u{feff}'
            )
    }) {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    const RESERVED: [&str; 16] = [
        "y", "n", "yes", "no", "on", "off", "true", "false", "null", "~", "=", "<<", ".inf",
        "+.inf", "-.inf", ".nan",
    ];
    if RESERVED.contains(&lower.as_str()) {
        return false;
    }
    // 數字／日期／六十進位長相（保守：開頭是數字或正負號、點，且只由數字類字元組成）
    if "0123456789+-.".contains(first)
        && s.chars()
            .all(|c| "0123456789+-.:_eExXoObBabcdefABCDEF TtZz".contains(c))
    {
        return false;
    }
    true
}

fn string(s: &str) -> String {
    if plain_ok(s) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\x{:02X}", c as u32))
            }
            '\u{85}' | '\u{2028}' | '\u{2029}' | '\u{feff}' => {
                out.push_str(&format!("\\u{:04X}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_like_pyyaml() {
        let v: Value = serde_yaml::from_str(
            "repos:\n- id: a\n  tags: []\n  relations:\n  - to: b\n    kind: pm-of\n  exports:\n    書摘:\n      path: x/\ngroups: []\nm: {}\n",
        )
        .unwrap();
        assert_eq!(
            to_string(&v),
            "repos:\n- id: a\n  tags: []\n  relations:\n  - to: b\n    kind: pm-of\n  exports:\n    書摘:\n      path: x/\ngroups: []\nm: {}\n"
        );
    }

    #[test]
    fn ambiguous_strings_are_quoted_and_roundtrip() {
        let vals = [
            "2026-09-25",
            "2026-09-25 14:00",
            "09:00",
            "yes",
            "On",
            "No",
            "1e3",
            "0123",
            "0x1F",
            "~",
            "null",
            "3.0",
            "中文: x",
            "-",
            "a #b",
            ".inf",
            "1_000",
            "y",
            "+12",
            "",
            "190:20:30",
            "多行\n第二行",
            "引號\"與\\",
            " 前空白",
            "a: b",
            "ok 字串",
            "repo-a",
            "notes/",
        ];
        let mut m = Mapping::new();
        for (i, s) in vals.iter().enumerate() {
            m.insert(format!("k{i}").into(), (*s).into());
        }
        let text = to_string(&Value::Mapping(m.clone()));
        assert_eq!(
            serde_yaml::from_str::<Value>(&text).unwrap(),
            Value::Mapping(m)
        );
        assert!(
            text.contains("k25: ok 字串\n")
                && text.contains("k26: repo-a\n")
                && text.contains("k0: \"2026-09-25\"\n")
        );
    }

    #[test]
    fn numbers_and_nested_seq() {
        let v: Value =
            serde_yaml::from_str("a: 1\nb: 1.5\nc: [[x, y], []]\nd: null\ne: true\n").unwrap();
        assert_eq!(
            to_string(&v),
            "a: 1\nb: 1.5\nc:\n- - x\n  - \"y\"\n- []\nd: null\ne: true\n"
        );
    }
}
