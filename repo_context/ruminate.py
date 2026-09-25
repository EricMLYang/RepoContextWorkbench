"""反芻輪（2026-09-25）——新進的知識與別組的決策，主動對上每組手上的工作。

工具想法.md 的兩個痛點到這裡為止都只有「拉」：search_knowledge 要 agent 先想到去搜。
但使用者持續把文章、書摘、想法 repo 化（card_notes），也在各組留下判斷——這些新東西
跟另一組正在做的事強相關時，沒有人會想到去搜。這裡補上「推」：

- 每組的「手上工作」＝目標＋未結事項＋上次交接＋最近幾筆判斷（全是使用者／agent 自己寫的字）。
- 候選＝上次反芻之後**別組 repo 新增或改過的 md**，加上**別組新留下的判斷／交接**。
- BM25 比對（沿用 knowledge.py），只收「共同的少見詞」夠多的——寧缺勿濫，每組最多 3 筆。
- 結果住 `.state/ruminate.json`（候選層，不進收件匣——跟引用紀錄同一個理由：不是要人處理的事），
  由 SessionStart 開場注入、`ctx fresh` 看、`ctx fresh --dismiss` 打掉。
- **越用越準的量尺**：agent 之後 read_from 讀了某筆推薦（cites.jsonl 有記錄）＝用上了；
  `ctx fresh --stats` 算命中率。沒人用的推薦＝閾值該調，不是 agent 懶。

Letta 的 sleep-time compute／Claude 的 Dreaming 是同一個想法：在會話之間把料先想好；
這裡刻意不用 LLM——零成本、可解釋（每筆都列出共同詞），排程每天跑一次也不心疼。
"""
import datetime as _dt
import json
import math
import os
import subprocess
import sys
from collections import Counter
from pathlib import Path

from . import config as _config
from . import crossref as _xref
from . import knowledge as _knowledge
from . import registry, spine
from .locking import FileLock
from .spine import LOCK_NAME

MAX_PER_GROUP = 3
_KEEP_DAYS = 30        # 推薦保留天數（之後自然淡出，不堆積）
_FOCUS_DECISIONS = 5   # 手上工作取最近幾筆判斷
_STALE_HOURS = 20      # 開場發現超過這麼久沒反芻 → 背景補跑一次


def _state_path(spine_dir):
    return Path(spine_dir) / ".state" / "ruminate.json"


