//! 會話全文搜尋：`.state/sessions.db`（SQLite＋FTS5，2026-11-18 F-134；Rust 才有，沒有對拍合約）。
//!
//! 過去的會話清單只比標題、第一句、最後回覆；這裡把兩家逐字稿裡的話（你說的、回覆、工具的一行摘要）
//! 都索引起來，`ctx sessions --grep`、App ⌘K 的「&」前綴查得到「那次跟 agent 講過 X 的會話」，一鍵接續。
//!
//! ## 範圍
//! - 只索引 cwd 落在**已登記 repo** 底下的互動會話（跟過去的會話同一套規則：`claude -p`、`codex exec`、子 agent 不收）。
//!   查詢時再照組別範圍過濾（`sessions.repo`）。登記的 repo 變了，下次查詢時重新歸屬：不在任何 repo 底下的整場刪掉。
//! - 只存三種訊息：你說的、回覆、工具（名稱＋一行摘要）。思考、工具結果、壓縮摘要不存（太大，也不是要找的話）。
//!   每則最多 [`TEXT_MAX`] 個字元。
//! - **秘密**：每段文字（訊息、標題、第一句、最後回覆）寫進庫之前都過 [`crate::redact::redact_secrets`]
//!   （spine 寫入的同一套）；片段是從庫裡（遮蔽後的）文字切的。
//!
//! ## schema（`PRAGMA user_version` ＝ [`DB_SCHEMA_VERSION`]；不符就整個重建）
//! - `files(path, agent, size, mtime, offset, session, skip, cwd)`：每份看過的逐字稿一列。`offset`＝讀到哪個位元組
//!   （一定是行首）。`skip`：`programmatic`（不是互動會話，永遠不收）／`scope`（cwd 不在登記的 repo 底下；
//!   登記變了會重看）／`unreadable`（還沒寫出開頭；檔案變了會重看）。
//! - `sessions(id, agent, sid, cwd, repo, title, prompt, reply, branch, started, mtime, path, seq)`：一份檔一場。
//! - `messages(id, session, seq, role, at)` ＋ `messages_fts(text)`（FTS5，`rowid`＝`messages.id`）。
//! - `meta('generation')`：每次寫入交易加一（測試用）。
//!
//! ## 增量
//! 逐字稿是 append-only：大小、mtime 都沒變＝不讀；變大了＝從 `offset` 往下讀新增的完整行（還沒寫完的最後一行
//! 留到下次）；變小了或換了＝整份重讀。查詢時才更新（跟知識索引一樣），不常駐輪詢。
//!
//! ## 中文
//! FTS5 的 `trigram` 斷詞：三個字元以上的詞走索引（`MATCH`）；一兩個字的詞（「索引」「登入」）trigram 查不到，
//! 改用 `LIKE`（同一張表，SQLite 對短樣式退回逐列比對）。實測見 `docs/RUST_PORTING.md`。
//!
//! ## 多個行程、壞掉的時候
//! 照 [`crate::knowledge`] 的索引庫紀律：WAL、`synchronous=NORMAL`、busy 3 秒；一份檔一個 `BEGIN IMMEDIATE` 交易，
//! 交易裡再看一次那份檔的列（別的行程剛更新過就跳過，不會重複）；拿不到寫入鎖就不更新、照現有的查。
//! 開不起來、不是資料庫、損毀、表不見——busy 以外的錯誤都刪庫重建一次。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, ErrorCode, OptionalExtension};
use serde::Serialize;

use super::{repo_of, targets_of, Item, Items, Origin, Role, Scanner, Sources, Start};
use crate::config::AgentKind;
use crate::redact::redact_secrets;
use crate::transcript::one_line;
use crate::{registry, Error, Result};

