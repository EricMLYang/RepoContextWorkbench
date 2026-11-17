//! Codex 的掃描器（2026-11-14 起列進過去的會話；2026-11-18 F-134 收進 [`super`] 的正規化層，訊息也讀得出來）。
//!
//! 紀錄在 `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<時間>-<id>.jsonl`（預設 `~/.codex`），不像 Claude Code
//! 依專案分資料夾，所以要看每份的第一行才知道是哪個目錄的會話：
//! - 第一行 `session_meta`：`id`、`cwd`、`timestamp`、`source`（`exec`＝`codex exec` 程式化呼叫、物件＝子 agent，
//!   都不是人談過的會話，不列）、`git.branch`。這一行含整份系統指示，實測約 20 KB，所以整行讀、上限 [`META_MAX`]。
//! - 第一句話：第一則 `response_item`／`message`／`role: user` 裡不是 Codex 自己塞的那些
//!   （`<environment_context>`、`# AGENTS.md instructions…`）。
//! - 最後回覆：尾巴最後一筆 `event_msg`／`task_complete` 的 `last_agent_message`。
//! - 標題：`$CODEX_HOME/session_index.jsonl`（`id` → `thread_name`）；沒有就用第一句話。
//!
//! 訊息（[`row_items`]）只看 `response_item`——Codex 的 rollout 政策（`codex-rs/rollout/src/policy.rs`）
//! 兩種歷史模式都一定存它；`event_msg` 的 `user_message`／`agent_message`／`item_completed` 是同一件事的另一份，不讀以免重複：
//! - `message`：`role: user` 的 `input_text`（Codex 塞的不算）＝你說的；`role: assistant` 的 `output_text`＝回覆；
//!   `developer` 不算。
//! - `reasoning`：只有 `summary` 有字時算思考（內容是加密的）。
//! - 工具：`function_call`（`name`＋JSON `arguments`）、`custom_tool_call`（0.169 幾乎都是 `exec`：一段 JS，
//!   裡面 `tools.exec_command({cmd:"…"})`、`tools.apply_patch("*** Begin Patch…")`，一個 `tools.X(` 算一次呼叫）、
//!   `local_shell_call`、`web_search_call`；`*_output` 是結果（配 `call_id`）。
//! - `compacted`（頂層類型）＝壓縮摘要。
//!
//! 過去的會話清單只看最近 [`MAX_FILES`] 份（檔名帶時間，新的在前）：不在範圍內的只讀第一行就跳過。
//! 格式是照 Codex 0.163–0.169 的實際紀錄寫的，不是公開合約——讀不懂的檔、欄位一律略過，不報錯。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, OnceLock};
use std::time::SystemTime;

use regex::Regex;
use serde_json::Value;

use super::claude::one_line;
use super::{Item, Message, Origin, Role, Scanner, Session, ToolAction};
use crate::config::AgentKind;

/// 第一行（session_meta）最多讀多少。
const META_MAX: u64 = 1024 * 1024;
/// 找第一句話最多往下讀多少（開頭是系統指示、工具清單、AGENTS.md，實測幾十 KB）。
const HEAD_MAX: u64 = 1024 * 1024;
/// 找最後回覆讀尾巴多少。
const TAIL_MAX: u64 = 256 * 1024;
/// 過去的會話清單最多看幾份紀錄（最近的）。
pub const MAX_FILES: usize = 400;

/// `$CODEX_HOME`（沒設＝`~/.codex`）。
pub fn codex_home() -> PathBuf {
    crate::agentsetup::codex_home()
}

fn sorted_desc(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    v.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    v
}

/// `sessions` 底下的紀錄檔，新的在前，最多 `max` 份。目錄與檔名都帶時間，照名字倒著排就是時間序。
fn rollouts(sessions: &Path, max: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for year in sorted_desc(sessions) {
        for month in sorted_desc(&year) {
            for day in sorted_desc(&month) {
                for f in sorted_desc(&day) {
                    let name = f.file_name().unwrap_or_default().to_string_lossy();
                    if name.starts_with("rollout-") && name.ends_with(".jsonl") {
                        out.push(f.clone());
                        if out.len() >= max {
                            return out;
                        }
                    }
                }
            }
        }
    }
    out
}

