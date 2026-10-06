//! Repo Context 工作台的 Tauri v2 殼。
//!
//! 殼只做：視窗、IPC（`desktop/IPC.md`）、PTY 轉接、選單列、通知、單一實例、開機自啟。
//! 業務邏輯一律在 `ctx-core`（AGENTS.md：引擎 crate 不依賴 Tauri）。
//!
//! 生命週期：關視窗＝隱藏（會話照跑，從選單列或 Dock 叫回來）；選單列「結束」或 ⌘Q 才收掉所有會話。

pub mod app_state;
pub mod env_path;
pub mod terminal;
pub mod watch;
pub mod workbench;

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};
use tauri_plugin_notification::NotificationExt;

use app_state::{persist_spine, spine_json, AppState, Notifier};

/// 前端監聽的事件名（IPC.md「事件」）。
pub const EVENT_CHANGED: &str = "workbench://changed";
const MAIN: &str = "main";
const TRAY_ID: &str = "main";

struct TauriNotifier {
    app: AppHandle,
}

impl Notifier for TauriNotifier {
    fn changed(&self, reason: &str) {
        let _ = self
            .app
            .emit(EVENT_CHANGED, serde_json::json!({ "reason": reason }));
    }

    fn session_exited(&self, _sid: &str, title: &str) {
        let _ = self
            .app
            .notification()
            .builder()
            .title("會話結束")
            .body(title)
            .show();
    }
}

/// `--spine <path>`／`--spine=<path>`（最高優先；其次 `REPOENGINE_SPINE`、使用者設定檔）。
pub fn spine_arg(args: &[String]) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--spine" {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix("--spine=") {
            return Some(v.to_string());
        }
    }
    None
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "開啟工作台", true, None::<&str>)?;
    // TODO(第三波)：get_state 接上後，以 unread.len() 更新這一行（set_unread）與 tray 標題
    let unread = MenuItem::with_id(app, "unread", "未讀：—", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "結束", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open, &unread, &PredefinedMenuItem::separator(app)?, &quit],
    )?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Repo Context 工作台")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, ev| match ev.id.as_ref() {
            "open" => show_main(app),
            "quit" => {
                app.state::<AppState>().shutdown();
                app.exit(0);
            }
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// 選單列的未讀數（placeholder：第三波接 `get_state` 後呼叫）。
pub fn set_unread(app: &AppHandle, n: Option<usize>) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_title(n.filter(|n| *n > 0).map(|n| n.to_string()));
    }
}

// ── IPC 命令（desktop/IPC.md）──

#[tauri::command]
fn get_spine(state: State<'_, AppState>) -> Value {
    spine_json(state.spine().as_deref())
}

#[tauri::command]
fn set_spine(state: State<'_, AppState>, path: String) -> Result<Value, String> {
    let p = persist_spine(&path)?;
    state.activate(&p)?;
    state.notify_changed("spine");
    Ok(spine_json(Some(&p)))
}

async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&AppState) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || f(&app.state::<AppState>()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_state(app: AppHandle) -> Result<Value, String> {
    blocking(app, workbench::get_state).await
}

#[tauri::command]
async fn get_scan(
    app: AppHandle,
    group: Option<String>,
    repos: Option<Vec<String>>,
) -> Result<Value, String> {
    blocking(app, move |st| workbench::get_scan(st, group, repos)).await
}

#[tauri::command]
async fn run_action(app: AppHandle, name: String, payload: Value) -> Result<Value, String> {
    blocking(app, move |st| workbench::run_action(st, &name, &payload)).await
}

pub fn run() {
    env_path::fix_path();
    let args: Vec<String> = std::env::args().collect();
    let explicit = spine_arg(&args);

    let app = tauri::Builder::default()
        // 單一實例要第一個註冊：第二次開＝把既有視窗叫到前面
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app)
        }))
        .plugin(tauri_plugin_notification::init())
        // 開機自啟：預設關（plugin 只提供開關，沒人呼叫 enable 就不會註冊 LaunchAgent）
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(move |app| {
            let handle = app.handle().clone();
            let state = AppState::new(Arc::new(TauriNotifier {
                app: handle.clone(),
            }));
            if let Some(sp) = ctx_core::config::find_spine(explicit.as_deref()) {
                let _ = state.activate(&sp);
            }
            app.manage(state);
            build_tray(&handle)?;
            Ok(())
        })
        .on_window_event(|window, ev| {
            // 關視窗＝隱藏：會話生命週期獨立於視窗
            if let WindowEvent::CloseRequested { api, .. } = ev {
                if window.label() == MAIN {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_spine,
            set_spine,
            get_state,
            get_scan,
            run_action,
            terminal::term_open,
            terminal::term_write,
            terminal::term_resize,
            terminal::term_close,
        ])
        .build(tauri::generate_context!())
        .expect("建立 Tauri app 失敗");

    app.run(|app, ev| match ev {
        RunEvent::ExitRequested { .. } => app.state::<AppState>().shutdown(),
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => show_main(app),
        _ => {}
    });
}

/// 給測試：spine 路徑解析（`--spine` ＞ `REPOENGINE_SPINE` ＞ 使用者設定檔）。
pub fn resolve_spine(args: &[String]) -> Option<PathBuf> {
    ctx_core::config::find_spine(spine_arg(args).as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spine_arg_forms() {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            spine_arg(&v(&["app", "--spine", "/s"])).as_deref(),
            Some("/s")
        );
        assert_eq!(spine_arg(&v(&["app", "--spine=/t"])).as_deref(), Some("/t"));
        assert_eq!(spine_arg(&v(&["app"])), None);
        assert_eq!(spine_arg(&v(&["app", "--spine"])), None);
    }
}
