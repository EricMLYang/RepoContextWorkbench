//! P6 打包（簡版）：只撞文件層（v2 §9-7 最簡選料規則）。對應 `repo_context/pack.py`。
//!
//! 選料＝各 repo 的 *.md，git repo 走 `git ls-files` **尊重 .gitignore**
//! （L4 實錘：rglob 會把 tool_experiments 的 clone 森林全掃進料，小檔擠占預算，
//! 自己 repo 的正文反而被擠到未納入）；非 git 目錄 fallback 遞迴走訪。
//! 由小到大裝進 token 預算（粗估 tokens ≈ chars / 3），裝不下列在「未納入」。
//! 洩密哨兵抄 Repomix 的 Secretlint 課（拆機報告 §4.1）：引擎要開源、資料要私有，
//! 打包＝內容離開 repo 的那一刻，長得像憑證的檔案在這裡自動擋下（公私分界的哨兵）。
//! 輸出格式維持單一 md，下一代工具仍能直接讀。

use std::path::{Path, PathBuf};
use std::process::Command;

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;

use crate::fsutil::{expand_user, read_text_lossy, splitlines};

/// 對應 `pack.EXCLUDE_DIRS`（非 git fallback 時略過的目錄名）。
pub const EXCLUDE_DIRS: &[&str] = &[".git", "node_modules", "__pycache__", ".venv", "venv"];

/// 對應 `pack.SECRET_PATTERNS`：高精度樣式優先（誤攔正文比漏攔更傷用感；正式版換 Secretlint 全集）。
static SECRET_PATTERNS: Lazy<Vec<(&'static str, Regex)>> = Lazy::new(|| {
    let pats: [(&str, &str); 5] = [
        ("private-key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
        ("aws-key", r"\bAKIA[0-9A-Z]{16}\b"),
        (
            "github-token",
            r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,}\b|\bgithub_pat_[A-Za-z0-9_]{22,}\b",
        ),
        ("slack-token", r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b"),
        (
            "generic-secret",
            r#"(?i)\b(?:api[_-]?key|secret[_-]?key|access[_-]?token)\b\s*[:=]\s*['"][A-Za-z0-9_\-]{16,}['"]"#,
        ),
    ];
    pats.iter()
        .map(|(n, p)| (*n, Regex::new(p).expect("SECRET_PATTERNS 寫死的 regex")))
        .collect()
});

/// 一個要選料的 repo：registry entry 的 `id`＋`path`（呼叫端從 registry 取）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    pub id: String,
    pub path: PathBuf,
}

impl RepoRef {
    pub fn new(id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        RepoRef {
            id: id.into(),
            path: path.into(),
        }
    }
}

/// registry 登記項 → 選料 entry（`id`＋原樣的 `path`，`~` 由選料端展開）。
impl From<&crate::registry::Repo> for RepoRef {
    fn from(r: &crate::registry::Repo) -> Self {
        RepoRef::new(r.id(), r.path_str())
    }
}

/// 對應 `pack.secret_hits`：回傳命中的樣式名清單（空＝乾淨）。
pub fn secret_hits(text: &str) -> Vec<&'static str> {
    SECRET_PATTERNS
        .iter()
        .filter(|(_, rx)| rx.is_match(text))
        .map(|(n, _)| *n)
        .collect()
}

/// 對應 `pack._doc_files`：repo 底下的 *.md 檔（絕對路徑）。
///
/// git repo 走 `git ls-files -z -co --exclude-standard -- *.md`（tracked ＋ untracked-not-ignored）。
/// `-z`：不然 git 會把非 ASCII 路徑加引號跳脫（core.quotePath），中文檔名的 md 全部「不存在」
/// 而被默默略過（2026-09-25 搜尋測試抓到）。非 git 目錄 fallback 遞迴，略過 EXCLUDE_DIRS 與 `.` 開頭目錄。
pub fn doc_files(repo_path: &Path) -> Vec<PathBuf> {
    let root = expand_user(repo_path);
    let r = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["ls-files", "-z", "-co", "--exclude-standard", "--", "*.md"])
        .output();
    if let Ok(out) = r {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            return stdout
                .split('\0')
                .filter(|ln| !ln.is_empty())
                .map(|ln| root.join(ln))
                .filter(|p| p.is_file())
                .collect();
        }
    }
    let mut out = Vec::new();
    walk_md(&root, &mut out);
    out.sort();
    out
}

