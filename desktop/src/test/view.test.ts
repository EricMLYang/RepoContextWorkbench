import { describe, expect, it } from "vitest";
import type { InboxCard, QuestionCard } from "../api/types";
import {
  draggedFileAbs,
  firstNeedingYou,
  reportLine,
  sessionNeedsYou,
  pathForTerminal,
  actionLabel,
  actionToast,
  cardCounts,
  cardSections,
  emptyState,
  primaryAction,
  scopedCards,
  scopeKindLabel,
  scopeLabel,
  sessionPayload,
} from "../lib/view";
import { interruptCard, loopCard, makeScan, makeState } from "./fixtures";
import { tabLabels } from "../components/viewer/FileTabs";

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
    expect(actionLabel(primaryAction(loopCard)!)).toBe("請 Agent 分析這個問題…");
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

describe("拖檔到終端", () => {
  it("agent 貼 @路徑；shell 有特殊字元才加引號；後面一個空白、不換行", () => {
    expect(pathForTerminal("/r/a/README.md", "agent")).toBe("@/r/a/README.md ");
    expect(pathForTerminal("/r/a/README.md", "shell")).toBe("/r/a/README.md ");
    expect(pathForTerminal("/r/my notes/it's.md", "shell")).toBe("'/r/my notes/it'\\''s.md' ");
    expect(pathForTerminal("C:\\r\\my notes\\a.md", "shell", true)).toBe(
      '"C:\\r\\my notes\\a.md" ',
    );
  });
  it("只認我們拖的資料", () => {
    expect(draggedFileAbs(JSON.stringify({ repo: "a", path: "x", abs: "/r/a/x" }))).toBe("/r/a/x");
    expect(draggedFileAbs("")).toBeNull();
    expect(draggedFileAbs("not json")).toBeNull();
    expect(draggedFileAbs(JSON.stringify({ abs: 3 }))).toBeNull();
  });
});

describe("會話在等你（Claude Code hook 自動寫）", () => {
  const term = (agent_status: unknown, alive = true) =>
    ({
      sid: "t1",
      title: "claude · a",
      alive,
      kind: "agent",
      origin: null,
      scope: "a",
      scope_repos: ["a"],
      started: "2026-10-14 01:00",
      task: null,
      report: null,
      report_basis: "",
      agent_status,
    }) as never;
  it("hook 寫的標 Claude Code、agent 自己回報的標 Agent 回報；結束的會話不算在等你", () => {
    const hook = {
      status: "waiting",
      text: "這一輪做完了，等你回覆",
      at: "2026-10-14 01:02",
      event: "stop",
    };
    expect(reportLine(term(hook))).toBe("Claude Code「等你回應」01:02：這一輪做完了，等你回覆");
    expect(reportLine(term({ ...hook, event: undefined }))).toContain("Agent 回報「等你回應」");
    expect(sessionNeedsYou(term(hook))).toBe(true);
    expect(sessionNeedsYou(term({ ...hook, status: "working" }))).toBe(false);
    expect(sessionNeedsYou(term(hook, false))).toBe(false);
    expect(sessionNeedsYou(term(null))).toBe(false);
  });
  it("最該先處理的：卡住的優先，同類取最早開始的；沒有＝null", () => {
    const t = (sid: string, status: string | null, started: string, alive = true) =>
      ({
        ...(term(status ? { status, text: "", at: "2026-10-14 01:00" } : null, alive) as object),
        sid,
        started,
      }) as never;
    expect(firstNeedingYou([t("a", "working", "1")])).toBeNull();
    const w1 = t("w1", "waiting", "2026-10-14 01:00");
    const w0 = t("w0", "waiting", "2026-10-14 00:50");
    const b = t("b", "blocked", "2026-10-14 01:10");
    const dead = t("d", "blocked", "2026-10-14 00:10", false);
    expect((firstNeedingYou([w1, w0]) as { sid: string }).sid).toBe("w0");
    expect((firstNeedingYou([w1, w0, b, dead]) as { sid: string }).sid).toBe("b");
  });
});

describe("cardCounts（UX-04：未讀不算待辦）", () => {
  it("照 kind 分：待跟進、要你判斷、未讀、分析中", () => {
    const cards = ["loop", "loop", "retro", "collision", "event", "fresh", "pending"].map(
      (kind, i) => ({ key: String(i), kind }) as never,
    );
    expect(cardCounts(cards)).toEqual({ loop: 2, decide: 2, unread: 2, pending: 1 });
  });
});

describe("tabLabels（UX-08：分頁只顯示檔名，同名才補最短路徑）", () => {
  it("不同名只有檔名；同名補上層目錄；路徑相同補 repo", () => {
    expect(
      tabLabels([
        { repo: "a", path: "docs/README.md" },
        { repo: "a", path: "src/README.md" },
        { repo: "a", path: "src/main.rs" },
        { repo: "b", path: "src/main.rs" },
        { repo: "a", path: "x/y/z.md" },
      ]),
    ).toEqual([
      ["README.md", "docs"],
      ["README.md", "src"],
      ["main.rs", "a › src"],
      ["main.rs", "b › src"],
      ["z.md", ""],
    ]);
  });
});

describe("單一 repo 的範圍不講「組」（#73）", () => {
  it("scopeLabel／scopeKindLabel：一個 repo 叫 repo 名", () => {
    expect(scopeLabel({ group: null, repos: ["a"] })).toBe("a");
    expect(scopeKindLabel({ group: null, repos: ["a"] })).toBe("單一 repo");
    expect(scopeLabel({ group: null, repos: ["a", "b"] })).toBe("臨時工作組");
    expect(scopeLabel({ group: "g", repos: null })).toBe("g");
  });
});
