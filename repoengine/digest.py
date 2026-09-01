"""P5 理解 digest——agent 讀變更寫摘要（v2 §2 P5：一律增量、一律便宜模型）。

增量＝只 digest 上次 digest 事件之後的 commit（水位線就在脊椎裡：
最後一筆 `suggestion [digest] repo:<id>` 的 date+time）；沒有新 commit 就不寫事件（無事不報）。
模型走 config provider.models.digest（預設便宜模型）；agent 失敗落 system-unsure 事件，不准沉默。
"""
import datetime as _dt
import subprocess
from pathlib import Path

from . import config as _config
from . import registry, spine
from .agents import AgentUnsure, run_text

PROMPT = """你是 repo 變更摘要器。以下是 repo「{rid}」自上次摘要以來的 commit 與變更統計。
用繁體中文寫 3 行以內的摘要：改了什麼、為什麼重要、有沒有需要人注意的點。只輸出摘要本文。

{log}
"""


def _watermark(spine_dir, rid):
    """最後一筆該 repo 的 digest 事件時間（'YYYY-MM-DD HH:MM'）；沒有 → None。"""
    last = None
    for ev in spine.iter_events(spine_dir):
        if ev.type == "suggestion" and ev.source == "digest" and ev.kv("repo") == rid:
            last = f"{ev.date} {ev.time}"
    return last


def _git_log_since(repo_path, since):
    args = ["git", "-C", str(repo_path), "log",
            f"--since={since or '7.days'}",
            "--format=%h %ad %s", "--date=short", "--stat"]
    r = subprocess.run(args, capture_output=True, text=True, encoding="utf-8")
    return (r.stdout or "").strip() if r.returncode == 0 else ""


def run(spine_dir, group=None, repos=None, provider=None, when=None):
    """對組內 mine repo 逐一增量 digest。回傳 [{repo, file|None, note}]。"""
    now = when or _dt.datetime.now()
    cfg = _config.load(spine_dir)
    provider = provider or cfg["provider"]["default"]
    model = cfg["provider"]["models"].get("digest")
    gname, entries = registry.resolve_group(spine_dir, group, repos)
    outdir = Path(spine_dir) / "groups" / (group or "adhoc") / "digests"
    results = []
    for e in entries:
        if e.get("type") != "mine":
            continue
        log = _git_log_since(e["path"], _watermark(spine_dir, e["id"]))
        if not log:
            results.append({"repo": e["id"], "file": None, "note": "無新 commit"})
            continue
        tokens = [f"repo:{e['id']}"] + ([f"group:{group}"] if group else [])
        try:
            text = run_text(provider, PROMPT.format(rid=e["id"], log=log),
                            spine_dir, model=model)
        except AgentUnsure as err:
            spine.append_event(spine_dir, "suggestion", "digest", tokens,
                               body=f"system-unsure：digest 失敗：{err}", when=now)
            results.append({"repo": e["id"], "file": None, "note": "失敗（已浮出）"})
            continue
        outdir.mkdir(parents=True, exist_ok=True)
        out = outdir / f"{now:%Y-%m-%d}-{e['id']}.md"
        out.write_text(f"# digest {now:%Y-%m-%d} {e['id']}\n\n{text}\n\n"
                       f"## 依據（git log）\n```\n{log}\n```\n", encoding="utf-8")
        first = text.splitlines()[0]
        spine.append_event(spine_dir, "suggestion", "digest", tokens,
                           body=f"{first}\n→ {out.name}", when=now)
        results.append({"repo": e["id"], "file": str(out), "note": "ok"})
    return results