/// 索引庫格式版本。欄位、斷詞、收哪些訊息有改就加一：舊庫整個重建。
pub const DB_SCHEMA_VERSION: i64 = 1;
const DB_NAME: &str = "sessions.db";
const BUSY: Duration = Duration::from_secs(3);
/// 一則訊息最多存幾個字元。
pub const TEXT_MAX: usize = 20_000;
/// 一次查詢最多看幾則命中的訊息（之後才依會話分組）。
const ROW_CAP: i64 = 5_000;
/// trigram 查得到的最短詞（字元）。
const TRIGRAM_MIN: usize = 3;

const SCHEMA: &str = "
CREATE TABLE meta(key TEXT PRIMARY KEY, value INTEGER NOT NULL) WITHOUT ROWID;
CREATE TABLE files(
  path TEXT PRIMARY KEY,
  agent TEXT NOT NULL,
  size INTEGER NOT NULL,
  mtime INTEGER NOT NULL,
  offset INTEGER NOT NULL,
  session INTEGER,
  skip TEXT,
  cwd TEXT
);
CREATE TABLE sessions(
  id INTEGER PRIMARY KEY,
  agent TEXT NOT NULL,
  sid TEXT NOT NULL,
  cwd TEXT NOT NULL,
  repo TEXT,
  title TEXT,
  prompt TEXT,
  reply TEXT,
  branch TEXT,
  started TEXT,
  mtime INTEGER NOT NULL,
  path TEXT NOT NULL UNIQUE,
  seq INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX sessions_repo ON sessions(repo, mtime);
CREATE TABLE messages(
  id INTEGER PRIMARY KEY,
  session INTEGER NOT NULL,
  seq INTEGER NOT NULL,
  role TEXT NOT NULL,
  at TEXT
);
CREATE INDEX messages_session ON messages(session);
CREATE VIRTUAL TABLE messages_fts USING fts5(text, tokenize='trigram');
INSERT INTO meta VALUES('generation', 0);
";
const TABLES: [&str; 5] = ["meta", "files", "sessions", "messages", "messages_fts"];

enum Fail {
    Sql(rusqlite::Error),
    Io,
}

impl From<rusqlite::Error> for Fail {
    fn from(e: rusqlite::Error) -> Self {
        Fail::Sql(e)
    }
}

fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

pub fn db_path(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join(DB_NAME)
}

fn remove_db(spine_dir: &Path) {
    let p = db_path(spine_dir);
    for suffix in ["", "-wal", "-shm"] {
        let mut s = p.clone().into_os_string();
        s.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(s));
    }
}

fn open(spine_dir: &Path) -> std::result::Result<Connection, Fail> {
    let p = db_path(spine_dir);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|_| Fail::Io)?;
    }
    let mut c = Connection::open(&p)?;
    c.busy_timeout(BUSY)?;
    let _: String = c.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
    c.pragma_update(None, "synchronous", "NORMAL")?;
    c.pragma_update(None, "journal_size_limit", 16 * 1024 * 1024)?;
    ensure_schema(&mut c)?;
    Ok(c)
}

fn user_version(c: &Connection) -> rusqlite::Result<i64> {
    c.pragma_query_value(None, "user_version", |r| r.get(0))
}

fn ensure_schema(c: &mut Connection) -> std::result::Result<(), Fail> {
    if user_version(c)? == DB_SCHEMA_VERSION {
        // 表被刪掉之類的損毀：一句便宜的查詢就會失敗、走重建
        c.query_row("SELECT count(*) FROM meta", [], |r| r.get::<_, i64>(0))?;
        return Ok(());
    }
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if user_version(&tx)? != DB_SCHEMA_VERSION {
        for t in TABLES {
            tx.execute_batch(&format!("DROP TABLE IF EXISTS {t};"))?;
        }
        tx.execute_batch(SCHEMA)?;
        tx.pragma_update(None, "user_version", DB_SCHEMA_VERSION)?;
    }
    tx.commit()?;
    Ok(())
}

