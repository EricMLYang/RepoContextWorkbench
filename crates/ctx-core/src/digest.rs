//! P5 理解 digest——agent 讀變更寫摘要（v2 §2 P5：一律增量、一律便宜模型）。對應 Python `repo_context/digest.py`。
//!
//! 增量＝只 digest 上次 digest 事件之後的 commit（水位線就在脊椎裡：
//! 最後一筆 `suggestion [digest] repo:<id>` 的 date+time）；沒有新 commit 就不寫事件（無事不報）。
//! 模型走 config `provider.models.digest`（預設便宜模型）；agent 失敗落 system-unsure 事件，不准沉默。

use std::path::Path;
use std::process::Command;

use chrono::{Local, NaiveDateTime};
use serde::Serialize;

use crate::agents::{self, AgentProvider};
use crate::spine::{self, EventType};
use crate::{config, registry, Error, Result};

/// 對應 `digest.PROMPT`（`{rid}`、`{log}` 由 [`prompt`] 代入）。
pub const PROMPT: &str =
    "你是 repo 變更摘要器。以下是 repo「{rid}」自上次摘要以來的 commit 與變更統計。
用繁體中文寫 3 行以內的摘要：改了什麼、為什麼重要、有沒有需要人注意的點。只輸出摘要本文。

{log}
";

fn prompt(rid: &str, log: &str) -> String {
    PROMPT.replacen("{rid}", rid, 1).replacen("{log}", log, 1)
}

/// 每個 repo 一筆結果（對應 Python `{repo, file|None, note}`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DigestResult {
    pub repo: String,
    pub file: Option<String>,
    pub note: String,
}

/// 對應 `digest._watermark`：最後一筆該 repo 的 digest 事件時間（'YYYY-MM-DD HH:MM'）；沒有 → None。
pub fn watermark(spine_dir: impl AsRef<Path>, rid: &str) -> Result<Option<String>> {
    Ok(spine::iter_events(spine_dir)?
        .iter()
        .rev()
        .find(|ev| {
            ev.etype == EventType::Suggestion && ev.source == "digest" && ev.kv("repo") == Some(rid)
        })
        .map(|ev| ev.stamp()))
}

fn git_log_since(repo_path: &Path, since: Option<&str>) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("log")
        .arg(format!("--since={}", since.unwrap_or("7.days")))
        .args(["--format=%h %ad %s", "--date=short", "--stat"])
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => String::new(),
    }
}

/// 對應 `digest.run`：對組內 mine repo 逐一增量 digest。`provider` 缺＝config `provider.default`。
pub fn run(
    spine_dir: impl AsRef<Path>,
    group: Option<&str>,
    repos: Option<&[String]>,
    provider: Option<&str>,
    when: Option<NaiveDateTime>,
) -> Result<Vec<DigestResult>> {
    let spine_dir = spine_dir.as_ref();
    let cfg = config::load(spine_dir)?;
    let name = provider
        .filter(|p| !p.is_empty())
        .unwrap_or(cfg.provider_default())
        .to_string();
    let p = agents::provider_by_name(&name).ok_or_else(|| {
        Error::invalid(format!(
            "未知 provider: {name}（可用：{}）",
            agents::PROVIDER_NAMES.join(", ")
        ))
    })?;
    run_with(spine_dir, group, repos, p.as_ref(), when)
}

/// `run` 的本體，provider 由呼叫端給（測試注入用）。model 仍讀 config `provider.models.digest`。
pub fn run_with(
    spine_dir: impl AsRef<Path>,
    group: Option<&str>,
    repos: Option<&[String]>,
    provider: &dyn AgentProvider,
    when: Option<NaiveDateTime>,
) -> Result<Vec<DigestResult>> {
    let spine_dir = spine_dir.as_ref();
    let group = group.filter(|g| !g.is_empty());
    let now = when.unwrap_or_else(|| Local::now().naive_local());
    let cfg = config::load(spine_dir)?;
    let model = cfg.provider_model("digest");
    let (_, entries) = registry::resolve_group(spine_dir, group, repos)?;
    let outdir = spine_dir
        .join("groups")
        .join(group.unwrap_or("adhoc"))
        .join("digests");
    let mut results = Vec::new();
    for e in &entries {
        if e.kind().as_deref() != Some("mine") {
            continue;
        }
        let rid = e.id();
        let log = git_log_since(&e.path(), watermark(spine_dir, &rid)?.as_deref());
        if log.is_empty() {
            results.push(DigestResult {
                repo: rid,
                file: None,
                note: "無新 commit".into(),
            });
            continue;
        }
        let mut tokens = vec![format!("repo:{rid}")];
        if let Some(g) = group {
            tokens.push(format!("group:{g}"));
        }
        let text = match agents::run_text_with(
            provider,
            &prompt(&rid, &log),
            spine_dir,
            model,
            None,
            false,
        ) {
            Ok(t) => t,
            Err(Error::External(err)) => {
                spine::append_event(
                    spine_dir,
                    "suggestion",
                    "digest",
                    &tokens,
                    &format!("system-unsure：digest 失敗：{err}"),
                    Some(now),
                )?;
                results.push(DigestResult {
                    repo: rid,
                    file: None,
                    note: "失敗（已浮出）".into(),
                });
                continue;
            }
            Err(e) => return Err(e),
        };
        std::fs::create_dir_all(&outdir)?;
        let out = outdir.join(format!("{}-{rid}.md", now.format("%Y-%m-%d")));
        std::fs::write(
            &out,
            format!(
                "# digest {} {rid}\n\n{text}\n\n## 依據（git log）\n```\n{log}\n```\n",
                now.format("%Y-%m-%d")
            ),
        )?;
        let first = crate::pystr::splitlines(&text)
            .first()
            .copied()
            .unwrap_or("")
            .to_string();
        let fname = out
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        spine::append_event(
            spine_dir,
            "suggestion",
            "digest",
            &tokens,
            &format!("{first}\n→ {fname}"),
            Some(now),
        )?;
        results.push(DigestResult {
            repo: rid,
            file: Some(out.to_string_lossy().into_owned()),
            note: "ok".into(),
        });
    }
    Ok(results)
}
