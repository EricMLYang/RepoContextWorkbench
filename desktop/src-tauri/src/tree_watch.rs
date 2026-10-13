//! 檔案樹跟著磁碟變：repo 裡有檔案新增／刪除／改名／改內容 → `repo://changed`。
//!
//! 前端只告訴殼「哪些 repo 的哪些資料夾現在展開著」（`watch_tree`）；殼對那些 repo 開
//! OS 檔案事件（遞迴，FSEvents／ReadDirectoryChangesW），只回報父資料夾在展開清單裡的路徑——
//! `target/`、`node_modules/` 這類沒展開的大目錄怎麼寫都不會吵到前端。沒有樹開著＝不監看任何 repo。
//! 一波寫入合併成一次通知（[`DEBOUNCE`]）；閒置不輪詢。
//!
//! Agent 一次改很多檔（git checkout、格式化整個專案）時，OS 可能來不及逐檔回報、只說「這裡要重掃」
//! （FSEvents MustScanSubDirs、ReadDirectoryChangesW 緩衝滿）或直接回錯誤。這時不能丟掉——
//! 改成通知每個監看中的 repo「全部重讀」（`rescan`），否則樹會停在舊狀態直到手動 ⟳。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// 合併窗口（存檔常是「寫暫存檔＋改名」好幾個事件）。
pub const DEBOUNCE: Duration = Duration::from_millis(300);

/// 一個 repo：根目錄（canonical）＋展開中的資料夾（相對路徑，`""`＝根）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Watched {
    root: PathBuf,
    dirs: HashSet<PathBuf>,
}

type Table = Arc<Mutex<HashMap<String, Watched>>>;

/// 監看執行緒收到的東西。
#[derive(Debug)]
enum Msg {
    Path(PathBuf),
    /// OS 漏了事件（溢出或錯誤）：不知道哪裡變了。
    Rescan,
}

/// 一次通知：(repo id, 相對路徑們, 是否全部重讀)。全部重讀時路徑＝所有展開中的資料夾。
type Notice = (String, Vec<String>, bool);

/// 一批訊息 → 每個 repo 一則通知。
fn collect(table: &HashMap<String, Watched>, batch: &[Msg]) -> Vec<Notice> {
    if batch.iter().any(|m| matches!(m, Msg::Rescan)) {
        let mut out: Vec<Notice> = table
            .iter()
            .map(|(id, w)| {
                let mut dirs: Vec<String> = w
                    .dirs
                    .iter()
                    .map(|d| d.to_string_lossy().into_owned())
                    .collect();
                dirs.sort();
                (id.clone(), dirs, true)
            })
            .collect();
        out.sort();
        return out;
    }
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for m in batch {
        if let Msg::Path(p) = m {
            if let Some((id, rel)) = route(table, p) {
                out.entry(id)
                    .or_default()
                    .insert(rel.to_string_lossy().into_owned());
            }
        }
    }
    out.into_iter()
        .map(|(id, paths)| (id, paths.into_iter().collect(), false))
        .collect()
}

/// 事件路徑 → 要通知的（repo id, 相對路徑）。`.git` 裡的變動不算；父資料夾沒展開也不算。
fn route(table: &HashMap<String, Watched>, p: &Path) -> Option<(String, PathBuf)> {
    // repo 套在另一個 repo 裡（例：工作資料夾裡的子 repo）：算給最內層那個
    let (id, w, rel) = table
        .iter()
        .filter_map(|(id, w)| Some((id, w, p.strip_prefix(&w.root).ok()?)))
        .max_by_key(|(_, w, _)| w.root.components().count())?;
    if rel.as_os_str().is_empty()
        || rel
            .components()
            .any(|c| matches!(c, Component::Normal(n) if n == ".git"))
    {
        return None;
    }
    let parent = rel.parent().map(Path::to_path_buf).unwrap_or_default();
    w.dirs
        .contains(&parent)
        .then(|| (id.clone(), rel.to_path_buf()))
}

/// 活著就在監看；drop 即停。
pub struct TreeWatcher {
    table: Table,
    watcher: Mutex<RecommendedWatcher>,
}

