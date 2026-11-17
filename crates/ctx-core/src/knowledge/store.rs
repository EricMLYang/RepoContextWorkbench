//! 知識索引的儲存：`.state/knowledge.db`（SQLite，Rust 才有，N-51，2026-11-18）。
//!
//! 以前整份索引是一個 JSON（真資料 100 MB 上下），有檔改過就整份重寫、每次查詢整份讀進來解析。
//! 這裡只把「儲存」換掉：SQLite 當可增量更新、可局部讀取的倒排索引。斷詞、BM25、加權、片段都還是
//! `knowledge` 的算法（不用 FTS5，也不用它的 bm25()），所以結果跟在記憶體裡從頭建一模一樣。
//!
//! ## schema（`PRAGMA user_version` ＝ [`DB_SCHEMA_VERSION`]；不符就整個重建）
//! - `docs(id, repo, path, rel, mtime, secret, len, hlen, terms)`：一份檔一列，`(repo, path)` 唯一
//!   （`path` 是絕對路徑；同一個檔被兩個 repo 列到就各一列）。`mtime` 跟現在的 stat 一樣才算沒變。
//!   秘密檔只記 `secret=1`（不進 posting，也不會再讀它，直到 mtime 變了）。
//!   `terms`＝這份檔的 (詞 id, tf) 清單（varint；內文一段、標題一段）：改檔／刪檔時用它刪舊 posting，
//!   要整份 doc 的呼叫端（反芻、相關 repo 建議）也從這裡讀，不必掃 posting 表。
//! - `terms(id, term)`：詞表。刪檔不回收沒人用的詞（只多佔一點空間；重建時清掉）。
//! - `body(term, doc, tf)`／`head(term, doc, tf)`：posting（內文／標題＋frontmatter＋檔名），主鍵 (term, doc)。
//!   查詢只撈查詢詞的這幾列。
//! - `meta('generation')`：每次寫入交易加一（測試用來確認「沒變動時不寫」）。
//!
//! ## 多個行程同時用（CLI、多支 `ctx mcp`、App）
//! - WAL：讀的人不擋寫的人、寫的人不擋讀的人；同時只有一個寫的人。`synchronous=NORMAL`：這是重建得回來的快取，
//!   當機最多丟掉最後一筆交易。
//! - 一次查詢＝ (1) 自動提交讀出 docs 表比 mtime →(2) 有新增／變動／刪除才開 `BEGIN IMMEDIATE` 寫入交易：
//!   交易裡每份檔再查一次，別的行程已經寫好的（mtime 一樣）就跳過，所以兩個行程同時更新同一批檔也不會重複或打架
//!   →(3) 另開一個讀交易（同一個快照）撈 docs 長度與查詢詞的 posting。
//! - busy timeout 3 秒（[`BUSY`]）。拿不到寫入鎖（別人在寫、例如冷建）就**不寫**，這次查詢照樣用剛算好的
//!   那幾份（記憶體裡）算分——結果一樣，只是下次還要再算一次。所以別人在寫不會讓查詢失敗。
//! - 讀交易裡發現某份檔的列跟 (1) 看到的不一樣（別的行程剛好在中間改了它）：那份就當場重讀重算，不混用。
//!
//! ## 壞掉的時候
//! 開不起來、不是資料庫、損毀、表不見、`terms` 解不開——除了 busy 以外的錯誤都當「壞了」：刪掉
//! `knowledge.db`（連同 `-wal`、`-shm`）重建一次；再不行就回 `None`，呼叫端退回在記憶體裡從頭建。
//! 搜尋永遠不會因為索引庫失敗。版本不符（舊版或新版的 ctx 建的）在 [`ensure_schema`] 整個重建。
//!
//! 第一次成功建好（或確認已是最新）後刪掉舊的 `.state/knowledge_index.json`（Python 版的快取格式，Rust 不再讀寫）。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rusqlite::{params, Connection, ErrorCode, OpenFlags, OptionalExtension, Statement};

use super::{list_files, par_map, read_rec, Doc, Listing, Rec, ScoreInput};
use crate::fsutil::mtime_secs;
use crate::pack::RepoRef;

