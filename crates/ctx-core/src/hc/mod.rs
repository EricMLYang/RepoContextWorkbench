//! herdr-core（`hc`，Apache-2.0）當會話後端（F-128 路線 A，Rust 才有）：App 開的 agent／shell 會話跑在 App 之外，
//! App ⌘Q、當掉、換版之後重開，會話還在、畫面還原、可以繼續輸入。config `session.backend: herdr-core` 才用；預設仍是行程內 PTY。
//!
//! 分工：
//! - [`frame`]：分幀編解碼（純函式）
//! - [`client`]：一條分幀連線——JSON-RPC、attach／events 串流、CREDIT
//! - [`locate`]：找 `hc`、找 socket、`hc server start`
//! - 這裡（[`HcBackend`]）：開會話（`terminal.run`）、attach 進 [`TermSession`]、程式狀態（core 的 `status` 事件 →
//!   [`ProgramRecorder`]）、中繼資料存 `.state/hc_sessions/<sid>.json`、App 開時接回、斷線重連、App 結束時放手。
//!
//! 會話對回來靠終端機的標籤：`ctx.sid`（我們的 sid）、`ctx.spine`（spine 路徑的短雜湊，別的 spine 的會話不接）、`ctx.kind`。
//! 狀態：core 自己解析 OSC 7501、自己回探詢——這條路上**不跑**我們自己的 Scanner（不然探詢會被回兩次）。
//! Windows：herdr-core 走 named pipe，這次沒做——[`HcBackend::start`] 回「尚未支援」，manager 退回 PTY。

pub mod frame;
pub mod locate;

#[cfg(unix)]
pub mod client;
#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::HcBackend;

#[cfg(not(unix))]
mod stub;
#[cfg(not(unix))]
pub use stub::HcBackend;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::term::{TermKind, TermMeta};

/// 開不起來的原因（manager 退回 PTY 時給使用者看）。
#[derive(Debug, Clone)]
pub struct StartError {
    pub reason: String,
    /// 找到的 hc（找到了但叫不起來時才有）
    pub exe: Option<PathBuf>,
}

/// `.state/hc_sessions`：每個 herdr-core 會話一個中繼資料檔。
pub fn meta_dir(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("hc_sessions")
}

fn meta_path(spine_dir: &Path, sid: &str) -> PathBuf {
    meta_dir(spine_dir).join(format!("{sid}.json"))
}

/// 重建會話列需要的欄位（`TermMeta`＋開始時間、目錄、agent 名、core 裡的位址）。只存這些。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredMeta {
    pub sid: String,
    /// core 裡的位址（`t7`）
    pub address: String,
    pub kind: TermKind,
    pub title: String,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub scope_repos: Option<Vec<String>>,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub home_group: Option<String>,
    pub started: String,
    #[serde(default)]
    pub cwd: Option<String>,
    /// agent 名（config `agents` 的鍵）；shell＝None
    #[serde(default)]
    pub agent: Option<String>,
}

impl StoredMeta {
    pub fn term_meta(&self) -> TermMeta {
        TermMeta {
            kind: self.kind,
            title: self.title.clone(),
            origin: self.origin.clone(),
            scope: self.scope.clone(),
            scope_repos: self.scope_repos.clone(),
            task: self.task.clone(),
            home_group: self.home_group.clone(),
        }
    }
}

/// 存一份（原子寫入）。
pub fn save_meta(spine_dir: &Path, m: &StoredMeta) -> crate::Result<()> {
    std::fs::create_dir_all(meta_dir(spine_dir))?;
    let bytes = serde_json::to_vec_pretty(m)?;
    crate::fsutil::write_atomic(&meta_path(spine_dir, &m.sid), &bytes)?;
    Ok(())
}

pub fn load_meta(spine_dir: &Path, sid: &str) -> Option<StoredMeta> {
    let s = std::fs::read_to_string(meta_path(spine_dir, sid)).ok()?;
    serde_json::from_str(&s).ok()
}

pub fn remove_meta(spine_dir: &Path, sid: &str) {
    let _ = std::fs::remove_file(meta_path(spine_dir, sid));
}

/// 新 sid 從哪裡編起：`.state/hc_sessions` 裡最大的 `t<n>` 之後（沒有就 1）。
/// PTY 後端也照這個編：退回 PTY 的那次不會跟還在 core 裡跑的會話撞名（狀態檔是用 sid 命名的）。
pub fn next_free_id(spine_dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(meta_dir(spine_dir)) else {
        return 1;
    };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json")?
                .strip_prefix('t')?
                .parse::<u64>()
                .ok()
        })
        .max()
        .map_or(1, |n| n + 1)
}

/// spine 路徑的短雜湊（`ctx.spine` 標籤）：FNV-1a 64，取 12 個十六進位字。
pub fn spine_tag(spine_dir: &Path) -> String {
    let p = crate::fsutil::resolve(spine_dir);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in p.to_string_lossy().as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")[..12].to_string()
}

