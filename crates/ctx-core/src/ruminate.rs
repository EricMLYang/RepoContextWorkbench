//! 反芻輪（2026-09-25）——新進的知識與別組的決策，主動對上每組手上的工作。對應 `repo_context/ruminate.py`。
//!
//! search_knowledge 只有「拉」：agent 要先想到去搜。但使用者持續把文章、書摘、想法 repo 化，也在各組
//! 留下判斷——這些新東西跟另一組正在做的事強相關時，沒有人會想到去搜。這裡補上「推」：
//!
//! - 每組的「手上工作」＝目標＋未結事項＋上次交接＋最近幾筆判斷（全是使用者／agent 自己寫的字）。
//! - 候選＝上次反芻之後**別組 repo 新增或改過的 md**，加上**別組新留下的判斷／交接**。
//! - BM25 比對（沿用 knowledge），只收「共同的少見詞」夠多的——寧缺勿濫，每組最多 3 筆。
//! - 結果住 `.state/ruminate.json`（候選層，不進收件匣——跟引用紀錄同一個理由：不是要人處理的事），
//!   由 SessionStart 開場注入、`ctx fresh` 看、`ctx fresh --dismiss` 打掉。
//! - **越用越準的量尺**：agent 之後 read_from 讀了某筆推薦（cites.jsonl 有記錄）＝用上了；
//!   `ctx fresh --stats` 算命中率。沒人用的推薦＝閾值該調，不是 agent 懶。
//!
//! 刻意不用 LLM——零成本、可解釋（每筆都列出共同詞），排程每天跑一次也不心疼。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{Duration, Local, NaiveDateTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::crossref::{self, ser_pairs};
use crate::knowledge::{self, Doc, Snippet};
use crate::locking::FileLock;
use crate::pack::RepoRef;
use crate::spine::{self, Event, EventType, LOCK_NAME};
use crate::{config, registry, Error, Result};

/// 對應 `MAX_PER_GROUP`。
pub const MAX_PER_GROUP: usize = 3;
/// 推薦保留天數（之後自然淡出，不堆積）。
const KEEP_DAYS: i64 = 30;
/// 手上工作取最近幾筆判斷。
const FOCUS_DECISIONS: usize = 5;
/// 開場發現超過這麼久沒反芻 → 背景補跑一次。
const STALE_HOURS: i64 = 20;

fn state_path(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("ruminate.json")
}

/// 推薦來源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    /// 別組 repo 新進的 md
    File,
    /// 別組新留下的判斷／交接
    Decision,
}

/// 推薦狀態。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkStatus {
    New,
    Used,
    Dismissed,
}

/// 一筆推薦（`.state/ruminate.json` 的 `links[]`；鍵名、順序同 Python）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub id: Option<String>,
    pub group: String,
    pub kind: LinkKind,
    pub repo: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub title: String,
    /// 檔案＝int(mtime)；判斷＝"YYYY-MM-DD HH:MM"（同一份料改了才重推）。
    pub stamp: Value,
    pub score: f64,
    pub terms: Vec<String>,
    pub basis: Vec<String>,
    pub found: String,
    pub status: LinkStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippets: Option<Vec<Snippet>>,
    /// 只在讀出時算（`_mark_used`），不寫回檔。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_at: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `.state/ruminate.json` 整份。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct State {
    #[serde(default)]
    pub last_run: Option<String>,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// 對應 `ruminate.load_state`：讀不到或壞掉＝`{last_run: None, links: []}`。
