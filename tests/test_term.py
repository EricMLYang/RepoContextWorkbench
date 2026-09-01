"""內嵌 terminal：L1 PTY 會話（echo／replay／kill）＋WS codec 純函數＋L2 真 WS 往返。"""
import base64
import io
import json
import os
import socket
import threading
import time
import urllib.request

import pytest

from repoengine import term, webui

posix_only = pytest.mark.skipif(os.name == "nt", reason="POSIX pty；Windows 走 pywinpty")


# ── L1：PTY 會話 ─────────────────────────────────────────────

@posix_only
def test_session_echo_replay_kill(tmp_path):
    got = []
    s = term.TermSession("t1", ["/bin/cat"], cwd=str(tmp_path))
    s.attach(got.append)
    s.write(b"PTY_OK_42\n")
    deadline = time.time() + 10
    while time.time() < deadline and b"PTY_OK_42" not in b"".join(got):
        time.sleep(0.05)
    joined = b"".join(got)
    assert joined.count(b"PTY_OK_42") >= 2  # pty echo ＋ cat 回吐
    # 晚到的訂閱者拿 replay（會話生命週期獨立於連線）
    replay = s.attach(lambda d: None)
    assert b"PTY_OK_42" in replay
    s.resize(100, 30)  # 不炸即可
    s.kill()
    deadline = time.time() + 10
    while time.time() < deadline and s.alive:
        time.sleep(0.05)
    assert not s.alive


@posix_only
def test_manager_shell_and_kill(spine):
    m = term.TermManager(spine)
    s = m.create_shell()
    assert m.list()[0]["sid"] == s.sid and m.list()[0]["alive"]
    assert m.kill(s.sid) and m.list() == []
    assert not m.kill("t999")  # 未知會話 → False


# ── L1：WS codec（RFC6455 最小子集）────────────────────────────

def test_ws_accept_key_rfc_vector():
    # RFC 6455 §1.3 的官方例子
    assert webui.ws_accept_key("dGhlIHNhbXBsZSBub25jZQ==") == \
        "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="


def test_ws_frame_roundtrip():
    for payload in (b"", b"hi", "中文資料".encode("utf-8"), b"x" * 70000):
        frame = webui.ws_encode(payload, opcode=2, mask=True)  # client→server 帶 mask
        opcode, out = webui.ws_read_frame(io.BytesIO(frame))
        assert opcode == 2 and out == payload
    frame = webui.ws_encode("text", opcode=1, mask=False)      # server→client 不 mask
    opcode, out = webui.ws_read_frame(io.BytesIO(frame))
    assert opcode == 1 and out == b"text"


# ── L2：真 WS 往返（create → handshake → input → echo 回來）────

@pytest.mark.e2e
@posix_only
def test_ws_terminal_roundtrip(spine_with_repos, monkeypatch):
    monkeypatch.setenv("SHELL", "/bin/sh")  # 測試用最素的 shell
    srv = webui.make_server(spine_with_repos, port=0)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    port, token = srv.server_address[1], srv.token
    try:
        req = urllib.request.Request(
            f"http://127.0.0.1:{port}/api/term_create",
            data=json.dumps({"kind": "shell"}).encode(),
            headers={"Content-Type": "application/json", "X-Auth": token},
            method="POST")
        with urllib.request.urlopen(req, timeout=10) as r:
            sid = json.loads(r.read())["sid"]

        sock = socket.create_connection(("127.0.0.1", port), timeout=10)
        key = base64.b64encode(os.urandom(16)).decode()
        sock.sendall((
            f"GET /ws/term/{sid}?token={token} HTTP/1.1\r\n"
            "Host: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        ).encode())
        f = sock.makefile("rb")
        assert b"101" in f.readline()
        accept = None
        while True:
            ln = f.readline().strip()
            if not ln:
                break
            if ln.lower().startswith(b"sec-websocket-accept:"):
                accept = ln.split(b":", 1)[1].strip().decode()
        assert accept == webui.ws_accept_key(key)

        sock.sendall(webui.ws_encode(
            json.dumps({"t": "i", "d": "echo WS_OK_42\n"}), opcode=1, mask=True))
        sock.sendall(webui.ws_encode(
            json.dumps({"t": "r", "c": 100, "r": 30}), opcode=1, mask=True))
        seen = b""
        deadline = time.time() + 15
        while time.time() < deadline and b"WS_OK_42" not in seen:
            opcode, payload = webui.ws_read_frame(f)
            if opcode == 2:
                seen += payload
        assert b"WS_OK_42" in seen  # 輸入進了 PTY、輸出走 WS 回來
        sock.sendall(webui.ws_encode(b"", opcode=8, mask=True))  # close
        sock.close()
    finally:
        srv.terms.kill_all()
        srv.shutdown()
        srv.server_close()