/// 索引庫格式版本（`PRAGMA user_version`）。斷詞、欄位、編碼有改就加一：舊庫整個重建。
pub const DB_SCHEMA_VERSION: i64 = 1;
const DB_NAME: &str = "knowledge.db";
/// Python 版的快取（Rust 以前也共用）。建好 db 後刪掉。
const LEGACY_JSON: &str = "knowledge_index.json";
/// 等別的行程放掉鎖最多多久。寫入拿不到就不寫（見檔頭）；讀幾乎不會等（WAL）。
const BUSY: Duration = Duration::from_secs(3);
/// 一次寫超過這麼多份檔（冷建、版本重建）之後把 WAL 截短，不然 -wal 檔會留在跟整個庫一樣大。
const BIG_WRITE: usize = 500;

const SCHEMA: &str = "
CREATE TABLE meta(key TEXT PRIMARY KEY, value INTEGER NOT NULL) WITHOUT ROWID;
CREATE TABLE docs(
  id INTEGER PRIMARY KEY,
  repo TEXT NOT NULL,
  path TEXT NOT NULL,
  rel TEXT NOT NULL,
  mtime REAL NOT NULL,
  secret INTEGER NOT NULL,
  len INTEGER NOT NULL,
  hlen INTEGER NOT NULL,
  terms BLOB NOT NULL,
  UNIQUE(repo, path)
);
CREATE TABLE terms(id INTEGER PRIMARY KEY, term TEXT NOT NULL UNIQUE);
CREATE TABLE body(term INTEGER NOT NULL, doc INTEGER NOT NULL, tf INTEGER NOT NULL,
  PRIMARY KEY(term, doc)) WITHOUT ROWID;
CREATE TABLE head(term INTEGER NOT NULL, doc INTEGER NOT NULL, tf INTEGER NOT NULL,
  PRIMARY KEY(term, doc)) WITHOUT ROWID;
INSERT INTO meta VALUES('generation', 0);
";
const TABLES: [&str; 5] = ["meta", "docs", "terms", "body", "head"];

/// 失敗的種類：決定要不要刪庫重來。
enum Fail {
    Sql(rusqlite::Error),
    /// `terms` 欄解不開之類（庫的內容不一致）＝當成壞了。
    Broken,
    /// 讀到一半格式版本被別的行程換掉：重來一次（不刪庫）。
    Moved,
    /// 連 `.state/` 都建不起來：刪庫也沒用。
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

fn db_path(spine_dir: &Path) -> PathBuf {
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

fn remove_legacy_json(spine_dir: &Path) {
    let p = spine_dir.join(".state").join(LEGACY_JSON);
    if p.exists() {
        let _ = std::fs::remove_file(p);
    }
}

fn open(spine_dir: &Path) -> Result<Connection, Fail> {
    let p = db_path(spine_dir);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|_| Fail::Io)?;
    }
    let mut c = Connection::open(&p)?;
    c.busy_timeout(BUSY)?;
    // WAL 是寫進檔頭的持久設定；已經是 WAL 時這行只是讀（不搶鎖）
    let _: String = c.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
    c.pragma_update(None, "synchronous", "NORMAL")?;
    c.pragma_update(None, "journal_size_limit", 16 * 1024 * 1024)?;
    // 頁快取 32 MB（預設 2 MB）：冷建時詞表的唯一索引是亂序插入；只是上限，用多少佔多少
    c.pragma_update(None, "cache_size", -32 * 1024)?;
    ensure_schema(&mut c)?;
    Ok(c)
}

fn user_version(c: &Connection) -> rusqlite::Result<i64> {
    c.pragma_query_value(None, "user_version", |r| r.get(0))
}