/// 開庫跑 `f`；壞了（busy 以外的錯）就刪庫重來一次。
fn with_db<T>(
    spine_dir: &Path,
    mut f: impl FnMut(&mut Connection) -> std::result::Result<T, Fail>,
) -> Result<T> {
    let mut last = String::new();
    for attempt in 0..2 {
        match open(spine_dir).and_then(|mut c| f(&mut c)) {
            Ok(v) => return Ok(v),
            Err(Fail::Sql(e)) if attempt == 0 && !is_busy(&e) => {
                last = e.to_string();
                remove_db(spine_dir);
            }
            Err(Fail::Sql(e)) => {
                last = e.to_string();
                break;
            }
            Err(Fail::Io) => {
                last = "建不了 .state/".into();
                break;
            }
        }
    }
    Err(Error::External(format!(
        "會話索引（{}）用不了：{last}；可以刪掉它再查一次",
        db_path(spine_dir).display()
    )))
}

/// 每次寫入交易加一的計數（測試用；沒有庫＝None）。
#[doc(hidden)]
pub fn generation(spine_dir: &Path) -> Option<i64> {
    let c = Connection::open_with_flags(
        db_path(spine_dir),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    c.query_row("SELECT value FROM meta WHERE key='generation'", [], |r| {
        r.get(0)
    })
    .ok()
}

fn bump(tx: &Connection) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE meta SET value = value + 1 WHERE key = 'generation'",
        [],
    )
    .map(|_| ())
}

fn nanos(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

fn from_nanos(n: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_nanos(n.max(0) as u64)
}

/// 寫進庫的文字：遮秘密、截長度。
fn clean(s: &str, max: usize) -> String {
    redact_secrets(&crate::pystr::clip(s.trim(), max)).0
}

// ── 增量更新 ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
struct FileRow {
    size: i64,
    mtime: i64,
    offset: i64,
    session: Option<i64>,
    skip: Option<String>,
    cwd: Option<String>,
}

fn file_row(c: &Connection, path: &str) -> rusqlite::Result<Option<FileRow>> {
    c.query_row(
        "SELECT size, mtime, offset, session, skip, cwd FROM files WHERE path = ?1",
        [path],
        |r| {
            Ok(FileRow {
                size: r.get(0)?,
                mtime: r.get(1)?,
                offset: r.get(2)?,
                session: r.get(3)?,
                skip: r.get(4)?,
                cwd: r.get(5)?,
            })
        },
    )
    .optional()
}

/// 這次更新做了什麼（`ctx sessions --grep --json` 的 `index` 欄、驗收量測用）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IndexStats {
    /// 庫裡的會話（全部登記的 repo，不只查詢範圍）。
    pub sessions: usize,
    /// 各家幾場（`claude`、`codex`）。
    pub by_agent: Vec<(String, usize)>,
    /// 庫裡的訊息則數。
    pub messages: usize,
    /// 這次整份讀的檔。
    pub read: usize,
    /// 這次只讀新增部分的檔。
    pub appended: usize,
    /// 這次新增的訊息則數。
    pub added: usize,
    /// 這次拿不到寫入鎖（別的行程在寫），沒更新完、照現有的查。
    pub busy: bool,
}

enum Plan {
    /// 從頭讀（新檔、變小、或登記的 repo 變了要重看）。
    Full,
    /// 從 `offset` 往下讀。
    Append,
}

/// 把這份檔讀進庫（在呼叫端開好的寫入交易裡）。回傳新增的訊息則數。
/// 一份檔現在的大小與 mtime（奈秒）。
#[derive(Clone, Copy)]
struct Stat {
    size: i64,
    mtime: i64,
}

