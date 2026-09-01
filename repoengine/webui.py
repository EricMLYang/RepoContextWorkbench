"""S4 工作台（IDE 風格；2026-09-01 使用者裁定：要工作台不要玩具監控頁，
內嵌 terminal 直接與 agent 對話——取代 v2 §7「不內嵌 terminal」舊裁定）。

版面＝IDE 三件套：左 sidebar（組＋repo 選取與狀態＋audit）｜中 main（事件流／簡報／深看分頁）
｜下 terminal 面板（xterm.js＋PTY，shell 與帶料 agent 會話都開在這）｜狀態列（量測）。

殼原則（v2 §1）不變：
- /api/state  輕量輪詢（純脊椎/registry 檔案讀）＝「殼只 poll 脊椎未讀」
- /api/scan   才跑 P3 採集（開頁/切組/勾選/手動），高頻輪詢不打 git subprocess
- 所有行動按鈕都只是呼叫引擎原語再落脊椎；terminal 會話生命週期獨立於視窗

安全（檢討④ VDI multi-session：localhost 埠跨 session 共享）：
- 只綁 127.0.0.1 ＋ 每次啟動隨機 token——URL 帶 token 才服務（/static 除外）；
  pywebview 視窗拿完整 URL，其他本機使用者猜不到
- WS（terminal）同樣驗 token；WS 實作為 stdlib 手寫 RFC6455 最小子集（零依賴）
"""
import base64
import hashlib
import json
import queue
import secrets
import struct
import threading
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

from . import brief as _brief
from . import collect as _collect
from . import collide as _collide
from . import config as _config
from . import notify as _notify
from . import registry as _registry
from . import spine as _spine
from . import term as _term
from . import timer as _timer

STATIC_DIR = Path(__file__).parent / "static"


def _ev_dict(ev):
    return {"date": ev.date, "time": ev.time, "type": ev.type,
            "source": ev.source, "tokens": ev.tokens, "body": ev.body,
            "header": ev.header()}


def build_state(spine_dir):
    """輕量狀態（純檔案讀，供 5 秒輪詢）。interrupt 先於 normal（打斷要掙得，其餘累積）。"""
    data = _registry.load(spine_dir)
    p = _notify.pending(spine_dir)
    vocab = list(_config.load(spine_dir)["tags"]["suggestions"])
    for t in _registry.all_tags(spine_dir):
        if t not in vocab:
            vocab.append(t)
    return {
        "groups": [{"name": g["name"], "members": g["members"]}
                   for g in data["groups"]],
        "repos": [r["id"] for r in data["repos"]],
        "tags": {r["id"]: (r.get("tags") or []) for r in data["repos"]},
        "tag_vocab": vocab,
        "unread": [dict(_ev_dict(e), interrupt=True) for e in p["interrupt"]]
                  + [_ev_dict(e) for e in p["normal"]],
        "loops": [_ev_dict(e) for e in _spine.open_loops(spine_dir)],
        "stats": {**_spine.stats(spine_dir), **_registry.survival(spine_dir)},
    }


def build_scan(spine_dir, group=None, repos=None):
    """P3 採集＋閾值問句＋audit（開頁/切組/勾選/手動才跑，因為會打 git）。
    repos 給定＝臨時組合（P2 免建組），優先於 group。"""
    cfg = _config.load(spine_dir)
    if repos:
        gname, entries = _registry.resolve_group(spine_dir, None, repos)
    elif group:
        gname, entries = _registry.resolve_group(spine_dir, group)
    else:
        entries = _registry.load(spine_dir)["repos"]
        gname = "全部"
    states = _collect.collect_group(entries)
    return {
        "group": gname,
        "states": states,
        "questions": _brief.build_questions(states, cfg["thresholds"]),
        "audit": _registry.audit(spine_dir),
    }


def _handle_action(spine_dir, action, payload, terms=None):
    if action == "ack":
        _spine.ack_unread(spine_dir)
        return {"ok": True}
    if action == "collide":
        idea = (payload.get("idea") or "").strip()
        if not idea:
            return {"error": "想法不可為空"}
        repos = payload.get("repos") or None
        group = payload.get("group") or None
        cid = _collide.submit(spine_dir, idea, group=group, repos=repos,
                              source="monitor")
        if payload.get("wait"):
            j = _collide.run_judgement(spine_dir, cid)
            return {"ok": True, "cid": cid, "judgement": j}
        _collide.spawn_detached(spine_dir, cid)
        return {"ok": True, "cid": cid}
    if action == "ignore":
        etype, etime = payload.get("type"), payload.get("time")
        if etype not in _spine.EVENT_TYPES:
            return {"error": f"未知事件型別: {etype}"}
        _spine.append_event(spine_dir, "chosen", "monitor",
                            [f"ref:{etype}:{etime}"],
                            body=f"忽略並記錄：{payload.get('header', '')}")
        return {"ok": True}
    if action == "close_loop":
        num = payload.get("num", "")
        if not (num.startswith("#") and num[1:].isdigit()):
            return {"error": f"loop 編號不合法: {num}"}
        _spine.append_event(spine_dir, "open-loop", "monitor", [num],
                            body="closed → 從工作台關閉")
        return {"ok": True}
    if action == "tag":
        rid = payload.get("id") or ""
        tags = _registry.tag_repo(spine_dir, rid,
                                  add=payload.get("add") or None,
                                  remove=payload.get("remove") or None)
        return {"ok": True, "id": rid, "tags": tags}
    if action == "save_group":
        name = (payload.get("name") or "").strip()
        members = payload.get("repos") or []
        if not name:
            return {"error": "組名不可為空"}
        if not members:
            return {"error": "至少勾選一個 repo"}
        _registry.add_group(spine_dir, name, members)
        return {"ok": True, "name": name}
    if action == "brief":
        out, text = _brief.run(spine_dir, payload.get("group") or None,
                               payload.get("repos") or None)
        return {"ok": True, "text": text, "file": str(out)}
    if action == "term_create":
        if terms is None:
            return {"error": "本 server 未啟用 terminal"}
        kind = payload.get("kind", "shell")
        if kind == "agent":
            s = terms.create_agent(group=payload.get("group") or None,
                                   repos=payload.get("repos") or None,
                                   repo=payload.get("repo") or None,
                                   task=payload.get("task") or None,
                                   agent=payload.get("agent") or "claude")
        else:
            s = terms.create_shell(cwd=payload.get("cwd") or None)
        return {"ok": True, "sid": s.sid, "title": s.title}
    if action == "term_kill":
        if terms is None or not terms.kill(payload.get("sid", "")):
            return {"error": f"未知會話: {payload.get('sid')}"}
        return {"ok": True}
    return {"error": f"未知 action: {action}"}


