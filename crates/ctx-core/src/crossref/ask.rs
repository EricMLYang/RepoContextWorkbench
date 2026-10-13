//! 委派（L3）：在對方 repo 開唯讀 agent 回答。

use super::*;

// ---------------------------------------------------------------- 委派（L3）

/// 對應 `crossref.ASK_PROMPT`（`{repo}`／`{consumer}`／`{question}` 由 [`ask_prompt`] 代入）。
pub const ASK_PROMPT: &str = "你在 repo「{repo}」裡，被另一個 repo（{consumer}）的 agent 請來回答一個問題。
只能讀檔（Read／Grep／Glob），不要改任何東西。先照這個 repo 的 AGENTS.md／CLAUDE.md 慣例找資料。

【問題】
{question}

回答規則：
- 只根據這個 repo 裡的檔案回答；找不到就直說找不到，不要編。
- 結論精簡（300 字以內），每個重點標出來源檔（repo 內相對路徑，能給行號就給）。
- 最後只輸出一個 JSON 物件，不要其他文字：
{\"answer\": \"<回答>\", \"citations\": [{\"file\": \"<相對路徑>\", \"lines\": \"<例 12-40，可省略>\", \"why\": \"<這份檔支持哪個重點>\"}], \"not_found\": false}
";

/// 組 ask_repo 的 prompt（Python `ASK_PROMPT.format(...)`）。
pub fn ask_prompt(repo: &str, consumer: &str, question: &str) -> String {
    ASK_PROMPT
        .replacen("{repo}", repo, 1)
        .replacen("{consumer}", consumer, 1)
        .replacen("{question}", question, 1)
}

/// ask_repo 回答解析後的形狀（對應 `_parse_ask`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AskParsed {
    pub answer: String,
    pub citations: Vec<Value>,
    pub not_found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// 對應 `crossref._parse_ask`：取第一個 `{` 到最後一個 `}` 解析；不是預期的 JSON 就原文照回。
pub fn parse_ask(text: &str) -> AskParsed {
    if let (Some(a), Some(b)) = (text.find('{'), text.rfind('}')) {
        if a < b {
            if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&text[a..=b]) {
                if let Some(ans) = obj.get("answer") {
                    let citations = match obj.get("citations") {
                        Some(Value::Array(xs)) => xs
                            .iter()
                            .filter(|c| {
                                c.as_object().is_some_and(|o| {
                                    o.get("file").is_some_and(crate::pystr::truthy)
                                })
                            })
                            .cloned()
                            .collect(),
                        _ => Vec::new(),
                    };
                    return AskParsed {
                        answer: crate::pystr::py_str(ans),
                        citations,
                        not_found: obj.get("not_found").is_some_and(crate::pystr::truthy),
                        note: None,
                    };
                }
            }
        }
    }
    AskParsed {
        answer: text.trim().to_string(),
        citations: Vec::new(),
        not_found: false,
        note: Some("回答不是預期的 JSON，原文照回；沒有結構化引用。".into()),
    }
}

/// [`ask_repo`] 的結果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AskResult {
    pub ok: bool,
    pub repo: String,
    pub provider: String,
    pub commit: Option<String>,
    #[serde(flatten)]
    pub parsed: AskParsed,
}

/// 文字 agent 的呼叫介面（同 `agents::run_text` 的參數：provider 名, prompt, spine, model, cwd, read_only）。
/// 測試注入假的用（Python 是 monkeypatch `agents.run_text`）。
pub type RunText<'a> =
    dyn Fn(&str, &str, &Path, Option<&str>, Option<&Path>, bool) -> Result<String> + 'a;

/// 對應 `crossref.ask_repo`（L3）：在對方 repo 開一個唯讀 agent 回答問題，只回結論＋引用，
/// 不把一堆檔案塞進你的 context。provider 預設 config `provider.ask`（沒設＝claude）。
pub fn ask_repo(
    spine_dir: &Path,
    repo_id: &str,
    question: &str,
    consumer: Option<&str>,
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<AskResult> {
    ask_repo_with(
        spine_dir,
        repo_id,
        question,
        consumer,
        provider,
        model,
        &agents::run_text,
    )
}

/// [`ask_repo`] 的本體，文字 agent 由呼叫端給。
#[allow(clippy::too_many_arguments)]
pub fn ask_repo_with(
    spine_dir: &Path,
    repo_id: &str,
    question: &str,
    consumer: Option<&str>,
    provider: Option<&str>,
    model: Option<&str>,
    run_text: &RunText<'_>,
) -> Result<AskResult> {
    if question.trim().is_empty() {
        return Err(ref_err(
            crate::RejectCode::BadRequest,
            "question 不能是空的",
            None,
        ));
    }
    let entry = repo_entry(spine_dir, repo_id)?;
    let root = py_resolve(&entry.path());
    if !root.is_dir() {
        return Err(ref_err(
            crate::RejectCode::NotFound,
            format!("{repo_id} 的路徑不存在：{}", root.display()),
            None,
        ));
    }
    let cfg = config::load(spine_dir)?;
    let provider = provider
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .or_else(|| {
            cfg.str("provider.ask")
                .filter(|p| !p.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "claude".into());
    let prompt = ask_prompt(
        repo_id,
        consumer.filter(|c| !c.is_empty()).unwrap_or("未登記的目錄"),
        question.trim(),
    );
    let raw = match run_text(&provider, &prompt, spine_dir, model, Some(&root), true) {
        Ok(s) => s,
        Err(Error::External(e)) => {
            return Err(ref_err(
                crate::RejectCode::Failed,
                format!("ask_repo 失敗：{e}"),
                Some("改用 find＋read 自己找；或確認 claude CLI 已登入。".into()),
            ))
        }
        Err(e) => return Err(e),
    };
    let parsed = parse_ask(&raw);
    let commit = head(&root);
    for c in &parsed.citations {
        let file = c.get("file").map(crate::pystr::py_str).unwrap_or_default();
        let rel = file.trim_start_matches(['.', '/']);
        record_cite(
            spine_dir,
            consumer,
            repo_id,
            rel,
            commit.as_deref(),
            "ask",
            None,
            None,
        )?;
    }
    Ok(AskResult {
        ok: true,
        repo: repo_id.to_string(),
        provider,
        commit,
        parsed,
    })
}
