"""監控台雛形驗證：L1 狀態函數 ＋ L2 HTTP E2E（thread 起真 server，urllib 打）。"""
import datetime as dt
import json
import threading
import urllib.request

import pytest

from repoengine import spine as spine_mod
from repoengine import webui


def _t(h, m):
    return dt.datetime.now().replace(hour=h, minute=m)


# ── L1：狀態函數 ─────────────────────────────────────────────

def test_build_state_fields(spine_with_repos):
    spine_mod.append_event(spine_with_repos, "open-loop", "hotkey", ["#1"],
                           body="opened →「試」", when=_t(9, 0))
    st = webui.build_state(spine_with_repos)
    assert [g["name"] for g in st["groups"]] == ["g1"]
    assert st["groups"][0]["members"] == ["repo-a", "repo-b"]
    assert set(st["repos"]) == {"repo-a", "repo-b"}
    assert len(st["unread"]) == 1 and st["unread"][0]["type"] == "open-loop"
    assert "interrupt" not in st["unread"][0]  # 一般事件不帶打斷標記
    assert len(st["loops"]) == 1
    assert set(st["stats"]) == {"collisions", "collisions_with_outcome", "hit_rate",
                                "spawned", "alive", "survival_rate"}


def test_build_state_interrupt_first(spine_with_repos):
    spine_mod.append_event(spine_with_repos, "decision", "manual", [],
                           body="一般", when=_t(9, 0))
    spine_mod.append_event(spine_with_repos, "suggestion", "timer", [],
                           body="system-unsure：任務掛了", when=_t(9, 1))
    st = webui.build_state(spine_with_repos)
    assert st["unread"][0]["interrupt"] is True   # interrupt 置頂
    assert st["unread"][0]["source"] == "timer"
    assert "interrupt" not in st["unread"][1]


def test_app_without_pywebview_exits_with_hint(spine, monkeypatch):
    """桌面視窗模式缺 pywebview 時要講人話退場（附安裝指令），不是 traceback。"""
    import builtins
    real_import = builtins.__import__

    def fake(name, *a, **kw):
        if name == "webview":
            raise ImportError("No module named 'webview'")
        return real_import(name, *a, **kw)
    monkeypatch.setattr(builtins, "__import__", fake)
    with pytest.raises(SystemExit, match="pywebview"):
        webui.serve_window(spine)


def test_build_scan_fields(spine_with_repos):
    sc = webui.build_scan(spine_with_repos, "g1")
    assert sc["group"] == "g1"
    assert {s["id"] for s in sc["states"]} == {"repo-a", "repo-b"}
    assert any("repo-b" in q for q in sc["questions"])  # dirty 5 天 → 問句
    sc_all = webui.build_scan(spine_with_repos, None)   # 無組＝全部
    assert sc_all["group"] == "全部" and len(sc_all["states"]) == 2


# ── L2：HTTP E2E ────────────────────────────────────────────

_TOKEN = ""


@pytest.fixture
def server(spine_with_repos):
    global _TOKEN
    srv = webui.make_server(spine_with_repos, port=0)
    _TOKEN = srv.token
    th = threading.Thread(target=srv.serve_forever, daemon=True)
    th.start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"
    yield base, spine_with_repos
    srv.terms.kill_all()
    srv.shutdown()
    srv.server_close()


def _get(base, path):
    req = urllib.request.Request(base + path, headers={"X-Auth": _TOKEN})
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            return r.status, r.read().decode("utf-8")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8")


def _post(base, path, payload):
    req = urllib.request.Request(
        base + path, data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json", "X-Auth": _TOKEN},
        method="POST")
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            return r.status, json.loads(r.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read().decode("utf-8"))


@pytest.mark.e2e
def test_inv_binds_localhost_only(server):
    base, _ = server
    assert "127.0.0.1" in base  # make_server 寫死 127.0.0.1，不對外暴露


@pytest.mark.e2e
def test_inv_requires_token(server):
    """VDI 多使用者防護：沒 token 一律 403（/static 除外——只是前端函式庫）。"""
    base, _ = server
    for path in ("/", "/api/state", "/api/scan"):
        try:
            with urllib.request.urlopen(base + path, timeout=10) as r:
                code = r.status
        except urllib.error.HTTPError as e:
            code = e.code
        assert code == 403, path
    code, _body = _get(base, "/static/xterm.css")  # 帶不帶 token 都可
    assert code == 200


