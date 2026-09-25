"""跨 repo 參考（2026-09-25 跨 repo 參考輪）——agent 在 A repo 工作，要又快又準地拿到 B repo 的東西。

之前的做法與問題（檢討見 docs/20260925_跨repo參考輪.md）：
- AGENTS.md 寫死相對路徑（`../x/y`、`projects/x/`）：repo 一搬家就斷，而且沒人發現。
- 整批複製進來：上游一改，副本默默過期。
- agent 不知道對方 repo 哪裡有料，只好整份讀或亂 grep。

四層，由便宜到貴（agent 先用便宜的，不夠再往下）：
    L0 地圖   neighbors()      開場注入「有關係的 repo 提供什麼（exports）」——只注入鄰居，不是全部
    L1 定位   code_search()    ＋ knowledge.search（md，標題加權、關係加分）＝ agentapi.search_knowledge
    L2 讀取   read()           用 `<repo>:<export>/子路徑` 讀，回傳附 commit，並記一筆引用
    L3 委派   ask_repo()       在對方 repo 開一個唯讀 agent 回答（對方的 AGENTS.md／skill 自動生效）

引用紀錄（cites）讓它越用越準：每次 read／ask_repo 記「誰引用了誰的哪個檔、當時的 commit」，
落在 spine 的 `refs/cites.jsonl`（不是事件檔——引用不是要人處理的事，不進收件匣；
但跟 spine 一起被批次 commit，換機器也在）。由此算出：
    drift()            引用過的上游檔之後又改了 → 下游可能過期
    suggest_exports()  常被引用、卻還沒宣告成 export 的目錄 → 建議宣告
另有 lint_doc_refs()：掃各 repo 的 agent 文件，抓出指向其他 repo 卻已失效的路徑。
"""
import datetime as _dt
import json
import re
import subprocess
from collections import Counter, defaultdict
from pathlib import Path

from . import agents as _agents
from . import config as _config
from . import pack as _pack
from . import registry
from .locking import FileLock
from .spine import LOCK_NAME

_MAX_READ_CHARS = 40_000
_MAX_DIR_ENTRIES = 200


class RefError(ValueError):
    """code／hint 形狀與 agentapi.AgentError 相同（agentapi 會轉成 AgentError）。"""

    def __init__(self, code, message, hint=None):
        super().__init__(message)
        self.code, self.message, self.hint = code, message, hint


def _git(path, *args, timeout=15):
    try:
        r = subprocess.run(["git", "-c", "core.quotePath=false", "-C", str(path), *args],
                           capture_output=True, text=True, encoding="utf-8",
                           errors="replace", timeout=timeout)
    except (OSError, subprocess.SubprocessError):
        return 1, ""
    return r.returncode, (r.stdout or "").rstrip("\n")


# ---------------------------------------------------------------- 名字 → 路徑

def parse_ref(ref):
    """`<repo>`｜`<repo>:<export>[/<子路徑>]`｜`<repo>:<相對路徑>`｜`<repo>/<相對路徑>` → (repo, rest)。"""
    ref = (ref or "").strip()
    if not ref:
        raise RefError("bad_request", "ref 不能是空的",
                       "格式：<repo>、<repo>:<export>、<repo>:<export>/<子路徑> 或 <repo>/<相對路徑>")
    if ":" in ref:
        repo, rest = ref.split(":", 1)
    elif "/" in ref:
        repo, rest = ref.split("/", 1)
    else:
        repo, rest = ref, ""
    return repo.strip(), rest.strip().lstrip("/")


def _repo_entry(spine_dir, repo_id):
    try:
        return registry.get_repo(spine_dir, repo_id)
    except ValueError:
        ids = [r["id"] for r in registry.load(spine_dir)["repos"]]
        close = [i for i in ids if repo_id.lower() in i.lower() or i.lower() in repo_id.lower()]
        raise RefError("not_found", f"沒有登記這個 repo：{repo_id}",
                       ("你是不是要找：" + "、".join(close[:5])) if close
                       else "用 registry list 看已登記的 repo id。")