pub fn load_state(spine_dir: &Path) -> State {
    std::fs::read_to_string(state_path(spine_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_state(spine_dir: &Path, state: &State) -> Result<()> {
    let p = state_path(spine_dir);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = p.with_extension("tmp");
    // 同 Python json.dumps(ensure_ascii=False, indent=1)
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    state.serialize(&mut ser)?;
    std::fs::write(&tmp, buf)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

fn parse_iso(s: &str) -> Option<NaiveDateTime> {
    [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
    ]
    .iter()
    .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
    .or_else(|| {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
    })
}

fn local_ts(dt: NaiveDateTime) -> f64 {
    match Local.from_local_datetime(&dt).earliest() {
        Some(t) => t.timestamp() as f64 + f64::from(t.timestamp_subsec_nanos()) * 1e-9,
        None => dt.and_utc().timestamp() as f64,
    }
}

// ---------------------------------------------------------------- 手上工作

fn is_work(ev: &Event) -> bool {
    matches!(ev.etype, EventType::Decision | EventType::Outcome)
        && !ev.body.starts_with(spine::ADMIN_TAG)
}

/// 對應 `ruminate.focus`：一組手上的工作 → (文字, 依據清單)。全是寫下來的字；沒寫就沒有，不從 commit 猜。
/// `events`／`loops` 由呼叫端給（Python 預設各自從 spine 讀）。
pub fn focus(
    spine_dir: &Path,
    group: &str,
    events: &[Event],
    loops: &[Event],
) -> Result<(String, Vec<String>)> {
    let g = registry::get_group(spine_dir, group)?;
    let ids = g.members();
    let mut parts: Vec<String> = Vec::new();
    let mut basis: Vec<String> = Vec::new();
    if let Some(goal) = g.get_str("goal").filter(|s| !s.trim().is_empty()) {
        parts.push(goal);
        basis.push("目標".into());
    }
    let mine: Vec<&Event> = loops
        .iter()
        .filter(|ev| spine::in_scope(ev, Some(group), &ids))
        .collect();
    if !mine.is_empty() {
        parts.extend(mine.iter().map(|ev| ev.first_line().to_string()));
        basis.push(format!("未結 {}", mine.len()));
    }
    // Rust 才有（F-07）：被取代的判斷不算手上工作
    let superseded = crate::agentapi::superseded_map(events);
    let work: Vec<&Event> = events
        .iter()
        .filter(|ev| is_work(ev) && spine::in_scope(ev, Some(group), &ids))
        .filter(|ev| !superseded.contains_key(&crate::agentapi::event_ref(ev)))
        .collect();
    if !work.is_empty() {
        let k = work.len().saturating_sub(FOCUS_DECISIONS);
        parts.extend(work[k..].iter().map(|ev| ev.body.clone()));
        basis.push(format!(
            "最近判斷／交接 {}",
            work.len().min(FOCUS_DECISIONS)
        ));
    }
    let text = parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Ok((text, basis))
}

// ---------------------------------------------------------------- 比對

fn idf_of<'a>(docs: impl Iterator<Item = &'a Doc>) -> (HashMap<String, f64>, usize) {
    let mut df: HashMap<&str, usize> = HashMap::new();
    let mut n = 0usize;
    for d in docs {
        n += 1;
        for t in d.tf.keys() {
            *df.entry(t.as_str()).or_default() += 1;
        }
    }
    let nn = n.max(1) as f64;
    let idf = df
        .into_iter()
        .map(|(t, c)| {
            let c = c as f64;
            (t.to_string(), (1.0 + (nn - c + 0.5) / (c + 0.5)).ln())
        })
        .collect();
    (idf, n.max(1))
}

/// 對應 `ruminate._rare_floor`：「少見詞」＝出現在 ≤10% 文件（至少容許 2 份）。用中位數會在真語料裡
/// 只剩只出現一次的詞：卡片庫大部分詞都很少見，中位數 idf 貼著最大值（2026-09-25 真資料 1,585 檔實測）。
pub fn rare_floor(n: usize) -> f64 {
    let n = n as f64;
    let k = (n * 0.1).floor().max(2.0);
    (1.0 + (n - k + 0.5) / (k + 0.5)).ln()
}

/// 虛字：中文 bigram 跨詞邊界時會切出「接是」「得上」「的起」這種假詞，
/// 真資料實測它們足以把不相關的檔湊過門檻 → 含虛字的 bigram 不算共同詞。
const FUNCTION_CHARS: &str =
    "的是了不會在要就也和與及把讓得都而並或很這那有我你他它們一上下起個為以於之所被從到對";

/// 對應 `ruminate._meaningful`：含虛字的中文 bigram、單一非 ASCII 字不算共同詞。
///
/// 與 Python 的差異（N-03）：純數字／日期（`54`、`2026-10-06`、`v0.2` 以外只有數字與分隔號的）也不算——
/// 兩份文件剛好寫了同一天，不代表講同一件事。
pub fn meaningful(t: &str) -> bool {
    let cs: Vec<char> = t.chars().collect();
    let has_fn =
        cs.len() == 2 && (FUNCTION_CHARS.contains(cs[0]) || FUNCTION_CHARS.contains(cs[1]));
    let lone_cjk = cs.len() == 1 && !cs[0].is_ascii();
    let numeric = cs
        .iter()
        .all(|c| c.is_ascii_digit() || matches!(c, '-' | '.' | '_' | '/' | ':'));
    !has_fn && !lone_cjk && !numeric
}

/// Rust 才有（N-03）：中文 bigram 是否「像一個詞」的門檻——見 [`CharStats::word_like`]。
pub const MIN_BIGRAM_ASSOC: f64 = 0.08;

fn is_cjk_bigram(t: &str) -> bool {
    let mut cs = t.chars();
    matches!((cs.next(), cs.next(), cs.next()), (Some(a), Some(b), None) if is_cjk(a) && is_cjk(b))
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x9FFF | 0xF900..=0xFAFF)
}

/// 語料裡每個中文 bigram／單字出現在幾份文件——判斷 bigram 是詞還是跨詞邊界切出來的假詞。
///
/// 也記每個詞散布在幾個 repo（[`CharStats::generic`]）。
#[derive(Debug, Default)]
pub struct CharStats {
    bigram_df: HashMap<String, usize>,
    char_df: HashMap<char, usize>,
    /// 詞 → 出現在幾個 repo
    spread: HashMap<String, usize>,
    repos: usize,
}

/// Rust 才有：repo 數至少這麼多，「幾乎每個 repo 都有」才有統計意義（[`CharStats::generic`]）。
pub const MIN_REPOS_FOR_SPREAD: usize = 6;

impl CharStats {
    /// 從文件的 tf（詞 → 次數）統計。
    pub fn from_docs<'a>(docs: impl Iterator<Item = &'a Doc>) -> Self {
        let mut st = CharStats::default();
        let mut seen: HashMap<String, HashSet<String>> = HashMap::new();
        let mut repos: HashSet<String> = HashSet::new();
        for d in docs {
            repos.insert(d.repo.clone());
            for t in d.tf.keys() {
                let rs = seen.entry(t.clone()).or_default();
                if !rs.contains(&d.repo) {
                    rs.insert(d.repo.clone());
                }
            }
            let mut chars: HashSet<char> = HashSet::new();
            for t in d.tf.keys().filter(|t| is_cjk_bigram(t)) {
                *st.bigram_df.entry(t.clone()).or_default() += 1;
                chars.extend(t.chars());
            }
            for c in chars {
                *st.char_df.entry(c).or_default() += 1;
            }
        }
        st.spread = seen.into_iter().map(|(t, rs)| (t, rs.len())).collect();
        st.repos = repos.len();
        st
    }

    /// 一般用語：出現在超過三分之二的 repo（repo 數 ≥ [`MIN_REPOS_FOR_SPREAD`] 才判斷）。
    /// 「尚未」「交付」「等待」這種詞在每個 repo 都有，idf 卻不低（大語料裡只佔 4–9% 的檔），
    /// 會把不相干的檔湊過門檻；主題詞（「試播」「反芻」「鄰居」）集中在少數 repo
    /// （2026-10-10 真語料 15 個 repo 實測：一般用語 14–15 個 repo、主題詞 2–6 個）。
    pub fn generic(&self, t: &str) -> bool {
        self.repos >= MIN_REPOS_FOR_SPREAD
            && self.spread.get(t).copied().unwrap_or(0) * 3 > self.repos * 2
    }

    /// 中文 bigram 的「黏度」＝含這個 bigram 的文件數 ÷ 含其中較少見那個字的文件數。
    /// 跨詞邊界的假詞（「者決」「用後」「際使」）兩個字都很常見、湊在一起卻很少，黏度 < 0.05；
    /// 真詞（「鄰居」「回填」「交接」）多在 0.1 以上（2026-10-09 真語料 5,204 份實測：
    /// 0.08 擋掉 53 個假詞裡的 50 個，留下 25 個真詞裡的 21 個）。非中文 bigram 一律算像詞。
    pub fn word_like(&self, t: &str) -> bool {
        if !is_cjk_bigram(t) {
            return true;
        }
        let mut cs = t.chars();
        let (Some(a), Some(b)) = (cs.next(), cs.next()) else {
            return true;
        };
        let rarer = self
            .char_df
            .get(&a)
            .copied()
            .unwrap_or(0)
            .min(self.char_df.get(&b).copied().unwrap_or(0))
            .max(1);
        let df = self.bigram_df.get(t).copied().unwrap_or(0);
        df as f64 / rarer as f64 >= MIN_BIGRAM_ASSOC
    }
}

/// 對應 `_shared_terms`：共同的少見詞（idf 高於 floor、不含虛字），依 idf 排序——推薦的「為什麼」。
///
/// 與 Python 的差異（N-03）：中文 bigram 還要「像一個詞」（[`CharStats::word_like`]），
/// 也不能是幾乎每個 repo 都有的一般用語（[`CharStats::generic`]）。
fn shared_terms(
    qcount: &[(String, u64)],
    doc: &Doc,
    idf: &HashMap<String, f64>,
    floor: f64,
    stats: &CharStats,
) -> Vec<String> {
    let idf_of = |t: &str| idf.get(t).copied().unwrap_or(0.0);
    let mut terms: Vec<&String> = qcount
        .iter()
        .map(|(t, _)| t)
        .filter(|t| {
            doc.tf.contains_key(*t)
                && idf_of(t) >= floor
                && meaningful(t)
                && stats.word_like(t)
                && !stats.generic(t)
        })
        .collect();
    terms.sort_by(|a, b| idf_of(b).total_cmp(&idf_of(a)));
    terms.into_iter().cloned().collect()
}

/// 比對候選（檔案或判斷）。
struct Cand {
    doc: Doc,
    kind: LinkKind,
    reference: String,
    title: String,
    stamp: Value,
}

/// 對應 `_event_docs`：上次反芻後新留下的判斷／交接 → [(候選, 事件)]；排除本組由呼叫端做。
fn event_docs<'a>(events: &'a [Event], since: &str) -> Vec<(Cand, &'a Event)> {
    let mut out = Vec::new();
    // Rust 才有（F-07）：被取代的判斷不推給別組
    let superseded = crate::agentapi::superseded_map(events);
    for ev in events {
        if !is_work(ev) || ev.stamp().as_str() < since {
            continue;
        }
        if superseded.contains_key(&crate::agentapi::event_ref(ev)) {
            continue;
        }
        let counted = knowledge::count(knowledge::tokenize(&ev.body));
        let len: u64 = counted.iter().map(|x| x.1).sum();
        let nonempty = |s: Option<&str>| s.filter(|x| !x.is_empty()).map(str::to_string);
        let repo = nonempty(ev.kv("repo"))
            .or_else(|| nonempty(ev.kv("group")))
            .unwrap_or_else(|| "全域".into());
        let at = ev.stamp();
        let id = nonempty(ev.kv("id")).unwrap_or_else(|| ev.time.clone());
        out.push((
            Cand {
                doc: Doc {
                    repo,
                    path: String::new(),
                    rel: String::new(),
                    tf: counted.into_iter().map(|(k, v)| (k, v as f64)).collect(),
                    len: len as f64,
                    htf: HashMap::new(),
                    hlen: 0.0,
                    mtime: 0.0,
                },
                kind: LinkKind::Decision,
                reference: format!("{}:{id}", ev.etype),
                title: crate::pystr::take_chars(ev.first_line(), 80).to_string(),
                stamp: Value::String(at),
            },
            ev,
        ));
    }
    out
}

