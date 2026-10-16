//! F-86（2026-10-16，抄 Orca 的 awake service）：有 agent 會話在跑就不讓機器睡。
//!
//! macOS 用系統內建的 `caffeinate -i`（擋閒置睡眠，螢幕照樣可以暗）：引擎 `get_state` 的
//! `attention.working` > 0 就開一個、歸零就收掉——不用自己算 agent 狀態，殼只看引擎給的數字。
//! Windows／Linux 還沒做（`SetThreadExecutionState`／`systemd-inhibit`），`sync` 什麼都不做。
//! 使用者層設定 `keep_awake: false`（`~/.config/repo-context/config.json`）可以關掉。
//! App 結束一定收掉（`RunEvent::ExitRequested`），不留孤兒程序。

use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

/// 預設的擋睡眠指令（macOS）。
const DEFAULT_COMMAND: &[&str] = &["caffeinate", "-i"];

/// 擋睡眠的子程序；`None`＝現在沒在擋。
pub struct KeepAwake {
    child: Mutex<Option<Child>>,
    command: Vec<String>,
    /// 這個平台有沒有做（沒做＝`sync` 永遠不開）。
    supported: bool,
}

impl Default for KeepAwake {
    fn default() -> Self {
        Self::with_command(
            DEFAULT_COMMAND.iter().map(|s| s.to_string()).collect(),
            cfg!(target_os = "macos"),
        )
    }
}

impl KeepAwake {
    /// 自訂指令（測試用假指令）；`supported`＝這個平台要不要真的開。
    pub fn with_command(command: Vec<String>, supported: bool) -> Self {
        KeepAwake {
            child: Mutex::new(None),
            command,
            supported,
        }
    }

    /// 使用者層設定 `keep_awake`（預設 true）。
    pub fn enabled_by_user() -> bool {
        ctx_core::config::load_user_config()
            .get("keep_awake")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true)
    }

    /// 要擋就確保有一個活著的子程序，不要擋就收掉。回傳現在有沒有在擋。
    pub fn sync(&self, want: bool) -> bool {
        let mut slot = self.child.lock().unwrap_or_else(|e| e.into_inner());
        // 子程序自己死了（被殺、找不到指令）：當作沒在擋
        if let Some(c) = slot.as_mut() {
            if matches!(c.try_wait(), Ok(Some(_)) | Err(_)) {
                *slot = None;
            }
        }
        match (want && self.supported, slot.is_some()) {
            (true, false) => {
                let Some((bin, args)) = self.command.split_first() else {
                    return false;
                };
                *slot = Command::new(bin)
                    .args(args)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .ok();
                slot.is_some()
            }
            (false, true) => {
                Self::kill(slot.take());
                false
            }
            (want, _) => want && slot.is_some(),
        }
    }

    /// App 結束：收掉。
    pub fn stop(&self) {
        let mut slot = self.child.lock().unwrap_or_else(|e| e.into_inner());
        Self::kill(slot.take());
    }

    fn kill(child: Option<Child>) {
        if let Some(mut c) = child {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// 現在有沒有在擋（測試用）。
    pub fn active(&self) -> bool {
        self.child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
            .is_some_and(|c| matches!(c.try_wait(), Ok(None)))
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake() -> KeepAwake {
        KeepAwake::with_command(vec!["sleep".into(), "60".into()], true)
    }

    #[test]
    fn starts_once_keeps_while_wanted_and_stops_when_not() {
        let k = fake();
        assert!(!k.active());
        assert!(k.sync(true));
        assert!(k.active());
        let pid = k.child.lock().unwrap().as_ref().map(|c| c.id());
        // 再要一次：不重開
        assert!(k.sync(true));
        assert_eq!(k.child.lock().unwrap().as_ref().map(|c| c.id()), pid);
        // 不要了：收掉
        assert!(!k.sync(false));
        assert!(!k.active());
        assert!(!k.sync(false));
    }

    #[test]
    fn unsupported_platform_or_missing_binary_never_blocks() {
        let k = KeepAwake::with_command(vec!["sleep".into(), "60".into()], false);
        assert!(!k.sync(true));
        assert!(!k.active());
        let k = KeepAwake::with_command(vec!["definitely-not-a-command-xyz".into()], true);
        assert!(!k.sync(true));
        assert!(!k.active());
        k.stop();
    }
}