def resolve(spine_dir, ref):
    """名字 → 實際路徑。回傳 {repo, export, rel, path, root, exists, kind, desc?}。
    rest 的第一段若是 export 名稱就換成 export 路徑，否則當成 repo 內相對路徑。"""
    repo_id, rest = parse_ref(ref)
    entry = _repo_entry(spine_dir, repo_id)
    root = Path(entry["path"]).expanduser().resolve()
    exports = registry.exports_of(entry)
    export, desc = None, None
    head, _, tail = rest.partition("/")
    if head and head in exports:
        export, desc = head, exports[head].get("desc")
        base = exports[head]["path"].rstrip("/")
        rest = f"{base}/{tail}" if tail else base
    rel = rest.strip("/") or "."
    target = (root / rel).resolve()
    if target != root and root not in target.parents:
        raise RefError("bad_request", f"路徑跳出 repo：{ref}", "ref 只能指向 repo 內的檔案。")
    out = {"ok": True, "ref": ref, "repo": repo_id, "export": export,
           "rel": "." if target == root else target.relative_to(root).as_posix(),
           "path": str(target), "root": str(root), "exists": target.exists(),
           "kind": "dir" if target.is_dir() else ("file" if target.is_file() else "missing")}
    if desc:
        out["desc"] = desc
    if not out["exists"]:
        out["hint"] = ("路徑不存在。" + (f"{repo_id} 的 exports：" + "、".join(
            f"{n}（{v['path']}）" for n, v in exports.items()) if exports else
            f"{repo_id} 還沒宣告 exports，用 find 找檔案。"))
    return out


def neighbors(spine_dir, repo_id):
    """L0 地圖：跟 repo_id 有關係的 repo，各自提供什麼。
    [{peer, kind, label, direction, note?, path, exports: {名稱: {path, desc?}}, uses?}]。
    關係上有標 exports（例 feeds 只吃「書摘」）時，uses 只列那幾個；exports 仍列對方全部。"""
    out = []
    data = registry.load(spine_dir)
    by_id = {r["id"]: r for r in data["repos"]}
    for rel in registry.relations_of(spine_dir, repo_id):
        peer = by_id.get(rel["peer"])
        if not peer:
            continue
        row = {"peer": rel["peer"], "kind": rel["kind"], "label": rel["label"],
               "direction": rel["direction"], "path": peer["path"],
               "exports": registry.exports_of(peer)}
        if rel.get("note"):
            row["note"] = rel["note"]
        if rel.get("exports") and rel["direction"] == "in":
            row["uses"] = rel["exports"]  # 對方 feeds 我、而且只給這幾個
        out.append(row)
    return out


def neighbor_lines(nbs, max_exports=4):
    """鄰居地圖 → 人話行（開場注入用，每個鄰居 1～3 行）。"""
    lines = []
    for n in nbs:
        arrow = (f"{n['peer']}（{n['label']}{'' if n['direction'] == 'in' else ' 你→它'}）")
        ex = n["exports"]
        if n.get("uses"):
            ex = {k: v for k, v in ex.items() if k in n["uses"]}
        if ex:
            items = [f"`{n['peer']}:{k}`→{v['path']}" + (f"（{v['desc']}）" if v.get("desc") else "")
                     for k, v in list(ex.items())[:max_exports]]
            more = f" …另 {len(ex) - max_exports} 個" if len(ex) > max_exports else ""
            lines.append(f"- {arrow}：" + "；".join(items) + more)
        else:
            lines.append(f"- {arrow}：沒宣告 exports（用 find 找，或 ask_repo 問它）")
    return lines


# ---------------------------------------------------------------- 讀 ＋ 引用紀錄

def _cites_path(spine_dir):
    return Path(spine_dir) / "refs" / "cites.jsonl"


def load_cites(spine_dir):
    p = _cites_path(spine_dir)
    out = []
    if not p.exists():
        return out
    for ln in p.read_text(encoding="utf-8").splitlines():
        try:
            out.append(json.loads(ln))
        except ValueError:
            continue
    return out


def record_cite(spine_dir, consumer, provider, rel, commit, via, note=None, when=None):
    """記一筆引用。同一天同 (consumer, provider, rel, via) 只記一次（讀十次不是十個訊號）。
    consumer 是 None（在 spine 或未登記目錄）或等於 provider（自己讀自己）時不記。"""
    if not consumer or consumer == provider:
        return None
    now = when or _dt.datetime.now()
    rec = {"at": f"{now:%Y-%m-%d %H:%M}", "consumer": consumer, "provider": provider,
           "rel": rel, "commit": commit, "via": via}
    if note:
        rec["note"] = note
    p = _cites_path(spine_dir)
    p.parent.mkdir(parents=True, exist_ok=True)
    with FileLock(Path(spine_dir) / LOCK_NAME):
        day = rec["at"][:10]
        for c in load_cites(spine_dir):
            if (c.get("at", "")[:10], c.get("consumer"), c.get("provider"), c.get("rel"),
                    c.get("via")) == (day, consumer, provider, rel, via):
                return None
        with p.open("a", encoding="utf-8") as f:
            f.write(json.dumps(rec, ensure_ascii=False) + "\n")
    return rec


