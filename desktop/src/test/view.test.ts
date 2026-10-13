import { describe, expect, it } from "vitest";
import type { InboxCard, QuestionCard } from "../api/types";
import {
  actionLabel,
  actionToast,
  cardSections,
  emptyState,
  primaryAction,
  scopedCards,
  sessionPayload,
} from "../lib/view";
import { interruptCard, loopCard, makeScan, makeState } from "./fixtures";

describe("按鈕文案與主次", () => {
  it("主要動作照引擎標的 primary；有建議落點時文案是「存入〈repo〉」", () => {
    const c = {
      ...interruptCard,
      kind: "collision",
      actions: [
        { label: "改落點…", action: "route_pick", payload: { cid: "x" } },
        {
          label: "照建議落",
          action: "route_collision",
          payload: { cid: "x", dest: "repo:MI_PM" },
          primary: true,
        },
      ],
    } as unknown as InboxCard;
    const p = primaryAction(c)!;
    expect(p.action).toBe("route_collision");
    expect(actionLabel(p)).toBe("存入 MI_PM");
  });

  it("引擎沒標 primary 就沒有主要動作（前端不自己挑）", () => {
    expect(primaryAction(loopCard)!.action).toBe("collide_prefill");
    expect(actionLabel(primaryAction(loopCard)!)).toBe("分析這個問題");
    const bare = {
      ...loopCard,
      actions: loopCard.actions.map((a) => ({ ...a, primary: undefined })),
    };
    expect(primaryAction(bare)).toBeUndefined();
  });

  it("toast 帶檔名／下次提醒／日期", () => {
    const a = { label: "x", action: "defer" as const, payload: {} };
    expect(actionToast(a, { file: "/a/b/c.md" })).toBe("x：完成 · c.md");
    expect(actionToast(a, { until: "2026-10-13" })).toBe("x：完成 · 下次提醒 2026-10-13");
  });
});

describe("收件匣分段（照引擎給的 lane，前端不判斷範圍）", () => {
  it("global 的打斷進「全域警示」、global 的其他收合、other 不顯示", () => {
    const state = makeState({
      cards: [
        { ...interruptCard, lane: "global", scope_repos: ["c"], scope_label: "g2" },
        {
          ...loopCard,
          key: "loop:#9",
          lane: "global",
          scope_kind: "unclassified",
          scope_repos: [],
          scope_label: "未分類",
        },
        { ...loopCard, key: "loop:#10", lane: "other" },
        loopCard,
      ],
    });
    const q: QuestionCard = {
      key: "question:a:dirty",
      kind: "question",
      qkind: "dirty",
      repo: "a",
      title: "a 有未提交檔放太久",
      actions: [],
      scope_kind: "repo",
      scope_label: "a",
      scope_repos: ["a"],
    };
    const s = cardSections(state, makeScan({ question_cards: [q] }), "", "all");
    expect(s.map((x) => [x.key, x.cards.length, !!x.collapsed])).toEqual([
      ["global", 1, false],
      ["question", 1, false],
      ["loop", 1, false],
      ["unclassified", 1, true],
    ]);
    expect(scopedCards(state, makeScan({ question_cards: [q] })).map((c) => c.key)).toEqual([
      "question:a:dirty",
      "loop:#3",
    ]);
  });

  it("搜尋與類型篩選只作用在顯示", () => {
    const state = makeState();
    expect(cardSections(state, makeScan(), "vdi", "all").map((x) => x.key)).toEqual(["loop"]);
    expect(cardSections(state, makeScan(), "", "interrupt").map((x) => x.key)).toEqual([
      "interrupt",
    ]);
  });
});

describe("空狀態", () => {
  const base = {
    stateReady: true,
    stateError: "",
    repoCount: 2,
    scanReady: true,
    scanError: "",
    filtered: false,
  };
  it("四種＋篩選", () => {
    expect(emptyState({ ...base, stateReady: false }).title).toBe("正在載入工作空間");
    expect(emptyState({ ...base, stateReady: false, stateError: "x" }).title).toBe(
      "無法載入工作空間",
    );
    expect(emptyState({ ...base, repoCount: 0 }).onboard).toBe(true);
    expect(emptyState({ ...base, scanError: "x" }).title).toBe("無法確認目前狀態");
    expect(emptyState({ ...base, filtered: true }).title).toBe("沒有符合條件的事項");
    expect(emptyState(base).title).toBe("目前沒有待處理事項");
  });
});

describe("開會話 payload", () => {
  it("repo 優先於 group；group 蓋過 repos", () => {
    expect(sessionPayload({ group: "g" }, { repo: "a", repos: ["a"] })).toEqual({
      repo: "a",
      repos: ["a"],
    });
    expect(sessionPayload({ repos: ["a", "b"] }, { group: "g" })).toEqual({ group: "g" });
    expect(sessionPayload({ group: "g" }, { group: null, repos: null })).toEqual({
      group: null,
      repos: null,
    });
  });
});