/// 一則 user 訊息的文字是不是 Codex 自己塞的（環境說明、AGENTS.md…），不是人說的話。
fn injected(text: &str) -> bool {
    let t = text.trim_start();
    t.is_empty() || t.starts_with('<') || t.starts_with("# AGENTS.md instructions")
}

fn message_text(payload: &Value, kind: &str) -> Option<String> {
    payload["content"].as_array()?.iter().find_map(|c| {
        (c["type"] == kind)
            .then(|| c["text"].as_str())
            .flatten()
            .map(str::to_string)
    })
}

/// 第一行（session_meta）讀得出的：id、cwd、開始時間、分支、誰開的。
struct MetaLine {
    id: String,
    cwd: String,
    started: Option<String>,
    branch: Option<String>,
    origin: Origin,
}

fn read_meta(line: &str) -> Option<MetaLine> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v["type"] != "session_meta" {
        return None;
    }
    let p = &v["payload"];
    // source：`cli`／`vscode`…＝互動；`exec`＝codex exec；物件（{"subagent": …}）＝子 agent
    let origin = if p["source"].is_object() {
        Origin::Subagent
    } else if p["source"] == "exec" {
        Origin::Programmatic
    } else {
        Origin::Interactive
    };
    let id = p["id"]
        .as_str()
        .filter(|i| crate::history::valid_session_id(i))?;
    let cwd = p["cwd"].as_str().filter(|c| !c.is_empty())?;
    Some(MetaLine {
        id: id.to_string(),
        cwd: cwd.to_string(),
        started: p["timestamp"].as_str().map(str::to_string),
        branch: p["git"]["branch"].as_str().map(str::to_string),
        origin,
    })
}

/// 尾巴最後一則回覆。
fn last_reply(path: &Path, len: u64) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let start = len.saturating_sub(TAIL_MAX);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.take(TAIL_MAX).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.lines();
    if start > 0 {
        lines.next(); // 第一行可能被切一半
    }
    let mut done = None;
    let mut said = None;
    for l in lines {
        let Ok(v) = serde_json::from_str::<Value>(l) else {
            continue;
        };
        let p = &v["payload"];
        if v["type"] == "event_msg" && p["type"] == "task_complete" {
            if let Some(m) = p["last_agent_message"]
                .as_str()
                .filter(|m| !m.trim().is_empty())
            {
                done = Some(m.to_string());
            }
        } else if v["type"] == "response_item" && p["type"] == "message" && p["role"] == "assistant"
        {
            if let Some(m) = message_text(p, "output_text").filter(|m| !m.trim().is_empty()) {
                said = Some(m);
            }
        }
    }
    done.or(said)
}

/// `session_index.jsonl`：id → Codex 取的標題（同一個 id 後面的蓋前面的）。
fn thread_names(home: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(text) = std::fs::read_to_string(home.join("session_index.jsonl")) else {
        return out;
    };
    for l in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(l) else {
            continue;
        };
        if let (Some(id), Some(name)) = (v["id"].as_str(), v["thread_name"].as_str()) {
            if !name.trim().is_empty() {
                out.insert(id.to_string(), name.trim().to_string());
            }
        }
    }
    out
}

