//! 自動繼續（2026-10-22，ideation F-98；Rust 才有，抄 BAT `/auto-continue`）。
//!
//! 有些工作一輪做不完：agent 做完一步就停下來等你說「繼續」。開了自動繼續，這一輪**順利做完**
//! （Claude Code `Stop`）就由 App 替你送一句（預設「繼續」），最多 N 次；你自己送出別的話就歸零重算。
//! 出錯中斷（`StopFailure`）、要你允許／回答、程序結束都不送。
//!
//! 狀態放在主機（`.state/auto_continue/<sid>.json`），不放畫面：關掉視窗、重開畫面，計數都還在。
//! 送不送由 [`take`] 決定（引擎），殼只負責把回傳的話寫進那個會話的 PTY。
//! 同一次 `Stop` 只送一次：`Stop` 的狀態列帶一個 `stop_id`（[`crate::session_status`] 寫），送過的記在這裡。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::status::AgentStatus;
use crate::{Error, Result};

/// 沒給就送這句。
pub const DEFAULT_PROMPT: &str = "繼續";
/// 沒給上限就是 3 次；最多 20 次（再多就該重新想這件事怎麼拆）。
pub const DEFAULT_MAX: u32 = 3;
pub const MAX_MAX: u32 = 20;
/// 一句最多幾個字元。
const PROMPT_MAX: usize = 500;

/// 一個會話的自動繼續設定與計數。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoContinue {
    pub prompt: String,
    pub max: u32,
    /// 這一串已經自動送了幾次（你自己送出別的話就歸零）。
    pub count: u32,
    /// 最後一次送的時候是哪一次 `Stop`（`stop_id`）；同一次不送第二遍。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_stop: Option<String>,
}

/// `.state/auto_continue`。
pub fn dir(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("auto_continue")
}

fn path_for(spine_dir: &Path, sid: &str) -> PathBuf {
    dir(spine_dir).join(format!("{sid}.json"))
}

/// 從會話狀態檔的路徑（hook 拿到的那個）找同一個會話的自動繼續檔。
fn path_from_status(status: &Path) -> Option<PathBuf> {
    let state = status.parent()?.parent()?;
    let name = status.file_name()?;
    Some(state.join("auto_continue").join(name))
}

