import { vi } from "vitest";
import type { Backend, TermStream } from "../api/backend";
import type { ScanResult, StateCard, WorkbenchState } from "../api/types";
import { EMPTY_SCAN, EMPTY_STATE } from "../store/store";

export const interruptCard: StateCard = {
  key: "collision:09:10",
  kind: "interrupt",
  lane: "scope",
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
    { label: "重跑判定", action: "collide_rerun", payload: { cid: "c1" }, primary: true },
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
  lane: "scope",
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
    { label: "開碰撞", action: "collide_prefill", payload: { text: "驗證 VDI" }, primary: true },
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
    // 引擎照請求的範圍算好的成員（預設＝所有 repo）
    scope: { group: null, repos: null, members: over.repos ?? ["a", "b"] },
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

// ── F-127：決策卡假資料 ──

import type { DecideCard, DecideQueue, Decided } from "../api/types";

export function decideCard(over: Partial<DecideCard> = {}): DecideCard {
  return {
    repo: "a",
    group: "產品開發",
    about: "登入流程與帳號設定的前端",
    role: "實作",
    path: "/r/a",
    handoff_ref: "decision:2026-11-14-d1",
    handoff_at: "2026-11-14 09:00",
    days: 0,
    hours: 3,
    structured: true,
    question: "登入頁的錯誤訊息改紅字，要 commit 嗎？",
    kind: "commit",
    options: [
      { n: 1, text: "要，commit", recommended: true },
      { n: 2, text: "先不要", recommended: false },
      { n: 3, text: "改完測試再說", recommended: false },
    ],
    recommend: "要，commit",
    reversible: true,
    evidence: [
      {
        ref: "src/Login.tsx",
        path: "/r/a/src/Login.tsx",
        exists: true,
        repo: "a",
        rel: "src/Login.tsx",
      },
      {
        ref: "docs/none.md",
        path: "/r/a/docs/none.md",
        exists: false,
        repo: "a",
        rel: "docs/none.md",
      },
    ],
    relay_status: "needs_you",
    notes: [],
    advice: {
      ref: "chosen:2026-11-14-q9",
      at: "2026-11-14 10:00",
      repo: "a",
      handoff_ref: "decision:2026-11-14-d1",
      from: { kind: "steward" },
      text: "要，commit",
      option: 1,
      why: "diff 只動兩個檔、測試過了",
    },
    class: "程式",
    done: "改了錯誤訊息的顏色與按鈕對齊",
    remaining: null,
    next_step: "等使用者決定要不要 commit",
    ...over,
  };
}

export function decideQueue(cards: DecideCard[]): DecideQueue {
  return {
    scope: "全部",
    at: "2026-11-14 12:00",
    cards,
    structured: cards.length,
    suggestions: [],
  };
}

export function decided(over: Partial<Decided> = {}): Decided {
  return {
    ref: "chosen:2026-11-14-q10",
    at: "2026-11-14 12:01",
    repo: "a",
    handoff_ref: "decision:2026-11-14-d1",
    kind: "commit",
    follow: "followed",
    by: "human",
    reversible: true,
    rule: null,
    question: "登入頁的錯誤訊息改紅字，要 commit 嗎？",
    answer: "要，commit",
    recommend: "要，commit",
    overturned_by: null,
    option: 1,
    relay: {
      repo: "a",
      path: "/r/a",
      handoff_at: "2026-11-14 09:00",
      handoff_ref: "decision:2026-11-14-d1",
      next_step: "等使用者決定",
      task: "接續 a 上次的交接…使用者對「…」的回答是：要，commit",
      delivery: { kind: "new_session", cwd: "/r/a" },
      commands: [],
      redacted: 0,
    },
    ...over,
  };
}
