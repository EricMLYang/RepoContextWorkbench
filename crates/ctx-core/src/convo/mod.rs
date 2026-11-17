//! 過去的 agent 會話的正規化層（2026-11-18，F-134；Rust 才有）。
//!
//! 兩家 CLI 各自把會話寫成 JSONL，格式完全不同：Claude Code 在 `~/.claude/projects/<編碼後的 cwd>/<id>.jsonl`
//! （[`claude`]），Codex 在 `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-…-<id>.jsonl`（[`codex`]）。
//! 這裡一家一個**掃描器**（[`Scanner`]），產出同一種 [`Session`] 與 [`Item`]（[`Message`]、工具結果、標題）；
//! 過去的會話清單（[`crate::history`]）、逐字稿時間軸與交接包（[`crate::timeline`]）、回填
//! （[`crate::backfill`]）、全文搜尋（[`index`]）都只面對這一層。
//!
//! 讀訊息一律串流（[`Lines`]）：逐字稿可以到幾十 MB，一次只握一行。
//! 格式不是公開合約：讀不懂的行、欄位一律略過，不報錯。

pub mod claude;
pub mod codex;
pub mod index;

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::config::AgentKind;

/// 會話是誰開的。只有 [`Origin::Interactive`] 列進過去的會話、進索引。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// 人在終端（或 IDE）裡談的。
    Interactive,
    /// 程式化呼叫：`claude -p`（`entrypoint: sdk-*`）、`codex exec`（`source: exec`）。
    Programmatic,
    /// 子 agent（Codex `source: {"subagent": …}`）。
    Subagent,
}

/// 一場會話（兩家共用的欄位）。
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub agent: AgentKind,
    /// 那一家自己的會話 id（接續指令用；已驗過 [`crate::history::valid_session_id`]）。
    pub id: String,
    pub cwd: String,
    /// 開始時間（UTC ISO）。
    pub started: Option<String>,
    /// 那一家取的標題（Claude Code `ai-title`、Codex `session_index.jsonl`）。
    pub title: Option<String>,
    /// 第一句人說的話（原文，未截）。
    pub prompt: Option<String>,
    /// 最後一則回覆（原文，未截）。
    pub reply: Option<String>,
    pub branch: Option<String>,
    pub origin: Origin,
    /// 開頭是上一場的壓縮摘要（這場是接續的；只有 Claude Code 認得出來）。
    pub continued: bool,
    /// 檔案最後修改時間＝最後活動。
    pub mtime: SystemTime,
    pub path: PathBuf,
}

/// 訊息的角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// 人說的話（harness／CLI 自己塞的不算）。
    User,
    /// agent 的回覆文字。
    Assistant,
    /// 思考（Claude Code 的 thinking、Codex 的 reasoning 摘要；加密的內容沒有）。
    Thinking,
    /// 工具呼叫：只有名稱與一行摘要（[`Tool`]）。
    Tool,
    /// 壓縮摘要（context 滿了之後接續的地方）。
    Compact,
}

impl Role {
    /// 索引庫與 JSON 用的名字。
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Thinking => "thinking",
            Role::Tool => "tool",
            Role::Compact => "compact",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "thinking" => Role::Thinking,
            "tool" => Role::Tool,
            "compact" => Role::Compact,
            _ => return None,
        })
    }
}

/// 工具做的是哪一類事（交接包挑「動過的檔」「跑過的指令」用；兩家工具名不同）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolAction {
    /// 跑 shell 指令（摘要＝指令）。
    Shell,
    /// 改檔（摘要＝路徑）。
    Edit,
    #[default]
    Other,
}

/// 一次工具呼叫：名稱、類別、配結果用的 id。輸入輸出整包不留。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tool {
    pub name: String,
    pub action: ToolAction,
    pub call_id: Option<String>,
}

/// 一則訊息。`text`：人說的、回覆、思考是原文；工具是一行摘要（指令、路徑…）。
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: Role,
    /// 逐字稿的時間（UTC ISO）。
    pub at: Option<String>,
    pub text: String,
    /// 只有 [`Role::Tool`] 有。
    pub tool: Option<Tool>,
}

impl Message {
    pub fn new(role: Role, text: &str, at: Option<String>) -> Self {
        Message {
            role,
            at,
            text: text.to_string(),
            tool: None,
        }
    }

    pub fn tool(
        name: &str,
        action: ToolAction,
        summary: &str,
        call_id: Option<String>,
        at: Option<String>,
    ) -> Self {
        Message {
            role: Role::Tool,
            at,
            text: summary.to_string(),
            tool: Some(Tool {
                name: name.to_string(),
                action,
                call_id,
            }),
        }
    }
}

/// 逐字稿裡讀得出來的一件事。
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Message(Message),
    /// 工具結果（配回 `call_id` 那次呼叫；只有時間軸要預覽，索引不存）。
    ToolResult {
        call_id: String,
        text: String,
        error: bool,
    },
    /// 那一家取的（新）標題。
    Title(String),
}

/// 一家的掃描器。實作：[`claude::ClaudeScanner`]、[`codex::CodexScanner`]。
pub trait Scanner: Sync {
    fn kind(&self) -> AgentKind;