impl TreeWatcher {
    /// `on_change(id, 相對路徑們, 全部重讀)`：合併後每個 repo 呼叫一次。
    pub fn start(
        on_change: impl Fn(&str, &[String], bool) + Send + 'static,
    ) -> notify::Result<TreeWatcher> {
        let table: Table = Arc::default();
        let (tx, rx) = mpsc::channel::<Msg>();
        let watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
                Ok(ev) if !ev.need_rescan() => {
                    for p in ev.paths {
                        let _ = tx.send(Msg::Path(p));
                    }
                }
                _ => {
                    let _ = tx.send(Msg::Rescan);
                }
            })?;
        let t2 = table.clone();
        std::thread::Builder::new()
            .name("tree-watch".into())
            .spawn(move || {
                while let Ok(first) = rx.recv() {
                    std::thread::sleep(DEBOUNCE);
                    let mut batch = vec![first];
                    batch.extend(rx.try_iter());
                    let notices = {
                        let table = t2.lock().unwrap_or_else(|e| e.into_inner());
                        collect(&table, &batch)
                    };
                    for (id, paths, rescan) in notices {
                        on_change(&id, &paths, rescan);
                    }
                }
            })
            .map_err(|e| notify::Error::generic(&e.to_string()))?;
        Ok(TreeWatcher {
            table,
            watcher: Mutex::new(watcher),
        })
    }

    /// 換成這一份清單：`repos`＝(id, 根目錄, 展開中的資料夾)。多的開始看、少的停掉。
    pub fn set(&self, repos: Vec<(String, PathBuf, Vec<String>)>) {
        let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
        let mut watcher = self.watcher.lock().unwrap_or_else(|e| e.into_inner());
        let next: HashMap<String, Watched> = repos
            .into_iter()
            .map(|(id, root, dirs)| {
                let root = root.canonicalize().unwrap_or(root);
                let dirs = dirs.into_iter().map(PathBuf::from).collect();
                (id, Watched { root, dirs })
            })
            .collect();
        let old_roots: HashSet<&PathBuf> = table.values().map(|w| &w.root).collect();
        let new_roots: HashSet<&PathBuf> = next.values().map(|w| &w.root).collect();
        for r in old_roots.difference(&new_roots) {
            let _ = watcher.unwatch(r);
        }
        for r in new_roots.difference(&old_roots) {
            // 看不了（資料夾不見了、權限）就算了：前端還有手動 ⟳
            let _ = watcher.watch(r, RecursiveMode::Recursive);
        }
        *table = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(dirs: &[&str]) -> HashMap<String, Watched> {
        HashMap::from([(
            "a".to_string(),
            Watched {
                root: PathBuf::from("/r/a"),
                dirs: dirs.iter().map(PathBuf::from).collect(),
            },
        )])
    }

    #[test]
    fn routes_only_expanded_parents() {
        let t = table(&["", "src"]);
        let r = |p: &str| {
            route(&t, Path::new(p)).map(|(id, rel)| (id, rel.to_string_lossy().into_owned()))
        };
        assert_eq!(r("/r/a/README.md"), Some(("a".into(), "README.md".into())));
        assert_eq!(
            r("/r/a/src/main.rs"),
            Some(("a".into(), "src/main.rs".into()))
        );
        assert_eq!(r("/r/a/target/debug/x"), None);
        assert_eq!(r("/r/a/.git/index"), None);
        assert_eq!(r("/r/a/src/.git"), None);
        assert_eq!(r("/r/a"), None);
        assert_eq!(r("/r/b/x"), None);
    }

    #[test]
    fn rescan_reloads_every_watched_dir() {
        let t = table(&["", "src"]);
        let batch = [Msg::Path(PathBuf::from("/r/a/README.md")), Msg::Rescan];
        assert_eq!(
            collect(&t, &batch),
            vec![("a".into(), vec!["".into(), "src".into()], true)]
        );
        let batch = [Msg::Path(PathBuf::from("/r/a/src/x.rs"))];
        assert_eq!(
            collect(&t, &batch),
            vec![("a".into(), vec!["src/x.rs".into()], false)]
        );
    }

    #[test]
    fn file_event_reaches_callback() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let hits = Arc::new(Mutex::new(Vec::<(String, Vec<String>)>::new()));
        let h2 = hits.clone();
        let w = TreeWatcher::start(move |id, paths, _rescan| {
            h2.lock().unwrap().push((id.to_string(), paths.to_vec()))
        })
        .unwrap();
        w.set(vec![("a".into(), root.clone(), vec!["".into()])]);
        std::thread::sleep(Duration::from_millis(200)); // FSEvents 起跑
        std::fs::create_dir_all(root.join("deep")).unwrap();
        std::fs::write(root.join("deep/x.txt"), "x").unwrap(); // deep 沒展開：不報
        std::fs::write(root.join("new.txt"), "x").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let found = || {
            hits.lock()
                .unwrap()
                .iter()
                .any(|(id, p)| id == "a" && p.iter().any(|x| x == "new.txt"))
        };
        while !found() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(found(), "應收到 new.txt：{:?}", hits.lock().unwrap());
        assert!(!hits
            .lock()
            .unwrap()
            .iter()
            .any(|(_, p)| p.iter().any(|x| x.ends_with("x.txt"))));
        // 清空＝不再監看
        w.set(Vec::new());
    }
}