def _head(root):
    rc, out = _git(root, "rev-parse", "--short", "HEAD")
    return out if rc == 0 and out else None


def _parse_lines(lines):
    if not lines:
        return None
    m = re.fullmatch(r"\s*(\d+)\s*(?:-\s*(\d+))?\s*", str(lines))
    if not m:
        raise RefError("bad_request", f"lines 格式不對：{lines!r}", "用 120 或 120-180。")
    a = int(m.group(1))
    b = int(m.group(2) or a)
    return (min(a, b), max(a, b))


def read(spine_dir, ref, consumer=None, lines=None, max_chars=_MAX_READ_CHARS):
    """L2：讀 `<repo>:<export>/子路徑`。檔案回內容（可指定行段），目錄回清單。
    一律附 commit（引用時說得出版本）；長得像憑證的檔拒讀（洩密哨兵同 pack）。"""
    r = resolve(spine_dir, ref)
    if not r["exists"]:
        raise RefError("not_found", f"不存在：{r['repo']}/{r['rel']}", r.get("hint"))
    root, target = Path(r["root"]), Path(r["path"])
    out = {"ok": True, "repo": r["repo"], "export": r["export"], "rel": r["rel"],
           "path": r["path"], "commit": _head(root), "kind": r["kind"]}
    if r["kind"] == "dir":
        rc, listed = _git(root, "ls-files", "-z", "-co", "--exclude-standard", "--",
                          r["rel"] if r["rel"] != "." else ".")
        names = [x for x in listed.split("\0") if x] if rc == 0 else [
            p.relative_to(root).as_posix() for p in target.rglob("*") if p.is_file()]
        prefix = "" if r["rel"] == "." else r["rel"].rstrip("/") + "/"
        children = Counter()
        files = []
        for n in names:
            sub = n[len(prefix):] if n.startswith(prefix) else n
            if "/" in sub:
                children[sub.split("/", 1)[0] + "/"] += 1
            else:
                files.append(sub)
        entries = [{"name": k, "files": v} for k, v in sorted(children.items())] + \
                  [{"name": f} for f in sorted(files)]
        out["entries"] = entries[:_MAX_DIR_ENTRIES]
        out["total_entries"] = len(entries)
        out["hint"] = "挑檔案再 read `<repo>:<路徑>`；不確定哪份相關就先用 find。"
    else:
        try:
            text = target.read_text(encoding="utf-8", errors="replace")
        except OSError as e:
            raise RefError("failed", f"讀不到：{e}")
        hits = _pack.secret_hits(text)
        if hits:
            raise RefError("secret", f"{r['repo']}/{r['rel']} 疑似含憑證（{', '.join(hits)}），不讀出",
                           "這是洩密哨兵；真的需要請使用者自己看。")
        all_lines = text.splitlines()
        rng = _parse_lines(lines)
        if rng:
            a, b = rng
            body = "\n".join(all_lines[a - 1:b])
            out["lines"] = f"{a}-{min(b, len(all_lines))}"
        else:
            body = text
        out["total_lines"] = len(all_lines)
        out["truncated"] = len(body) > max_chars
        out["content"] = body[:max_chars]
        if out["truncated"]:
            out["hint"] = f"內容超過 {max_chars} 字被截斷；用 lines 指定段落（總共 {len(all_lines)} 行）。"
        rc, fc = _git(root, "log", "-1", "--format=%h", "--", r["rel"])
        if rc == 0 and fc:
            out["file_commit"] = fc
    record_cite(spine_dir, consumer, r["repo"], r["rel"], out["commit"], "read")
    return out


# ---------------------------------------------------------------- 程式碼定位（L1 的 code 半邊）

_ASCII_TERM = re.compile(r"[A-Za-z_][A-Za-z0-9_\-\.]{2,}")
_CJK_TERM = re.compile(r"[㐀-鿿豈-﫿]{2,8}")
_CODE_STOP = {"the", "and", "for", "with", "that", "this", "from", "what", "how", "where",
              "which", "does", "into", "about", "when", "use", "are"}


