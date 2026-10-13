//! Repo Context 工作台的 Tauri v2 殼。
//!
//! 殼只做：視窗、IPC（`desktop/IPC.md`）、PTY 轉接、選單列、通知、單一實例、開機自啟。
//! 業務邏輯一律在 `ctx-core`（AGENTS.md：引擎 crate 不依賴 Tauri）。
//!
//! 生命週期：關視窗＝隱藏（會話照跑，從選單列或 Dock 叫回來）；選單列「結束」或 ⌘Q 才收掉所有會話。

pub mod app_state;
pub mod env_path;
pub mod terminal;
pub mod tree_watch;
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
/// 檔案樹展開中的資料夾有變動（`{id, paths, rescan}`，見 `tree_watch`）。
pub const EVENT_REPO_CHANGED: &str = "repo://changed";
const MAIN: &str = "main";
const TRAY_ID: &str = "main";
/// 選單列「記一筆…」→ 前端（desktop/IPC.md 事件表）。
const EVENT_CAPTURE: &str = "ui://capture";
/// 選單列「等你的會話」→ 前端切到第一個在等你的會話。
const EVENT_NEXT_WAITING: &str = "ui://next-waiting";

struct TauriNotifier {
    app: AppHandle,
}

impl Notifier for TauriNotifier {
    fn changed(&self, reason: &str) {
        let _ = self
            .app
            .emit(EVENT_CHANGED, serde_json::json!({ "reason": reason }));
        // 視窗藏起來時前端不輪詢——選單列的未讀數由殼自己更新
        refresh_unread(&self.app);
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

/// 選單列「未讀：N」那一行（set_tray 更新它的文字）。
struct UnreadItem(MenuItem<tauri::Wry>);
/// 選單列「等你的會話：N」那一行。
struct WaitingItem(MenuItem<tauri::Wry>);
/// 已經發過系統通知的「要你允許」：sid → 請求時間（同一次請求只通知一次）。
#[derive(Default)]
struct NotifiedAsks(std::sync::Mutex<std::collections::HashMap<String, String>>);

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "開啟工作台", true, None::<&str>)?;
    let capture = MenuItem::with_id(app, "capture", "記一筆…", true, None::<&str>)?;
    let unread = MenuItem::with_id(app, "unread", "未讀：—", false, None::<&str>)?;
    app.manage(UnreadItem(unread.clone()));
    // 有會話在等你時可以點：叫出視窗、切到那個會話（set_tray 依數量開關）
    let waiting = MenuItem::with_id(app, "waiting", "等你的會話：—", false, None::<&str>)?;
    app.manage(WaitingItem(waiting.clone()));
    let quit = MenuItem::with_id(app, "quit", "結束", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &capture,
            &unread,
            &waiting,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Repo Context 工作台")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, ev| match ev.id.as_ref() {
            "open" => show_main(app),
            // 隨手記一筆到目前牌組的筆記（F-37 第一步）：叫出視窗，前端切到筆記分頁、游標放進輸入框
            "waiting" => {
                show_main(app);
                let _ = app.emit(EVENT_NEXT_WAITING, ());
            }
            "capture" => {
                show_main(app);
                let _ = app.emit(EVENT_CAPTURE, ());
            }
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

/// 選單列標題：未讀數，加上在等你的會話數（餘光看得到哪個 agent 卡在你這裡）。都是 0＝不顯示。
fn tray_title(unread: Option<usize>, waiting: Option<usize>) -> Option<String> {
    match (unread.unwrap_or(0), waiting.unwrap_or(0)) {
        (0, 0) => None,
        (u, 0) => Some(u.to_string()),
        (0, w) => Some(format!("等你 {w}")),
        (u, w) => Some(format!("{u} · 等你 {w}")),
    }
}

/// 選單列：標題（[`tray_title`]）＋選單「未讀：N」「等你的會話：N」。`None`＝不知道（spine 未設定）。
pub fn set_tray(app: &AppHandle, unread: Option<usize>, waiting: Option<usize>) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_title(tray_title(unread, waiting));
    }
    let line = |label: &str, n: Option<usize>| match n {
        Some(n) => format!("{label}：{n}"),
        None => format!("{label}：—"),
    };
    if let Some(item) = app.try_state::<UnreadItem>() {
        let _ = item.0.set_text(line("未讀", unread));
    }
    if let Some(item) = app.try_state::<WaitingItem>() {
        let n = waiting.unwrap_or(0);
        let text = if n > 0 {
            format!("等你的會話：{n}（點了切過去）")
        } else {
            line("等你的會話", waiting)
        };
        let _ = item.0.set_text(text);
        let _ = item.0.set_enabled(n > 0);
    }
}

