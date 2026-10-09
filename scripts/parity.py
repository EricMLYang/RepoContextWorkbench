"""Python ↔ Rust 對拍（scripts/parity.sh 呼叫）。

同一份 fixture spine（init、3 個假 git repo、兩組、關係、export、事件／todo／handoff）複製兩份：
一份給 Python `python3 -m repo_context`，一份給 Rust `target/debug/ctx`。對每個指令兩邊各跑一次，比較：
  - exit code
  - stdout：`--json` 的解析後比（浮點數容許小誤差）；人話逐字比（路徑、時間先換成佔位符）
  - 寫出的檔：spine 事件檔、dead-letter、registry.yaml（解析成 YAML 物件比）、.state 底下的 JSON、
    其他 spine 檔（正規化後逐字比）
MCP 也對拍：同一串 JSON-RPC 餵兩邊 stdin，逐則比回應。

允許差異（ALLOW）：Python 的 bug 而 Rust 已修、或刻意改變的行為——列在表上並註明理由，
比較前把那一欄位從兩邊拿掉（不是整個指令跳過）。
"""
import copy
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
CTX = Path(os.environ.get("CTX_BIN") or ROOT / "target" / "debug" / "ctx")
PY = [sys.executable, "-m", "repo_context"]

# ── 允許差異 ───────────────────────────────────────────────────────────
# key＝指令名稱，value＝(理由, 把差異欄位拿掉的函式（作用在兩邊已解析的 stdout）)


def _drop_dirty_days(obj):
    if isinstance(obj, list):
        for s in obj:
            if isinstance(s, dict):
                s.pop("dirty_days", None)
    return obj


ALLOW = {
    "collect --json": (
        "collect dirty_days：Python 版用最舊 dirty 檔 mtime 卻只看前幾個檔（實測會漏掉較舊的檔），"
        "Rust 已修成看全部 dirty 檔——數值可能不同，其餘欄位照比",
        _drop_dirty_days),
}

# 只在檔案比對時允許的差異：(路徑 regex, 理由)
ALLOW_FILES = [
    (r"^\.mcp\.json$",
     "session 掛的 MCP 指令改指 Rust ctx（`ctx mcp`），不再是 `python -m repo_context mcp`"),
    (r"^\.state/relation_suggest\.json$",
     "相關 repo 建議快取（F-33，Rust 才有：反芻時順便算，開場讀）"),
    (r"^\.state/review\.json$",
     "未結體檢結果快取（F-21／F-46，Rust 才有：開場與 scan 算好給桌面 App 收件匣讀）"),
    (r"^\.agent_logs/",
     "provider 原始紀錄（除錯用 log，含 prompt 全文與呼叫序號；MockProvider 計數改成實例層）"),
]

# MCP 工具上 Rust 才有的選填參數：(工具名, 參數名, 理由)。比對 tools/list 前從 Rust 那邊拿掉，記為允許差異。
MCP_EXTRA_PARAMS = [
    ("log_decision", "supersedes", "log_decision 多了選填 supersedes／reopen_when（F-07 判斷取代＋何時重看，Rust 才有）"),
    ("log_decision", "reopen_when", "log_decision 多了選填 supersedes／reopen_when（F-07 判斷取代＋何時重看，Rust 才有）"),
    ("log_decision", "outcome_of", "log_decision 多了選填 outcome_of（F-23 判斷回顧：記結果，Rust 才有）"),
]


def drop_mcp_extras(resp, allowed):
    """tools/list 回應：拿掉 Rust 才有的選填參數（就地改），用到的理由加進 allowed。"""
    for t in ((resp.get("result") or {}).get("tools") or []):
        props = (t.get("inputSchema") or {}).get("properties") or {}
        for name, param, why in MCP_EXTRA_PARAMS:
            if t.get("name") == name and param in props:
                del props[param]
                allowed.add(why)


SETUP_EXE_REASON = ("agentsetup 的 MCP／hooks 指令改指 Rust ctx（`<ctx> hook …`／`-- <ctx> mcp`），"
                    "Python 是 `<python> -m repo_context …`——指令字串不同，其餘照比")


def setup_fix(s):
    """setup 輸出裡的 MCP／hook 指令字串：Python 是 `<python> -m repo_context …`，Rust 是 `<ctx> …`。"""
    s = re.sub(r"(註冊 MCP server：|手動跑：|claude mcp add)[^\n]*", r"\1<MCP 指令>", s)
    s = re.sub(r"(：)[^\n]*hook session-start[^\n]*", r"\1<hook 指令>", s)
    return s


# ── fixture ─────────────────────────────────────────────────────────────

def git(cwd, *args, env=None):
    e = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t",
             GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t")
    e.update(env or {})
    subprocess.run(["git", "-C", str(cwd), *args], check=True, capture_output=True, env=e)


