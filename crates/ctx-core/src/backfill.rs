//! 回填（2026-09-26）——從過去的 Claude Code 會話紀錄，把判斷／未結／交接撈回 spine。
//! 對應 `repo_context/backfill.py`。
//!
//! 新 spine 是空的：沒有判斷、沒有未結、沒有交接 → agent 開場接不上、反芻沒有「手上工作」可比。
//! 但使用者跟 agent 過去的會話都在 `~/.claude/projects/*/<session>.jsonl`，判斷與沒做完的事其實都在裡面。
//!
//! 三段，每段都可中斷續跑（結果落 `.state/backfill/`，已做過的不重做）：
//! 1. scan：會話 → 依紀錄裡的 cwd 對到已登記 repo（Google Drive 舊路徑依資料夾名對）；
//!    正在寫的會話（30 分鐘內有更新）與對不到 repo 的略過。
//! 2. extract：會話壓成精簡文字（使用者原話＋agent 的文字＋工具一行），切段，每段一次模型呼叫，
//!    抽出判斷／待辦／完成／下一步／目標線索（都要標第幾輪，說得出出處）。
//! 3. consolidate：同一 repo 的所有會話依時間排好再彙整一次——後來推翻的判斷、做完的待辦剔掉。
//!
//! **不直接寫進 spine**：結果是候選層，review 給人看、accept 才收。收進去的事件 source 是 `backfill`、
//! 時間是收下的當下，內文註明回填自哪天哪場會話——不假裝是當時寫的。目標只由使用者給：
//! goal_hints 只列出來參考，不代寫進 registry。
//!
//! `claude -p` 在 spine 目錄跑（未登記目錄＝hook 都靜默），它自己產生的會話紀錄落在 spine 的
//! project 資料夾，對不到 repo，不會被回填回來。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use chrono::Local;
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::sync::LazyLock;

use crate::agentapi::{self, Target};
use crate::agents::{AgentProvider, ClaudeProvider};
use crate::fsutil::{expanduser, mtime_secs, now_secs, resolve as py_resolve};
use crate::pystr::py_str;
use crate::{registry, Error, Result};

/// 對應 `CHUNK_CHARS`。
pub const CHUNK_CHARS: usize = 40_000;
const MSG_CHARS: usize = 1_500;
const LIVE_MINUTES: f64 = 30.0;

/// Python 用 `<(tag1|tag2…)>.*?</\1>`（反向參照）；Rust regex 沒有反向參照，逐一展開成等價的交替。
static NOISE: LazyLock<Regex> = LazyLock::new(|| {
    let tags = [
        "system-reminder",
        "local-command-caveat",
        "local-command-stdout",
        "command-name",
        "command-message",
        "command-args",
        "persisted-output",
    ];
    let alts: Vec<String> = tags.iter().map(|t| format!("<{t}>.*?</{t}>")).collect();
    Regex::new(&format!("(?s){}", alts.join("|"))).expect("固定 regex")
});

/// 模型呼叫：prompt → 文字輸出（可並行，所以要 Sync）。測試注入假的。
pub type Call<'a> = dyn Fn(&str) -> Result<String> + Sync + 'a;
/// 進度輸出（Python `log=print`）。
pub type Log<'a> = dyn Fn(&str) + Sync + 'a;

/// 對應 `backfill.projects_root`：`$CLAUDE_CONFIG_DIR`（或 `~/.claude`）底下的 `projects`。
pub fn projects_root() -> PathBuf {
    let base = match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v),
        None => expanduser("~/.claude"),
    };
    base.join("projects")
}