/// [`run`] 的結果：`{since, candidates, groups: {組: [新推薦]}}`。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunResult {
    pub since: String,
    pub candidates: usize,
    #[serde(serialize_with = "ser_pairs")]
    pub groups: Vec<(String, Vec<Link>)>,
}

impl RunResult {
    /// 某組這輪的新推薦。
    pub fn group(&self, name: &str) -> Option<&Vec<Link>> {
        self.groups.iter().find(|(g, _)| g == name).map(|(_, v)| v)
    }
}

fn stamp_key(v: &Value) -> String {
    v.to_string()
}

/// 對應 `ruminate.run`：跑一輪反芻。`window_days` 給了就看最近幾天（否則接上次的水位線；
/// 第一次跑看 config `ruminate.window_days`）。
pub fn run(
    spine_dir: &Path,
    now: Option<NaiveDateTime>,
    window_days: Option<i64>,
) -> Result<RunResult> {
    let now = now.unwrap_or_else(|| Local::now().naive_local());
    let cfg = config::load(spine_dir)?.ruminate();
    let state = load_state(spine_dir);
    let since_dt = match (window_days, state.last_run.as_deref().and_then(parse_iso)) {
        (Some(d), _) => now - Duration::days(d),
        (None, Some(last)) => last,
        (None, None) => now - Duration::days(cfg.window_days),
    };
    let since_ts = local_ts(since_dt);
    let since = since_dt.format("%Y-%m-%d %H:%M").to_string();

    let reg = registry::load(spine_dir)?;
    // 點開頭目錄（.claude／.agents／.github…）是工具設定不是知識；同內容多份只留一份
    // ——兩者都先剔掉再算 idf，不然重複檔會把真正的關鍵詞稀釋成「常見詞」
    let refs: Vec<RepoRef> = reg.repos.iter().map(RepoRef::from).collect();
    let mut all_docs: Vec<Doc> = Vec::new();
    let mut seen_tf: HashSet<Vec<(String, u64)>> = HashSet::new();
    for d in knowledge::build_docs(&refs, Some(spine_dir)) {
        let parts: Vec<&str> = d.rel.split('/').collect();
        if parts[..parts.len() - 1].iter().any(|p| p.starts_with('.')) {
            continue;
        }
        let mut sig: Vec<(String, u64)> =
            d.tf.iter().map(|(k, v)| (k.clone(), v.to_bits())).collect();
        sig.sort();
        if seen_tf.insert(sig) {
            all_docs.push(d);
        }
    }
    let events = spine::iter_events(spine_dir)?;
    let ev_docs = event_docs(&events, &since);
    let (idf, n_docs) = idf_of(all_docs.iter().chain(ev_docs.iter().map(|(c, _)| &c.doc)));
    let floor = rare_floor(n_docs);
    let stats = CharStats::from_docs(all_docs.iter().chain(ev_docs.iter().map(|(c, _)| &c.doc)));
    let loops = spine::open_loops_of(&events);
    let known: HashSet<(String, String, String)> = state
        .links
        .iter()
        .map(|l| (l.group.clone(), l.reference.clone(), stamp_key(&l.stamp)))
        .collect();
    // 與 Python 的差異（N-03）：同一組同一份料，還有一筆沒處理的（new）或被打掉過（dismissed），
    // 檔案改了也不再推一筆新的——不然常改的檔（AGENTS.md）每輪都會換個 id 重推。
    let settled: HashSet<(String, String)> = state
        .links
        .iter()
        .filter(|l| l.status != LinkStatus::Used)
        .map(|l| (l.group.clone(), l.reference.clone()))
        .collect();

    let found_at = now.format("%Y-%m-%d %H:%M").to_string();
    let mut found: Vec<(String, Vec<Link>)> = Vec::new();
    let fresh_files: Vec<&Doc> = all_docs.iter().filter(|d| d.mtime >= since_ts).collect();
    for g in &reg.groups {
        let name = g.name();
        let ids = g.members();
        let (text, basis) = focus(spine_dir, &name, &events, &loops)?;
        let qcount = knowledge::count(knowledge::tokenize(&text));
        if qcount.is_empty() {
            continue;
        }
        let mut cands: Vec<Cand> = fresh_files
            .iter()
            .filter(|d| !ids.contains(&d.repo))
            .map(|d| Cand {
                doc: (*d).clone(),
                kind: LinkKind::File,
                reference: format!("{}:{}", d.repo, d.rel),
                title: d.rel.clone(),
                stamp: Value::from(d.mtime as i64),
            })
            .collect();
        cands.extend(
            ev_docs
                .iter()
                .filter(|(_, ev)| !spine::in_scope(ev, Some(&name), &ids))
                .map(|(c, _)| Cand {
                    doc: c.doc.clone(),
                    kind: c.kind,
                    reference: c.reference.clone(),
                    title: c.title.clone(),
                    stamp: c.stamp.clone(),
                }),
        );
        if cands.is_empty() {
            continue;
        }
        let docs: Vec<Doc> = cands.iter().map(|c| c.doc.clone()).collect();
        let body = knowledge::bm25(&qcount, &docs, false);
        let head = knowledge::bm25(&qcount, &docs, true);
        let mut scored: Vec<(f64, &Cand, Vec<String>)> = Vec::new();
        for ((c, sb), sh) in cands.iter().zip(body).zip(head) {
            let shared = shared_terms(&qcount, &c.doc, &idf, floor, &stats);
            if (shared.len() as i64) < cfg.min_shared {
                continue; // 寧缺勿濫：共同少見詞不夠多＝只是剛好用到常見字
            }
            scored.push((sb + 2.0 * sh, c, shared));
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut picks: Vec<Link> = Vec::new();
        for (s, c, shared) in scored {
            if picks.len() as i64 >= cfg.max_per_group {
                break;
            }
            if known.contains(&(name.clone(), c.reference.clone(), stamp_key(&c.stamp)))
                || settled.contains(&(name.clone(), c.reference.clone()))
            {
                continue;
            }
            picks.push(Link {
                id: None,
                group: name.clone(),
                kind: c.kind,
                repo: c.doc.repo.clone(),
                reference: c.reference.clone(),
                title: c.title.clone(),
                stamp: c.stamp.clone(),
                score: knowledge::round2(s),
                terms: shared.iter().take(6).cloned().collect(),
                basis: basis.clone(),
                found: found_at.clone(),
                status: LinkStatus::New,
                snippets: (c.kind == LinkKind::File)
                    .then(|| knowledge::snippets(&c.doc.path, &shared, 2)),
                used_at: None,
                extra: Map::new(),
            });
        }
        if !picks.is_empty() {
            found.push((name, picks));
        }
    }

    {
        let _g = FileLock::new(spine_dir.join(LOCK_NAME)).acquire()?;
        let mut state = load_state(spine_dir);
        let cutoff = (now - Duration::days(KEEP_DAYS))
            .format("%Y-%m-%d")
            .to_string();
        let mut n = state
            .links
            .iter()
            .filter_map(|l| l.id.as_deref())
            .filter_map(|id| id.get(1..).and_then(|x| x.parse::<u64>().ok()))
            .max()
            .unwrap_or(0);
        let mut links: Vec<Link> = state
            .links
            .drain(..)
            .filter(|l| crate::pystr::take_chars(&l.found, 10) >= cutoff.as_str())
            .collect();
        // 與 Python 的差異：持鎖後重看一次——兩個開場同時在背景補跑反芻時，雙方都用舊狀態算，
        // 不重看就會各寫一份一模一樣的推薦（2026-10-09 真資料 r29–r36／r37–r44）。
        let pending: HashSet<(String, String)> = links
            .iter()
            .filter(|l| l.status != LinkStatus::Used)
            .map(|l| (l.group.clone(), l.reference.clone()))
            .collect();
        for (_, picks) in &mut found {
            picks.retain(|l| !pending.contains(&(l.group.clone(), l.reference.clone())));
        }
        found.retain(|(_, picks)| !picks.is_empty());
        for (_, picks) in &mut found {
            for link in picks.iter_mut() {
                n += 1;
                link.id = Some(format!("r{n}"));
                links.push(link.clone());
            }
        }
        state.last_run = Some(now.format("%Y-%m-%dT%H:%M:%S").to_string());
        state.links = links;
        save_state(spine_dir, &state)?;
    }
    let _ = std::fs::remove_file(running_path(spine_dir));
    // Rust 才有（F-33）：每天的反芻順便更新相關 repo 建議（共用同一份知識索引快取）
    let _ = crate::relsuggest::compute(spine_dir);
    Ok(RunResult {
        since,
        candidates: fresh_files.len(),
        groups: found,
    })
}

// ---------------------------------------------------------------- 讀出來用

/// 對應 `_mark_used`：推薦出現之後，組內有人 read_from 了它（cites.jsonl）＝用上了。
fn mark_used(spine_dir: &Path, mut links: Vec<Link>) -> Result<Vec<Link>> {
    let cites = crossref::load_cites(spine_dir);
    let members: HashMap<String, Vec<String>> = registry::load(spine_dir)?
        .groups
        .iter()
        .map(|g| (g.name(), g.members()))
        .collect();
    for l in &mut links {
        if l.status != LinkStatus::New || l.kind != LinkKind::File {
            continue;
        }
        let mem = members.get(&l.group);
        for c in &cites {
            if format!("{}:{}", c.provider, c.rel) == l.reference
                && mem.is_some_and(|m| m.contains(&c.consumer))
                && c.at >= l.found
            {
                l.status = LinkStatus::Used;
                l.used_at = Some(c.at.clone());
                break;
            }
        }
    }
    Ok(links)
}

/// 對應 `ruminate.fresh`：某組目前的推薦（新的在前）。include_seen＝連用過／打掉的也列。
///
/// 與 Python 的差異：同一份料只列一次（留最新的）——舊版並行反芻曾寫出重複的推薦，
/// 常改的檔（AGENTS.md）也曾每輪換個 id 重推。
pub fn fresh(spine_dir: &Path, group: &str, include_seen: bool) -> Result<Vec<Link>> {
    let mut out: Vec<Link> = mark_used(spine_dir, load_state(spine_dir).links)?
        .into_iter()
        .filter(|l| l.group == group && (include_seen || l.status == LinkStatus::New))
        .collect();
    out.sort_by(|a, b| {
        b.found
            .cmp(&a.found)
            .then_with(|| b.score.total_cmp(&a.score))
    });
    let mut seen: HashSet<String> = HashSet::new();
    out.retain(|l| seen.insert(l.reference.clone()));
    Ok(out)
}

/// 對應 `ruminate.dismiss`：打掉一筆推薦。找不到＝`Error::Invalid`。
pub fn dismiss(spine_dir: &Path, link_id: &str) -> Result<Link> {
    let _g = FileLock::new(spine_dir.join(LOCK_NAME)).acquire()?;
    let mut state = load_state(spine_dir);
    let hit = state
        .links
        .iter_mut()
        .find(|l| l.id.as_deref() == Some(link_id))
        .map(|l| {
            l.status = LinkStatus::Dismissed;
            l.clone()
        });
    match hit {
        Some(l) => {
            save_state(spine_dir, &state)?;
            Ok(l)
        }
        None => Err(Error::invalid(format!(
            "找不到推薦 {link_id}（ctx fresh --all 看全部）"
        ))),
    }
}

/// [`stats`] 的結果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stats {
    pub total: usize,
    pub new: usize,
    pub used: usize,
    pub dismissed: usize,
    pub hit_rate: Option<f64>,
    pub last_run: Option<String>,
}

