//! 知識相關度（2026-09-25 Agent 友善輪）——「這段文字跟哪些 repo 的哪些檔強相關」。對應 `repo_context/knowledge.py`。
//!
//! 痛點：card_notes 的文章與書，要能很快比對出跟哪些 repo 強相關。先用最簡單、可解釋、零依賴的做法：
//! BM25 over 各 repo 的 *.md（選料沿用 `pack::doc_files`，尊重 .gitignore）。中文用字元 bigram、
//! 英數用小寫單字，不需要斷詞器也不需要 embedding。
//!
//! 紀律：
//! - 洩密哨兵照走——長得像憑證的檔不進索引，也不會出現在片段裡（公私分界）。
//! - 回傳帶分數與命中片段的「依據」，agent 要引用時說得出來自哪個 repo 哪份檔哪一行。
//! - 索引存在 `.state/knowledge.db`（SQLite，Rust 才有，N-51，見 [`store`]）：依檔案 mtime 增量更新，
//!   查詢只撈查詢詞的 posting。SQLite 只當倒排索引的儲存——斷詞、BM25、加權、片段全是這裡的算法，
//!   不用 FTS5 的排序，結果跟從頭在記憶體裡建索引（[`build_docs`] 不給 spine、[`search`] 不給 spine）逐位元組相同。
//!   Python 版還用它自己的 `.state/knowledge_index.json`；Rust 不讀不寫那份，第一次建好 db 後把它刪掉。
//!
//! 找得「準」（跨 repo 參考輪）：
//! - 標題／frontmatter／檔名另建一份索引（htf），命中加權：md 有結構，標題命中比內文命中準得多。
//! - boosts：呼叫端（agentapi）依主場算好——有關係的 repo 加分、落在 export 內加分；
//!   近 30 天改過的檔小幅加分。每筆結果帶 why（為什麼排這裡），agent 才判斷得了要不要信。

mod store;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;

use crate::fsutil::{expanduser, mtime_secs, now_secs, read_text_lossy, resolve};
use crate::pack::{self, RepoRef};
use crate::pystr::{splitlines, take_chars};

pub use store::{index_fallbacks, index_generation, DB_SCHEMA_VERSION};

static WORD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[a-z0-9][a-z0-9_\-\.]*[a-z0-9]|[a-z0-9]").expect("寫死的 regex"));
static CJK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\x{3400}-\x{9FFF}\x{F900}-\x{FAFF}]+").expect("寫死的 regex"));
const K1: f64 = 1.5;
const B: f64 = 0.75;
/// 單檔上限：超大檔只索引前段（避免一本書的全文拖垮查詢）。
const MAX_CHARS: usize = 200_000;
/// 標題／frontmatter／檔名命中的權重（相對內文）。
const HEAD_WEIGHT: f64 = 2.0;
const RECENT_DAYS: f64 = 30.0;
const RECENT_BOOST: f64 = 1.1;
/// 在封存路徑（[`in_archive`]）的檔分數乘這個（Rust 才有，2026-11-14 F-126）。
pub const ARCHIVE_FACTOR: f64 = 0.3;
/// 「標題命中」：查詢詞（去重後）至少這個比例出現在標題索引（標題、檔名）才標（Rust 才有，F-126；分數算法不變）。
pub const TITLE_HIT_RATIO: f64 = 0.5;
/// 封存資料夾的名字（整段，或底線／連字號切開後的一節，例 `01_inbox_retired_20261104`）。
const ARCHIVE_WORDS: [&str; 4] = ["archive", "archived", "retired", "deprecated"];