/// 讀一份紀錄的開頭；`wanted`＝這個 cwd 要不要（不要的只讀第一行）。`interactive_only`：程式化呼叫與子 agent＝None。
fn read_head(
    path: &Path,
    wanted: &dyn Fn(&str) -> bool,
    interactive_only: bool,
) -> Option<Session> {
    let meta = std::fs::metadata(path).ok()?;
    let mut rd = BufReader::new(std::fs::File::open(path).ok()?.take(META_MAX + HEAD_MAX));
    let mut first = String::new();
    rd.read_line(&mut first).ok()?;
    let MetaLine {
        id,
        cwd,
        started,
        branch,
        origin,
    } = read_meta(&first)?;
    if (interactive_only && origin != Origin::Interactive) || !wanted(&cwd) {
        return None;
    }
    let mut prompt = None;
    let mut line = String::new();
    loop {
        line.clear();
        match rd.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        // 便宜的預篩：不是 user 訊息就不解析（開頭幾行是幾十 KB 的系統指示）
        if !line.contains("\"role\":\"user\"") && !line.contains("\"role\": \"user\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let p = &v["payload"];
        if v["type"] == "response_item" && p["type"] == "message" && p["role"] == "user" {
            if let Some(t) = message_text(p, "input_text").filter(|t| !injected(t)) {
                prompt = Some(t);
                break;
            }
        }
    }
    Some(Session {
        agent: AgentKind::Codex,
        id,
        cwd,
        started,
        title: None,
        prompt,
        reply: last_reply(path, meta.len()),
        branch,
        origin,
        continued: false,
        mtime: meta.modified().unwrap_or_else(|_| SystemTime::now()),
        path: path.to_path_buf(),
    })
}

// ── 訊息 ────────────────────────────────────────────────────────────────

/// `exec` 那段 JS 裡的一次工具呼叫：`tools.<名>(`。
static TOOL_CALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"tools\.([A-Za-z_][A-Za-z0-9_]*)\(").expect("固定 regex"));
/// `cmd:"…"`／`"cmd": "…"`／``cmd:`…` ``（JS 物件裡的指令字串）。
static CMD_ARG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)["']?(?:cmd|command)["']?\s*:\s*(?:"((?:[^"\\]|\\.)*)"|`((?:[^`\\]|\\.)*)`|'((?:[^'\\]|\\.)*)')"#)
        .expect("固定 regex")
});
/// patch 裡動到的檔（字串裡的換行可能是跳脫過的 `\n`）。
static PATCH_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\*\*\* (?:Update|Add|Delete) File: ([^\n\\\x22`]+)").expect("固定 regex")
});
/// 指令輸出裡的非零結束碼。
static EXIT_CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:exit(?:ed with)? code:?|\x22exit_code\x22\s*:)\s*(-?\d+)")
        .expect("固定 regex")
});

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

fn is_shell(name: &str) -> bool {
    matches!(
        name,
        "exec_command" | "shell" | "shell_command" | "local_shell" | "container.exec"
    )
}

/// 一段工具輸入 → 這次呼叫做了什麼（一行）。改檔的一個檔一則。
fn calls_in(name: &str, input: &str) -> Vec<(String, ToolAction, String)> {
    if name == "apply_patch" {
        let files = patch_files(input);
        if !files.is_empty() {
            return files
                .into_iter()
                .map(|f| (name.to_string(), ToolAction::Edit, f))
                .collect();
        }
    }
    if is_shell(name) {
        if let Some(c) = CMD_ARG.captures(input) {
            let raw = c
                .get(1)
                .or(c.get(2))
                .or(c.get(3))
                .map_or("", |m| m.as_str());
            return vec![(
                name.to_string(),
                ToolAction::Shell,
                one_line(&unescape(raw), 300),
            )];
        }
    }
    vec![(name.to_string(), ToolAction::Other, one_line(input, 160))]
}

fn patch_files(input: &str) -> Vec<String> {
    PATCH_FILE
        .captures_iter(input)
        .map(|c| c[1].trim().to_string())
        .filter(|f| !f.is_empty())
        .collect()
}

/// `custom_tool_call` 的 `exec`：JS 裡每個 `tools.X(` 一次；一個都沒有就整段算一次 `exec`。
fn exec_calls(input: &str) -> Vec<(String, ToolAction, String)> {
    let found: Vec<(usize, usize, String)> = TOOL_CALL
        .captures_iter(input)
        .filter_map(|c| {
            let m = c.get(0)?;
            Some((m.start(), m.end(), c[1].to_string()))
        })
        .collect();
    if found.is_empty() {
        return vec![("exec".into(), ToolAction::Other, one_line(input, 300))];
    }
    let mut out = Vec::new();
    for (i, (_, end, name)) in found.iter().enumerate() {
        let stop = found.get(i + 1).map_or(input.len(), |n| n.0);
        out.extend(calls_in(name, &input[*end..stop]));
    }
    out
}

/// `function_call` 的 JSON 參數 → 一行。
fn function_calls(name: &str, args: &str) -> Vec<(String, ToolAction, String)> {
    let Ok(v) = serde_json::from_str::<Value>(args) else {
        return calls_in(name, args);
    };
    let cmd = match &v["command"] {
        // `["bash", "-lc", "<指令>"]`：最後一段才是人看得懂的
        Value::Array(a) => {
            let parts: Vec<&str> = a.iter().filter_map(Value::as_str).collect();
            match parts.as_slice() {
                [sh, flag, rest] if sh.ends_with("sh") && flag.starts_with('-') => {
                    Some(rest.to_string())
                }
                _ => Some(parts.join(" ")),
            }
        }
        Value::String(s) => Some(s.clone()),
        _ => v["cmd"].as_str().map(str::to_string),
    };
    if let Some(c) = cmd.filter(|_| is_shell(name) || name == "exec") {
        return vec![(name.to_string(), ToolAction::Shell, one_line(&c, 300))];
    }
    if name == "apply_patch" {
        if let Some(i) = v["input"].as_str().or(v["patch"].as_str()) {
            return calls_in(name, i);
        }
    }
    let detail = [
        "path",
        "file_path",
        "query",
        "url",
        "message",
        "prompt",
        "task_name",
    ]
    .iter()
    .find_map(|k| v[*k].as_str())
    .map(|s| one_line(s, 300))
    .unwrap_or_default();
    vec![(name.to_string(), ToolAction::Other, detail)]
}

fn output_text(v: &Value) -> String {
    match v {
        Value::String(s) => {
            // 舊版：`{"output": "…", "metadata": {"exit_code": 0}}` 包成字串
            match serde_json::from_str::<Value>(s) {
                Ok(Value::Object(o)) if o.contains_key("output") => {
                    o["output"].as_str().unwrap_or_default().to_string()
                }
                _ => s.clone(),
            }
        }
        Value::Array(a) => a
            .iter()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(o) => o
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

fn output_failed(v: &Value, text: &str) -> bool {
    if let Value::String(s) = v {
        if let Ok(o) = serde_json::from_str::<Value>(s) {
            if let Some(c) = o["metadata"]["exit_code"].as_i64() {
                return c != 0;
            }
        }
    }
    EXIT_CODE
        .captures(text)
        .and_then(|c| c[1].parse::<i64>().ok())
        .is_some_and(|c| c != 0)
}

/// rollout 一行 → 事件。
pub fn row_items(o: &Value, out: &mut Vec<Item>) {
    let at = o["timestamp"].as_str().map(str::to_string);
    let p = &o["payload"];
    match o["type"].as_str() {
        Some("compacted") => {
            if let Some(m) = p["message"].as_str().filter(|m| !m.trim().is_empty()) {
                out.push(Item::Message(Message::new(Role::Compact, m.trim(), at)));
            }
            return;
        }
        Some("response_item") => {}
        _ => return,
    }
    match p["type"].as_str() {
        Some("message") => {
            let (role, kind) = match p["role"].as_str() {
                Some("user") => (Role::User, "input_text"),
                Some("assistant") => (Role::Assistant, "output_text"),
                _ => return,
            };
            let Some(parts) = p["content"].as_array() else {
                return;
            };
            let text: Vec<&str> = parts
                .iter()
                .filter(|c| c["type"] == kind)
                .filter_map(|c| c["text"].as_str())
                .filter(|t| role != Role::User || !injected(t))
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .collect();
            if !text.is_empty() {
                out.push(Item::Message(Message::new(role, &text.join("\n"), at)));
            }
        }
        Some("reasoning") => {
            let text: Vec<&str> = p["summary"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s["text"].as_str())
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .collect();
            if !text.is_empty() {
                out.push(Item::Message(Message::new(
                    Role::Thinking,
                    &text.join("\n"),
                    at,
                )));
            }
        }
        Some(
            kind @ ("function_call" | "custom_tool_call" | "local_shell_call" | "web_search_call"),
        ) => {
            let name = p["name"].as_str().unwrap_or(match kind {
                "local_shell_call" => "local_shell",
                "web_search_call" => "web_search",
                _ => "工具",
            });
            let calls = match kind {
                "function_call" => function_calls(name, p["arguments"].as_str().unwrap_or("")),
                "custom_tool_call" if name == "exec" => {
                    exec_calls(p["input"].as_str().unwrap_or(""))
                }
                "custom_tool_call" => calls_in(name, p["input"].as_str().unwrap_or("")),
                "local_shell_call" => {
                    let args = serde_json::json!({"command": p["action"]["command"]}).to_string();
                    function_calls("local_shell", &args)
                }
                _ => vec![(
                    name.to_string(),
                    ToolAction::Other,
                    one_line(p["action"]["query"].as_str().unwrap_or(""), 300),
                )],
            };
            let call_id = p["call_id"]
                .as_str()
                .or(p["id"].as_str())
                .map(str::to_string);
            for (name, action, summary) in calls {
                out.push(Item::Message(Message::tool(
                    &name,
                    action,
                    &summary,
                    call_id.clone(),
                    at.clone(),
                )));
            }
        }
        Some("function_call_output" | "custom_tool_call_output" | "local_shell_call_output") => {
            if let Some(id) = p["call_id"].as_str() {
                let text = output_text(&p["output"]);
                out.push(Item::ToolResult {
                    call_id: id.to_string(),
                    error: output_failed(&p["output"], &text),
                    text,
                });
            }
        }
        _ => {}
    }
}

/// Codex 的掃描器：`home`＝`$CODEX_HOME`。
#[derive(Debug)]
pub struct CodexScanner {
    pub home: PathBuf,
    names: OnceLock<HashMap<String, String>>,
}

impl CodexScanner {
    pub fn new(home: PathBuf) -> Self {
        CodexScanner {
            home,
            names: OnceLock::new(),
        }
    }

    /// Codex 取的標題（`session_index.jsonl`，這個掃描器第一次用到時讀一次）。
    pub fn title_of(&self, id: &str) -> Option<String> {
        self.names
            .get_or_init(|| thread_names(&self.home))
            .get(id)
            .cloned()
    }

    /// 全部的紀錄檔（新的在前；建索引、回填用）。
    pub fn all_files(&self) -> Vec<PathBuf> {
        rollouts(&self.home.join("sessions"), usize::MAX)
    }

    /// 讀一份的開頭（不管範圍、不管是不是互動）。回填用。
    pub fn head_of(&self, path: &Path) -> Option<Session> {
        let mut s = read_head(path, &|_| true, false)?;
        s.title = self.title_of(&s.id);
        Some(s)
    }
}

impl Scanner for CodexScanner {
    fn kind(&self) -> AgentKind {
        AgentKind::Codex
    }

    fn sessions(&self, roots: &[PathBuf]) -> Vec<Session> {
        let wanted = |cwd: &str| roots.iter().any(|r| Path::new(cwd).starts_with(r));
        rollouts(&self.home.join("sessions"), MAX_FILES)
            .iter()
            .filter_map(|f| read_head(f, &wanted, true))
            .filter(|s| s.prompt.is_some()) // 只有 meta、沒人說過話
            .map(|mut s| {
                s.title = self.title_of(&s.id);
                s
            })
            .collect()
    }

    fn files(&self, _roots: &[PathBuf]) -> Vec<PathBuf> {
        self.all_files()
    }

    fn probe(&self, path: &Path) -> Option<Session> {
        self.head_of(path)
    }

    /// 檔名尾巴就是 id。
    fn locate(&self, id: &str, _cwd: Option<&str>) -> Option<PathBuf> {
        if !crate::history::valid_session_id(id) {
            return None;
        }
        let tail = format!("-{id}.jsonl");
        self.all_files().into_iter().find(|f| {
            f.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(&tail))
        })
    }

    fn row_items(&self, row: &Value, out: &mut Vec<Item>) {
        row_items(row, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_rollout(home: &Path, day: &str, stamp: &str, id: &str, lines: &[Value]) -> PathBuf {
        let dir = home.join("sessions").join(day);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("rollout-{stamp}-{id}.jsonl"));
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(&p, text).unwrap();
        p
    }

    fn meta(id: &str, cwd: &str, source: Value) -> Value {
        json!({"type": "session_meta", "timestamp": "2026-11-14T12:27:19.843Z", "payload": {
            "id": id, "cwd": cwd, "timestamp": "2026-11-14T12:27:19.759Z", "source": source,
            "originator": "codex-tui", "base_instructions": {"text": "x".repeat(30_000)},
            "git": {"branch": "main"}}})
    }

    fn user(text: &str) -> Value {
        json!({"type": "response_item", "payload": {"type": "message", "role": "user",
            "content": [{"type": "input_text", "text": text}]}})
    }

    fn done(text: &str) -> Value {
        json!({"type": "event_msg", "payload": {"type": "task_complete", "last_agent_message": text}})
    }

    const A: &str = "01a1da06-c73f-79b3-bb63-a14cb2e0b2b6";
    const B: &str = "01a1da14-2615-7571-a7d4-2e6da4de1c9f";
    const C: &str = "01a1da12-c4bb-73c0-b55a-7f7b063f20f5";

    #[test]
    fn reads_interactive_sessions_and_skips_exec_and_other_dirs() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        write_rollout(
            home,
            "2026/11/14",
            "2026-11-14T20-27-19",
            A,
            &[
                meta(A, "/r/app", json!("vscode")),
                user("<environment_context>\n  <cwd>/r/app</cwd>"),
                user("# AGENTS.md instructions for /r/app\n…"),
                user("你可以知道現在多少 repo 嗎\n第二行"),
                done("先查一下"),
                done("共有 **35 個 repo**"),
            ],
        );
        // codex exec（程式化呼叫）與子 agent 不列；別的目錄不列
        write_rollout(
            home,
            "2026/11/14",
            "2026-11-14T20-41-56",
            B,
            &[meta(B, "/r/app", json!("exec")), user("hi"), done("ok")],
        );
        write_rollout(
            home,
            "2026/11/13",
            "2026-11-13T09-00-00",
            C,
            &[meta(C, "/elsewhere", json!("cli")), user("hi"), done("ok")],
        );
        std::fs::write(
            home.join("session_index.jsonl"),
            format!(
                "{}\n",
                json!({"id": A, "thread_name": "查看目前 repo 數量"})
            ),
        )
        .unwrap();
        let sc = CodexScanner::new(home.to_path_buf());
        let got = sc.sessions(&[PathBuf::from("/r/app")]);
        assert_eq!(got.len(), 1);
        let s = &got[0];
        assert_eq!(s.id, A);
        assert_eq!(s.title.as_deref(), Some("查看目前 repo 數量"));
        assert_eq!(
            s.prompt.as_deref(),
            Some("你可以知道現在多少 repo 嗎\n第二行")
        );
        assert_eq!(s.reply.as_deref(), Some("共有 **35 個 repo**"));
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(sc.locate(A, None).as_deref(), Some(s.path.as_path()));
        assert!(sc.locate("nope-nope", None).is_none());
    }

    #[test]
    fn missing_home_and_empty_sessions_are_quietly_empty() {
        let t = tempfile::tempdir().unwrap();
        let all = [PathBuf::from("/")];
        assert!(CodexScanner::new(t.path().join("nope"))
            .sessions(&all)
            .is_empty());
        // 只有 meta、沒人說過話
        write_rollout(
            t.path(),
            "2026/11/14",
            "2026-11-14T10-00-00",
            A,
            &[meta(A, "/r/app", json!("cli"))],
        );
        assert!(CodexScanner::new(t.path().to_path_buf())
            .sessions(&all)
            .is_empty());
    }
}
