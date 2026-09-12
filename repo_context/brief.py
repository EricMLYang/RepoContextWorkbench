"""早晨簡報產生器（v2 §4.5 樣貌 A，純事實版、無 LLM）。

三段固定結構：需要你判斷的（≤3 條問句＋出口）／未結 open loops／沉默摘要一行。
v1.1（2026-09-08）：沉默摘要前多一行「脈動」（近 7/30 天 commit 數＋最新一則主旨）——資訊不是問句。
超過 3 條問句＝過濾閾值錯了（照樣印，但標警語——這是免費校準資料）。
產出同時寫 groups/<g>/briefs/ 並落 presented 事件（教練規格：出手即留痕）。
v1.2（2026-09-12 UX 檢討 §6.1／§6.2）：
- 行動文案＝實際行為：defer 一律叫「七天後再提醒」，不叫「本週排進度」（它沒有排任何進度）；
- 未結依所選組過濾（scope_loops），全域／未分類獨立一段——畫面宣告的組別要跟內容一致；
- 沉默摘要只講採集支持得起的話：「未觸發提醒條件」，不宣稱「無變化」。
"""
import datetime as _dt
from pathlib import Path

from . import collect as _collect
from . import config as _config
from . import registry, spine


# 問句種類的人話（§8：qkind 是機器用的 key，不丟給人看）
QKIND_NAME = {"dirty": "未提交檔案放太久", "inactive": "長期沒有 commit",
              "broken": "採集失敗"}


def qkind_name(qkind):
    return QKIND_NAME.get(qkind, qkind)


def _act(label, action, **payload):
    return {"label": label, "action": action, "payload": payload}


def build_question_cards(states, thresholds):
    """問句＝結構化卡（2026-09-02 UI 檢討 P0：出口是按鈕不是字）。
    每張卡：repo／qkind／title（問句）／actions（label＋引擎原語＋payload）。
    文字版（簡報 md、CLI）一律由 card_text 渲染而來，兩邊不會分岔。"""
    cards = []
    for s in states:
        rid = s["id"]
        if s["dirty_days"] is not None and s["dirty_days"] >= thresholds["dirty_stale_days"]:
            cards.append({
                "kind": "question", "qkind": "dirty", "repo": rid,
                "key": f"q:dirty:{rid}",
                "title": f"{rid} 有 {s['dirty']} 個未提交檔案、最舊已 {s['dirty_days']:.0f} 天，"
                         f"本週要處理嗎？",
                "actions": [
                    _act("與 Agent 一起收尾", "term_create", kind="agent", repo=rid,
                         repos=[rid], task=f"收尾 {rid} 的 {s['dirty']} 個未提交檔：先看 git status 與 diff，"
                                          f"判斷該 commit、丟棄或拆開，跟我確認後執行",
                         origin=f"q:dirty:{rid}"),
                    _act("七天後再提醒", "defer", id=rid, qkind="dirty", days=7),
                ]})
        elif (s["tier"] == "active" and s["last_commit_days"] is not None
              and s["last_commit_days"] >= thresholds["inactive_days"]):
            cards.append({
                "kind": "question", "qkind": "inactive", "repo": rid,
                "key": f"q:inactive:{rid}",
                "title": f"{rid} 標記為進行中，但已 {s['last_commit_days']:.0f} 天沒有 commit，"
                         f"還算進行中嗎？",
                "actions": [
                    # 主要動作由「要完成的工作」決定，不是維護登記狀態
                    # （2026-09-12 檢討 §3：降 tier 不該拿到最強的視覺優先權）
                    _act("看這段期間的變化", "term_create", kind="agent", repo=rid,
                         repos=[rid],
                         task=f"{rid} 標記為進行中，但已 {s['last_commit_days']:.0f} 天沒有 commit："
                              f"先看 git log、未合併分支與工作區，說明這段期間實際發生什麼、"
                              f"現在卡在哪，再跟我確認要繼續、暫停還是改標記為休眠",
                         origin=f"q:inactive:{rid}"),
                    _act("改標記為休眠", "tier", id=rid, tier="dormant"),
                    _act("七天後再提醒", "defer", id=rid, qkind="inactive", days=7),
                ]})
        if not s["exists"] or s["note"] == "非 git repo":
            cards.append({
                "kind": "question", "qkind": "broken", "repo": rid,
                "key": f"q:broken:{rid}",
                "title": f"{rid} 採集失敗（{s['note'] or '路徑不存在'}），registry 要修嗎？",
                "actions": [
                    _act("開 Shell 修登記", "term_create", kind="shell", origin=f"q:broken:{rid}"),
                    _act("移出 registry", "remove_repo", id=rid),
                    _act("先略過，七天後再提醒", "defer", id=rid, qkind="broken", days=7),
                ]})
    return cards