fn ingest(
    tx: &Connection,
    sc: &dyn Scanner,
    path: &Path,
    stat: Stat,
    plan: Plan,
    old: Option<&FileRow>,
    targets: &[(String, PathBuf)],
) -> rusqlite::Result<usize> {
    let Stat { size, mtime } = stat;
    let p = path.to_string_lossy();
    let agent = sc.kind().name();
    let (session, start, mut seq) = match (plan, old) {
        (
            Plan::Append,
            Some(FileRow {
                session: Some(s),
                offset,
                ..
            }),
        ) => {
            let seq: i64 =
                tx.query_row("SELECT seq FROM sessions WHERE id = ?1", [s], |r| r.get(0))?;
            (*s, Start::At(*offset as u64), seq)
        }
        _ => {
            if let Some(s) = old.and_then(|o| o.session) {
                drop_session(tx, s)?;
            }
            let skip = |why: &str, cwd: Option<&str>| {
                tx.execute(
                    "INSERT OR REPLACE INTO files(path, agent, size, mtime, offset, session, skip, cwd)
                     VALUES(?1, ?2, ?3, ?4, 0, NULL, ?5, ?6)",
                    params![p, agent, size, mtime, why, cwd],
                )
                .map(|_| 0)
            };
            let Some(s) = sc.probe(path) else {
                return skip("unreadable", None);
            };
            if s.origin != Origin::Interactive {
                return skip("programmatic", Some(&s.cwd));
            }
            let Some(repo) = repo_of(targets, &s.cwd) else {
                return skip("scope", Some(&s.cwd));
            };
            tx.execute(
                "INSERT INTO sessions(agent, sid, cwd, repo, title, prompt, reply, branch, started, mtime, path)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    agent,
                    s.id,
                    s.cwd,
                    repo,
                    s.title.as_deref().map(|t| clean(t, 300)),
                    s.prompt.as_deref().map(|t| clean(t, 2_000)),
                    s.reply.as_deref().map(|t| clean(t, 2_000)),
                    s.branch,
                    s.started,
                    mtime,
                    p
                ],
            )?;
            (tx.last_insert_rowid(), Start::Head, 0)
        }
    };
    let Ok(mut items) = Items::open_with(sc, path, start, true) else {
        // 讀不了（剛被刪掉之類）：下次再看
        return Ok(0);
    };
    let mut added = 0usize;
    let mut title: Option<String> = None;
    let mut reply: Option<String> = None;
    {
        let mut ins_msg =
            tx.prepare("INSERT INTO messages(session, seq, role, at) VALUES(?1, ?2, ?3, ?4)")?;
        let mut ins_fts = tx.prepare("INSERT INTO messages_fts(rowid, text) VALUES(?1, ?2)")?;
        for it in items.by_ref() {
            let m = match it {
                Item::Title(t) => {
                    title = Some(t);
                    continue;
                }
                Item::Message(m) => m,
                Item::ToolResult { .. } => continue,
            };
            let text = match (m.role, &m.tool) {
                (Role::User | Role::Assistant, _) => m.text.clone(),
                (Role::Tool, Some(t)) => format!("{} {}", t.name, m.text),
                _ => continue,
            };
            let text = clean(&text, TEXT_MAX);
            if text.is_empty() {
                continue;
            }
            if m.role == Role::Assistant {
                reply = Some(m.text.clone());
            }
            seq += 1;
            ins_msg.execute(params![session, seq, m.role.as_str(), m.at])?;
            ins_fts.execute(params![tx.last_insert_rowid(), text])?;
            added += 1;
        }
    }
    let offset = items.lines.pos() as i64;
    tx.execute(
        "UPDATE sessions SET seq = ?2, mtime = ?3,
           title = COALESCE(?4, title), reply = COALESCE(?5, reply) WHERE id = ?1",
        params![
            session,
            seq,
            mtime,
            title.as_deref().map(|t| clean(t, 300)),
            reply.as_deref().map(|t| clean(t, 2_000)),
        ],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO files(path, agent, size, mtime, offset, session, skip, cwd)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, NULL, NULL)",
        params![p, agent, size, mtime, offset, session],
    )?;
    Ok(added)
}

fn drop_session(tx: &Connection, s: i64) -> rusqlite::Result<()> {
    tx.execute(
        "DELETE FROM messages_fts WHERE rowid IN (SELECT id FROM messages WHERE session = ?1)",
        [s],
    )?;
    tx.execute("DELETE FROM messages WHERE session = ?1", [s])?;
    tx.execute("DELETE FROM sessions WHERE id = ?1", [s])?;
    Ok(())
}