/// 對應 `ruminate.stats`：命中率——推薦出去的，後來有多少被讀了。這是閾值的校準資料。
pub fn stats(spine_dir: &Path) -> Result<Stats> {
    let state = load_state(spine_dir);
    let last_run = state.last_run.clone();
    let links = mark_used(spine_dir, state.links)?;
    let count = |s: LinkStatus| links.iter().filter(|l| l.status == s).count();
    let files: Vec<&Link> = links.iter().filter(|l| l.kind == LinkKind::File).collect();
    let used = files
        .iter()
        .filter(|l| l.status == LinkStatus::Used)
        .count();
    Ok(Stats {
        total: links.len(),
        new: count(LinkStatus::New),
        used: count(LinkStatus::Used),
        dismissed: count(LinkStatus::Dismissed),
        hit_rate: (!files.is_empty()).then(|| knowledge::round2(used as f64 / files.len() as f64)),
        last_run,
    })
}

/// 對應 `ruminate.is_stale`：沒跑過、或上次反芻超過 20 小時。
/// （last_run 解析不了時 Python 會拋錯；這裡當成過期。）
pub fn is_stale(spine_dir: &Path, now: Option<NaiveDateTime>) -> bool {
    let Some(last) = load_state(spine_dir)
        .last_run
        .as_deref()
        .and_then(parse_iso)
    else {
        return true;
    };
    let now = now.unwrap_or_else(|| Local::now().naive_local());
    now - last > Duration::hours(STALE_HOURS)
}