/// fallback 遞迴（對應 `root.rglob("*.md")`＋目錄過濾）。不跟隨目錄 symlink，避免迴圈。
fn walk_md(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        let p = ent.path();
        let Ok(ft) = ent.file_type() else { continue };
        if ft.is_dir() {
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || EXCLUDE_DIRS.contains(&name.as_ref()) {
                continue;
            }
            walk_md(&p, out);
        } else if ent.file_name().to_string_lossy().ends_with(".md") {
            out.push(p);
        }
    }
}

/// `f.relative_to(root).as_posix()`。
pub(crate) fn rel_posix(f: &Path, root: &Path) -> String {
    let rel = f.strip_prefix(root).unwrap_or(f);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// 對應 `pack.est_tokens`：粗估 tokens ≈ 字元數 / 3 ＋ 1。
pub fn est_tokens(text: &str) -> usize {
    text.chars().count() / 3 + 1
}

/// 對應 `pack.pack_index`：P16 會話料＝**文件地圖**，不是全文（2026-09-01 L4 回饋「料太長」）。
///
/// 互動 agent 自己有讀檔工具，開場塞 60k token 全文只會讓它讀到飽——給路徑＋行數＋首標題的地圖，
/// 讓它先挑再細讀。全文打包（`pack_group`）保留給 P7 headless 判定（背景判定器沒有工具，只能餵全文）。
/// Python 預設 `max_files_per_repo=80`。
pub fn pack_index(entries: &[RepoRef], max_files_per_repo: usize) -> String {
    let mut out = vec!["# 會話料（文件地圖——先挑相關檔案自己細讀，不要整包吞）".to_string()];
    for e in entries {
        let root = expand_user(&e.path);
        if !root.is_dir() {
            continue;
        }
        let mut infos: Vec<(String, usize, String)> = Vec::new();
        for f in doc_files(&root) {
            let Ok(text) = read_text_lossy(&f) else {
                continue;
            };
            let heading = splitlines(&text)
                .into_iter()
                .find(|ln| ln.starts_with('#'))
                .map(|ln| ln.trim_start_matches('#').trim().to_string())
                .unwrap_or_default();
            infos.push((
                rel_posix(&f, &root),
                text.matches('\n').count() + 1,
                heading,
            ));
        }
        infos.sort_by_key(|x| std::cmp::Reverse(x.1)); // 大檔在前（通常是正文）
        out.push(format!(
            "\n## {}（{} 個 md，路徑：{}）",
            e.id,
            infos.len(),
            root.display()
        ));
        for (rel, lines, heading) in infos.iter().take(max_files_per_repo) {
            let tail = if heading.is_empty() {
                String::new()
            } else {
                format!("｜{heading}")
            };
            out.push(format!("- {rel}（{lines} 行）{tail}"));
        }
        if infos.len() > max_files_per_repo {
            out.push(format!(
                "- …另 {} 個較小檔未列",
                infos.len() - max_files_per_repo
            ));
        }
    }
    out.join("\n") + "\n"
}

/// `estimate` 的一列。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Estimate {
    pub id: String,
    pub files: usize,
    pub tokens: usize,
}

