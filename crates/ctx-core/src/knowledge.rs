//! 知識相關度（2026-09-25 Agent 友善輪）——「這段文字跟哪些 repo 的哪些檔強相關」。對應 `repo_context/knowledge.py`。
//!
//! 痛點：card_notes 的文章與書，要能很快比對出跟哪些 repo 強相關。先用最簡單、可解釋、零依賴的做法：
//! BM25 over 各 repo 的 *.md（選料沿用 `pack::doc_files`，尊重 .gitignore）。中文用字元 bigram、
//! 英數用小寫單字，不需要斷詞器也不需要 embedding。
//!
//! 紀律：
//! - 洩密哨兵照走——長得像憑證的檔不進索引，也不會出現在片段裡（公私分界）。
//! - 回傳帶分數與命中片段的「依據」，agent 要引用時說得出來自哪個 repo 哪份檔哪一行。
//! - 索引依檔案 mtime 快取在 `.state/knowledge_index.json`（.state/ 已 gitignore；格式與 Python 共用）。
//!
//! 找得「準」（跨 repo 參考輪）：
//! - 標題／frontmatter／檔名另建一份索引（htf），命中加權：md 有結構，標題命中比內文命中準得多。
//! - boosts：呼叫端（agentapi）依主場算好——有關係的 repo 加分、落在 export 內加分；
//!   近 30 天改過的檔小幅加分。每筆結果帶 why（為什麼排這裡），agent 才判斷得了要不要信。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{json, Value};
use std::sync::LazyLock;

use crate::fsutil::{expanduser, mtime_secs, now_secs, read_text_lossy, resolve};
use crate::pack::{self, RepoRef};
use crate::pystr::{splitlines, take_chars};

static WORD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[a-z0-9][a-z0-9_\-\.]*[a-z0-9]|[a-z0-9]").expect("寫死的 regex"));
static CJK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\x{3400}-\x{9FFF}\x{F900}-\x{FAFF}]+").expect("寫死的 regex"));
const K1: f64 = 1.5;
const B: f64 = 0.75;
/// 單檔上限：超大檔只索引前段（避免一本書的全文拖垮查詢）。
const MAX_CHARS: usize = 200_000;
/// 快取格式版本：舊快取沒有 htf，版本不符就重算。
const INDEX_VERSION: i64 = 2;
/// 標題／frontmatter／檔名命中的權重（相對內文）。
const HEAD_WEIGHT: f64 = 2.0;
const RECENT_DAYS: f64 = 30.0;
const RECENT_BOOST: f64 = 1.1;
const STOP: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "are", "is", "of", "to", "in", "on", "md", "a",
    "an", "or", "be", "it", "as", "by", "at",
];

/// 對應 `knowledge.tokenize`：英數＝小寫單字（去停用字）；中文＝字元 bigram（單字元段落保留單字）。
pub fn tokenize(text: &str) -> Vec<String> {
    let text = text.to_lowercase();
    let mut toks: Vec<String> = WORD_RE
        .find_iter(&text)
        .map(|m| m.as_str())
        .filter(|w| !STOP.contains(w))
        .map(String::from)
        .collect();
    for seg in CJK_RE.find_iter(&text) {
        let chars: Vec<char> = seg.as_str().chars().collect();
        if chars.len() == 1 {
            toks.push(chars[0].to_string());
        } else {
            toks.extend(chars.windows(2).map(|w| w.iter().collect::<String>()));
        }
    }
    toks
}

