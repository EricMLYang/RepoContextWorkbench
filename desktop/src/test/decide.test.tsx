import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import type { DecideCard, Decided } from "../api/types";
import { WorkbenchStore } from "../store/store";
import { imeGuard } from "../lib/ime";
import {
  decideCard,
  decideQueue,
  decided,
  interruptCard,
  loopCard,
  makeState,
  mockBackend,
} from "./fixtures";

type Handler = (name: string, p: Record<string, unknown>) => unknown;

function backend(cards: DecideCard[], handle: Handler = () => ({ ok: true }), autoN = 0) {
  return mockBackend({
    getState: vi.fn(async () =>
      makeState({
        cards: [interruptCard, loopCard],
        decide: {
          count: cards.length,
          all: cards.length + 2,
          auto_recent: autoN,
          key: String(cards.length),
        },
      }),
    ),
    action: vi.fn(async (name: string, p: Record<string, unknown>) =>
      name === "decide_queue" ? decideQueue(cards) : handle(name, p),
    ) as never,
  });
}

async function mount(b: ReturnType<typeof backend>, group: string | null = null) {
  const store = new WorkbenchStore(b, { group, repos: null });
  render(<App store={store} autoStart={false} />);
  await act(async () => {
    await store.refreshAll();
  });
  await waitFor(() => expect(b.action).toHaveBeenCalledWith("decide_queue", expect.anything()));
  return store;
}

const key = (k: string, over: KeyboardEventInit = {}) =>
  act(async () => {
    fireEvent.keyDown(window, { key: k, ...over });
  });

beforeEach(() => localStorage.clear());
afterEach(cleanup);