    /// `roots`（repo 路徑）底下的互動會話；不在範圍內的盡量只讀頭就跳過。順序不保證。
    fn sessions(&self, roots: &[PathBuf]) -> Vec<Session>;

    /// 可能在 `roots` 底下的逐字稿檔（建索引用；還沒讀內容，範圍與是否互動由 [`Scanner::probe`] 判斷）。
    fn files(&self, roots: &[PathBuf]) -> Vec<PathBuf>;

    /// 讀一份逐字稿的開頭：會話資訊；讀不懂／還沒寫出開頭＝None。
    fn probe(&self, path: &Path) -> Option<Session>;

    /// 依會話 id 找逐字稿（`cwd` 是提示，可能找得快一點）。id 不合格＝None。
    fn locate(&self, id: &str, cwd: Option<&str>) -> Option<PathBuf>;

    /// 一行 JSON → 這一行讀得出的事（照順序推進 `out`）。
    fn row_items(&self, row: &Value, out: &mut Vec<Item>);

    /// 從 `start` 串流讀訊息。
    fn items(&self, path: &Path, start: Start) -> std::io::Result<Items<'_>>
    where
        Self: Sized,
    {
        Items::open(self, path, start)
    }
}

/// 兩家的掃描器（`claude`＝Claude Code 專案資料夾根目錄、`codex`＝`$CODEX_HOME`；None＝不讀那一家）。
pub struct Sources {
    pub claude: Option<claude::ClaudeScanner>,
    pub codex: Option<codex::CodexScanner>,
}

impl Sources {
    /// 預設位置：`$CLAUDE_CONFIG_DIR/projects`（[`crate::backfill::projects_root`]）與 `$CODEX_HOME`。
    pub fn default_homes() -> Self {
        Sources {
            claude: Some(claude::ClaudeScanner::new(crate::backfill::projects_root())),
            codex: Some(codex::CodexScanner::new(codex::codex_home())),
        }
    }

    /// 只讀 Claude Code 的（測試、對拍用暫存目錄時）。
    pub fn claude_only(root: &Path) -> Self {
        Sources {
            claude: Some(claude::ClaudeScanner::new(root.to_path_buf())),
            codex: None,
        }
    }

    pub fn new(claude: Option<&Path>, codex: Option<&Path>) -> Self {
        Sources {
            claude: claude.map(|p| claude::ClaudeScanner::new(p.to_path_buf())),
            codex: codex.map(|p| codex::CodexScanner::new(p.to_path_buf())),
        }
    }

    /// 有開的掃描器。
    pub fn all(&self) -> Vec<&dyn Scanner> {
        let mut v: Vec<&dyn Scanner> = Vec::new();
        if let Some(c) = &self.claude {
            v.push(c);
        }
        if let Some(c) = &self.codex {
            v.push(c);
        }
        v
    }

    pub fn get(&self, kind: AgentKind) -> Option<&dyn Scanner> {
        match kind {
            AgentKind::ClaudeCode => self.claude.as_ref().map(|c| c as &dyn Scanner),
            AgentKind::Codex => self.codex.as_ref().map(|c| c as &dyn Scanner),
            AgentKind::Other => None,
        }
    }

    /// 依 id 找逐字稿：`agent` 給了就只找那一家，否則 Claude Code 先、Codex 後。
    pub fn locate(
        &self,
        id: &str,
        cwd: Option<&str>,
        agent: Option<AgentKind>,
    ) -> Option<(AgentKind, PathBuf)> {
        self.all()
            .into_iter()
            .filter(|s| agent.is_none_or(|a| a == s.kind()))
            .find_map(|s| s.locate(id, cwd).map(|p| (s.kind(), p)))
    }
}

/// 從哪裡開始讀。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// 檔頭。
    Head,
    /// 這個位元組位置（上次讀到的地方，一定是行首）。
    At(u64),
    /// 只讀最後這麼多位元組（起點不在檔頭時丟掉第一個半行）。
    Tail(u64),
}

/// 串流讀 JSONL：一次一行，只握住目前這一行。`complete_only`：沒有換行結尾的最後一行（還在寫）不讀，
/// [`Lines::pos`] 停在它之前（下次增量從這裡接）。
pub struct Lines {
    rd: BufReader<std::fs::File>,
    pos: u64,
    buf: Vec<u8>,
    complete_only: bool,
    /// 開始讀的位置不在檔頭（丟了第一個半行）。
    pub cut: bool,
    peak: usize,
}

impl Lines {
    pub fn open(path: &Path, start: Start, complete_only: bool) -> std::io::Result<Self> {
        let mut f = std::fs::File::open(path)?;
        let size = f.metadata()?.len();
        let (from, skip_partial) = match start {
            Start::Head => (0, false),
            Start::At(o) => (o.min(size), false),
            Start::Tail(n) => {
                let o = size.saturating_sub(n);
                (o, o > 0)
            }
        };
        f.seek(SeekFrom::Start(from))?;
        let mut me = Lines {
            rd: BufReader::with_capacity(64 * 1024, f),
            pos: from,
            buf: Vec::new(),
            complete_only,
            cut: skip_partial,
            peak: 0,
        };
        if skip_partial {
            let mut sink = Vec::new();
            let n = me.rd.read_until(b'\n', &mut sink)?;
            me.pos += n as u64;
        }
        Ok(me)
    }

