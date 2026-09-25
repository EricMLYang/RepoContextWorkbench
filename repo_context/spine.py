"""脊椎（P10 append ＋ P11 query）。

事件標頭嚴格文法（v2 §3.2）：
    ## HH:MM <type> [<source>] key:value ...
- type ∈ 七種；source＝入口（morning-brief/hotkey/manual/engine/...）
- kv token：group:/repo:/ref:/due:/id: 或 #編號（id: 為原型擴充，供 collision 回連）
- 標頭一行機器可解析，區塊內文自由文字（但不得再出現 '## ' 開頭行）
三件套：validator 拒寫｜跨行程 lock（含 registry）｜git 不逐事件 commit（單一提交者＝spine commit 指令）。
拒寫去向：程式入口 raise（agent 自我修正）；人的入口 dead-letter 落檔（想法永不丟）。
"""
import datetime as _dt
import os
import re
import subprocess
from pathlib import Path

from .locking import FileLock

EVENT_TYPES = (
    "presented", "chosen", "decision", "suggestion",
    "collision", "open-loop", "outcome",
)

_HEADER_RE = re.compile(
    r"^## (?P<h>[0-2]\d):(?P<m>[0-5]\d) (?P<type>[a-z-]+) \[(?P<source>[a-z0-9-]+)\](?P<tail>( \S+)*)$"
)
_KV_RE = re.compile(r"^(?:#\d+|(?:group|repo|ref|due|id):\S+)$")
_DUE_RE = re.compile(r"^due:\d{4}-\d{2}-\d{2}$")

LOCK_NAME = ".engine.lock"


class ValidationError(Exception):
    pass


class Event:
    def __init__(self, date, time, type, source, tokens, body):
        self.date = date          # 'YYYY-MM-DD'
        self.time = time          # 'HH:MM'
        self.type = type
        self.source = source
        self.tokens = tokens      # list[str]
        self.body = body          # str（不含標頭）

    def kv(self, key):
        prefix = key + ":"
        for t in self.tokens:
            if t.startswith(prefix):
                return t[len(prefix):]
        return None

    def header(self):
        tail = (" " + " ".join(self.tokens)) if self.tokens else ""
        return f"## {self.time} {self.type} [{self.source}]{tail}"


def parse_header(line, date=""):
    m = _HEADER_RE.match(line)
    if not m:
        raise ValidationError(f"標頭不合文法: {line!r}（需 '## HH:MM <type> [<source>] key:value ...'）")
    if int(m.group("h")) > 23:
        raise ValidationError(f"時間不合法: {line!r}")
    if m.group("type") not in EVENT_TYPES:
        raise ValidationError(f"未知事件型別 {m.group('type')!r}（合法: {', '.join(EVENT_TYPES)}）")
    tokens = m.group("tail").split()
    for t in tokens:
        if not _KV_RE.match(t):
            raise ValidationError(f"kv token 不合法: {t!r}（合法: #編號 或 group:/repo:/ref:/due:/id:）")
        if t.startswith("due:") and not _DUE_RE.match(t):
            raise ValidationError(f"due 需為 YYYY-MM-DD: {t!r}")
    return Event(date, f"{m.group('h')}:{m.group('m')}", m.group("type"), m.group("source"), tokens, "")


def validate_block(header_line, body):
    ev = parse_header(header_line)
    for ln in (body or "").splitlines():
        if ln.startswith("## "):
            raise ValidationError("內文不得含 '## ' 開頭行（會破壞事件邊界）")
    ev.body = (body or "").strip()
    return ev


def _events_dir(spine_dir):
    return Path(spine_dir) / "spine" / "events"


# 管理類留痕（registry／組／關係的設定變更，ops.py 寫的）的 body 前綴：
# 是「改了設定」不是「做了工作」——工作摘要的「上次進度」不拿它充數。
ADMIN_TAG = "〔registry〕"


