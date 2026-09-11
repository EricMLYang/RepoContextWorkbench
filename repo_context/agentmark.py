"""工作台 agent 會話的存活標記——P3「agent 偵測」的資料源。

抄 gitpane（拆機報告 §2.1）：11 個外部工具裡唯一感知「這個 repo 上有 agent
正在跑」的就是它。原型版不掃全系統進程，只標引擎自己 spawn 的內嵌會話：
- 工作台開 agent 會話時 mark、殺會話/進程結束時 unmark
- marker 落 spine/.state/agent_sessions/<sid>.json（.state/ 已 gitignore，不進 git）
- 讀取端以 pid 活性掃；pid 已死的 marker 視為殘骸就地清掉——工作台被硬殺不留幽靈
- P16 外開終端的會話生命週期不歸引擎管，不標（誠實邊界）
"""
import json
import os
from pathlib import Path


def _dir(spine_dir):
    return Path(spine_dir) / ".state" / "agent_sessions"


def mark(spine_dir, sid, cwd, agent, pid):
    d = _dir(spine_dir)
    d.mkdir(parents=True, exist_ok=True)
    (d / f"{sid}.json").write_text(
        json.dumps({"sid": sid, "cwd": str(cwd), "agent": agent, "pid": pid},
                   ensure_ascii=False), encoding="utf-8")


def unmark(spine_dir, sid):
    try:
        (_dir(spine_dir) / f"{sid}.json").unlink()
    except OSError:
        pass


def _pid_alive(pid):
    if pid is None or pid <= 0:
        return False
    if os.name == "nt":
        # 不可用 os.kill(pid, 0)——Windows 上任意 sig 都走 TerminateProcess！
        import ctypes
        PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
        STILL_ACTIVE = 259
        k32 = ctypes.windll.kernel32
        h = k32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, int(pid))
        if not h:
            return False
        code = ctypes.c_ulong()
        ok = k32.GetExitCodeProcess(h, ctypes.byref(code))
        k32.CloseHandle(h)
        return bool(ok) and code.value == STILL_ACTIVE
    try:
        os.kill(int(pid), 0)
    except ProcessLookupError:
        return False
    except OSError:
        return True  # 沒權限查＝pid 存在
    return True


def live(spine_dir):
    """存活的 agent 會話 [{sid, cwd, agent, pid}]；順手清掉 pid 已死的殘骸。"""
    d = _dir(spine_dir)
    if not d.is_dir():
        return []
    out = []
    for p in sorted(d.glob("*.json")):
        try:
            m = json.loads(p.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        if not _pid_alive(m.get("pid")):
            try:
                p.unlink()
            except OSError:
                pass
            continue
        out.append(m)
    return out
