import { vi } from "vitest";
import type { Backend, TermStream } from "../api/backend";
import type { ScanResult, StateCard, WorkbenchState } from "../api/types";
import { EMPTY_SCAN, EMPTY_STATE } from "../store/store";

export const interruptCard: StateCard = {
  key: "collision:09:10",
  kind: "interrupt",
  scope_kind: "group",
  scope_label: "產品開發",
  scope_repos: ["a", "b"],
  time: "09:10",
  date: "2026-10-06",
  etype: "collision",
  source: "engine",
  title: "判定失敗（system-unsure）",
  lines: ["provider 回傳非 JSON"],
  actions: [
    { label: "重跑判定", action: "collide_rerun", payload: { cid: "c1" } },
    {
      label: "忽略並記錄",
      action: "ignore",
      payload: { type: "collision", time: "09:10", header: "h" },
    },
  ],
};

export const loopCard: StateCard = {
  key: "loop:#3",
  kind: "loop",
  num: "#3",
  due: "2026-10-06",
  scope_kind: "repo",
  scope_label: "a",
  scope_repos: ["a"],
  time: "08:00",
  date: "2026-10-05",
  title: "驗證 VDI",
  lines: [],
  actions: [
    { label: "開碰撞", action: "collide_prefill", payload: { text: "驗證 VDI" } },
    { label: "延 7 天", action: "loop_defer", payload: { num: "#3", days: 7 } },
    { label: "關閉", action: "close_loop", payload: { num: "#3" } },
  ],
};

export function makeState(over: Partial<WorkbenchState> = {}): WorkbenchState {
  return {
    ...EMPTY_STATE,
    repos: ["a", "b"],
    groups: [{ name: "產品開發", members: ["a", "b"], goal: "", goal_at: "" }],
    tiers: { a: "active", b: "active" },
    tags: { a: [], b: [] },
    agents: ["claude"],
    cards: [interruptCard, loopCard],
    ...over,
  };
}

export function makeScan(over: Partial<ScanResult> = {}): ScanResult {
  return { ...EMPTY_SCAN, group: "全部", states: [], quiet: 2, ...over };
}

export function fakeStream(): TermStream {
  return {
    onData() {},
    onOpen() {},
    onClose() {},
    write() {},
    resize() {},
    close() {},
    connected: false,
  };
}

export function mockBackend(over: Partial<Backend> = {}) {
  const backend = {
    kind: "http" as const,
    getState: vi.fn(async () => makeState()),
    getScan: vi.fn(async () => makeScan()),
    action: vi.fn(async () => ({ ok: true })),
    openTerminal: vi.fn(() => fakeStream()),
    ...over,
  };
  return backend as typeof backend & Backend;
}