/// 依現在的檔案更新索引（只動有變的）。
fn refresh(
    c: &mut Connection,
    src: &Sources,
    targets: &[(String, PathBuf)],
) -> std::result::Result<IndexStats, Fail> {
    let roots: Vec<PathBuf> = targets.iter().map(|(_, p)| p.clone()).collect();
    let mut st = IndexStats::default();
    let mut known: HashMap<String, FileRow> = HashMap::new();
    {
        let mut q = c.prepare("SELECT path, size, mtime, offset, session, skip, cwd FROM files")?;
        let rows = q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                FileRow {
                    size: r.get(1)?,
                    mtime: r.get(2)?,
                    offset: r.get(3)?,
                    session: r.get(4)?,
                    skip: r.get(5)?,
                    cwd: r.get(6)?,
                },
            ))
        })?;
        for r in rows {
            let (p, row) = r?;
            known.insert(p, row);
        }
    }
    let mut seen: HashSet<String> = HashSet::new();
    'scan: for sc in src.all() {
        for path in sc.files(&roots) {
            let key = path.to_string_lossy().into_owned();
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            seen.insert(key.clone());
            let size = meta.len() as i64;
            let mtime = nanos(meta.modified().unwrap_or(UNIX_EPOCH));
            let old = known.get(&key);
            let plan = match old {
                Some(o) if o.size == size && o.mtime == mtime => {
                    // 沒變；只有「不在範圍」的要看登記是不是變了
                    let now_in = o.skip.as_deref() == Some("scope")
                        && o.cwd
                            .as_deref()
                            .and_then(|cw| repo_of(targets, cw))
                            .is_some();
                    if !now_in {
                        continue;
                    }
                    Plan::Full
                }
                Some(o) if o.session.is_some() && size >= o.size && size >= o.offset => {
                    Plan::Append
                }
                _ => Plan::Full,
            };
            let tx = match c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate) {
                Ok(tx) => tx,
                Err(e) if is_busy(&e) => {
                    st.busy = true;
                    break 'scan;
                }
                Err(e) => return Err(e.into()),
            };
            // 交易裡再看一次：別的行程可能剛更新過這份
            let fresh = file_row(&tx, &key)?;
            if fresh.as_ref() != old {
                continue; // tx 丟掉＝rollback
            }
            let full = matches!(plan, Plan::Full);
            st.added += ingest(&tx, sc, &path, Stat { size, mtime }, plan, old, targets)?;
            bump(&tx)?;
            tx.commit()?;
            if full {
                st.read += 1;
            } else {
                st.appended += 1;
            }
        }
    }
    // 不見的檔、歸屬變了的會話、Codex 改過的標題
    if !st.busy {
        let gone: Vec<(&String, &FileRow)> =
            known.iter().filter(|(p, _)| !seen.contains(*p)).collect();
        // (id, agent, sid, repo, title, cwd)
        type SessRow = (i64, String, String, Option<String>, Option<String>, String);
        let mut sessions: Vec<SessRow> = Vec::new();
        {
            let mut q = c.prepare("SELECT id, agent, sid, repo, title, cwd FROM sessions")?;
            let rows = q.query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?;
            for r in rows {
                sessions.push(r?);
            }
        }
        let codex = src.codex.as_ref();
        let mut changes: Vec<(i64, Option<String>, Option<String>, String)> = Vec::new();
        for (id, agent, sid, repo, title, cwd) in &sessions {
            let now_repo = repo_of(targets, cwd).map(str::to_string);
            let now_title = match (agent.as_str(), codex) {
                ("codex", Some(cx)) => cx.title_of(sid).map(|t| clean(&t, 300)).or(title.clone()),
                _ => title.clone(),
            };
            if &now_repo != repo || &now_title != title {
                changes.push((*id, now_repo, now_title, cwd.clone()));
            }
        }
        if !gone.is_empty() || !changes.is_empty() {
            match c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate) {
                Ok(tx) => {
                    for (p, row) in &gone {
                        if let Some(s) = row.session {
                            drop_session(&tx, s)?;
                        }
                        tx.execute("DELETE FROM files WHERE path = ?1", [p])?;
                    }
                    for (id, repo, title, cwd) in &changes {
                        match repo {
                            Some(r) => {
                                tx.execute(
                                    "UPDATE sessions SET repo = ?2, title = ?3 WHERE id = ?1",
                                    params![id, r, title],
                                )?;
                            }
                            None => {
                                // 不在任何登記的 repo 底下了：整場拿掉，檔案記成「不在範圍」（登記回來會重看）
                                drop_session(&tx, *id)?;
                                tx.execute(
                                    "UPDATE files SET session = NULL, skip = 'scope', cwd = ?2, offset = 0
                                     WHERE session = ?1",
                                    params![id, cwd],
                                )?;
                            }
                        }
                    }
                    bump(&tx)?;
                    tx.commit()?;
                }
                Err(e) if is_busy(&e) => st.busy = true,
                Err(e) => return Err(e.into()),
            }
        }
    }
    let mut q = c.prepare("SELECT agent, count(*) FROM sessions GROUP BY agent ORDER BY agent")?;
    st.by_agent = q
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
        })?
        .collect::<rusqlite::Result<_>>()?;
    st.sessions = st.by_agent.iter().map(|(_, n)| n).sum();
    st.messages =
        c.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))? as usize;
    Ok(st)
}

