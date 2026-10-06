import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { MSG_FAILED, MSG_UNREACHABLE, type Backend, type TermStream } from "./backend";

/**
 * Tauri v2 殼（Rust 引擎）的 IPC。合約全文見 desktop/IPC.md——改這裡要同步改那份。
 *
 * 命令：get_state／get_scan／run_action／term_open／term_write／term_resize／term_close；
 * spine 定位：get_spine／set_spine（createSpineApi）。
 * 事件：`workbench://changed`（引擎寫入脊椎／registry 後通知前端重抓 state，取代高頻輪詢）。
 */

/** term_open 的 Channel 收到的訊息：輸出位元組，或控制訊息。 */
export type TermChannelMessage =
  | ArrayBuffer // InvokeResponseBody::Raw——PTY 輸出（主要格式）
  | number[] // 容錯：Rust 端若以 Vec<u8> 走 JSON 序列化
  | { t: "data"; b64: string } // 容錯：base64 字串
  | { t: "closed"; reason: "detached" | "exited" | "error"; message?: string };

export interface TermOpenResult {
  attach_id: string;
}

type InvokeFn = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** invoke 的 reject 值 → Error（Rust 端 Err(String) 到這裡是字串）。 */
export function toError(e: unknown): Error {
  if (e instanceof Error) return e;
  if (typeof e === "string") return new Error(e || MSG_FAILED);
  if (e && typeof e === "object") {
    const o = e as { message?: unknown; error?: unknown };
    const m = o.message ?? o.error;
    if (typeof m === "string" && m) return new Error(m);
  }
  return new Error(MSG_UNREACHABLE);
}

export function decodeTermMessage(
  msg: TermChannelMessage,
): { kind: "data"; bytes: Uint8Array } | { kind: "closed" } | null {
  if (msg instanceof ArrayBuffer) return { kind: "data", bytes: new Uint8Array(msg) };
  if (ArrayBuffer.isView(msg))
    return {
      kind: "data",
      bytes: new Uint8Array(msg.buffer, msg.byteOffset, msg.byteLength),
    };
  if (Array.isArray(msg)) return { kind: "data", bytes: Uint8Array.from(msg) };
  if (msg && typeof msg === "object") {
    if (msg.t === "data") {
      const bin = atob(msg.b64);
      const out = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
      return { kind: "data", bytes: out };
    }
    if (msg.t === "closed") return { kind: "closed" };
  }
  return null;
}

/** `get_spine`／`set_spine` 的回傳：目前的 spine 路徑（未設定＝null）。 */
export interface SpineInfo {
  spine: string | null;
}

/**
 * spine 定位（只有桌面殼有；HTTP 版的 server 啟動時就綁好 spine）。
 * 找不到 spine（`--spine`／`REPOENGINE_SPINE`／使用者設定檔都沒有）→ 前端先顯示首次設定。
 */
export function createSpineApi(deps: { invoke?: InvokeFn } = {}) {
  const call = deps.invoke ?? (invoke as InvokeFn);
  const run = async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
    try {
      return (await call(cmd, args)) as T;
    } catch (e) {
      throw toError(e);
    }
  };
  return {
    getSpine: () => run<SpineInfo>("get_spine"),
    setSpine: (path: string) => run<SpineInfo>("set_spine", { path }),
  };
}

export function createTauriBackend(
  deps: { invoke?: InvokeFn; Channel?: typeof Channel } = {},
): Backend {
  const call = deps.invoke ?? (invoke as InvokeFn);
  const Chan = deps.Channel ?? Channel;
  const run = async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
    try {
      return (await call(cmd, args)) as T;
    } catch (e) {
      throw toError(e);
    }
  };

  return {
    kind: "tauri",
    getState: () => run("get_state"),
    getScan: (p) => run("get_scan", { group: p.group ?? null, repos: p.repos ?? null }),
    action: (name, payload) => run("run_action", { name, payload: payload ?? {} }),
    onChanged(cb) {
      let stop: (() => void) | null = null;
      let cancelled = false;
      listen("workbench://changed", () => cb())
        .then((un) => (cancelled ? un() : (stop = un)))
        .catch(() => {});
      return () => {
        cancelled = true;
        stop?.();
      };
    },
    openTerminal(sid): TermStream {
      const dataCbs: ((b: Uint8Array) => void)[] = [];
      const openCbs: (() => void)[] = [];
      const closeCbs: (() => void)[] = [];
      let open = false;
      let done = false;
      let attachId: string | null = null;
      const fireClose = () => {
        if (done) return;
        done = true;
        open = false;
        closeCbs.forEach((cb) => cb());
      };
      const channel = new Chan<TermChannelMessage>();
      channel.onmessage = (msg) => {
        const m = decodeTermMessage(msg);
        if (!m) return;
        if (m.kind === "data") dataCbs.forEach((cb) => cb(m.bytes));
        else fireClose();
      };
      // 先回傳物件讓呼叫端掛 handler，再非同步 attach（replay 會從 channel 進來）
      queueMicrotask(() => {
        run<TermOpenResult>("term_open", { sid, onEvent: channel })
          .then((r) => {
            if (done) {
              void run("term_close", { sid, attachId: r.attach_id }).catch(() => {});
              return;
            }
            attachId = r.attach_id;
            open = true;
            openCbs.forEach((cb) => cb());
          })
          .catch(fireClose);
      });
      return {
        onData: (cb) => void dataCbs.push(cb),
        onOpen: (cb) => void openCbs.push(cb),
        onClose: (cb) => void closeCbs.push(cb),
        write(data) {
          if (open) void run("term_write", { sid, data }).catch(() => {});
        },
        resize(cols, rows) {
          if (open) void run("term_resize", { sid, cols, rows }).catch(() => {});
        },
        close() {
          done = true;
          open = false;
          if (attachId) void run("term_close", { sid, attachId }).catch(() => {});
        },
        get connected() {
          return open;
        },
      };
    },
  };
}