def query_terms(query, limit=8):
    """查詢 → git grep 用的字面詞：英數識別字（≥3 字）＋中文詞段（2～8 字）。"""
    terms = []
    for t in _ASCII_TERM.findall(query or ""):
        if t.lower() not in _CODE_STOP and t not in terms:
            terms.append(t)
    for t in _CJK_TERM.findall(query or ""):
        if t not in terms:
            terms.append(t)
    return terms[:limit]


def code_search(entries, query, limit=8, per_repo=40):
    """程式碼不建索引（改得快、索引跟不上），直接 git grep -F，排除 *.md（md 走 BM25）。
    分數＝命中幾個不同的查詢詞，同分看命中行數。回傳 [{repo, file, path, score, why, snippets}]。"""
    terms = query_terms(query)
    if not terms:
        return []
    rows = []
    for e in entries:
        root = Path(e["path"]).expanduser()
        if not root.is_dir() or e.get("type") == "external":
            continue
        args = ["grep", "-n", "-I", "-i", "-F", "--max-count=20"]
        for t in terms:
            args += ["-e", t]
        args += ["--", ".", ":(exclude)*.md", ":(exclude)*.lock", ":(exclude)*-lock.json",
                 ":(exclude)*.min.js", ":(exclude)*.svg"]
        rc, out = _git(root, *args, timeout=10)
        if rc not in (0,) or not out:
            continue
        per_file = defaultdict(list)
        for ln in out.splitlines()[:2000]:
            parts = ln.split(":", 2)
            if len(parts) == 3 and parts[1].isdigit():
                per_file[parts[0]].append((int(parts[1]), parts[2]))
        for f, hits in list(per_file.items())[:per_repo]:
            low = " ".join(h[1].lower() for h in hits)
            matched = [t for t in terms if t.lower() in low]
            best = sorted(hits, key=lambda h: -sum(t.lower() in h[1].lower() for t in terms))[:2]
            rows.append({"repo": e["id"], "file": f, "path": str(root / f),
                         "score": len(matched) + min(len(hits), 20) / 100,
                         "why": [f"程式碼含 {'、'.join(matched[:4])}"],
                         "snippets": [{"line": n, "text": t.strip()[:160]}
                                      for n, t in sorted(best)]})
    rows.sort(key=lambda x: -x["score"])
    for r in rows:
        r["score"] = round(r["score"], 2)
    return rows[:limit]


# ---------------------------------------------------------------- 委派（L3）

ASK_PROMPT = """你在 repo「{repo}」裡，被另一個 repo（{consumer}）的 agent 請來回答一個問題。
只能讀檔（Read／Grep／Glob），不要改任何東西。先照這個 repo 的 AGENTS.md／CLAUDE.md 慣例找資料。

【問題】
{question}

回答規則：
- 只根據這個 repo 裡的檔案回答；找不到就直說找不到，不要編。
- 結論精簡（300 字以內），每個重點標出來源檔（repo 內相對路徑，能給行號就給）。
- 最後只輸出一個 JSON 物件，不要其他文字：
{{"answer": "<回答>", "citations": [{{"file": "<相對路徑>", "lines": "<例 12-40，可省略>", "why": "<這份檔支持哪個重點>"}}], "not_found": false}}
"""


def _parse_ask(text):
    m = re.search(r"\{.*\}", text or "", re.S)
    if m:
        try:
            obj = json.loads(m.group(0))
            if isinstance(obj, dict) and "answer" in obj:
                cites = [c for c in obj.get("citations") or [] if isinstance(c, dict) and c.get("file")]
                return {"answer": str(obj["answer"]), "citations": cites,
                        "not_found": bool(obj.get("not_found"))}
        except ValueError:
            pass
    return {"answer": (text or "").strip(), "citations": [], "not_found": False,
            "note": "回答不是預期的 JSON，原文照回；沒有結構化引用。"}


