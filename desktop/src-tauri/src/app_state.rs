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
        // 監看失敗不擋啟動：前端還有 5 秒輪詢當保險
        let watcher = SpineWatcher::start(spine, move |reason| n.changed(reason)).ok();
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
