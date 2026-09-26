"""回填（2026-09-26）——從過去的 Claude Code 會話紀錄，把判斷／未結／交接撈回 spine。

新 spine 是空的：沒有判斷、沒有未結、沒有交接 → agent 開場接不上、反芻沒有「手上工作」可比。
但使用者跟 agent 過去的會話都在 `~/.claude/projects/*/<session>.jsonl`，判斷與沒做完的事其實都在裡面。

三段，每段都可中斷續跑（結果落 .state/backfill/，已做過的不重做）：
1. scan：會話 → 依紀錄裡的 cwd 對到已登記 repo（Google Drive 舊路徑依資料夾名對）；
   正在寫的會話（30 分鐘內有更新）與對不到 repo 的略過。
2. extract：會話壓成精簡文字（使用者原話＋agent 的文字＋工具一行），切段，每段一次 `claude -p`，
   抽出判斷／待辦／完成／下一步／目標線索（都要標第幾輪，說得出出處）。
3. consolidate：同一 repo 的所有會話依時間排好再彙整一次——後來推翻的判斷、後來做完的待辦剔掉。

**不直接寫進 spine**：結果是候選層，`ctx backfill review` 給人看、`ctx backfill accept` 收。
收進去的事件 source 是 `backfill`、時間是收下的當下，內文註明回填自哪天哪場會話——不假裝是當時寫的。
目標只由使用者給：goal_hints 只列出來參考，不代寫進 registry。

claude -p 在 spine 目錄跑（未登記目錄＝SessionStart／SessionEnd hook 都靜默），
它自己產生的會話紀錄落在 spine 的 project 資料夾，對不到 repo，不會被回填回來。
"""
import datetime as _dt
import json
import os
import re
from collections import Counter
from pathlib import Path

from . import agents as _agents
from . import registry

CHUNK_CHARS = 40_000
_MSG_CHARS = 1_500
_LIVE_MINUTES = 30
_NOISE = re.compile(r"<(system-reminder|local-command-caveat|local-command-stdout|command-name|"
                    r"command-message|command-args|persisted-output)>.*?</\1>", re.S)


def projects_root():
    return Path(os.environ.get("CLAUDE_CONFIG_DIR") or Path.home() / ".claude") / "projects"


def _dir(spine_dir, *parts):
    p = Path(spine_dir) / ".state" / "backfill" / Path(*parts)
    p.mkdir(parents=True, exist_ok=True)
    return p


