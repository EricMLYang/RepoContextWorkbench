//! 脊椎（P10 append ＋ P11 query）。對應 Python `repo_context/spine.py`。
//!
//! 事件標頭嚴格文法（v2 §3.2）：
//! ```text
//! ## HH:MM <type> [<source>] key:value ...
//! ```
//! - type ∈ 七種；source＝入口（morning-brief/hotkey/manual/engine/...）
//! - kv token：group:/repo:/ref:/due:/id: 或 #編號（id: 為原型擴充，供 collision 回連）
//! - 標頭一行機器可解析，區塊內文自由文字（但不得再出現 '## ' 開頭行）
//!
//! 三件套：validator 拒寫｜跨行程 lock（含 registry）｜git 不逐事件 commit（單一提交者＝spine commit 指令）。
//! 拒寫去向：程式入口回錯（agent 自我修正）；人的入口 dead-letter 落檔（想法永不丟）。
//!
//! 事件檔 `spine/events/YYYY-MM-DD.md` 是與 Python 版共用的合約，寫出的位元組要一致。

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use chrono::{Duration, Local, NaiveDateTime, TimeZone};
use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;

use crate::locking::FileLock;
use crate::pystr::{repr, splitlines};
use crate::{Error, Result};

/// 對應 `spine.EVENT_TYPES`（順序即錯誤訊息裡的順序）。
pub const EVENT_TYPES: [&str; 7] = [
    "presented",
    "chosen",
    "decision",
    "suggestion",
    "collision",
    "open-loop",
    "outcome",
];

/// 對應 `spine.LOCK_NAME`。
pub const LOCK_NAME: &str = ".engine.lock";

/// 管理類留痕（registry／組／關係的設定變更，ops 寫的）的 body 前綴：
/// 是「改了設定」不是「做了工作」——工作摘要的「上次進度」不拿它充數。
pub const ADMIN_TAG: &str = "〔registry〕";

/// 空 token 清單（`append_event(.., NO_TOKENS, ..)`，免得型別推不出來）。
pub const NO_TOKENS: &[&str] = &[];

static HEADER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^## (?P<h>[0-2]\d):(?P<m>[0-5]\d) (?P<type>[a-z-]+) \[(?P<source>[a-z0-9-]+)\](?P<tail>( \S+)*)$")
        .expect("固定 regex")
});
static KV_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:#\d+|(?:group|repo|ref|due|id):\S+)$").expect("固定 regex"));
static DUE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^due:\d{4}-\d{2}-\d{2}$").expect("固定 regex"));

/// 事件型別（序列化字串與 Python 相同）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EventType {
    Presented,
    Chosen,
    Decision,
    Suggestion,
    Collision,
    OpenLoop,
    Outcome,
}

impl EventType {
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::Presented => "presented",
            EventType::Chosen => "chosen",
            EventType::Decision => "decision",
            EventType::Suggestion => "suggestion",
            EventType::Collision => "collision",
            EventType::OpenLoop => "open-loop",
            EventType::Outcome => "outcome",
        }
    }
}

impl FromStr for EventType {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "presented" => EventType::Presented,
            "chosen" => EventType::Chosen,
            "decision" => EventType::Decision,
            "suggestion" => EventType::Suggestion,
            "collision" => EventType::Collision,
            "open-loop" => EventType::OpenLoop,
            "outcome" => EventType::Outcome,
            _ => {
                return Err(Error::Validation(format!(
                    "未知事件型別 {}（合法: {}）",
                    repr(s),
                    EVENT_TYPES.join(", ")
                )))
            }
        })
    }
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for EventType {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// 對應 `spine.Event`。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Event {
    /// 'YYYY-MM-DD'
    pub date: String,
    /// 'HH:MM'
    pub time: String,
    #[serde(rename = "type")]
    pub etype: EventType,
    pub source: String,
    pub tokens: Vec<String>,
    /// 不含標頭
    pub body: String,
}