/// 對應 `knowledge.heading_text`：標題行＋開頭 frontmatter＋檔名（去副檔名）——結構化訊號，另建索引加權。
pub fn heading_text(text: &str, name: &str) -> String {
    let lines = splitlines(text);
    let mut out: Vec<String> = Vec::new();
    if !name.is_empty() {
        out.push(
            Path::new(name)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
    }
    if lines.first().is_some_and(|l| l.trim() == "---") {
        for ln in lines.iter().skip(1).take(59) {
            if ln.trim() == "---" {
                break;
            }
            out.push((*ln).to_string());
        }
    }
    out.extend(
        lines
            .iter()
            .filter(|l| l.starts_with('#'))
            .map(|l| l.trim_start_matches('#').trim().to_string()),
    );
    out.join("\n")
}

fn cache_path(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("knowledge_index.json")
}

/// 快取一筆（`.state/knowledge_index.json` 的 value；欄位同 Python）。只在命中時解析。
#[derive(Debug, Deserialize)]
struct CachedRec {
    mtime: Option<f64>,
    #[serde(default)]
    secret: bool,
    #[serde(default)]
    tf: HashMap<String, f64>,
    #[serde(default)]
    len: f64,
    #[serde(default)]
    htf: HashMap<String, f64>,
    #[serde(default)]
    hlen: f64,
    v: Option<i64>,
}

/// 整份快取：路徑 → 原樣保留的 JSON（不展開成 Value 樹；只解析範圍內命中的那幾筆）。
type Cache = BTreeMap<String, Box<RawValue>>;

/// 行程內記住上次讀到的快取（MCP server／桌面 App 是常駐行程，同一份檔不用每次重讀重解析）。
/// 檔案的 (mtime, 大小) 變了＝別的行程（Python 版、另一個 ctx）寫過 → 重讀。
struct Memo {
    path: PathBuf,
    stamp: Option<(SystemTime, u64)>,
    cache: Cache,
}

static MEMO: LazyLock<Mutex<Option<Memo>>> = LazyLock::new(|| Mutex::new(None));

fn file_stamp(p: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// 讀快取：行程內有同一份（stamp 沒變）就直接拿走，否則讀檔。壞檔＝空（同 Python）。
fn take_cache(memo: &mut Option<Memo>, spine_dir: &Path) -> Cache {
    let p = cache_path(spine_dir);
    let stamp = file_stamp(&p);
    if let Some(m) = memo.take() {
        if m.path == p && m.stamp.is_some() && m.stamp == stamp {
            return m.cache;
        }
    }
    std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str::<Cache>(&s).ok())
        .unwrap_or_default()
}

/// 寫快取（先寫暫存檔再改名，別的行程不會讀到半份）。寫不進去不影響查詢。回傳寫完後的 stamp。
fn save_cache(spine_dir: &Path, cache: &Cache) -> Option<(SystemTime, u64)> {
    let p = cache_path(spine_dir);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let s = serde_json::to_string(cache).ok()?;
    let tmp = p.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, s).ok()?;
    std::fs::rename(&tmp, &p).ok()?;
    file_stamp(&p)
}

/// 有序計數（插入順序＝首次出現順序，同 Python `Counter`）。
pub(crate) fn count(toks: Vec<String>) -> Vec<(String, u64)> {
    let mut idx: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<(String, u64)> = Vec::new();
    for t in toks {
        match idx.get(&t) {
            Some(&i) => out[i].1 += 1,
            None => {
                idx.insert(t.clone(), out.len());
                out.push((t, 1));
            }
        }
    }
    out
}

fn counts_to_json(c: &[(String, u64)]) -> Value {
    Value::Object(c.iter().map(|(k, v)| (k.clone(), json!(v))).collect())
}

/// 索引裡的一份文件（對應 `build_docs` 回傳的 dict）。
#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    pub repo: String,
    /// 絕對路徑字串（同時是快取鍵）。
    pub path: String,
    pub rel: String,
    pub tf: HashMap<String, f64>,
    pub len: f64,
    pub htf: HashMap<String, f64>,
    pub hlen: f64,
    pub mtime: f64,
}

/// 對應 `knowledge.build_docs`。有 `spine_dir` 就走 mtime 快取（與 Python 共用同一份檔）。
///
/// 與 Python 的差異（N-02，格式不變）：快取不展開成整棵 JSON 樹，只解析範圍內命中的檔；
/// 沒有新算的檔就不重寫快取（Python 每次查詢都整份重寫）；常駐行程記住上次讀到的快取。
pub fn build_docs(entries: &[RepoRef], spine_dir: Option<&Path>) -> Vec<Doc> {
    let Some(sd) = spine_dir else {
        return build_docs_with(entries, &mut Cache::new()).0;
    };
    let mut memo = MEMO.lock().unwrap_or_else(|e| e.into_inner());
    let mut cache = take_cache(&mut memo, sd);
    let (docs, dirty) = build_docs_with(entries, &mut cache);
    let stamp = if dirty {
        save_cache(sd, &cache)
    } else {
        file_stamp(&cache_path(sd))
    };
    *memo = Some(Memo {
        path: cache_path(sd),
        stamp,
        cache,
    });
    docs
}