def load_state(spine_dir):
    try:
        return json.loads(_state_path(spine_dir).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {"last_run": None, "links": []}


def _save_state(spine_dir, state):
    p = _state_path(spine_dir)
    p.parent.mkdir(parents=True, exist_ok=True)
    tmp = p.with_suffix(".tmp")
    tmp.write_text(json.dumps(state, ensure_ascii=False, indent=1), encoding="utf-8")
    os.replace(tmp, p)


def _cfg(spine_dir):
    c = (_config.load(spine_dir).get("ruminate") or {})
    return {"window_days": int(c.get("window_days", 14)),
            "min_shared": int(c.get("min_shared", 3)),
            "max_per_group": int(c.get("max_per_group", MAX_PER_GROUP))}


# ---------------------------------------------------------------- 手上工作

def _is_work(ev):
    body = ev.body or ""
    return ev.type in ("decision", "outcome") and not body.startswith(spine.ADMIN_TAG)


def focus(spine_dir, group, events=None, loops=None):
    """一組手上的工作 → (文字, 依據清單)。全是寫下來的字；沒寫就沒有，不從 commit 猜。"""
    g = registry.get_group(spine_dir, group)
    ids = list(g.get("members") or [])
    events = spine.iter_events(spine_dir) if events is None else events
    loops = spine.open_loops(spine_dir) if loops is None else loops
    parts, basis = [], []
    if (g.get("goal") or "").strip():
        parts.append(g["goal"])
        basis.append("目標")
    mine = [ev for ev in loops if spine.in_scope(ev, group, ids)]
    if mine:
        parts += [ev.body.splitlines()[0] if ev.body else "" for ev in mine]
        basis.append(f"未結 {len(mine)}")
    work = [ev for ev in events if _is_work(ev) and spine.in_scope(ev, group, ids)]
    if work:
        parts += [ev.body for ev in work[-_FOCUS_DECISIONS:]]
        basis.append(f"最近判斷／交接 {min(len(work), _FOCUS_DECISIONS)}")
    return "\n".join(p for p in parts if p), basis


# ---------------------------------------------------------------- 比對

def _idf(docs):
    n = len(docs) or 1
    df = Counter()
    for d in docs:
        df.update(d["tf"].keys())
    return {t: math.log(1 + (n - c + 0.5) / (c + 0.5)) for t, c in df.items()}, n


def _rare_floor(n, share=0.1):
    """「少見詞」＝出現在 ≤10% 文件（至少容許 2 份）。用中位數會在真語料裡只剩只出現一次的詞：
    卡片庫大部分詞都很少見，中位數 idf 貼著最大值（2026-09-25 真資料 1,585 檔實測）。"""
    k = max(2, int(n * share))
    return math.log(1 + (n - k + 0.5) / (k + 0.5))


# 虛字：中文 bigram 跨詞邊界時會切出「接是」「得上」「的起」這種假詞，
# 真資料實測它們足以把不相關的檔湊過門檻 → 含虛字的 bigram 不算共同詞。
_FUNCTION_CHARS = set("的是了不會在要就也和與及把讓得都而並或很這那有我你他它們一上下起個為以於之所被從到對")


def _meaningful(t):
    return not (len(t) == 2 and (t[0] in _FUNCTION_CHARS or t[1] in _FUNCTION_CHARS)) \
        and not (len(t) == 1 and not t.isascii())


def _shared_terms(qcount, doc, idf, floor):
    """共同的少見詞（idf 高於 floor、不含虛字），依 idf 排序——推薦的「為什麼」。"""
    terms = [t for t in qcount if t in doc["tf"] and idf.get(t, 0) >= floor and _meaningful(t)]
    return sorted(terms, key=lambda t: -idf.get(t, 0))


def _event_docs(events, since):
    """上次反芻後新留下的判斷／交接 → [(可比對的 doc, 事件)]；排除本組由呼叫端做。"""
    out = []
    for ev in events:
        if not _is_work(ev) or f"{ev.date} {ev.time}" < since:
            continue
        tf = Counter(_knowledge.tokenize(ev.body or ""))
        out.append(({"kind": "decision", "repo": ev.kv("repo") or ev.kv("group") or "全域",
                    "group": ev.kv("group"), "ref": f"{ev.type}:{ev.kv('id') or ev.time}",
                    "at": f"{ev.date} {ev.time}", "title": (ev.body or "").splitlines()[0][:80],
                    "tf": dict(tf), "len": sum(tf.values()), "htf": {}, "hlen": 0}, ev))
    return out


def run(spine_dir, now=None, window_days=None):
    """跑一輪反芻。回傳 {"groups": {組: [新推薦]}, "since": ..., "candidates": n}。"""
    now = now or _dt.datetime.now()
    cfg = _cfg(spine_dir)
    state = load_state(spine_dir)
    if window_days is not None:
        since_dt = now - _dt.timedelta(days=window_days)
    elif state.get("last_run"):
        since_dt = _dt.datetime.fromisoformat(state["last_run"])
    else:
        since_dt = now - _dt.timedelta(days=cfg["window_days"])
    since_ts, since = since_dt.timestamp(), f"{since_dt:%Y-%m-%d %H:%M}"

    reg = registry.load(spine_dir)
    # 點開頭目錄（.claude／.agents／.github…）是工具設定不是知識；同內容多份只留一份
    # ——兩者都先剔掉再算 idf，不然重複檔會把真正的關鍵詞稀釋成「常見詞」
    all_docs, seen_tf = [], set()
    for d in _knowledge.build_docs(reg["repos"], spine_dir):
        if any(p.startswith(".") for p in d["rel"].split("/")[:-1]):
            continue
        sig = hash(frozenset(d["tf"].items()))
        if sig not in seen_tf:
            seen_tf.add(sig)
            all_docs.append(d)
    events = list(spine.iter_events(spine_dir))
    ev_docs = _event_docs(events, since)
    idf, n_docs = _idf(all_docs + [d for d, _ev in ev_docs])
    floor = _rare_floor(n_docs)
    loops = spine.open_loops(spine_dir)
    known = {(l["group"], l["ref"], l.get("stamp")) for l in state.get("links", [])}

    found = {}
    fresh_files = [d for d in all_docs if d["mtime"] >= since_ts]
    for g in reg["groups"]:
        name, ids = g["name"], set(g.get("members") or [])
        text, basis = focus(spine_dir, name, events, loops)
        qcount = Counter(_knowledge.tokenize(text))
        if not qcount:
            continue
        cands = [dict(d, kind="file", ref=f"{d['repo']}:{d['rel']}",
                      title=d["rel"], stamp=int(d["mtime"]))
                 for d in fresh_files if d["repo"] not in ids]
        cands += [dict(d, stamp=d["at"]) for d, ev in ev_docs
                  if not spine.in_scope(ev, name, ids)]
        if not cands:
            continue
        body = _knowledge._bm25(qcount, cands, "tf", "len")
        head = _knowledge._bm25(qcount, cands, "htf", "hlen")
        scored = []
        for d, sb, sh in zip(cands, body, head):
            shared = _shared_terms(qcount, d, idf, floor)
            if len(shared) < cfg["min_shared"]:
                continue  # 寧缺勿濫：共同少見詞不夠多＝只是剛好用到常見字
            scored.append((sb + 2.0 * sh, d, shared))
        scored.sort(key=lambda x: -x[0])
        picks = []
        for s, d, shared in scored:
            if len(picks) >= cfg["max_per_group"]:
                break
            if (name, d["ref"], d["stamp"]) in known:
                continue
            link = {"id": None, "group": name, "kind": d["kind"], "repo": d["repo"],
                    "ref": d["ref"], "title": d["title"], "stamp": d["stamp"],
                    "score": round(s, 2), "terms": shared[:6], "basis": basis,
                    "found": f"{now:%Y-%m-%d %H:%M}", "status": "new"}
            if d["kind"] == "file":
                link["snippets"] = _knowledge._snippets(d["path"], shared, limit=2)
            picks.append(link)
        if picks:
            found[name] = picks

    with FileLock(Path(spine_dir) / LOCK_NAME):
        state = load_state(spine_dir)
        cutoff = f"{now - _dt.timedelta(days=_KEEP_DAYS):%Y-%m-%d}"
        links = [l for l in state.get("links", []) if l["found"][:10] >= cutoff]
        n = max((int(l["id"][1:]) for l in state.get("links", []) if l.get("id")), default=0)
        for picks in found.values():
            for link in picks:
                n += 1
                link["id"] = f"r{n}"
                links.append(link)
        state.update(last_run=now.isoformat(timespec="seconds"), links=links)
        _save_state(spine_dir, state)
    return {"since": since, "candidates": len(fresh_files), "groups": found}


# ---------------------------------------------------------------- 讀出來用

def _mark_used(spine_dir, links):
    """推薦出現之後，組內有人 read_from 了它（cites.jsonl）＝用上了。"""
    cites = _xref.load_cites(spine_dir)
    members = {g["name"]: set(g.get("members") or []) for g in registry.load(spine_dir)["groups"]}
    for l in links:
        if l["status"] != "new" or l["kind"] != "file":
            continue
        for c in cites:
            if (f"{c.get('provider')}:{c.get('rel')}" == l["ref"]
                    and c.get("consumer") in members.get(l["group"], ())
                    and c.get("at", "") >= l["found"]):
                l["status"], l["used_at"] = "used", c["at"]
                break
    return links


def fresh(spine_dir, group, include_seen=False):
    """某組目前的推薦（新的在前）。include_seen＝連用過／打掉的也列。"""
    links = _mark_used(spine_dir, load_state(spine_dir).get("links", []))
    out = [l for l in links if l["group"] == group
           and (include_seen or l["status"] == "new")]
    return sorted(out, key=lambda l: (l["found"], l["score"]), reverse=True)


def dismiss(spine_dir, link_id):
    with FileLock(Path(spine_dir) / LOCK_NAME):
        state = load_state(spine_dir)
        for l in state.get("links", []):
            if l.get("id") == link_id:
                l["status"] = "dismissed"
                _save_state(spine_dir, state)
                return l
    raise ValueError(f"找不到推薦 {link_id}（ctx fresh --all 看全部）")


def stats(spine_dir):
    """命中率：推薦出去的，後來有多少被讀了。這是閾值的校準資料。"""
    links = _mark_used(spine_dir, load_state(spine_dir).get("links", []))
    c = Counter(l["status"] for l in links)
    files = [l for l in links if l["kind"] == "file"]
    used = sum(1 for l in files if l["status"] == "used")
    return {"total": len(links), "new": c["new"], "used": c["used"],
            "dismissed": c["dismissed"],
            "hit_rate": round(used / len(files), 2) if files else None,
            "last_run": load_state(spine_dir).get("last_run")}


def is_stale(spine_dir, now=None):
    last = load_state(spine_dir).get("last_run")
    if not last:
        return True
    now = now or _dt.datetime.now()
    return now - _dt.datetime.fromisoformat(last) > _dt.timedelta(hours=_STALE_HOURS)


def spawn_detached(spine_dir):
    """開場發現太久沒反芻 → 背景跑，不拖慢開 agent（下一次開場就看得到）。"""
    if os.environ.get("REPOCTX_NO_BACKGROUND"):
        return
    cmd = [sys.executable, "-m", "repo_context", "--spine", str(spine_dir), "fresh", "--run"]
    kw = {"env": dict(os.environ, PYTHONUTF8="1"), "stdout": subprocess.DEVNULL,
          "stderr": subprocess.DEVNULL, "stdin": subprocess.DEVNULL,
          "cwd": str(Path(__file__).resolve().parents[1])}
    if os.name == "nt":
        kw["creationflags"] = 0x00000008 | 0x00000200
    else:
        kw["start_new_session"] = True
    subprocess.Popen(cmd, **kw)


def link_lines(links, limit=3):
    """開場注入／CLI 共用的一行一筆。"""
    out = []
    for l in links[:limit]:
        src = "判斷" if l["kind"] == "decision" else "新料"
        out.append(f"- [{l['id']}] {src} {l['ref']}"
                   + (f"「{l['title']}」" if l["kind"] == "decision" else "")
                   + f"——共同詞：{'、'.join(l['terms'][:5])}")
        for sn in (l.get("snippets") or [])[:1]:
            out.append(f"    L{sn['line']}: {sn['text'][:100]}")
    return out
