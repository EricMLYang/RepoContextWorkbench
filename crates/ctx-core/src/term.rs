//! 內嵌 terminal——工作台的底板。對應 `repo_context/term.py`。
//!
//! 2026-09-01 使用者裁定：要 IDE 風格工作台，可在其中直接與 agent 對話（取代 v2 §7「不內嵌 terminal」）。
//!
//! PTY 會話管理（Rust 版用 `portable-pty`：macOS／Linux 走 openpty、Windows 走 ConPTY，一套程式）：
//! - 會話生命週期獨立於連線：工作台視窗關掉／重連，agent 對話不死（除非人按關閉）；
//!   重連 replay ring buffer（保留最後 ~200KB 輸出）。
//! - 輸出以 bytes 廣播給訂閱者（不經解碼——TUI 的 escape sequence 與半個 UTF-8 序列都不會被折斷）。
//! - replay 在 session 鎖內交給新訂閱者，之後的即時輸出才會進來：同一個訂閱者看到的順序不亂。
//!
//! 與 Python 版的分工差異：agent 會話的指令（`session.build` 打包＋prompt）由呼叫端組好再交進來
//! （[`AgentLaunch`]），term 只管 PTY、登入 shell 包裝與 agentmark 標記——不依賴 session 模組。

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;

use crate::{agentmark, Error, Result};

/// ring buffer 上限（位元組）。對應 `term.BUFFER_CAP`。
pub const BUFFER_CAP: usize = 200_000;
/// 程序結束時附在輸出尾巴的提示。對應 `term._EXIT_MSG`。
pub const EXIT_MSG: &str = "\r\n\x1b[2m[進程已結束——關這個分頁或重開一個會話]\x1b[0m\r\n";

/// 預設終端尺寸（與 Python `TermSession(cols=120, rows=32)` 同）。
pub const DEFAULT_COLS: u16 = 120;
pub const DEFAULT_ROWS: u16 = 32;

/// 訂閱者收到的事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermEvent<'a> {
    /// PTY 輸出（attach 當下的第一筆是 replay，可能是空的）。
    Data(&'a [u8]),
    /// 程序結束（[`EXIT_MSG`] 已經以 `Data` 送過）。之後不會再有事件。
    Exited,
}

/// 訂閱者：回傳 `false` 表示送不出去（連線斷了），session 會把它拆掉——對應 Python cb 丟例外就 detach。
pub type Subscriber = Box<dyn FnMut(TermEvent<'_>) -> bool + Send>;

/// attach 的識別碼（`term_close` 用它只拆掉那一條連線）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AttachId(pub u64);

impl std::fmt::Display for AttachId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a{}", self.0)
    }
}

impl std::str::FromStr for AttachId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        s.strip_prefix('a')
            .and_then(|n| n.parse().ok())
            .map(AttachId)
            .ok_or_else(|| Error::invalid(format!("attach id 不合法: {s}")))
    }
}

/// 會話種類（序列化字串與 Python `kind` 相同）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TermKind {
    Shell,
    Agent,
}

/// 要在 PTY 裡跑的指令（已組好的 argv＋cwd＋額外環境變數）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

impl SpawnSpec {
    pub fn new(argv: Vec<String>) -> Self {
        SpawnSpec {
            argv,
            cwd: None,
            env: Vec::new(),
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
        }
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }
}

/// 會話的描述欄位（Python 版是建好 TermSession 後再掛上的屬性）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermMeta {
    pub kind: TermKind,
    pub title: String,
    /// 由哪張卡／哪次碰撞開的（卡 key 或 `collision:<cid>`）。
    pub origin: Option<String>,
    pub scope: Option<String>,
    pub scope_repos: Option<Vec<String>>,
    pub task: Option<String>,
}

impl TermMeta {
    pub fn shell(title: impl Into<String>) -> Self {
        TermMeta {
            kind: TermKind::Shell,
            title: title.into(),
            origin: None,
            scope: None,
            scope_repos: None,
            task: None,
        }
    }
}

/// `TermManager.list()` 的一筆（欄位名與 Python 相同）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TermInfo {
    pub sid: String,
    pub title: String,
    pub alive: bool,
    pub kind: TermKind,
    pub origin: Option<String>,
    pub scope: Option<String>,
    pub scope_repos: Option<Vec<String>>,
    pub started: String,
    pub task: Option<String>,
}