def _load(p):
    try:
        return json.loads(Path(p).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None


def _save(p, obj):
    tmp = Path(str(p) + ".tmp")
    tmp.write_text(json.dumps(obj, ensure_ascii=False, indent=1), encoding="utf-8")
    os.replace(tmp, p)


# ---------------------------------------------------------------- 1. scan

def _repo_matcher(spine_dir):
    """cwd → repo id：先比已登記路徑（含子目錄），再比路徑上任一段資料夾名＝repo 資料夾名（Drive 舊路徑）。"""
    entries = [(e["id"], Path(e["path"]).expanduser().resolve())
               for e in registry.load(spine_dir)["repos"]]
    by_name = {}
    for rid, p in entries:
        by_name.setdefault(p.name.lower(), rid)
        by_name.setdefault(rid.lower(), rid)

    def match(cwd):
        if not cwd:
            return None
        c = Path(cwd)
        for rid, p in sorted(entries, key=lambda x: -len(str(x[1]))):
            if c == p or p in c.parents:
                return rid
        for part in reversed(c.parts):  # 最深的一段先比：Drive/舊工作區/projects/<repo> → <repo>
            rid = by_name.get(part.lower())
            if rid:
                return rid
        return None
    return match


def _meta(path):
    cwds, first, last, title, n_user = Counter(), None, None, None, 0
    with open(path, encoding="utf-8", errors="replace") as f:
        for ln in f:
            try:
                o = json.loads(ln)
            except ValueError:
                continue
            if o.get("cwd"):
                cwds[o["cwd"]] += 1
            ts = o.get("timestamp")
            if ts:
                first = first or ts
                last = ts
            if o.get("type") == "ai-title" and o.get("aiTitle"):
                title = o["aiTitle"]
            if o.get("type") == "user" and isinstance((o.get("message") or {}).get("content"), str):
                n_user += 1
    return {"cwd": cwds.most_common(1)[0][0] if cwds else None,
            "first": first, "last": last, "title": title, "user_turns": n_user}


def scan(spine_dir, root=None, now=None):
    """回傳 [{sid, path, repo, cwd, first, last, title, user_turns, status}]。
    status：pending／done／live（還在寫）／unmapped（對不到 repo）／empty（沒有使用者發言）。"""
    root = Path(root) if root else projects_root()
    now = now or _dt.datetime.now().timestamp()
    match = _repo_matcher(spine_dir)
    out = []
    for p in sorted(root.glob("*/*.jsonl")):
        m = _meta(p)
        sid = p.stem
        rec = {"sid": sid, "path": str(p), "repo": match(m["cwd"]), **m}
        if now - p.stat().st_mtime < _LIVE_MINUTES * 60:
            rec["status"] = "live"
        elif not rec["repo"]:
            rec["status"] = "unmapped"
        elif not m["user_turns"]:
            rec["status"] = "empty"
        elif _load(_dir(spine_dir, "sessions") / f"{sid}.json"):
            rec["status"] = "done"
        else:
            rec["status"] = "pending"
        out.append(rec)
    return out


# ---------------------------------------------------------------- 2. extract

def _tool_line(block):
    name, inp = block.get("name", "?"), block.get("input") or {}
    detail = (inp.get("description") or inp.get("file_path") or inp.get("pattern")
              or inp.get("command") or inp.get("url") or inp.get("query") or "")
    return f"〔工具 {name}〕{str(detail)[:120]}"


def compact(path):
    """會話 → 精簡文字行：[tN] 使用者原話／agent 文字／工具一行。tool_result、thinking、系統標記全部丟掉。"""
    lines, turn = [], 0
    with open(path, encoding="utf-8", errors="replace") as f:
        for ln in f:
            try:
                o = json.loads(ln)
            except ValueError:
                continue
            if o.get("isSidechain"):
                continue
            msg = o.get("message") or {}
            content = msg.get("content")
            if o.get("type") == "user" and isinstance(content, str):
                text = _NOISE.sub("", content).strip()
                if not text or text.startswith("<"):
                    continue
                turn += 1
                lines.append(f"[t{turn}] 【使用者】{text[:_MSG_CHARS]}")
            elif o.get("type") == "assistant" and isinstance(content, list):
                for b in content:
                    if b.get("type") == "text" and b.get("text", "").strip():
                        lines.append(f"【agent】{b['text'].strip()[:_MSG_CHARS]}")
                    elif b.get("type") == "tool_use":
                        lines.append(_tool_line(b))
    return lines


def chunks(lines, size=CHUNK_CHARS):
    out, cur, n = [], [], 0
    for ln in lines:
        if cur and n + len(ln) > size:
            out.append("\n".join(cur))
            cur, n = [], 0
        cur.append(ln)
        n += len(ln) + 1
    if cur:
        out.append("\n".join(cur))
    return out


EXTRACT_PROMPT = """你在整理一場「使用者 × coding agent」的過往會話紀錄（repo：{repo}，{date}，第 {i}/{n} 段）。
目的：把值得留下的東西撈出來，讓之後接手的 agent 知道做過什麼判斷、還剩什麼。
不要使用任何工具，只根據下面的紀錄回答。只輸出一個 JSON 物件，不要其他文字。

規則：
- 判斷（decisions）：使用者明確做出、或使用者同意採納的結論／取捨（選 A 不選 B、定規則、定方向）。
  agent 自己的推測、還在討論沒定案的、純操作步驟都不算。每筆附依據與出處輪次（[tN]）。
- 待辦（todos）：這段裡明確提到要做、但到這段結尾還沒做完的事。已經做完的不要列。
- 完成（done）：這段實際完成的事（一句一件，最多 5 件）。
- 下一步（next）：{next_rule}
- 目標線索（goal_hints）：使用者用自己的話說出的這個 repo 的目的或動機（盡量原話）；操作指令與提問句不算。
- 全部用繁體中文，每項 ≤120 字；不確定就不要列，不要編造。decisions ≤8、todos ≤6。

JSON 形狀：
{{"summary": "這段在做什麼（一句）",
  "decisions": [{{"text": "結論", "basis": "為什麼", "turn": "t12"}}],
  "todos": [{{"text": "要做的事", "turn": "t20"}}],
  "done": ["…"], "next": "…", "goal_hints": ["…"]}}

=== 會話紀錄 ===
{body}
"""


def _parse_json(text):
    m = re.search(r"\{.*\}", text or "", re.S)
    if not m:
        raise ValueError("輸出裡找不到 JSON")
    obj = json.loads(m.group(0))
    if not isinstance(obj, dict):
        raise ValueError("輸出不是 JSON 物件")
    for k in ("decisions", "todos", "done", "goal_hints"):
        v = obj.get(k) or []
        obj[k] = v if isinstance(v, list) else []
    obj["decisions"] = [d for d in obj["decisions"] if isinstance(d, dict) and str(d.get("text", "")).strip()]
    obj["todos"] = [t for t in obj["todos"] if isinstance(t, dict) and str(t.get("text", "")).strip()]
    obj["summary"] = str(obj.get("summary") or "")
    obj["next"] = str(obj.get("next") or "")
    return obj


def claude_caller(spine_dir, model="sonnet"):
    """預設的模型呼叫：claude -p，在 spine 目錄、唯讀工具。回傳 call(prompt) -> str。"""
    provider = _agents.ClaudeProvider()

    def call(prompt):
        return provider._call(prompt, spine_dir, model=model, cwd=spine_dir, read_only=True)
    return call


def _ask_json(call, prompt):
    last = None
    for _ in (1, 2):  # 解析失敗重試一次；仍壞就浮出，不吞
        try:
            return _parse_json(call(prompt))
        except _agents.AgentUnsure:
            raise
        except (ValueError, json.JSONDecodeError) as e:
            last = e
    raise _agents.AgentUnsure(f"輸出解析兩次皆失敗：{last}")


def extract(spine_dir, rec, call):
    """一場會話 → sessions/<sid>.json。回傳結果 dict。"""
    parts = chunks(compact(rec["path"]))
    date = (rec.get("first") or "")[:10]
    results = []
    for i, body in enumerate(parts, 1):
        prompt = EXTRACT_PROMPT.format(
            repo=rec["repo"], date=date, i=i, n=len(parts), body=body,
            next_rule=("這是最後一段：會話結束時，下次該從哪裡接（一句）；看不出來就空字串。"
                       if i == len(parts) else "不是最後一段，留空字串。"))
        results.append(_ask_json(call, prompt))
    out = {"sid": rec["sid"], "repo": rec["repo"], "cwd": rec.get("cwd"),
           "first": rec.get("first"), "last": rec.get("last"), "title": rec.get("title"),
           "chunks": len(parts), "extracted_at": _dt.datetime.now().isoformat(timespec="seconds"),
           "parts": results}
    _save(_dir(spine_dir, "sessions") / f"{rec['sid']}.json", out)
    return out


# ---------------------------------------------------------------- 3. consolidate

CONSOLIDATE_PROMPT = """下面是 repo「{repo}」過去 {n} 場 coding agent 會話的逐場整理，依時間由舊到新。
請彙整成「現在接手這個 repo 的人需要知道的」。不要使用任何工具。只輸出一個 JSON 物件。

規則：
- decisions：至今仍有效、會影響之後怎麼做的判斷，最多 15 筆，重要的在前。
  後來被推翻或取代的不要列（或只列最新版本）；同一件事多次出現合成一筆。每筆標來源會話（sid 前 8 碼）與日期。
- open_todos：到最後一場為止看起來仍沒做完的待辦，最多 10 筆。後面會話已完成的剔掉；看不出有沒有做完的，標 uncertain=true。
- handoff：最近一場會話結束時的狀態——done（完成了什麼）、next（下次從哪接）、date、sid。
- goal_hints：使用者說明這個 repo「為什麼存在、要達成什麼」的句子，最多 3 句（盡量原話），附 sid。
  操作指令（例：commit push、幫我改 X）與提問句不算目標線索。
- 繁體中文，每項 ≤150 字。不確定就不列，不要編造。

JSON 形狀：
{{"decisions": [{{"text": "…", "basis": "…", "sid": "1a2b3c4d", "date": "2026-09-11"}}],
  "open_todos": [{{"text": "…", "sid": "…", "date": "…", "uncertain": false}}],
  "handoff": {{"done": "…", "next": "…", "sid": "…", "date": "…"}},
  "goal_hints": [{{"text": "…", "sid": "…"}}]}}

=== 逐場整理 ===
{body}
"""


def _session_digest(s):
    lines = [f"## 會話 {s['sid'][:8]}（{(s.get('first') or '')[:10]}）{s.get('title') or ''}"]
    for p in s["parts"]:
        if p.get("summary"):
            lines.append(f"摘要：{p['summary']}")
        for d in p["decisions"]:
            lines.append(f"判斷：{d['text']}｜依據：{d.get('basis', '')}")
        for t in p["todos"]:
            lines.append(f"待辦：{t['text']}")
        for x in p["done"]:
            lines.append(f"完成：{x}")
        if p.get("next"):
            lines.append(f"下一步：{p['next']}")
        for g in p["goal_hints"]:
            lines.append(f"目標線索：{g}")
    return "\n".join(lines)


def consolidate(spine_dir, repo, call):
    sessions = [s for s in (_load(p) for p in _dir(spine_dir, "sessions").glob("*.json"))
                if s and s.get("repo") == repo]
    if not sessions:
        return None
    sessions.sort(key=lambda s: s.get("first") or "")
    body = "\n\n".join(_session_digest(s) for s in sessions)
    if len(body) > 120_000:  # 太長：留最新的（彙整最在意的是現況）
        body = "…（較舊的會話省略）\n" + body[-120_000:]
    obj = _ask_json(call, CONSOLIDATE_PROMPT.format(repo=repo, n=len(sessions), body=body))
    obj["decisions"] = [dict(d, id=f"d{i}") for i, d in enumerate(obj.get("decisions") or [], 1)
                        if isinstance(d, dict) and d.get("text")]
    obj["open_todos"] = [dict(t, id=f"t{i}") for i, t in enumerate(obj.get("open_todos") or [], 1)
                         if isinstance(t, dict) and t.get("text")]
    ho = obj.get("handoff")
    obj["handoff"] = dict(ho, id="h") if isinstance(ho, dict) and (ho.get("done") or ho.get("next")) else None
    prev = _load(_dir(spine_dir, "repos") / f"{repo}.json") or {}
    out = {"repo": repo, "sessions": [s["sid"] for s in sessions],
           "consolidated_at": _dt.datetime.now().isoformat(timespec="seconds"),
           "accepted": prev.get("accepted", []) if prev.get("sessions") == [s["sid"] for s in sessions] else [],
           **{k: obj.get(k) for k in ("decisions", "open_todos", "handoff", "goal_hints")}}
    _save(_dir(spine_dir, "repos") / f"{repo}.json", out)
    return out


def _each(fn, items, jobs):
    if jobs <= 1:
        for a, b in items:
            fn(a, b)
        return
    from concurrent.futures import ThreadPoolExecutor
    with ThreadPoolExecutor(max_workers=jobs) as ex:
        list(ex.map(lambda ab: fn(*ab), items))


def run(spine_dir, call=None, root=None, limit=None, repo=None, log=print, jobs=1):
    """抽取所有 pending 會話，再彙整有新會話的 repo。單場失敗記下來繼續（整晚跑不能一場壞就停）。
    jobs＞1：同時跑幾個模型呼叫（時間幾乎都在等模型；每場結果各寫各的檔，彼此不衝突）。"""
    call = call or claude_caller(spine_dir)
    scanned = scan(spine_dir, root)
    # 歸屬改了（例：後來才登記更精確的子專案）：已抽的結果改掛新 repo，新舊兩邊都重彙整，不重抽
    remapped = set()
    for r in scanned:
        if r["status"] != "done":
            continue
        sp = _dir(spine_dir, "sessions") / f"{r['sid']}.json"
        s = _load(sp)
        if s and s.get("repo") != r["repo"]:
            remapped |= {s.get("repo"), r["repo"]}
            s["repo"] = r["repo"]
            _save(sp, s)
    recs = [r for r in scanned if r["status"] == "pending"
            and (repo is None or r["repo"] == repo)]
    recs.sort(key=lambda r: r.get("first") or "")
    if limit:
        recs = recs[:limit]
    touched, failed = set(), []

    def one(i, r):
        log(f"[{i}/{len(recs)}] {r['repo']} {r['sid'][:8]} {(r.get('first') or '')[:10]} "
            f"{r.get('title') or ''}")
        try:
            extract(spine_dir, r, call)
            touched.add(r["repo"])
        except Exception as e:  # 浮出、記下、繼續
            failed.append({"sid": r["sid"], "repo": r["repo"], "error": str(e)[:300]})
            log(f"  失敗：{r['sid'][:8]} {str(e)[:200]}")

    _each(one, list(enumerate(recs, 1)), jobs)
    # 要重彙整的 repo 從「狀態」推，不從「這輪發生了什麼」推——中途被殺過也能自癒：
    # 候選記的會話 ≠ 實際歸這個 repo 的會話 → 重彙整；一場都不剩 → 候選清掉（還沒收的不能留著誤導）
    actual = {}
    for sp in _dir(spine_dir, "sessions").glob("*.json"):
        s = _load(sp)
        if s and s.get("repo"):
            actual.setdefault(s["repo"], set()).add(s["sid"])
    need = set(touched) | (remapped & set(actual))
    for cp in _dir(spine_dir, "repos").glob("*.json"):
        c = _load(cp) or {}
        if cp.stem not in actual:
            cp.unlink(missing_ok=True)
        elif set(c.get("sessions") or []) != actual[cp.stem]:
            need.add(cp.stem)
    need |= {r for r in actual if not (_dir(spine_dir, "repos") / f"{r}.json").exists()}
    if repo:
        need &= {repo}
    def merge(_, rid):
        log(f"彙整 {rid}")
        try:
            consolidate(spine_dir, rid, call)
        except Exception as e:
            failed.append({"repo": rid, "stage": "consolidate", "error": str(e)[:300]})
            log(f"  失敗：{rid} {str(e)[:200]}")

    _each(merge, [(None, rid) for rid in sorted(need)], jobs)
    _save(_dir(spine_dir) / "last_run.json",
          {"at": _dt.datetime.now().isoformat(timespec="seconds"),
           "extracted": len(recs) - len([f for f in failed if "stage" not in f]),
           "consolidated": sorted(need), "failed": failed})
    return {"extracted": len(recs), "consolidated": sorted(need), "failed": failed}


# ---------------------------------------------------------------- 4. review ／ accept

def candidates(spine_dir, repo=None):
    out = []
    for p in sorted(_dir(spine_dir, "repos").glob("*.json")):
        c = _load(p)
        if c and (repo is None or c["repo"] == repo):
            out.append(c)
    return out


def review_text(spine_dir, repo=None):
    out = ["# 回填候選（從過去的 Claude Code 會話撈出來的；還沒寫進 spine）",
           "收：`ctx backfill accept <repo> d1 d3 t2 h`（或 `ctx backfill accept <repo> all`）。目標只列線索，請自己寫 `goal`。", ""]
    for c in candidates(spine_dir, repo):
        acc = set(c.get("accepted") or [])
        mark = lambda i: "✔ 已收 " if i in acc else ""
        out.append(f"## {c['repo']}（{len(c['sessions'])} 場會話）")
        if c.get("goal_hints"):
            out.append("目標線索：")
            out += [f"- 「{g.get('text') if isinstance(g, dict) else g}」" for g in c["goal_hints"]]
        if c.get("handoff"):
            h = c["handoff"]
            out.append(f"[h] {mark('h')}最後交接（{h.get('date', '')}）：{h.get('done', '')}"
                       f" → 下次：{h.get('next', '')}")
        for d in c.get("decisions") or []:
            out.append(f"[{d['id']}] {mark(d['id'])}判斷（{d.get('date', '')}）：{d['text']}"
                       + (f"｜依據：{d['basis']}" if d.get("basis") else ""))
        for t in c.get("open_todos") or []:
            out.append(f"[{t['id']}] {mark(t['id'])}待辦（{t.get('date', '')}）：{t['text']}"
                       + ("（不確定是否已做完）" if t.get("uncertain") else ""))
        out.append("")
    if len(out) == 3:
        out.append("還沒有候選——先跑 `ctx backfill run`。")
    return "\n".join(out)


def accept(spine_dir, repo, ids):
    """把勾選的候選寫進 spine（source=backfill）。回傳寫入的 ref 清單。已收過的略過。"""
    from . import agentapi as _api
    p = _dir(spine_dir, "repos") / f"{repo}.json"
    c = _load(p)
    if not c:
        raise ValueError(f"{repo} 沒有回填候選（先跑 ctx backfill run）")
    path = registry.get_repo(spine_dir, repo)["path"]
    items = {d["id"]: ("d", d) for d in c.get("decisions") or []}
    items.update({t["id"]: ("t", t) for t in c.get("open_todos") or []})
    if c.get("handoff"):
        items["h"] = ("h", c["handoff"])
    want = list(items) if ids in (["all"], "all") else ids
    unknown = [i for i in want if i not in items]
    if unknown:
        raise ValueError(f"沒有這些候選：{', '.join(unknown)}（ctx backfill review 看編號）")
    accepted, written = set(c.get("accepted") or []), []
    # 交接最後寫：它是「上次做到哪」，要是最新一筆
    for i in sorted(want, key=lambda x: (x == "h", x)):
        if i in accepted:
            continue
        kind, it = items[i]
        src = f"（回填自 {it.get('date') or '?'} 會話 {str(it.get('sid') or '?')[:8]}）"
        if kind == "d":
            body = it["text"] + (f"\n依據：{it['basis']}" if it.get("basis") else "") + f"\n{src}"
            r = _api.log_decision(spine_dir, body, cwd=path, source="backfill")
            written.append(r["ref"])
        elif kind == "t":
            r = _api.add_todo(spine_dir, f"{it['text']} {src}", cwd=path, source="backfill")
            written.append(r["id"])
        else:
            r = _api.handoff(spine_dir, it.get("done") or "（未記錄）",
                             f"{it.get('next') or '（未記錄）'} {src}", cwd=path, source="backfill")
            written.append(r["ref"])
        accepted.add(i)
    c["accepted"] = sorted(accepted)
    _save(p, c)
    return written