def make_repo(base, name, days_old, dirty=0, extra=None):
    p = base / name
    p.mkdir(parents=True)
    git(p, "init", "-q")
    (p / "README.md").write_text(f"# {name}\n這個 repo 談 widget 快取設計。\n", encoding="utf-8")
    (p / "notes").mkdir()
    (p / "notes" / "plan.md").write_text("## 計劃\n已知結論：快取要用 LRU。\n", encoding="utf-8")
    for rel, text in (extra or {}).items():
        f = p / rel
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_text(text, encoding="utf-8")
    when = time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime(time.time() - days_old * 86400))
    git(p, "add", "-A")
    git(p, "commit", "-q", "-m", "init", env={"GIT_AUTHOR_DATE": when, "GIT_COMMITTER_DATE": when})
    old = time.time() - 5 * 86400
    for i in range(dirty):
        f = p / f"wip{i}.md"
        f.write_text("未提交草稿\n", encoding="utf-8")
        os.utime(f, (old, old))
    return p


def py_env(home):
    e = dict(os.environ)
    for k in ("REPOENGINE_SPINE", "CTX_CWD", "PYTHONUTF8"):
        e.pop(k, None)
    e.update(HOME=str(home), XDG_CONFIG_HOME=str(home / ".config"),
             CLAUDE_CONFIG_DIR=str(home / ".claude"), REPOCTX_NO_BACKGROUND="1",
             PYTHONPATH=str(ROOT), PYTHONIOENCODING="utf-8",
             GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t",
             GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t")
    return e


def build_fixture(fx):
    """用 Python CLI 建 fixture（Python 是規格）。"""
    repos = fx / "repos"
    impl = make_repo(repos, "impl", 1, extra={
        "AGENTS.md": "# impl\n規格在 ../notes/notes/plan.md，舊版 ../notes/gone/spec.md。\n",
        "src/app.py": "def cache_lru():\n    return 'widget 快取'\n"})
    make_repo(repos, "notes", 3, dirty=1, extra={"書摘/ch01.md": "# 第一章\n快取與 LRU 的取捨。\n"})
    make_repo(repos, "ext", 40)
    sp = fx / "spine"
    home = fx / "home"
    home.mkdir()
    env = py_env(home)

    def run(*args, cwd=None):
        r = subprocess.run([*PY, "--spine", str(sp), *args], cwd=cwd or ROOT, env=env,
                           capture_output=True, text=True, encoding="utf-8")
        if r.returncode != 0:
            raise SystemExit(f"fixture 失敗：{args}\n{r.stdout}\n{r.stderr}")
        return r

    subprocess.run([*PY, "init", str(sp)], cwd=ROOT, env=env, check=True, capture_output=True)
    run("registry", "add", "impl", str(impl), "--tags", "app,core")
    run("registry", "add", "notes", str(repos / "notes"))
    run("registry", "add", "ext", str(repos / "ext"), "--type", "external", "--tier", "dormant")
    run("group", "add", "產品", "impl")
    run("group", "add", "研究", "notes,ext")
    run("registry", "relate", "notes", "impl", "--kind", "feeds", "--note", "書摘餵實作")
    run("registry", "export", "notes", "書摘", "書摘", "--note", "讀書筆記")
    run("append", "--type", "decision", "--source", "manual", "--kv", "group:產品",
        "--body", "快取用 LRU\n依據 notes/plan.md")
    run("todo", "add", "補壓測", "--cwd", str(impl))
    run("todo", "add", "整理書摘", "--cwd", str(repos / "notes"), "--due", "2020-01-01")
    run("handoff", "--done", "加了水位線", "--next", "跑壓測", "--cwd", str(impl))
    run("log", "索引改增量重建", "--cwd", str(impl))
    run("collide", "submit", "已知的快取想法", "--group", "產品", "--no-run")
    # backfill 彙整結果（review／accept 用）：照 backfill.consolidate 寫出的形狀
    bdir = sp / ".state" / "backfill" / "repos"
    bdir.mkdir(parents=True)
    (bdir / "impl.json").write_text(json.dumps({
        "repo": "impl", "sessions": ["s1aaaaaa"], "consolidated_at": "2026-09-11T10:00:00",
        "accepted": [],
        "decisions": [{"text": "索引改增量重建", "basis": "全量太慢", "sid": "s1aaaaaa",
                       "date": "2026-09-10", "id": "d1"},
                      {"text": "快取用 LRU", "basis": "", "sid": "s1aaaaaa", "date": "2026-09-10",
                       "id": "d2"}],
        "open_todos": [{"text": "補壓測", "sid": "s1aaaaaa", "date": "2026-09-10",
                        "uncertain": False, "id": "t1"}],
        "handoff": {"done": "加水位線", "next": "跑壓測", "sid": "s1aaaaaa", "date": "2026-09-10",
                    "id": "h"},
        "goal_hints": [{"text": "讓搜尋更快", "sid": "s1aaaaaa"}]},
        ensure_ascii=False, indent=1), encoding="utf-8")
    # Claude Code 會話（backfill scan 用）
    proj = home / ".claude" / "projects" / "-x-impl"
    proj.mkdir(parents=True)
    rows = [{"type": "ai-title", "aiTitle": "索引重建"},
            {"type": "user", "cwd": str(impl), "timestamp": "2026-09-10T10:00:00Z",
             "message": {"role": "user", "content": "把索引改成增量"}},
            {"type": "assistant", "cwd": str(impl), "timestamp": "2026-09-10T10:00:00Z",
             "message": {"role": "assistant", "content": [{"type": "text", "text": "好"}]}}]
    f = proj / "s1aaaaaa.jsonl"
    f.write_text("\n".join(json.dumps(r, ensure_ascii=False) for r in rows), encoding="utf-8")
    t = time.time() - 5 * 3600
    os.utime(f, (t, t))
    return sp


def clone(fx, dst):
    shutil.copytree(fx, dst, symlinks=True, copy_function=shutil.copy2)
    # 路徑改指新的根（registry.yaml、.state 底下的 JSON、會話紀錄裡的 cwd）
    for p in dst.rglob("*"):
        if ".git" in p.parts or not p.is_file():
            continue
        try:
            t = p.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        if str(fx) in t:
            st = p.stat()
            p.write_text(t.replace(str(fx), str(dst)), encoding="utf-8")
            os.utime(p, (st.st_atime, st.st_mtime))
    # Claude Code 專案目錄名是 cwd 編碼，不影響（scan 讀檔內 cwd）


# ── 正規化與比較 ────────────────────────────────────────────────────────

DT_RE = re.compile(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}(:\d{2}(\.\d+)?)?([+-]\d{2}:\d{2}|Z)?")
HM_RE = re.compile(r"(?<![\d:])\d{2}:\d{2}(?![\d:])")
PACK_RE = re.compile(r"(pack|brief|digest)-\d{8}-\d{6}")
STAMP_RE = re.compile(r"(?<!\d)\d{8}-\d{6}(?!\d)")
DL_RE = re.compile(r"\d{4}-\d{2}-\d{2}-\d{6}-\d+")  # dead-letter 檔名（含微秒）


def norm_text(s, side):
    root = str(side["root"])
    real = os.path.realpath(root)
    for a, b in ((real, "<R>"), (root, "<R>"), (str(side["exe"]), "<CTX>")):
        s = s.replace(a, b)
    s = s.replace(sys.executable + " -m repo_context", "<CTX>")
    s = DL_RE.sub("<DL>", s)
    s = DT_RE.sub("<DT>", s)
    s = HM_RE.sub("<HM>", s)
    s = PACK_RE.sub(r"\1-<STAMP>", s)
    s = STAMP_RE.sub("<STAMP>", s)
    return s


def deep_eq(a, b, path="$"):
    """回傳第一個差異的描述（None＝相同）。浮點數容許誤差（時間算出來的天數）。"""
    if isinstance(a, bool) or isinstance(b, bool):
        return None if a == b else f"{path}: {a!r} != {b!r}"
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        if isinstance(a, float) or isinstance(b, float):
            return None if abs(a - b) <= max(0.06, 0.02 * abs(a)) else f"{path}: {a!r} != {b!r}"
        return None if a == b else f"{path}: {a!r} != {b!r}"
    if type(a) is not type(b):
        return f"{path}: 型別 {type(a).__name__} != {type(b).__name__}（{a!r} vs {b!r}）"
    if isinstance(a, dict):
        if set(a) != set(b):
            return f"{path}: 鍵不同 只在 py={sorted(set(a) - set(b))} 只在 rs={sorted(set(b) - set(a))}"
        for k in a:
            d = deep_eq(a[k], b[k], f"{path}.{k}")
            if d:
                return d
        return None
    if isinstance(a, list):
        if len(a) != len(b):
            return f"{path}: 長度 {len(a)} != {len(b)}"
        for i, (x, y) in enumerate(zip(a, b)):
            d = deep_eq(x, y, f"{path}[{i}]")
            if d:
                return d
        return None
    return None if a == b else f"{path}: {a!r} != {b!r}"


def text_diff(a, b):
    if a == b:
        return None
    al, bl = a.splitlines(), b.splitlines()
    for i in range(max(len(al), len(bl))):
        x = al[i] if i < len(al) else "<EOF>"
        y = bl[i] if i < len(bl) else "<EOF>"
        if x != y:
            return f"第 {i + 1} 行：\n      py: {x!r}\n      rs: {y!r}"
    return "結尾換行不同"


def parse_maybe_json(s):
    try:
        return json.loads(s)
    except ValueError:
        return None


def snapshot(side):
    """spine 底下所有檔（.git 與鎖檔除外）→ {相對路徑: 正規化內容}。"""
    sp = side["root"] / "spine"
    out = {}
    for p in sorted(sp.rglob("*")):
        rel = p.relative_to(sp).as_posix()
        if not p.is_file() or rel.startswith(".git/") or rel == ".engine.lock":
            continue
        rel_n = norm_text(rel, side)
        try:
            t = p.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            out[rel_n] = ("bin", p.stat().st_size)
            continue
        t = norm_text(t, side)
        if p.suffix == ".json":
            j = parse_maybe_json(t)
            out[rel_n] = ("json", j) if j is not None else ("text", t)
        elif rel == "registry.yaml" or p.suffix in (".yaml", ".yml"):
            out[rel_n] = ("yaml", yaml.safe_load(t))
        else:
            out[rel_n] = ("text", t)
    return out


FILE_ALLOWED_USED = set()

RULES_WHY = ("Rust 才有的鍵，比對前拿掉：反芻推薦的 rules（N-20 規則版本）、"
             "search 結果的 decisions（F-22 判斷可搜尋）")


def strip_rust_only(v):
    """Rust 才有的鍵（就地拿掉）：反芻推薦（有 terms＋found 的 dict）的 rules。回傳有沒有拿掉東西。"""
    hit = False
    if isinstance(v, dict):
        if "terms" in v and "found" in v and "rules" in v:
            del v["rules"]
            hit = True
        if "query" in v and "files" in v and "decisions" in v:  # search_knowledge 的判斷段（F-22）
            del v["decisions"]
            hit = True
        for x in v.values():
            hit = strip_rust_only(x) or hit
    elif isinstance(v, list):
        for x in v:
            hit = strip_rust_only(x) or hit
    return hit


def file_allowed(rel):
    return next((why for rx, why in ALLOW_FILES if re.search(rx, rel)), None)


def diff_snapshots(a, b, allowed_used):
    for rel in sorted(set(a) | set(b)):
        why = file_allowed(rel)
        if why:
            if a.get(rel) != b.get(rel):
                FILE_ALLOWED_USED.add(why)
            continue
        if rel not in a:
            return f"只有 rs 寫了 {rel}"
        if rel not in b:
            return f"只有 py 寫了 {rel}"
        (ka, va), (kb, vb) = a[rel], b[rel]
        if ka == "json" and kb == ka and strip_rust_only(vb):
            FILE_ALLOWED_USED.add(RULES_WHY)
        if ka in ("json", "yaml") and kb == ka:
            d = deep_eq(va, vb, rel)
        else:
            d = text_diff(str(va), str(vb))
            d = d and f"{rel} {d}"
        if d:
            return d
    return None


# ── 執行 ────────────────────────────────────────────────────────────────

def subst(s, side):
    """參數裡的佔位符：{R}／__R__＝這一邊的根目錄，<today>＝今天日期。"""
    for k in ("{R}", "__R__"):
        s = s.replace(k, str(side["root"]))
    return s.replace("<today>", time.strftime("%Y-%m-%d"))


def align_minute(margin=15):
    """未讀用 HH:MM 記「已讀到哪」：兩邊各跑一次，分鐘交界若落在兩邊之間，未讀數就差一筆
    （兩個引擎都沒錯，是對拍的時序）。離整分鐘不到 margin 秒就先等到下一分鐘開頭再跑。"""
    import time
    sec = time.localtime().tm_sec
    if sec >= 60 - margin:
        time.sleep(60 - sec + 1)


def run_side(side, args, cwd=None, stdin=None, spine=True, env_extra=None, pre=()):
    cmd = (PY if side["name"] == "py" else [str(side["exe"])])
    full = list(cmd) + list(pre)
    if spine:
        full += ["--spine", str(side["root"] / "spine")]
    full += [subst(a, side) for a in args]
    if stdin is not None:
        stdin = subst(stdin, side)
    e = py_env(side["home"])
    e.update(env_extra or {})
    wd = side["root"] / cwd if cwd else ROOT
    r = subprocess.run(full, cwd=wd, env=e, input=stdin, capture_output=True, text=True,
                       encoding="utf-8", timeout=180)
    return r


def compare(name, sides, args, *, cwd=None, stdin=None, spine=True, json_out=None,
            files=True, env_extra=None, pre=(), stdout_fix=None, results):
    outs = [run_side(s, args, cwd=cwd, stdin=stdin, spine=spine, env_extra=env_extra, pre=pre)
            for s in sides]
    py, rs = outs
    problems = []
    allowed = set()
    if py.returncode != rs.returncode:
        problems.append(f"exit code py={py.returncode} rs={rs.returncode}")
    a, b = norm_text(py.stdout, sides[0]), norm_text(rs.stdout, sides[1])
    if stdout_fix:
        a, b = stdout_fix(a), stdout_fix(b)
    is_json = json_out if json_out is not None else ("--json" in args)
    if is_json:
        ja, jb = parse_maybe_json(a), parse_maybe_json(b)
        if ja is None or jb is None:
            d = text_diff(a, b)
        else:
            if strip_rust_only(jb):
                allowed.add(RULES_WHY)
            d = deep_eq(ja, jb)
            if d and name in ALLOW:
                why, fix = ALLOW[name]
                d = deep_eq(fix(copy.deepcopy(ja)), fix(copy.deepcopy(jb)))
                if not d:
                    allowed.add(why)
    else:
        d = text_diff(a, b)
    if d:
        problems.append(f"stdout {d}")
    if files:
        fd = diff_snapshots(snapshot(sides[0]), snapshot(sides[1]), allowed)
        if fd:
            problems.append(f"檔案 {fd}")
    results.append({"name": name, "ok": not problems, "problems": problems,
                    "allowed": sorted(allowed), "rc": (py.returncode, rs.returncode),
                    "stderr": (py.stderr[-600:], rs.stderr[-600:])})
    return outs


def mcp_compare(name, sides, msgs, admin, results, cwd="repos/impl"):
    stdin = "\n".join(json.dumps(m, ensure_ascii=False) for m in msgs) + "\n"
    args = ["mcp"] + (["--admin"] if admin else [])
    outs = [run_side(s, args, cwd=cwd, stdin=stdin) for s in sides]
    problems = []
    la = [json.loads(norm_text(x, sides[0])) for x in outs[0].stdout.splitlines() if x.strip()]
    lb = [json.loads(norm_text(x, sides[1])) for x in outs[1].stdout.splitlines() if x.strip()]
    if len(la) != len(lb):
        problems.append(f"回應數 py={len(la)} rs={len(lb)}；stderr rs={outs[1].stderr[-300:]}")
    allowed = set()
    for y in lb:
        drop_mcp_extras(y, allowed)
        for c in (y.get("result") or {}).get("content") or []:
            j = parse_maybe_json(c.get("text", ""))
            if j is not None and strip_rust_only(j):
                c["text"] = json.dumps(j, ensure_ascii=False)
                allowed.add(RULES_WHY)
    for x, y in zip(la, lb):
        # tool 回的 text 若是 JSON 就解析後比（工作層），否則逐字比
        for r in (x, y):
            for c in (r.get("result") or {}).get("content") or []:
                j = parse_maybe_json(c.get("text", ""))
                if j is not None:
                    c["text"] = j
        d = deep_eq(x, y, f"id={x.get('id')}")
        if d:
            problems.append(d)
            break
    fd = diff_snapshots(snapshot(sides[0]), snapshot(sides[1]), allowed)
    if fd:
        problems.append(f"檔案 {fd}")
    results.append({"name": name, "ok": not problems, "problems": problems,
                    "allowed": sorted(allowed), "rc": (outs[0].returncode, outs[1].returncode),
                    "stderr": (outs[0].stderr[-600:], outs[1].stderr[-600:])})


def tool(i, tool_name, **arguments):
    return {"jsonrpc": "2.0", "id": i, "method": "tools/call",
            "params": {"name": tool_name, "arguments": arguments}}


def main():
    if not CTX.exists():
        raise SystemExit(f"找不到 {CTX}；先 cargo build -p ctx-cli")
    base = Path(tempfile.mkdtemp(prefix="ctx-parity-"))
    fx = base / "fx"
    build_fixture(fx)
    sides = []
    for nm in ("py", "rs"):
        root = base / nm
        clone(fx, root)
        sides.append({"name": nm, "root": root, "home": root / "home",
                      "exe": CTX if nm == "rs" else Path(sys.executable)})
    res = []
    C = lambda name, args, **kw: compare(name, sides, args, results=res, **kw)  # noqa: E731
    impl, notes = "repos/impl", "repos/notes"

    # ── 讀（初始狀態）──
    C("instructions", ["instructions"], spine=False)
    C("registry list", ["registry", "list"])
    C("registry list --json", ["registry", "list", "--json"])
    C("registry relations", ["registry", "relations"])
    C("registry relations notes", ["registry", "relations", "notes"])
    C("registry exports", ["registry", "exports"])
    C("registry exports --json", ["registry", "exports", "--json"])
    C("registry scan", ["registry", "scan", "{R}/repos"])
    C("registry audit", ["registry", "audit"])
    C("group list", ["group", "list"])
    C("group list --json", ["--json", "group", "list"])
    C("group goal (讀)", ["group", "goal", "產品"])
    C("group context", ["group", "context", "產品"])
    C("group context 臨時", ["group", "context", "", "impl,notes"])
    C("group summary", ["group", "summary", "產品"])
    C("group summary --json", ["group", "summary", "研究", "--json"])
    C("collect", ["collect", "--group", "研究"], stdout_fix=lambda s: re.sub(r"dirty \d+ 天", "dirty N 天", s))
    C("collect --json", ["collect", "--group", "研究", "--json"])
    C("pulse", ["pulse", "--group", "研究"])
    C("pack --estimate", ["pack", "--estimate"])
    C("query --today", ["query", "--today"])
    C("query -v", ["query", "-v"])
    C("query --json", ["query", "--type", "open-loop", "--json"])
    C("query --stats", ["query", "--stats"])
    C("loops", ["loops"])
    C("loops --group 產品", ["loops", "--group", "產品"])
    C("loops --json", ["loops", "--json"])
    C("lint", ["lint"])
    C("unread", ["unread"])
    C("notify", ["notify"])
    C("where --json", ["where", "--json"], cwd=impl)
    C("where (人話)", ["where"], cwd=notes)
    C("where 沒登記", ["where"], cwd="home")
    C("context", ["context"], cwd=impl)
    C("context --json", ["context", "--json"], cwd=impl)
    C("next", ["next"], cwd=impl)
    C("next --json", ["next", "--json"], cwd=notes)
    C("search", ["search", "快取", "LRU"], cwd=impl,
      stdout_fix=lambda s: re.sub(r"\n過去的判斷：.*?(?=\n程式碼：|\Z)", "", s, flags=re.S))
    C("search --json --here", ["search", "快取", "--here", "--json"], cwd=impl)
    C("resolve", ["resolve", "notes:書摘/ch01.md"])
    C("resolve --json", ["resolve", "notes", "--json"])
    C("resolve 錯誤", ["resolve", "nope:x", "--json"])
    C("read", ["read", "notes:書摘/ch01.md"], cwd=impl)
    C("read dir --json", ["read", "notes:書摘", "--json"], cwd=impl)
    C("repo --peek", ["repo", "impl", "--peek"])
    C("repo --json", ["repo", "notes", "--json"])
    C("repo 不在 repo 裡", ["repo", "--json"], cwd="home")
    C("refs check", ["refs", "check"])
    C("refs check --json", ["refs", "check", "--json"])
    C("refs drift", ["refs", "drift"])
    C("refs suggest", ["refs", "suggest", "--min", "1"])
    C("fresh (未跑過)", ["fresh"], cwd=impl)
    C("fresh --stats", ["fresh", "--stats"])
    C("backfill scan", ["backfill", "scan"])
    C("backfill scan --json", ["backfill", "scan", "--json"])
    C("backfill review", ["backfill", "review"])
    C("backfill review --json", ["backfill", "review", "--repo", "impl", "--json"])
    C("hook session-start", ["hook", "session-start"], cwd="home", json_out=True,
      stdin=json.dumps({"session_id": "abc", "cwd": "__R__/repos/impl"}))
    C("upstream", ["upstream", "--group", "研究"])

    # ── 錯誤與 exit code ──
    C("找不到 spine --json", ["where", "--json"], spine=False, cwd="home")
    C("找不到 spine（人話）", ["query"], spine=False, cwd="home")
    C("--spine 不是 spine", ["query"], spine=False, pre=("--spine", "{R}/repos"))
    C("跨組寫入被拒", ["todo", "add", "跨組", "--group", "研究", "--json"], cwd=impl)
    C("registry remove 不存在", ["registry", "remove", "nope"])
    C("registry remove 不存在 --json", ["registry", "remove", "nope", "--json"])
    C("relate 壞 kind", ["registry", "relate", "impl", "notes", "--kind", "nope"])
    C("group goal 不存在", ["group", "goal", "nope", "--json"])
    C("append 壞事件 dead-letter", ["append", "--type", "bogus", "--source", "hotkey",
                                 "--body", "想法別丟", "--dead-letter"])
    C("todo close 不存在", ["todo", "close", "#99", "--json"], cwd=impl)
    C("fresh --dismiss 不存在", ["fresh", "--dismiss", "r99", "--json"])
    C("backfill accept 缺參數", ["backfill", "accept", "--json"])
    C("refs cite 缺 ref", ["refs", "cite"])
    C("collide run 不存在的 cid", ["collide", "run", "2000-01-01-z", "--provider", "mock"])
    C("hook 沒 spine", ["hook", "session-start"], spine=False, cwd="home", stdin="not json")

    # ── 寫（每個寫完比檔案）──
    C("registry add", ["registry", "add", "extra", "{R}/repos/ext", "--tags", "x,y"])
    C("registry set", ["registry", "set", "extra", "resume_when", "等 Q4"])
    C("registry tier", ["registry", "tier", "extra", "paused", "--resume-when", "下季"])
    C("registry tag", ["registry", "tag", "extra", "--add", "z", "--remove", "x"])
    C("registry tag --json", ["registry", "tag", "extra", "--add", "w", "--json"])
    C("registry relate", ["registry", "relate", "impl", "notes", "--kind", "pm-of",
                          "--note", "impl 規劃", "--export", "書摘"])
    C("registry relations impl", ["registry", "relations", "impl"])
    C("registry unrelate", ["registry", "unrelate", "impl", "notes"])
    C("registry export", ["registry", "export", "impl", "源碼", "src", "--note", "程式"])
    C("registry unexport", ["registry", "unexport", "impl", "源碼"])
    C("registry remove", ["registry", "remove", "extra"])
    C("group add", ["group", "add", "暫", "impl,notes"])
    C("group add-member", ["group", "add-member", "暫", "ext"])
    C("group remove-member", ["group", "remove-member", "暫", "notes"])
    C("group rename", ["group", "rename", "暫", "臨"])
    C("group goal 寫", ["group", "goal", "產品", "--text", "把兩個 repo 的說法對齊"])
    C("group goal 讀", ["group", "goal", "產品"])
    C("group remove", ["group", "remove", "臨", "--json"])
    C("append", ["append", "--type", "decision", "--source", "manual", "--kv", "group:研究",
                 "--body", "決定 X"])
    C("append open-loop 預設 due", ["append", "--type", "open-loop", "--source", "manual",
                                    "--kv", "#7", "--body", "opened → 驗證 Y"])
    C("log", ["log", "選 LRU", "--ref", "decision:2026-01-01-d1"], cwd=impl)
    C("log --json", ["log", "第二個判斷", "--json", "--source", "manual"], cwd=impl)
    C("todo add --json", ["todo", "add", "寫文件", "--json"], cwd=impl)
    C("todo close", ["todo", "close", "1", "--note", "做完了"], cwd=impl)
    C("status waiting", ["status", "waiting", "等使用者確認"], cwd=impl)
    C("status --json", ["status", "working", "--json"], cwd=notes)
    C("handoff", ["handoff", "--done", "寫了 X", "--next", "寫 Y", "--remaining", "補測試",
                  "--remaining", "改文件"], cwd=notes)
    C("next 寫完後", ["next", "--json"], cwd=impl)
    C("context 寫完後", ["context", "--json"], cwd=notes)
    C("route", ["route", "--text", "撞出來的結論", "--dest", "repo:impl", "--title", "結論"])
    C("route incubator", ["route", "--text", "孵化想法", "--dest", "incubator"])
    C("collide submit --wait", ["collide", "submit", "全新的想法試撞", "--group", "產品",
                                "--provider", "mock", "--wait"])
    C("collide submit --no-run", ["collide", "submit", "已知的想法", "--repos", "impl,notes",
                                  "--no-run"])
    C("collide run", ["collide", "run", "--cid", "<today>-c", "--provider", "mock"])
    C("append outcome", ["append", "--type", "outcome", "--source", "manual",
                         "--kv", "ref:collision:<today>-a", "--body", "長成了"])
    C("query --stats 後", ["query", "--stats"])
    C("spawn", ["spawn", "born", "{R}/repos/born", "--origin", "collision:<today>-b"])
    C("query --stats spawn 後", ["query", "--stats"])
    C("digest", ["digest", "--group", "產品"])
    C("digest 第二輪", ["digest", "--group", "產品"])
    C("brief", ["brief", "--group", "產品"])
    C("pack", ["pack", "--group", "研究"])
    C("pack --repos", ["pack", "--repos", "impl"])
    C("refs cite", ["refs", "cite", "notes:書摘/ch01.md", "--repo", "impl", "--note", "抄進來"])
    C("refs cite --json 重複", ["refs", "cite", "notes:書摘/ch01.md", "--repo", "impl", "--json"])
    C("refs drift 後", ["refs", "drift", "--json"])
    C("read 記引用", ["read", "notes:notes/plan.md", "--lines", "1-1", "--json"], cwd=impl)
    C("refs suggest --json", ["refs", "suggest", "--min", "1", "--json"])
    C("fresh --run", ["fresh", "--run", "--days", "30"])
    C("fresh", ["fresh", "--all"], cwd=impl)
    C("fresh --json", ["fresh", "--json"], cwd=notes)
    C("fresh --stats 後", ["fresh", "--stats", "--json"])
    C("backfill accept", ["backfill", "accept", "impl", "d1", "t1"])
    C("backfill accept 重複", ["backfill", "accept", "impl", "d1", "--json"])
    C("ask mock", ["ask", "notes", "快取", "怎麼做", "--provider", "mock"], cwd=impl)
    C("ask --json", ["ask", "notes", "LRU？", "--provider", "mock", "--json"], cwd=impl)
    C("repo 推水位線", ["repo", "impl"])
    C("repo 第二次", ["repo", "impl", "--peek"])
    C("hook session-end", ["hook", "session-end"], cwd="home",
      stdin=json.dumps({"session_id": "abc"}))
    C("unread 寫完後", ["unread"])
    align_minute()  # ack 與之後讀未讀的步驟要在兩邊同一分鐘內（N-18）
    C("unread --ack", ["unread", "--ack"])
    C("notify ack 後", ["notify"])
    C("lint 寫完後", ["lint"])
    C("timer --once（無排程）", ["timer", "--once"])
    for s in sides:  # 兩邊同樣加排程
        cfg = s["root"] / "spine" / "config.yaml"
        cfg.write_text(cfg.read_text(encoding="utf-8")
                       + '\nschedule:\n  - {task: commit, at: "00:00"}\n'
                       + '  - {task: brief, at: "00:01", group: 產品}\n', encoding="utf-8")
    C("timer --once", ["timer", "--once"])
    C("timer --once 同日不重跑", ["timer", "--once"])
    C("commit", ["commit", "-m", "parity"])
    C("upstream 後", ["upstream"])
    C("session --print", ["session", "--group", "產品", "--task", "寫文", "--print"],
      stdout_fix=lambda s: re.sub(r"<CTX>[^\n]*", "<CTX…>", s))
    C("setup --dry-run", ["setup", "--spine", "{R}/spine", "--dry-run"], spine=False)
    C("setup", ["setup", "--spine", "{R}/spine"], spine=False)
    bare = {"PATH": "/usr/bin:/bin"}  # 沒有 claude 指令：印手動註冊指令、不失敗
    C("setup --claude --dry-run", ["setup", "--spine", "{R}/spine", "--claude", "--dry-run"],
      spine=False, env_extra=bare, stdout_fix=setup_fix)
    C("setup --claude", ["setup", "--spine", "{R}/spine", "--claude"], spine=False,
      env_extra=bare, stdout_fix=setup_fix)
    C("setup 後不帶 --spine", ["where", "--json"], spine=False, cwd=impl)
    C("setup 不是 spine", ["setup", "--spine", "{R}/repos", "--json"], spine=False)
    C("init 已存在", ["init", "{R}/spine"], spine=False)
    C("init 新的", ["init", "{R}/spine2"], spine=False)

    # ── MCP ──
    init = [{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
            {"jsonrpc": "2.0", "id": 3, "method": "ping"},
            {"jsonrpc": "2.0", "id": 4, "method": "no/such"}]
    align_minute(25)  # MCP 一段會寫事件又讀 notify：整段要落在同一分鐘（N-18）
    mcp_compare("mcp 工作層", sides, init + [
        tool(10, "where_am_i"),
        tool(11, "context_for", card=False),
        tool(12, "next_work", limit=3),
        tool(13, "log_decision", text="MCP 判斷"),
        tool(14, "add_todo", text="MCP 待辦", due="2030-01-01"),
        tool(15, "add_todo", text="跨組", group="研究"),
        tool(16, "close_todo", id="#99"),
        tool(17, "report_status", status="blocked", text="卡住"),
        tool(18, "report_status", status="nope"),
        tool(19, "handoff", done="MCP 收尾", next_step="下一步", remaining=["新事項"]),
        tool(20, "search_knowledge", query="快取 LRU", limit=3),
        tool(21, "read_from", ref="notes:書摘/ch01.md"),
        tool(22, "repo_status", repo="notes"),
        tool(23, "ask_repo", repo="notes", question="LRU？", provider="mock"),
        tool(24, "log_decision"),
        tool(25, "spine_append", type="decision", source="agent"),
        tool(26, "no_such_tool"),
    ], admin=False, results=res)
    align_minute(25)
    mcp_compare("mcp 管理層", sides, init + [
        tool(30, "registry_list"),
        tool(31, "registry_audit"),
        tool(32, "registry_scan", dir="__R__/repos"),
        tool(33, "registry_relations", id="notes"),
        tool(34, "refs_check"),
        tool(35, "group_context", group="研究"),
        tool(36, "group_list"),
        tool(37, "collect", group="產品"),
        tool(38, "pack_estimate"),
        tool(39, "spine_append", type="decision", source="agent", body="MCP 往返"),
        tool(40, "spine_append", type="bogus", source="agent"),
        tool(41, "spine_append", type="open-loop", source="agent", kv=["#9"], body="opened → 驗證"),
        tool(42, "spine_query", today=True, verbose=True),
        tool(43, "spine_stats"),
        tool(44, "open_loops", group="產品"),
        tool(45, "spine_lint"),
        tool(46, "unread"),
        tool(47, "collide_submit", idea="已知的快取想法", group="產品", wait=True, provider="mock"),
        tool(48, "route", text="MCP 落點", dest="incubator", title="mcp"),
        tool(49, "upstream_check"),
        tool(50, "digest_run", group="研究"),
        tool(51, "registry_relate", a="impl", b="impl", kind="pm-of"),
        tool(52, "registry_tag", id="impl", add=["mcp"]),
        tool(53, "group_update", name="產品", add=["notes"]),
        tool(54, "group_goal", name="研究", text="讀完再寫"),
        tool(55, "registry_add", id="x"),
        tool(56, "spawn", id="mcp-born", path="__R__/repos/mcp-born"),
        tool(57, "brief", group="研究"),
        tool(58, "pack", repos="impl", budget=100),
        tool(59, "unread", ack=True),
    ], admin=True, results=res)

    report(res, base)


def report(res, base):
    ok = sum(1 for r in res if r["ok"])
    print(f"\n{'#':>3}  {'結果':<4} 指令")
    for i, r in enumerate(res, 1):
        mark = "PASS" if r["ok"] else "FAIL"
        extra = "（允許差異）" if r["allowed"] else ""
        print(f"{i:>3}  {mark:<4} {r['name']}{extra}  rc={r['rc'][0]}/{r['rc'][1]}")
        for p in r["problems"]:
            print(f"        ✘ {p}")
        if not r["ok"] and (r["stderr"][0] or r["stderr"][1]):
            print(f"        stderr py: {r['stderr'][0].strip()[-300:]!r}")
            print(f"        stderr rs: {r['stderr'][1].strip()[-300:]!r}")
    used = sorted({a for r in res for a in r["allowed"]} | FILE_ALLOWED_USED)
    print(f"\n共 {len(res)} 項，通過 {ok}，失敗 {len(res) - ok}")
    if used:
        print("允許差異：")
        for a in used:
            print(f"  - {a}")
    if ok == len(res) and not os.environ.get("PARITY_KEEP"):
        shutil.rmtree(base, ignore_errors=True)
    else:
        print(f"（fixture 留在 {base}）")
    sys.exit(0 if ok == len(res) else 1)


if __name__ == "__main__":
    main()
