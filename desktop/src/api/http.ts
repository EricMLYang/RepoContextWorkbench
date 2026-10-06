import {
  MSG_BAD_RESPONSE,
  MSG_FAILED,
  MSG_UNREACHABLE,
  type Backend,
  type TermStream,
} from "./backend";
import type { ScanParams } from "./types";

/**
 * Python 版工作台（`python3 -m repo_context ui`）的 HTTP／WS API。開發與對拍用。
 *
 * - token：網址 `?token=`，沒有就讀 `VITE_WORKBENCH_TOKEN`。
 * - base：預設同源（Vite dev server 代理 /api、/ws 到 8765）；`VITE_WORKBENCH_URL` 可直連。
 */
export interface HttpBackendOptions {
  token?: string;
  base?: string;
  fetch?: typeof fetch;
  WebSocket?: typeof WebSocket;
}

export function defaultToken(): string {
  const fromUrl =
    typeof location !== "undefined" ? new URLSearchParams(location.search).get("token") : null;
  return fromUrl || import.meta.env.VITE_WORKBENCH_TOKEN || "";
}

export function scanQuery(p: ScanParams): string {
  if (p.repos) return "?repos=" + encodeURIComponent(p.repos.join(","));
  if (p.group) return "?group=" + encodeURIComponent(p.group);
  return "";
}

export function createHttpBackend(opts: HttpBackendOptions = {}): Backend {
  const token = opts.token ?? defaultToken();
  const base = (opts.base ?? import.meta.env.VITE_WORKBENCH_URL ?? "").replace(/\/$/, "");
  const doFetch = opts.fetch ?? ((...a: Parameters<typeof fetch>) => fetch(...a));
  const WS = opts.WebSocket ?? (typeof WebSocket !== "undefined" ? WebSocket : undefined);

  async function api<T>(path: string, payload?: unknown): Promise<T> {
    let r: Response;
    try {
      r = await doFetch(base + path, {
        headers: {
          "X-Auth": token,
          ...(payload !== undefined ? { "Content-Type": "application/json" } : {}),
        },
        ...(payload !== undefined ? { method: "POST", body: JSON.stringify(payload) } : {}),
      });
    } catch {
      throw new Error(MSG_UNREACHABLE);
    }
    let data: { error?: string } & Record<string, unknown>;
    try {
      data = await r.json();
    } catch {
      // 非 JSON 的錯誤回應＝中間的代理（Vite dev server）連不到工作台
      throw new Error(r.ok ? MSG_BAD_RESPONSE : MSG_UNREACHABLE);
    }
    if (!r.ok || data.error) throw new Error(data.error || MSG_FAILED);
    return data as T;
  }

  function wsUrl(sid: string): string {
    let origin: string;
    if (base) origin = base.replace(/^http/, "ws");
    else origin = (location.protocol === "https:" ? "wss://" : "ws://") + location.host;
    return origin + "/ws/term/" + encodeURIComponent(sid) + "?token=" + encodeURIComponent(token);
  }

  return {
    kind: "http",
    getState: () => api("/api/state"),
    getScan: (p) => api("/api/scan" + scanQuery(p)),
    action: (name, payload) => api("/api/" + name, payload ?? {}),
    openTerminal(sid): TermStream {
      if (!WS) throw new Error("這個環境沒有 WebSocket");
      const ws = new WS(wsUrl(sid));
      ws.binaryType = "arraybuffer";
      let open = false;
      const dataCbs: ((b: Uint8Array) => void)[] = [];
      const openCbs: (() => void)[] = [];
      const closeCbs: (() => void)[] = [];
      let closed = false;
      const fireClose = () => {
        if (closed) return;
        closed = true;
        open = false;
        closeCbs.forEach((cb) => cb());
      };
      ws.onmessage = (e) => {
        const bytes =
          typeof e.data === "string"
            ? new TextEncoder().encode(e.data)
            : new Uint8Array(e.data as ArrayBuffer);
        dataCbs.forEach((cb) => cb(bytes));
      };
      ws.onopen = () => {
        open = true;
        openCbs.forEach((cb) => cb());
      };
      ws.onclose = fireClose;
      ws.onerror = fireClose;
      const send = (msg: unknown) => {
        if (ws.readyState === 1) ws.send(JSON.stringify(msg));
      };
      return {
        onData: (cb) => void dataCbs.push(cb),
        onOpen: (cb) => void openCbs.push(cb),
        onClose: (cb) => void closeCbs.push(cb),
        write: (d) => send({ t: "i", d }),
        resize: (c, r) => send({ t: "r", c, r }),
        close() {
          // 主動關閉不觸發 onClose（跟舊版 ws.onclose = null 一樣：不寫「連線已中斷」）
          closed = true;
          open = false;
          ws.close();
        },
        get connected() {
          return open;
        },
      };
    },
  };
}
