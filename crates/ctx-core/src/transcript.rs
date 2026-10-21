//! 只讀 Claude Code 逐字稿（`~/.claude/projects/<編碼後的 cwd>/<session>.jsonl`）的頭尾（2026-10-16，Rust 才有）。
//!
//! 逐字稿一份可以到 20MB；會話列的標題、最後回覆，過去會話清單的 cwd、開始時間，都只要頭或尾幾十 KB。
//! 不整份讀、不寫——backfill 要整份內容時走它自己的路。

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

/// 頭尾各讀多少位元組。一行（附件、工具輸出）可能很長，尾巴給寬一點。
const HEAD_BYTES: u64 = 64 * 1024;
const TAIL_BYTES: u64 = 256 * 1024;

/// 逐字稿尾巴看得到的：Claude Code 自己取的標題（`ai-title`，會隨對話更新，取最後一筆）、
/// 最後一則 agent 回覆的文字、最後一筆時間。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tail {
    pub title: Option<String>,
    pub reply: Option<String>,
    pub last: Option<String>,
}

/// 逐字稿開頭看得到的：工作目錄、開始時間、第一句使用者訊息、Claude Code 版本給的 git 分支。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Head {
    pub cwd: Option<String>,
    pub first: Option<String>,
    pub prompt: Option<String>,
    pub branch: Option<String>,
    /// 誰開的：`cli`＝互動、`sdk-cli`＝`claude -p` 程式化呼叫（取第一則使用者訊息上的）。
    pub entrypoint: Option<String>,
    /// 開頭是上一場的壓縮摘要（這場是接續的；`prompt` 只取摘要之後真的有人打的那句）。
    pub continued: bool,
}

/// 讀 `[start, start+len)` 的完整行（起點不在檔頭時丟掉第一個半行）。
fn lines_in(path: &Path, from_end: bool, len: u64) -> Option<Vec<Value>> {
    let mut f = std::fs::File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    let start = if from_end {
        size.saturating_sub(len)
    } else {
        0
    };
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.take(len).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let mut it = text.split('\n');
    if start > 0 {
        it.next();
    }
    let mut rows: Vec<&str> = it.collect();
    if !from_end && size > len {
        // 讀到一半的最後一行不算
        rows.pop();
    }
    Some(
        rows.into_iter()
            .filter_map(|ln| serde_json::from_str::<Value>(ln).ok())
            .collect(),
    )
}

/// assistant 訊息裡的文字區塊（不含 thinking、工具呼叫）接起來。
fn assistant_text(o: &Value) -> Option<String> {
    let blocks = o.get("message")?.get("content")?.as_array()?;
    let text: Vec<&str> = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    (!text.is_empty()).then(|| text.join("\n"))
}

/// 壓縮摘要的開頭（舊版逐字稿沒有 `isCompactSummary` 旗標時靠這句認）。
const COMPACT_PREFIX: &str = "this session is being continued from a previous conversation";
/// 壓縮摘要列（N-40）：模型確實讀到了，但不是人打的——不能當第一句。
pub(crate) fn is_compact_summary(o: &Value, text: &str) -> bool {
    o.get("isCompactSummary") == Some(&Value::Bool(true))
        || text
            .get(..COMPACT_PREFIX.len())
            .is_some_and(|h| h.eq_ignore_ascii_case(COMPACT_PREFIX))
}

/// harness 自己塞進去、寫成 `type: user` 的列（N-40，抄 BAT `harness-noise.mjs`）：
/// 背景任務通知（`origin.kind: task-notification`／`promptSource: system`）、本機指令的 caveat（有 `isMeta`）、
/// 斜線指令（`<command-name>/clear…`，沒有任何旗標）、工具中斷標記。人沒打過字，不能當標題或第一句。
/// 開頭是 `<` 標籤的一律不算（跟 backfill `compact` 同一條規則；人打的提示幾乎不會以標籤開頭）。
pub(crate) fn is_harness_noise(o: &Value, text: &str) -> bool {
    o.get("isMeta") == Some(&Value::Bool(true))
        || o.get("promptSource").and_then(Value::as_str) == Some("system")
        || o.get("origin")
            .and_then(|x| x.get("kind"))
            .and_then(Value::as_str)
            == Some("task-notification")
        || text.starts_with('<')
        || text.starts_with("[Request interrupted by user")
}

/// 使用者自己打的訊息（content 是字串；工具結果是陣列，不算；harness 雜訊與壓縮摘要不算）。
fn user_text(o: &Value) -> Option<&str> {
    if o.get("type").and_then(Value::as_str) != Some("user") {
        return None;
    }
    let text = o
        .get("message")?
        .get("content")?
        .as_str()
        .map(str::trim)
        .filter(|t| !t.is_empty())?;
    (!is_harness_noise(o, text) && !is_compact_summary(o, text)).then_some(text)
}