def card_text(card):
    """卡 → 一行文字（問句 → 〔出口〕〔出口〕），簡報 md 與 CLI 用。"""
    outs = "".join(f"〔{a['label']}〕" for a in card["actions"])
    return f"{card['title']} → {outs}" if outs else card["title"]


def build_questions(states, thresholds):
    return [card_text(c) for c in build_question_cards(states, thresholds)]


def scope_loops(spine_dir, group_name, group=None, ids=None):
    """未結依工作組分流 → (本組, 全域或未分類)。別組的事項一律不進來。

    2026-09-12 檢討 §6.2：`build_brief()` 直接吃 `spine.open_loops()`，
    「知識研究」的簡報會列出「產品開發」的待辦——介面宣告的組別與內容不一致，
    使用者或 agent 會把別組的任務當成當前責任。範圍判定改走 `spine.in_scope`，
    跟情境卡、工作摘要同一份規則；全域／未分類要保留就獨立標示，不併進本組。"""
    scoped = bool(group_name and group_name != "全部")
    gkey = group or group_name
    ids = list(ids or [])
    own, loose = [], []
    for ev in spine.open_loops(spine_dir):
        if not scoped or spine.in_scope(ev, gkey, ids):
            own.append(ev)
        elif spine.unscoped(ev):
            loose.append(ev)
    return own, loose


def _loop_lines(loops, today):
    """只列到期與新增（其餘不吵人）。"""
    lines = []
    for ev in loops:
        num = next((t for t in ev.tokens if t.startswith("#")), "#?")
        due = ev.kv("due")
        flag = ""
        if due and due <= today:
            flag = "今天到期" if due == today else f"已逾期（{due}）"
        elif ev.date == today:
            flag = "今日新增"
        if flag:
            first = ev.body.splitlines()[0] if ev.body else ""
            lines.append(f"- {num}「{first}」{flag} → 〔開碰撞〕〔延期〕〔關閉〕")
    return lines


def build_brief(spine_dir, group_name, states, when=None, group=None):
    cfg = _config.load(spine_dir)
    th = cfg["thresholds"]
    now = when or _dt.datetime.now()
    cards = build_question_cards(states, th)
    qs = [card_text(c) for c in cards]
    today = f"{now:%Y-%m-%d}"
    ids = [s["id"] for s in states]
    own, loose = scope_loops(spine_dir, group_name, group, ids)
    loop_lines = _loop_lines(own, today)
    loose_lines = _loop_lines(loose, today)
    quiet = len(states) - len({c["repo"] for c in cards})
    lines = [f"# 早晨簡報 {today}（{group_name} 組）", ""]
    lines.append("## 需要你判斷的（每條一個問句）")
    if qs:
        lines += [f"{i}. {q}" for i, q in enumerate(qs, 1)]
        if len(qs) > 3:
            lines.append("")
            lines.append(f"> ⚠ 問句 {len(qs)} 條 > 3——過濾閾值該修了（config.yaml thresholds）")
    else:
        lines.append("（今天沒有需要判斷的事）")
    lines += ["", "## 未結（open loops，本組，只列到期與新增）"]
    lines += loop_lines if loop_lines else ["（無到期或新增）"]
    if loose_lines:
        lines += ["", "## 未結（全域／未分類，不屬於本組，只列到期與新增）"]
        lines += loose_lines
    lines += ["", "## 脈動（一行）",
              _collect.pulse_line(_collect.group_pulse(states))]
    # 沉默摘要只說採集撐得住的話：沒觸發閾值 ≠ 沒有變化
    lines += ["", "## 沉默摘要（一行）",
              f"其餘 {max(quiet, 0)} 個 repo 本次採集未觸發提醒條件。", ""]
    return "\n".join(lines)


def run(spine_dir, group=None, repos=None, when=None):
    now = when or _dt.datetime.now()
    gname, entries = registry.resolve_group(spine_dir, group, repos)
    states = _collect.collect_group(entries)
    text = build_brief(spine_dir, gname, states, now, group=group)
    out = Path(spine_dir) / "groups" / (group or "adhoc") / "briefs" / f"{now:%Y-%m-%d}.md"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(text, encoding="utf-8")
    cfg = _config.load(spine_dir)
    n_q = len(build_question_cards(states, cfg["thresholds"]))
    own, loose = scope_loops(spine_dir, gname, group, [s["id"] for s in states])
    spine.append_event(
        spine_dir, "presented", "morning-brief",
        (["group:" + group] if group else []),
        body=f"呈現了：判斷 {n_q} 條、本組未結 {len(own)} 條"
             + (f"、全域／未分類 {len(loose)} 條" if loose else "")
             + f" → {out.name}",
        when=now)
    return out, text
