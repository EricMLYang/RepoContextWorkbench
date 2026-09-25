"""registry.yaml 的讀寫：load、原子寫入、transaction（讀–改–寫同一把 lock）。"""
import os
from contextlib import contextmanager
from pathlib import Path

import yaml

from ..locking import FileLock
from ..spine import LOCK_NAME


def _path(spine_dir):
    return Path(spine_dir) / "registry.yaml"


def load(spine_dir):
    p = _path(spine_dir)
    if not p.exists():
        return {"repos": [], "groups": []}
    data = yaml.safe_load(p.read_text(encoding="utf-8")) or {}
    data.setdefault("repos", [])
    data.setdefault("groups", [])
    return data


def _write(spine_dir, data):
    """原子寫入：先寫暫存檔再 os.replace——讀的人（不拿 lock）永遠讀到完整的一份。"""
    p = _path(spine_dir)
    text = yaml.safe_dump(data, allow_unicode=True, sort_keys=False)
    tmp = p.with_name(f".{p.name}.{os.getpid()}.tmp")
    tmp.write_text(text, encoding="utf-8")
    os.replace(tmp, p)


def save(spine_dir, data):
    """整份覆寫（不讀舊檔）。改既有資料請用 transaction()，否則會蓋掉別人同時寫的。"""
    with FileLock(Path(spine_dir) / LOCK_NAME):
        _write(spine_dir, data)


@contextmanager
def transaction(spine_dir):
    """讀–改–寫在同一把 lock 裡（2026-09-25 架構檢查實測：只鎖寫入那一刻，
    2 進程各寫 40 筆只剩 41）。區塊裡丟例外＝不寫回。lock 不可重入：區塊內別再呼叫會寫入的函式。"""
    with FileLock(Path(spine_dir) / LOCK_NAME):
        data = load(spine_dir)
        yield data
        _write(spine_dir, data)