fn valid_sid(sid: &str) -> bool {
    (1..=40).contains(&sid.len())
        && sid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn load(p: &Path) -> Option<AutoContinue> {
    let s = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

fn store(p: &Path, a: &AutoContinue) -> Result<()> {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::fsutil::write_atomic(p, serde_json::to_string(a)?.as_bytes())?;
    Ok(())
}

/// 這個會話的設定；沒開＝None。
pub fn get(spine_dir: &Path, sid: &str) -> Option<AutoContinue> {
    valid_sid(sid).then(|| load(&path_for(spine_dir, sid)))?
}

/// 開（或改）自動繼續：計數歸零。空白的話＝[`DEFAULT_PROMPT`]；上限 1～[`MAX_MAX`]。
pub fn set(
    spine_dir: &Path,
    sid: &str,
    max: Option<u32>,
    prompt: Option<&str>,
) -> Result<AutoContinue> {
    if !valid_sid(sid) {
        return Err(Error::invalid(format!("會話 id 不對：{sid}")));
    }
    let max = max.unwrap_or(DEFAULT_MAX);
    if !(1..=MAX_MAX).contains(&max) {
        return Err(Error::invalid(format!("次數要在 1～{MAX_MAX} 之間")));
    }
    let prompt = prompt
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or(DEFAULT_PROMPT);
    if prompt.chars().count() > PROMPT_MAX {
        return Err(Error::invalid(format!(
            "這句太長了（最多 {PROMPT_MAX} 字）"
        )));
    }
    // 送進終端的是一行：換行改成空格（換行在 Claude Code 的輸入框裡不會送出，會卡在那）
    let prompt = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let a = AutoContinue {
        prompt,
        max,
        count: 0,
        last_stop: None,
    };
    store(&path_for(spine_dir, sid), &a)?;
    Ok(a)
}

/// 關掉。本來就沒開＝沒事。
pub fn off(spine_dir: &Path, sid: &str) {
    if valid_sid(sid) {
        let _ = std::fs::remove_file(path_for(spine_dir, sid));
    }
}

/// App 重開：前一次的會話都不在了，全部清掉。
pub fn clear_all(spine_dir: &Path) {
    let _ = std::fs::remove_dir_all(dir(spine_dir));
}

/// 這個會話現在該不該自動送一句：設定開著、還沒到上限、狀態是**順利做完**（`Stop`）、而且這次 `Stop` 還沒送過。
/// 該送就把計數加一、記下這次 `Stop`，回傳要送的話（殼寫進 PTY）；不該送＝None。
pub fn take(spine_dir: &Path, sid: &str) -> Option<String> {
    let p = path_for(spine_dir, sid);
    let mut a = get(spine_dir, sid)?;
    if a.count >= a.max {
        return None;
    }
    let st = crate::session_status::read(spine_dir, sid)?;
    let field = |k: &str| st.get(k).and_then(Value::as_str);
    if field("status") != Some(AgentStatus::Finished.as_str())
        || field("event") != Some(crate::session_status::StatusHook::Stop.arg())
    {
        return None;
    }
    let stop = field("stop_id")?.to_string();
    if a.last_stop.as_deref() == Some(stop.as_str()) {
        return None;
    }
    a.count += 1;
    a.last_stop = Some(stop);
    store(&p, &a).ok()?;
    Some(a.prompt)
}

/// `UserPromptSubmit` hook 收到一句（`status`＝會話狀態檔路徑）：不是自動送的那句＝你自己說話了，計數歸零。
pub fn on_prompt(status: &Path, prompt: &str) {
    let Some(p) = path_from_status(status) else {
        return;
    };
    let Some(mut a) = load(&p) else {
        return;
    };
    if prompt.trim() != a.prompt && a.count > 0 {
        a.count = 0;
        let _ = store(&p, &a);
    }
}

/// 打完字隔多久再按 Enter：一次寫進「字＋Enter」，Claude Code 的輸入框可能當成貼上（換行而不送出）。
const ENTER_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

/// 殼收到會話狀態變動時呼叫：每個活著的 agent 會話看 [`take`]，該送就把話寫進它的 PTY、稍後按 Enter。
/// 回傳送了幾個會話。
pub fn tick(spine_dir: &Path, terms: &crate::term::TermManager) -> usize {
    let mut sent = 0;
    for info in terms.list() {
        if !info.alive || info.kind != crate::term::TermKind::Agent {
            continue;
        }
        let Some(text) = take(spine_dir, &info.sid) else {
            continue;
        };
        let Some(sess) = terms.get(&info.sid) else {
            continue;
        };
        sess.write(text.as_bytes());
        let _ = std::thread::Builder::new()
            .name("auto-continue".into())
            .spawn(move || {
                std::thread::sleep(ENTER_DELAY);
                sess.write(b"\r");
            });
        sent += 1;
    }
    sent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_status::{path_for as status_path, record_hook, StatusHook};

    #[test]
    fn take_sends_once_per_stop_up_to_max_and_your_own_prompt_resets() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        let st = status_path(d, "t1");
        let tgt = st.to_str();
        assert_eq!(take(d, "t1"), None, "沒開");
        let a = set(d, "t1", Some(2), Some("  照計畫\n做下一步 ")).unwrap();
        assert_eq!(a.prompt, "照計畫 做下一步");
        assert_eq!(take(d, "t1"), None, "還沒有狀態");
        record_hook(StatusHook::Stop, "{}", tgt, None);
        assert_eq!(take(d, "t1").as_deref(), Some("照計畫 做下一步"));
        assert_eq!(take(d, "t1"), None, "同一次 Stop 不送第二遍");
        // 看過了（seen）不算新的一次 Stop
        crate::session_status::mark_seen(d, "t1");
        assert_eq!(take(d, "t1"), None);
        // 自動送的那句 → 不歸零；下一次 Stop 再送
        record_hook(
            StatusHook::Prompt,
            r#"{"prompt":"照計畫 做下一步"}"#,
            tgt,
            None,
        );
        record_hook(StatusHook::Stop, "{}", tgt, None);
        assert!(take(d, "t1").is_some());
        record_hook(StatusHook::Stop, "{}", tgt, None);
        assert_eq!(take(d, "t1"), None, "到上限");
        assert_eq!(get(d, "t1").unwrap().count, 2);
        // 你自己說了別的話 → 歸零
        record_hook(StatusHook::Prompt, r#"{"prompt":"先停一下"}"#, tgt, None);
        assert_eq!(get(d, "t1").unwrap().count, 0);
        // 出錯中斷、要你允許：不送
        record_hook(
            StatusHook::StopFailure,
            r#"{"error":"rate_limit"}"#,
            tgt,
            None,
        );
        assert_eq!(take(d, "t1"), None);
        record_hook(StatusHook::Permission, "{}", tgt, None);
        assert_eq!(take(d, "t1"), None);
        off(d, "t1");
        record_hook(StatusHook::Stop, "{}", tgt, None);
        assert_eq!(take(d, "t1"), None, "關了");
    }

    #[test]
    fn set_validates() {
        let t = tempfile::tempdir().unwrap();
        assert!(set(t.path(), "../x", None, None).is_err());
        assert!(set(t.path(), "t1", Some(0), None).is_err());
        assert!(set(t.path(), "t1", Some(MAX_MAX + 1), None).is_err());
        assert!(set(t.path(), "t1", None, Some(&"字".repeat(501))).is_err());
        let a = set(t.path(), "t1", None, Some("  ")).unwrap();
        assert_eq!((a.prompt.as_str(), a.max), (DEFAULT_PROMPT, DEFAULT_MAX));
    }
}
