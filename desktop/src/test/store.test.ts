import { afterEach, describe, expect, it, vi } from "vitest";
import { WorkbenchStore } from "../store/store";
import { mockBackend } from "./fixtures";

describe("state 更新來源", () => {
  afterEach(() => vi.useRealTimers());

  it("後端有變動事件：只靠事件，閒置不輪詢", async () => {
    vi.useFakeTimers();
    let fire = () => {};
    const backend = mockBackend({
      onChanged: (cb) => {
        fire = cb;
        return () => {};
      },
    });
    const store = new WorkbenchStore(backend, { group: null, repos: null });
    store.start();
    await vi.advanceTimersByTimeAsync(100);
    const base = (backend.getState as ReturnType<typeof vi.fn>).mock.calls.length;
    await vi.advanceTimersByTimeAsync(30_000);
    expect((backend.getState as ReturnType<typeof vi.fn>).mock.calls.length).toBe(base);
    fire();
    await vi.advanceTimersByTimeAsync(10);
    expect((backend.getState as ReturnType<typeof vi.fn>).mock.calls.length).toBe(base + 1);
    store.stop();
  });

  it("沒有變動事件（HTTP 版）：每 5 秒輪詢", async () => {
    vi.useFakeTimers();
    const backend = mockBackend({ onChanged: undefined });
    const store = new WorkbenchStore(backend, { group: null, repos: null });
    store.start();
    await vi.advanceTimersByTimeAsync(100);
    const base = (backend.getState as ReturnType<typeof vi.fn>).mock.calls.length;
    await vi.advanceTimersByTimeAsync(10_050);
    expect((backend.getState as ReturnType<typeof vi.fn>).mock.calls.length).toBe(base + 2);
    store.stop();
  });
});