@pytest.mark.e2e
def test_page_and_state(server):
    base, _ = server
    code, html = _get(base, "/")
    assert code == 200
    for key in ("工作台", "碰撞台", "收件匣", "需要你判斷", "深看",
                "牌", "臨時組", "會話", "終端", "存成今日簡報", "一切正常"):
        assert key in html
    code, body = _get(base, "/api/state")
    st = json.loads(body)
    assert code == 200 and st["groups"][0]["name"] == "g1"
    assert st["terms"] == []  # 終端清冊隨 state 輪詢
    code, body = _get(base, "/api/scan?group=g1")
    assert code == 200 and json.loads(body)["group"] == "g1"


@pytest.mark.e2e
def test_brief_via_api(server):
    base, spine_dir = server
    code, r = _post(base, "/api/brief", {"group": "g1"})
    assert code == 200 and "早晨簡報" in r["text"] and "沉默摘要" in r["text"]
    assert spine_mod.query(spine_dir, type="presented")  # presented 有留痕


@pytest.mark.e2e
def test_collide_via_api(server):
    base, spine_dir = server
    code, r = _post(base, "/api/collide",
                    {"idea": "從監控台丟的想法", "group": "g1", "wait": True})
    assert code == 200 and r["ok"] and r["judgement"]["判定"]
    evs = spine_mod.query(spine_dir, type="collision")
    assert len(evs) == 2  # opened + 判定結果
    assert "從監控台丟的想法" in evs[0].body
    code, r = _post(base, "/api/collide", {"idea": "  "})
    assert code == 400 and "error" in r  # 空想法拒收


@pytest.mark.e2e
def test_ignore_writes_chosen(server):
    base, spine_dir = server
    spine_mod.append_event(spine_dir, "suggestion", "engine",
                           body="upstream 有新 release", when=_t(8, 0))
    code, r = _post(base, "/api/ignore",
                    {"type": "suggestion", "time": "08:00",
                     "header": "## 08:00 suggestion [engine]"})
    assert code == 200 and r["ok"]
    chosen = spine_mod.query(spine_dir, type="chosen")
    assert len(chosen) == 1
    assert chosen[0].kv("ref") == "suggestion:08:00"
    assert "忽略並記錄" in chosen[0].body  # 忽略＝chosen 的負形，有留痕


@pytest.mark.e2e
def test_close_loop_and_ack(server):
    base, spine_dir = server
    # 事件時間要早於 ack 時刻，否則 ack 不掉（曾在 09:00 前跑出 flaky）
    spine_mod.append_event(spine_dir, "open-loop", "hotkey", ["#7"],
                           body="opened →「待辦」",
                           when=dt.datetime.now() - dt.timedelta(minutes=1))
    assert len(spine_mod.open_loops(spine_dir)) == 1
    code, r = _post(base, "/api/close_loop", {"num": "#7"})
    assert code == 200 and not spine_mod.open_loops(spine_dir)
    code, r = _post(base, "/api/close_loop", {"num": "７；rm -rf"})
    assert code == 400  # 編號不合法拒收
    _post(base, "/api/ack", {})
    assert spine_mod.get_unread(spine_dir) == []


@pytest.mark.e2e
def test_unknown_routes(server):
    base, _ = server
    code, _body = _get(base, "/api/nope")
    assert code == 404
    code, r = _post(base, "/api/nope", {})
    assert code == 400 and "未知 action" in r["error"]


@pytest.mark.e2e
def test_scan_with_repos_subset(server):
    """勾選＝臨時組合：scan 只回選取的 repo，問句同步過濾。"""
    base, _ = server
    code, body = _get(base, "/api/scan?repos=repo-a")
    sc = json.loads(body)
    assert code == 200
    assert [s["id"] for s in sc["states"]] == ["repo-a"]
    assert sc["group"].startswith("臨時(")
    assert not any("repo-b" in q for q in sc["questions"])  # dirty 的 repo-b 被排除


@pytest.mark.e2e
def test_collide_with_repos_adhoc(server):
    base, spine_dir = server
    code, r = _post(base, "/api/collide",
                    {"idea": "只撞 repo-a 的想法", "repos": ["repo-a"], "wait": True})
    assert code == 200 and r["ok"] and r["judgement"]
    opened = [e for e in spine_mod.query(spine_dir, type="collision")
              if e.body.startswith("opened")]
    assert opened[-1].kv("group") == "臨時(repo-a)"