/// 重算未讀數（`notify.pending` 的 count——跟 `get_state` 的 `unread` 同一份）與在等你的會話數，
/// 更新選單列。背景執行緒跑。
fn refresh_unread(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let st = app.try_state::<AppState>();
        let n = st
            .as_ref()
            .and_then(|s| s.spine())
            .and_then(|sp| ctx_core::notify::pending(&sp).ok())
            .map(|p| p.count);
        let waiting = st.as_ref().and_then(|s| workbench::needs_you_now(s));
        set_tray(&app, n, waiting);
        // agent 停在「要你允許工具」：打斷是掙來的——視窗不在前面就發系統通知（同一次請求只發一次）
        let asks = st
            .as_ref()
            .map(|s| workbench::permission_asks_now(s))
            .unwrap_or_default();
        let fresh = match app.try_state::<NotifiedAsks>() {
            Some(n) => match n.0.lock() {
                Ok(mut seen) => workbench::fresh_permission_asks(&mut seen, &asks),
                Err(_) => Vec::new(),
            },
            None => Vec::new(),
        };
        let focused = app
            .get_webview_window(MAIN)
            .and_then(|w| w.is_focused().ok())
            .unwrap_or(false);
        if !focused {
            for a in fresh {
                let _ = app
                    .notification()
                    .builder()
                    .title(format!(
                        "{} 要你允許",
                        a["title"].as_str().unwrap_or("會話")
                    ))
                    .body(a["text"].as_str().unwrap_or(""))
                    .show();
            }
        }
    });
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

/// 首次設定「建立新的 spine」：只在新路徑或空資料夾建（見 `app_state::create_spine`）。
#[tauri::command]
fn create_spine(state: State<'_, AppState>, path: String) -> Result<Value, String> {
    let p = app_state::create_spine(&path)?;
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
async fn get_state(
    app: AppHandle,
    group: Option<String>,
    repos: Option<Vec<String>>,
) -> Result<Value, String> {
    let handle = app.clone();
    let st = blocking(app, move |st| workbench::get_state(st, group, repos)).await?;
    set_tray(
        &handle,
        Some(workbench::unread_count(&st)),
        Some(workbench::needs_you_count(&st)),
    );
    Ok(st)
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

/// 原生「選擇資料夾」（加入資料夾／掃描資料夾用）。取消＝None。只回路徑，不讀也不改任何檔。
#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_title("選擇 repo 資料夾")
        .blocking_pick_folder()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

/// 檔案樹要跟著磁碟變：前端給「每個 repo 展開中的資料夾」（`{id: [相對路徑…]}`，`""`＝根），
/// 殼只監看這些 repo、只報這些資料夾裡的變動。空物件＝全部停掉。查不到的 repo 略過。
#[tauri::command]
async fn watch_tree(
    app: AppHandle,
    repos: std::collections::HashMap<String, Vec<String>>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(tw) = app.try_state::<tree_watch::TreeWatcher>() else {
            return;
        };
        let spine = app.state::<AppState>().spine();
        let list = repos
            .into_iter()
            .filter_map(|(id, dirs)| {
                let root = ctx_core::registry::get_repo(spine.as_deref()?, &id)
                    .ok()?
                    .path();
                Some((id, root, dirs))
            })
            .collect();
        tw.set(list);
    })
    .await
    .map_err(|e| e.to_string())
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
        // 只給 pick_folder 命令在 Rust 端用；前端不開 dialog 權限
        .plugin(tauri_plugin_dialog::init())
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
            let h2 = handle.clone();
            // 監看起不來不擋啟動：檔案樹還有手動 ⟳
            if let Ok(tw) = tree_watch::TreeWatcher::start(move |id, paths, rescan| {
                let _ = h2.emit(
                    EVENT_REPO_CHANGED,
                    serde_json::json!({ "id": id, "paths": paths, "rescan": rescan }),
                );
            }) {
                app.manage(tw);
            }
            build_tray(&handle)?;
            app.manage(NotifiedAsks::default());
            refresh_unread(&handle);
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
            create_spine,
            get_state,
            get_scan,
            run_action,
            pick_folder,
            watch_tree,
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

    #[test]
    fn tray_title_shows_unread_and_waiting() {
        assert_eq!(tray_title(None, None), None);
        assert_eq!(tray_title(Some(0), Some(0)), None);
        assert_eq!(tray_title(Some(3), Some(0)).as_deref(), Some("3"));
        assert_eq!(tray_title(Some(0), Some(2)).as_deref(), Some("等你 2"));
        assert_eq!(tray_title(Some(3), Some(2)).as_deref(), Some("3 · 等你 2"));
    }
}