struct Shared {
    buf: VecDeque<Vec<u8>>,
    buf_len: usize,
    subs: Vec<(AttachId, Subscriber)>,
    next_attach: u64,
}

type ExitHook = Box<dyn FnOnce() + Send>;

/// 一個 PTY 會話。對應 `term.TermSession`。
pub struct TermSession {
    sid: String,
    meta: TermMeta,
    started: String,
    pid: Option<u32>,
    alive: AtomicBool,
    shared: Mutex<Shared>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    /// 子程序已被回收（waiter 執行緒設）。kill 等它決定要不要補 SIGKILL。
    reaped: Arc<(Mutex<bool>, Condvar)>,
}

fn joined(buf: &VecDeque<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::with_capacity(buf.iter().map(Vec::len).sum());
    for chunk in buf {
        out.extend_from_slice(chunk);
    }
    out
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // 訂閱者 panic 不該讓整個會話卡死：毒化的鎖照樣拿來用
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl TermSession {
    /// 對應 `TermSession.__init__`：開 PTY、spawn、起 reader 執行緒。`on_exit` 在程序自然結束後呼叫。
    pub fn spawn(
        sid: impl Into<String>,
        spec: &SpawnSpec,
        meta: TermMeta,
        on_exit: Option<ExitHook>,
    ) -> Result<Arc<TermSession>> {
        let sid = sid.into();
        let Some(prog) = spec.argv.first() else {
            return Err(Error::invalid("指令是空的"));
        };
        let pty = portable_pty::native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: spec.rows,
                cols: spec.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::External(format!("開 PTY 失敗：{e}")))?;
        let mut cmd = CommandBuilder::new(prog);
        cmd.args(&spec.argv[1..]);
        if let Some(cwd) = &spec.cwd {
            cmd.cwd(cwd);
        }
        cmd.env("PYTHONUTF8", "1");
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| Error::External(format!("啟動 {prog} 失敗：{e}")))?;
        // slave 端交給子程序後自己這份要關，否則子程序結束時 master 讀不到 EOF
        drop(pair.slave);
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| Error::External(format!("PTY 讀取端：{e}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| Error::External(format!("PTY 寫入端：{e}")))?;
        let pid = child.process_id();
        let killer = child.clone_killer();
        let title = if meta.title.is_empty() {
            Path::new(prog)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| prog.clone())
        } else {
            meta.title.clone()
        };
        let reaped = Arc::new((Mutex::new(false), Condvar::new()));
        let sess = Arc::new(TermSession {
            sid,
            meta: TermMeta { title, ..meta },
            // 開始時間＝之後判斷「這個會話期間有沒有留痕」的起點（2026-09-12 檢討 §5）
            started: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
            pid,
            alive: AtomicBool::new(true),
            shared: Mutex::new(Shared {
                buf: VecDeque::new(),
                buf_len: 0,
                subs: Vec::new(),
                next_attach: 1,
            }),
            writer: Mutex::new(Some(writer)),
            master: Mutex::new(pair.master),
            killer: Mutex::new(killer),
            reaped: reaped.clone(),
        });
        // waiter：回收子程序（不留 zombie），並讓 kill 知道它死了沒
        std::thread::Builder::new()
            .name(format!("term-wait-{}", sess.sid))
            .spawn(move || {
                let _ = child.wait();
                let (m, cv) = &*reaped;
                *lock(m) = true;
                cv.notify_all();
            })?;
        let s2 = sess.clone();
        std::thread::Builder::new()
            .name(format!("term-read-{}", sess.sid))
            .spawn(move || s2.reader_loop(reader, on_exit))?;
        Ok(sess)
    }

    fn reader_loop(&self, mut reader: Box<dyn Read + Send>, on_exit: Option<ExitHook>) {
        let mut buf = vec![0u8; 65536];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => self.push(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break, // macOS／Linux：slave 全關後讀 master 是 EIO
            }
        }
        {
            // 結束三件事在同一把鎖裡做完：之後 attach 的人一定拿得到含結束提示的 replay
            let mut sh = lock(&self.shared);
            self.alive.store(false, Ordering::SeqCst);
            Self::push_locked(&mut sh, EXIT_MSG.as_bytes());
            for (_, cb) in sh.subs.iter_mut() {
                cb(TermEvent::Exited);
            }
            sh.subs.clear();
        }
        *lock(&self.writer) = None;
        if let Some(f) = on_exit {
            f();
        }
    }

    /// 對應 `TermSession._push`：進 ring buffer（超過上限丟最舊、至少留一塊）並廣播。
    fn push(&self, data: &[u8]) {
        Self::push_locked(&mut lock(&self.shared), data);
    }

    fn push_locked(sh: &mut Shared, data: &[u8]) {
        sh.buf.push_back(data.to_vec());
        sh.buf_len += data.len();
        while sh.buf_len > BUFFER_CAP && sh.buf.len() > 1 {
            if let Some(old) = sh.buf.pop_front() {
                sh.buf_len -= old.len();
            }
        }
        // 在鎖內送：保證 attach 時的 replay 與之後的即時輸出順序一致
        sh.subs.retain_mut(|(_, cb)| cb(TermEvent::Data(data)));
    }

    pub fn sid(&self) -> &str {
        &self.sid
    }

    pub fn title(&self) -> &str {
        &self.meta.title
    }

    pub fn meta(&self) -> &TermMeta {
        &self.meta
    }

    pub fn started(&self) -> &str {
        &self.started
    }

    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// 對應 `TermSession.pid`。
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// 對應 `TermSession.write`：程序已結束就靜默丟掉。
    pub fn write(&self, data: &[u8]) {
        if !self.alive() {
            return;
        }
        let mut w = lock(&self.writer);
        if let Some(wr) = w.as_mut() {
            if wr.write_all(data).and_then(|_| wr.flush()).is_err() {
                self.alive.store(false, Ordering::SeqCst);
            }
        }
    }

    /// 對應 `TermSession.resize`：失敗不報錯（Python 吞 OSError）。
    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = lock(&self.master).resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    /// 目前 ring buffer 的內容（replay）。
    pub fn replay(&self) -> Vec<u8> {
        joined(&lock(&self.shared).buf)
    }

    /// 對應 `TermSession.attach`：註冊訂閱者，**先**以 `Data(replay)` 呼叫它一次（在鎖內），之後才收即時輸出。
    pub fn attach(&self, mut cb: Subscriber) -> AttachId {
        let mut sh = lock(&self.shared);
        let id = AttachId(sh.next_attach);
        sh.next_attach += 1;
        let replay = joined(&sh.buf);
        if cb(TermEvent::Data(&replay)) && self.alive() {
            sh.subs.push((id, cb));
        }
        id
    }

    /// 對應 `TermSession.detach`。回傳是否真的拆掉了一個。
    pub fn detach(&self, id: AttachId) -> bool {
        let mut sh = lock(&self.shared);
        let before = sh.subs.len();
        sh.subs.retain(|(a, _)| *a != id);
        sh.subs.len() != before
    }

    /// 目前掛著幾條連線。
    pub fn attach_count(&self) -> usize {
        lock(&self.shared).subs.len()
    }

    /// 對應 `TermSession.kill`：先客氣（SIGHUP＋SIGTERM），3 秒內沒走就 SIGKILL。Windows 走 ConPTY 的 terminate。
    pub fn kill(&self) {
        self.alive.store(false, Ordering::SeqCst);
        if self.is_reaped() {
            return;
        }
        #[cfg(unix)]
        if let Some(pid) = self.pid.and_then(|p| libc::pid_t::try_from(p).ok()) {
            // 互動式 shell 會忽略 SIGTERM（Python 版因此常要等滿 3 秒）；先送 SIGHUP——
            // 等同「關掉終端機」，shell 與 claude 都會收尾離開。
            // SAFETY: 對自己 spawn 的子程序送訊號；pid 由 waiter 回收前不會被重用。
            unsafe {
                libc::kill(pid, libc::SIGHUP);
                libc::kill(pid, libc::SIGTERM);
            }
            if self.wait_reaped(Duration::from_secs(3)) {
                return;
            }
        }
        let _ = lock(&self.killer).kill();
    }

    fn is_reaped(&self) -> bool {
        *lock(&self.reaped.0)
    }

    /// 等子程序被回收，最多 `timeout`。回傳是否已回收。
    pub fn wait_reaped(&self, timeout: Duration) -> bool {
        let (m, cv) = &*self.reaped;
        let g = lock(m);
        let (g, _) = cv
            .wait_timeout_while(g, timeout, |done| !*done)
            .unwrap_or_else(|e| e.into_inner());
        *g
    }

    /// `list()` 的一筆。
    pub fn info(&self) -> TermInfo {
        TermInfo {
            sid: self.sid.clone(),
            title: self.meta.title.clone(),
            alive: self.alive(),
            kind: self.meta.kind,
            origin: self.meta.origin.clone(),
            scope: self.meta.scope.clone(),
            scope_repos: self.meta.scope_repos.clone(),
            started: self.started.clone(),
            task: self.meta.task.clone(),
        }
    }
}

