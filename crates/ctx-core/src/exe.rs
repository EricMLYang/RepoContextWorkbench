//! 找 `ctx` 執行檔、起脫離呼叫端的背景進程。
//!
//! Python 版一律 `sys.executable -m repo_context …`；Rust 版的「引擎入口」是 `ctx` 這支 binary，
//! 但呼叫端不一定是它（桌面 App 的執行檔叫 `repo-context-desktop`，測試是 cargo 的 test binary），
//! 所以 MCP 指令、hooks、背景判定／反芻都經 [`ctx_exe`] 找同一支 `ctx`，規則只寫在這裡。

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::Result;

fn ctx_file_name() -> String {
    format!("ctx{}", std::env::consts::EXE_SUFFIX)
}

/// 要給 MCP／hooks／背景進程跑的 `ctx` 執行檔。依序：
/// 1. 自己就叫 `ctx` → 自己；
/// 2. 與自己同目錄的 `ctx`（例：.app 包裡、`target/debug/` 底下）；
/// 3. PATH 上第一個可執行的 `ctx`（注意：搬遷期間可能是 Python 版的 `ctx`——兩邊 CLI 合約相同）；
/// 4. 都沒有 → 自己（`std::env::current_exe()`）。
pub fn ctx_exe() -> Result<PathBuf> {
    let me = std::env::current_exe()?;
    Ok(ctx_exe_from(&me, std::env::var_os("PATH").as_deref()))
}

/// [`ctx_exe`] 的純函數版：自己的路徑與 PATH 由呼叫端給（測試用）。
pub fn ctx_exe_from(me: &Path, path_var: Option<&OsStr>) -> PathBuf {
    if is_ctx(me) {
        return me.to_path_buf();
    }
    let name = ctx_file_name();
    let sibling = me.with_file_name(&name);
    if sibling.is_file() {
        return sibling;
    }
    which_in("ctx", path_var).unwrap_or_else(|| me.to_path_buf())
}

/// 對應 `shutil.which`：PATH 上找得到這個可執行檔嗎（Windows 另試 .exe／.cmd／.bat）。
pub fn which(name: &str) -> Option<PathBuf> {
    which_in(name, std::env::var_os("PATH").as_deref())
}

/// [`which`] 的純函數版：PATH 由呼叫端給。
pub fn which_in(name: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    for dir in std::env::split_paths(path_var?) {
        for ext in exts {
            let cand = dir.join(format!("{name}{ext}"));
            if is_executable(&cand) {
                return Some(cand);
            }
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// 這個路徑的檔名（去掉副檔名）是不是 `ctx`。
pub fn is_ctx(p: &Path) -> bool {
    p.file_stem().is_some_and(|s| s == "ctx")
}

/// 環境變數 `REPOCTX_NO_BACKGROUND` 有值（非空）＝不起背景進程（測試／對拍用）。
pub fn background_disabled() -> bool {
    std::env::var_os("REPOCTX_NO_BACKGROUND").is_some_and(|v| !v.is_empty())
}

/// 起一個脫離呼叫端的進程（Unix：新 session｜Windows：DETACHED_PROCESS＋新進程組），stdio 全接 null。
/// 同 Python `subprocess.Popen(..., start_new_session=True)`：hook／App 結束也不會被一起收掉。
/// 不檢查 [`background_disabled`]（呼叫端決定）。
pub fn spawn_detached<I, S>(exe: &Path, args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut cmd);
    cmd.spawn()?;
    Ok(())
}

#[cfg(unix)]
fn detach(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid 是 async-signal-safe，fork 後、exec 前只呼叫它（同 Python start_new_session=True）。
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(windows)]
fn detach(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn detach(_cmd: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().expect("有上層")).expect("建目錄");
        std::fs::write(p, b"").expect("寫檔");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
    }

    #[test]
    fn self_named_ctx_wins() {
        let t = tempfile::tempdir().expect("tempdir");
        let me = t.path().join("a").join(ctx_file_name());
        touch(&me);
        touch(&t.path().join("p").join(ctx_file_name()));
        let pv = std::env::join_paths([t.path().join("p")]).expect("PATH");
        assert_eq!(ctx_exe_from(&me, Some(&pv)), me);
    }

    #[test]
    fn sibling_then_path_then_self() {
        let t = tempfile::tempdir().expect("tempdir");
        let me = t.path().join("a").join("repo-context-desktop");
        touch(&me);
        let on_path = t.path().join("p").join(ctx_file_name());
        touch(&on_path);
        let pv = std::env::join_paths([t.path().join("empty"), t.path().join("p")]).expect("PATH");
        // 沒有同目錄 ctx → PATH 上的
        assert_eq!(ctx_exe_from(&me, Some(&pv)), on_path);
        // 都沒有 → 自己
        assert_eq!(ctx_exe_from(&me, None), me);
        // 有同目錄 ctx → 它優先於 PATH
        let sib = t.path().join("a").join(ctx_file_name());
        touch(&sib);
        assert_eq!(ctx_exe_from(&me, Some(&pv)), sib);
    }
}
