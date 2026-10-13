import { describe, expect, it, vi } from "vitest";
import { MSG_BAD_RESPONSE, MSG_UNREACHABLE } from "../api/backend";
import { createHttpBackend, scanQuery } from "../api/http";
import { createSpineApi, createTauriBackend, decodeTermMessage, toError } from "../api/tauri";

function res(status: number, body: string) {
  return new Response(body, { status, headers: { "Content-Type": "application/json" } });
}

describe("httpBackend", () => {
  it("帶 token、POST JSON、回傳資料", async () => {
    const fetch = vi.fn(async () => res(200, JSON.stringify({ ok: true, sid: "t1", title: "x" })));
    const b = createHttpBackend({ token: "tok", base: "", fetch });
    const r = await b.action("term_create", { kind: "shell" });
    expect(r.sid).toBe("t1");
    const [url, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("/api/term_create");
    expect(init.method).toBe("POST");
    expect((init.headers as Record<string, string>)["X-Auth"]).toBe("tok");
    expect(JSON.parse(String(init.body))).toEqual({ kind: "shell" });
  });

  it("後端 {error} → 拋出那句話", async () => {
    const fetch = vi.fn(async () => res(400, JSON.stringify({ error: "想法不可為空" })));
    const b = createHttpBackend({ token: "", base: "", fetch });
    await expect(b.action("collide", { idea: "" })).rejects.toThrow("想法不可為空");
  });

  it("網路失敗與代理錯誤都說連不上；200 卻不是 JSON 說回應不完整", async () => {
    const down = createHttpBackend({
      token: "",
      base: "",
      fetch: vi.fn(async () => Promise.reject(new TypeError("x"))),
    });
    await expect(down.getState()).rejects.toThrow(MSG_UNREACHABLE);
    const proxy = createHttpBackend({
      token: "",
      base: "",
      fetch: vi.fn(async () => res(502, "Bad Gateway")),
    });
    await expect(proxy.getState()).rejects.toThrow(MSG_UNREACHABLE);
    const broken = createHttpBackend({
      token: "",
      base: "",
      fetch: vi.fn(async () => res(200, "<html>")),
    });
    await expect(broken.getState()).rejects.toThrow(MSG_BAD_RESPONSE);
  });

  it("state 帶目前範圍（同 scan 的參數）", async () => {
    const fetch = vi.fn(async () => res(200, JSON.stringify({ groups: [] })));
    const b = createHttpBackend({ token: "", base: "", fetch });
    await b.getState({ group: "g1" });
    expect((fetch.mock.calls[0] as unknown as [string])[0]).toBe("/api/state?group=g1");
  });

  it("scan 參數：repos 優先、逗號串接", () => {
    expect(scanQuery({})).toBe("");
    expect(scanQuery({ group: "產品 開發" })).toBe("?group=" + encodeURIComponent("產品 開發"));
    expect(scanQuery({ repos: ["a", "b"], group: "g" })).toBe("?repos=a%2Cb");
  });
});

describe("tauriBackend", () => {
  it("命令名稱與參數照 IPC.md", async () => {
    const invoke = vi.fn(async () => ({ ok: true }));
    const b = createTauriBackend({ invoke });
    await b.getState();
    await b.getState({ repos: ["a"] });
    await b.getScan({ group: "g" });
    await b.action("ack", {});
    expect(invoke.mock.calls).toEqual([
      ["get_state", { group: null, repos: null }],
      ["get_state", { group: null, repos: ["a"] }],
      ["get_scan", { group: "g", repos: null }],
      ["run_action", { name: "ack", payload: {} }],
    ]);
  });

  it("spine 定位：get_spine／set_spine 照 IPC.md", async () => {
    const invoke = vi.fn(async (cmd: string) =>
      cmd === "get_spine" ? { spine: null } : { spine: "/s" },
    );
    const api = createSpineApi({ invoke });
    expect(await api.getSpine()).toEqual({ spine: null });
    expect(await api.setSpine("~/s")).toEqual({ spine: "/s" });
    expect(await api.createSpine("~/new")).toEqual({ spine: "/s" });
    expect(invoke.mock.calls).toEqual([
      ["get_spine", undefined],
      ["set_spine", { path: "~/s" }],
      ["create_spine", { path: "~/new" }],
    ]);
    const bad = createSpineApi({ invoke: vi.fn(async () => Promise.reject("不是 spine 目錄")) });
    await expect(bad.setSpine("/x")).rejects.toThrow("不是 spine 目錄");
  });

  it("Rust 的 Err(String) → Error(同一句話)", async () => {
    const b = createTauriBackend({
      invoke: vi.fn(async () => Promise.reject("至少選擇一個 repo；空範圍不代表全部")),
    });
    await expect(b.action("collide", { idea: "x", repos: [] })).rejects.toThrow(
      "至少選擇一個 repo",
    );
    expect(toError({ message: "m" }).message).toBe("m");
  });

  it("終端：Channel 輸出解碼（ArrayBuffer／number[]／base64／closed）", () => {
    const ab = new Uint8Array([104, 105]).buffer;
    expect(decodeTermMessage(ab)).toEqual({ kind: "data", bytes: new Uint8Array([104, 105]) });
    expect(decodeTermMessage([104, 105])).toEqual({
      kind: "data",
      bytes: new Uint8Array([104, 105]),
    });
    expect(decodeTermMessage({ t: "data", b64: btoa("hi") })).toEqual({
      kind: "data",
      bytes: new Uint8Array([104, 105]),
    });
    expect(decodeTermMessage({ t: "closed", reason: "exited" })).toEqual({ kind: "closed" });
  });

  it("終端：term_open 帶 Channel，write／resize／close 帶 sid 與 attachId", async () => {
    const invoke = vi.fn(async (cmd: string) => (cmd === "term_open" ? { attach_id: "A1" } : null));
    class FakeChannel {
      onmessage: (m: unknown) => void = () => {};
    }
    const b = createTauriBackend({ invoke, Channel: FakeChannel as never });
    const s = b.openTerminal("t1");
    const opened = new Promise<void>((r) => s.onOpen(r));
    await opened;
    s.write("ls\r");
    s.resize(80, 24);
    s.close();
    const calls = invoke.mock.calls as unknown as [string, Record<string, unknown>][];
    expect(calls[0][0]).toBe("term_open");
    expect(calls[0][1].sid).toBe("t1");
    expect(calls[0][1].onEvent).toBeInstanceOf(FakeChannel);
    expect(calls.slice(1)).toEqual([
      ["term_write", { sid: "t1", data: "ls\r" }],
      ["term_resize", { sid: "t1", cols: 80, rows: 24 }],
      ["term_close", { sid: "t1", attachId: "A1" }],
    ]);
  });
});