/// 這份檔是不是在封存路徑：repo 內相對路徑的**資料夾**（不含檔名）任一段是封存的名字，
/// 或底線／連字號切開後有一節是（`_archive`、`archive-2024`、`01_inbox_retired_20261104`）。
/// 只看資料夾：`plans/workspace-archive-sort.md` 這種檔名不算。
pub fn in_archive(rel: &str) -> bool {
    let segs: Vec<&str> = rel.split(['/', '\\']).collect();
    segs[..segs.len().saturating_sub(1)].iter().any(|seg| {
        let low = seg.to_lowercase();
        low.split(['_', '-'])
            .any(|part| ARCHIVE_WORDS.contains(&part))
    })
}

/// 查詢詞（去重）有幾成出現在這份檔的標題索引。`has` 回答第 i 個查詢詞在不在標題索引。
fn title_coverage(qcount: &[(String, u64)], has: impl Fn(usize) -> bool) -> f64 {
    if qcount.is_empty() {
        return 0.0;
    }
    let hit = (0..qcount.len()).filter(|&i| has(i)).count();
    hit as f64 / qcount.len() as f64
}
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

/// 索引裡的一份文件（對應 `build_docs` 回傳的 dict）。
#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    pub repo: String,
    /// 絕對路徑字串。
    pub path: String,
    pub rel: String,
    pub tf: HashMap<String, f64>,
    pub len: f64,
    pub htf: HashMap<String, f64>,
    pub hlen: f64,
    pub mtime: f64,
}

/// 對應 `knowledge.build_docs`：範圍內每份可索引的 md（秘密檔不算），順序＝repo 順序 → 檔案順序。
///
/// 有 `spine_dir` 就走 `.state/knowledge.db`（增量更新後讀出整份；索引庫開不起來就退回在記憶體裡從頭建）；
/// 沒有就在記憶體裡從頭建——這條是對照組（測試拿來證明索引庫的結果相同），正式路徑都給 spine。
pub fn build_docs(entries: &[RepoRef], spine_dir: Option<&Path>) -> Vec<Doc> {
    if let Some(sd) = spine_dir {
        if let Some(docs) = store::full_docs(sd, entries) {
            return docs;
        }
    }
    build_docs_mem(entries)
}

/// 範圍內列出來的檔：存在的 repo 根（照 entries 順序），以及每份檔（屬於哪個根、絕對路徑），照 repo 順序 → 檔案順序。
struct Listing<'a> {
    roots: Vec<(&'a RepoRef, PathBuf)>,
    files: Vec<(usize, PathBuf)>,
}

impl Listing<'_> {
    fn repo(&self, i: usize) -> &str {
        &self.roots[self.files[i].0].0.id
    }
    fn file(&self, i: usize) -> &Path {
        &self.files[i].1
    }
    /// 絕對路徑字串（`Doc::path`，索引庫的鍵之一）。
    fn key(&self, i: usize) -> String {
        self.files[i].1.to_string_lossy().into_owned()
    }
    fn rel(&self, i: usize) -> String {
        pack::rel_posix(&self.files[i].1, &self.roots[self.files[i].0].1)
    }
    fn meta(&self, i: usize, mtime: f64) -> Meta {
        Meta {
            repo: self.repo(i).to_string(),
            path: self.key(i),
            rel: self.rel(i),
            mtime,
        }
    }
}

/// 列檔（`pack::doc_files`＝`git ls-files`，各 repo 同時跑）。不存在的 repo 目錄略過。
fn list_files(entries: &[RepoRef]) -> Listing<'_> {
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
    let files = listed
        .into_iter()
        .enumerate()
        .flat_map(|(i, fs)| fs.into_iter().map(move |f| (i, f)))
        .collect();
    Listing { roots, files }
}

/// 把 `0..n` 分給多條執行緒算，結果照原本順序。
fn par_map<R: Send>(n: usize, f: impl Fn(usize) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8);
    let chunk = n.div_ceil(threads).max(64);
    let f = &f;
    std::thread::scope(|sc| {
        let hs: Vec<_> = (0..n)
            .step_by(chunk)
            .map(|start| {
                sc.spawn(move || (start..(start + chunk).min(n)).map(f).collect::<Vec<R>>())
            })
            .collect();
        hs.into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    })
}