describe("要你決定（F-127）", () => {
  it("沒有卡時整區不出現，其他卡照舊", async () => {
    await mount(backend([]));
    expect(document.getElementById("decide")).toBeNull();
    expect(document.querySelector("details.othercards")).toBeNull();
    expect(screen.getByText(/待跟進 · 1/)).toBeTruthy();
  });

  it("聚焦一張、其他是短列；建議是唯一的主要按鈕；證據、建議、交接全文", async () => {
    const b = backend([
      decideCard(),
      decideCard({
        repo: "b",
        question: "push 到 origin？",
        kind: "push",
        reversible: false,
        options: [],
        recommend: null,
        advice: null,
        handoff_ref: "decision:x",
      }),
    ]);
    await mount(b);
    const area = document.getElementById("decide")!;
    expect(area.querySelector(".decideq")!.textContent).toBe(
      "登入頁的錯誤訊息改紅字，要 commit 嗎？",
    );
    const primaries = area.querySelectorAll(".decideopts button.primary");
    expect(primaries.length).toBe(1);
    expect(primaries[0].textContent).toContain("建議");
    expect(area.textContent).toContain("登入流程與帳號設定的前端");
    expect(area.textContent).toContain("docs/none.md（不存在）");
    expect(area.textContent).toContain("總管建議：1) 要，commit（diff 只動兩個檔、測試過了）");
    expect(area.querySelector(".decidefull")!.textContent).toContain("改了錯誤訊息的顏色");
    const rows = area.querySelectorAll(".decidelist li");
    expect(rows.length).toBe(1);
    expect(rows[0].textContent).toContain("push 到 origin？");
    // 其他類型收在「其他（N）」底下，預設摺起來
    const other = document.querySelector("details.othercards") as HTMLDetailsElement;
    expect(other.open).toBe(false);
    expect(other.textContent).toContain("其他（2）");
  });

  it("鍵盤：數字標記、Esc 取消、Enter 才送；↓ 換卡；焦點在輸入框時不攔", async () => {
    const answer = vi.fn<(p: unknown) => Decided>(() => decided());
    const b = backend(
      [decideCard(), decideCard({ repo: "b", handoff_ref: "decision:x", question: "第二張" })],
      (n, p) => (n === "decide_answer" ? answer(p as never) : { ok: true }),
    );
    await mount(b);
    await key("2");
    const opt2 = screen.getByRole("button", { name: /先不要/ });
    expect(opt2.getAttribute("aria-pressed")).toBe("true");
    await key("Escape");
    expect(opt2.getAttribute("aria-pressed")).toBe("false");
    await key("9"); // 超過選項數：不標記
    expect(document.querySelector(".decideopts .marked")).toBeNull();
    // 換卡
    await key("ArrowDown");
    expect(document.querySelector(".decideq")!.textContent).toBe("第二張");
    await key("k");
    expect(document.querySelector(".decideq")!.textContent).toContain("登入頁");
    // 焦點在搜尋框：數字照常打字
    const search = document.getElementById("search") as HTMLInputElement;
    search.focus();
    await act(async () => {
      fireEvent.keyDown(search, { key: "2" });
    });
    expect(document.querySelector(".decideopts .marked")).toBeNull();
    search.blur();
    // 標記後 Enter 送出
    await key("2");
    await key("Enter");
    await waitFor(() => expect(answer).toHaveBeenCalledWith({ repo: "a", reply: "2" }));
  });

  it("輸入法選字中的 Enter 不送", async () => {
    const answer = vi.fn<(p: unknown) => Decided>(() => decided());
    const b = backend([decideCard({ options: [], recommend: null })], (n, p) =>
      n === "decide_answer" ? answer(p as never) : { ok: true },
    );
    await mount(b);
    const box = screen.getByRole("textbox", { name: /回答「/ }) as HTMLInputElement;
    fireEvent.change(box, { target: { value: "好" } });
    await act(async () => {
      fireEvent.keyDown(box, { key: "Enter", isComposing: true });
    });
    expect(answer).not.toHaveBeenCalled();
    imeGuard.webkit = false;
    await act(async () => {
      fireEvent.submit(box.form!);
    });
    await waitFor(() => expect(answer).toHaveBeenCalledWith({ repo: "a", reply: "好" }));
  });

  it("回答後的下一步：開新會話預填任務；接回原會話；之後再送", async () => {
    let next = decided();
    const calls: [string, unknown][] = [];
    const b = backend([decideCard()], (n, p) => {
      calls.push([n, p]);
      if (n === "decide_answer") return next;
      if (n === "term_create") return { ok: true, sid: "t9", title: "claude · a" };
      return { ok: true };
    });
    await mount(b);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /要，commit/ }));
    });
    expect(await screen.findByText(/已記下：a「登入頁/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "開新會話接著做…" }));
    const dlg = await screen.findByRole("dialog");
    expect(dlg.textContent).toContain("repo〈a〉");
    expect((dlg.querySelector("textarea") as HTMLTextAreaElement).value).toContain(
      "的回答是：要，commit",
    );
    cleanup();
    // 接回原會話
    next = decided({
      relay: {
        ...decided().relay,
        delivery: { kind: "resume", claude_session_id: "abc", cwd: "/r/a" },
      },
    });
    await mount(
      backend([decideCard()], (n, p) => {
        calls.push([n, p]);
        if (n === "decide_answer") return next;
        if (n === "term_create") return { ok: true, sid: "t9", title: "claude · a" };
        return { ok: true };
      }),
    );
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /要，commit/ }));
    });
    await act(async () => {
      fireEvent.click(await screen.findByRole("button", { name: "接回原會話接著做" }));
    });
    expect(calls).toContainEqual([
      "term_create",
      expect.objectContaining({ kind: "agent", resume: "abc", cwd: "/r/a" }),
    ]);
    cleanup();
    // 之後再送：不開任何會話
    calls.length = 0;
    next = decided();
    await mount(
      backend([decideCard()], (n, p) => {
        calls.push([n, p]);
        return n === "decide_answer" ? next : { ok: true };
      }),
    );
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /要，commit/ }));
    });
    fireEvent.click(await screen.findByRole("button", { name: "之後再送" }));
    expect(calls.map((c) => c[0])).not.toContain("term_create");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("送不出去（busy）：卡片留著並顯示原因", async () => {
    const b = backend([decideCard()], (n) => {
      if (n === "decide_answer") throw new Error("a 的會話 t1 正在處理中，不要打斷");
      return { ok: true };
    });
    await mount(b);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /要，commit/ }));
    });
    expect(await screen.findByText(/正在處理中，不要打斷/)).toBeTruthy();
    expect(document.querySelector(".decideq")!.textContent).toContain("登入頁");
  });

  it("升級建議：設為自動要按兩次", async () => {
    const trust = vi.fn<(p: unknown) => unknown>(() => ({ rule: {}, previous: null, past: {} }));
    const b = backend([decideCard()], (n, p) => {
      if (n === "decide_answer")
        return decided({
          suggestion: {
            repo: "a",
            kind: "commit",
            streak: 3,
            command: "ctx decide trust a commit",
          },
        });
      if (n === "decide_trust") return trust(p as never);
      return { ok: true };
    });
    await mount(b);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /要，commit/ }));
    });
    expect(await screen.findByText(/已連續 3 次照建議/)).toBeTruthy();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "設為自動" }));
    });
    expect(trust).not.toHaveBeenCalled();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "再按一次，設為自動" }));
    });
    expect(trust).toHaveBeenCalledWith({ repo: "a", kind: "commit" });
  });

  it("「其他」的展開與否記住", async () => {
    await mount(backend([decideCard()]));
    const other = document.querySelector("details.othercards") as HTMLDetailsElement;
    other.open = true;
    fireEvent(other, new Event("toggle"));
    expect(localStorage.getItem("workbench:inbox:otherOpen")).toBe("true");
    cleanup();
    await mount(backend([decideCard()]));
    expect((document.querySelector("details.othercards") as HTMLDetailsElement).open).toBe(true);
  });

  it("自動放行面板：推翻要按兩次；規則可降回", async () => {
    const calls: string[] = [];
    const b = backend(
      [decideCard()],
      (n) => {
        calls.push(n);
        if (n === "decide_auto")
          return {
            since: "2026-11-07 12:00",
            items: [{ ...decided(), by: "rule", rule: "a/commit", ref: "chosen:p1", command: "x" }],
          };
        if (n === "decide_rules")
          return {
            file: "/s/policy.yaml",
            exists: true,
            rules: [
              {
                repo: "a",
                kind: "commit",
                level: "auto",
                since: "2026-11-14 10:00",
                paths: [],
                approvals: 1,
                overturned: 0,
              },
            ],
            suggestions: [],
            problems: ["第 2 條：level「x」看不懂"],
          };
        if (n === "decide_mine")
          return {
            since: "2026-11-07 12:00",
            items: [
              { ...decided(), ref: "chosen:h1", command: "ctx decide overturn chosen:h1" },
              {
                ...decided({ repo: "b", question: "舊的那題" }),
                ref: "chosen:h0",
                overturned_by: "chosen:h2",
                command: null,
              },
            ],
          };
        if (n === "decide_overturn")
          return {
            ref: "chosen:q11",
            what: "overturn",
            approval: { handoff_ref: "decision:d1" },
            rule: { repo: "a", kind: "commit" },
            note: null,
          };
        return { ok: true };
      },
      1,
    );
    await mount(b);
    fireEvent.click(screen.getByRole("button", { name: "回答與放行 · 自動 1" }));
    const dlg = await screen.findByRole("dialog");
    expect(dlg.textContent).toContain("你最近的回答");
    expect(dlg.textContent).toContain("已撤回");
    await screen.findByText(/問：舊的那題/);
    // 推翻、降回、撤回都看得出是按鈕（不是 ghost）
    for (const name of ["推翻", "降回每次都問", "撤回"])
      expect(screen.getByRole("button", { name }).className).not.toContain("ghost");
    expect(dlg.textContent).toContain("level「x」看不懂");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "推翻" }));
    });
    expect(calls).not.toContain("decide_overturn");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "再按一次推翻" }));
    });
    expect(calls).toContain("decide_overturn");
    expect(await screen.findByText(/降回每次都問，那張卡回到「要你決定」/)).toBeTruthy();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "降回每次都問" }));
    });
    expect(calls).toContain("decide_untrust");
  });

  it("面板裡撤回你最近的回答：按一次就撤回，提醒已送出的要另外說", async () => {
    const calls: [string, unknown][] = [];
    const b = backend([], (n, p) => {
      calls.push([n, p]);
      if (n === "decide_mine")
        return {
          since: "2026-11-07 12:00",
          items: [{ ...decided(), command: "ctx decide overturn chosen:2026-11-14-q10" }],
        };
      if (n === "decide_auto") return { since: "2026-11-07 12:00", items: [] };
      if (n === "decide_rules")
        return { file: "/s/policy.yaml", exists: false, rules: [], suggestions: [], problems: [] };
      if (n === "decide_overturn")
        return { ref: "chosen:q12", what: "withdraw", approval: decided(), rule: null, note: null };
      return { ok: true };
    });
    await mount(b);
    // 沒有卡時：從 ⋯ 選單進面板
    fireEvent.click(document.getElementById("inboxmore")!);
    fireEvent.click(await screen.findByRole("menuitem", { name: /回答、放行與規則/ }));
    await screen.findByText(/問：登入頁/);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "撤回" }));
    });
    expect(calls).toContainEqual(["decide_overturn", { ref: "chosen:2026-11-14-q10" }]);
    expect(
      await screen.findByText(/已撤回 a 的回答.*已經送給 agent 的，要另外跟它說/),
    ).toBeTruthy();
  });

  it("回答後點錯了：撤回一次就回到可回答的卡", async () => {
    let cards = [decideCard()];
    const calls: [string, unknown][] = [];
    const b = mockBackend({
      getState: vi.fn(async () =>
        makeState({
          decide: {
            count: cards.length,
            all: cards.length,
            auto_recent: 0,
            key: String(cards.length),
          },
        }),
      ),
      action: vi.fn(async (n: string, p: Record<string, unknown>) => {
        calls.push([n, p]);
        if (n === "decide_queue") return decideQueue(cards);
        if (n === "decide_answer") {
          cards = [];
          return decided();
        }
        if (n === "decide_overturn") {
          cards = [decideCard()];
          return {
            ref: "chosen:q12",
            what: "withdraw",
            approval: decided(),
            rule: null,
            note: null,
          };
        }
        return { ok: true };
      }) as never,
    });
    await mount(b);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /要，commit/ }));
    });
    expect(await screen.findByText(/已記下：a「登入頁/)).toBeTruthy();
    const undo = screen.getByRole("button", { name: "撤回" });
    expect(undo.className).not.toContain("primary");
    expect(screen.getByRole("button", { name: "之後再送" }).className).not.toContain("ghost");
    await act(async () => {
      fireEvent.click(undo);
    });
    expect(calls).toContainEqual(["decide_overturn", { ref: "chosen:2026-11-14-q10" }]);
    await waitFor(() =>
      expect(document.querySelector(".decideq")?.textContent).toContain("登入頁"),
    );
    expect(screen.queryByText(/已記下/)).toBeNull();
  });

  it("做了收不回來：醒目標示，沒有預設的主要按鈕（建議只標籤）", async () => {
    await mount(
      backend([
        decideCard({
          question: "push 到 origin/main？",
          kind: "push",
          reversible: false,
          options: [
            { n: 1, text: "push", recommended: true },
            { n: 2, text: "先不要", recommended: false },
          ],
          recommend: "push",
        }),
      ]),
    );
    const area = document.getElementById("decide")!;
    expect(area.querySelector(".decidewarn")!.textContent).toContain("做了收不回來");
    expect(area.querySelectorAll(".decideopts button.primary").length).toBe(0);
    expect(area.querySelector(".decideopts")!.textContent).toContain("建議");
  });

  it("總管列：全部範圍的件數、切到全部、開總管會話的範圍說明；組與 repo 的範圍說明", async () => {
    const store = await mount(backend([decideCard()]), "產品開發");
    const sel = screen.getByRole("button", { name: /總管：看所有 repo 要你決定的（3 件）/ });
    expect(sel.textContent).toBe("總管3");
    fireEvent.click(screen.getByRole("button", { name: "開總管會話" }));
    let dlg = await screen.findByRole("dialog");
    expect(dlg.textContent).toContain("總管：看全部 2 個 repo，只讀與留建議，不改檔");
    expect(dlg.textContent).not.toContain("有情境的起手");
    expect(dlg.textContent).not.toContain("隔離的 git worktree");
    fireEvent.click(screen.getByRole("button", { name: "關閉詳情" }));
    fireEvent.click(document.getElementById("newagent")!);
    dlg = await screen.findByRole("dialog");
    expect(dlg.textContent).toContain("組〈產品開發〉：2 個 repo");
    fireEvent.click(screen.getByRole("button", { name: "關閉詳情" }));
    await act(async () => {
      fireEvent.click(sel);
    });
    expect(store.getSnapshot().scope).toEqual({ group: null, repos: null });
  });
});
