//! 跨行程 file lock（P10 三件套之二；範圍含 registry.yaml）。對應 Python `repo_context/locking.py`。
//!
//! 單一 lock 檔守全 spine repo 的寫入（spine events ＋ registry.yaml），
//! O_CREAT|O_EXCL 原子建檔（檔內寫持鎖者 pid）；持鎖行程異常死亡以 stale 秒數回收。
//! 這是與 Python 版共用的合約：兩邊同時寫同一個 spine 時靠它互斥。
//! 鎖不可重入：持鎖期間別再呼叫會拿同一把鎖的函式（會等到 timeout）。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::{Error, Result};

/// 對應 `locking.FileLock`（設定；`acquire` 拿到 [`LockGuard`]）。
#[derive(Debug, Clone)]
pub struct FileLock {
    pub lock_path: PathBuf,
    pub timeout: Duration,
    pub stale: Duration,
}

/// 持鎖期間的守衛；drop＝釋放（關檔＋刪鎖檔）。
#[derive(Debug)]
pub struct LockGuard {
    path: PathBuf,
    file: Option<File>,
}

impl FileLock {
    /// 對應 `FileLock(lock_path)`：timeout 10 秒、stale 120 秒。
    pub fn new(lock_path: impl Into<PathBuf>) -> Self {
        FileLock {
            lock_path: lock_path.into(),
            timeout: Duration::from_secs(10),
            stale: Duration::from_secs(120),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_stale(mut self, stale: Duration) -> Self {
        self.stale = stale;
        self
    }

    /// 對應 `FileLock.acquire`。逾時回 `Error::LockTimeout`。
    pub fn acquire(&self) -> Result<LockGuard> {
        let deadline = Instant::now() + self.timeout;
        loop {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&self.lock_path)
            {
                Ok(mut f) => {
                    f.write_all(std::process::id().to_string().as_bytes())?;
                    return Ok(LockGuard {
                        path: self.lock_path.clone(),
                        file: Some(f),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if self.is_stale() && std::fs::remove_file(&self.lock_path).is_ok() {
                        continue; // 持鎖者死了，回收
                    }
                    if Instant::now() > deadline {
                        return Err(Error::LockTimeout(self.lock_path.display().to_string()));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn is_stale(&self) -> bool {
        // 讀不到 mtime＝對手行程剛好釋放了
        std::fs::metadata(&self.lock_path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > self.stale)
    }
}

impl LockGuard {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for LockGuard {
    /// 對應 `FileLock.release`。
    fn drop(&mut self) {
        if self.file.take().is_some() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// 便利函式：拿 `<spine>/.engine.lock` 跑 `f`。
pub fn with_spine_lock<T>(spine_dir: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let _g = FileLock::new(spine_dir.join(crate::spine::LOCK_NAME)).acquire()?;
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_then_timeout_then_released() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join(".engine.lock");
        let g = FileLock::new(&p).acquire().unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            std::process::id().to_string()
        );
        let r = FileLock::new(&p)
            .with_timeout(Duration::from_millis(120))
            .acquire();
        assert!(matches!(r, Err(Error::LockTimeout(_))));
        drop(g);
        assert!(!p.exists());
        FileLock::new(&p).acquire().unwrap();
    }

    #[test]
    fn stale_lock_is_reclaimed() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join(".engine.lock");
        std::fs::write(&p, "99999").unwrap();
        let old = SystemTime::now() - Duration::from_secs(300);
        File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let _g = FileLock::new(&p)
            .with_timeout(Duration::from_millis(200))
            .acquire()
            .unwrap();
    }
}