/// 回傳 (docs, 快取有沒有新算的檔)。列檔（git ls-files）與逐檔解析／索引都分給多條執行緒，
/// 結果照原本順序（repo 順序 → 檔案順序）合併。
fn build_docs_with(entries: &[RepoRef], cache: &mut Cache) -> (Vec<Doc>, bool) {
    let roots: Vec<(&RepoRef, PathBuf)> = entries
        .iter()
        .map(|e| (e, expanduser(&e.path)))
        .filter(|(_, root)| root.is_dir())
        .collect();
    let listed: Vec<Vec<PathBuf>> = std::thread::scope(|sc| {
        let hs: Vec<_> = roots
            .iter()
            .map(|(_, root)| sc.spawn(move || pack::doc_files(root)))
            .collect();
        hs.into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });
    let jobs: Vec<(&RepoRef, &Path, PathBuf)> = roots
        .iter()
        .zip(listed)
        .flat_map(|((e, root), files)| files.into_iter().map(move |f| (*e, root.as_path(), f)))
        .collect();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8);
    let chunk = jobs.len().div_ceil(threads).max(64);
    let shared: &Cache = cache;
    let results: Vec<Vec<Indexed>> = std::thread::scope(|sc| {
        let hs: Vec<_> = jobs
            .chunks(chunk)
            .map(|part| {
                sc.spawn(move || {
                    part.iter()
                        .filter_map(|(e, root, f)| index_one(shared, e, root, f))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        hs.into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });
    let mut dirty = false;
    let mut docs = Vec::new();
    for it in results.into_iter().flatten() {
        if let Some((key, raw)) = it.fresh {
            cache.insert(key, raw);
            dirty = true;
        }
        docs.extend(it.doc);
    }
    (docs, dirty)
}

/// 單一檔的結果：要給查詢的 doc（秘密檔沒有）＋要寫回快取的新紀錄（命中快取就沒有）。
struct Indexed {
    doc: Option<Doc>,
    fresh: Option<(String, Box<RawValue>)>,
}

/// 單一檔：快取命中就解析快取，否則讀檔重算。讀不到的檔＝None（同 Python 略過）。
fn index_one(cache: &Cache, e: &RepoRef, root: &Path, f: &Path) -> Option<Indexed> {
    let key = f.to_string_lossy().into_owned();
    let mtime = mtime_secs(f).ok()?;
    let hit = cache
        .get(&key)
        .and_then(|raw| serde_json::from_str::<CachedRec>(raw.get()).ok())
        .filter(|r| r.mtime == Some(mtime) && r.v == Some(INDEX_VERSION));
    let (rec, fresh) = match hit {
        Some(r) => (r, None),
        None => {
            let full = read_text_lossy(f).ok()?;
            let (raw, rec) = index_file(f, take_chars(&full, MAX_CHARS), mtime);
            let fresh = serde_json::value::to_raw_value(&raw)
                .ok()
                .map(|raw| (key.clone(), raw));
            (rec, fresh)
        }
    };
    let doc = (!rec.secret).then(|| Doc {
        repo: e.id.clone(),
        path: key,
        rel: pack::rel_posix(f, root),
        tf: rec.tf,
        len: rec.len,
        htf: rec.htf,
        hlen: rec.hlen,
        mtime,
    });
    Some(Indexed { doc, fresh })
}

/// 算一個檔的索引：(寫進快取的 JSON——計數是整數，同 Python, 給本次查詢用的紀錄)。
fn index_file(f: &Path, text: &str, mtime: f64) -> (Value, CachedRec) {
    if !pack::secret_hits(text).is_empty() {
        let raw = json!({"mtime": mtime, "secret": true, "v": INDEX_VERSION});
        let rec = CachedRec {
            mtime: Some(mtime),
            secret: true,
            tf: HashMap::new(),
            len: 0.0,
            htf: HashMap::new(),
            hlen: 0.0,
            v: Some(INDEX_VERSION),
        };
        return (raw, rec);
    }
    let tf = count(tokenize(text));
    let name = f
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let htf = count(tokenize(&heading_text(text, &name)));
    let len: u64 = tf.iter().map(|x| x.1).sum();
    let hlen: u64 = htf.iter().map(|x| x.1).sum();
    let raw = json!({"mtime": mtime, "tf": counts_to_json(&tf), "len": len,
                     "htf": counts_to_json(&htf), "hlen": hlen, "v": INDEX_VERSION});
    let to_map = |c: &[(String, u64)]| c.iter().map(|(k, v)| (k.clone(), *v as f64)).collect();
    let rec = CachedRec {
        mtime: Some(mtime),
        secret: false,
        tf: to_map(&tf),
        len: len as f64,
        htf: to_map(&htf),
        hlen: hlen as f64,
        v: Some(INDEX_VERSION),
    };
    (raw, rec)
}

/// 命中片段（行號 1 起算）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Snippet {
    pub line: usize,
    pub text: String,
}

/// 對應 `knowledge._snippets`：命中最多查詢詞的幾行（附行號）——給 agent 引用的依據。
pub(crate) fn snippets(path: &str, qtoks: &[String], limit: usize) -> Vec<Snippet> {
    let Ok(text) = read_text_lossy(Path::new(path)) else {
        return Vec::new();
    };
    let lines = splitlines(&text);
    let qset: HashSet<&str> = qtoks.iter().map(String::as_str).collect();
    let mut scored: Vec<(usize, usize)> = Vec::new();
    for (i, ln) in lines.iter().take(5000).enumerate() {
        let toks: HashSet<String> = tokenize(ln).into_iter().collect();
        let s = toks.iter().filter(|t| qset.contains(t.as_str())).count();
        if s > 0 {
            scored.push((s, i));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.truncate(limit);
    scored.sort_by_key(|x| x.1);
    scored
        .into_iter()
        .map(|(_, i)| Snippet {
            line: i + 1,
            text: take_chars(lines[i].trim(), 160).to_string(),
        })
        .collect()
}

/// 對應 `knowledge._bm25`：每份 doc 的 BM25 分數（同一份 docs 順序）。`head` 選標題索引。
pub(crate) fn bm25(qcount: &[(String, u64)], docs: &[Doc], head: bool) -> Vec<f64> {
    fn tf_of(d: &Doc, head: bool) -> &HashMap<String, f64> {
        if head {
            &d.htf
        } else {
            &d.tf
        }
    }
    let tf = |d| tf_of(d, head);
    let dl = |d: &Doc| if head { d.hlen } else { d.len };
    let n = docs.len() as f64;
    let mut avg = if docs.is_empty() {
        1.0
    } else {
        docs.iter().map(dl).sum::<f64>() / n
    };
    if avg == 0.0 {
        avg = 1.0;
    }
    let df: Vec<f64> = qcount
        .iter()
        .map(|(t, _)| docs.iter().filter(|d| tf(d).contains_key(t)).count() as f64)
        .collect();
    docs.iter()
        .map(|d| {
            let mut s = 0.0;
            for ((t, qn), dft) in qcount.iter().zip(&df) {
                let f = match tf(d).get(t) {
                    Some(&f) if f != 0.0 => f,
                    _ => continue,
                };
                let idf = (1.0 + (n - dft + 0.5) / (dft + 0.5)).ln();
                s += *qn as f64 * idf * f * (K1 + 1.0) / (f + K1 * (1.0 - B + B * dl(d) / avg));
            }
            s
        })
        .collect()
}

/// 路徑前綴加權（對應 Python boosts["paths"] 的一列 `(repo, 前綴, 倍率, 理由)`）。
#[derive(Debug, Clone, PartialEq)]
pub struct PathBoost {
    pub repo: String,
    pub prefix: String,
    pub factor: f64,
    pub reason: String,
}

/// 對應 Python `boosts={"repos": {id: (倍率, 理由)}, "paths": [...]}`——由呼叫端（agentapi）依主場算好。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Boosts {
    pub repos: HashMap<String, (f64, String)>,
    pub paths: Vec<PathBoost>,
}

/// 對應 `knowledge._boost`：回傳 (倍率, why 清單)。
fn boost(d: &Doc, boosts: &Boosts, now: f64) -> (f64, Vec<String>) {
    let (mut factor, mut why) = (1.0, Vec::new());
    if let Some((f, reason)) = boosts.repos.get(&d.repo) {
        factor *= f;
        why.push(reason.clone());
    }
    for pb in &boosts.paths {
        if pb.repo == d.repo
            && (pb.prefix.is_empty() || pb.prefix == "." || d.rel.starts_with(&pb.prefix))
        {
            factor *= pb.factor;
            why.push(pb.reason.clone());
            break;
        }
    }
    if now - d.mtime <= RECENT_DAYS * 86400.0 {
        factor *= RECENT_BOOST;
        why.push(format!("{} 天內改過", RECENT_DAYS as i64));
    }
    (factor, why)
}

/// 搜尋結果的一份檔。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileHit {
    pub repo: String,
    pub file: String,
    pub path: String,
    pub score: f64,
    pub why: Vec<String>,
    pub snippets: Vec<Snippet>,
}

/// 搜尋結果的 repo 彙總。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepoHit {
    pub repo: String,
    pub score: f64,
    pub matching_files: usize,
}

/// 對應 `knowledge.search` 回傳的 dict（鍵名同 Python；`note` 與 `indexed_files` 擇一出現）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchResult {
    pub query: String,
    pub files: Vec<FileHit>,
    pub repos: Vec<RepoHit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_files: Option<usize>,
}