# ── WebSocket 最小子集（RFC6455；不做 fragmentation/extension——瀏覽器端
#    terminal 訊息都是小 frame，prototype 夠用）─────────────────────────

_WS_MAGIC = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


def ws_accept_key(key):
    return base64.b64encode(
        hashlib.sha1((key + _WS_MAGIC).encode("ascii")).digest()).decode("ascii")


def ws_encode(data, opcode=2, mask=False):
    """單一 frame。server→client 不 mask；mask=True 給測試當 client 用。"""
    if isinstance(data, str):
        data = data.encode("utf-8")
    head = bytes([0x80 | opcode])
    ln = len(data)
    mbit = 0x80 if mask else 0
    if ln < 126:
        head += bytes([mbit | ln])
    elif ln < 65536:
        head += bytes([mbit | 126]) + struct.pack(">H", ln)
    else:
        head += bytes([mbit | 127]) + struct.pack(">Q", ln)
    if mask:
        key = secrets.token_bytes(4)
        return head + key + bytes(c ^ key[i % 4] for i, c in enumerate(data))
    return head + data


def ws_read_frame(rfile):
    """回傳 (opcode, payload bytes)。連線斷 → ConnectionError。"""
    h = rfile.read(2)
    if len(h) < 2:
        raise ConnectionError("ws closed")
    opcode = h[0] & 0x0F
    masked = h[1] & 0x80
    ln = h[1] & 0x7F
    if ln == 126:
        ln = struct.unpack(">H", rfile.read(2))[0]
    elif ln == 127:
        ln = struct.unpack(">Q", rfile.read(8))[0]
    key = rfile.read(4) if masked else b""
    payload = rfile.read(ln)
    if masked:
        payload = bytes(c ^ key[i % 4] for i, c in enumerate(payload))
    return opcode, payload


def _make_handler(spine_dir):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *a):  # 安靜，不刷終端
            pass

        def _authed(self):
            tok = getattr(self.server, "token", "")
            if not tok:
                return True
            q = parse_qs(urlparse(self.path).query)
            return (self.headers.get("X-Auth") == tok
                    or (q.get("token") or [None])[0] == tok)

        def _json(self, obj, code=200):
            body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
            self.send_response(code)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def _bytes(self, body, ctype):
            self.send_response(200)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            u = urlparse(self.path)
            try:
                if u.path.startswith("/static/"):
                    return self._static(u.path[len("/static/"):])
                if not self._authed():
                    return self._json({"error": "token 不對（重開工作台拿新網址）"}, 403)
                if u.path == "/":
                    self._bytes(PAGE.encode("utf-8"), "text/html; charset=utf-8")
                elif u.path == "/api/state":
                    st = build_state(spine_dir)
                    st["terms"] = self.server.terms.list()
                    st["agents"] = list(_config.load(spine_dir)["agents"])
                    self._json(st)
                elif u.path == "/api/scan":
                    q = parse_qs(u.query)
                    self._json(build_scan(
                        spine_dir,
                        group=(q.get("group") or [None])[0],
                        repos=(q.get("repos") or [None])[0]))
                elif u.path.startswith("/ws/term/"):
                    self._handle_ws(u.path[len("/ws/term/"):])
                else:
                    self._json({"error": "not found"}, 404)
            except Exception as e:  # 失敗必須浮出，不准白屏
                try:
                    self._json({"error": str(e)}, 500)
                except OSError:
                    pass

        def do_POST(self):
            u = urlparse(self.path)
            try:
                if not self._authed():
                    return self._json({"error": "token 不對"}, 403)
                n = int(self.headers.get("Content-Length") or 0)
                payload = json.loads(self.rfile.read(n) or b"{}")
                if not u.path.startswith("/api/"):
                    return self._json({"error": "not found"}, 404)
                result = _handle_action(spine_dir, u.path[len("/api/"):],
                                        payload, terms=self.server.terms)
                self._json(result, 400 if "error" in result else 200)
            except Exception as e:
                self._json({"error": str(e)}, 500)

        def _static(self, name):
            p = STATIC_DIR / name
            if "/" in name or ".." in name or not p.is_file():
                return self._json({"error": "not found"}, 404)
            ctype = ("text/css" if name.endswith(".css")
                     else "application/javascript; charset=utf-8")
            self._bytes(p.read_bytes(), ctype)

        def _handle_ws(self, sid):
            key = self.headers.get("Sec-WebSocket-Key")
            sess = self.server.terms.get(sid)
            if not key or sess is None:
                return self._json({"error": f"未知會話或非 ws: {sid}"}, 404)
            self.wfile.write((
                "HTTP/1.1 101 Switching Protocols\r\n"
                "Upgrade: websocket\r\nConnection: Upgrade\r\n"
                f"Sec-WebSocket-Accept: {ws_accept_key(key)}\r\n\r\n"
            ).encode("ascii"))
            self.wfile.flush()
            sendlock = threading.Lock()

            def send(data, opcode=2):
                with sendlock:
                    self.connection.sendall(ws_encode(data, opcode))

            # replay 先送、live 輸出經 queue 排在後面——順序不亂
            q = queue.Queue()
            replay = sess.attach(q.put)
            try:
                send(replay)

                def _pump():
                    while True:
                        chunk = q.get()
                        if chunk is None:
                            return
                        try:
                            send(chunk)
                        except OSError:
                            return
                pump = threading.Thread(target=_pump, daemon=True)
                pump.start()
                while True:
                    opcode, payload = ws_read_frame(self.rfile)
                    if opcode == 8:      # close
                        break
                    if opcode == 9:      # ping → pong
                        send(payload, 0xA)
                        continue
                    if opcode in (1, 2) and payload:
                        msg = json.loads(payload)
                        if msg.get("t") == "i":
                            sess.write(msg["d"].encode("utf-8"))
                        elif msg.get("t") == "r":
                            sess.resize(int(msg["c"]), int(msg["r"]))
            except Exception:   # 101 之後只能斷線，不能再回 HTTP 錯誤
                pass
            finally:
                sess.detach(q.put)
                q.put(None)
                self.close_connection = True

    return Handler


