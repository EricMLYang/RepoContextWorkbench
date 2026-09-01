"""跨行程 file lock（P10 三件套之二；範圍含 registry.yaml）。

單一 lock 檔守全 spine repo 的寫入（spine events ＋ registry.yaml），
O_CREAT|O_EXCL 原子建檔；持鎖行程異常死亡以 stale 秒數回收。
"""
import os
import time


class LockTimeout(Exception):
    pass


class FileLock:
    def __init__(self, lock_path, timeout=10.0, stale=120.0):
        self.lock_path = str(lock_path)
        self.timeout = timeout
        self.stale = stale
        self._fd = None

    def acquire(self):
        deadline = time.monotonic() + self.timeout
        while True:
            try:
                self._fd = os.open(self.lock_path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
                os.write(self._fd, str(os.getpid()).encode("ascii"))
                return self
            except FileExistsError:
                try:
                    if time.time() - os.path.getmtime(self.lock_path) > self.stale:
                        os.unlink(self.lock_path)  # 持鎖者死了，回收
                        continue
                except OSError:
                    pass  # 對手行程剛好釋放了
                if time.monotonic() > deadline:
                    raise LockTimeout(f"lock timeout: {self.lock_path}")
                time.sleep(0.05)

    def release(self):
        if self._fd is not None:
            os.close(self._fd)
            self._fd = None
            try:
                os.unlink(self.lock_path)
            except OSError:
                pass

    def __enter__(self):
        return self.acquire()

    def __exit__(self, *exc):
        self.release()
