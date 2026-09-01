"""內嵌 terminal——工作台的底板（2026-09-01 使用者裁定：要 IDE 風格工作台，
可在其中直接與 agent 對話；取代 v2 §7「不內嵌 terminal」的舊裁定）。

PTY 會話管理：
- POSIX 用 stdlib pty；Windows 需 pywinpty（缺了給人話錯誤，VDI 移植時再驗）
- 會話生命週期獨立於 WS 連線：工作台關掉／重連，agent 對話不死（除非人按關閉）；
  重連 replay ring buffer（保留最後 ~200KB 輸出）
- 輸出以 bytes 廣播給訂閱者（WS 端用 binary frame 直送 xterm，不經解碼——
  TUI 的 escape sequence 與半個 UTF-8 序列都不會被折斷）
"""
import collections
import itertools
import os
import shutil
import subprocess
import threading

BUFFER_CAP = 200_000
_EXIT_MSG = "\r\n\x1b[2m[進程已結束——關這個分頁或重開一個會話]\x1b[0m\r\n"


class TermSession:
    def __init__(self, sid, cmd, cwd=None, env=None, title="", cols=120, rows=32):
        self.sid = sid
        self.title = title or os.path.basename(str(cmd[0]))
        self.alive = True
        self._buf = collections.deque()   # bytes chunks
        self._buf_len = 0
        self._subs = set()                # callables(data: bytes)
        self._lock = threading.Lock()
        full_env = dict(os.environ, PYTHONUTF8="1",
                        TERM="xterm-256color", COLORTERM="truecolor")
        if env:
            full_env.update(env)
        if os.name == "nt":
            self._spawn_windows(cmd, cwd, full_env, cols, rows)
        else:
            self._spawn_posix(cmd, cwd, full_env, cols, rows)
        threading.Thread(target=self._reader, daemon=True).start()

    # ── spawn ──
    def _spawn_posix(self, cmd, cwd, env, cols, rows):
        import pty
        self._master, slave = pty.openpty()
        self._proc = subprocess.Popen(
            cmd, stdin=slave, stdout=slave, stderr=slave, cwd=cwd, env=env,
            start_new_session=True, close_fds=True)
        os.close(slave)
        self.resize(cols, rows)

    def _spawn_windows(self, cmd, cwd, env, cols, rows):
        try:
            from winpty import PtyProcess  # pywinpty（ConPTY）
        except ImportError:
            raise RuntimeError("Windows 內嵌 terminal 需要 pywinpty：pip install pywinpty")
        self._winpty = PtyProcess.spawn(
            cmd, cwd=cwd, env=env, dimensions=(rows, cols))
        self._proc = None

    # ── I/O ──
    def _read_raw(self):
        if os.name == "nt":
            return self._winpty.read(65536).encode("utf-8", "replace")
        return os.read(self._master, 65536)

    def _reader(self):
        while True:
            try:
                data = self._read_raw()
            except (OSError, EOFError):
                data = b""
            if not data:
                break
            self._push(data)
        self.alive = False
        self._push(_EXIT_MSG.encode("utf-8"))
        if os.name != "nt":
            try:
                os.close(self._master)
            except OSError:
                pass

    def _push(self, data):
        with self._lock:
            self._buf.append(data)
            self._buf_len += len(data)
            while self._buf_len > BUFFER_CAP and len(self._buf) > 1:
                self._buf_len -= len(self._buf.popleft())
            subs = list(self._subs)
        for cb in subs:
            try:
                cb(data)
            except Exception:
                self.detach(cb)

    def write(self, data: bytes):
        if not self.alive:
            return
        try:
            if os.name == "nt":
                self._winpty.write(data.decode("utf-8", "replace"))
            else:
                os.write(self._master, data)
        except OSError:
            self.alive = False

    def resize(self, cols, rows):
        try:
            if os.name == "nt":
                self._winpty.setwinsize(rows, cols)
            else:
                import fcntl
                import struct
                import termios
                fcntl.ioctl(self._master, termios.TIOCSWINSZ,
                            struct.pack("HHHH", rows, cols, 0, 0))
        except OSError:
            pass

    # ── 訂閱（WS 連線）──
    def attach(self, cb):
        """註冊訂閱者，回傳 replay 用的既有輸出。"""
        with self._lock:
            self._subs.add(cb)
            return b"".join(self._buf)

    def detach(self, cb):
        with self._lock:
            self._subs.discard(cb)

    def kill(self):
        self.alive = False
        try:
            if os.name == "nt":
                self._winpty.terminate()
            elif self._proc.poll() is None:
                self._proc.terminate()
                try:
                    self._proc.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    self._proc.kill()
        except OSError:
            pass


def default_shell():
    if os.name == "nt":
        return [os.environ.get("COMSPEC", "cmd.exe")]
    return [os.environ.get("SHELL") or shutil.which("zsh") or "/bin/sh"]


class TermManager:
    """工作台的會話清冊。agent 會話帶料（P16 session.build）、shell 會話裸開。"""

    def __init__(self, spine_dir):
        self.spine_dir = spine_dir
        self.sessions = {}
        self._ids = itertools.count(1)

    def create_shell(self, cwd=None):
        sid = f"t{next(self._ids)}"
        s = TermSession(sid, default_shell(), cwd=cwd or str(self.spine_dir),
                        title="shell")
        self.sessions[sid] = s
        return s

    def create_agent(self, group=None, repos=None, repo=None, task=None,
                     agent="claude"):
        from . import session as _session
        cwd, cmd, _pack = _session.build(self.spine_dir, group=group,
                                         repos=repos, repo=repo, task=task,
                                         agent=agent)
        sid = f"t{next(self._ids)}"
        scope = repo or group or ("臨時" if repos else "全部")
        s = TermSession(sid, cmd, cwd=str(cwd), title=f"{agent}:{scope}")
        self.sessions[sid] = s
        return s

    def get(self, sid):
        return self.sessions.get(sid)

    def kill(self, sid):
        s = self.sessions.pop(sid, None)
        if s:
            s.kill()
        return s is not None

    def list(self):
        return [{"sid": s.sid, "title": s.title, "alive": s.alive}
                for s in self.sessions.values()]

    def kill_all(self):
        for sid in list(self.sessions):
            self.kill(sid)
