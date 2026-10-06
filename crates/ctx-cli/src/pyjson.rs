//! 照 Python `json.dumps(..., ensure_ascii=False)` 的輸出格式：
//! - 不帶 indent：分隔符是 `", "` 與 `": "`（Python 預設），不是 serde 的 `,`／`:`。
//! - `indent=1`：跟 serde 的 PrettyFormatter(" ") 一樣（Python 有 indent 時項目分隔是 `","`）。
//!
//! CLI 的 `--json`、`_fail`、MCP 的每一行回應都走這裡，輸出跟 Python 版逐字一致（對拍用）。

use std::io;

use serde::Serialize;
use serde_json::ser::{Formatter, PrettyFormatter, Serializer};

/// Python `json.dumps(obj, ensure_ascii=False)` 的分隔符。
struct PyCompact;

impl Formatter for PyCompact {
    fn begin_array_value<W: ?Sized + io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            w.write_all(b", ")
        }
    }
    fn begin_object_key<W: ?Sized + io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            w.write_all(b", ")
        }
    }
    fn begin_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        w.write_all(b": ")
    }
}

/// `json.dumps(v, ensure_ascii=False)`。
pub fn dumps<T: Serialize + ?Sized>(v: &T) -> String {
    let mut buf = Vec::new();
    let mut ser = Serializer::with_formatter(&mut buf, PyCompact);
    v.serialize(&mut ser)
        .expect("serde_json::Value 序列化不會失敗");
    String::from_utf8(buf).expect("serde_json 只寫 UTF-8")
}

/// `json.dumps(v, ensure_ascii=False, indent=1)`。
pub fn dumps_indent<T: Serialize + ?Sized>(v: &T) -> String {
    let mut buf = Vec::new();
    let mut ser = Serializer::with_formatter(&mut buf, PrettyFormatter::with_indent(b" "));
    v.serialize(&mut ser)
        .expect("serde_json::Value 序列化不會失敗");
    String::from_utf8(buf).expect("serde_json 只寫 UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn python_separators() {
        let v = json!({"ok": false, "a": [1, "中"], "o": {}});
        assert_eq!(dumps(&v), r#"{"ok": false, "a": [1, "中"], "o": {}}"#);
        assert_eq!(
            dumps_indent(&v),
            "{\n \"ok\": false,\n \"a\": [\n  1,\n  \"中\"\n ],\n \"o\": {}\n}"
        );
    }
}
