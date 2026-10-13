//! 引用讀取（L2）＋引用紀錄（cites）。

use super::*;

// ---------------------------------------------------------------- 讀 ＋ 引用紀錄

fn cites_path(spine_dir: &Path) -> PathBuf {
    spine_dir.join("refs").join("cites.jsonl")
}

/// `refs/cites.jsonl` 的一筆引用。欄位順序同 Python 寫出的順序；不認得的鍵保留在 `extra`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Cite {
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub consumer: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub rel: String,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub via: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// 對應 `crossref.load_cites`：壞行略過。
pub fn load_cites(spine_dir: &Path) -> Vec<Cite> {
    let Ok(text) = std::fs::read_to_string(cites_path(spine_dir)) else {
        return Vec::new();
    };
    splitlines(&text)
        .into_iter()
        .filter_map(|ln| serde_json::from_str::<Cite>(ln).ok())
        .collect()
}

/// 對應 `crossref.record_cite`：記一筆引用。同一天同 (consumer, provider, rel, via) 只記一次
/// （讀十次不是十個訊號）。consumer 是 None（在 spine 或未登記目錄）或等於 provider（自己讀自己）時不記。
#[allow(clippy::too_many_arguments)]
pub fn record_cite(
    spine_dir: &Path,
    consumer: Option<&str>,
    provider: &str,
    rel: &str,
    commit: Option<&str>,
    via: &str,
    note: Option<&str>,
    when: Option<NaiveDateTime>,
) -> Result<Option<Cite>> {
    let Some(consumer) = consumer.filter(|c| !c.is_empty() && *c != provider) else {
        return Ok(None);
    };
    let now = when.unwrap_or_else(|| Local::now().naive_local());
    let rec = Cite {
        at: now.format("%Y-%m-%d %H:%M").to_string(),
        consumer: consumer.to_string(),
        provider: provider.to_string(),
        rel: rel.to_string(),
        commit: commit.map(str::to_string),
        via: via.to_string(),
        note: note.filter(|n| !n.is_empty()).map(str::to_string),
        extra: Map::new(),
    };
    let p = cites_path(spine_dir);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _g = FileLock::new(spine_dir.join(LOCK_NAME)).acquire()?;
    let day = &rec.at[..10];
    for c in load_cites(spine_dir) {
        if take_chars(&c.at, 10) == day
            && c.consumer == rec.consumer
            && c.provider == rec.provider
            && c.rel == rec.rel
            && c.via == rec.via
        {
            return Ok(None);
        }
    }
    let line = py_dumps(&serde_json::to_value(&rec)?);
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)?;
    f.write_all(format!("{line}\n").as_bytes())?;
    Ok(Some(rec))
}

/// `git rev-parse --short HEAD`（拿不到＝None）。
pub(crate) fn head(root: &Path) -> Option<String> {
    let (rc, out) = git(root, &["rev-parse", "--short", "HEAD"], GIT_TIMEOUT);
    (rc == 0 && !out.is_empty()).then_some(out)
}

static LINES_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(\d+)\s*(?:-\s*(\d+))?\s*$").expect("寫死的 regex"));

fn parse_lines(lines: Option<&str>) -> Result<Option<(usize, usize)>> {
    let Some(s) = lines.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let bad = || {
        ref_err(
            crate::RejectCode::BadRequest,
            format!("lines 格式不對：{}", repr(s)),
            Some("用 120 或 120-180。".into()),
        )
    };
    let m = LINES_RE.captures(s).ok_or_else(bad)?;
    let a: usize = m[1].parse().map_err(|_| bad())?;
    let b: usize = match m.get(2) {
        Some(x) => x.as_str().parse().map_err(|_| bad())?,
        None => a,
    };
    Ok(Some((a.min(b), a.max(b))))
}

/// 目錄清單的一項：子目錄（帶檔數）或檔案。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirEntry {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<usize>,
}

/// [`read`] 的結果（鍵名、順序同 Python；目錄與檔案各有自己的欄位）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReadOut {
    pub ok: bool,
    pub repo: String,
    pub export: Option<String>,
    pub rel: String,
    pub path: String,
    pub commit: Option<String>,
    pub kind: PathKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<DirEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_entries: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_lines: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_commit: Option<String>,
}

fn walk_files(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_files(&p, root, out);
        } else if p.is_file() {
            out.push(posix_rel(&p, root));
        }
    }
}

/// Python 切片 `xs[start:stop]`（start 可為 -1：`a=0` 時 Python 會算成 `[-1:b]`）。
pub(super) fn py_slice<'a>(xs: &[&'a str], start: i64, stop: i64) -> Vec<&'a str> {
    let n = xs.len() as i64;
    let norm = |i: i64| if i < 0 { (n + i).max(0) } else { i.min(n) };
    let (s, e) = (norm(start), norm(stop));
    if s >= e {
        return Vec::new();
    }
    xs[s as usize..e as usize].to_vec()
}