// ── 查詢 ────────────────────────────────────────────────────────────────────

/// 一場命中的會話。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    /// `claude`｜`codex`。
    pub agent: &'static str,
    /// 那一家的會話 id。
    pub id: String,
    pub title: String,
    pub repo: String,
    pub cwd: String,
    pub branch: Option<String>,
    pub started: Option<String>,
    /// 最後活動（本地 `YYYY-MM-DD HH:MM`）。
    pub updated: String,
    /// 命中的那則（最新的一則）是誰說的：`user`｜`assistant`｜`tool`。
    pub role: &'static str,
    /// 那則的時間（UTC ISO）。
    pub at: Option<String>,
    /// 命中處前後的一段（一行，秘密已遮）。
    pub snippet: String,
    /// 這場有幾則命中（上限內）。
    pub hits: usize,
    /// 在終端自己接續的指令（同過去的會話的 `resume`）。
    pub resume: String,
    pub path: String,
}

/// `ctx sessions --grep` 的結果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Found {
    pub hits: Vec<Hit>,
    pub index: IndexStats,
}

/// 查詢 → 詞（空白分開，每個詞都要在同一則訊息裡；不分大小寫）。
fn terms_of(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(str::to_string)
        .filter(|t| !t.is_empty())
        .collect()
}

fn like_escape(t: &str) -> String {
    let mut s = String::with_capacity(t.len() + 2);
    s.push('%');
    for c in t.chars() {
        if matches!(c, '%' | '_' | '\\') {
            s.push('\\');
        }
        s.push(c);
    }
    s.push('%');
    s
}

/// 命中處前後一段：第一個詞第一次出現的地方往前 30、往後 90 個字元。
pub fn snippet(text: &str, terms: &[String]) -> String {
    let chars: Vec<char> = text.chars().collect();
    let low: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let hit = terms
        .iter()
        .filter_map(|t| {
            let tl: Vec<char> = t
                .chars()
                .map(|c| c.to_lowercase().next().unwrap_or(c))
                .collect();
            if tl.is_empty() || tl.len() > low.len() {
                return None;
            }
            (0..=low.len() - tl.len()).find(|&i| low[i..i + tl.len()] == tl[..])
        })
        .min()
        .unwrap_or(0);
    let from = hit.saturating_sub(30);
    let to = (hit + 90).min(chars.len());
    let body: String = chars[from..to].iter().collect();
    let mut s = one_line(&body, 200);
    if from > 0 {
        s = format!("…{s}");
    }
    if to < chars.len() && !s.ends_with('…') {
        s.push('…');
    }
    s
}