@pytest.mark.e2e
def test_tag_api(server):
    """tag：加/移除經 API 落 registry；state 帶 tags map 與詞彙（suggestions ∪ 既有）。"""
    base, spine_dir = server
    code, r = _post(base, "/api/tag", {"id": "repo-a", "add": ["PM", "開發"]})
    assert code == 200 and r["tags"] == ["PM", "開發"]
    from repoengine import registry
    assert registry.get_repo(spine_dir, "repo-a")["tags"] == ["PM", "開發"]
    _, body = _get(base, "/api/state")
    st = json.loads(body)
    assert st["tags"]["repo-a"] == ["PM", "開發"] and st["tags"]["repo-b"] == []
    assert "PM" in st["tag_vocab"]
    code, r = _post(base, "/api/tag", {"id": "repo-a", "remove": ["開發"]})
    assert code == 200 and r["tags"] == ["PM"]
    code, r = _post(base, "/api/tag", {"id": "ghost", "add": ["x"]})
    assert code == 500  # 未知 repo 錯誤浮出


@pytest.mark.e2e
def test_save_group_api(server):
    base, spine_dir = server
    code, r = _post(base, "/api/save_group",
                    {"name": "g2", "repos": ["repo-b"]})
    assert code == 200 and r["name"] == "g2"
    from repoengine import registry
    _, entries = registry.resolve_group(spine_dir, "g2")
    assert [e["id"] for e in entries] == ["repo-b"]
    code, r = _post(base, "/api/save_group", {"name": "", "repos": ["repo-a"]})
    assert code == 400
    code, r = _post(base, "/api/save_group", {"name": "g3", "repos": []})
    assert code == 400


# ── UI 第三輪（2026-09-02 檢討：出口變按鈕／統一卡片／會話有主）────────────

def _card_keys(st, kind=None):
    return [c["key"] for c in st["cards"] if kind is None or c["kind"] == kind]


def test_cards_collision_pending_then_judged(spine_with_repos):
    """自己丟的想法＝「處理中」卡（無出口），不是一則待忽略的未讀；
    判定回來→原地換成判定卡，帶五個真出口（route/深撞/丟棄）。"""
    from repoengine import collide
    cid = collide.submit(spine_with_repos, "測試想法", group="g1",
                         when=dt.datetime.now() - dt.timedelta(minutes=2))
    st = webui.build_state(spine_with_repos)
    pend = [c for c in st["cards"] if c["kind"] == "pending"]
    assert len(pend) == 1 and "測試想法" in pend[0]["title"]
    assert pend[0]["actions"] == []
    # opened 事件不會再以「一則可忽略的未讀」出現在收件匣
    assert not any(c["kind"] == "event" and c["title"].startswith("opened")
                   for c in st["cards"])
    collide.run_judgement(spine_with_repos, cid, provider="mock",
                          when=dt.datetime.now() - dt.timedelta(minutes=1))
    st = webui.build_state(spine_with_repos)
    assert not [c for c in st["cards"] if c["kind"] == "pending"]
    judged = [c for c in st["cards"] if c["kind"] == "collision"]
    assert len(judged) == 1
    c = judged[0]
    assert c["cid"] == cid and "測試想法" in c["title"]
    assert c["judgement"]["判定"]
    names = {a["action"] for a in c["actions"]}
    assert {"route_collision", "term_create", "ignore"} <= names
    labels = [a["label"] for a in c["actions"]]
    assert any(l.startswith("照建議落") for l in labels)
    assert "深撞" in "".join(labels) and "丟棄並記錄" in labels
    assert len([l for l in labels if "incubator" in l]) == 1   # 建議＝incubator 時不重複出「升格」


def test_cards_hide_own_records_and_handled(spine_with_repos):
    """presented/chosen 是自己出手的留痕，不進收件匣；被 chosen ref 到的事件＝已處理，ack 前就消失。"""
    t0 = dt.datetime.now() - dt.timedelta(minutes=3)
    spine_mod.append_event(spine_with_repos, "presented", "morning-brief", [],
                           body="呈現了", when=t0)
    ev = spine_mod.append_event(spine_with_repos, "suggestion", "engine", [],
                                body="upstream 有新 release",
                                when=t0 + dt.timedelta(minutes=1))
    st = webui.build_state(spine_with_repos)
    assert _card_keys(st) == [f"suggestion:{ev.time}"]
    webui._handle_action(spine_with_repos, "ignore",
                         {"type": "suggestion", "time": ev.time, "header": ev.header()})
    st = webui.build_state(spine_with_repos)
    assert _card_keys(st) == []          # chosen 本身也不進收件匣