fn str_of(o: &Value, k: &str) -> Option<String> {
    o.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 讀逐字稿尾巴。檔案不在或讀不了＝全部 None。
pub fn tail(path: &Path) -> Tail {
    let Some(rows) = lines_in(path, true, TAIL_BYTES) else {
        return Tail::default();
    };
    let mut t = Tail::default();
    for o in rows.iter().rev() {
        if t.last.is_none() {
            t.last = str_of(o, "timestamp");
        }
        match o.get("type").and_then(Value::as_str) {
            Some("ai-title") if t.title.is_none() => t.title = str_of(o, "aiTitle"),
            Some("assistant") if t.reply.is_none() => t.reply = assistant_text(o),
            _ => {}
        }
        if t.title.is_some() && t.reply.is_some() && t.last.is_some() {
            break;
        }
    }
    t
}

/// 讀逐字稿開頭。
pub fn head(path: &Path) -> Head {
    let Some(rows) = lines_in(path, false, HEAD_BYTES) else {
        return Head::default();
    };
    let mut h = Head::default();
    for o in &rows {
        if h.cwd.is_none() {
            h.cwd = str_of(o, "cwd");
        }
        if h.first.is_none() {
            h.first = str_of(o, "timestamp");
        }
        if h.branch.is_none() {
            h.branch = str_of(o, "gitBranch");
        }
        if h.entrypoint.is_none() && o.get("type").and_then(Value::as_str) == Some("user") {
            h.entrypoint = str_of(o, "entrypoint");
        }
        if h.prompt.is_none() {
            h.prompt = user_text(o).map(str::to_string);
        }
        if !h.continued
            && o.get("type").and_then(Value::as_str) == Some("user")
            && o.get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_str)
                .is_some_and(|t| is_compact_summary(o, t.trim()))
        {
            h.continued = true;
        }
        if h.cwd.is_some()
            && h.first.is_some()
            && h.prompt.is_some()
            && h.branch.is_some()
            && h.entrypoint.is_some()
        {
            break;
        }
    }
    h
}

/// 一行預覽：空白摺成一個空格、最多 `n` 個字元（截掉補「…」）。
pub fn one_line(s: &str, n: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    crate::pystr::clip(&flat, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rows: &[&str]) -> std::path::PathBuf {
        let p = dir.join("s1.jsonl");
        std::fs::write(&p, rows.join("\n") + "\n").unwrap();
        p
    }

    #[test]
    fn head_and_tail_pick_the_right_rows() {
        let t = tempfile::tempdir().unwrap();
        let p = write(
            t.path(),
            &[
                r#"{"type":"mode","mode":"x"}"#,
                r#"{"type":"user","isMeta":true,"message":{"content":"<command>"},"cwd":"/r","timestamp":"2026-10-16T01:00:00Z","gitBranch":"main"}"#,
                r#"{"type":"user","message":{"content":"  幫我修登入  "},"cwd":"/r","timestamp":"2026-10-16T01:00:01Z"}"#,
                r#"{"type":"ai-title","aiTitle":"舊標題"}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"第一輪"}]}}"#,
                r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"x"}]}}"#,
                r#"{"type":"ai-title","aiTitle":"修登入流程"}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"嗯"},{"type":"text","text":"改好了，\n測試都過"},{"type":"tool_use","name":"Bash"}]},"timestamp":"2026-10-16T01:05:00Z"}"#,
                r#"{"type":"last-prompt"}"#,
                "壞掉的一行",
            ],
        );
        assert_eq!(
            tail(&p),
            Tail {
                title: Some("修登入流程".into()),
                reply: Some("改好了，\n測試都過".into()),
                last: Some("2026-10-16T01:05:00Z".into()),
            }
        );
        assert_eq!(
            head(&p),
            Head {
                cwd: Some("/r".into()),
                first: Some("2026-10-16T01:00:00Z".into()),
                prompt: Some("幫我修登入".into()),
                branch: Some("main".into()),
                entrypoint: None,
                continued: false,
            }
        );
        assert_eq!(tail(&t.path().join("none.jsonl")), Tail::default());
        assert_eq!(one_line(" 改好了，\n 測試都過 ", 4), "改好了，…");
    }

    #[test]
    fn head_skips_harness_noise_and_the_compact_summary() {
        let t = tempfile::tempdir().unwrap();
        let p = write(
            t.path(),
            &[
                r#"{"type":"user","isCompactSummary":true,"message":{"content":"This session is being continued from a previous conversation that ran out of context."},"cwd":"/r","timestamp":"2026-10-16T01:00:00Z"}"#,
                r#"{"type":"user","promptSource":"system","origin":{"kind":"task-notification"},"message":{"content":"<task-notification>\n<task-id>x</task-id>\n</task-notification>"}}"#,
                r#"{"type":"user","isMeta":true,"message":{"content":"<local-command-caveat>…</local-command-caveat>"}}"#,
                r#"{"type":"user","message":{"content":"[Request interrupted by user for tool use]"}}"#,
                r#"{"type":"user","message":{"content":"<command-name>/clear</command-name>\n<command-message>clear</command-message>"}}"#,
                r#"{"type":"user","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#,
                r#"{"type":"user","message":{"content":"git add commit push"}}"#,
            ],
        );
        let h = head(&p);
        assert_eq!(h.prompt.as_deref(), Some("git add commit push"));
        assert!(h.continued);
        // 舊版逐字稿沒有旗標：靠那句開頭認
        let p2 = write(
            t.path(),
            &[
                r#"{"type":"user","message":{"content":"this session is being continued from a previous conversation…"}}"#,
            ],
        );
        let h2 = head(&p2);
        assert_eq!(h2.prompt, None);
        assert!(h2.continued);
    }

    #[test]
    fn tail_skips_the_partial_first_line_of_a_big_file() {
        let t = tempfile::tempdir().unwrap();
        let big = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{}"}}]}}}}"#,
            "很長".repeat(200_000)
        );
        let p = write(t.path(), &[&big, r#"{"type":"ai-title","aiTitle":"尾巴"}"#]);
        let got = tail(&p);
        assert_eq!(got.title.as_deref(), Some("尾巴"));
        assert_eq!(got.reply, None);
    }
}
