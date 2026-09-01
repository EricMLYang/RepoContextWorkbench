"""P9 生 spawn——兩段式升格＋出生登記（含血統）。

段一＝素材落 incubator（P8 route dest=incubator，已有）；
段二＝本模組：incubator 素材升格成獨立 git repo → registry 出生登記（origin 血統）
→ 脊椎 `decision [spawn]` 事件；origin 是 collision 時另落 `outcome` 回連
（碰撞→產出的反向追溯，靈感命中率的分子，v2 §3.2）。
新 repo 自己的 git init/commit 不受「spine 單一提交者」約束——那條守的是 spine repo 的 git。
"""
import datetime as _dt
import os
import shutil
import subprocess
from pathlib import Path

from . import registry, spine


def _git(cwd, *args):
    env = dict(os.environ)
    for k in ("GIT_AUTHOR_NAME", "GIT_COMMITTER_NAME"):
        env.setdefault(k, "repoengine")
    for k in ("GIT_AUTHOR_EMAIL", "GIT_COMMITTER_EMAIL"):
        env.setdefault(k, "repoengine@local")
    subprocess.run(["git", "-C", str(cwd)] + list(args),
                   check=True, capture_output=True, env=env)


def spawn(spine_dir, new_id, target_path, from_incubator=None, origin=None, when=None):
    """升格：建 repo、搬素材、登記血統、落事件。回傳新 repo 路徑。"""
    now = when or _dt.datetime.now()
    target = Path(target_path).expanduser()
    if target.exists() and any(target.iterdir()):
        raise ValueError(f"目標已存在且非空: {target}")
    src = None
    if from_incubator:
        src = Path(spine_dir) / "incubator" / from_incubator
        if not src.is_file():
            raise ValueError(f"incubator 素材不存在: {from_incubator}")
    target.mkdir(parents=True, exist_ok=True)
    lineage = origin or (f"incubator/{from_incubator}" if from_incubator else "manual")
    (target / "README.md").write_text(
        f"# {new_id}\n\n出生：{now:%Y-%m-%d}｜血統：{lineage}\n", encoding="utf-8")
    if src is not None:
        shutil.move(str(src), str(target / src.name))  # 搬離 incubator＝升格完成
    subprocess.run(["git", "init", "-q"], cwd=target, check=True, capture_output=True)
    _git(target, "add", "-A")
    _git(target, "commit", "-q", "-m", f"birth: {lineage}")
    registry.add_repo(spine_dir, new_id, target, origin=origin)
    spine.append_event(spine_dir, "decision", "spawn", [f"repo:{new_id}"],
                       body=f"出生登記：{new_id} ← {lineage} → {target}", when=now)
    if origin and origin.startswith("collision:"):
        spine.append_event(spine_dir, "outcome", "spawn",
                           [f"ref:{origin}", f"repo:{new_id}"],
                           body=f"長成了 repo:{new_id}", when=now)
    return target