impl Event {
    /// 對應 `Event.kv`：第一個 `key:` token 的值。
    pub fn kv(&self, key: &str) -> Option<&str> {
        let prefix = format!("{key}:");
        self.tokens
            .iter()
            .find_map(|t| t.strip_prefix(prefix.as_str()))
    }

    /// 第一個 `#編號` token。
    pub fn num(&self) -> Option<&str> {
        self.tokens
            .iter()
            .find(|t| t.starts_with('#'))
            .map(String::as_str)
    }

    /// 對應 `Event.header`。
    pub fn header(&self) -> String {
        let tail = if self.tokens.is_empty() {
            String::new()
        } else {
            format!(" {}", self.tokens.join(" "))
        };
        format!("## {} {} [{}]{}", self.time, self.etype, self.source, tail)
    }

    /// `"YYYY-MM-DD HH:MM"`：事件時間，可直接字串比大小。
    pub fn stamp(&self) -> String {
        format!("{} {}", self.date, self.time)
    }

    /// body 第一行（沒有＝空字串）。
    pub fn first_line(&self) -> &str {
        splitlines(&self.body).first().copied().unwrap_or("")
    }
}

/// 對應 `spine.parse_header`。
pub fn parse_header(line: &str, date: &str) -> Result<Event> {
    let m = HEADER_RE.captures(line).ok_or_else(|| {
        Error::Validation(format!(
            "標頭不合文法: {}（需 '## HH:MM <type> [<source>] key:value ...'）",
            repr(line)
        ))
    })?;
    let h: u32 = m["h"].parse().unwrap_or(99);
    if h > 23 {
        return Err(Error::Validation(format!("時間不合法: {}", repr(line))));
    }
    let etype: EventType = m["type"].parse()?;
    let tokens: Vec<String> = m["tail"].split_whitespace().map(str::to_string).collect();
    for t in &tokens {
        if !KV_RE.is_match(t) {
            return Err(Error::Validation(format!(
                "kv token 不合法: {}（合法: #編號 或 group:/repo:/ref:/due:/id:）",
                repr(t)
            )));
        }
        if t.starts_with("due:") && !DUE_RE.is_match(t) {
            return Err(Error::Validation(format!(
                "due 需為 YYYY-MM-DD: {}",
                repr(t)
            )));
        }
    }
    Ok(Event {
        date: date.to_string(),
        time: format!("{}:{}", &m["h"], &m["m"]),
        etype,
        source: m["source"].to_string(),
        tokens,
        body: String::new(),
    })
}

/// 對應 `spine.validate_block`。
pub fn validate_block(header_line: &str, body: &str) -> Result<Event> {
    let mut ev = parse_header(header_line, "")?;
    if splitlines(body).iter().any(|ln| ln.starts_with("## ")) {
        return Err(Error::Validation(
            "內文不得含 '## ' 開頭行（會破壞事件邊界）".into(),
        ));
    }
    ev.body = body.trim().to_string();
    Ok(ev)
}

fn events_dir(spine_dir: &Path) -> PathBuf {
    spine_dir.join("spine").join("events")
}

fn now_or(when: Option<NaiveDateTime>) -> NaiveDateTime {
    when.unwrap_or_else(|| Local::now().naive_local())
}

/// 組名／repo id 寫進事件標頭時的樣子：空白換成 `_`（標頭的 token 用空白分隔，「PM Group」整個會變成
/// 不合法的 token，寫入直接失敗——2026-10-14 實測 PM Group 的 agent 交接、判斷、待辦全寫不進去）。
/// 沒有空白的名字原樣不變（既有事件與 Python 版照舊）。Rust 才有。
pub fn token_value(name: &str) -> String {
    if name.contains(char::is_whitespace) {
        name.split_whitespace().collect::<Vec<_>>().join("_")
    } else {
        name.to_string()
    }
}

/// 事件標頭裡的值（`group:`／`repo:`）是不是這個組名／repo id（考慮 [`token_value`] 的換寫）。
pub fn same_token(value: &str, name: &str) -> bool {
    value == name || value == token_value(name)
}