def append_event(spine_dir, type, source, tokens=None, body="", when=None, dead_letter=False):
    """寫入一筆事件。驗證失敗：dead_letter=True（人的入口）落 dead-letter 檔並回傳 None；
    否則 raise ValidationError（程式/MCP 入口，錯誤原樣給 agent）。回傳 Event。"""
    spine_dir = Path(spine_dir)
    now = when or _dt.datetime.now()
    header = f"## {now:%H:%M} {type} [{source}]" + (
        (" " + " ".join(tokens)) if tokens else "")
    try:
        ev = validate_block(header, body)
    except ValidationError as e:
        if dead_letter:
            _write_dead_letter(spine_dir, header, body, str(e), now)
            return None
        raise
    ev.date = f"{now:%Y-%m-%d}"
    day_file = _events_dir(spine_dir) / f"{ev.date}.md"
    with FileLock(spine_dir / LOCK_NAME):
        day_file.parent.mkdir(parents=True, exist_ok=True)
        with open(day_file, "a", encoding="utf-8", newline="\n") as f:
            f.write(ev.header() + "\n")
            if ev.body:
                f.write(ev.body + "\n")
    return ev


def _write_dead_letter(spine_dir, header, body, error, now):
    dl_dir = Path(spine_dir) / "spine" / "dead-letter"
    dl_dir.mkdir(parents=True, exist_ok=True)
    p = dl_dir / f"{now:%Y-%m-%d-%H%M%S-%f}.md"
    p.write_text(
        f"<!-- validator 拒寫: {error} -->\n{header}\n{body or ''}\n",
        encoding="utf-8")
    return p


def iter_events(spine_dir):
    """讀回全部事件（依檔名日期、檔內順序）。壞行直接 raise——脊椎裡不該有壞行。"""
    d = _events_dir(spine_dir)
    if not d.is_dir():
        return
    for f in sorted(d.glob("*.md")):
        date = f.stem
        cur, body_lines = None, []
        for ln in f.read_text(encoding="utf-8").splitlines():
            if ln.startswith("## "):
                if cur is not None:
                    cur.body = "\n".join(body_lines).strip()
                    yield cur
                cur = parse_header(ln, date)
                body_lines = []
            elif cur is not None and ln.strip() != "---":
                body_lines.append(ln)
        if cur is not None:
            cur.body = "\n".join(body_lines).strip()
            yield cur


def query(spine_dir, type=None, date=None, group=None, since=None):
    out = []
    for ev in iter_events(spine_dir):
        if type and ev.type != type:
            continue
        if date and ev.date != date:
            continue
        if group and ev.kv("group") != group:
            continue
        if since and (ev.date, ev.time) <= since:
            continue
        out.append(ev)
    return out


def in_scope(ev, group=None, ids=None):
    """事件是否屬於某個工作組範圍：group token 相符，或 repo token 在成員內。
    group 與 ids 都沒給＝不限範圍（「全部」）。

    範圍判定只有這一份（2026-09-12 UX 檢討 §6.2：畫面宣告的組別與內容必須一致）——
    情境卡、簡報、工作摘要、會話回報都走這裡，不各自用各自的規則。"""
    if not group and not ids:
        return True
    if group is not None and ev.kv("group") == group:
        return True
    rid = ev.kv("repo")
    return bool(rid and rid in set(ids or []))


def unscoped(ev):
    """既非某組也非某 repo 的事件（全域或未分類）——要保留就獨立標示，不併進某一組。"""
    return not ev.kv("group") and not ev.kv("repo")


def open_loops(spine_dir):
    """未結清單＝算出來的視圖：opened 過、之後沒有 closed 事件 ref 到它。"""
    opened, closed = {}, set()
    for ev in iter_events(spine_dir):
        if ev.type != "open-loop":
            continue
        num = next((t for t in ev.tokens if t.startswith("#")), None)
        if ev.body.startswith("closed") or "closed" in ev.body.split("\n")[0][:12]:
            if num:
                closed.add(num)
        elif num:
            opened[num] = ev
    return [ev for num, ev in opened.items() if num not in closed]


def stats(spine_dir):
    """品味量測最小版（M2 驗收③：靈感命中率要算得出來）。"""
    collisions = set()
    hit = set()
    for ev in iter_events(spine_dir):
        if ev.type == "collision":
            cid = ev.kv("id")
            if cid:
                collisions.add(cid)
        elif ev.type == "outcome":
            ref = ev.kv("ref") or ""
            if ref.startswith("collision:"):
                hit.add(ref[len("collision:"):])
    n, h = len(collisions), len(hit & collisions)
    return {
        "collisions": n,
        "collisions_with_outcome": h,
        "hit_rate": (h / n) if n else None,
    }


