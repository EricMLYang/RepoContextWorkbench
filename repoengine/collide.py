"""P7 撞 collide——兩段式（v2 檢討④）：

段一 submit：想法原文即刻落脊椎（collision opened，判定掛了想法也不丟），
段二 run：pack 組文件層 → provider 判定（五欄位）→ 結果落脊椎（unread 浮出）。
--wait 同步跑段二；--detach spawn detached 進程（不掛呼叫者進程樹）；
判定失敗 → 落 system-unsure 事件（准打斷類，不准沉默）。
P7 不准跑在殼的 thread 裡——引擎這邊以獨立進程支持這條。
"""
import datetime as _dt
import os
import string
import subprocess
import sys
from pathlib import Path

from . import config as _config
from . import pack as _pack
from . import registry, spine
from .agents import AgentUnsure, judge

PROMPT_TEMPLATE = """你是「碰撞判定器」。判斷【想法】相對【組內容】是：已知（組內已有）、衝突（與組內既有結論矛盾）、還是真增量（新的、值得落地）。

只輸出一個 JSON 物件，欄位固定：
{{"判定": "已知|衝突|真增量", "理由": "一句話", "證據": "哪個 repo 哪份文件的哪段（已知/衝突必填實引；真增量寫『組內無相關段』並點名最接近的文件）", "落點建議": "repo:<id>/01_inbox | group:<g>/materials | incubator", "下一步": "一個具體動作"}}

【想法】
{idea}

【組內容（文件層選料，未納入清單見 pack 標頭）】
{packed}
"""


def _next_collision_id(spine_dir, date_str):
    used = set()
    for ev in spine.iter_events(spine_dir):
        cid = ev.kv("id")
        if ev.type == "collision" and cid and cid.startswith(date_str):
            used.add(cid)
    for c in string.ascii_lowercase:
        cid = f"{date_str}-{c}"
        if cid not in used:
            return cid
    return f"{date_str}-z{len(used)}"


def submit(spine_dir, idea, group=None, repos=None, source="hotkey", when=None):
    """段一：想法落脊椎。回傳 collision id。
    repos 給定＝臨時組合（P2 免建組）：token 落 group:臨時(a,b)（§3.2 文法範例），
    detached 進程重讀事件即可解回選料範圍。"""
    now = when or _dt.datetime.now()
    cid = _next_collision_id(spine_dir, f"{now:%Y-%m-%d}")
    if repos:
        ids = [r.strip() for r in repos.split(",")] if isinstance(repos, str) else list(repos)
        group = f"臨時({','.join(ids)})"
    tokens = [f"id:{cid}"] + ([f"group:{group}"] if group else [])
    spine.append_event(spine_dir, "collision", source, tokens,
                       body=f"opened\n輸入：{idea}", when=now)
    return cid


def run_judgement(spine_dir, cid, group=None, repos=None, provider=None, when=None):
    """段二：判定並落脊椎。回傳判定 dict（或 system-unsure 時 None）。"""
    now = when or _dt.datetime.now()
    cfg = _config.load(spine_dir)
    provider = provider or cfg["provider"]["default"]
    idea = None
    for ev in spine.iter_events(spine_dir):
        if ev.type == "collision" and ev.kv("id") == cid and ev.body.startswith("opened"):
            idea = ev.body.split("輸入：", 1)[-1].strip()
            group = group or ev.kv("group")
    if idea is None:
        raise ValueError(f"找不到 collision opened 事件: {cid}")
    prefix = "臨時("
    if repos is None and group and group.startswith(prefix) and group.endswith(")"):
        repos = group[len(prefix):-1]   # 臨時組合：從事件 token 解回 repo 清單
        group = None
    _, entries = registry.resolve_group(spine_dir, group, repos)
    packed, _, _ = _pack.pack_group(entries, cfg["pack"]["token_budget"])
    prompt = PROMPT_TEMPLATE.format(idea=idea, packed=packed)
    try:
        j = judge(provider, prompt, spine_dir,
                  model=cfg["provider"]["models"].get("collide"))
    except AgentUnsure as e:
        spine.append_event(
            spine_dir, "collision", "engine", [f"id:{cid}", f"ref:collision:{cid}"],
            body=f"判定失敗（system-unsure，准打斷）：{e}", when=now)
        return None
    body = (f"判定：{j['判定']}\n理由：{j['理由']}\n證據：{j['證據']}\n"
            f"落點建議：{j['落點建議']}\n下一步：{j['下一步']}\n"
            f"〔照建議落〕〔改落點〕〔升格 incubator〕〔深撞：開 agent 視窗〕〔丟棄並記錄〕")
    spine.append_event(spine_dir, "collision", "engine",
                       [f"id:{cid}", f"ref:collision:{cid}"], body=body, when=now)
    return j


def spawn_detached(spine_dir, cid, group=None, provider=None):
    """段二丟 detached 進程（Windows：DETACHED_PROCESS｜其他：start_new_session）。"""
    cmd = [sys.executable, "-m", "repoengine", "--spine", str(spine_dir),
           "collide", "run", cid]
    if group:
        cmd += ["--group", group]
    if provider:
        cmd += ["--provider", provider]
    env = dict(os.environ, PYTHONUTF8="1")
    kw = {"env": env, "stdout": subprocess.DEVNULL, "stderr": subprocess.DEVNULL,
          "stdin": subprocess.DEVNULL, "cwd": str(Path(__file__).resolve().parents[1])}
    if os.name == "nt":
        kw["creationflags"] = 0x00000008 | 0x00000200  # DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    else:
        kw["start_new_session"] = True
    subprocess.Popen(cmd, **kw)