/// 對應 `term.agent_title`：會話標題＝任務摘要（2026-09-02 檢討 F5：會話要有主，開三個分得出誰是誰）。
/// 沒任務才退回 `agent · scope`。`limit` 以字元計。
pub fn agent_title(agent: &str, scope: &str, task: Option<&str>, limit: usize) -> String {
    if let Some(task) = task.filter(|t| !t.is_empty()) {
        let trimmed = task.trim();
        let head = crate::pystr::splitlines(trimmed)
            .first()
            .copied()
            .unwrap_or("");
        let t = format!("{agent} · {head}");
        if t.chars().count() <= limit {
            return t;
        }
        let cut: String = t.chars().take(limit.saturating_sub(1)).collect();
        return cut + "…";
    }
    format!("{agent} · {scope}")
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// 對應 `term.default_shell`。
pub fn default_shell() -> Vec<String> {
    if cfg!(windows) {
        return vec![std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())];
    }
    let sh = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| which("zsh").map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "/bin/sh".into());
    vec![sh]
}

/// Python `shlex.quote`。
pub fn shlex_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    let safe = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }
}

/// 經登入 shell 跑：PATH／profile（~/.local/bin、npm-global…）天然正確，不受「工作台是怎麼被啟動的」影響；
/// command not found 也會顯示在終端裡。Windows 原樣回傳。對應 `TermManager.create_agent` 裡的包裝。
pub fn login_shell_wrap(argv: &[String]) -> Vec<String> {
    if cfg!(windows) {
        return argv.to_vec();
    }
    let sh = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/zsh".into());
    let joined: Vec<String> = argv.iter().map(|a| shlex_quote(a)).collect();
    vec![sh, "-lc".into(), format!("exec {}", joined.join(" "))]
}

