"""P4 上游 upstream——pull 式查 external repo 的 release/commit（gh api），只偵測與抓取。

無事不報在這裡的落實：
- 預設只報正式 release（config thresholds.upstream.kinds／prerelease，§3.4）
- 已見過的 release/commit 記在 .state/upstream.json，同一筆不重報（洪流不進未讀）
新發現落 `suggestion [upstream] repo:<id>` 事件 → 走未讀回程，儀式時刻集中呈現。
"""
import json
import subprocess
from pathlib import Path

from . import config as _config
from . import registry, spine


def _state_path(spine_dir):
    return Path(spine_dir) / ".state" / "upstream.json"


def _load_state(spine_dir):
    p = _state_path(spine_dir)
    if p.exists():
        return json.loads(p.read_text(encoding="utf-8"))
    return {}


def _save_state(spine_dir, state):
    p = _state_path(spine_dir)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(state, ensure_ascii=False, indent=1), encoding="utf-8")


def normalize(upstream):
    """upstream 欄位 → owner/repo（容忍完整 URL 與 .git 尾綴）。"""
    u = upstream.strip().rstrip("/")
    for prefix in ("https://github.com/", "http://github.com/", "github.com/"):
        if u.startswith(prefix):
            u = u[len(prefix):]
    if u.endswith(".git"):
        u = u[:-4]
    return u


def gh_fetch(upstream, kind, prerelease=False):
    """gh api 抓最新一筆。回傳 {kind, id, title, url} 或 None（查無/失敗）。"""
    up = normalize(upstream)
    if kind == "release":
        r = subprocess.run(["gh", "api", f"repos/{up}/releases?per_page=10"],
                           capture_output=True, text=True, encoding="utf-8")
        if r.returncode != 0:
            return None
        for rel in json.loads(r.stdout or "[]"):
            if rel.get("draft"):
                continue
            if rel.get("prerelease") and not prerelease:
                continue
            return {"kind": "release", "id": rel["tag_name"],
                    "title": rel.get("name") or rel["tag_name"],
                    "url": rel.get("html_url", "")}
        return None
    if kind == "commit":
        r = subprocess.run(["gh", "api", f"repos/{up}/commits?per_page=1"],
                           capture_output=True, text=True, encoding="utf-8")
        if r.returncode != 0:
            return None
        commits = json.loads(r.stdout or "[]")
        if not commits:
            return None
        c = commits[0]
        return {"kind": "commit", "id": c["sha"][:7],
                "title": (c["commit"]["message"] or "").splitlines()[0],
                "url": c.get("html_url", "")}
    raise ValueError(f"未知 upstream kind: {kind}（release|commit）")


def check(spine_dir, group=None, repos=None, fetch=None, when=None):
    """查一輪。回傳 (external_checked 數, 新發現 list)。fetch 可注入（測試用）。"""
    fetch = fetch or gh_fetch
    cfg = _config.load(spine_dir)["thresholds"]["upstream"]
    if group or repos:
        _, entries = registry.resolve_group(spine_dir, group, repos)
    else:
        entries = registry.load(spine_dir)["repos"]
    externals = [e for e in entries
                 if e.get("type") == "external" and e.get("upstream")]
    state = _load_state(spine_dir)
    findings = []
    for e in externals:
        seen = state.setdefault(e["id"], {})
        for kind in cfg["kinds"]:
            latest = fetch(e["upstream"], kind, cfg.get("prerelease", False))
            if latest is None or seen.get(kind) == latest["id"]:
                continue
            seen[kind] = latest["id"]
            body = f"上游 {kind}：{latest['id']} {latest['title']}"
            if latest.get("url"):
                body += f"\n{latest['url']}"
            spine.append_event(spine_dir, "suggestion", "upstream",
                               [f"repo:{e['id']}"], body=body, when=when)
            findings.append({"repo": e["id"], **latest})
    _save_state(spine_dir, state)
    return len(externals), findings
