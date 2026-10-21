//! 殼的共用狀態：目前的 spine、會話清冊（TermManager）、檔案監看。
//!
//! spine 可能一開始找不到（首次使用）：此時 state／PTY 命令回「尚未設定 spine」，
//! 前端先顯示首次設定畫面（`get_spine`／`set_spine`）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use ctx_core::term::TermManager;
use serde_json::{json, Value};

use crate::watch::SpineWatcher;

pub const MSG_NO_SPINE: &str = "尚未設定 spine：請先選擇 spine 目錄";

/// 引擎有變動時要做的事（發 `workbench://changed`、系統通知…）；由 lib.rs 接上 Tauri。
pub trait Notifier: Send + Sync + 'static {
    fn changed(&self, reason: &str);
    fn session_exited(&self, sid: &str, title: &str);
}

struct Inner {
    spine: Option<PathBuf>,
    terms: Option<Arc<TermManager>>,
    watcher: Option<SpineWatcher>,
}

pub struct AppState {
    inner: Mutex<Inner>,
    notifier: Arc<dyn Notifier>,
}

impl AppState {
    pub fn new(notifier: Arc<dyn Notifier>) -> Self {
        AppState {
            inner: Mutex::new(Inner {
                spine: None,
                terms: None,
                watcher: None,
            }),
            notifier,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn spine(&self) -> Option<PathBuf> {
        self.lock().spine.clone()
    }

    /// 目前 spine 的會話清冊；spine 未設定回 [`MSG_NO_SPINE`]。
    pub fn terms(&self) -> Result<Arc<TermManager>, String> {
        self.lock().terms.clone().ok_or_else(|| MSG_NO_SPINE.into())
    }

    /// 只拿已存在的清冊（spine 未設定＝沒有任何會話）。
    pub fn terms_opt(&self) -> Option<Arc<TermManager>> {
        self.lock().terms.clone()
    }

    /// 切到 `spine`：建會話清冊、開始監看。已有活著的會話時拒絕切換（會話屬於舊 spine）。
    pub fn activate(&self, spine: &Path) -> Result<(), String> {
        let mut g = self.lock();
        if g.spine.as_deref() == Some(spine) {
            return Ok(());
        }
        if let Some(t) = &g.terms {
            if t.list().iter().any(|s| s.alive) {
                return Err("還有會話在跑；先關掉所有會話再切換 spine".into());
            }
        }
        let terms = Arc::new(TermManager::new(spine));
        let n = self.notifier.clone();
        let weak = Arc::downgrade(&terms);
        terms.set_on_exit(move |sid| {
            // 還在清冊裡＝程序自己結束（不是人按關閉）→ 值得發系統通知
            if let Some(s) = weak.upgrade().and_then(|t| t.get(sid)) {
                n.session_exited(sid, s.title());
            }
            n.changed("term-exit");
        });
        let n = self.notifier.clone();
        let weak = Arc::downgrade(&terms);
        let sp = spine.to_path_buf();
        // 監看失敗不擋啟動：前端還有 5 秒輪詢當保險
        let watcher = SpineWatcher::start(spine, move |reason| {
            // F-98：會話做完一輪 → 開了自動繼續的，由引擎決定要不要替你送下一句（殼只轉寫進 PTY）。
            // 不看 reason：合併窗口裡只報第一個變動的檔，狀態檔可能排在後面；tick 只讀幾個小檔，每次都看
            if let Some(t) = weak.upgrade() {
                ctx_core::auto_continue::tick(&sp, &t);
            }
            n.changed(reason)
        })
        .ok();
        g.spine = Some(spine.to_path_buf());
        g.terms = Some(terms);
        g.watcher = watcher;
        Ok(())
    }

    pub fn notify_changed(&self, reason: &str) {
        self.notifier.changed(reason);
    }

    /// 結束前收尾：殺掉所有會話（清掉 agentmark）。
    pub fn shutdown(&self) {
        if let Some(t) = self.terms_opt() {
            t.kill_all();
        }
        self.lock().watcher = None;
    }
}

/// `get_spine` 的回傳：`{spine: string|null}`。
pub fn spine_json(spine: Option<&Path>) -> Value {
    json!({ "spine": spine.map(|p| p.to_string_lossy().into_owned()) })
}

/// 對應 `set_spine`：驗證是 spine、寫進使用者設定檔（`~/.config/repo-context/config.json` 的 `spine`，
/// 保留其他鍵），回傳解析後的絕對路徑。
pub fn persist_spine(path: &str) -> Result<PathBuf, String> {
    let p = ctx_core::fsutil::expanduser(path.trim());
    if path.trim().is_empty() || !ctx_core::config::is_spine(&p) {
        return Err(format!(
            "不是 spine 目錄（找不到 config.yaml 或 registry.yaml）：{}",
            path.trim()
        ));
    }
    let p = ctx_core::fsutil::resolve(&p);
    let mut cfg = ctx_core::config::load_user_config();
    if !cfg.is_object() {
        cfg = json!({});
    }
    cfg["spine"] = Value::String(p.to_string_lossy().into_owned());
    ctx_core::config::save_user_config(&cfg).map_err(|e| e.to_string())?;
    Ok(p)
}

/// 對應 `create_spine`：在新路徑或空資料夾建一個 spine（`ctx init` 同一份 scaffold），再照 `set_spine` 記住。
/// 已經是 spine 就直接用。有東西又不是 spine 的資料夾一律拒絕——工作台不往使用者既有的資料夾（多半是 repo）塞檔。
pub fn create_spine(path: &str) -> Result<PathBuf, String> {
    let p = ctx_core::fsutil::expanduser(path.trim());
    if path.trim().is_empty() {
        return Err("請填 spine 要建在哪裡".into());
    }
    if !ctx_core::config::is_spine(&p) {
        if p.exists() {
            let empty = std::fs::read_dir(&p)
                .map(|mut d| d.next().is_none())
                .map_err(|e| format!("讀不到這個位置：{}（{e}）", path.trim()))?;
            if !empty {
                return Err(format!(
                    "這個資料夾已經有東西、又不是 spine；為了不動到你的檔案，請換一個新路徑或空資料夾：{}",
                    path.trim()
                ));
            }
        }
        ctx_core::init::init_spine(&p).map_err(|e| e.to_string())?;
    }
    persist_spine(&p.to_string_lossy())
}

#[cfg(test)]
mod create_tests {
    use super::*;

    /// 只測不碰使用者設定檔的部分：拒絕的情況在寫任何東西之前就回錯。
    #[test]
    fn refuses_non_empty_non_spine_folder_and_leaves_it_untouched() {
        let t = tempfile::tempdir().unwrap();
        let repo = t.path().join("my-repo");
        std::fs::create_dir(&repo).unwrap();
        std::fs::write(repo.join("README.md"), "hi").unwrap();
        let e = create_spine(&repo.to_string_lossy()).unwrap_err();
        assert!(e.contains("不是 spine"), "{e}");
        let names: Vec<_> = std::fs::read_dir(&repo)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("README.md")]);
        assert!(create_spine("  ").unwrap_err().contains("請填"));
    }
}