/// 對應 `crossref.read`（L2）：讀 `<repo>:<export>/子路徑`。檔案回內容（可指定行段），目錄回清單。
/// 一律附 commit（引用時說得出版本）；長得像憑證的檔拒讀（`secret`，洩密哨兵同 pack）。
/// Python 預設 `max_chars=MAX_READ_CHARS`。
pub fn read(
    spine_dir: &Path,
    reference: &str,
    consumer: Option<&str>,
    lines: Option<&str>,
    max_chars: usize,
) -> Result<ReadOut> {
    let r = resolve(spine_dir, reference)?;
    if !r.exists {
        return Err(ref_err(
            crate::RejectCode::NotFound,
            format!("不存在：{}/{}", r.repo, r.rel),
            r.hint.clone(),
        ));
    }
    let root = PathBuf::from(&r.root);
    let target = PathBuf::from(&r.path);
    let mut out = ReadOut {
        ok: true,
        repo: r.repo.clone(),
        export: r.export.clone(),
        rel: r.rel.clone(),
        path: r.path.clone(),
        commit: head(&root),
        kind: r.kind,
        entries: None,
        total_entries: None,
        lines: None,
        total_lines: None,
        truncated: None,
        content: None,
        hint: None,
        file_commit: None,
    };
    if r.kind == PathKind::Dir {
        let spec = if r.rel == "." { "." } else { r.rel.as_str() };
        let (rc, listed) = git(
            &root,
            &["ls-files", "-z", "-co", "--exclude-standard", "--", spec],
            GIT_TIMEOUT,
        );
        let names: Vec<String> = if rc == 0 {
            listed
                .split('\0')
                .filter(|x| !x.is_empty())
                .map(str::to_string)
                .collect()
        } else {
            let mut v = Vec::new();
            walk_files(&target, &root, &mut v);
            v
        };
        let prefix = if r.rel == "." {
            String::new()
        } else {
            format!("{}/", r.rel.trim_end_matches('/'))
        };
        let mut children: BTreeMap<String, usize> = BTreeMap::new();
        let mut files = Vec::new();
        for n in &names {
            let sub = n.strip_prefix(prefix.as_str()).unwrap_or(n);
            match sub.split_once('/') {
                Some((first, _)) => *children.entry(format!("{first}/")).or_default() += 1,
                None => files.push(sub.to_string()),
            }
        }
        files.sort();
        let entries: Vec<DirEntry> = children
            .into_iter()
            .map(|(name, n)| DirEntry {
                name,
                files: Some(n),
            })
            .chain(files.into_iter().map(|name| DirEntry { name, files: None }))
            .collect();
        out.total_entries = Some(entries.len());
        out.entries = Some(entries.into_iter().take(MAX_DIR_ENTRIES).collect());
        out.hint = Some("挑檔案再 read `<repo>:<路徑>`；不確定哪份相關就先用 find。".into());
    } else {
        let text = read_text_lossy(&target)
            .map_err(|e| ref_err(crate::RejectCode::Failed, format!("讀不到：{e}"), None))?;
        let hits = pack::secret_hits(&text);
        if !hits.is_empty() {
            return Err(ref_err(
                crate::RejectCode::Secret,
                format!(
                    "{}/{} 疑似含憑證（{}），不讀出",
                    r.repo,
                    r.rel,
                    hits.join(", ")
                ),
                Some("這是洩密哨兵；真的需要請使用者自己看。".into()),
            ));
        }
        let all_lines = splitlines(&text);
        let body = match parse_lines(lines)? {
            Some((a, b)) => {
                out.lines = Some(format!("{a}-{}", b.min(all_lines.len())));
                py_slice(&all_lines, a as i64 - 1, b as i64).join("\n")
            }
            None => text.clone(),
        };
        let truncated = body.chars().count() > max_chars;
        out.total_lines = Some(all_lines.len());
        out.truncated = Some(truncated);
        out.content = Some(take_chars(&body, max_chars).to_string());
        if truncated {
            out.hint = Some(format!(
                "內容超過 {max_chars} 字被截斷；用 lines 指定段落（總共 {} 行）。",
                all_lines.len()
            ));
        }
        let (rc, fc) = git(
            &root,
            &["log", "-1", "--format=%h", "--", &r.rel],
            GIT_TIMEOUT,
        );
        if rc == 0 && !fc.is_empty() {
            out.file_commit = Some(fc);
        }
    }
    record_cite(
        spine_dir,
        consumer,
        &r.repo,
        &r.rel,
        out.commit.as_deref(),
        "read",
        None,
        None,
    )?;
    Ok(out)
}