def ask_repo(spine_dir, repo_id, question, consumer=None, provider=None, model=None):
    """L3：在對方 repo 開一個唯讀 agent 回答問題，只回結論＋引用，不把一堆檔案塞進你的 context。
    provider 預設 config provider.ask（沒設＝claude；這個動作本來就是要 LLM）。"""
    if not (question or "").strip():
        raise RefError("bad_request", "question 不能是空的")
    entry = _repo_entry(spine_dir, repo_id)
    root = Path(entry["path"]).expanduser().resolve()
    if not root.is_dir():
        raise RefError("not_found", f"{repo_id} 的路徑不存在：{root}")
    cfg = _config.load(spine_dir)
    provider = provider or (cfg.get("provider") or {}).get("ask") or "claude"
    prompt = ASK_PROMPT.format(repo=repo_id, consumer=consumer or "未登記的目錄",
                               question=question.strip())
    try:
        raw = _agents.run_text(provider, prompt, spine_dir, model=model, cwd=root,
                               read_only=True)
    except _agents.AgentUnsure as e:
        raise RefError("failed", f"ask_repo 失敗：{e}",
                       "改用 find＋read 自己找；或確認 claude CLI 已登入。")
    res = _parse_ask(raw)
    commit = _head(root)
    for c in res["citations"]:
        rel = str(c["file"]).lstrip("./")
        record_cite(spine_dir, consumer, repo_id, rel, commit, "ask")
    return {"ok": True, "repo": repo_id, "provider": provider, "commit": commit, **res}


# ---------------------------------------------------------------- drift ＋ 建議 exports

def drift(spine_dir, consumer=None, provider=None):
    """引用過的上游檔，在引用當時的 commit 之後又改過 → 下游可能過期。
    每個 (consumer, provider, rel) 只看最近一次引用。"""
    latest = {}
    for c in load_cites(spine_dir):
        if consumer and c.get("consumer") != consumer:
            continue
        if provider and c.get("provider") != provider:
            continue
        key = (c.get("consumer"), c.get("provider"), c.get("rel"))
        if key not in latest or c.get("at", "") >= latest[key].get("at", ""):
            latest[key] = c
    roots = {r["id"]: Path(r["path"]).expanduser() for r in registry.load(spine_dir)["repos"]}
    out = []
    for (cons, prov, rel), c in sorted(latest.items()):
        root = roots.get(prov)
        if root is None or not root.is_dir() or not c.get("commit"):
            continue
        rc, log = _git(root, "log", "--format=%h%x09%ad%x09%s", "--date=short",
                       f"{c['commit']}..HEAD", "--", rel)
        if rc != 0:
            out.append({"consumer": cons, "provider": prov, "rel": rel,
                        "cited_commit": c["commit"], "cited_at": c["at"],
                        "changes": None, "note": "引用時的 commit 已不在歷史裡（rebase？），無法比對"})
            continue
        commits = [ln.split("\t", 2) for ln in log.splitlines() if ln]
        if commits:
            out.append({"consumer": cons, "provider": prov, "rel": rel,
                        "cited_commit": c["commit"], "cited_at": c["at"],
                        "changes": len(commits),
                        "latest": {"hash": commits[0][0], "date": commits[0][1],
                                   "subject": commits[0][2] if len(commits[0]) > 2 else ""}})
    return out


def drift_lines(items, limit=5):
    lines = []
    for d in items[:limit]:
        if d.get("changes") is None:
            lines.append(f"- {d['consumer']} 引用的 {d['provider']}:{d['rel']}：{d['note']}")
        else:
            lines.append(f"- {d['consumer']} 引用的 {d['provider']}:{d['rel']}（{d['cited_at'][:10]}）"
                         f"之後改了 {d['changes']} 次；最新 {d['latest']['date']}「{d['latest']['subject']}」")
    if len(items) > limit:
        lines.append(f"- …另 {len(items) - limit} 筆（ctx refs drift 看全部）")
    return lines


def suggest_exports(spine_dir, min_count=3, depth=1):
    """常被別的 repo 引用、卻沒被任何 export 涵蓋的目錄 → 建議宣告成 export。
    depth＝取引用路徑的前幾層目錄當候選。"""
    data = registry.load(spine_dir)
    exp = {r["id"]: [v["path"].rstrip("/") for v in registry.exports_of(r).values()]
           for r in data["repos"]}
    counts, consumers = Counter(), defaultdict(set)
    for c in load_cites(spine_dir):
        prov, rel = c.get("provider"), c.get("rel") or ""
        if any(p in (".", "") or rel == p or rel.startswith(p + "/") for p in exp.get(prov, [])):
            continue
        parts = rel.split("/")
        cand = "/".join(parts[:depth]) if len(parts) > depth else (parts[0] if len(parts) > 1 else rel)
        counts[(prov, cand)] += 1
        consumers[(prov, cand)].add(c.get("consumer"))
    return [{"repo": prov, "path": cand, "cites": n, "consumers": sorted(consumers[(prov, cand)]),
             "command": f"ctx registry export {prov} <名稱> {cand}"}
            for (prov, cand), n in counts.most_common() if n >= min_count]


