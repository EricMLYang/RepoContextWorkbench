import { describe, expect, it } from "vitest";
import type { InboxCard } from "../api/types";
import {
  actionLabel,
  actionToast,
  cardSections,
  emptyState,
  primaryAction,
  sessionPayload,
} from "../lib/view";
import { interruptCard, loopCard, makeScan, makeState } from "./fixtures";

describe("按鈕文案與主次", () => {
  it("有建議落點就是「存入〈repo〉」", () => {
    const c = {
      ...interruptCard,
      kind: "collision",
      actions: [
        { label: "改落點…", action: "route_pick", payload: { cid: "x" } },
        { label: "照建議落", action: "route_collision", payload: { cid: "x", dest: "repo:MI_PM" } },
      ],
    } as unknown as InboxCard;
    const p = primaryAction(c)!;
    expect(p.action).toBe("route_collision");
    expect(actionLabel(p)).toBe("存入 MI_PM");
  });

  it("否則第一個非忽略／延後／改落點的動作", () => {
    expect(primaryAction(loopCard)!.action).toBe("collide_prefill");
    expect(actionLabel(primaryAction(loopCard)!)).toBe("分析這個問題");
  });

  it("toast 帶檔名／下次提醒／日期", () => {
    const a = { label: "x", action: "defer" as const, payload: {} };
    expect(actionToast(a, { file: "/a/b/c.md" })).toBe("x：完成 · c.md");
    expect(actionToast(a, { until: "2026-10-13" })).toBe("x：完成 · 下次提醒 2026-10-13");
  });
});

describe("收件匣分段", () => {
  it("別組的打斷進「全域警示」，未分類收合", () => {
    const state = makeState({
      repos: ["a", "b", "c"],
      groups: [
        { name: "g1", members: ["a"], goal: "", goal_at: "" },
        { name: "g2", members: ["c"], goal: "", goal_at: "" },
      ],
      cards: [
        { ...interruptCard, scope_repos: ["c"], scope_label: "g2" },
        { ...loopCard, key: "loop:#9", scope_kind: "unclassified", scope_repos: [], scope_label: "未分類" },
        loopCard,
      ],
    });
    const s = cardSections(state, makeScan(), { group: "g1", repos: null }, "", "all");
    expect(s.map((x) => [x.key, x.cards.length, !!x.collapsed])).toEqual([
      ["global", 1, false],
      ["loop", 1, false],
      ["unclassified", 1, true],
    ]);
  });
});

describe("空狀態", () => {
  const base = { stateReady: true, stateError: "", repoCount: 2, scanReady: true, scanError: "", filtered: false };
  it("四種＋篩選", () => {
    expect(emptyState({ ...base, stateReady: false }).title).toBe("正在載入工作空間");
    expect(emptyState({ ...base, stateReady: false, stateError: "x" }).title).toBe("無法載入工作空間");
    expect(emptyState({ ...base, repoCount: 0 }).onboard).toBe(true);
    expect(emptyState({ ...base, scanError: "x" }).title).toBe("無法確認目前狀態");
    expect(emptyState({ ...base, filtered: true }).title).toBe("沒有符合條件的事項");
    expect(emptyState(base).title).toBe("目前沒有待處理事項");
  });
});

describe("開會話 payload", () => {
  it("repo 優先於 group；group 蓋過 repos", () => {
    expect(sessionPayload({ group: "g" }, { repo: "a", repos: ["a"] })).toEqual({ repo: "a", repos: ["a"] });
    expect(sessionPayload({ repos: ["a", "b"] }, { group: "g" })).toEqual({ group: "g" });
    expect(sessionPayload({ group: "g" }, { group: null, repos: null })).toEqual({ group: null, repos: null });
  });
});