/// 一份檔算出來的索引（計數照首次出現順序；秘密檔只有 `secret`）。
#[derive(Debug, Clone, Default)]
struct Rec {
    secret: bool,
    tf: Vec<(String, u64)>,
    len: u64,
    htf: Vec<(String, u64)>,
    hlen: u64,
}

impl Rec {
    fn into_doc(self, meta: Meta) -> Doc {
        let to_map = |c: Vec<(String, u64)>| c.into_iter().map(|(k, v)| (k, v as f64)).collect();
        Doc {
            repo: meta.repo,
            path: meta.path,
            rel: meta.rel,
            tf: to_map(self.tf),
            len: self.len as f64,
            htf: to_map(self.htf),
            hlen: self.hlen as f64,
            mtime: meta.mtime,
        }
    }
}

/// 讀檔並算索引。讀不到＝None（同 Python 略過）。
fn read_rec(f: &Path) -> Option<Rec> {
    let full = read_text_lossy(f).ok()?;
    Some(index_text(f, take_chars(&full, MAX_CHARS)))
}

/// 算一個檔的索引：像憑證的檔只記 `secret`（不進索引，也不會出現在片段裡）。
fn index_text(f: &Path, text: &str) -> Rec {
    if !pack::secret_hits(text).is_empty() {
        return Rec {
            secret: true,
            ..Rec::default()
        };
    }
    let tf = count(tokenize(text));
    let name = f
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let htf = count(tokenize(&heading_text(text, &name)));
    let len: u64 = tf.iter().map(|x| x.1).sum();
    let hlen: u64 = htf.iter().map(|x| x.1).sum();
    Rec {
        secret: false,
        tf,
        len,
        htf,
        hlen,
    }
}

/// 在記憶體裡從頭建（對照組，也是索引庫打不開時的退路）：列檔 → 逐檔讀、斷詞、計數。
fn build_docs_mem(entries: &[RepoRef]) -> Vec<Doc> {
    let listing = list_files(entries);
    par_map(listing.files.len(), |i| {
        let mtime = mtime_secs(listing.file(i)).ok()?;
        let rec = read_rec(listing.file(i))?;
        (!rec.secret).then(|| rec.into_doc(listing.meta(i, mtime)))
    })
    .into_iter()
    .flatten()
    .collect()
}

