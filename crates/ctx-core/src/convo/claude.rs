//! Claude Code 的掃描器（2026-11-18 F-134 從 `transcript.rs`、`history.rs`、`timeline.rs` 收進來；行為不變）。
//!
//! 逐字稿在 `~/.claude/projects/<編碼後的 cwd>/<session>.jsonl`，一份可以到 20MB 以上。
//! - 會話列的標題、最後回覆，過去會話清單的 cwd、開始時間，都只要頭或尾幾十 KB（[`head`]、[`tail`]）。
//! - 只看範圍內 repo 的專案資料夾：Claude Code 把 cwd 的非英數字元都換成 `-` 當資料夾名，
//!   repo 本身與它的子目錄（名稱多一段 `-…`）才讀，再用逐字稿裡的 cwd 確認真的在 repo 底下。
//! - 程式化呼叫（`claude -p`：App 助手、`ask_repo`、碰撞判定…，`entrypoint` 是 `sdk-*`）不列。
//! - 訊息（[`ClaudeScanner::row_items`]）：你說的（harness 塞的列不算）、回覆、思考、工具（名稱＋一行摘要）、
//!   工具結果、壓縮摘要、`ai-title`。只讀不寫。

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use super::{Item, Message, Origin, Role, Scanner, Session, ToolAction};
use crate::config::AgentKind;

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