def test_cards_interrupt_first_and_loops(spine_with_repos):
    t0 = dt.datetime.now() - dt.timedelta(minutes=3)
    spine_mod.append_event(spine_with_repos, "open-loop", "hotkey", ["#1", "due:2030-01-01"],
                           body="opened →「試」", when=t0)
    spine_mod.append_event(spine_with_repos, "collision", "engine", ["id:x1"],
                           body="判定失敗（system-unsure，准打斷）：boom",
                           when=t0 + dt.timedelta(minutes=1))
    st = webui.build_state(spine_with_repos)
    assert st["cards"][0]["kind"] == "interrupt"
    loop = [c for c in st["cards"] if c["kind"] == "loop"][0]
    assert loop["num"] == "#1" and loop["due"] == "2030-01-01"
    assert {a["action"] for a in loop["actions"]} == {"collide_prefill", "loop_defer", "close_loop"}
    # open-loop 事件只以 loop 卡呈現，不再重複成一則「其他未讀」
    assert not [c for c in st["cards"] if c["kind"] == "event" and c.get("etype") == "open-loop"]


def test_scan_question_cards_and_defer(spine_with_repos):
    """問句是卡：有 kind/repo/actions；defer 後同一問句 7 天內不再浮出（chosen 留痕）。"""
    sc = webui.build_scan(spine_with_repos, "g1")
    qc = [c for c in sc["question_cards"] if c["repo"] == "repo-b"]
    assert len(qc) == 1 and qc[0]["kind"] == "question"
    assert {a["action"] for a in qc[0]["actions"]} == {"term_create", "defer"}
    webui._handle_action(spine_with_repos, "defer",
                         {"id": "repo-b", "qkind": qc[0]["qkind"], "days": 7,
                          "text": qc[0]["title"]})
    sc = webui.build_scan(spine_with_repos, "g1")
    assert not [c for c in sc["question_cards"] if c["repo"] == "repo-b"]
    assert not any("repo-b" in q for q in sc["questions"])
    ch = spine_mod.query(spine_with_repos, type="chosen")
    assert len(ch) == 1 and ch[0].kv("repo") == "repo-b" and "snooze" in ch[0].body


@pytest.mark.e2e
def test_tier_action(server):
    base, spine_dir = server
    code, r = _post(base, "/api/tier", {"id": "repo-a", "tier": "dormant"})
    assert code == 200 and r["tier"] == "dormant"
    from repoengine import registry
    assert registry.get_repo(spine_dir, "repo-a")["tier"] == "dormant"
    dec = spine_mod.query(spine_dir, type="decision")
    assert dec and dec[-1].kv("repo") == "repo-a" and "dormant" in dec[-1].body
    code, r = _post(base, "/api/tier", {"id": "repo-a", "tier": "bogus"})
    assert code == 400


@pytest.mark.e2e
def test_route_collision_action(server):
    base, spine_dir = server
    code, r = _post(base, "/api/collide",
                    {"idea": "落到 incubator 的想法", "group": "g1", "wait": True})
    cid = r["cid"]
    code, r = _post(base, "/api/route_collision", {"cid": cid, "dest": "incubator"})
    assert code == 200 and r["ok"]
    from pathlib import Path
    files = list((Path(spine_dir) / "incubator").glob("*.md"))
    assert len(files) == 1 and "落到 incubator 的想法" in files[0].read_text(encoding="utf-8")
    ch = spine_mod.query(spine_dir, type="chosen")
    assert ch and ch[-1].kv("ref") == f"collision:{cid}"
    st = json.loads(_get(base, "/api/state")[1])
    assert not [c for c in st["cards"] if c.get("cid") == cid]   # 落完就從收件匣消失
    code, r = _post(base, "/api/route_collision", {"cid": cid, "dest": "mars"})
    assert code == 400


@pytest.mark.e2e
def test_loop_defer_action(server):
    base, spine_dir = server
    today = dt.date.today()
    spine_mod.append_event(spine_dir, "open-loop", "hotkey",
                           ["#9", f"due:{today:%Y-%m-%d}"], body="opened →「明天再說」",
                           when=dt.datetime.now() - dt.timedelta(minutes=1))
    code, r = _post(base, "/api/loop_defer", {"num": "#9", "days": 7})
    assert code == 200
    loops = spine_mod.open_loops(spine_dir)
    assert len(loops) == 1
    assert loops[0].kv("due") == f"{today + dt.timedelta(days=7):%Y-%m-%d}"
    code, r = _post(base, "/api/loop_defer", {"num": "9", "days": 7})
    assert code == 400


def test_agent_session_title_has_task():
    """會話有主：title 用任務摘要，不再只有 claude:全部（開三個分得出誰是誰）。"""
    from repoengine import term
    assert term.agent_title("claude", "MI_PM", None) == "claude · MI_PM"
    t = term.agent_title("claude", "全部", "把 §7 動詞鏈對到 MCP 工具清單，順便檢查 README")
    assert t.startswith("claude · 把 §7 動詞鏈對到") and len(t) <= 40
