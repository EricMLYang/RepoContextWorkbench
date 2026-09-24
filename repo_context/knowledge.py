"""知識相關度（2026-09-25 Agent 友善輪）——「這段文字跟哪些 repo 的哪些檔強相關」。

工具想法.md 的第三個痛點：card_notes 的文章與書，要能很快比對出跟哪些 repo 強相關。
先用最簡單、可解釋、零依賴的做法：BM25 over 各 repo 的 *.md（選料沿用 pack._doc_files，
尊重 .gitignore）。中文用字元 bigram、英數用小寫單字，不需要斷詞器也不需要 embedding。

紀律：
- 洩密哨兵照走——長得像憑證的檔不進索引，也不會出現在片段裡（公私分界）。
- 回傳帶分數與命中片段的「依據」，agent 要引用時說得出來自哪個 repo 哪份檔哪一行。
- 索引依檔案 mtime 快取在 .state/knowledge_index.json（.state/ 已 gitignore）。
"""
import json
import math
import re
from collections import Counter
from pathlib import Path

from . import pack as _pack

_WORD_RE = re.compile(r"[a-z0-9][a-z0-9_\-\.]*[a-z0-9]|[a-z0-9]")
_CJK_RE = re.compile(r"[㐀-鿿豈-﫿]+")
_K1, _B = 1.5, 0.75
_MAX_CHARS = 200_000  # 單檔上限：超大檔只索引前段（避免一本書的全文拖垮查詢）
_STOP = {"the", "and", "for", "with", "that", "this", "are", "is", "of", "to",
         "in", "on", "md", "a", "an", "or", "be", "it", "as", "by", "at"}


def tokenize(text):
    """英數：小寫單字（去停用字）；中文：字元 bigram（單字元段落保留單字）。"""
    text = text.lower()
    toks = [w for w in _WORD_RE.findall(text) if w not in _STOP]
    for seg in _CJK_RE.findall(text):
        if len(seg) == 1:
            toks.append(seg)
        else:
            toks.extend(seg[i:i + 2] for i in range(len(seg) - 1))
    return toks


def _cache_path(spine_dir):
    return Path(spine_dir) / ".state" / "knowledge_index.json"


def _load_cache(spine_dir):
    try:
        return json.loads(_cache_path(spine_dir).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}


def _save_cache(spine_dir, cache):
    p = _cache_path(spine_dir)
    try:
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(json.dumps(cache, ensure_ascii=False), encoding="utf-8")
    except OSError:
        pass  # 快取寫不進去不影響查詢


def build_docs(entries, spine_dir=None):
    """[{repo, path, rel, tf, len}]。有 spine_dir 就走 mtime 快取。"""
    cache = _load_cache(spine_dir) if spine_dir else {}
    fresh, docs = {}, []
    for e in entries:
        root = Path(e["path"]).expanduser()
        if not root.is_dir():
            continue
        for f in _pack._doc_files(root):
            key = str(f)
            try:
                mtime = f.stat().st_mtime
            except OSError:
                continue
            hit = cache.get(key)
            if hit and hit.get("mtime") == mtime:
                rec = hit
            else:
                try:
                    text = f.read_text(encoding="utf-8", errors="replace")[:_MAX_CHARS]
                except OSError:
                    continue
                if _pack.secret_hits(text):
                    rec = {"mtime": mtime, "secret": True}
                else:
                    tf = Counter(tokenize(text))
                    rec = {"mtime": mtime, "tf": dict(tf), "len": sum(tf.values())}
            fresh[key] = rec
            if rec.get("secret"):
                continue
            docs.append({"repo": e["id"], "path": key,
                         "rel": f.relative_to(root).as_posix(),
                         "tf": rec["tf"], "len": rec["len"]})
    if spine_dir:
        cache.update(fresh)
        _save_cache(spine_dir, cache)
    return docs


def _snippets(path, qtoks, limit=2):
    """命中最多查詢詞的幾行（附行號）——給 agent 引用的依據。"""
    try:
        lines = Path(path).read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return []
    qset = set(qtoks)
    scored = []
    for i, ln in enumerate(lines[:5000]):
        s = sum(1 for t in set(tokenize(ln)) if t in qset)
        if s:
            scored.append((s, i))
    scored.sort(key=lambda x: (-x[0], x[1]))
    return [{"line": i + 1, "text": lines[i].strip()[:160]}
            for _, i in sorted(scored[:limit], key=lambda x: x[1])]


def search(entries, query, limit=8, spine_dir=None, exclude_paths=()):
    """BM25 → {"files": [...], "repos": [...]}。

    files：{repo, file, path, score, snippets}；repos：依命中檔分數加總排序，
    回答「這段文字跟哪些 repo 最相關」。exclude_paths＝不把查詢來源檔自己算進去。"""
    qtoks = tokenize(query)
    if not qtoks:
        return {"query": query, "files": [], "repos": [],
                "note": "查詢字串斷不出可比對的詞"}
    excluded = {str(Path(p).expanduser().resolve()) for p in exclude_paths}
    docs = [d for d in build_docs(entries, spine_dir)
            if str(Path(d["path"]).resolve()) not in excluded]
    n = len(docs)
    if not n:
        return {"query": query, "files": [], "repos": [],
                "note": "範圍內沒有可索引的 md 檔"}
    avg = sum(d["len"] for d in docs) / n or 1
    qcount = Counter(qtoks)
    df = {t: sum(1 for d in docs if t in d["tf"]) for t in qcount}
    scored = []
    for d in docs:
        s = 0.0
        for t, qn in qcount.items():
            f = d["tf"].get(t)
            if not f:
                continue
            idf = math.log(1 + (n - df[t] + 0.5) / (df[t] + 0.5))
            s += qn * idf * f * (_K1 + 1) / (f + _K1 * (1 - _B + _B * d["len"] / avg))
        if s > 0:
            scored.append((s, d))
    scored.sort(key=lambda x: -x[0])
    files = [{"repo": d["repo"], "file": d["rel"], "path": d["path"],
              "score": round(s, 2), "snippets": _snippets(d["path"], qtoks)}
             for s, d in scored[:limit]]
    per_repo = Counter()
    hits = Counter()
    for s, d in scored[:50]:
        per_repo[d["repo"]] += s
        hits[d["repo"]] += 1
    repos = [{"repo": r, "score": round(v, 2), "matching_files": hits[r]}
             for r, v in per_repo.most_common()]
    return {"query": query, "files": files, "repos": repos,
            "indexed_files": n}