def make_server(spine_dir, port=0, token=None):
    """只綁 127.0.0.1（不變式：不對外暴露）＋啟動 token（VDI 多使用者防護）。"""
    srv = ThreadingHTTPServer(("127.0.0.1", port), _make_handler(spine_dir))
    srv.token = secrets.token_hex(16) if token is None else token
    srv.terms = _term.TermManager(spine_dir)
    return srv


def _url(srv):
    return f"http://127.0.0.1:{srv.server_address[1]}/?token={srv.token}"


def _start_timer_if_scheduled(spine_dir):
    if _config.load(spine_dir).get("schedule"):
        threading.Thread(target=_timer.run_loop, args=(spine_dir,),
                         daemon=True).start()
        return True
    return False


def serve_window(spine_dir, port=0, width=1280, height=860):
    """工作台桌面視窗（pywebview 包 OS 內建 webview；預設入口，不開瀏覽器）。"""
    try:
        import webview  # pywebview（optional dependency：pip install pywebview）
    except ImportError:
        raise SystemExit("缺 pywebview——桌面視窗模式需要它：pip install pywebview\n"
                         "（過渡替代：`ui` 子命令走瀏覽器模式）")
    srv = make_server(spine_dir, port)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    if _start_timer_if_scheduled(spine_dir):
        print("timer 已隨工作台啟動（schedule 見 config.yaml）")
    try:
        webview.create_window("repo 工作台", _url(srv),
                              width=width, height=height)
        webview.start()  # 阻塞在主執行緒直到視窗關閉（pywebview 的要求）
    finally:
        srv.terms.kill_all()
        srv.shutdown()
        srv.server_close()


def serve(spine_dir, port=8765, open_browser=True):
    srv = make_server(spine_dir, port)
    url = _url(srv)
    print(f"工作台（瀏覽器模式）：{url}（Ctrl+C 結束）")
    if _start_timer_if_scheduled(spine_dir):
        print("timer 已隨工作台啟動（schedule 見 config.yaml）")
    if open_browser:
        threading.Timer(0.3, lambda: webbrowser.open(url)).start()
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        print("\n工作台已關閉")
    finally:
        srv.terms.kill_all()
        srv.server_close()