/// 標籤鍵。
pub const LABEL_SID: &str = "ctx.sid";
pub const LABEL_SPINE: &str = "ctx.spine";
pub const LABEL_KIND: &str = "ctx.kind";

/// core 裡一個帶我們標籤的終端機（App 開時列出來、準備接回）。
#[derive(Debug, Clone)]
pub struct Existing {
    pub meta: StoredMeta,
    /// core 的 `Terminal` 紀錄
    pub record: serde_json::Value,
}

impl Existing {
    pub fn sid(&self) -> &str {
        &self.meta.sid
    }

    pub fn kind(&self) -> TermKind {
        self.meta.kind
    }

    pub fn live(&self) -> bool {
        self.record.get("state").and_then(|v| v.as_str()) == Some("live")
    }
}

/// core 的 `Status`（OSC 7501 根紀錄，null＝撤掉）→ 我們的 [`ProgramReport`]。不認得的狀態＝None（不記）。
pub fn report_of(status: &serde_json::Value) -> Option<crate::program_status::ProgramReport> {
    use crate::program_status::{BlockedKind, ProgramReport, ProgramState};
    if status.is_null() {
        return Some(ProgramReport {
            id: None,
            state: ProgramState::Clear,
            kind: None,
            progress: None,
            app: None,
            msg: None,
            title: None,
        });
    }
    let s = |k: &str| {
        status
            .get(k)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|v| !v.is_empty())
    };
    let state: ProgramState = serde_json::from_value(status.get("state")?.clone()).ok()?;
    let kind = status
        .get("kind")
        .and_then(|v| serde_json::from_value::<BlockedKind>(v.clone()).ok());
    let progress = status
        .get("progress")
        .and_then(|v| v.as_u64())
        .and_then(|p| u8::try_from(p.min(100)).ok());
    Some(ProgramReport {
        id: None,
        state,
        kind,
        progress,
        app: s("app"),
        msg: s("msg"),
        title: s("title"),
    })
}

/// 終端機結束的說明（`{"reason":"exited","exit_code":0,"signal":null}` → 一行給人看的）。
pub fn ended_note(rec: &serde_json::Value, while_away: bool) -> String {
    let reason = rec
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("exited");
    let code = rec.get("exit_code").and_then(|v| v.as_i64());
    let what = match (reason, code) {
        ("exited", Some(c)) => format!("結束碼 {c}"),
        ("killed", _) => "被結束".to_string(),
        ("server_stop", _) => "herdr-core 停掉了".to_string(),
        ("holder_lost", _) => "herdr-core 的 holder 不見了".to_string(),
        (r, _) => r.to_string(),
    };
    if while_away {
        format!("\r\n\x1b[2m[這個會話在 App 關著的時候結束了：{what}]\x1b[0m")
    } else {
        format!("\r\n\x1b[2m[{what}]\x1b[0m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program_status::{BlockedKind, ProgramState};
    use serde_json::json;

    #[test]
    fn status_maps_to_program_report() {
        let r = report_of(&json!({"state": "working", "app": "fake", "progress": 140})).unwrap();
        assert_eq!(r.state, ProgramState::Working);
        assert_eq!(r.app.as_deref(), Some("fake"));
        assert_eq!(r.progress, Some(100));
        let r = report_of(&json!({"state": "blocked", "kind": "permission", "msg": ""})).unwrap();
        assert_eq!(
            (r.state, r.kind, r.msg),
            (ProgramState::Blocked, Some(BlockedKind::Permission), None)
        );
        assert_eq!(report_of(&json!(null)).unwrap().state, ProgramState::Clear);
        assert!(report_of(&json!({"state": "dancing"})).is_none());
    }

    #[test]
    fn meta_roundtrip_and_next_id() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(next_free_id(t.path()), 1);
        let m = StoredMeta {
            sid: "t7".into(),
            address: "t3".into(),
            kind: TermKind::Agent,
            title: "claude · x".into(),
            origin: None,
            scope: Some("x".into()),
            scope_repos: None,
            task: None,
            home_group: Some("g".into()),
            started: "2026-11-18 10:00".into(),
            cwd: Some("/tmp".into()),
            agent: Some("claude".into()),
        };
        save_meta(t.path(), &m).unwrap();
        assert_eq!(load_meta(t.path(), "t7"), Some(m));
        assert_eq!(next_free_id(t.path()), 8);
        remove_meta(t.path(), "t7");
        assert_eq!(load_meta(t.path(), "t7"), None);
    }

    #[test]
    fn spine_tag_is_short_and_stable() {
        let a = spine_tag(Path::new("/tmp/a"));
        assert_eq!(a.len(), 12);
        assert_eq!(a, spine_tag(Path::new("/tmp/a")));
        assert_ne!(a, spine_tag(Path::new("/tmp/b")));
    }
}