fn local_minute(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// 全文搜尋過去的會話：先增量更新 `.state/sessions.db`（全部登記的 repo），再查範圍（`group`／`repos`，
/// 同 [`crate::history::past_sessions`]）內的會話。每場一筆、最近活動的在前，最多 `limit` 場。
pub fn search(
    spine_dir: &Path,
    src: &Sources,
    group: Option<&str>,
    repos: Option<&[String]>,
    query: &str,
    limit: usize,
) -> Result<Found> {
    let terms = terms_of(query);
    if terms.is_empty() {
        return Err(Error::invalid("要給查詢的字（--grep <文字>）"));
    }
    let all = registry::load(spine_dir)?;
    let targets = targets_of(&all.repos);
    let (_, entries) = registry::resolve_group(spine_dir, group, repos)?;
    let scope: Vec<String> = entries.iter().map(|e| e.id()).collect();
    let scope_json = serde_json::to_string(&scope)?;
    let long: Vec<&String> = terms
        .iter()
        .filter(|t| t.chars().count() >= TRIGRAM_MIN)
        .collect();
    let short: Vec<String> = terms
        .iter()
        .filter(|t| t.chars().count() < TRIGRAM_MIN)
        .map(|t| like_escape(t))
        .collect();
    let mut sql = String::from(
        "SELECT s.agent, s.sid, s.title, s.prompt, s.repo, s.cwd, s.branch, s.started, s.mtime, s.path,
                m.role, m.at, f.text, m.session
         FROM messages_fts f JOIN messages m ON m.id = f.rowid JOIN sessions s ON s.id = m.session
         WHERE s.repo IN (SELECT value FROM json_each(?1))",
    );
    let mut args: Vec<String> = vec![scope_json];
    if !long.is_empty() {
        let expr = long
            .iter()
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ");
        args.push(expr);
        sql.push_str(&format!(" AND messages_fts MATCH ?{}", args.len()));
    }
    for l in &short {
        args.push(l.clone());
        sql.push_str(&format!(" AND f.text LIKE ?{} ESCAPE '\\'", args.len()));
    }
    sql.push_str(&format!(
        " ORDER BY s.mtime DESC, m.seq DESC LIMIT {ROW_CAP}"
    ));
    with_db(spine_dir, |c| {
        let index = refresh(c, src, &targets)?;
        let mut q = c.prepare(&sql)?;
        let mut rows = q.query(rusqlite::params_from_iter(args.iter()))?;
        let mut hits: Vec<Hit> = Vec::new();
        let mut at_of: HashMap<i64, usize> = HashMap::new();
        while let Some(r) = rows.next()? {
            let sess: i64 = r.get(13)?;
            if let Some(&i) = at_of.get(&sess) {
                hits[i].hits += 1;
                continue;
            }
            if hits.len() >= limit {
                continue; // 還要數完前面幾場的命中數
            }
            let agent = AgentKind::parse(&r.get::<_, String>(0)?).unwrap_or(AgentKind::ClaudeCode);
            let id: String = r.get(1)?;
            let cwd: String = r.get(5)?;
            let title: Option<String> = r.get(2)?;
            let prompt: Option<String> = r.get(3)?;
            let text: String = r.get(12)?;
            let role = Role::parse(&r.get::<_, String>(10)?).unwrap_or(Role::User);
            at_of.insert(sess, hits.len());
            hits.push(Hit {
                agent: agent.name(),
                resume: crate::history::resume_shell_command(agent, &id, &cwd),
                title: title
                    .or_else(|| prompt.map(|p| one_line(&p, 120)))
                    .unwrap_or_default(),
                repo: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                branch: r.get(6)?,
                started: r.get(7)?,
                updated: local_minute(from_nanos(r.get(8)?)),
                path: r.get(9)?,
                role: role.as_str(),
                at: r.get(11)?,
                snippet: snippet(&text, &terms),
                hits: 1,
                id,
                cwd,
            });
        }
        Ok(Found { hits, index })
    })
}