/// 開 agent 會話所需的一切（指令已由呼叫端用 `session::build` 組好）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentLaunch {
    /// agent 名（config `agents` 的鍵，例 `claude`）。
    pub agent: String,
    /// 已組好的 agent 指令（尚未包登入 shell）。
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub group: Option<String>,
    pub repos: Option<Vec<String>>,
    pub repo: Option<String>,
    pub task: Option<String>,
    pub origin: Option<String>,
}

type ExitListener = Arc<dyn Fn(&str) + Send + Sync>;

/// 工作台的會話清冊。對應 `term.TermManager`。agent 會話帶料、shell 會話裸開。
pub struct TermManager {
    spine_dir: PathBuf,
    sessions: Mutex<Vec<Arc<TermSession>>>,
    ids: AtomicU64,
    on_exit: Mutex<Option<ExitListener>>,
}

impl TermManager {
    pub fn new(spine_dir: impl Into<PathBuf>) -> Self {
        TermManager {
            spine_dir: spine_dir.into(),
            sessions: Mutex::new(Vec::new()),
            ids: AtomicU64::new(1),
            on_exit: Mutex::new(None),
        }
    }

    pub fn spine_dir(&self) -> &Path {
        &self.spine_dir
    }

    /// 任一會話程序結束時呼叫（參數是 sid）——桌面殼拿來發 `workbench://changed`。
    pub fn set_on_exit(&self, f: impl Fn(&str) + Send + Sync + 'static) {
        *lock(&self.on_exit) = Some(Arc::new(f));
    }

    fn next_sid(&self) -> String {
        format!("t{}", self.ids.fetch_add(1, Ordering::SeqCst))
    }

    fn exit_hook(&self, sid: &str, unmark_agent: bool) -> ExitHook {
        let listener = lock(&self.on_exit).clone();
        let spine = self.spine_dir.clone();
        let sid = sid.to_string();
        Box::new(move || {
            if unmark_agent {
                agentmark::unmark(&spine, &sid);
            }
            if let Some(l) = listener {
                l(&sid);
            }
        })
    }

