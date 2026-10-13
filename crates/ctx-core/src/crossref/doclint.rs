//! agent 文件裡指向其他 repo、已失效的路徑。

use super::*;

// ---------------------------------------------------------------- agent 文件裡的跨 repo 路徑

/// 對應 `crossref.AGENT_DOCS`。
pub const AGENT_DOCS: [&str; 3] = ["AGENTS.md", "CLAUDE.md", "GEMINI.md"];
/// 對應 `crossref.SKILL_GLOBS`（`<dir>/*/SKILL.md`）。
pub const SKILL_DIRS: [&str; 2] = [".claude/skills", ".agents/skills"];

static CODE_SPAN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`([^`\n]+)`").expect("寫死的 regex"));
static MD_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\]\(([^)\s]+)\)").expect("寫死的 regex"));
/// `_BARE_UP` 去掉 lookbehind 的部分（regex crate 不支援 lookbehind，由 [`bare_ups`] 補判）。
static BARE_UP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\.\./[^\s`)\]'"，。、；：）]+"#).expect("寫死的 regex"));
static GLOB_CUT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[*<{?]").expect("寫死的 regex"));
static LINE_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r":\d+(?:-\d+)?$").expect("寫死的 regex"));
const TRAIL: &[char] = &[
    '，', '。', '、', '；', '：', '）', ')', ']', '}', '>', '\'', '"', '.', ',', ';', ':',
];

fn doc_files(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = AGENT_DOCS
        .iter()
        .map(|n| root.join(n))
        .filter(|p| p.is_file())
        .collect();
    for d in SKILL_DIRS {
        let mut found: Vec<PathBuf> = std::fs::read_dir(root.join(d))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path().join("SKILL.md"))
                    .filter(|p| p.is_file())
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        out.extend(found);
    }
    out
}

/// Python `(?<![\w/.`])(\.\./...)` 的 findall：逐字元掃，前一字元不是字元／`/`／`.`／反引號才算。
pub(super) fn bare_ups(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut prev: Option<char> = None;
    let mut i = 0;
    while i < line.len() {
        let ok_before =
            prev.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || "/.`".contains(c)));
        if ok_before {
            if let Some(m) = BARE_UP.find(&line[i..]) {
                out.push(m.as_str().to_string());
                let end = i + m.end();
                prev = line[..end].chars().next_back();
                i = end;
                continue;
            }
        }
        let c = line[i..].chars().next().expect("i 在字元邊界上");
        prev = Some(c);
        i += c.len_utf8();
    }
    out
}

fn path_candidates(line: &str) -> Vec<String> {
    let cands = CODE_SPAN
        .captures_iter(line)
        .map(|c| c[1].to_string())
        .chain(MD_LINK.captures_iter(line).map(|c| c[1].to_string()))
        .chain(bare_ups(line));
    let mut out: Vec<String> = Vec::new();
    for c in cands {
        let c = c.trim().trim_end_matches(TRAIL);
        if !c.contains('/')
            || c.contains(' ')
            || c.contains("://")
            || c.starts_with(['-', '@', '#'])
        {
            continue;
        }
        if c.contains("...") || c.contains('…') {
            continue; // 省略號＝示意路徑，不驗
        }
        let c = GLOB_CUT.split(c).next().unwrap_or(""); // glob／佔位符之後不驗
        let c = LINE_SUFFIX.replace(c, "").into_owned(); // 檔案:行號
        if !c.is_empty() && !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// [`lint_doc_refs`] 的一筆失效引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BrokenRef {
    pub repo: String,
    pub doc: String,
    pub line: usize,
    #[serde(rename = "ref")]
    pub reference: String,
    pub target: String,
    pub target_path: String,
    pub fix: String,
}

/// 對應 `crossref.lint_doc_refs`：掃已登記 repo 的 agent 文件（AGENTS／CLAUDE／GEMINI.md、skills 的 SKILL.md），
/// 找出「指向其他已登記 repo、但路徑已失效」的引用。只報有把握的：路徑裡要出現某個其他 repo 的 id
/// 或資料夾名，且依文件所在位置、repo 根目錄都解析不到；以 repo 名開頭、子路徑在該 repo 裡確實存在的
/// （`<repo>/子路徑`）視為用名字引用，不報。`repo_ids=None`＝全部。
pub fn lint_doc_refs(spine_dir: &Path, repo_ids: Option<&[String]>) -> Result<Vec<BrokenRef>> {
    let data = registry::load(spine_dir)?;
    let mut names: HashMap<String, &Repo> = HashMap::new();
    for r in &data.repos {
        names.insert(r.id(), r);
    }
    for r in &data.repos {
        if let Some(n) = r.path().file_name() {
            names.entry(n.to_string_lossy().into_owned()).or_insert(r);
        }
    }
    let mut out = Vec::new();
    for r in data
        .repos
        .iter()
        .filter(|r| repo_ids.is_none_or(|ids| ids.contains(&r.id())))
    {
        let root = r.path();
        if !root.is_dir() {
            continue;
        }
        let own: [String; 2] = [
            r.id(),
            root.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ];
        for doc in doc_files(&root) {
            let Ok(text) = read_text_lossy(&doc) else {
                continue;
            };
            let mut in_fence = false;
            for (i, line) in splitlines(&text).into_iter().enumerate() {
                if line.trim_start().starts_with("```") {
                    in_fence = !in_fence;
                    continue;
                }
                if in_fence || line.contains("~~") {
                    continue; // 程式碼區塊是範例；刪除線是已作廢的舊路徑
                }
                for c in path_candidates(line) {
                    let segs: Vec<&str> = c
                        .split('/')
                        .filter(|s| !["", ".", "..", "~"].contains(s))
                        .collect();
                    let Some(seg) = segs
                        .iter()
                        .copied()
                        .find(|s| names.contains_key(*s) && !own.iter().any(|o| o == s))
                    else {
                        continue;
                    };
                    let target = names[seg];
                    let tries: Vec<PathBuf> = if c.starts_with('~') || c.starts_with('/') {
                        vec![expanduser(&c)]
                    } else {
                        vec![doc.parent().unwrap_or(&root).join(&c), root.join(&c)]
                    };
                    if tries.iter().any(|t| t.exists()) {
                        continue;
                    }
                    let after = c
                        .split_once(seg)
                        .map(|(_, a)| a.trim_matches('/'))
                        .unwrap_or("")
                        .to_string();
                    let tpath = target.path();
                    if segs[0] == seg && tpath.is_dir() && tpath.join(&after).exists() {
                        continue; // `<repo名>/子路徑` 且子路徑在該 repo 裡存在＝用名字引用，不算失效
                    }
                    let tid = target.id();
                    let reference = if after.is_empty() {
                        tid.clone()
                    } else {
                        format!("{tid}:{after}")
                    };
                    out.push(BrokenRef {
                        repo: r.id(),
                        doc: posix_rel(&doc, &root),
                        line: i + 1,
                        reference: c.clone(),
                        target: tid.clone(),
                        target_path: target.path_str(),
                        fix: format!(
                            "{tid} 實際在 {}；改寫成 `{reference}` 並用 `ctx resolve {reference}` 取路徑",
                            target.path_str()
                        ),
                    });
                }
            }
        }
    }
    Ok(out)
}
