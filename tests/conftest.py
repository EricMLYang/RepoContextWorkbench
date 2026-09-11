"""共用 fixture：temp spine repo ＋ temp 假 git repo 群 ＋ CLI subprocess runner。"""
import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from repo_context import registry  # noqa: E402
from repo_context.cli import cmd_init  # noqa: E402


def _run_git(cwd, *args, env=None):
    e = dict(os.environ)
    e.update({"GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t",
              "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"})
    if env:
        e.update(env)
    subprocess.run(["git", "-C", str(cwd)] + list(args),
                   check=True, capture_output=True, env=e)


def make_git_repo(base, name, days_old=0, dirty_files=0, notes=True):
    """建一個假 git repo：README＋筆記，可指定最後 commit 幾天前、幾個 dirty 檔。"""
    import datetime as dt
    p = Path(base) / name
    p.mkdir(parents=True)
    subprocess.run(["git", "init", "-q"], cwd=p, check=True, capture_output=True)
    (p / "README.md").write_text(f"# {name}\n這個 repo 談 widget 快取設計。\n",
                                 encoding="utf-8")
    if notes:
        (p / "notes").mkdir()
        (p / "notes" / "plan.md").write_text(
            "## 計劃\n已知結論：快取要用 LRU。\n", encoding="utf-8")
    when = (dt.datetime.now() - dt.timedelta(days=days_old)).strftime(
        "%Y-%m-%dT%H:%M:%S")
    _run_git(p, "add", "-A")
    _run_git(p, "commit", "-q", "-m", "init",
             env={"GIT_AUTHOR_DATE": when, "GIT_COMMITTER_DATE": when})
    for i in range(dirty_files):
        (p / f"wip{i}.md").write_text("未提交草稿\n", encoding="utf-8")
    return p


@pytest.fixture
def spine(tmp_path):
    """已 init 的 spine repo 路徑。"""
    d = tmp_path / "spine-repo"

    class A:
        path = str(d)
    cmd_init(A())
    return d


@pytest.fixture
def spine_with_repos(spine, tmp_path):
    """spine ＋ 兩個 mine repo（一乾淨、一 dirty 5 天）＋ 一個組 g1。"""
    ra = make_git_repo(tmp_path, "repo-a", days_old=1)
    rb = make_git_repo(tmp_path, "repo-b", days_old=6, dirty_files=2)
    # 讓 dirty 檔看起來已放了 5 天
    old = __import__("time").time() - 5 * 86400
    for f in rb.glob("wip*.md"):
        os.utime(f, (old, old))
    registry.add_repo(spine, "repo-a", ra)
    registry.add_repo(spine, "repo-b", rb)
    registry.add_group(spine, "g1", ["repo-a", "repo-b"])
    return spine


def run_cli(*args, spine_dir=None, env=None, check=False):
    """subprocess 跑真 CLI（L2 用），PYTHONUTF8=1。"""
    e = dict(os.environ, PYTHONUTF8="1", PYTHONIOENCODING="utf-8",
             GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t",
             GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t")
    if env:
        e.update(env)
    cmd = [sys.executable, "-m", "repo_context"]
    if spine_dir:
        cmd += ["--spine", str(spine_dir)]
    cmd += list(args)
    r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8",
                       cwd=ROOT, env=e)
    if check and r.returncode != 0:
        raise AssertionError(f"CLI 失敗 rc={r.returncode}\ncmd={args}\n"
                             f"stdout={r.stdout}\nstderr={r.stderr}")
    return r