/// Claude Code 專案資料夾名：cwd 的非英數字元都換成 `-`。
pub fn project_dir_name(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// 工具輸入裡人看得懂的那個欄位（跟會話狀態的工具標籤同一組鍵）。
fn tool_detail(input: Option<&Value>) -> String {
    const KEYS: [&str; 9] = [
        "command",
        "file_path",
        "path",
        "notebook_path",
        "url",
        "pattern",
        "description",
        "prompt",
        "query",
    ];
    let Some(obj) = input.and_then(Value::as_object) else {
        return String::new();
    };
    KEYS.iter()
        .find_map(|k| obj.get(*k).and_then(Value::as_str))
        .map(|v| one_line(v, 300))
        .unwrap_or_default()
}

/// 工具名 → 類別（交接包的「動過的檔」「跑過的指令」）。
fn tool_action(name: &str) -> ToolAction {
    match name {
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => ToolAction::Edit,
        "Bash" => ToolAction::Shell,
        _ => ToolAction::Other,
    }
}

/// 工具結果的文字：字串，或區塊陣列裡的文字（圖片等略過）。
fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// 逐字稿一列 → 事件（2026-10-21 F-94 時間軸的分類，抄 BAT `claude-cli-frames.mjs`）。
pub fn row_items(o: &Value, out: &mut Vec<Item>) {
    let at = o
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);
    let content = o.get("message").and_then(|m| m.get("content"));
    match o.get("type").and_then(Value::as_str) {
        Some("ai-title") => {
            if let Some(t) = o.get("aiTitle").and_then(Value::as_str) {
                out.push(Item::Title(t.to_string()));
            }
        }
        Some("user") => match content {
            Some(Value::String(s)) => {
                let t = s.trim();
                if t.is_empty() {
                    return;
                }
                if is_compact_summary(o, t) {
                    out.push(Item::Message(Message::new(Role::Compact, t, at)));
                } else if !is_harness_noise(o, t) {
                    out.push(Item::Message(Message::new(Role::User, t, at)));
                }
            }
            Some(Value::Array(blocks)) => {
                let mut said = Vec::new();
                for b in blocks {
                    match b.get("type").and_then(Value::as_str) {
                        Some("tool_result") => {
                            if let Some(id) = b.get("tool_use_id").and_then(Value::as_str) {
                                out.push(Item::ToolResult {
                                    call_id: id.to_string(),
                                    text: result_text(b.get("content")),
                                    error: b.get("is_error") == Some(&Value::Bool(true)),
                                });
                            }
                        }
                        Some("text") => {
                            if let Some(t) = b.get("text").and_then(Value::as_str) {
                                said.push(t.trim());
                            }
                        }
                        _ => {}
                    }
                }
                let text = said.join("\n");
                if !text.is_empty() && !is_harness_noise(o, &text) {
                    out.push(Item::Message(Message::new(Role::User, &text, at)));
                }
            }
            _ => {}
        },
        Some("assistant") => {
            let Some(blocks) = content.and_then(Value::as_array) else {
                return;
            };
            for b in blocks {
                match b.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        let t = b.get("text").and_then(Value::as_str).unwrap_or("");
                        if !t.trim().is_empty() {
                            out.push(Item::Message(Message::new(Role::Assistant, t, at.clone())));
                        }
                    }
                    Some("thinking") => {
                        let t = b.get("thinking").and_then(Value::as_str).unwrap_or("");
                        if !t.trim().is_empty() {
                            out.push(Item::Message(Message::new(Role::Thinking, t, at.clone())));
                        }
                    }
                    Some("tool_use") => {
                        let name = b.get("name").and_then(Value::as_str).unwrap_or("工具");
                        out.push(Item::Message(Message::tool(
                            name,
                            tool_action(name),
                            &tool_detail(b.get("input")),
                            b.get("id").and_then(Value::as_str).map(str::to_string),
                            at.clone(),
                        )));
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// Claude Code 的掃描器：`root`＝專案資料夾根目錄（預設 [`crate::backfill::projects_root`]）。
#[derive(Debug, Clone)]
pub struct ClaudeScanner {
    pub root: PathBuf,
}

impl ClaudeScanner {
    pub fn new(root: PathBuf) -> Self {
        ClaudeScanner { root }
    }

    /// 名稱合 `roots` 某個 repo（或它的子目錄）的專案資料夾。
    fn dirs_for(&self, roots: &[PathBuf]) -> Vec<PathBuf> {
        let names: Vec<String> = roots.iter().map(|p| project_dir_name(p)).collect();
        std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|d| {
                let dname = d.file_name().to_string_lossy().into_owned();
                names
                    .iter()
                    .any(|n| dname == *n || dname.starts_with(&format!("{n}-")))
            })
            .map(|d| d.path())
            .collect()
    }

    /// 資料夾裡的會話逐字稿（`<合格的 id>.jsonl`）。
    fn transcripts_in(dir: &Path) -> impl Iterator<Item = PathBuf> {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|f| f.path())
            .filter(|fp| {
                fp.extension().and_then(|x| x.to_str()) == Some("jsonl")
                    && fp
                        .file_stem()
                        .is_some_and(|s| crate::history::valid_session_id(&s.to_string_lossy()))
            })
    }

    /// 每個專案資料夾裡的每份 `.jsonl`（回填掃全部、照路徑排序；不看 id 合不合格，對應 Python）。
    pub fn all_files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = Vec::new();
        for proj in std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
        {
            let pp = proj.path();
            if !pp.is_dir() {
                continue;
            }
            for f in std::fs::read_dir(&pp).into_iter().flatten().flatten() {
                let fp = f.path();
                if fp.extension().and_then(|x| x.to_str()) == Some("jsonl") {
                    files.push(fp);
                }
            }
        }
        files.sort();
        files
    }

    /// 讀頭尾組出會話（不管範圍）。沒有 cwd＝None。
    fn read_session(path: &Path, with_tail: bool) -> Option<Session> {
        let id = path.file_stem()?.to_string_lossy().into_owned();
        if !crate::history::valid_session_id(&id) {
            return None;
        }
        let h = head(path);
        let cwd = h.cwd.clone()?;
        let origin = if h
            .entrypoint
            .as_deref()
            .is_some_and(|e| e.starts_with("sdk"))
        {
            Origin::Programmatic
        } else {
            Origin::Interactive
        };
        let t = if with_tail {
            tail(path)
        } else {
            Tail::default()
        };
        let mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or_else(|_| SystemTime::now());
        Some(Session {
            agent: AgentKind::ClaudeCode,
            id,
            cwd,
            started: h.first,
            title: t.title,
            prompt: h.prompt,
            reply: t.reply,
            branch: h.branch,
            origin,
            continued: h.continued,
            mtime,
            path: path.to_path_buf(),
        })
    }
}

impl Scanner for ClaudeScanner {
    fn kind(&self) -> AgentKind {
        AgentKind::ClaudeCode
    }

    fn sessions(&self, roots: &[PathBuf]) -> Vec<Session> {
        let mut out = Vec::new();
        for dir in self.dirs_for(roots) {
            for fp in Self::transcripts_in(&dir) {
                // 先只讀頭：程式化呼叫、空會話、不在 repo 底下的（資料夾名撞到，例：RepoX 與 RepoX-old）不讀尾巴
                let Some(s) = Self::read_session(&fp, false) else {
                    continue;
                };
                if s.origin != Origin::Interactive
                    || (s.prompt.is_none() && !s.continued)
                    || !roots.iter().any(|r| Path::new(&s.cwd).starts_with(r))
                {
                    continue;
                }
                let t = tail(&fp);
                out.push(Session {
                    title: t.title,
                    reply: t.reply,
                    ..s
                });
            }
        }
        out
    }

    fn files(&self, roots: &[PathBuf]) -> Vec<PathBuf> {
        self.dirs_for(roots)
            .iter()
            .flat_map(|d| Self::transcripts_in(d))
            .collect()
    }

    fn probe(&self, path: &Path) -> Option<Session> {
        Self::read_session(path, true)
    }

    /// 先看 cwd 對應的專案資料夾，找不到再掃全部專案資料夾（cwd 是子目錄或搬過時）。
    fn locate(&self, id: &str, cwd: Option<&str>) -> Option<PathBuf> {
        if !crate::history::valid_session_id(id) {
            return None;
        }
        let file = format!("{id}.jsonl");
        if let Some(c) = cwd.filter(|c| !c.is_empty()) {
            let p = self.root.join(project_dir_name(Path::new(c))).join(&file);
            if p.is_file() {
                return Some(p);
            }
        }
        std::fs::read_dir(&self.root)
            .ok()?
            .flatten()
            .map(|e| e.path().join(&file))
            .find(|p| p.is_file())
    }

    fn row_items(&self, row: &Value, out: &mut Vec<Item>) {
        row_items(row, out)
    }
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
