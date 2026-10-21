import { describe, expect, it } from "vitest";
import { usageText } from "../components/Chrome";
import type { InboxCard, QuestionCard } from "../api/types";
import {
  sessionFailed,
  sessionGauges,
  draggedFileAbs,
  firstNeedingYou,
  reportLine,
  sessionDot,
  sessionFresh,
  sessionNeedsYou,
  sessionWantsYou,
  sessionHeadline,
  sessionMeta,
  sessionTooltip,
  statusText,
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
import { worktreeState } from "../components/deep/RepoWorktrees";

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
      text: "Claude is waiting for your input",
      at: "2026-10-14 01:02",
      event: "notification",
    };
    expect(reportLine(term(hook))).toBe(
      "Claude Code「等你回應」01:02：Claude is waiting for your input",
    );
    expect(reportLine(term({ ...hook, event: undefined }))).toContain("Agent 回報「等你回應」");
    expect(sessionNeedsYou(term(hook))).toBe(true);
    expect(sessionNeedsYou(term({ ...hook, status: "working" }))).toBe(false);
    expect(sessionNeedsYou(term(hook, false))).toBe(false);
    expect(sessionNeedsYou(term(null))).toBe(false);
  });
  it("F-80：做完一輪不算等你，是「做完沒看」；看過了就退掉；小圓點照狀態分", () => {
    const done = {
      status: "finished",
      text: "這一輪做完了",
      at: "2026-10-14 01:02",
      event: "stop",
    };
    expect(sessionNeedsYou(term(done))).toBe(false);
    expect(sessionFresh(term(done))).toBe(true);
    expect(sessionWantsYou(term(done))).toBe(true);
    expect(sessionFresh(term({ ...done, seen: true }))).toBe(false);
    expect(sessionWantsYou(term({ ...done, seen: true }))).toBe(false);
    expect(sessionFresh(term(done, false))).toBe(false);
    expect(sessionDot(term(done))).toEqual(["fresh"]);
    expect(sessionDot(term({ ...done, seen: true }))).toEqual(["seen"]);
    expect(sessionDot(term({ ...done, status: "working" }))).toEqual(["busy"]);
    // F-82：處理中帶工具與分鐘；超過 10 分改說可能卡住、小點加紅圈
    const busy = { ...(term({ ...done, status: "working" }) as object) } as never;
    expect(statusText(busy)).toBe("處理中");
    const withTool = {
      ...(busy as object),
      tool: { label: "Bash：cargo test", minutes: 3, stalled: false },
    } as never;
    expect(statusText(withTool)).toBe("處理中：Bash：cargo test（3 分）");
    expect(sessionDot(withTool)).toEqual(["busy"]);
    const stalled = {
      ...(busy as object),
      tool: { label: "Bash：cargo test", minutes: 12, stalled: true },
    } as never;
    expect(statusText(stalled)).toBe("可能卡住：Bash：cargo test（12 分沒結束）");
    expect(sessionDot(stalled)).toEqual(["stalled"]);
    expect(sessionMeta(stalled)).toContain("可能卡住");
    expect(sessionDot(term({ ...done, status: "blocked" }))).toEqual(["needs"]);
    expect(sessionDot(term(done, false))).toEqual(["dead"]);
    expect(sessionDot(term(null))).toEqual(["silent"]);
    // 最該先處理的：卡住 > 等你 > 做完沒看
    const t = (sid: string, status: string | null, seen = false) =>
      ({
        ...(term(status ? { status, text: "", at: "2026-10-14 01:00", seen } : null) as object),
        sid,
        started: "1",
      }) as never;
    expect(
      firstNeedingYou([t("f", "finished"), t("w", "waiting"), t("b", "blocked")]),
    ).toMatchObject({ sid: "b" });
    expect(firstNeedingYou([t("f", "finished"), t("w", "waiting")])).toMatchObject({ sid: "w" });
    expect(firstNeedingYou([t("f", "finished"), t("s", "finished", true)])).toMatchObject({
      sid: "f",
    });
    expect(firstNeedingYou([t("s", "finished", true), t("x", "working")])).toBeNull();
  });
  it("N-41：做完沒看超過一天就不亮；處理中 30 分沒動靜改說沒動靜", () => {
    const done = { status: "finished", text: "", at: "2026-10-14 01:02", faded: true };
    expect(sessionFresh(term(done))).toBe(false);
    expect(sessionWantsYou(term(done))).toBe(false);
    expect(sessionDot(term(done))).toEqual(["seen"]);
    const quiet = term({ status: "working", text: "", at: "2026-10-14 01:02", quiet_minutes: 42 });
    expect(statusText(quiet)).toBe("沒動靜 42 分");
    expect(sessionDot(quiet)).toEqual(["stalled"]);
  });
  it("F-71：有 Claude Code 取的標題就當主標題，提示框帶最後回覆", () => {
    const plain = term(null) as object;
    expect(sessionHeadline(plain as never)).toBe("claude · a");
    const named = {
      ...plain,
      ai_title: "修登入流程",
      last_reply: "改好了，要 commit 嗎？",
    } as never;
    expect(sessionHeadline(named)).toBe("修登入流程");
    const tip = sessionTooltip(named);
    expect(tip.startsWith("修登入流程\nclaude · a\n")).toBe(true);
    expect(tip.endsWith("\n最後回覆：改好了，要 commit 嗎？")).toBe(true);
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

describe("額度餘光（F-83）", () => {
  it("5h／7d 各一格，有重置時間就帶；沒收過就空字串", () => {
    expect(usageText(null)).toBe("");
    expect(usageText(undefined)).toBe("");
    expect(
      usageText({
        at: "2026-10-16 20:50",
        five_hour: { used: 62, resets: "23:10" },
        seven_day: { used: 31, resets: null },
        context: 41,
        model: "Opus",
        warn: false,
      }),
    ).toBe("5h 62%（23:10 重置） · 7d 31%");
    expect(
      usageText({
        at: "x",
        five_hour: null,
        seven_day: { used: 90, resets: "10-20 09:00" },
        context: null,
        model: null,
        warn: true,
      }),
    ).toBe("7d 90%（10-20 09:00 重置）");
  });
});

describe("會話列小量表與出錯中斷（F-99～F-102）", () => {
  const base = {
    sid: "t1",
    title: "claude",
    alive: true,
    kind: "agent" as const,
    origin: null,
    scope: null,
    scope_repos: null,
    started: null,
    task: null,
    report: null,
    report_basis: "",
  };
  it("情境 %（80% 以上醒目）、子 agent、快取冷了；結束的與 shell 不畫", () => {
    const g = sessionGauges({
      ...base,
      context_pct: 85,
      model: "Opus",
      subagents: 2,
      cache_cold: true,
    });
    expect(g.map((x) => x.text)).toEqual(["情境 85%", "子 agent 2", "快取已冷"]);
    expect(g[0].warn).toBe(true);
    expect(g[0].title).toContain("Opus");
    expect(sessionGauges({ ...base, context_pct: 30 })[0].warn).toBe(false);
    expect(sessionGauges({ ...base, alive: false, context_pct: 30 })).toEqual([]);
    expect(sessionGauges({ ...base, kind: "shell", context_pct: 30 })).toEqual([]);
  });
  it("StopFailure 寫的狀態＝出錯中斷", () => {
    const st = { status: "waiting", text: "這一輪出錯中斷：額度", at: "x", event: "stop-failure" };
    expect(sessionFailed({ ...base, agent_status: st } as never)).toBe(true);
    expect(sessionFailed({ ...base, agent_status: { ...st, event: "stop" } } as never)).toBe(false);
  });
});

describe("worktree 狀態一句話（F-107）", () => {
  const w = {
    path: "/r/wt",
    branch: "x",
    head: "0123abcd",
    main: false,
    dirty: 0,
    unique: 0,
    locked: false,
    prunable: false,
    removable: true,
  };
  it("乾淨、沒 commit 的檔、只在這裡的 commit、鎖住、目錄不在", () => {
    expect(worktreeState(w)).toBe("乾淨");
    expect(worktreeState({ ...w, dirty: 2, unique: 1, locked: true })).toBe(
      "2 檔沒 commit · 1 個 commit 只在這裡 · 鎖住",
    );
    expect(worktreeState({ ...w, dirty: null })).toBe("目錄已不在");
    expect(worktreeState({ ...w, prunable: true })).toBe("目錄已不在");
  });
});