    /// 讀完的位置（下一行的開頭）。
    pub fn pos(&self) -> u64 {
        self.pos
    }

    /// 到目前為止最長的一行佔了多少位元組（測試用來確認是串流讀）。
    pub fn peak_line(&self) -> usize {
        self.peak
    }

    /// 下一行（不含換行）；讀完或讀不了＝None。
    pub fn next_line(&mut self) -> Option<&[u8]> {
        self.buf.clear();
        let n = self.rd.read_until(b'\n', &mut self.buf).ok()?;
        if n == 0 {
            return None;
        }
        let ended = self.buf.last() == Some(&b'\n');
        if !ended && self.complete_only {
            return None;
        }
        self.pos += n as u64;
        self.peak = self.peak.max(self.buf.len());
        if ended {
            self.buf.pop();
            if self.buf.last() == Some(&b'\r') {
                self.buf.pop();
            }
        }
        Some(&self.buf)
    }

    /// 下一個解得開的 JSON 行（解不開的略過）。
    pub fn next_value(&mut self) -> Option<Value> {
        loop {
            let line = self.next_line()?;
            if let Ok(v) = serde_json::from_slice::<Value>(line) {
                return Some(v);
            }
            // UTF-8 壞掉的行：跟以前 `from_utf8_lossy` 再解析一樣，換掉壞位元組再試一次
            if std::str::from_utf8(line).is_err() {
                let s = String::from_utf8_lossy(line).into_owned();
                if let Ok(v) = serde_json::from_str::<Value>(&s) {
                    return Some(v);
                }
            }
        }
    }
}

/// 串流讀出的事：每一項帶「讀完這一行之後的位置」。
pub struct Items<'a> {
    scanner: &'a dyn Scanner,
    pub lines: Lines,
    pending: VecDeque<Item>,
    scratch: Vec<Item>,
}

impl<'a> Items<'a> {
    /// 給 `&dyn Scanner` 用（trait 物件呼叫不到預設方法 [`Scanner::items`]）。
    pub fn open(scanner: &'a dyn Scanner, path: &Path, start: Start) -> std::io::Result<Self> {
        Self::open_with(scanner, path, start, false)
    }

    /// `complete_only`：見 [`Lines`]（增量索引用 true）。
    pub fn open_with(
        scanner: &'a dyn Scanner,
        path: &Path,
        start: Start,
        complete_only: bool,
    ) -> std::io::Result<Self> {
        Ok(Items {
            scanner,
            lines: Lines::open(path, start, complete_only)?,
            pending: VecDeque::new(),
            scratch: Vec::new(),
        })
    }
}

impl Iterator for Items<'_> {
    type Item = Item;

    fn next(&mut self) -> Option<Item> {
        loop {
            if let Some(it) = self.pending.pop_front() {
                return Some(it);
            }
            let v = self.lines.next_value()?;
            self.scratch.clear();
            self.scanner.row_items(&v, &mut self.scratch);
            self.pending.extend(self.scratch.drain(..));
        }
    }
}

/// 一則工具呼叫配上結果之後（時間軸用）。
#[derive(Debug, Clone, PartialEq)]
pub struct Paired {
    pub message: Message,
    pub result: Option<String>,
    pub error: bool,
}

/// 把事件流配好：工具結果配回它的呼叫（`call_id`；同一個 id 有好幾則時配最後一則），標題取最後一個。回傳 (訊息, 標題)。
pub fn pair(items: impl Iterator<Item = Item>) -> (Vec<Paired>, Option<String>) {
    let mut out: Vec<Paired> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    let mut title = None;
    for it in items {
        match it {
            Item::Title(t) => title = Some(t),
            Item::Message(m) => {
                if let Some(id) = m.tool.as_ref().and_then(|t| t.call_id.clone()) {
                    at.insert(id, out.len());
                }
                out.push(Paired {
                    message: m,
                    result: None,
                    error: false,
                });
            }
            Item::ToolResult {
                call_id,
                text,
                error,
            } => {
                if let Some(&i) = at.get(&call_id) {
                    out[i].result = Some(text);
                    out[i].error = error;
                }
            }
        }
    }
    (out, title)
}

/// cwd 在哪個 repo 底下（`targets` 要路徑長的在前：巢狀 repo 歸最深的那個）。
pub fn repo_of<'a>(targets: &'a [(String, PathBuf)], cwd: &str) -> Option<&'a str> {
    let c = Path::new(cwd);
    targets
        .iter()
        .find(|(_, p)| c.starts_with(p))
        .map(|(id, _)| id.as_str())
}

/// (repo id, 路徑)，路徑長的在前。
pub fn targets_of(entries: &[crate::registry::Repo]) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = entries.iter().map(|e| (e.id(), e.path())).collect();
    v.sort_by_key(|(_, p)| std::cmp::Reverse(p.as_os_str().len()));
    v
}