/// `group:`／`repo:` 的值有空白就換寫（見 [`token_value`]），其他 token 原樣。
fn normalize_token(t: &str) -> String {
    for key in ["group:", "repo:"] {
        if let Some(v) = t.strip_prefix(key) {
            return format!("{key}{}", token_value(v));
        }
    }
    t.to_string()
}

fn build_header<S: AsRef<str>>(
    now: NaiveDateTime,
    etype: &str,
    source: &str,
    tokens: &[S],
) -> String {
    let mut header = format!("## {} {} [{}]", now.format("%H:%M"), etype, source);
    if !tokens.is_empty() {
        header.push(' ');
        header.push_str(
            &tokens
                .iter()
                .map(|t| normalize_token(t.as_ref()))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    header
}

/// 對應 `spine.append_event(..., dead_letter=False)`：驗證失敗回 `Error::Validation`
/// （程式／MCP 入口，錯誤原樣給 agent）。`etype` 收字串，因為壞型別本身也要能被拒寫。
pub fn append_event<S: AsRef<str>>(
    spine_dir: impl AsRef<Path>,
    etype: &str,
    source: &str,
    tokens: &[S],
    body: &str,
    when: Option<NaiveDateTime>,
) -> Result<Event> {
    let spine_dir = spine_dir.as_ref();
    let now = now_or(when);
    let header = build_header(now, etype, source, tokens);
    let ev = validate_block(&header, body)?;
    write_event(spine_dir, ev, now)
}

/// 對應 `spine.append_event(..., dead_letter=True)`（人的入口）：驗證失敗落 dead-letter 檔、回 `Ok(None)`。
pub fn append_event_or_dead_letter<S: AsRef<str>>(
    spine_dir: impl AsRef<Path>,
    etype: &str,
    source: &str,
    tokens: &[S],
    body: &str,
    when: Option<NaiveDateTime>,
) -> Result<Option<Event>> {
    let spine_dir = spine_dir.as_ref();
    let now = now_or(when);
    let header = build_header(now, etype, source, tokens);
    match validate_block(&header, body) {
        Ok(ev) => write_event(spine_dir, ev, now).map(Some),
        Err(Error::Validation(msg)) => {
            write_dead_letter(spine_dir, &header, body, &msg, now)?;
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

fn write_event(spine_dir: &Path, mut ev: Event, now: NaiveDateTime) -> Result<Event> {
    ev.date = now.format("%Y-%m-%d").to_string();
    let day_file = events_dir(spine_dir).join(format!("{}.md", ev.date));
    let _g = FileLock::new(spine_dir.join(LOCK_NAME)).acquire()?;
    if let Some(parent) = day_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut buf = format!("{}\n", ev.header());
    if !ev.body.is_empty() {
        buf.push_str(&ev.body);
        buf.push('\n');
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&day_file)?;
    f.write_all(buf.as_bytes())?;
    Ok(ev)
}

/// 對應 `spine._write_dead_letter`。
fn write_dead_letter(
    spine_dir: &Path,
    header: &str,
    body: &str,
    error: &str,
    now: NaiveDateTime,
) -> Result<PathBuf> {
    let dl_dir = spine_dir.join("spine").join("dead-letter");
    std::fs::create_dir_all(&dl_dir)?;
    let p = dl_dir.join(format!("{}.md", now.format("%Y-%m-%d-%H%M%S-%6f")));
    std::fs::write(
        &p,
        format!("<!-- validator 拒寫: {error} -->\n{header}\n{body}\n"),
    )?;
    Ok(p)
}

/// 對應 `spine.iter_events`：讀回全部事件（依檔名日期、檔內順序）。壞行直接回錯——脊椎裡不該有壞行。
pub fn iter_events(spine_dir: impl AsRef<Path>) -> Result<Vec<Event>> {
    let d = events_dir(spine_dir.as_ref());
    let mut out = Vec::new();
    if !d.is_dir() {
        return Ok(out);
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&d)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md") && p.is_file())
        .collect();
    files.sort();
    for f in files {
        let date = f
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = std::fs::read_to_string(&f)?;
        let mut cur: Option<Event> = None;
        let mut body_lines: Vec<&str> = Vec::new();
        for ln in splitlines(&text) {
            if ln.starts_with("## ") {
                if let Some(mut ev) = cur.take() {
                    ev.body = body_lines.join("\n").trim().to_string();
                    out.push(ev);
                }
                cur = Some(parse_header(ln, &date)?);
                body_lines.clear();
            } else if cur.is_some() && ln.trim() != "---" {
                body_lines.push(ln);
            }
        }
        if let Some(mut ev) = cur {
            ev.body = body_lines.join("\n").trim().to_string();
            out.push(ev);
        }
    }
    Ok(out)
}

/// `query` 的條件（全部 `None`＝不篩）。
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub etype: Option<EventType>,
    pub date: Option<String>,
    pub group: Option<String>,
    /// 只要 (date, time) 嚴格晚於這個的事件。
    pub since: Option<(String, String)>,
}

impl Query {
    pub fn of_type(t: EventType) -> Self {
        Query {
            etype: Some(t),
            ..Default::default()
        }
    }
}

/// 對應 `spine.query`。
pub fn query(spine_dir: impl AsRef<Path>, q: &Query) -> Result<Vec<Event>> {
    Ok(iter_events(spine_dir)?
        .into_iter()
        .filter(|ev| matches_query(ev, q))
        .collect())
}

fn matches_query(ev: &Event, q: &Query) -> bool {
    if q.etype.is_some_and(|t| ev.etype != t) {
        return false;
    }
    if q.date
        .as_deref()
        .is_some_and(|d| !d.is_empty() && ev.date != d)
    {
        return false;
    }
    if q.group
        .as_deref()
        .is_some_and(|g| !g.is_empty() && !ev.kv("group").is_some_and(|v| same_token(v, g)))
    {
        return false;
    }
    if let Some((sd, st)) = &q.since {
        if (ev.date.as_str(), ev.time.as_str()) <= (sd.as_str(), st.as_str()) {
            return false;
        }
    }
    true
}

/// 對應 `spine.in_scope`：事件是否屬於某個工作組範圍——group token 相符，或 repo token 在成員內。
/// group 與 ids 都沒給＝不限範圍（「全部」）。
///
/// 範圍判定只有這一份（2026-09-12 UX 檢討 §6.2：畫面宣告的組別與內容必須一致）——
/// 情境卡、簡報、工作摘要、會話回報都走這裡，不各自用各自的規則。
pub fn in_scope<S: AsRef<str>>(ev: &Event, group: Option<&str>, ids: &[S]) -> bool {
    let group = group.filter(|g| !g.is_empty());
    if group.is_none() && ids.is_empty() {
        return true;
    }
    if let (Some(g), Some(v)) = (group, ev.kv("group")) {
        if same_token(v, g) {
            return true;
        }
    }
    match ev.kv("repo") {
        Some(rid) if !rid.is_empty() => ids.iter().any(|i| same_token(rid, i.as_ref())),
        _ => false,
    }
}

/// 對應 `spine.unscoped`：既非某組也非某 repo 的事件（全域或未分類）——要保留就獨立標示，不併進某一組。
pub fn unscoped(ev: &Event) -> bool {
    ev.kv("group").is_none_or(str::is_empty) && ev.kv("repo").is_none_or(str::is_empty)
}

/// 對應 `spine.open_loops`：未結清單＝算出來的視圖——opened 過、之後沒有 closed 事件 ref 到它。
pub fn open_loops(spine_dir: impl AsRef<Path>) -> Result<Vec<Event>> {
    Ok(open_loops_of(&iter_events(spine_dir)?))
}

/// 同 [`open_loops`]，但吃已讀出的事件。
pub fn open_loops_of(events: &[Event]) -> Vec<Event> {
    let mut opened: Vec<(String, Event)> = Vec::new();
    let mut closed: HashSet<String> = HashSet::new();
    for ev in events.iter().filter(|e| e.etype == EventType::OpenLoop) {
        let num = ev.num().map(str::to_string);
        let first: String = ev
            .body
            .split('\n')
            .next()
            .unwrap_or("")
            .chars()
            .take(12)
            .collect();
        if ev.body.starts_with("closed") || first.contains("closed") {
            if let Some(n) = num {
                closed.insert(n);
            }
        } else if let Some(n) = num {
            match opened.iter_mut().find(|(k, _)| *k == n) {
                Some(slot) => slot.1 = ev.clone(),
                None => opened.push((n, ev.clone())),
            }
        }
    }
    opened
        .into_iter()
        .filter(|(n, _)| !closed.contains(n))
        .map(|(_, e)| e)
        .collect()
}

/// `stats` 的結果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stats {
    pub collisions: usize,
    pub collisions_with_outcome: usize,
    pub hit_rate: Option<f64>,
}

/// 對應 `spine.stats`：品味量測最小版（M2 驗收③：靈感命中率要算得出來）。
pub fn stats(spine_dir: impl AsRef<Path>) -> Result<Stats> {
    let mut collisions = HashSet::new();
    let mut hit = HashSet::new();
    for ev in iter_events(spine_dir)? {
        match ev.etype {
            EventType::Collision => {
                if let Some(cid) = ev.kv("id").filter(|s| !s.is_empty()) {
                    collisions.insert(cid.to_string());
                }
            }
            EventType::Outcome => {
                if let Some(r) = ev.kv("ref").and_then(|r| r.strip_prefix("collision:")) {
                    hit.insert(r.to_string());
                }
            }
            _ => {}
        }
    }
    let n = collisions.len();
    let h = hit.intersection(&collisions).count();
    Ok(Stats {
        collisions: n,
        collisions_with_outcome: h,
        hit_rate: (n > 0).then(|| h as f64 / n as f64),
    })
}

/// 對應 `spine.lint`：脊椎衛生迴圈（second-brain 的 /second-brain-lint 課，拆機報告 §5.1：
/// 知識庫會爛掉，衛生檢查要當一等公民；staleness 做在資料層——Szapar 課 §5.2）。
/// 回傳紅字清單；跟 registry.audit 同形態，工作台併同一個紅字面呈現。`today`：'YYYY-MM-DD'，`None`＝今天。
pub fn lint(spine_dir: impl AsRef<Path>, today: Option<&str>) -> Result<Vec<String>> {
    let spine_dir = spine_dir.as_ref();
    let today = today
        .map(str::to_string)
        .unwrap_or_else(|| Local::now().format("%Y-%m-%d").to_string());
    let mut reds = Vec::new();
    let events = iter_events(spine_dir)?;
    // 1) ref 斷鏈：ref:<type>:<key> 要解得回既有事件（key＝id: 值或 HH:MM）
    let mut keys: HashSet<(&str, &str)> = HashSet::new();
    for ev in &events {
        keys.insert((ev.etype.as_str(), ev.time.as_str()));
        if let Some(eid) = ev.kv("id").filter(|s| !s.is_empty()) {
            keys.insert((ev.etype.as_str(), eid));
        }
    }
    for ev in &events {
        let Some(r) = ev.kv("ref").filter(|r| r.contains(':')) else {
            continue;
        };
        let (rtype, rkey) = r.split_once(':').unwrap_or((r, ""));
        if !keys.contains(&(rtype, rkey)) {
            reds.push(format!(
                "[ref 斷鏈] {} {} {} ref:{} 解不回任何事件",
                ev.date, ev.time, ev.etype, r
            ));
        }
    }
    // 2) 逾期 open loops（過 due 未 closed＝最該浮上來的陳舊）
    for ev in open_loops_of(&events) {
        if let Some(due) = ev.kv("due").filter(|d| !d.is_empty()) {
            if due < today.as_str() {
                let num = ev.num().unwrap_or("#?");
                reds.push(format!(
                    "[open-loop 逾期] {num} due:{due}「{}」",
                    ev.first_line()
                ));
            }
        }
    }
    // 3) 碰撞無回程：opened 後判定沒回來（判定分鐘級，隔天還沒回＝detached 進程死了）
    let mut opened: BTreeMap<&str, &Event> = BTreeMap::new();
    let mut answered: HashSet<&str> = HashSet::new();
    for ev in events.iter().filter(|e| e.etype == EventType::Collision) {
        let Some(cid) = ev.kv("id").filter(|s| !s.is_empty()) else {
            continue;
        };
        if ev.body.starts_with("opened") {
            opened.insert(cid, ev);
        } else {
            answered.insert(cid);
        }
    }
    for (cid, ev) in opened {
        if !answered.contains(cid) && ev.date.as_str() < today.as_str() {
            reds.push(format!(
                "[碰撞無回程] {cid}：opened 後判定沒回來（想法在，判定丟了）"
            ));
        }
    }
    // 4) dead-letter 積壓（拒寫原文等人撿，放著＝想法實質丟了）
    let dl = spine_dir.join("spine").join("dead-letter");
    let n = if dl.is_dir() {
        std::fs::read_dir(&dl)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
            .count()
    } else {
        0
    };
    if n > 0 {
        reds.push(format!("[dead-letter 積壓] {n} 件拒寫原文待人工撿回"));
    }
    Ok(reds)
}

/// 對應 `spine.default_due_tokens`：open-loop opened 未帶 due → 補預設到期（§3.4 openloop_default_due_days）。
/// closed 事件（狀態變化記新事件）不補。
pub fn default_due_tokens<S: AsRef<str>>(
    tokens: &[S],
    body: &str,
    due_days: i64,
    when: Option<NaiveDateTime>,
) -> Vec<String> {
    let mut out: Vec<String> = tokens.iter().map(|t| t.as_ref().to_string()).collect();
    if out.iter().any(|t| t.starts_with("due:")) || body.trim_start().starts_with("closed") {
        return out;
    }
    let due = now_or(when) + Duration::days(due_days);
    out.push(format!("due:{}", due.format("%Y-%m-%d")));
    out
}

/// 對應 `spine.batch_commit`：git 單一提交者的唯一入口（P10 三件套之三）——CLI `commit` 與 S6 timer
/// 都走這裡，其他進程只寫檔不碰 git。回傳 true＝有 commit，false＝無變更。
///
/// 與 Python 的差異：持 `.engine.lock` 才 add＋commit——hook 的自動 commit（[`autocommit`]）與 timer、
/// CLI 可能同時跑，也不會把寫到一半的事件檔 commit 進去。
pub fn batch_commit(spine_dir: impl AsRef<Path>, message: &str) -> Result<bool> {
    let d = spine_dir.as_ref();
    crate::locking::with_spine_lock(d, || git_add_commit(d, message))
}

fn git_add_commit(d: &Path, message: &str) -> Result<bool> {
    let st = Command::new("git")
        .args(["add", "-A"])
        .current_dir(d)
        .output()?;
    if !st.status.success() {
        return Err(Error::External(format!(
            "git add -A 失敗：{}",
            String::from_utf8_lossy(&st.stderr).trim()
        )));
    }
    let r = Command::new("git")
        .args(["commit", "-q", "-m", message])
        .current_dir(d)
        .output()?;
    Ok(r.status.success())
}

/// `batch_commit` 的預設訊息。
pub const DEFAULT_COMMIT_MESSAGE: &str = "spine: batch commit";

/// [`autocommit`] 的 commit 訊息。
pub const AUTO_COMMIT_MESSAGE: &str = "spine: auto commit (session end)";

/// config `thresholds.spine_autocommit_hours` 沒設時的預設：上次 commit 超過幾小時才自動 commit。
pub const AUTOCOMMIT_HOURS_DEFAULT: i64 = 6;

/// [`autocommit`] 做了什麼。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoCommit {
    /// config 設 0＝關掉
    Disabled,
    /// spine 不是 git repo，或一個 commit 都還沒有（使用者還沒開始用 git 管它，不替他開頭）
    NoHistory,
    /// 沒有變更
    Clean,
    /// 上次 commit 還不夠久（留給 timer／手動 commit 批次收）
    Recent,
    /// 已 commit；`push`＝有 upstream，已在背景 push
    Committed { push: bool },
}

/// Rust 才有（N-01）：會話結束時補一次批次 commit——timer 住在桌面 App 裡，App 沒開時
/// 排程的 commit 不會跑，判斷就只存在工作目錄裡。條件：spine 已有 git 歷史、有變更、
/// 上次 commit 超過 `thresholds.spine_autocommit_hours`（預設 6；0＝關）。
/// commit 走 [`batch_commit`]（單一入口、持鎖）；分支有 upstream 就背景 `git push`（不等、不報錯，
/// 失敗下次再推）。`REPOCTX_NO_BACKGROUND` 有值時不 push。
pub fn autocommit(spine_dir: &Path, now: Option<NaiveDateTime>) -> Result<AutoCommit> {
    let hours = crate::config::load(spine_dir)?
        .i64("thresholds.spine_autocommit_hours")
        .unwrap_or(AUTOCOMMIT_HOURS_DEFAULT);
    if hours <= 0 {
        return Ok(AutoCommit::Disabled);
    }
    if !spine_dir.join(".git").exists() {
        return Ok(AutoCommit::NoHistory);
    }
    let Some(last) = git_out(spine_dir, &["log", "-1", "--format=%ct"])
        .and_then(|s| s.trim().parse::<i64>().ok())
    else {
        return Ok(AutoCommit::NoHistory);
    };
    if git_out(spine_dir, &["status", "--porcelain"]).is_none_or(|s| s.trim().is_empty()) {
        return Ok(AutoCommit::Clean);
    }
    let now_ts = Local
        .from_local_datetime(&now_or(now))
        .earliest()
        .map(|t| t.timestamp())
        .unwrap_or(last);
    if now_ts - last < hours * 3600 {
        return Ok(AutoCommit::Recent);
    }
    if !batch_commit(spine_dir, AUTO_COMMIT_MESSAGE)? {
        return Ok(AutoCommit::Clean);
    }
    let upstream = git_out(
        spine_dir,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .is_some();
    if upstream && !crate::exe::background_disabled() {
        if let Some(git) = crate::exe::which("git") {
            let dir = spine_dir.as_os_str();
            let _ = crate::exe::spawn_detached(
                &git,
                ["-C".as_ref(), dir, "push".as_ref(), "-q".as_ref()],
            );
        }
    }
    Ok(AutoCommit::Committed { push: upstream })
}

/// `git -C <d> <args>` 成功時的 stdout；失敗＝None。
fn git_out(d: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(d)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 對應 `spine.last_seen_path`。
pub fn last_seen_path(spine_dir: impl AsRef<Path>) -> PathBuf {
    spine_dir.as_ref().join(".state").join("last_seen.txt")
}

/// 對應 `spine.get_unread`：`.state/last_seen.txt`（'YYYY-MM-DD HH:MM'）之後的事件。
pub fn get_unread(spine_dir: impl AsRef<Path>) -> Result<Vec<Event>> {
    let spine_dir = spine_dir.as_ref();
    let p = last_seen_path(spine_dir);
    let mut since = None;
    if p.exists() {
        let raw = std::fs::read_to_string(&p)?;
        let raw = raw.trim();
        if !raw.is_empty() {
            let (d, t) = raw.split_once(' ').unwrap_or((raw, ""));
            since = Some((d.to_string(), t.to_string()));
        }
    }
    query(
        spine_dir,
        &Query {
            since,
            ..Default::default()
        },
    )
}

/// 對應 `spine.ack_unread`。
pub fn ack_unread(spine_dir: impl AsRef<Path>, when: Option<NaiveDateTime>) -> Result<()> {
    let now = now_or(when);
    let p = last_seen_path(spine_dir);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&p, now.format("%Y-%m-%d %H:%M").to_string())?;
    Ok(())
}
