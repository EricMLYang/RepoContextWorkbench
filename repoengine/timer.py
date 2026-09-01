"""P12 排程——timer 住殼內（S6）：殼（`ui`／`timer` 子命令）按時呼叫引擎，不用 OS 排程器。

- config.yaml `schedule:` 每項 {task: brief|digest|upstream|commit, at: "HH:MM", group: <組名|省略>}
- 補課規則（檢討④）：錯過排程（睡眠/登出/殼未開）→ 醒來後**當日內補跑一次，跨日不補**
  （實作＝到點後當天只要沒跑過就跑；狀態記 .state/timer.json 的最後執行日）
- 任務失敗必須浮出：落 system-unsure 事件（准打斷類），不准沉默。
"""
import datetime as _dt
import json
import time
from pathlib import Path

from . import digest as _digest
from . import upstream as _upstream
from . import brief as _brief
from . import spine


def _state_path(spine_dir):
    return Path(spine_dir) / ".state" / "timer.json"


def _load_state(spine_dir):
    p = _state_path(spine_dir)
    return json.loads(p.read_text(encoding="utf-8")) if p.exists() else {}


def _save_state(spine_dir, state):
    p = _state_path(spine_dir)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(state, ensure_ascii=False, indent=1), encoding="utf-8")


def _key(entry):
    return f"{entry['task']}@{entry.get('at', '')}@{entry.get('group') or ''}"


def due_entries(schedule, state, now):
    """到點且今天沒跑過的任務（純函數）。跨日不補：只比對「今天」。"""
    today = f"{now:%Y-%m-%d}"
    cur = f"{now:%H:%M}"
    return [e for e in (schedule or [])
            if e.get("at", "99:99") <= cur and state.get(_key(e)) != today]


def run_task(spine_dir, entry):
    task, group = entry["task"], entry.get("group")
    if task == "brief":
        _brief.run(spine_dir, group)
    elif task == "digest":
        _digest.run(spine_dir, group)
    elif task == "upstream":
        _upstream.check(spine_dir, group)
    elif task == "commit":
        spine.batch_commit(spine_dir, "spine: scheduled batch commit")
    else:
        raise ValueError(f"未知排程任務: {task}（brief|digest|upstream|commit）")


def tick(spine_dir, now=None, schedule=None):
    """跑一輪到期任務。回傳 [(key, ok)]。成功失敗都記「今天跑過」——失敗浮出而非重試轟炸。"""
    from . import config as _config
    now = now or _dt.datetime.now()
    if schedule is None:
        schedule = _config.load(spine_dir).get("schedule") or []
    state = _load_state(spine_dir)
    ran = []
    for entry in due_entries(schedule, state, now):
        key = _key(entry)
        try:
            run_task(spine_dir, entry)
            ran.append((key, True))
        except Exception as e:  # 失敗必須浮出，不准沉默
            spine.append_event(spine_dir, "suggestion", "timer", [],
                               body=f"system-unsure：排程任務 {key} 失敗：{e}", when=now)
            ran.append((key, False))
        state[_key(entry)] = f"{now:%Y-%m-%d}"
    if ran:
        _save_state(spine_dir, state)
    return ran


def run_loop(spine_dir, interval=30):
    """殼的常駐引信：每 interval 秒 tick 一次。Ctrl+C 結束。"""
    while True:
        try:
            tick(spine_dir)
        except Exception:
            pass  # tick 內部已把單一任務失敗浮出；這裡只保 loop 不死
        time.sleep(interval)