fn dir(spine_dir: &Path, parts: &[&str]) -> Result<PathBuf> {
    let mut p = spine_dir.join(".state").join("backfill");
    for x in parts {
        p.push(x);
    }
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

fn load(p: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

fn truthy(v: Option<&Value>) -> bool {
    v.is_some_and(crate::pystr::truthy)
}

fn save(p: &Path, obj: &Value) -> Result<()> {
    let tmp = PathBuf::from(format!("{}.tmp", p.display()));
    // 同 Python json.dumps(ensure_ascii=False, indent=1)
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    obj.serialize(&mut ser)?;
    std::fs::write(&tmp, buf)?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

fn json_files(d: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(d)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn take(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

// ---------------------------------------------------------------- 1. scan

/// 對應 `_repo_matcher`：cwd → repo id。先比已登記路徑（含子目錄），再比路徑上任一段資料夾名
/// ＝repo 資料夾名或 id（Drive 舊路徑）。
struct Matcher {
    entries: Vec<(String, PathBuf)>,
    by_name: HashMap<String, String>,
}

impl Matcher {
    fn new(spine_dir: &Path) -> Result<Self> {
        let mut entries: Vec<(String, PathBuf)> = registry::load(spine_dir)?
            .repos
            .iter()
            .map(|e| (e.id(), py_resolve(&expanduser(e.path_str()))))
            .collect();
        let mut by_name = HashMap::new();
        for (rid, p) in &entries {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            by_name.entry(name).or_insert_with(|| rid.clone());
            by_name
                .entry(rid.to_lowercase())
                .or_insert_with(|| rid.clone());
        }
        // 最長路徑先比（穩定排序，同 Python sorted(key=-len)）
        entries.sort_by_key(|(_, p)| std::cmp::Reverse(p.to_string_lossy().len()));
        Ok(Matcher { entries, by_name })
    }

    fn matches(&self, cwd: Option<&str>) -> Option<String> {
        let cwd = cwd.filter(|c| !c.is_empty())?;
        let c = Path::new(cwd);
        for (rid, p) in &self.entries {
            if c.starts_with(p) {
                return Some(rid.clone());
            }
        }
        // 最深的一段先比：Drive/舊工作區/projects/<repo> → <repo>
        let parts: Vec<String> = c
            .components()
            .map(|x| x.as_os_str().to_string_lossy().to_lowercase())
            .collect();
        parts
            .iter()
            .rev()
            .find_map(|part| self.by_name.get(part).cloned())
    }
}

/// 會話的 metadata（對應 `_meta`）。
#[derive(Debug, Clone, Default)]
struct Meta {
    cwd: Option<String>,
    first: Option<String>,
    last: Option<String>,
    title: Option<String>,
    user_turns: usize,
}

fn jsonl_rows(path: &Path) -> Result<Vec<Value>> {
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(text
        .lines()
        .filter_map(|ln| serde_json::from_str::<Value>(ln).ok())
        .collect())
}

fn meta(path: &Path) -> Result<Meta> {
    let mut cwds: Vec<(String, usize)> = Vec::new();
    let mut m = Meta::default();
    for o in jsonl_rows(path)? {
        if let Some(c) = o.get("cwd").filter(|c| truthy(Some(c))) {
            let c = py_str(c);
            match cwds.iter_mut().find(|(k, _)| *k == c) {
                Some(slot) => slot.1 += 1,
                None => cwds.push((c, 1)),
            }
        }
        if let Some(ts) = o.get("timestamp").filter(|t| truthy(Some(t))) {
            let ts = py_str(ts);
            if m.first.is_none() {
                m.first = Some(ts.clone());
            }
            m.last = Some(ts);
        }
        if o.get("type").and_then(Value::as_str) == Some("ai-title") && truthy(o.get("aiTitle")) {
            m.title = Some(py_str(&o["aiTitle"]));
        }
        if o.get("type").and_then(Value::as_str) == Some("user")
            && o.get("message")
                .and_then(|x| x.get("content"))
                .is_some_and(Value::is_string)
        {
            m.user_turns += 1;
        }
    }
    // Counter.most_common(1)：次數最多、同次數取先出現的
    let mut best: Option<&(String, usize)> = None;
    for e in &cwds {
        if best.is_none_or(|b| e.1 > b.1) {
            best = Some(e);
        }
    }
    m.cwd = best.map(|b| b.0.clone());
    Ok(m)
}

/// scan 的一筆（對應 Python dict：sid, path, repo, cwd, first, last, title, user_turns, status）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionRec {
    pub sid: String,
    pub path: String,
    pub repo: Option<String>,
    pub cwd: Option<String>,
    pub first: Option<String>,
    pub last: Option<String>,
    pub title: Option<String>,
    pub user_turns: usize,
    pub status: SessionStatus,
}

/// 一場會話的回填狀態（JSON 裡是小寫字串，合約不變）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    /// 還沒抽取。
    Pending,
    /// 抽過了。
    Done,
    /// 還在寫（最近有變動），先不抽。
    Live,
    /// 對不到已登記的 repo。
    Unmapped,
    /// 沒有使用者發言。
    Empty,
}

impl SessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionStatus::Pending => "pending",
            SessionStatus::Done => "done",
            SessionStatus::Live => "live",
            SessionStatus::Unmapped => "unmapped",
            SessionStatus::Empty => "empty",
        }
    }
}

/// `ctx backfill scan` 的統計（Rust 2026-10-13 從 CLI 搬來）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanSummary {
    /// 各狀態幾場，依第一次出現的順序（＝Python `dict(Counter)`）。
    pub by_status: Vec<(SessionStatus, usize)>,
    /// 各 repo 有幾場（只算 pending＋done），場次多的在前、同數維持第一次出現的順序。
    pub by_repo: Vec<(String, usize)>,
    /// 對不到 repo 的 cwd（排序、去重；沒有 cwd 記成 `?`）。
    pub unmapped_cwds: Vec<String>,
}

/// 依第一次出現的順序計數。
fn count_in_order<T: PartialEq>(items: impl Iterator<Item = T>) -> Vec<(T, usize)> {
    let mut out: Vec<(T, usize)> = Vec::new();
    for x in items {
        match out.iter_mut().find(|(k, _)| *k == x) {
            Some((_, n)) => *n += 1,
            None => out.push((x, 1)),
        }
    }
    out
}