    /// 對應 `TermManager.create_shell`：沒給 cwd 就開在 spine。
    pub fn create_shell(
        &self,
        cwd: Option<&Path>,
        origin: Option<String>,
    ) -> Result<Arc<TermSession>> {
        let sid = self.next_sid();
        let title = if cwd.is_some() {
            "shell"
        } else {
            "shell · spine"
        };
        let spec = SpawnSpec::new(default_shell()).cwd(cwd.unwrap_or(&self.spine_dir));
        let meta = TermMeta {
            origin,
            ..TermMeta::shell(title)
        };
        let s = TermSession::spawn(&sid, &spec, meta, Some(self.exit_hook(&sid, false)))?;
        lock(&self.sessions).push(s.clone());
        Ok(s)
    }

    /// 對應 `TermManager.create_agent`（指令組裝那段在呼叫端）：登入 shell 包裝→PTY spawn→agentmark 標記。
    pub fn create_agent(&self, launch: AgentLaunch) -> Result<Arc<TermSession>> {
        let sid = self.next_sid();
        let scope = launch
            .repo
            .clone()
            .or_else(|| launch.group.clone())
            .unwrap_or_else(|| {
                if launch.repos.is_some() {
                    "臨時".into()
                } else {
                    "全部".into()
                }
            });
        let agent = if launch.agent.is_empty() {
            "claude".to_string()
        } else {
            launch.agent.clone()
        };
        let spec = SpawnSpec {
            argv: login_shell_wrap(&launch.argv),
            cwd: Some(launch.cwd.clone()),
            env: launch.env.clone(),
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
        };
        let meta = TermMeta {
            kind: TermKind::Agent,
            title: agent_title(&agent, &scope, launch.task.as_deref(), 40),
            origin: launch.origin.clone(),
            scope: Some(scope),
            scope_repos: launch.repos.clone(),
            task: launch.task.clone(),
        };
        let s = TermSession::spawn(&sid, &spec, meta, Some(self.exit_hook(&sid, true)))?;
        // agent 偵測（gitpane 課）：標記存活會話，P3 採集據此標「🤖 誰在跑」
        let pid = s.pid().map(i64::from).unwrap_or(0);
        agentmark::mark(&self.spine_dir, &sid, &launch.cwd, &agent, pid)?;
        lock(&self.sessions).push(s.clone());
        Ok(s)
    }

    /// 對應 `TermManager.get`。
    pub fn get(&self, sid: &str) -> Option<Arc<TermSession>> {
        lock(&self.sessions).iter().find(|s| s.sid == sid).cloned()
    }

    /// 對應 `TermManager.kill`：移出清冊並殺程序；未知 sid 回 `false`。
    pub fn kill(&self, sid: &str) -> bool {
        let s = {
            let mut v = lock(&self.sessions);
            let Some(i) = v.iter().position(|s| s.sid == sid) else {
                return false;
            };
            v.remove(i)
        };
        s.kill();
        agentmark::unmark(&self.spine_dir, sid); // reader 收尾可能晚到，這裡先清
        true
    }

    /// 對應 `TermManager.list`（建立順序）。
    pub fn list(&self) -> Vec<TermInfo> {
        lock(&self.sessions).iter().map(|s| s.info()).collect()
    }

    /// 對應 `TermManager.kill_all`。
    pub fn kill_all(&self) {
        let sids: Vec<String> = lock(&self.sessions).iter().map(|s| s.sid.clone()).collect();
        for sid in sids {
            self.kill(&sid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_title_uses_task_head_and_truncates() {
        assert_eq!(agent_title("claude", "g1", None, 40), "claude · g1");
        assert_eq!(
            agent_title("claude", "g1", Some("  修 bug\n細節"), 40),
            "claude · 修 bug"
        );
        let long = "字".repeat(60);
        let t = agent_title("claude", "g1", Some(&long), 40);
        assert_eq!(t.chars().count(), 40);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn shlex_quote_matches_python() {
        assert_eq!(shlex_quote(""), "''");
        assert_eq!(shlex_quote("abc-1.2/x"), "abc-1.2/x");
        assert_eq!(shlex_quote("a b"), "'a b'");
        assert_eq!(shlex_quote("it's"), "'it'\"'\"'s'");
        assert_eq!(shlex_quote("中文"), "'中文'");
    }

    #[test]
    fn attach_id_roundtrip() {
        let id: AttachId = "a7".parse().unwrap();
        assert_eq!(id, AttachId(7));
        assert_eq!(id.to_string(), "a7");
        assert!("x".parse::<AttachId>().is_err());
    }
}