def lint(spine_dir, today=None):
    """脊椎衛生迴圈（second-brain 的 /second-brain-lint 課，拆機報告 §5.1：
    知識庫會爛掉，衛生檢查要當一等公民；staleness 做在資料層——Szapar 課 §5.2）。
    回傳紅字清單；跟 registry.audit 同形態，工作台併同一個紅字面呈現。"""
    today = today or f"{_dt.date.today():%Y-%m-%d}"
    reds = []
    events = list(iter_events(spine_dir))
    # 1) ref 斷鏈：ref:<type>:<key> 要解得回既有事件（key＝id: 值或 HH:MM）
    keys = set()
    for ev in events:
        keys.add((ev.type, ev.time))
        eid = ev.kv("id")
        if eid:
            keys.add((ev.type, eid))
    for ev in events:
        ref = ev.kv("ref")
        if not ref or ":" not in ref:
            continue
        rtype, rkey = ref.split(":", 1)
        if (rtype, rkey) not in keys:
            reds.append(f"[ref 斷鏈] {ev.date} {ev.time} {ev.type} "
                        f"ref:{ref} 解不回任何事件")
    # 2) 逾期 open loops（過 due 未 closed＝最該浮上來的陳舊）
    for ev in open_loops(spine_dir):
        due = ev.kv("due")
        if due and due < today:
            num = next((t for t in ev.tokens if t.startswith("#")), "#?")
            first = ev.body.splitlines()[0] if ev.body else ""
            reds.append(f"[open-loop 逾期] {num} due:{due}「{first}」")
    # 3) 碰撞無回程：opened 後判定沒回來（判定分鐘級，隔天還沒回＝detached 進程死了）
    opened, answered = {}, set()
    for ev in events:
        if ev.type != "collision":
            continue
        cid = ev.kv("id")
        if not cid:
            continue
        if ev.body.startswith("opened"):
            opened[cid] = ev
        else:
            answered.add(cid)
    for cid, ev in sorted(opened.items()):
        if cid not in answered and ev.date < today:
            reds.append(f"[碰撞無回程] {cid}：opened 後判定沒回來（想法在，判定丟了）")
    # 4) dead-letter 積壓（拒寫原文等人撿，放著＝想法實質丟了）
    dl = Path(spine_dir) / "spine" / "dead-letter"
    n = len(list(dl.glob("*.md"))) if dl.is_dir() else 0
    if n:
        reds.append(f"[dead-letter 積壓] {n} 件拒寫原文待人工撿回")
    return reds


def default_due_tokens(tokens, body, due_days, when=None):
    """open-loop opened 未帶 due → 補預設到期（§3.4 openloop_default_due_days）。
    closed 事件（狀態變化記新事件）不補。"""
    tokens = list(tokens or [])
    if any(t.startswith("due:") for t in tokens) or (body or "").lstrip().startswith("closed"):
        return tokens
    due = (when or _dt.datetime.now()) + _dt.timedelta(days=due_days)
    return tokens + [f"due:{due:%Y-%m-%d}"]


def batch_commit(spine_dir, message="spine: batch commit"):
    """git 單一提交者的唯一入口（P10 三件套之三）：CLI `commit` 與 S6 timer 都走這裡，
    其他進程只寫檔不碰 git。回傳 True＝有 commit，False＝無變更。"""
    subprocess.run(["git", "add", "-A"], cwd=str(spine_dir), check=True)
    r = subprocess.run(["git", "commit", "-q", "-m", message],
                       cwd=str(spine_dir), capture_output=True, text=True)
    return r.returncode == 0


def last_seen_path(spine_dir):
    return Path(spine_dir) / ".state" / "last_seen.txt"


def get_unread(spine_dir):
    p = last_seen_path(spine_dir)
    since = None
    if p.exists():
        raw = p.read_text(encoding="utf-8").strip()
        if raw:
            since = tuple(raw.split(" ", 1))  # (date, time)
    return query(spine_dir, since=since)


def ack_unread(spine_dir, when=None):
    now = when or _dt.datetime.now()
    p = last_seen_path(spine_dir)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(f"{now:%Y-%m-%d} {now:%H:%M}", encoding="utf-8")
