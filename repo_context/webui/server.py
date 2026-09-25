"""HTTP server：token 驗證、/api/state｜/api/scan｜POST /api/<action>、/static、/ws/term。"""
import datetime as _dt
import json
import queue
import secrets
import threading
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

from .. import config as _config
from .. import term as _term
from .. import timer as _timer

from .actions import handle_action as _handle_action
from .state import PAGE_PATH, STATIC_DIR, annotate_terms, build_scan, build_state
from .ws import ws_accept_key, ws_encode, ws_read_frame


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
                    st["terms"] = annotate_terms(spine_dir,
                                                 self.server.terms.list())
                    st["agents"] = list(_config.load(spine_dir)["agents"])
                    self._json(st)
                elif u.path == "/api/scan":
                    q = parse_qs(u.query, keep_blank_values=True)
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


PAGE = PAGE_PATH.read_text(encoding="utf-8")
