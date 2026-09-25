"""WebSocket 最小子集（RFC6455，stdlib 手寫、零依賴）——內嵌 terminal 用。"""
import base64
import hashlib
import secrets
import struct


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