/// Python `round(x, 2)`：以精確十進位展開四捨五入到兩位（Rust 格式化同為正確捨入）。
pub(crate) fn round2(x: f64) -> f64 {
    format!("{x:.2}").parse().unwrap_or(x)
}

/// 對應 `knowledge.search`：BM25（內文＋標題加權）× boosts → files／repos。
///
/// files：依分數排序的命中檔；repos：前 50 筆命中依 repo 加總排序，回答「這段文字跟哪些 repo 最相關」。
/// `exclude_paths`＝不把查詢來源檔自己算進去。Python 預設 `limit=8`。
pub fn search(
    entries: &[RepoRef],
    query: &str,
    limit: usize,
    spine_dir: Option<&Path>,
    exclude_paths: &[PathBuf],
    boosts: &Boosts,
) -> SearchResult {
    let empty = |note: &str| SearchResult {
        query: query.to_string(),
        files: Vec::new(),
        repos: Vec::new(),
        note: Some(note.to_string()),
        indexed_files: None,
    };
    let qtoks = tokenize(query);
    if qtoks.is_empty() {
        return empty("查詢字串斷不出可比對的詞");
    }
    let excluded: HashSet<PathBuf> = exclude_paths
        .iter()
        .map(|p| resolve(&expanduser(p)))
        .collect();
    let docs: Vec<Doc> = build_docs(entries, spine_dir)
        .into_iter()
        .filter(|d| !excluded.contains(&resolve(Path::new(&d.path))))
        .collect();
    if docs.is_empty() {
        return empty("範圍內沒有可索引的 md 檔");
    }
    let qcount = count(qtoks.clone());
    let body = bm25(&qcount, &docs, false);
    let head = bm25(&qcount, &docs, true);
    let now = now_secs();
    let mut scored: Vec<(f64, &Doc, Vec<String>)> = Vec::new();
    for ((d, sb), sh) in docs.iter().zip(body).zip(head) {
        let s = sb + HEAD_WEIGHT * sh;
        if s <= 0.0 {
            continue;
        }
        let (factor, mut why) = boost(d, boosts, now);
        if sh > 0.0 {
            why.insert(0, "標題命中".into());
        }
        scored.push((s * factor, d, why));
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0)); // 穩定排序，同分保留原順序
    let files = scored
        .iter()
        .take(limit)
        .map(|(s, d, why)| FileHit {
            repo: d.repo.clone(),
            file: d.rel.clone(),
            path: d.path.clone(),
            score: round2(*s),
            why: why.clone(),
            snippets: snippets(&d.path, &qtoks, 2),
        })
        .collect();
    // Counter.most_common：依分數降冪，同分照首次出現順序
    let mut per_repo: Vec<(String, f64, usize)> = Vec::new();
    for (s, d, _) in scored.iter().take(50) {
        match per_repo.iter_mut().find(|r| r.0 == d.repo) {
            Some(r) => {
                r.1 += s;
                r.2 += 1;
            }
            None => per_repo.push((d.repo.clone(), *s, 1)),
        }
    }
    per_repo.sort_by(|a, b| b.1.total_cmp(&a.1));
    let repos = per_repo
        .into_iter()
        .map(|(repo, v, n)| RepoHit {
            repo,
            score: round2(v),
            matching_files: n,
        })
        .collect();
    SearchResult {
        query: query.to_string(),
        files,
        repos,
        note: None,
        indexed_files: Some(docs.len()),
    }
}