/// 背景反芻進行中的標記；超過這麼久還在＝上一個背景進程死掉了，可以重佔。
const RUNNING_STALE_SECS: u64 = 600;

fn running_path(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("ruminate.running")
}

/// 佔住「背景反芻進行中」：佔到＝true；別人正在跑（標記還新）＝false。[`run`] 結束時清掉。
pub fn claim_background(spine_dir: &Path) -> bool {
    let p = running_path(spine_dir);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let fresh = std::fs::metadata(&p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() < RUNNING_STALE_SECS);
    if fresh {
        return false;
    }
    let _ = std::fs::remove_file(&p);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&p)
        .is_ok()
}

/// 對應 `ruminate.spawn_detached`：開場發現太久沒反芻 → 背景跑，不拖慢開 agent（下一次開場就看得到）。
///
/// 與 Python 的差異：Python 跑 `python -m repo_context --spine <spine> fresh --run`；
/// Rust 版跑 [`crate::exe::ctx_exe`] 加同樣的參數。
/// 環境變數 `REPOCTX_NO_BACKGROUND` 有值就不跑（測試用）。
///
/// 與 Python 的差異：先佔 `.state/ruminate.running`（[`claim_background`]）——幾個會話同時開場時只起一個。
pub fn spawn_detached(spine_dir: &Path) -> Result<()> {
    if crate::exe::background_disabled() || !claim_background(spine_dir) {
        return Ok(());
    }
    let spine = spine_dir.as_os_str();
    crate::exe::spawn_detached(
        &crate::exe::ctx_exe()?,
        [
            "--spine".as_ref(),
            spine,
            "fresh".as_ref(),
            "--run".as_ref(),
        ],
    )
}

/// 對應 `ruminate.link_lines`：開場注入／CLI 共用的一行一筆。Python 預設 `limit=3`。
pub fn link_lines(links: &[Link], limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    for l in links.iter().take(limit) {
        let decision = l.kind == LinkKind::Decision;
        out.push(format!(
            "- [{}] {} {}{}——共同詞：{}",
            l.id.as_deref().unwrap_or("None"),
            if decision { "判斷" } else { "新料" },
            l.reference,
            if decision {
                format!("「{}」", l.title)
            } else {
                String::new()
            },
            l.terms
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join("、")
        ));
        if let Some(sn) = l.snippets.as_ref().and_then(|s| s.first()) {
            out.push(format!(
                "    L{}: {}",
                sn.line,
                crate::pystr::take_chars(&sn.text, 100)
            ));
        }
    }
    out
}