/// 一份檔給排序用的身分（分數之外的部分）。
#[derive(Debug, Clone)]
struct Meta {
    repo: String,
    path: String,
    rel: String,
    mtime: f64,
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

/// 跟 [`bm25`] 同一條算式、同樣的累加順序（索引庫那條路用；結果要逐位元組相同）。
///
/// `dl`＝每份檔的長度（照 doc 順序）；`tfs[t]`＝第 t 個查詢詞出現在哪幾份檔（doc 序號 → tf）。
/// 一份檔在 `tfs[t]` 裡最多一筆，所以 df＝`tfs[t].len()`（同 [`bm25`] 數「有這個詞的 doc 有幾份」）。
fn bm25_sparse(qcount: &[(String, u64)], dl: &[f64], tfs: &[HashMap<usize, f64>]) -> Vec<f64> {
    let n = dl.len() as f64;
    let mut avg = if dl.is_empty() {
        1.0
    } else {
        dl.iter().copied().sum::<f64>() / n
    };
    if avg == 0.0 {
        avg = 1.0;
    }
    let df: Vec<f64> = tfs.iter().map(|m| m.len() as f64).collect();
    (0..dl.len())
        .map(|d| {
            let mut s = 0.0;
            for (((_, qn), dft), m) in qcount.iter().zip(&df).zip(tfs) {
                let f = match m.get(&d) {
                    Some(&f) if f != 0.0 => f,
                    _ => continue,
                };
                let idf = (1.0 + (n - dft + 0.5) / (dft + 0.5)).ln();
                s += *qn as f64 * idf * f * (K1 + 1.0) / (f + K1 * (1.0 - B + B * dl[d] / avg));
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
fn boost(d: &Meta, boosts: &Boosts, now: f64) -> (f64, Vec<String>) {
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
    if in_archive(&d.rel) {
        factor *= ARCHIVE_FACTOR;
        why.push("在封存路徑".into());
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
    /// 這個 repo 分數最高的那份檔（不序列化：JSON 跟 Python 一樣；精簡模式用，F-126）
    #[serde(skip)]
    pub best_score: f64,
    #[serde(skip)]
    pub best_file: String,
    #[serde(skip)]
    pub best_path: String,
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
///
/// 有 `spine_dir` 走索引庫（`.state/knowledge.db`：增量更新、只撈查詢詞的 posting）；索引庫開不起來就退回
/// 在記憶體裡從頭建——搜尋不會因為索引庫壞了而失敗。沒有 `spine_dir`＝在記憶體裡從頭建（對照組）。
pub fn search(
    entries: &[RepoRef],
    query: &str,
    limit: usize,
    spine_dir: Option<&Path>,
    exclude_paths: &[PathBuf],
    boosts: &Boosts,
) -> SearchResult {
    let qtoks = tokenize(query);
    if qtoks.is_empty() {
        return empty_result(query, "查詢字串斷不出可比對的詞");
    }
    let excluded: HashSet<PathBuf> = exclude_paths
        .iter()
        .map(|p| resolve(&expanduser(p)))
        .collect();
    // 沒有要排除的就不必每份檔都 canonicalize（真資料 8 千份檔約 60ms；結果一樣）
    let keep = |path: &str| excluded.is_empty() || !excluded.contains(&resolve(Path::new(path)));
    let qcount = count(qtoks.clone());
    let scores = spine_dir
        .and_then(|sd| store::score_input(sd, entries, &qcount, &keep))
        .map(|input| scores_of(&qcount, input))
        .unwrap_or_else(|| scores_mem(entries, &qcount, &keep));
    if scores.indexed == 0 {
        return empty_result(query, "範圍內沒有可索引的 md 檔");
    }
    rank(query, &qtoks, scores, limit, boosts)
}

fn empty_result(query: &str, note: &str) -> SearchResult {
    SearchResult {
        query: query.to_string(),
        files: Vec::new(),
        repos: Vec::new(),
        note: Some(note.to_string()),
        indexed_files: None,
    }
}

/// 排序前的一份命中檔：身分、內文分、標題分、查詢詞在標題索引的涵蓋率。
struct Scored {
    meta: Meta,
    sb: f64,
    sh: f64,
    cov: f64,
}

/// 打完分：分數 > 0 的檔（照 doc 順序）＋範圍內可索引的檔數（`indexed_files`）。
struct Scores {
    hits: Vec<Scored>,
    indexed: usize,
}

/// 索引庫撈回來的打分原料（[`store::score_input`]）：範圍內每份檔（照 doc 順序）的身分與長度，
/// 以及每個查詢詞（照 `qcount` 順序）出現在哪幾份檔、tf 多少——內文、標題各一份。
struct ScoreInput {
    metas: Vec<Meta>,
    len: Vec<f64>,
    hlen: Vec<f64>,
    body: Vec<HashMap<usize, f64>>,
    head: Vec<HashMap<usize, f64>>,
}

fn scores_of(qcount: &[(String, u64)], input: ScoreInput) -> Scores {
    let body = bm25_sparse(qcount, &input.len, &input.body);
    let head = bm25_sparse(qcount, &input.hlen, &input.head);
    let indexed = input.metas.len();
    let hits = input
        .metas
        .into_iter()
        .zip(body.into_iter().zip(head))
        .enumerate()
        .filter(|(_, (_, (sb, sh)))| sb + HEAD_WEIGHT * sh > 0.0)
        .map(|(d, (meta, (sb, sh)))| Scored {
            meta,
            sb,
            sh,
            cov: title_coverage(qcount, |t| input.head[t].contains_key(&d)),
        })
        .collect();
    Scores { hits, indexed }
}

/// 對照組：在記憶體裡從頭建索引再打分（[`bm25`]，搬到索引庫之前的算法原樣）。
fn scores_mem(
    entries: &[RepoRef],
    qcount: &[(String, u64)],
    keep: &dyn Fn(&str) -> bool,
) -> Scores {
    let docs: Vec<Doc> = build_docs_mem(entries)
        .into_iter()
        .filter(|d| keep(&d.path))
        .collect();
    let body = bm25(qcount, &docs, false);
    let head = bm25(qcount, &docs, true);
    let hits = docs
        .iter()
        .zip(body.into_iter().zip(head))
        .filter(|(_, (sb, sh))| sb + HEAD_WEIGHT * sh > 0.0)
        .map(|(d, (sb, sh))| Scored {
            meta: Meta {
                repo: d.repo.clone(),
                path: d.path.clone(),
                rel: d.rel.clone(),
                mtime: d.mtime,
            },
            sb,
            sh,
            cov: title_coverage(qcount, |t| d.htf.contains_key(&qcount[t].0)),
        })
        .collect();
    Scores {
        hits,
        indexed: docs.len(),
    }
}

/// 分數 × boosts → 排序 → files（附片段）／repos。
fn rank(
    query: &str,
    qtoks: &[String],
    scores: Scores,
    limit: usize,
    boosts: &Boosts,
) -> SearchResult {
    let now = now_secs();
    let mut scored: Vec<(f64, &Meta, Vec<String>)> = Vec::new();
    for c in &scores.hits {
        let s = c.sb + HEAD_WEIGHT * c.sh;
        if s <= 0.0 {
            continue;
        }
        let (factor, mut why) = boost(&c.meta, boosts, now);
        // Rust 才有（F-126）：以前只要有一個詞碰到標題就標，幾乎每筆都有；改成查詢詞過半出現在標題才標
        if c.sh > 0.0 && c.cov >= TITLE_HIT_RATIO {
            why.insert(0, "標題命中".into());
        }
        scored.push((s * factor, &c.meta, why));
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
            snippets: snippets(&d.path, qtoks, 2),
        })
        .collect();
    // Counter.most_common：依分數降冪，同分照首次出現順序
    // 第一次碰到的就是那個 repo 分數最高的檔（scored 已排序）
    let mut per_repo: Vec<(String, f64, usize, f64, &Meta)> = Vec::new();
    for (s, d, _) in scored.iter().take(50) {
        match per_repo.iter_mut().find(|r| r.0 == d.repo) {
            Some(r) => {
                r.1 += s;
                r.2 += 1;
            }
            None => per_repo.push((d.repo.clone(), *s, 1, *s, d)),
        }
    }
    per_repo.sort_by(|a, b| b.1.total_cmp(&a.1));
    let repos = per_repo
        .into_iter()
        .map(|(repo, v, n, best, d)| RepoHit {
            repo,
            score: round2(v),
            matching_files: n,
            best_score: best,
            best_file: d.rel.clone(),
            best_path: d.path.clone(),
        })
        .collect();
    SearchResult {
        query: query.to_string(),
        files,
        repos,
        note: None,
        indexed_files: Some(scores.indexed),
    }
}

#[cfg(test)]
mod archive_tests {
    use super::*;

    #[test]
    fn archive_paths() {
        assert!(in_archive("archive/x.md"));
        assert!(in_archive("docs/_archive/x.md"));
        assert!(in_archive("notes/01_inbox_retired_20261104/a.md"));
        assert!(in_archive("Deprecated-2024/a.md"));
        assert!(!in_archive("plans/workspace-archive-sort.md"), "檔名不算");
        assert!(!in_archive("archives/x.md"));
        assert!(!in_archive("README.md"));
    }
}