PAGE = r"""<!doctype html>
<html lang="zh-Hant"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>repo 工作台</title>
<link rel="stylesheet" href="/static/xterm.css">
<style>
  :root{--bg:#1b1d23;--panel:#22252d;--panel2:#282c36;--line:#343945;
        --ink:#d7dae0;--sub:#8b91a0;--accent:#6ca0dd;--red:#e06c75;
        --warn:#d19a66;--ok:#98c379;--badge:#c74e39;--termh:280px;}
  *{box-sizing:border-box}
  html,body{height:100%}
  body{margin:0;background:var(--bg);color:var(--ink);overflow:hidden;
       font:13px/1.55 "Segoe UI","Microsoft JhengHei",system-ui,sans-serif;
       display:grid;
       grid-template:"top top" 44px "side main" 1fr "side term" var(--termh)
                     "status status" 26px / 300px 1fr;}
  button,input,select{font:inherit;border:1px solid var(--line);border-radius:5px;
       background:var(--panel2);color:var(--ink);padding:3px 9px}
  button{cursor:pointer}
  button:hover{border-color:var(--accent);color:var(--accent)}
  button.small{font-size:12px;padding:1px 7px}
  h2{font-size:12px;margin:0 0 6px;color:var(--sub);font-weight:600;letter-spacing:.06em}
  ::-webkit-scrollbar{width:9px;height:9px}
  ::-webkit-scrollbar-thumb{background:var(--line);border-radius:5px}

  #top{grid-area:top;display:flex;align-items:center;gap:10px;padding:0 12px;
       background:var(--panel);border-bottom:1px solid var(--line)}
  #logo{font-weight:700;white-space:nowrap}
  #collidewrap{flex:1;display:flex;gap:6px;align-items:center;min-width:0}
  #collidewrap .clabel{color:var(--sub);font-size:12px;white-space:nowrap}
  #idea{flex:1;min-width:0}
  #badge{background:var(--badge);color:#fff;border-radius:9px;padding:0 8px;
         font-size:12px;line-height:19px;display:none;white-space:nowrap}
  #scope{color:var(--sub);font-size:12px;white-space:nowrap;max-width:220px;
         overflow:hidden;text-overflow:ellipsis}

  #side{grid-area:side;background:var(--panel);border-right:1px solid var(--line);
        overflow-y:auto;padding:10px 10px 20px;display:flex;flex-direction:column;gap:10px}
  .srow{display:flex;gap:6px;align-items:center;flex-wrap:wrap}
  #repolist{display:flex;flex-direction:column}
  .repo{display:flex;gap:6px;align-items:center;padding:3px 2px;border-radius:4px}
  .repo:hover{background:var(--panel2)}
  .repo .rid{flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .repo.off .rid{color:var(--sub)}
  .repo .st{font-size:11px;color:var(--sub);white-space:nowrap}
  .repo .st.bad{color:var(--warn)}
  /* 永遠可見——hover 才現身的按鈕在 5 秒輪詢重繪的清單裡點不到（隱形不吃 click），
     L4 實錘「▶ 沒 work」的根因之一 */
  .repo .go{font-size:11px;padding:0 5px;opacity:.5}
  .repo:hover .go,.repo .go:hover{opacity:1}
  .chip{font-size:11px;border:1px solid var(--line);border-radius:9px;
        padding:0 8px 0 6px;cursor:pointer;color:var(--sub);white-space:nowrap;
        background:none;line-height:17px;display:inline-flex;
        align-items:center;gap:5px}
  .chip .dot{margin:0}
  .dot{width:9px;height:9px;border-radius:50%;display:inline-block;
       flex:none;cursor:pointer}
  .repo{border-left:3px solid transparent;padding-left:5px}
  .repo .dots{display:inline-flex;gap:3px;align-items:center;margin-left:2px}
  #tagfilter{display:flex;flex-wrap:wrap;gap:4px;margin:2px 0 6px}
  .tageditor{display:flex;flex-wrap:wrap;gap:4px;padding:5px 2px 9px 24px;
             border-bottom:1px solid var(--line);align-items:center}
  .tageditor input{width:96px;font-size:11px;padding:1px 6px}
  #auditbox{font-size:12px}
  #auditbox .red{color:var(--red)}
  #auditbox .okline{color:var(--ok)}

  #main{grid-area:main;overflow-y:auto;padding:12px 16px}
  #tabs{display:flex;gap:2px;margin-bottom:10px;border-bottom:1px solid var(--line)}
  #tabs button{border:none;background:none;border-radius:0;color:var(--sub);
               padding:5px 14px;border-bottom:2px solid transparent}
  #tabs button.on{color:var(--ink);border-bottom-color:var(--accent)}
  .tabpane{display:none}
  .tabpane.on{display:block}
  section{background:var(--panel);border:1px solid var(--line);border-radius:7px;
          padding:10px 14px;margin-bottom:12px}
  .row{display:flex;gap:8px;align-items:flex-start;padding:7px 0;
       border-top:1px solid var(--line)}
  .row:first-of-type{border-top:none}
  .meta{color:var(--sub);font-size:12px;white-space:nowrap;padding-top:2px}
  .body{flex:1;white-space:pre-wrap;word-break:break-word}
  .tag{font-size:11px;border:1px solid var(--line);border-radius:4px;
       padding:0 5px;color:var(--sub);white-space:nowrap}
  .tag.red,.body.red{color:var(--red);border-color:var(--red)}
  .warn{color:var(--warn)}
  #allclear{color:var(--ok);font-size:14px;padding:4px 0 10px}
  #briefout{background:var(--panel);border:1px solid var(--line);border-radius:7px;
            padding:12px;white-space:pre-wrap;font-size:13px}
  table{border-collapse:collapse;width:100%;font-size:12.5px}
  th,td{text-align:left;padding:4px 10px 4px 0;border-bottom:1px solid var(--line)}
  th{color:var(--sub);font-weight:600;white-space:nowrap}
  td.num,th.num{text-align:right}
  #statsraw{color:var(--sub);font-size:12px;margin-top:8px}

  #termpanel{grid-area:term;display:flex;flex-direction:column;min-height:0;
             background:#161a20;border-top:1px solid var(--line)}
  #termdrag{height:4px;cursor:ns-resize;background:transparent}
  #termdrag:hover{background:var(--accent)}
  #termbar{display:flex;align-items:center;gap:6px;padding:2px 10px 4px;flex-wrap:nowrap}
  #termbar .tlabel{color:var(--sub);font-size:12px;letter-spacing:.06em}
  #termtabs{display:flex;gap:4px;overflow-x:auto;flex:1}
  .ttab{display:flex;gap:6px;align-items:center;border:1px solid var(--line);
        border-radius:5px;padding:1px 8px;font-size:12px;color:var(--sub);
        cursor:pointer;white-space:nowrap;background:var(--panel)}
  .ttab.on{color:var(--ink);border-color:var(--accent)}
  .ttab.dead{opacity:.55;text-decoration:line-through}
  .ttab .x{color:var(--sub)}
  .ttab .x:hover{color:var(--red)}
  #termbody{flex:1;min-height:0;position:relative}
  .termhost{position:absolute;inset:0 0 0 8px;display:none}
  .termhost.on{display:block}
  #termempty{position:absolute;inset:0;display:flex;align-items:center;
             justify-content:center;color:var(--sub);font-size:13px}

  #status{grid-area:status;display:flex;align-items:center;gap:16px;
          padding:0 12px;background:var(--panel);border-top:1px solid var(--line);
          color:var(--sub);font-size:12px;white-space:nowrap;overflow:hidden}
  #toast{position:fixed;bottom:36px;left:50%;transform:translateX(-50%);
         background:#000c;color:#fff;border-radius:6px;padding:8px 16px;
         opacity:0;transition:opacity .3s;pointer-events:none;max-width:70%;z-index:9}
</style></head><body>

<div id="top">
  <span id="logo">⚙ repo 工作台</span>
  <div id="collidewrap"><span class="clabel">碰撞台</span>
    <input id="idea" placeholder="想法丟進來，Enter 送出（fire-and-forget，判定走未讀回程）">
    <button id="collidebtn">丟進碰撞</button></div>
  <span id="scope"></span>
  <span id="badge"></span>
  <button id="ackbtn" class="small">全部已讀</button>
  <button id="rescan" class="small">重新採集</button>
</div>

<div id="side">
  <div class="srow">組
    <select id="group" style="flex:1"></select></div>
  <div>
    <h2>Repo 選取與狀態
      <button id="checkall" class="small">全</button>
      <button id="checknone" class="small">無</button></h2>
    <div id="tagfilter" title="點 tag＝勾選有該 tag 的 repo（可多選聯集）"></div>
    <div id="repolist"></div>
    <div class="srow" style="margin-top:6px">
      <input id="newgroup" placeholder="組名" style="width:100px">
      <button id="savegroup" class="small">勾選存成組</button></div>
  </div>
  <div><h2>registry audit</h2><div id="auditbox"></div></div>
</div>

<div id="main">
  <div id="tabs">
    <button data-tab="events" class="on">事件流</button>
    <button data-tab="brief">簡報</button>
    <button data-tab="deep">深看</button>
  </div>
  <div id="tab-events" class="tabpane on">
    <div id="allclear" hidden>✅ 一切正常。</div>
    <section id="qsec" hidden><h2>需要你判斷的</h2><div id="questions"></div></section>
    <section id="usec" hidden><h2>未讀事件</h2><div id="unread"></div></section>
    <section id="lsec" hidden><h2>未結 open loops</h2><div id="loops"></div></section>
  </div>
  <div id="tab-brief" class="tabpane">
    <div style="margin-bottom:10px"><button id="briefbtn">產生早晨簡報（目前範圍，落 presented 事件）</button></div>
    <pre id="briefout">（還沒產生）</pre>
  </div>
  <div id="tab-deep" class="tabpane">
    <section><h2>深看：全量狀態</h2><div id="deeptable"></div>
      <div id="statsraw"></div></section>
  </div>
</div>

<div id="termpanel">
  <div id="termdrag" title="拖曳調整終端高度"></div>
  <div id="termbar">
    <span class="tlabel">終端</span>
    <div id="termtabs"></div>
    <button id="newshell" class="small">＋shell</button>
    <select id="agentsel" class="small" title="開哪家 agent CLI（config agents: 註冊）"></select>
    <button id="newagent" class="small">＋agent 會話（帶料）</button>
  </div>
  <div id="termbody"><div id="termempty">＋agent 會話＝打包目前勾選範圍開 claude；Ctrl+` 收合面板</div></div>
</div>

<div id="status">
  <span id="statsline"></span><span id="scantime"></span><span id="scanscope"></span>
</div>
<div id="toast"></div>

<script src="/static/xterm.js"></script>
<script src="/static/addon-fit.js"></script>
<script>
const $=id=>document.getElementById(id);
const TOKEN=new URLSearchParams(location.search).get("token")||"";
let curGroup=localStorage.getItem("group")||"";
let groupsData=[],allRepos=[],scopeRepos=[],lastStates=[],checked=new Set();
let tagsMap={},tagVocab=[],activeTags=new Set(),editorFor=null;

function toast(msg){const t=$("toast");t.textContent=msg;t.style.opacity=1;
  setTimeout(()=>t.style.opacity=0,2600);}
async function api(path,opts){
  opts=opts||{};opts.headers=Object.assign({"X-Auth":TOKEN},opts.headers||{});
  const r=await fetch(path,opts);const j=await r.json();
  if(j.error){toast("錯誤："+j.error);throw new Error(j.error);}return j;}
const post=o=>({method:"POST",headers:{"Content-Type":"application/json"},
  body:JSON.stringify(o)});
function el(tag,cls,text){const e=document.createElement(tag);
  if(cls)e.className=cls;if(text!==undefined)e.textContent=text;return e;}

function loadChecked(){try{const raw=localStorage.getItem("checked:"+curGroup);
  if(raw)return new Set(JSON.parse(raw));}catch(e){}return null;}
function saveChecked(){try{localStorage.setItem("checked:"+curGroup,
  JSON.stringify([...checked]));}catch(e){}}
function scopeReposFor(g){if(!g)return allRepos.slice();
  const hit=groupsData.find(x=>x.name===g);return hit?hit.members.slice():[];}
function selectedRepos(){return scopeRepos.filter(r=>checked.has(r));}
function partial(){const s=selectedRepos();return s.length&&s.length!==scopeRepos.length;}
function scopePayload(){const p={};if(partial())p.repos=selectedRepos();
  else if(curGroup)p.group=curGroup;return p;}
function scanParam(){if(partial())return "repos="+encodeURIComponent(selectedRepos().join(","));
  return curGroup?("group="+encodeURIComponent(curGroup)):"";}
function updateScope(){$("scope").textContent=partial()
  ?("臨時組合："+selectedRepos().join(", "))
  :(curGroup?("組："+curGroup):"範圍：全部");}

// ── 分頁 ──
document.querySelectorAll("#tabs button").forEach(b=>b.onclick=()=>{
  document.querySelectorAll("#tabs button").forEach(x=>x.classList.remove("on"));
  document.querySelectorAll(".tabpane").forEach(x=>x.classList.remove("on"));
  b.classList.add("on");$("tab-"+b.dataset.tab).classList.add("on");});

// ── 事件流 ──
const notified=new Set();
function maybeNotify(ev){const key=ev.date+" "+ev.time+" "+ev.header;
  if(notified.has(key))return;notified.add(key);
  try{if(!("Notification" in window))return;
    if(Notification.permission==="default")Notification.requestPermission();
    if(Notification.permission==="granted")
      new Notification("需要你看一下（system-unsure）",
        {body:(ev.body||"").split("\n")[0]});}catch(e){}}

function renderState(st){
  groupsData=st.groups;allRepos=st.repos;
  tagsMap=st.tags||{};tagVocab=st.tag_vocab||[];
  if(scopeRepos.length)renderRepoList();
  const n=st.unread.length;
  $("badge").style.display=n?"inline-block":"none";
  $("badge").textContent=n?("未讀 "+n):"";
  const sel=$("group");
  if(sel.options.length!==st.groups.length+1){
    sel.innerHTML="";sel.append(new Option("（全部）",""));
    st.groups.forEach(g=>sel.append(new Option(g.name,g.name)));
    sel.value=curGroup;}
  const u=$("unread");u.innerHTML="";
  st.unread.forEach(ev=>{
    const row=el("div","row");
    row.append(el("span","meta",ev.date+" "+ev.time));
    row.append(el("span","tag"+(ev.interrupt?" red":""),
      (ev.interrupt?"⚠ ":"")+ev.type+" ["+ev.source+"]"));
    row.append(el("div","body"+(ev.interrupt?" red":""),
      ev.body||ev.tokens.join(" ")));
    const btn=el("button","small","忽略並記錄");
    btn.onclick=async()=>{await api("/api/ignore",
      post({type:ev.type,time:ev.time,header:ev.header}));
      toast("已忽略並記錄");refreshState();};
    row.append(btn);u.append(row);
    if(ev.interrupt)maybeNotify(ev);});
  $("usec").hidden=!n;
  const L=$("loops");L.innerHTML="";
  st.loops.forEach(ev=>{
    const num=(ev.tokens.find(t=>t.startsWith("#"))||"#?");
    const due=(ev.tokens.find(t=>t.startsWith("due:"))||"").slice(4);
    const row=el("div","row");
    row.append(el("span","meta",num+(due?"（due "+due+"）":"")));
    row.append(el("div","body",(ev.body||"").split("\n")[0]));
    const btn=el("button","small","關閉");
    btn.onclick=async()=>{await api("/api/close_loop",post({num}));
      toast("已關閉 "+num);refreshState();};
    row.append(btn);L.append(row);});
  $("lsec").hidden=!st.loops.length;
  const s=st.stats;
  $("statsline").textContent="碰撞 "+s.collisions+"｜outcome 回連 "
    +s.collisions_with_outcome+"｜命中率 "
    +(s.hit_rate==null?"n/a":Math.round(s.hit_rate*100)+"%")
    +"｜生出 repo "+s.spawned+"｜存活率 "
    +(s.survival_rate==null?"n/a":Math.round(s.survival_rate*100)+"%");
  const asel=$("agentsel");
  if(st.agents&&asel.options.length!==st.agents.length){
    const cur=asel.value;asel.innerHTML="";
    st.agents.forEach(a=>asel.append(new Option(a,a)));
    if(st.agents.includes(cur))asel.value=cur;}
  syncTerms(st.terms||[]);
  updateAllClear();
}

// ── tag：色點制——列上只有顏色（hover 看名、點了開編輯器才有字），
//    同 tag 聚在一起（依主 tag 排序＋左緣同色描邊），篩選/編輯器同一套色 ──
const TAGCOLORS=["#6ca0dd","#98c379","#d19a66","#c678dd","#56b6c2",
                 "#e06c75","#e5c07b","#7f9f7f","#bf7fbf","#8fa1b3"];
function tagColor(t){
  let i=tagVocab.indexOf(t);
  if(i<0){i=0;for(const c of t)i=(i*31+c.codePointAt(0))%997;}
  return TAGCOLORS[i%TAGCOLORS.length];
}
function primaryOrder(id){
  const ts=tagsMap[id]||[];
  if(!ts.length)return 998;
  const i=tagVocab.indexOf(ts[0]);
  return i<0?997:i;
}
function makeDot(t,onclick){
  const d=el("span","dot");d.style.background=tagColor(t);d.title=t;
  if(onclick)d.onclick=onclick;return d;
}
function toggleTagFilter(t){
  activeTags.has(t)?activeTags.delete(t):activeTags.add(t);
  if(activeTags.size){
    checked=new Set(scopeRepos.filter(r=>
      (tagsMap[r]||[]).some(x=>activeTags.has(x))));
    saveChecked();updateScope();rescan();
  }
  renderRepoList();
}
function renderTagFilter(){
  const d=$("tagfilter");d.innerHTML="";
  const used=new Set();Object.values(tagsMap).forEach(a=>a.forEach(t=>used.add(t)));
  tagVocab.forEach(t=>{
    const c=el("button","chip"+(activeTags.has(t)?" on":""));
    c.append(makeDot(t),document.createTextNode(t));
    if(activeTags.has(t)){c.style.borderColor=tagColor(t);c.style.color=tagColor(t);}
    if(!used.has(t))c.style.opacity=.4;    // 詞彙裡有但還沒人用
    c.onclick=()=>toggleTagFilter(t);d.append(c);});
  if(activeTags.size){
    const x=el("button","chip","✕ 清除篩選");
    x.onclick=()=>{activeTags.clear();renderRepoList();};d.append(x);}
}
async function setTag(id,t,on){
  const r=await api("/api/tag",post(on?{id,add:[t]}:{id,remove:[t]}));
  tagsMap[id]=r.tags;renderRepoList();
}
function tagEditor(id){
  const ed=el("div","tageditor");
  const cur=new Set(tagsMap[id]||[]);
  tagVocab.forEach(t=>{
    const c=el("button","chip"+(cur.has(t)?" on":""));
    c.append(makeDot(t),document.createTextNode(t));
    if(cur.has(t)){c.style.borderColor=tagColor(t);c.style.color=tagColor(t);}
    c.onclick=()=>setTag(id,t,!cur.has(t));ed.append(c);});
  const inp=document.createElement("input");
  inp.placeholder="新 tag，Enter";
  inp.onkeydown=e=>{if(e.key==="Enter"&&inp.value.trim())
    setTag(id,inp.value.trim(),true);};
  ed.append(inp);
  const done=el("button","chip","完成");
  done.onclick=()=>{editorFor=null;renderRepoList();};ed.append(done);
  return ed;
}

function renderRepoList(){
  renderTagFilter();
  const d=$("repolist");d.innerHTML="";
  const byId={};lastStates.forEach(s=>byId[s.id]=s);
  // 同 tag 聚在一起：依主 tag（詞彙順序）排序，左緣同色描邊做視覺分塊
  const sorted=[...scopeRepos].sort((a,b)=>
    primaryOrder(a)-primaryOrder(b)||a.localeCompare(b));
  sorted.forEach(id=>{
    const s=byId[id];
    const row=el("div","repo"+(checked.has(id)?"":" off"));
    const ts=tagsMap[id]||[];
    if(ts.length)row.style.borderLeftColor=tagColor(ts[0]);
    const cb=document.createElement("input");cb.type="checkbox";
    cb.checked=checked.has(id);
    cb.onchange=()=>{cb.checked?checked.add(id):checked.delete(id);
      saveChecked();renderRepoList();updateScope();rescan();};
    row.append(cb);
    row.append(el("span","rid",id));
    const dots=el("span","dots");   // 色點制：hover 看名、點了開編輯器才有字
    ts.forEach(t=>dots.append(makeDot(t,
      ()=>{editorFor=editorFor===id?null:id;renderRepoList();})));
    row.append(dots);
    if(s){
      const bad=(s.dirty_days!=null&&s.dirty_days>=4)||!s.exists||s.note
                ||(s.behind||0)>0;
      const bits=[];
      if(s.dirty)bits.push("dirty "+s.dirty+(s.dirty_days!=null?"×"+s.dirty_days.toFixed(0)+"d":""));
      if(s.ahead)bits.push("↑"+s.ahead);      // 有 commit 沒 push
      if(s.behind)bits.push("↓"+s.behind);    // 落後遠端（fetch 後才準）
      if(s.last_commit_days!=null)bits.push(s.last_commit_days.toFixed(0)+"d");
      if(s.note)bits.push(s.note);
      row.append(el("span","st"+(bad?" bad":""),bits.join("｜")||"✓"));
    }else row.append(el("span","st","…"));
    const tg=el("button","small go","🏷");
    tg.title="編輯 tag";
    tg.onclick=()=>{editorFor=editorFor===id?null:id;renderRepoList();};
    row.append(tg);
    const go=el("button","small go","▶");
    go.title="在此 repo 開 agent 會話（帶料）";
    go.onclick=()=>newTerm("agent",{repo:id,repos:[id]});
    row.append(go);d.append(row);
    if(editorFor===id)d.append(tagEditor(id));});
}

function renderScan(sc){
  $("scantime").textContent="採集於 "+new Date().toLocaleTimeString();
  $("scanscope").textContent="採集範圍："+sc.group;
  sc.states.forEach(s=>{const i=lastStates.findIndex(x=>x.id===s.id);
    if(i>=0)lastStates[i]=s;else lastStates.push(s);});
  const q=$("questions");q.innerHTML="";
  sc.questions.forEach((t,i)=>{const row=el("div","row");
    row.append(el("span","meta",(i+1)+"."));
    row.append(el("div","body warn",t));q.append(row);});
  $("qsec").hidden=!sc.questions.length;
  renderRepoList();
  const a=$("auditbox");a.innerHTML="";
  sc.audit.forEach(t=>a.append(el("div","red","🔴 "+t)));
  if(!sc.audit.length)a.append(el("div","okline","零紅字 ✅"));
  const d=$("deeptable");d.innerHTML="";
  const tb=el("table");const hd=el("tr");
  [["repo",""],["tags",""],["branch",""],["tier",""],["dirty","num"],["dirty天","num"],
   ["末commit天","num"],["↑未push","num"],["↓落後","num"],
   ["最後 commit",""],["note",""]].forEach(([t,c])=>hd.append(el("th",c,t)));
  tb.append(hd);
  sc.states.forEach(s=>{const tr=el("tr");
    const f=v=>v==null?"-":v.toFixed(1);
    const n=v=>v==null?"-":String(v);
    tr.append(el("td","",s.id));
    tr.append(el("td","",(tagsMap[s.id]||[]).join("、")));
    tr.append(el("td","",s.branch||"-"));
    tr.append(el("td","",s.tier));
    tr.append(el("td","num",String(s.dirty)));
    tr.append(el("td","num",f(s.dirty_days)));
    tr.append(el("td","num",f(s.last_commit_days)));
    tr.append(el("td","num",n(s.ahead)));
    tr.append(el("td","num",n(s.behind)));
    tr.append(el("td","",(s.last_subject||"").slice(0,60)));
    tr.append(el("td","",s.note||""));tb.append(tr);});
  d.append(tb);
  $("statsraw").textContent=$("statsline").textContent;
  updateAllClear();
}
function updateAllClear(){
  $("allclear").hidden=!($("qsec").hidden&&$("usec").hidden&&$("lsec").hidden);}

async function refreshState(){try{renderState(await api("/api/state"));
  if(!scopeRepos.length)resetScope();}catch(e){}}
function resetScope(){
  scopeRepos=scopeReposFor(curGroup);
  const saved=loadChecked();
  checked=saved?new Set([...saved].filter(r=>scopeRepos.includes(r)))
               :new Set(scopeRepos);
  if(!checked.size)checked=new Set(scopeRepos);
  renderRepoList();updateScope();}
async function rescan(){try{const qp=scanParam();
  renderScan(await api("/api/scan"+(qp?"?"+qp:"")));}catch(e){}}

$("group").onchange=e=>{curGroup=e.target.value;
  try{localStorage.setItem("group",curGroup);}catch(err){}
  resetScope();rescan();};
$("rescan").onclick=rescan;
$("ackbtn").onclick=async()=>{await api("/api/ack",post({}));
  toast("已全部標記已讀");refreshState();};
$("checkall").onclick=()=>{checked=new Set(scopeRepos);saveChecked();
  renderRepoList();updateScope();rescan();};
$("checknone").onclick=()=>{checked=new Set();saveChecked();
  renderRepoList();updateScope();};
$("savegroup").onclick=async()=>{
  const name=$("newgroup").value.trim();
  if(!name){toast("先填組名");return;}
  const r=await api("/api/save_group",post({name,repos:selectedRepos()}));
  toast("組已建："+r.name);$("newgroup").value="";refreshState();};
async function submitIdea(){
  const v=$("idea").value.trim();if(!v)return;
  $("idea").value="";
  const r=await api("/api/collide",post(Object.assign({idea:v},scopePayload())));
  toast("已丟進碰撞 "+r.cid+"（判定回來會出現在未讀）");refreshState();}
$("collidebtn").onclick=submitIdea;
$("idea").addEventListener("keydown",e=>{if(e.key==="Enter")submitIdea();});
$("briefbtn").onclick=async()=>{
  const r=await api("/api/brief",post(scopePayload()));
  $("briefout").textContent=r.text;toast("簡報已產生（presented 已落脊椎）");
  refreshState();};

// ── 終端面板（xterm.js ＋ WS）──
const terms=new Map();let activeSid=null;
function termTheme(){return{background:"#161a20",foreground:"#d7dae0",
  cursor:"#6ca0dd",selectionBackground:"#3a4150"};}
function syncTerms(list){
  list.forEach(s=>{
    let rec=terms.get(s.sid);
    if(!rec){addTab(s.sid,s.title);rec=terms.get(s.sid);}
    rec.alive=s.alive;
    rec.tab.classList.toggle("dead",!s.alive);});
  for(const [sid,rec] of terms){
    if(!list.find(s=>s.sid===sid)&&!rec.t){removeTab(sid);}}
}
function addTab(sid,title){
  const tab=el("span","ttab");
  tab.append(el("span","",title));
  const x=el("span","x","✕");
  x.onclick=async e=>{e.stopPropagation();
    try{await api("/api/term_kill",post({sid}));}catch(err){}
    removeTab(sid);};
  tab.append(x);
  tab.onclick=()=>selectTerm(sid);
  $("termtabs").append(tab);
  terms.set(sid,{tab,title,t:null,ws:null,el:null,alive:true});
}
function removeTab(sid){
  const rec=terms.get(sid);if(!rec)return;
  rec.tab.remove();if(rec.el)rec.el.remove();
  if(rec.ws)try{rec.ws.close();}catch(e){}
  terms.delete(sid);
  if(activeSid===sid){activeSid=null;
    const first=terms.keys().next();
    if(!first.done)selectTerm(first.value);else $("termempty").style.display="";}
}
function selectTerm(sid){
  const rec=terms.get(sid);if(!rec)return;
  activeSid=sid;$("termempty").style.display="none";
  for(const [id,r] of terms){r.tab.classList.toggle("on",id===sid);
    if(r.el)r.el.classList.toggle("on",id===sid);}
  if(!rec.t)attachTerm(sid);
  else setTimeout(()=>{fitTerm(rec);rec.t.focus();},0);
}
function attachTerm(sid){
  const rec=terms.get(sid);
  const host=el("div","termhost on");$("termbody").append(host);
  const t=new Terminal({fontSize:12.5,fontFamily:"Menlo,Consolas,monospace",
    theme:termTheme(),cursorBlink:true,scrollback:5000});
  const fit=new FitAddon.FitAddon();t.loadAddon(fit);t.open(host);
  const ws=new WebSocket(
    (location.protocol==="https:"?"wss://":"ws://")+location.host
    +"/ws/term/"+sid+"?token="+TOKEN);
  ws.binaryType="arraybuffer";
  ws.onmessage=e=>t.write(new Uint8Array(e.data));
  ws.onclose=()=>{try{t.write("\r\n\x1b[2m[連線已斷——點分頁重連]\x1b[0m\r\n");}catch(err){}
    rec.t=null;rec.ws=null;host.remove();rec.el=null;};
  t.onData(d=>{if(ws.readyState===1)ws.send(JSON.stringify({t:"i",d}));});
  t.onResize(({cols,rows})=>{if(ws.readyState===1)
    ws.send(JSON.stringify({t:"r",c:cols,r:rows}));});
  ws.onopen=()=>{fitTerm({t,fit});t.focus();};
  Object.assign(rec,{t,fit,ws,el:host});
  for(const [id,r] of terms)if(r.el)r.el.classList.toggle("on",id===sid);
}
function fitTerm(rec){try{rec.fit&&rec.fit.fit();}catch(e){}}
function fitAll(){for(const r of terms.values())if(r.t)fitTerm(r);}
async function newTerm(kind,extra){
  const payload=Object.assign({kind},kind==="agent"?scopePayload():{},extra||{});
  if(kind==="agent")payload.agent=$("agentsel").value||"claude";
  const r=await api("/api/term_create",post(payload));
  if(!terms.has(r.sid))addTab(r.sid,r.title);
  selectTerm(r.sid);
  toast(kind==="agent"?(payload.agent+" 會話已開（料已打包）"):"shell 已開");}
$("newshell").onclick=()=>newTerm("shell");
$("newagent").onclick=()=>newTerm("agent");

// 拖曳調整高度 ＋ Ctrl+` 收合
let dragging=false,lastH=280;
$("termdrag").onmousedown=e=>{dragging=true;e.preventDefault();};
window.addEventListener("mousemove",e=>{if(!dragging)return;
  const h=Math.min(Math.max(window.innerHeight-e.clientY-26,42),
                   window.innerHeight-200);
  document.documentElement.style.setProperty("--termh",h+"px");fitAll();});
window.addEventListener("mouseup",()=>{if(dragging){dragging=false;fitAll();}});
window.addEventListener("keydown",e=>{
  if(e.ctrlKey&&e.key==="`"){e.preventDefault();
    const cur=getComputedStyle(document.documentElement)
      .getPropertyValue("--termh").trim();
    if(cur==="42px"){document.documentElement.style
      .setProperty("--termh",lastH+"px");}
    else{lastH=parseInt(cur)||280;document.documentElement.style
      .setProperty("--termh","42px");}
    fitAll();}});
window.addEventListener("resize",fitAll);

(async()=>{await refreshState();resetScope();rescan();})();
setInterval(refreshState,5000);   // 殼只 poll 脊椎：輕量輪詢
setInterval(rescan,120000);       // P3 採集低頻；手動按鈕隨時可補
</script></body></html>
"""