/// 對應 `pack.estimate`：token 預算函數（Repomix 課，拆機報告 §4.1）——「這組撞下去要花多少」
/// 先算再挑，挑的粒度由這個數字反推，不憑感覺。
pub fn estimate(entries: &[RepoRef]) -> Vec<Estimate> {
    entries
        .iter()
        .map(|e| {
            let root = expand_user(&e.path);
            let (mut files, mut tokens) = (0, 0);
            if root.is_dir() {
                for f in doc_files(&root) {
                    let Ok(text) = read_text_lossy(&f) else {
                        continue;
                    };
                    files += 1;
                    tokens += est_tokens(&text);
                }
            }
            Estimate {
                id: e.id.clone(),
                files,
                tokens,
            }
        })
        .collect()
}

/// `pack_group` 的結果——對應 Python 回傳的四元組 (打包文字, 收錄, 未納入, 疑似機密)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packed {
    pub text: String,
    pub included: Vec<String>,
    pub excluded: Vec<String>,
    pub flagged: Vec<String>,
}

/// 對應 `pack.pack_group`。`token_budget` 由呼叫端從 config 讀（Python 端亦由呼叫端傳入）。
pub fn pack_group(entries: &[RepoRef], token_budget: usize) -> Packed {
    let mut files: Vec<(String, String, String)> = Vec::new();
    let mut flagged = Vec::new();
    for e in entries {
        let root = expand_user(&e.path);
        if !root.is_dir() {
            continue;
        }
        for f in doc_files(&root) {
            let Ok(text) = read_text_lossy(&f) else {
                continue;
            };
            let rel = rel_posix(&f, &root);
            let hits = secret_hits(&text);
            if !hits.is_empty() {
                // 洩密哨兵：內容不進包、不進 LLM
                flagged.push(format!("{}/{}（{}）", e.id, rel, hits.join(", ")));
                continue;
            }
            files.push((e.id.clone(), rel, text));
        }
    }
    files.sort_by_cached_key(|x| x.2.chars().count()); // 由小到大裝（穩定排序，同 Python）
    let (mut used, mut included, mut excluded, mut chunks) =
        (0usize, Vec::new(), Vec::new(), String::new());
    for (repo_id, rel, text) in &files {
        let t = est_tokens(text);
        if used + t > token_budget {
            excluded.push(format!("{repo_id}/{rel} (~{t} tokens)"));
            continue;
        }
        used += t;
        included.push(format!("{repo_id}/{rel} (~{t} tokens)"));
        chunks.push_str(&format!("\n===== {repo_id}/{rel} =====\n{text}"));
    }
    let mut header = vec![
        "# pack（文件層選料）".to_string(),
        format!("預算 ~{token_budget} tokens，實用 ~{used}"),
        "## 收錄".to_string(),
    ];
    header.extend(included.iter().map(|x| format!("- {x}")));
    if !excluded.is_empty() {
        header.push("## 未納入（超預算——漏撞來源，判定時要知道）".to_string());
        header.extend(excluded.iter().map(|x| format!("- {x}")));
    }
    if !flagged.is_empty() {
        header.push("## 疑似機密（洩密哨兵已排除——公私分界，內容不出 repo）".to_string());
        header.extend(flagged.iter().map(|x| format!("- {x}")));
    }
    Packed {
        text: header.join("\n") + "\n" + &chunks,
        included,
        excluded,
        flagged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_patterns_each_hit() {
        assert_eq!(
            secret_hits("-----BEGIN RSA PRIVATE KEY-----"),
            vec!["private-key"]
        );
        assert_eq!(secret_hits("key AKIAABCDEFGHIJKLMNOP x"), vec!["aws-key"]);
        assert_eq!(
            secret_hits(&format!("ghp_{}", "a".repeat(36))),
            vec!["github-token"]
        );
        assert_eq!(secret_hits("xoxb-1234567890ab"), vec!["slack-token"]);
        assert_eq!(
            secret_hits("API_KEY = \"abcdefghijklmnop\""),
            vec!["generic-secret"]
        );
        assert!(secret_hits("普通筆記 api key 很重要").is_empty());
    }

    #[test]
    fn est_tokens_counts_chars() {
        assert_eq!(est_tokens(""), 1);
        assert_eq!(est_tokens("中文字"), 2);
    }
}