/// 格式版本不符（新庫是 0）就在寫入交易裡整個重建。交易裡再看一次：別的行程可能剛建好。
fn ensure_schema(c: &mut Connection) -> Result<(), Fail> {
    if user_version(c)? == DB_SCHEMA_VERSION {
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

/// 開庫跑 `f`；壞了（busy 以外的錯）就刪庫重來一次，還不行回 None（呼叫端退回記憶體）。
fn with_db<T>(
    spine_dir: &Path,
    mut f: impl FnMut(&mut Connection) -> Result<T, Fail>,
) -> Option<T> {
    for attempt in 0..2 {
        let r = open(spine_dir).and_then(|mut c| f(&mut c));
        match r {
            Ok(v) => return Some(v),
            Err(Fail::Moved) => continue,
            Err(Fail::Sql(e)) if attempt == 0 && !is_busy(&e) => remove_db(spine_dir),
            Err(Fail::Broken) if attempt == 0 => remove_db(spine_dir),
            Err(_) => break,
        }
    }
    FALLBACKS.fetch_add(1, Ordering::Relaxed);
    None
}

/// 索引庫用不了、退回在記憶體裡從頭建的次數（行程內；測試用來確認真的走了索引庫）。
static FALLBACKS: AtomicU64 = AtomicU64::new(0);

#[doc(hidden)]
pub fn index_fallbacks() -> u64 {
    FALLBACKS.load(Ordering::Relaxed)
}

/// 每次寫入交易加一的計數（測試用；沒有庫或讀不到＝None）。
#[doc(hidden)]
pub fn index_generation(spine_dir: &Path) -> Option<i64> {
    let c =
        Connection::open_with_flags(db_path(spine_dir), OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    c.query_row("SELECT value FROM meta WHERE key = 'generation'", [], |r| {
        r.get(0)
    })
    .ok()
}

// ── terms 欄的編碼：varint(n) (varint(詞 id) varint(tf))×n，內文一段、標題一段 ─────────────

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn get_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *buf.get(*pos)?;
        *pos += 1;
        v |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

type Pairs = Vec<(i64, u64)>;

fn encode(body: &[(i64, u64)], head: &[(i64, u64)]) -> Vec<u8> {
    let mut out = Vec::with_capacity((body.len() + head.len()) * 4 + 4);
    for part in [body, head] {
        put_varint(&mut out, part.len() as u64);
        for &(t, n) in part {
            put_varint(&mut out, t as u64);
            put_varint(&mut out, n);
        }
    }
    out
}

fn decode(buf: &[u8]) -> Result<(Pairs, Pairs), Fail> {
    let mut pos = 0;
    let mut part = || -> Option<Pairs> {
        let n = get_varint(buf, &mut pos)? as usize;
        let mut v = Vec::with_capacity(n.min(buf.len()));
        for _ in 0..n {
            let t = get_varint(buf, &mut pos)? as i64;
            v.push((t, get_varint(buf, &mut pos)?));
        }
        Some(v)
    };
    let body = part().ok_or(Fail::Broken)?;
    let head = part().ok_or(Fail::Broken)?;
    Ok((body, head))
}

// ── 增量更新 ─────────────────────────────────────────────────────────────────

/// docs 表的一列（比對 mtime、拿長度用）。
struct Row {
    id: i64,
    repo: String,
    path: String,
    mtime: f64,
    secret: bool,
    len: i64,
    hlen: i64,
}

fn load_rows(c: &Connection) -> Result<Vec<Row>, Fail> {
    let mut st = c.prepare("SELECT id, repo, path, mtime, secret, len, hlen FROM docs")?;
    let rows = st.query_map([], |r| {
        Ok(Row {
            id: r.get(0)?,
            repo: r.get(1)?,
            path: r.get(2)?,
            mtime: r.get(3)?,
            secret: r.get::<_, i64>(4)? != 0,
            len: r.get(5)?,
            hlen: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 這次查詢裡的一份檔：列檔序號、stat 到的 mtime，以及索引從哪來。
struct Slot {
    i: usize,
    mtime: f64,
    src: Src,
}

enum Src {
    /// 索引庫裡那一列（mtime 對得上）
    Db(i64),
    /// 剛讀檔算的（新增、改過，或讀交易裡發現對不上）
    Fresh(Rec),
}

/// 列檔、stat 的結果（跟庫無關，刪庫重來時不必重做）。
struct Scan<'a> {
    listing: Listing<'a>,
    keys: Vec<String>,
    mtimes: Vec<Option<f64>>,
}

fn scan(entries: &[RepoRef]) -> Scan<'_> {
    let listing = list_files(entries);
    let n = listing.files.len();
    let keys = (0..n).map(|i| listing.key(i)).collect();
    let mtimes = par_map(n, |i| mtime_secs(listing.file(i)).ok());
    Scan {
        listing,
        keys,
        mtimes,
    }
}

/// 比 mtime → 只讀新增／變動的檔 → 有新增／變動／刪除才寫庫。回傳範圍內的檔（照列檔順序；秘密檔、讀不到的不算）。
fn refresh(
    c: &mut Connection,
    spine_dir: &Path,
    entries: &[RepoRef],
    sc: &Scan,
) -> Result<Vec<Slot>, Fail> {
    let rows = load_rows(c)?;
    let by_key: HashMap<(&str, &str), &Row> = rows
        .iter()
        .map(|r| ((r.repo.as_str(), r.path.as_str()), r))
        .collect();
    enum Plan {
        Skip,
        Db(i64),
        Need,
    }
    let plans: Vec<Plan> = (0..sc.keys.len())
        .map(|i| {
            let Some(m) = sc.mtimes[i] else {
                return Plan::Skip;
            };
            match by_key.get(&(sc.listing.repo(i), sc.keys[i].as_str())) {
                Some(r) if r.mtime == m && r.secret => Plan::Skip,
                Some(r) if r.mtime == m => Plan::Db(r.id),
                _ => Plan::Need,
            }
        })
        .collect();
    let need: Vec<usize> = (0..plans.len())
        .filter(|&i| matches!(plans[i], Plan::Need))
        .collect();
    let fresh: Vec<Option<Rec>> = par_map(need.len(), |k| read_rec(sc.listing.file(need[k])));

    // 刪：列過的 repo 裡已經不在的檔；registry 已經沒有的 repo 的所有檔
    let listed_repos: HashSet<&str> = sc
        .listing
        .roots
        .iter()
        .map(|(e, _)| e.id.as_str())
        .collect();
    let listed_keys: HashSet<(&str, &str)> = (0..sc.keys.len())
        .map(|i| (sc.listing.repo(i), sc.keys[i].as_str()))
        .collect();
    let known: Option<HashSet<String>> = crate::registry::load(spine_dir)
        .ok()
        .filter(|r| !r.repos.is_empty())
        .map(|r| {
            let mut ids: HashSet<String> = r.repos.iter().map(|x| x.id()).collect();
            ids.extend(entries.iter().map(|e| e.id.clone()));
            ids
        });
    let dels: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| (r.repo.as_str(), r.path.as_str()))
        .filter(|k| {
            (listed_repos.contains(k.0) && !listed_keys.contains(k))
                || known.as_ref().is_some_and(|ids| !ids.contains(k.0))
        })
        .collect();

    let items: Vec<Item> = need
        .iter()
        .zip(&fresh)
        .filter_map(|(&i, rec)| {
            Some(Item {
                repo: sc.listing.repo(i),
                path: &sc.keys[i],
                rel: sc.listing.rel(i),
                mtime: sc.mtimes[i]?,
                rec: rec.as_ref()?,
            })
        })
        .collect();
    let up_to_date = if items.is_empty() && dels.is_empty() {
        true
    } else {
        write(c, &items, &dels)?
    };
    if up_to_date {
        remove_legacy_json(spine_dir);
    }

    let mut fresh = need
        .iter()
        .copied()
        .zip(fresh)
        .collect::<HashMap<usize, Option<Rec>>>();
    Ok(plans
        .into_iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let src = match p {
                Plan::Skip => return None,
                Plan::Db(id) => Src::Db(id),
                Plan::Need => Src::Fresh(fresh.remove(&i).flatten().filter(|r| !r.secret)?),
            };
            Some(Slot {
                i,
                mtime: sc.mtimes[i]?,
                src,
            })
        })
        .collect())
}

/// 要寫進庫的一份檔。
struct Item<'a> {
    repo: &'a str,
    path: &'a str,
    rel: String,
    mtime: f64,
    rec: &'a Rec,
}

/// 刪掉一份檔的舊 posting（照它 `terms` 欄記的詞）。
fn unpost(
    del_body: &mut Statement,
    del_head: &mut Statement,
    id: i64,
    blob: &[u8],
) -> Result<(), Fail> {
    let (body, head) = decode(blob)?;
    for (t, _) in body {
        del_body.execute(params![t, id])?;
    }
    for (t, _) in head {
        del_head.execute(params![t, id])?;
    }
    Ok(())
}

/// 一次插多列（冷建時幾百萬列，一列一次呼叫的開銷佔大半）。
fn insert_postings(tx: &Connection, table: &str, rows: &[(i64, i64, u64)]) -> Result<(), Fail> {
    const PER: usize = 256; // 3 × 256 個參數，低於 SQLite 的上限
    let sql = |k: usize| {
        let vals = vec!["(?, ?, ?)"; k].join(", ");
        format!("INSERT INTO {table}(term, doc, tf) VALUES {vals}")
    };
    let mut full = tx.prepare(&sql(PER))?;
    let (full_chunks, rest) = rows.as_chunks::<PER>();
    for ch in full_chunks {
        let ps: Vec<i64> = ch.iter().flat_map(|&(t, d, n)| [t, d, n as i64]).collect();
        full.execute(rusqlite::params_from_iter(ps))?;
    }
    if !rest.is_empty() {
        let ps: Vec<i64> = rest
            .iter()
            .flat_map(|&(t, d, n)| [t, d, n as i64])
            .collect();
        tx.prepare(&sql(rest.len()))?
            .execute(rusqlite::params_from_iter(ps))?;
    }
    Ok(())
}

/// 一個寫入交易：刪掉 `dels`、寫入 `items`。拿不到寫入鎖＝Ok(false)（不寫，見檔頭）。
fn write(c: &mut Connection, items: &[Item], dels: &[(&str, &str)]) -> Result<bool, Fail> {
    let tx = match c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate) {
        Ok(tx) => tx,
        Err(e) if is_busy(&e) => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    {
        let mut find =
            tx.prepare("SELECT id, mtime, terms FROM docs WHERE repo = ?1 AND path = ?2")?;
        let mut del_body = tx.prepare("DELETE FROM body WHERE term = ?1 AND doc = ?2")?;
        let mut del_head = tx.prepare("DELETE FROM head WHERE term = ?1 AND doc = ?2")?;
        let mut del_doc = tx.prepare("DELETE FROM docs WHERE id = ?1")?;
        let mut ins_doc = tx.prepare(
            "INSERT INTO docs(repo, path, rel, mtime, secret, len, hlen, terms) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        let mut upd_doc = tx.prepare(
            "UPDATE docs SET rel = ?2, mtime = ?3, secret = ?4, len = ?5, hlen = ?6, terms = ?7 WHERE id = ?1",
        )?;
        let mut term_get = tx.prepare("SELECT id FROM terms WHERE term = ?1")?;
        let mut term_ins = tx.prepare("INSERT INTO terms(term) VALUES(?1)")?;
        let found = |find: &mut Statement, repo: &str, path: &str| {
            find.query_row(params![repo, path], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            })
            .optional()
        };

        for (repo, path) in dels {
            if let Some((id, _, blob)) = found(&mut find, repo, path)? {
                unpost(&mut del_body, &mut del_head, id, &blob)?;
                del_doc.execute([id])?;
            }
        }

        let mut ids: HashMap<String, i64> = HashMap::new();
        let mut term_id = |t: &str| -> Result<i64, Fail> {
            if let Some(&id) = ids.get(t) {
                return Ok(id);
            }
            let id = match term_get.query_row([t], |r| r.get::<_, i64>(0)).optional()? {
                Some(id) => id,
                None => term_ins.insert([t])?,
            };
            ids.insert(t.to_string(), id);
            Ok(id)
        };
        let mut body_rows: Vec<(i64, i64, u64)> = Vec::new();
        let mut head_rows: Vec<(i64, i64, u64)> = Vec::new();
        for it in items {
            let old = found(&mut find, it.repo, it.path)?;
            if old.as_ref().is_some_and(|(_, m, _)| *m == it.mtime) {
                continue; // 別的行程已經寫好這一版
            }
            let body: Pairs = it
                .rec
                .tf
                .iter()
                .map(|(t, n)| Ok((term_id(t)?, *n)))
                .collect::<Result<_, Fail>>()?;
            let head: Pairs = it
                .rec
                .htf
                .iter()
                .map(|(t, n)| Ok((term_id(t)?, *n)))
                .collect::<Result<_, Fail>>()?;
            let blob = encode(&body, &head);
            let (secret, len, hlen) = (it.rec.secret as i64, it.rec.len as i64, it.rec.hlen as i64);
            let id = match old {
                Some((id, _, old_blob)) => {
                    unpost(&mut del_body, &mut del_head, id, &old_blob)?;
                    upd_doc.execute(params![id, it.rel, it.mtime, secret, len, hlen, blob])?;
                    id
                }
                None => ins_doc.insert(params![
                    it.repo, it.path, it.rel, it.mtime, secret, len, hlen, blob
                ])?,
            };
            body_rows.extend(body.into_iter().map(|(t, n)| (t, id, n)));
            head_rows.extend(head.into_iter().map(|(t, n)| (t, id, n)));
        }
        // 照主鍵順序插：冷建時幾百萬列，順序插比亂序快很多
        body_rows.sort_unstable();
        head_rows.sort_unstable();
        insert_postings(&tx, "body", &body_rows)?;
        insert_postings(&tx, "head", &head_rows)?;
        tx.execute(
            "UPDATE meta SET value = value + 1 WHERE key = 'generation'",
            [],
        )?;
    }
    tx.commit()?;
    if items.len() > BIG_WRITE {
        // 截短 WAL（別人正在讀就做不了，沒關係）
        let _ = c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    }
    Ok(true)
}

// ── 讀 ───────────────────────────────────────────────────────────────────────

/// 讀交易裡確認每份 `Db` 檔的列還是 refresh 時看到的那一列；不是（別的行程剛好在中間改了）就當場重讀重算。
/// 回傳 id → 列（長度用）。
fn verify(tx: &Connection, sc: &Scan, slots: &mut Vec<Slot>) -> Result<HashMap<i64, Row>, Fail> {
    if user_version(tx)? != DB_SCHEMA_VERSION {
        return Err(Fail::Moved);
    }
    let rows: HashMap<i64, Row> = load_rows(tx)?.into_iter().map(|r| (r.id, r)).collect();
    slots.retain_mut(|s| {
        let Src::Db(id) = s.src else {
            return true;
        };
        let ok = rows.get(&id).is_some_and(|r| {
            r.mtime == s.mtime
                && !r.secret
                && r.repo == sc.listing.repo(s.i)
                && r.path == sc.keys[s.i]
        });
        if ok {
            return true;
        }
        let file = sc.listing.file(s.i);
        match (mtime_secs(file), read_rec(file)) {
            (Ok(m), Some(rec)) if !rec.secret => {
                s.mtime = m;
                s.src = Src::Fresh(rec);
                true
            }
            _ => false,
        }
    });
    Ok(rows)
}

/// 搜尋用：增量更新後，撈範圍內每份檔的長度與查詢詞的 posting。`keep`＝排除查詢來源檔。
pub(super) fn score_input(
    spine_dir: &Path,
    entries: &[RepoRef],
    qcount: &[(String, u64)],
    keep: &dyn Fn(&str) -> bool,
) -> Option<ScoreInput> {
    let sc = scan(entries);
    with_db(spine_dir, |c| {
        let mut slots = refresh(c, spine_dir, entries, &sc)?;
        slots.retain(|s| keep(&sc.keys[s.i]));
        let tx = c.transaction()?;
        let rows = verify(&tx, &sc, &mut slots)?;
        let q = qcount.len();
        let mut input = ScoreInput {
            metas: Vec::with_capacity(slots.len()),
            len: Vec::with_capacity(slots.len()),
            hlen: Vec::with_capacity(slots.len()),
            body: vec![HashMap::new(); q],
            head: vec![HashMap::new(); q],
        };
        let qidx: HashMap<&str, usize> = qcount
            .iter()
            .enumerate()
            .map(|(t, (s, _))| (s.as_str(), t))
            .collect();
        let mut by_id: HashMap<i64, Vec<usize>> = HashMap::new();
        for (d, s) in slots.iter().enumerate() {
            input.metas.push(sc.listing.meta(s.i, s.mtime));
            match &s.src {
                Src::Db(id) => {
                    let r = &rows[id]; // verify 保證在
                    input.len.push(r.len as f64);
                    input.hlen.push(r.hlen as f64);
                    by_id.entry(*id).or_default().push(d);
                }
                Src::Fresh(rec) => {
                    input.len.push(rec.len as f64);
                    input.hlen.push(rec.hlen as f64);
                    for (term, n) in &rec.tf {
                        if let Some(&t) = qidx.get(term.as_str()) {
                            input.body[t].insert(d, *n as f64);
                        }
                    }
                    for (term, n) in &rec.htf {
                        if let Some(&t) = qidx.get(term.as_str()) {
                            input.head[t].insert(d, *n as f64);
                        }
                    }
                }
            }
        }
        {
            let mut term_get = tx.prepare("SELECT id FROM terms WHERE term = ?1")?;
            let mut post_body = tx.prepare("SELECT doc, tf FROM body WHERE term = ?1")?;
            let mut post_head = tx.prepare("SELECT doc, tf FROM head WHERE term = ?1")?;
            for (t, (term, _)) in qcount.iter().enumerate() {
                let Some(tid) = term_get
                    .query_row([term], |r| r.get::<_, i64>(0))
                    .optional()?
                else {
                    continue;
                };
                for (st, out) in [
                    (&mut post_body, &mut input.body[t]),
                    (&mut post_head, &mut input.head[t]),
                ] {
                    let hits =
                        st.query_map([tid], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
                    for h in hits {
                        let (doc, n) = h?;
                        for &d in by_id.get(&doc).map(Vec::as_slice).unwrap_or_default() {
                            out.insert(d, n as f64);
                        }
                    }
                }
            }
        }
        tx.commit()?;
        Ok(input)
    })
}

/// 要整份 doc 的呼叫端（反芻、相關 repo 建議）：增量更新後讀出範圍內每份檔的完整 tf。
pub(super) fn full_docs(spine_dir: &Path, entries: &[RepoRef]) -> Option<Vec<Doc>> {
    let sc = scan(entries);
    with_db(spine_dir, |c| {
        let mut slots = refresh(c, spine_dir, entries, &sc)?;
        let tx = c.transaction()?;
        let rows = verify(&tx, &sc, &mut slots)?;
        let mut blobs: HashMap<i64, Vec<u8>> = HashMap::new();
        let mut names: HashMap<i64, String> = HashMap::new();
        if slots.iter().any(|s| matches!(s.src, Src::Db(_))) {
            let mut st = tx.prepare("SELECT id, terms FROM docs WHERE secret = 0")?;
            for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))? {
                let (id, b) = r?;
                blobs.insert(id, b);
            }
            let mut st = tx.prepare("SELECT id, term FROM terms")?;
            for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
                let (id, t) = r?;
                names.insert(id, t);
            }
        }
        tx.commit()?;
        let to_map = |pairs: Pairs| -> Result<HashMap<String, f64>, Fail> {
            pairs
                .into_iter()
                .map(|(t, n)| Ok((names.get(&t).ok_or(Fail::Broken)?.clone(), n as f64)))
                .collect()
        };
        let mut docs = Vec::with_capacity(slots.len());
        for s in &slots {
            let meta = sc.listing.meta(s.i, s.mtime);
            let doc = match &s.src {
                Src::Fresh(rec) => rec.clone().into_doc(meta),
                Src::Db(id) => {
                    let (body, head) = decode(blobs.get(id).ok_or(Fail::Broken)?)?;
                    let r = &rows[id]; // verify 保證在
                    Doc {
                        len: r.len as f64,
                        hlen: r.hlen as f64,
                        tf: to_map(body)?,
                        htf: to_map(head)?,
                        repo: meta.repo,
                        path: meta.path,
                        rel: meta.rel,
                        mtime: meta.mtime,
                    }
                }
            };
            docs.push(doc);
        }
        Ok(docs)
    })
}
