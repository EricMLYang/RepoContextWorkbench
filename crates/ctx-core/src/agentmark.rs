//! 工作台 agent 會話的存活標記——P3「agent 偵測」的資料源。對應 `repo_context/agentmark.py`。
//!
//! 抄 gitpane（拆機報告 §2.1）：11 個外部工具裡唯一感知「這個 repo 上有 agent 正在跑」的就是它。
//! 原型版不掃全系統進程，只標引擎自己 spawn 的內嵌會話：
//! - 工作台開 agent 會話時 mark、殺會話/進程結束時 unmark
//! - marker 落 `spine/.state/agent_sessions/<sid>.json`（.state/ 已 gitignore，不進 git）
//! - 讀取端以 pid 活性掃；pid 已死的 marker 視為殘骸就地清掉——工作台被硬殺不留幽靈
//! - P16 外開終端的會話生命週期不歸引擎管，不標（誠實邊界）

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;

/// marker 檔內容（欄位與 Python 一致：sid／cwd／agent／pid；不認得的鍵保留）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentMarker {
    #[serde(default)]
    pub sid: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub pid: Option<i64>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn dir(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("agent_sessions")
}

/// 對應 `agentmark.mark`：寫 `<sid>.json`。
pub fn mark(spine_dir: &Path, sid: &str, cwd: &Path, agent: &str, pid: i64) -> Result<()> {
    let d = dir(spine_dir);
    std::fs::create_dir_all(&d)?;
    let m = serde_json::json!({
        "sid": sid, "cwd": cwd.to_string_lossy(), "agent": agent, "pid": pid,
    });
    // Python json.dumps 預設分隔 ", " ": "——照樣寫，兩邊產出的檔一致
    std::fs::write(d.join(format!("{sid}.json")), py_dumps(&m))?;
    Ok(())
}

/// `json.dumps(obj, ensure_ascii=False)` 的單行格式（`, ` 與 `: ` 分隔）。
pub(crate) fn py_dumps(v: &serde_json::Value) -> String {
    use serde_json::Value;
    match v {
        Value::Object(m) => {
            let parts: Vec<String> = m
                .iter()
                .map(|(k, v)| format!("{}: {}", Value::String(k.clone()), py_dumps(v)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(py_dumps).collect::<Vec<_>>().join(", ")
        ),
        other => other.to_string(),
    }
}

/// 對應 `agentmark.unmark`：刪 marker，不存在不報錯。
pub fn unmark(spine_dir: &Path, sid: &str) {
    let _ = std::fs::remove_file(dir(spine_dir).join(format!("{sid}.json")));
}

/// 對應 `agentmark._pid_alive`。unix：`kill(pid, 0)` 語意——ESRCH＝死、EPERM（沒權限查）＝存在。
pub fn pid_alive(pid: Option<i64>) -> bool {
    let Some(pid) = pid else { return false };
    if pid <= 0 {
        return false;
    }
    pid_alive_os(pid)
}

#[cfg(unix)]
fn pid_alive_os(pid: i64) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: kill(pid, 0) 不送訊號，只檢查進程是否存在與權限。
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// Windows：Python 版用 OpenProcess＋GetExitCodeProcess（**不可** `os.kill(pid, 0)`——
/// Windows 上任意 sig 都走 TerminateProcess）。Rust 版暫時一律回 true（未測：Windows 尚未實作，
/// 後果是死掉的會話 marker 不會被自動清，agent 欄位可能多標）。
#[cfg(not(unix))]
fn pid_alive_os(_pid: i64) -> bool {
    true
}

/// 對應 `agentmark.live`：存活的 agent 會話；順手清掉 pid 已死的殘骸。依檔名排序。
pub fn live(spine_dir: &Path) -> Vec<AgentMarker> {
    let d = dir(spine_dir);
    let Ok(rd) = std::fs::read_dir(&d) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(m) = serde_json::from_str::<AgentMarker>(&text) else {
            continue;
        };
        if !pid_alive(m.pid) {
            let _ = std::fs::remove_file(&p);
            continue;
        }
        out.push(m);
    }
    out
}