# ---------------------------------------------------------------- agent 文件裡的跨 repo 路徑

AGENT_DOCS = ("AGENTS.md", "CLAUDE.md", "GEMINI.md")
SKILL_GLOBS = (".claude/skills/*/SKILL.md", ".agents/skills/*/SKILL.md")
_CODE_SPAN = re.compile(r"`([^`\n]+)`")
_MD_LINK = re.compile(r"\]\(([^)\s]+)\)")
_BARE_UP = re.compile(r"(?<![\w/.`])(\.\./[^\s`)\]'\"，。、；：）]+)")
_TRAIL = "，。、；：）)]}>'\".,;:"


def _doc_files(root):
    out = [root / n for n in AGENT_DOCS if (root / n).is_file()]
    for g in SKILL_GLOBS:
        out += sorted(p for p in root.glob(g) if p.is_file())
    return out


def _path_candidates(line):
    cands = _CODE_SPAN.findall(line) + _MD_LINK.findall(line) + _BARE_UP.findall(line)
    out = []
    for c in cands:
        c = c.strip().rstrip(_TRAIL)
        if "/" not in c or " " in c or "://" in c or c.startswith(("-", "@", "#")):
            continue
        if "..." in c or "…" in c:               # 省略號＝示意路徑，不驗
            continue
        c = re.split(r"[*<{?]", c, 1)[0]          # glob／佔位符之後不驗
        c = re.sub(r":\d+(?:-\d+)?$", "", c)       # 檔案:行號
        if c and c not in out:
            out.append(c)
    return out


def lint_doc_refs(spine_dir, repo_ids=None):
    """掃已登記 repo 的 agent 文件（AGENTS／CLAUDE／GEMINI.md、skills 的 SKILL.md），
    找出「指向其他已登記 repo、但路徑已失效」的引用。只報有把握的：路徑裡要出現某個
    其他 repo 的 id 或資料夾名，且依文件所在位置、repo 根目錄都解析不到；
    以 repo 名開頭、子路徑在該 repo 裡確實存在的（`<repo>/子路徑`）視為用名字引用，不報。
    回傳 [{repo, doc, line, ref, target, target_path, fix}]。"""
    data = registry.load(spine_dir)
    repos = [r for r in data["repos"] if repo_ids is None or r["id"] in repo_ids]
    names = {}
    for r in data["repos"]:
        p = Path(r["path"]).expanduser()
        names[r["id"]] = r
        names.setdefault(p.name, r)
    out = []
    for r in repos:
        root = Path(r["path"]).expanduser()
        if not root.is_dir():
            continue
        own = {r["id"], root.name}
        for doc in _doc_files(root):
            try:
                text = doc.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            in_fence = False
            for i, line in enumerate(text.splitlines(), 1):
                if line.lstrip().startswith("```"):
                    in_fence = not in_fence
                    continue
                if in_fence or "~~" in line:   # 程式碼區塊是範例；刪除線是已作廢的舊路徑
                    continue
                for c in _path_candidates(line):
                    segs = [s for s in c.split("/") if s not in ("", ".", "..", "~")]
                    target = next((names[s] for s in segs if s in names and s not in own), None)
                    if target is None:
                        continue
                    if c.startswith("~") or c.startswith("/"):
                        tries = [Path(c).expanduser()]
                    else:
                        tries = [doc.parent / c, root / c]
                    if any(t.exists() for t in tries):
                        continue
                    seg = next(s for s in segs if s in names and s not in own)
                    after = c.split(seg, 1)[1].strip("/")
                    tpath = Path(target["path"]).expanduser()
                    if segs[0] == seg and tpath.is_dir() and (tpath / after).exists():
                        continue  # `<repo名>/子路徑` 且子路徑在該 repo 裡存在＝用名字引用，不算失效
                    ref = f"{target['id']}" + (f":{after}" if after else "")
                    out.append({"repo": r["id"], "doc": doc.relative_to(root).as_posix(),
                                "line": i, "ref": c, "target": target["id"],
                                "target_path": target["path"],
                                "fix": f"{target['id']} 實際在 {target['path']}；"
                                       f"改寫成 `{ref}` 並用 `ctx resolve {ref}` 取路徑"})
    return out