/// 統計 [`scan`] 的結果。
pub fn summarize(recs: &[SessionRec]) -> ScanSummary {
    let mut by_repo = count_in_order(
        recs.iter()
            .filter(|r| matches!(r.status, SessionStatus::Pending | SessionStatus::Done))
            .map(|r| r.repo.clone().unwrap_or_else(|| "None".into())),
    );
    by_repo.sort_by_key(|x| std::cmp::Reverse(x.1));
    let mut unmapped_cwds: Vec<String> = recs
        .iter()
        .filter(|r| r.status == SessionStatus::Unmapped)
        .map(|r| {
            r.cwd
                .clone()
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| "?".into())
        })
        .collect();
    unmapped_cwds.sort();
    unmapped_cwds.dedup();
    ScanSummary {
        by_status: count_in_order(recs.iter().map(|r| r.status)),
        by_repo,
        unmapped_cwds,
    }
}

/// 對應 `backfill.scan`：列出所有會話與狀態。`root` 預設 [`projects_root`]；`now` 為 epoch 秒（測試用）。
pub fn scan(spine_dir: &Path, root: Option<&Path>, now: Option<f64>) -> Result<Vec<SessionRec>> {
    let root = root.map(Path::to_path_buf).unwrap_or_else(projects_root);
    let now = now.unwrap_or_else(now_secs);
    let matcher = Matcher::new(spine_dir)?;
    let sessions_dir = dir(spine_dir, &["sessions"])?;
    let mut files: Vec<PathBuf> = Vec::new();
    for proj in std::fs::read_dir(&root).into_iter().flatten().flatten() {
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
    let mut out = Vec::new();
    for p in files {
        let m = meta(&p)?;
        let sid = stem(&p);
        let repo = matcher.matches(m.cwd.as_deref());
        let status = if now - mtime_secs(&p)? < LIVE_MINUTES * 60.0 {
            SessionStatus::Live
        } else if repo.is_none() {
            SessionStatus::Unmapped
        } else if m.user_turns == 0 {
            SessionStatus::Empty
        } else if truthy(load(&sessions_dir.join(format!("{sid}.json"))).as_ref()) {
            SessionStatus::Done
        } else {
            SessionStatus::Pending
        };
        out.push(SessionRec {
            sid,
            path: p.to_string_lossy().into_owned(),
            repo,
            cwd: m.cwd,
            first: m.first,
            last: m.last,
            title: m.title,
            user_turns: m.user_turns,
            status,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------- 2. extract

fn tool_line(b: &Value) -> String {
    let name = b.get("name").map(py_str).unwrap_or_else(|| "?".into());
    let inp = b.get("input").filter(|i| truthy(Some(i)));
    let detail = [
        "description",
        "file_path",
        "pattern",
        "command",
        "url",
        "query",
    ]
    .iter()
    .find_map(|k| inp.and_then(|i| i.get(*k)).filter(|v| truthy(Some(v))))
    .map(py_str)
    .unwrap_or_default();
    format!("〔工具 {name}〕{}", take(&detail, 120))
}

/// 對應 `backfill.compact`：會話 → 精簡文字行：`[tN]` 使用者原話／agent 文字／工具一行。
/// tool_result、thinking、系統標記全部丟掉。
pub fn compact(path: &Path) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    let mut turn = 0;
    for o in jsonl_rows(path)? {
        if truthy(o.get("isSidechain")) {
            continue;
        }
        let content = o.get("message").and_then(|m| m.get("content"));
        let typ = o.get("type").and_then(Value::as_str);
        match (typ, content) {
            (Some("user"), Some(Value::String(c))) => {
                let text = NOISE.replace_all(c, "");
                let text = text.trim();
                if text.is_empty() || text.starts_with('<') {
                    continue;
                }
                turn += 1;
                lines.push(format!("[t{turn}] 【使用者】{}", take(text, MSG_CHARS)));
            }
            (Some("assistant"), Some(Value::Array(blocks))) => {
                for b in blocks {
                    match b.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let t = b.get("text").and_then(Value::as_str).unwrap_or("").trim();
                            if !t.is_empty() {
                                lines.push(format!("【agent】{}", take(t, MSG_CHARS)));
                            }
                        }
                        Some("tool_use") => lines.push(tool_line(b)),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(lines)
}

/// 對應 `backfill.chunks`：依字數切段（每段 ≤ size，除非單行就超過）。Python 預設 size=[`CHUNK_CHARS`]。
pub fn chunks<S: AsRef<str>>(lines: &[S], size: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut n = 0usize;
    for ln in lines {
        let ln = ln.as_ref();
        let len = ln.chars().count();
        if !cur.is_empty() && n + len > size {
            out.push(cur.join("\n"));
            cur.clear();
            n = 0;
        }
        cur.push(ln);
        n += len + 1;
    }
    if !cur.is_empty() {
        out.push(cur.join("\n"));
    }
    out
}

/// 對應 `EXTRACT_PROMPT`（佔位：{repo} {date} {i} {n} {next_rule} {body}）。
pub const EXTRACT_PROMPT: &str = "你在整理一場「使用者 × coding agent」的過往會話紀錄（repo：{repo}，{date}，第 {i}/{n} 段）。
目的：把值得留下的東西撈出來，讓之後接手的 agent 知道做過什麼判斷、還剩什麼。
不要使用任何工具，只根據下面的紀錄回答。只輸出一個 JSON 物件，不要其他文字。

規則：
- 判斷（decisions）：使用者明確做出、或使用者同意採納的結論／取捨（選 A 不選 B、定規則、定方向）。
  agent 自己的推測、還在討論沒定案的、純操作步驟都不算。每筆附依據與出處輪次（[tN]）。
- 待辦（todos）：這段裡明確提到要做、但到這段結尾還沒做完的事。已經做完的不要列。
- 完成（done）：這段實際完成的事（一句一件，最多 5 件）。
- 下一步（next）：{next_rule}
- 目標線索（goal_hints）：使用者用自己的話說出的這個 repo 的目的或動機（盡量原話）；操作指令與提問句不算。
- 全部用繁體中文，每項 ≤120 字；不確定就不要列，不要編造。decisions ≤8、todos ≤6。

JSON 形狀：
{\"summary\": \"這段在做什麼（一句）\",
  \"decisions\": [{\"text\": \"結論\", \"basis\": \"為什麼\", \"turn\": \"t12\"}],
  \"todos\": [{\"text\": \"要做的事\", \"turn\": \"t20\"}],
  \"done\": [\"…\"], \"next\": \"…\", \"goal_hints\": [\"…\"]}

=== 會話紀錄 ===
{body}
";

/// 對應 `CONSOLIDATE_PROMPT`（佔位：{repo} {n} {body}）。
pub const CONSOLIDATE_PROMPT: &str = "下面是 repo「{repo}」過去 {n} 場 coding agent 會話的逐場整理，依時間由舊到新。
請彙整成「現在接手這個 repo 的人需要知道的」。不要使用任何工具。只輸出一個 JSON 物件。

規則：
- decisions：至今仍有效、會影響之後怎麼做的判斷，最多 15 筆，重要的在前。
  後來被推翻或取代的不要列（或只列最新版本）；同一件事多次出現合成一筆。每筆標來源會話（sid 前 8 碼）與日期。
- open_todos：到最後一場為止看起來仍沒做完的待辦，最多 10 筆。後面會話已完成的剔掉；看不出有沒有做完的，標 uncertain=true。
- handoff：最近一場會話結束時的狀態——done（完成了什麼）、next（下次從哪接）、date、sid。
- goal_hints：使用者說明這個 repo「為什麼存在、要達成什麼」的句子，最多 3 句（盡量原話），附 sid。
  操作指令（例：commit push、幫我改 X）與提問句不算目標線索。
- 繁體中文，每項 ≤150 字。不確定就不列，不要編造。

JSON 形狀：
{\"decisions\": [{\"text\": \"…\", \"basis\": \"…\", \"sid\": \"1a2b3c4d\", \"date\": \"2026-09-11\"}],
  \"open_todos\": [{\"text\": \"…\", \"sid\": \"…\", \"date\": \"…\", \"uncertain\": false}],
  \"handoff\": {\"done\": \"…\", \"next\": \"…\", \"sid\": \"…\", \"date\": \"…\"},
  \"goal_hints\": [{\"text\": \"…\", \"sid\": \"…\"}]}

=== 逐場整理 ===
{body}
";

/// 模板填空：一次掃過，`{key}` 換成值（body 裡的大括號不會被二次替換）。
fn fill(tpl: &str, vals: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(tpl.len());
    let mut rest = tpl;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('}').and_then(|j| {
            vals.iter()
                .find(|(k, _)| *k == &after[..j])
                .map(|kv| (j, kv.1))
        }) {
            Some((j, v)) => {
                out.push_str(v);
                rest = &after[j + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 對應 `_parse_json`：取第一個 `{` 到最後一個 `}`，必須是物件；清單欄位正規化。
fn parse_json(text: &str) -> std::result::Result<Map<String, Value>, String> {
    let (Some(a), Some(b)) = (text.find('{'), text.rfind('}')) else {
        return Err("輸出裡找不到 JSON".into());
    };
    if b < a {
        return Err("輸出裡找不到 JSON".into());
    }
    let v: Value = serde_json::from_str(&text[a..=b]).map_err(|e| e.to_string())?;
    let Value::Object(mut obj) = v else {
        return Err("輸出不是 JSON 物件".into());
    };
    for k in ["decisions", "todos", "done", "goal_hints"] {
        let v = match obj.get(k) {
            Some(Value::Array(a)) => Value::Array(a.clone()),
            _ => Value::Array(Vec::new()),
        };
        obj.insert(k.into(), v);
    }
    for k in ["decisions", "todos"] {
        if let Some(Value::Array(a)) = obj.get_mut(k) {
            a.retain(|d| {
                d.is_object()
                    && !d
                        .get("text")
                        .map(py_str)
                        .unwrap_or_default()
                        .trim()
                        .is_empty()
            });
        }
    }
    for k in ["summary", "next"] {
        let s = obj
            .get(k)
            .filter(|v| truthy(Some(v)))
            .map(py_str)
            .unwrap_or_default();
        obj.insert(k.into(), Value::String(s));
    }
    Ok(obj)
}

/// 對應 `backfill.claude_caller`：預設的模型呼叫——`claude -p`，在 spine 目錄、唯讀工具。Python 預設 model="sonnet"。
pub fn claude_caller(spine_dir: &Path, model: &str) -> impl Fn(&str) -> Result<String> + Sync {
    let sp = spine_dir.to_path_buf();
    let model = model.to_string();
    move |prompt: &str| ClaudeProvider.run_text(prompt, &sp, Some(&model), Some(&sp), true)
}

/// 對應 `_ask_json`：解析失敗重試一次；模型呼叫本身失敗直接浮出。
fn ask_json(call: &Call<'_>, prompt: &str) -> Result<Map<String, Value>> {
    let mut last = String::new();
    for _ in 0..2 {
        match parse_json(&call(prompt)?) {
            Ok(o) => return Ok(o),
            Err(e) => last = e,
        }
    }
    Err(Error::External(format!("輸出解析兩次皆失敗：{last}")))
}

fn opt_str(v: &Option<String>) -> Value {
    v.clone().map(Value::String).unwrap_or(Value::Null)
}

/// 對應 `backfill.extract`：一場會話 → `sessions/<sid>.json`。回傳結果。
pub fn extract(spine_dir: &Path, rec: &SessionRec, call: &Call<'_>) -> Result<Value> {
    let parts = chunks(&compact(Path::new(&rec.path))?, CHUNK_CHARS);
    let date = take(rec.first.as_deref().unwrap_or(""), 10);
    let repo = rec.repo.clone().unwrap_or_else(|| "None".into());
    let n = parts.len().to_string();
    let mut results = Vec::new();
    for (i, body) in parts.iter().enumerate() {
        let i = i + 1;
        let next_rule = if i == parts.len() {
            "這是最後一段：會話結束時，下次該從哪裡接（一句）；看不出來就空字串。"
        } else {
            "不是最後一段，留空字串。"
        };
        let prompt = fill(
            EXTRACT_PROMPT,
            &[
                ("repo", &repo),
                ("date", &date),
                ("i", &i.to_string()),
                ("n", &n),
                ("next_rule", next_rule),
                ("body", body),
            ],
        );
        results.push(Value::Object(ask_json(call, &prompt)?));
    }
    let out = json!({
        "sid": rec.sid, "repo": opt_str(&rec.repo), "cwd": opt_str(&rec.cwd),
        "first": opt_str(&rec.first), "last": opt_str(&rec.last), "title": opt_str(&rec.title),
        "chunks": parts.len(),
        "extracted_at": Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        "parts": results,
    });
    save(
        &dir(spine_dir, &["sessions"])?.join(format!("{}.json", rec.sid)),
        &out,
    )?;
    Ok(out)
}

// ---------------------------------------------------------------- 3. consolidate

fn arr(v: &Value, k: &str) -> Vec<Value> {
    v.get(k)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Python `d.get(k, default)` 後 f-string。
fn get_s(v: &Value, k: &str, default: &str) -> String {
    v.get(k).map(py_str).unwrap_or_else(|| default.to_string())
}

/// Python `x.get(k) or default`。
fn get_or(v: &Value, k: &str, default: &str) -> String {
    v.get(k)
        .filter(|x| truthy(Some(x)))
        .map(py_str)
        .unwrap_or_else(|| default.to_string())
}

fn session_digest(s: &Value) -> String {
    let mut lines = vec![format!(
        "## 會話 {}（{}）{}",
        take(&get_s(s, "sid", ""), 8),
        take(&get_or(s, "first", ""), 10),
        get_or(s, "title", "")
    )];
    for p in arr(s, "parts") {
        if truthy(p.get("summary")) {
            lines.push(format!("摘要：{}", py_str(&p["summary"])));
        }
        for d in arr(&p, "decisions") {
            lines.push(format!(
                "判斷：{}｜依據：{}",
                get_s(&d, "text", ""),
                get_s(&d, "basis", "")
            ));
        }
        for t in arr(&p, "todos") {
            lines.push(format!("待辦：{}", get_s(&t, "text", "")));
        }
        for x in arr(&p, "done") {
            lines.push(format!("完成：{}", py_str(&x)));
        }
        if truthy(p.get("next")) {
            lines.push(format!("下一步：{}", py_str(&p["next"])));
        }
        for g in arr(&p, "goal_hints") {
            lines.push(format!("目標線索：{}", py_str(&g)));
        }
    }
    lines.join("\n")
}

fn with_ids(items: Option<&Value>, prefix: &str) -> Vec<Value> {
    let Some(Value::Array(a)) = items else {
        return Vec::new();
    };
    a.iter()
        .enumerate()
        .filter(|(_, d)| d.is_object() && truthy(d.get("text")))
        .map(|(i, d)| {
            let mut d = d.clone();
            d["id"] = Value::String(format!("{prefix}{}", i + 1));
            d
        })
        .collect()
}

fn sids_of(v: &Value) -> Vec<String> {
    arr(v, "sessions").iter().map(py_str).collect()
}

/// 對應 `backfill.consolidate`：同一 repo 的所有已抽會話 → `repos/<repo>.json` 候選。沒有會話＝None。
pub fn consolidate(spine_dir: &Path, repo: &str, call: &Call<'_>) -> Result<Option<Value>> {
    let mut sessions: Vec<Value> = json_files(&dir(spine_dir, &["sessions"])?)
        .iter()
        .filter_map(|p| load(p))
        .filter(|s| truthy(Some(s)) && s.get("repo").and_then(Value::as_str) == Some(repo))
        .collect();
    if sessions.is_empty() {
        return Ok(None);
    }
    sessions.sort_by_key(|s| get_or(s, "first", ""));
    let mut body = sessions
        .iter()
        .map(session_digest)
        .collect::<Vec<_>>()
        .join("\n\n");
    let n_chars = body.chars().count();
    if n_chars > 120_000 {
        // 太長：留最新的（彙整最在意的是現況）
        let tail: String = body.chars().skip(n_chars - 120_000).collect();
        body = format!("…（較舊的會話省略）\n{tail}");
    }
    let n = sessions.len().to_string();
    let obj = ask_json(
        call,
        &fill(
            CONSOLIDATE_PROMPT,
            &[("repo", repo), ("n", &n), ("body", &body)],
        ),
    )?;
    let ov = Value::Object(obj);
    let decisions = with_ids(ov.get("decisions"), "d");
    let open_todos = with_ids(ov.get("open_todos"), "t");
    let handoff = match ov.get("handoff") {
        Some(h @ Value::Object(_)) if truthy(h.get("done")) || truthy(h.get("next")) => {
            let mut h = h.clone();
            h["id"] = Value::String("h".into());
            h
        }
        _ => Value::Null,
    };
    let sids: Vec<String> = sessions.iter().map(|s| get_s(s, "sid", "")).collect();
    let cand_path = dir(spine_dir, &["repos"])?.join(format!("{repo}.json"));
    let prev = load(&cand_path).unwrap_or_else(|| json!({}));
    let accepted = if prev.get("sessions").is_some() && sids_of(&prev) == sids {
        prev.get("accepted").cloned().unwrap_or_else(|| json!([]))
    } else {
        json!([])
    };
    let out = json!({
        "repo": repo, "sessions": sids,
        "consolidated_at": Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        "accepted": accepted,
        "decisions": decisions, "open_todos": open_todos, "handoff": handoff,
        "goal_hints": ov.get("goal_hints").cloned().unwrap_or(Value::Null),
    });
    save(&cand_path, &out)?;
    Ok(Some(out))
}

/// 對應 `_each`：jobs≤1 依序跑；否則開 jobs 條執行緒輪流取工作（時間幾乎都在等模型）。
fn each<T: Sync>(items: &[T], jobs: usize, f: &(dyn Fn(usize, &T) + Sync)) {
    if jobs <= 1 {
        for (i, it) in items.iter().enumerate() {
            f(i, it);
        }
        return;
    }
    let next = AtomicUsize::new(0);
    std::thread::scope(|sc| {
        for _ in 0..jobs.min(items.len()) {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(it) = items.get(i) else { break };
                f(i, it);
            });
        }
    });
}

/// [`run`] 的結果（對應 Python 回傳的 dict）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunReport {
    pub extracted: usize,
    pub consolidated: Vec<String>,
    pub failed: Vec<Value>,
}

/// [`run`] 的選項（Python 預設 root=None, limit=None, repo=None, jobs=1）。
#[derive(Debug, Clone, Default)]
pub struct RunOpts {
    pub root: Option<PathBuf>,
    pub limit: Option<usize>,
    pub repo: Option<String>,
    pub jobs: usize,
}

/// 對應 `backfill.run`：抽取所有 pending 會話，再彙整有變動的 repo。單場失敗記下來繼續
/// （整晚跑不能一場壞就停）。要重彙整的 repo 從「狀態」推，中途被殺過也能自癒。
pub fn run(spine_dir: &Path, call: &Call<'_>, opts: &RunOpts, log: &Log<'_>) -> Result<RunReport> {
    let scanned = scan(spine_dir, opts.root.as_deref(), None)?;
    let sdir = dir(spine_dir, &["sessions"])?;
    // 歸屬改了（例：後來才登記更精確的子專案）：已抽的結果改掛新 repo，新舊兩邊都重彙整，不重抽
    let mut remapped: BTreeSet<String> = BTreeSet::new();
    for r in scanned.iter().filter(|r| r.status == SessionStatus::Done) {
        let sp = sdir.join(format!("{}.json", r.sid));
        let Some(mut s) = load(&sp).filter(|s| truthy(Some(s))) else {
            continue;
        };
        let old = s.get("repo").and_then(Value::as_str).map(str::to_string);
        if old != r.repo {
            remapped.extend(old);
            remapped.extend(r.repo.clone());
            s["repo"] = opt_str(&r.repo);
            save(&sp, &s)?;
        }
    }
    let mut recs: Vec<SessionRec> = scanned
        .into_iter()
        .filter(|r| {
            r.status == SessionStatus::Pending
                && opts
                    .repo
                    .as_ref()
                    .is_none_or(|x| r.repo.as_ref() == Some(x))
        })
        .collect();
    recs.sort_by_key(|r| r.first.clone().unwrap_or_default());
    if let Some(l) = opts.limit.filter(|l| *l > 0) {
        recs.truncate(l);
    }
    let touched: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    let failed: Mutex<Vec<Value>> = Mutex::new(Vec::new());
    let total = recs.len();
    each(&recs, opts.jobs, &|i, r| {
        log(&format!(
            "[{}/{total}] {} {} {} {}",
            i + 1,
            r.repo.as_deref().unwrap_or("None"),
            take(&r.sid, 8),
            take(r.first.as_deref().unwrap_or(""), 10),
            r.title.as_deref().unwrap_or("")
        ));
        match extract(spine_dir, r, call) {
            Ok(_) => {
                if let Some(rp) = &r.repo {
                    lock(&touched).insert(rp.clone());
                }
            }
            Err(e) => {
                let msg = e.to_string();
                lock(&failed).push(
                    json!({"sid": r.sid, "repo": opt_str(&r.repo), "error": take(&msg, 300)}),
                );
                log(&format!("  失敗：{} {}", take(&r.sid, 8), take(&msg, 200)));
            }
        }
    });
    let mut actual: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for sp in json_files(&sdir) {
        if let Some(s) = load(&sp) {
            if let Some(rp) = s
                .get("repo")
                .and_then(Value::as_str)
                .filter(|x| !x.is_empty())
            {
                actual
                    .entry(rp.to_string())
                    .or_default()
                    .insert(get_s(&s, "sid", ""));
            }
        }
    }
    let mut need: BTreeSet<String> = lock(&touched).clone();
    need.extend(remapped.into_iter().filter(|r| actual.contains_key(r)));
    let rdir = dir(spine_dir, &["repos"])?;
    for cp in json_files(&rdir) {
        let st = stem(&cp);
        let c = load(&cp).unwrap_or_else(|| json!({}));
        match actual.get(&st) {
            None => {
                let _ = std::fs::remove_file(&cp);
            }
            Some(set) => {
                let have: BTreeSet<String> = sids_of(&c).into_iter().collect();
                if &have != set {
                    need.insert(st);
                }
            }
        }
    }
    for r in actual.keys() {
        if !rdir.join(format!("{r}.json")).exists() {
            need.insert(r.clone());
        }
    }
    if let Some(only) = &opts.repo {
        need.retain(|r| r == only);
    }
    let need: Vec<String> = need.into_iter().collect();
    each(&need, opts.jobs, &|_, rid| {
        log(&format!("彙整 {rid}"));
        if let Err(e) = consolidate(spine_dir, rid, call) {
            let msg = e.to_string();
            lock(&failed)
                .push(json!({"repo": rid, "stage": "consolidate", "error": take(&msg, 300)}));
            log(&format!("  失敗：{rid} {}", take(&msg, 200)));
        }
    });
    let failed = failed.into_inner().unwrap_or_else(|p| p.into_inner());
    let extract_failed = failed.iter().filter(|f| f.get("stage").is_none()).count();
    save(
        &dir(spine_dir, &[])?.join("last_run.json"),
        &json!({
            "at": Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            "extracted": total - extract_failed,
            "consolidated": need, "failed": failed,
        }),
    )?;
    Ok(RunReport {
        extracted: total,
        consolidated: need,
        failed,
    })
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

// ---------------------------------------------------------------- 4. review ／ accept

/// 對應 `backfill.candidates`：各 repo 的候選（依檔名排序）；repo 給了只回那個。
pub fn candidates(spine_dir: &Path, repo: Option<&str>) -> Result<Vec<Value>> {
    Ok(json_files(&dir(spine_dir, &["repos"])?)
        .iter()
        .filter_map(|p| load(p))
        .filter(|c| truthy(Some(c)))
        .filter(|c| repo.is_none_or(|r| c.get("repo").and_then(Value::as_str) == Some(r)))
        .collect())
}

/// 對應 `backfill.review_text`：候選給人看的文字。
pub fn review_text(spine_dir: &Path, repo: Option<&str>) -> Result<String> {
    let mut out: Vec<String> = vec![
        "# 回填候選（從過去的 Claude Code 會話撈出來的；還沒寫進 spine）".into(),
        "收：`ctx backfill accept <repo> d1 d3 t2 h`（或 `ctx backfill accept <repo> all`）。目標只列線索，請自己寫 `goal`。".into(),
        String::new(),
    ];
    for c in candidates(spine_dir, repo)? {
        let acc: BTreeSet<String> = arr(&c, "accepted").iter().map(py_str).collect();
        let mark = |i: &str| if acc.contains(i) { "✔ 已收 " } else { "" };
        out.push(format!(
            "## {}（{} 場會話）",
            get_s(&c, "repo", ""),
            arr(&c, "sessions").len()
        ));
        let hints = arr(&c, "goal_hints");
        if !hints.is_empty() {
            out.push("目標線索：".into());
            for g in hints {
                let t = if g.is_object() {
                    g.get("text").map(py_str).unwrap_or_else(|| "None".into())
                } else {
                    py_str(&g)
                };
                out.push(format!("- 「{t}」"));
            }
        }
        if let Some(h) = c.get("handoff").filter(|h| truthy(Some(h))) {
            out.push(format!(
                "[h] {}最後交接（{}）：{} → 下次：{}",
                mark("h"),
                get_s(h, "date", ""),
                get_s(h, "done", ""),
                get_s(h, "next", "")
            ));
        }
        for d in arr(&c, "decisions") {
            let id = get_s(&d, "id", "");
            let basis = if truthy(d.get("basis")) {
                format!("｜依據：{}", py_str(&d["basis"]))
            } else {
                String::new()
            };
            out.push(format!(
                "[{id}] {}判斷（{}）：{}{basis}",
                mark(&id),
                get_s(&d, "date", ""),
                get_s(&d, "text", "")
            ));
        }
        for t in arr(&c, "open_todos") {
            let id = get_s(&t, "id", "");
            out.push(format!(
                "[{id}] {}待辦（{}）：{}{}",
                mark(&id),
                get_s(&t, "date", ""),
                get_s(&t, "text", ""),
                if truthy(t.get("uncertain")) {
                    "（不確定是否已做完）"
                } else {
                    ""
                }
            ));
        }
        out.push(String::new());
    }
    if out.len() == 3 {
        out.push("還沒有候選——先跑 `ctx backfill run`。".into());
    }
    Ok(out.join("\n"))
}

/// 對應 `backfill.accept`：把勾選的候選寫進 spine（source=backfill，經 agentapi）。回傳寫入的 ref／編號。
/// 已收過的略過；`ids == ["all"]`＝全部。交接最後寫（它是「上次做到哪」，要是最新一筆）。
pub fn accept<S: AsRef<str>>(spine_dir: &Path, repo: &str, ids: &[S]) -> Result<Vec<String>> {
    let p = dir(spine_dir, &["repos"])?.join(format!("{repo}.json"));
    let Some(mut c) = load(&p).filter(|c| truthy(Some(c))) else {
        return Err(Error::invalid(format!(
            "{repo} 沒有回填候選（先跑 ctx backfill run）"
        )));
    };
    let path = registry::get_repo(spine_dir, repo)?.path_str();
    let mut items: Vec<(String, char, Value)> = Vec::new();
    let mut put = |id: String, kind: char, v: Value| match items.iter_mut().find(|x| x.0 == id) {
        Some(slot) => *slot = (id, kind, v),
        None => items.push((id, kind, v)),
    };
    for d in arr(&c, "decisions") {
        put(get_s(&d, "id", ""), 'd', d);
    }
    for t in arr(&c, "open_todos") {
        put(get_s(&t, "id", ""), 't', t);
    }
    if let Some(h) = c.get("handoff").filter(|h| truthy(Some(h))) {
        put("h".into(), 'h', h.clone());
    }
    let ids: Vec<&str> = ids.iter().map(AsRef::as_ref).collect();
    let want: Vec<String> = if ids == ["all"] {
        items.iter().map(|x| x.0.clone()).collect()
    } else {
        ids.iter().map(|s| s.to_string()).collect()
    };
    let unknown: Vec<&str> = want
        .iter()
        .filter(|i| !items.iter().any(|x| &x.0 == *i))
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        return Err(Error::invalid(format!(
            "沒有這些候選：{}（ctx backfill review 看編號）",
            unknown.join(", ")
        )));
    }
    let mut accepted: BTreeSet<String> = arr(&c, "accepted").iter().map(py_str).collect();
    let mut written = Vec::new();
    let mut order = want.clone();
    order.sort_by(|a, b| (a == "h", a).cmp(&(b == "h", b)));
    let t = Target::at(&path);
    for i in order {
        if accepted.contains(&i) {
            continue;
        }
        let Some((_, kind, it)) = items.iter().find(|x| x.0 == i) else {
            continue;
        };
        let src = format!(
            "（回填自 {} 會話 {}）",
            get_or(it, "date", "?"),
            take(&get_or(it, "sid", "?"), 8)
        );
        match kind {
            'd' => {
                let mut body = get_s(it, "text", "");
                if truthy(it.get("basis")) {
                    body.push_str(&format!("\n依據：{}", py_str(&it["basis"])));
                }
                body.push_str(&format!("\n{src}"));
                let r = agentapi::log_decision(spine_dir, &body, &t, None, false, "backfill")?;
                written.push(r.reference);
            }
            't' => {
                let text = format!("{} {src}", get_s(it, "text", ""));
                let r = agentapi::add_todo(spine_dir, &text, None, &t, false, "backfill")?;
                written.push(r.id);
            }
            _ => {
                let done = get_or(it, "done", "（未記錄）");
                let next = format!("{} {src}", get_or(it, "next", "（未記錄）"));
                let r =
                    agentapi::handoff::<&str>(spine_dir, &done, &next, &[], &t, false, "backfill")?;
                written.push(r.reference);
            }
        }
        accepted.insert(i);
    }
    c["accepted"] = json!(accepted.into_iter().collect::<Vec<_>>());
    save(&p, &c)?;
    Ok(written)
}
