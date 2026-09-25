"""知識相關度（2026-09-25 Agent 友善輪）——「這段文字跟哪些 repo 的哪些檔強相關」。

工具想法.md 的第三個痛點：card_notes 的文章與書，要能很快比對出跟哪些 repo 強相關。
先用最簡單、可解釋、零依賴的做法：BM25 over 各 repo 的 *.md（選料沿用 pack._doc_files，
尊重 .gitignore）。中文用字元 bigram、英數用小寫單字，不需要斷詞器也不需要 embedding。

紀律：
- 洩密哨兵照走——長得像憑證的檔不進索引，也不會出現在片段裡（公私分界）。
- 回傳帶分數與命中片段的「依據」，agent 要引用時說得出來自哪個 repo 哪份檔哪一行。
- 索引依檔案 mtime 快取在 .state/knowledge_index.json（.state/ 已 gitignore）。

跨 repo 參考輪（2026-09-25 同日第二輪）——找得「準」：
- 標題／frontmatter／檔名另建一份索引（htf），命中加權：md 有結構，標題命中比內文命中準得多。
- boosts：呼叫端（agentapi）依主場算好——有關係的 repo 加分、落在 export 內加分；
  近 30 天改過的檔小幅加分。每筆結果帶 why（為什麼排這裡），agent 才判斷得了要不要信。
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
_INDEX_VERSION = 2   # 快取格式版本：舊快取沒有 htf，版本不符就重算
_HEAD_WEIGHT = 2.0   # 標題／frontmatter／檔名命中的權重（相對內文）
_RECENT_DAYS, _RECENT_BOOST = 30, 1.1
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


def heading_text(text, name=""):
    """標題行＋開頭 frontmatter＋檔名（去副檔名）——結構化訊號，另建索引加權。"""
    lines = text.splitlines()
    out = [Path(name).stem] if name else []
    if lines and lines[0].strip() == "---":
        for ln in lines[1:60]:
            if ln.strip() == "---":
                break
            out.append(ln)
    out += [ln.lstrip("#").strip() for ln in lines if ln.startswith("#")]
    return "\n".join(out)


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
            if hit and hit.get("mtime") == mtime and hit.get("v") == _INDEX_VERSION:
                rec = hit
            else:
                try:
                    text = f.read_text(encoding="utf-8", errors="replace")[:_MAX_CHARS]
                except OSError:
                    continue
                if _pack.secret_hits(text):
                    rec = {"mtime": mtime, "secret": True, "v": _INDEX_VERSION}
                else:
                    tf = Counter(tokenize(text))
                    htf = Counter(tokenize(heading_text(text, f.name)))
                    rec = {"mtime": mtime, "tf": dict(tf), "len": sum(tf.values()),
                           "htf": dict(htf), "hlen": sum(htf.values()),
                           "v": _INDEX_VERSION}
            fresh[key] = rec
            if rec.get("secret"):
                continue
            docs.append({"repo": e["id"], "path": key,
                         "rel": f.relative_to(root).as_posix(),
                         "tf": rec["tf"], "len": rec["len"],
                         "htf": rec.get("htf") or {}, "hlen": rec.get("hlen") or 0,
                         "mtime": rec["mtime"]})
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


def _bm25(qcount, docs, tf_key, len_key):
    """每份 doc 的 BM25 分數（同一份 docs 順序）。"""
    n = len(docs)
    avg = (sum(d[len_key] for d in docs) / n) if n else 1
    avg = avg or 1
    df = {t: sum(1 for d in docs if t in d[tf_key]) for t in qcount}
    out = []
    for d in docs:
        s = 0.0
        for t, qn in qcount.items():
            f = d[tf_key].get(t)
            if not f:
                continue
            idf = math.log(1 + (n - df[t] + 0.5) / (df[t] + 0.5))
            s += qn * idf * f * (_K1 + 1) / (f + _K1 * (1 - _B + _B * d[len_key] / avg))
        out.append(s)
    return out


def _boost(d, boosts, now):
    """回傳 (倍率, why 清單)。boosts＝{"repos": {id: (倍率, 理由)}, "paths": [(repo, 前綴, 倍率, 理由)]}。"""
    factor, why = 1.0, []
    rb = (boosts.get("repos") or {}).get(d["repo"])
    if rb:
        factor *= rb[0]
        why.append(rb[1])
    for repo, prefix, f, reason in boosts.get("paths") or []:
        if repo == d["repo"] and (prefix in ("", ".") or d["rel"].startswith(prefix)):
            factor *= f
            why.append(reason)
            break
    if now - d.get("mtime", 0) <= _RECENT_DAYS * 86400:
        factor *= _RECENT_BOOST
        why.append(f"{_RECENT_DAYS} 天內改過")
    return factor, why


def search(entries, query, limit=8, spine_dir=None, exclude_paths=(), boosts=None):
    """BM25（內文＋標題加權）× boosts → {"files": [...], "repos": [...]}。

    files：{repo, file, path, score, why, snippets}；repos：依命中檔分數加總排序，
    回答「這段文字跟哪些 repo 最相關」。exclude_paths＝不把查詢來源檔自己算進去。"""
    import time as _time
    qtoks = tokenize(query)
    if not qtoks:
        return {"query": query, "files": [], "repos": [],
                "note": "查詢字串斷不出可比對的詞"}
    excluded = {str(Path(p).expanduser().resolve()) for p in exclude_paths or ()}
    docs = [d for d in build_docs(entries, spine_dir)
            if str(Path(d["path"]).resolve()) not in excluded]
    if not docs:
        return {"query": query, "files": [], "repos": [],
                "note": "範圍內沒有可索引的 md 檔"}
    qcount = Counter(qtoks)
    body = _bm25(qcount, docs, "tf", "len")
    head = _bm25(qcount, docs, "htf", "hlen")
    now = _time.time()
    scored = []
    for d, sb, sh in zip(docs, body, head):
        s = sb + _HEAD_WEIGHT * sh
        if s <= 0:
            continue
        factor, why = _boost(d, boosts or {}, now)
        if sh > 0:
            why.insert(0, "標題命中")
        scored.append((s * factor, d, why))
    scored.sort(key=lambda x: -x[0])
    files = [{"repo": d["repo"], "file": d["rel"], "path": d["path"],
              "score": round(s, 2), "why": why, "snippets": _snippets(d["path"], qtoks)}
             for s, d, why in scored[:limit]]
    per_repo = Counter()
    hits = Counter()
    for s, d, _ in scored[:50]:
        per_repo[d["repo"]] += s
        hits[d["repo"]] += 1
    repos = [{"repo": r, "score": round(v, 2), "matching_files": hits[r]}
             for r, v in per_repo.most_common()]
    return {"query": query, "files": files, "repos": repos,
            "indexed_files": len(docs)}
