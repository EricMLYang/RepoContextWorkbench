//! 引擎寫入 → `workbench://changed`。
//!
//! 監看 spine 目錄（非遞迴：`registry.yaml`、`config.yaml`）、`spine/events/`（每日事件檔），
//! 以及 `.state/` 裡前端會顯示的兩樣：`last_seen.txt`（未讀；CLI `ack` 也會改）與
//! `agent_status/*.json`（agent 經 MCP `report_status` 回報的階段，會話清冊顯示）、
//! `session_status/*.json`（App 開的 Claude Code 會話由 hook 自動寫的「做完等你／處理中」）。
//! 用 OS 檔案事件不輪詢；一波寫入（append＋鎖檔＋rename）合併成一次通知（[`DEBOUNCE`]）。
//! 不論誰寫的——工作台自己、CLI、MCP 裡的 agent——前端都會收到。

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// 合併窗口：收到第一個相關事件後再等這麼久，把同一波寫入併成一次。
pub const DEBOUNCE: Duration = Duration::from_millis(250);

/// 這個路徑的變動要不要通知前端（只看合約上的資料檔與上述 `.state/` 兩樣；忽略鎖檔、其他快取、版控內部檔）。
pub fn is_relevant(spine: &Path, p: &Path) -> bool {
    let Ok(rel) = p.strip_prefix(spine) else {
        return false;
    };
    let mut comps = rel.components().map(|c| c.as_os_str().to_string_lossy());
    match (
        comps.next().as_deref(),
        comps.next().as_deref(),
        comps.next(),
        comps.next(),
    ) {
        (Some("registry.yaml" | "config.yaml"), None, _, _) => true,
        (Some("spine"), Some("events"), Some(name), None) => name.ends_with(".md"),
        (Some(".state"), Some("last_seen.txt"), None, _) => true,
        (Some(".state"), Some("agent_status" | "session_status"), Some(name), None) => {
            name.ends_with(".json")
        }
        _ => false,
    }
}

/// 活著就在監看；drop 即停（watcher 與合併執行緒一起收）。
pub struct SpineWatcher {
    _watcher: RecommendedWatcher,
}

impl SpineWatcher {
    /// 開始監看 `spine`；有相關變動（合併後）就呼叫 `on_change(reason)`。
    pub fn start(
        spine: &Path,
        on_change: impl Fn(&str) + Send + 'static,
    ) -> notify::Result<SpineWatcher> {
        let spine: PathBuf = spine.canonicalize().unwrap_or_else(|_| spine.to_path_buf());
        let (tx, rx) = mpsc::channel::<String>();
        let root = spine.clone();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let Ok(ev) = res else { return };
                if let Some(p) = ev.paths.iter().find(|p| is_relevant(&root, p)) {
                    let reason = p
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let _ = tx.send(reason);
                }
            })?;
        watcher.watch(&spine, RecursiveMode::NonRecursive)?;
        let events = spine.join("spine").join("events");
        if events.is_dir() {
            watcher.watch(&events, RecursiveMode::NonRecursive)?;
        }
        // agent_status/ 可能還沒有人寫過——先建好才看得到第一次回報（.state 本來就是引擎的狀態目錄）
        let status = spine.join(".state").join("agent_status");
        let _ = std::fs::create_dir_all(&status);
        // session_status/：Claude Code hook 依會話寫的「做完等你／處理中」（Rust 才有，2026-10-14）
        let session = ctx_core::session_status::dir(&spine);
        let _ = std::fs::create_dir_all(&session);
        for d in [spine.join(".state"), status, session] {
            if d.is_dir() {
                watcher.watch(&d, RecursiveMode::NonRecursive)?;
            }
        }
        // 合併執行緒：阻塞在 recv（閒置不吃 CPU）；watcher drop 後 tx 斷線 → 執行緒結束
        std::thread::Builder::new()
            .name("spine-watch".into())
            .spawn(move || {
                while let Ok(first) = rx.recv() {
                    std::thread::sleep(DEBOUNCE);
                    while rx.try_recv().is_ok() {}
                    on_change(&first);
                }
            })
            .map_err(|e| notify::Error::generic(&e.to_string()))?;
        Ok(SpineWatcher { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn relevant_paths() {
        let s = Path::new("/sp");
        assert!(is_relevant(s, Path::new("/sp/registry.yaml")));
        assert!(is_relevant(s, Path::new("/sp/config.yaml")));
        assert!(is_relevant(s, Path::new("/sp/spine/events/2026-10-06.md")));
        assert!(!is_relevant(s, Path::new("/sp/events/2026-10-06.md")));
        assert!(!is_relevant(s, Path::new("/sp/.engine.lock")));
        assert!(!is_relevant(s, Path::new("/sp/.state/x.json")));
        assert!(!is_relevant(
            s,
            Path::new("/sp/.state/knowledge_index.json")
        ));
        assert!(is_relevant(s, Path::new("/sp/.state/last_seen.txt")));
        assert!(is_relevant(s, Path::new("/sp/.state/agent_status/g1.json")));
        assert!(is_relevant(
            s,
            Path::new("/sp/.state/session_status/t1.json")
        ));
        assert!(!is_relevant(
            s,
            Path::new("/sp/.state/session_status/t1.json.tmp42")
        ));
        assert!(!is_relevant(s, Path::new("/sp/.state/session_hooks.json")));
        assert!(!is_relevant(
            s,
            Path::new("/sp/.state/agent_status/sub/g1.json")
        ));
        assert!(!is_relevant(
            s,
            Path::new("/sp/.state/agent_sessions/t1.json")
        ));
        assert!(!is_relevant(s, Path::new("/sp/spine/events/sub/x.md")));
        assert!(!is_relevant(s, Path::new("/other/registry.yaml")));
    }

    #[test]
    fn write_to_registry_fires_once_debounced() {
        let tmp = tempfile::tempdir().unwrap();
        let spine = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(spine.join("spine/events")).unwrap();
        let hits = Arc::new(Mutex::new(Vec::<String>::new()));
        let h2 = hits.clone();
        let _w =
            SpineWatcher::start(&spine, move |r| h2.lock().unwrap().push(r.to_string())).unwrap();
        std::thread::sleep(Duration::from_millis(200)); // FSEvents 起跑
        std::fs::write(spine.join("registry.yaml"), "repos: []\n").unwrap();
        std::fs::write(spine.join("spine/events/2026-10-06.md"), "x\n").unwrap();
        std::fs::write(spine.join(".engine.lock"), "x").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while hits.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!hits.lock().unwrap().is_empty(), "應收到變動通知");
    }

    /// agent 經 MCP 回報階段（寫 `.state/agent_status/<key>.json`）→ 前端要收到通知（會話清冊顯示）。
    #[test]
    fn agent_status_write_fires() {
        let tmp = tempfile::tempdir().unwrap();
        let spine = tmp.path().canonicalize().unwrap();
        let hits = Arc::new(Mutex::new(Vec::<String>::new()));
        let h2 = hits.clone();
        let _w =
            SpineWatcher::start(&spine, move |r| h2.lock().unwrap().push(r.to_string())).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(spine.join(".state/agent_status/g1.json"), "{}").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while hits.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(
            hits.lock().unwrap().first().map(String::as_str),
            Some("g1.json")
        );
    }
}
