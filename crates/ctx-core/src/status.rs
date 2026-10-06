//! agent 階段回報的儲存（從 Python `agentapi.py` 的 `_status_dir`／`_status_key`／
//! `report_status` 寫檔部分／`get_status`／`all_statuses` 拆出來）。
//!
//! 為什麼獨立成模組：工作台（`workbench::state`）要讀「每組 agent 目前在做什麼」，
//! 但不該為此依賴整個 agentapi。這是會一直被覆寫的「現況」，不是歷史——
//! 歷史（waiting／blocked 留給使用者的事件）由 `agentapi::report_status` 另外落 spine。
//!
//! 檔案格式（合約）：`.state/agent_status/<key>.json`，單行
//! `json.dumps(rec, ensure_ascii=False)`，欄位依序 status／text／at／scope／group／repos／repo。

use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result};

/// 對應 `agentapi.STATUSES`。
pub const STATUSES: [&str; 4] = ["working", "waiting", "blocked", "done"];

/// agent 回報的階段（序列化字串與 Python 相同）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Working,
    Waiting,
    Blocked,
    Done,
}

impl AgentStatus {
    /// 對應 `agentapi.STATUSES` 的字串。
    pub fn as_str(self) -> &'static str {
        match self {
            AgentStatus::Working => "working",
            AgentStatus::Waiting => "waiting",
            AgentStatus::Blocked => "blocked",
            AgentStatus::Done => "done",
        }
    }

    /// 字串 → 階段；不認得回 `None`（呼叫端決定錯誤訊息）。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "working" => Some(AgentStatus::Working),
            "waiting" => Some(AgentStatus::Waiting),
            "blocked" => Some(AgentStatus::Blocked),
            "done" => Some(AgentStatus::Done),
            _ => None,
        }
    }
}

/// 一筆現況（`.state/agent_status/<key>.json` 的內容）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusRecord {
    pub status: String,
    pub text: String,
    /// `YYYY-MM-DD HH:MM`
    pub at: String,
    /// 範圍標籤（組名／臨時範圍名／repo id／「全部」）
    pub scope: String,
    pub group: Option<String>,
    pub repos: Option<Vec<String>>,
    /// 回報時 cwd 所在的 repo
    pub repo: Option<String>,
}

/// 對應 `agentapi._status_dir`。
pub fn status_dir(spine_dir: impl AsRef<Path>) -> PathBuf {
    spine_dir.as_ref().join(".state").join("agent_status")
}

static KEY_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[^\w\-\x{3400}-\x{9FFF}]+").expect("固定的 regex"));

/// 對應 `agentapi._status_key`：組名優先，否則 repo id 以逗號串，都沒有＝「全部」；非字元換 `_`。
pub fn status_key<S: AsRef<str>>(group: Option<&str>, repos: Option<&[S]>) -> String {
    let raw = match group.filter(|g| !g.is_empty()) {
        Some(g) => g.to_string(),
        None => {
            let joined = repos
                .unwrap_or(&[])
                .iter()
                .map(|s| s.as_ref())
                .collect::<Vec<_>>()
                .join(",");
            if joined.is_empty() {
                "全部".to_string()
            } else {
                joined
            }
        }
    };
    KEY_RE.replace_all(&raw, "_").into_owned()
}

/// 對應 `agentapi.report_status` 的寫檔部分：覆寫該範圍的現況檔，回傳檔案路徑。
pub fn save_status(spine_dir: impl AsRef<Path>, rec: &StatusRecord) -> Result<PathBuf> {
    let d = status_dir(&spine_dir);
    std::fs::create_dir_all(&d)?;
    let p = d.join(format!(
        "{}.json",
        status_key(rec.group.as_deref(), rec.repos.as_deref())
    ));
    let v = serde_json::to_value(rec)?;
    std::fs::write(&p, crate::agentmark::py_dumps(&v))?;
    Ok(p)
}

/// 對應 `agentapi.get_status(spine_dir, {"group":…, "repos":…})`：讀不到或壞掉＝`None`。
/// 回原樣 JSON（Python 寫入的檔可能有多的欄位，照樣帶出去）。
pub fn get_status<S: AsRef<str>>(
    spine_dir: impl AsRef<Path>,
    group: Option<&str>,
    repos: Option<&[S]>,
) -> Option<Value> {
    let p = status_dir(spine_dir).join(format!("{}.json", status_key(group, repos)));
    let s = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

/// 對應 `agentapi.all_statuses`：依檔名排序，壞檔略過。
pub fn all_statuses(spine_dir: impl AsRef<Path>) -> Vec<Value> {
    let d = status_dir(spine_dir);
    let Ok(rd) = std::fs::read_dir(&d) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|p| {
            let s = std::fs::read_to_string(p).ok()?;
            serde_json::from_str(&s).ok()
        })
        .collect()
}

/// 檢查 status 字串；不合法時的錯誤與 Python `report_status` 相同（code=bad_request）。
pub fn parse_status(status: &str) -> Result<AgentStatus> {
    AgentStatus::parse(status).ok_or_else(|| {
        Error::rejected(
            "bad_request",
            format!(
                "status 需為 {}：{}",
                STATUSES.join("／"),
                crate::pystr::repr(status)
            ),
            None,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_matches_python() {
        assert_eq!(status_key::<&str>(Some("產品 線"), None), "產品_線");
        assert_eq!(status_key(None, Some(&["a", "b"][..])), "a_b");
        assert_eq!(status_key::<&str>(None, None), "全部");
        assert_eq!(status_key::<String>(None, Some(&[][..])), "全部");
    }

    #[test]
    fn roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let rec = StatusRecord {
            status: "working".into(),
            text: "x".into(),
            at: "2026-10-06 10:00".into(),
            scope: "g1".into(),
            group: Some("g1".into()),
            repos: Some(vec!["a".into()]),
            repo: Some("a".into()),
        };
        let p = save_status(tmp.path(), &rec).unwrap();
        let raw = std::fs::read_to_string(p).unwrap();
        assert!(raw.starts_with("{\"status\": \"working\", \"text\": \"x\""));
        let got = get_status::<&str>(tmp.path(), Some("g1"), None).unwrap();
        assert_eq!(got["scope"], "g1");
        assert_eq!(all_statuses(tmp.path()).len(), 1);
    }
}
