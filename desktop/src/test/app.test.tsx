import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { SpineSetup } from "../components/SpineSetup";
import { WorkbenchStore } from "../store/store";
import type { Backend } from "../api";
import type { StateCard } from "../api/types";
import { FILE_DRAG_TYPE } from "../lib/view";
import { interruptCard, makeScan, makeState, mockBackend } from "./fixtures";

async function mount(backend = mockBackend()) {
  const store = new WorkbenchStore(backend, { group: null, repos: null });
  const utils = render(<App store={store} autoStart={false} />);
  await act(async () => {
    await store.refreshAll();
  });
  return { store, backend, ...utils };
}

beforeEach(() => localStorage.clear());
afterEach(() => cleanup());

describe("收件匣卡片按鈕", () => {
  it("主要動作呼叫卡片指定的 action 與 payload", async () => {
    const { backend } = await mount();
    const btn = await screen.findByRole("button", { name: "重新分析" });
    await act(async () => fireEvent.click(btn));
    expect(backend.action).toHaveBeenCalledWith("collide_rerun", { cid: "c1" });
    // 成功後重抓 state
    expect(backend.getState).toHaveBeenCalledTimes(2);
  });

  it("「更多」對話框裡的按鈕各自呼叫自己的 action", async () => {
    const { backend } = await mount();
    const card = (await screen.findByText("驗證 VDI")).closest("article")!;
    await act(async () => fireEvent.click(card.querySelector(".more")!));
    const dialog = document.getElementById("detaildialog")!;
    expect(dialog.hasAttribute("open")).toBe(true);
    const defer = [...dialog.querySelectorAll("button")].find((b) => b.textContent === "延 7 天")!;
    await act(async () => fireEvent.click(defer));
    expect(backend.action).toHaveBeenCalledWith("loop_defer", { num: "#3", days: 7 });
    expect(dialog.hasAttribute("open")).toBe(false);
  });

  it("動作執行中鎖住卡片按鈕，失敗把錯誤放在卡片上", async () => {
    let reject!: (e: Error) => void;
    const backend = mockBackend({
      action: vi.fn(() => new Promise((_, r) => (reject = r))) as never,
    });
    await mount(backend);
    const btn = (await screen.findByRole("button", { name: "重新分析" })) as HTMLButtonElement;
    act(() => void fireEvent.click(btn));
    await waitFor(() => expect(btn.disabled).toBe(true));
    await act(async () => reject(new Error("引擎拒絕")));
    const card = btn.closest("article")!;
    expect(card.querySelector(".carderror")!.textContent).toBe("引擎拒絕");
    expect(btn.disabled).toBe(false);
  });
});

describe("碰撞台草稿", () => {
  it("送出失敗不清草稿，並顯示原因", async () => {
    localStorage.setItem("workbench:draft", JSON.stringify("一個想法"));
    const backend = mockBackend({
      action: vi.fn(async () => {
        throw new Error("無法連接工作台。請確認本機服務仍在執行，再重試。");
      }) as never,
    });
    await mount(backend);
    const idea = document.getElementById("idea") as HTMLTextAreaElement;
    expect(idea.value).toBe("一個想法"); // 本機保留的草稿自動帶回、碰撞台展開
    expect(document.getElementById("composer")!.hidden).toBe(false);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "開始分析" })));
    expect(backend.action).toHaveBeenCalledWith("collide", { idea: "一個想法" });
    expect(idea.value).toBe("一個想法");
    expect(JSON.parse(localStorage.getItem("workbench:draft")!)).toBe("一個想法");
    expect(document.getElementById("ideaerror")!.textContent).toContain("草稿已保留");
  });

  it("送出成功才清草稿", async () => {
    localStorage.setItem("workbench:draft", JSON.stringify("另一個想法"));
    const { backend } = await mount();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "開始分析" })));
    expect(backend.action).toHaveBeenCalledWith("collide", { idea: "另一個想法" });
    expect((document.getElementById("idea") as HTMLTextAreaElement).value).toBe("");
    expect(JSON.parse(localStorage.getItem("workbench:draft")!)).toBe("");
  });

  it("空的想法不送出", async () => {
    const { backend } = await mount();
    act(() => void fireEvent.click(screen.getByRole("button", { name: "請 Agent 分析想法…" })));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "開始分析" })));
    expect(backend.action).not.toHaveBeenCalled();
    expect(document.getElementById("ideaerror")!.textContent).toBe("先寫下一個想法，再開始分析。");
  });
});

describe("斷線", () => {
  it("更新失敗：紅點＋橫幅，保留上次的卡片", async () => {
    const backend = mockBackend();
    const { store } = await mount(backend);
    expect(screen.getByText("驗證 VDI")).toBeTruthy();
    const down = new Error("無法連接工作台。請確認本機服務仍在執行，再重試。");
    vi.mocked(backend.getState).mockRejectedValue(down);
    vi.mocked(backend.getScan).mockRejectedValue(down);
    await act(async () => {
      await store.refreshAll();
    });
    const conn = document.getElementById("connection")!;
    expect(conn.textContent).toBe("連線或更新異常");
    expect(conn.className).toContain("offline");
    const banner = document.getElementById("syncerror")!;
    expect(banner.hidden).toBe(false);
    expect(banner.textContent).toContain("目前可能顯示上次成功取得的資料");
    expect(screen.getByText("驗證 VDI")).toBeTruthy(); // 上次資料還在
    expect(document.getElementById("quietline")!.textContent).toBe("");
    // 就地重試（UX-11）
    vi.mocked(backend.getState).mockResolvedValue(makeState());
    vi.mocked(backend.getScan).mockResolvedValue(makeScan());
    await act(async () => fireEvent.click(within(banner).getByRole("button", { name: "重試" })));
    expect(banner.hidden).toBe(true);
  });

  it("一開始就連不上：空狀態說無法載入", async () => {
    const backend = mockBackend({
      getState: vi.fn(async () => {
        throw new Error("無法連接工作台。");
      }),
    });
    await mount(backend);
    expect(document.querySelector("#allclear h2")!.textContent).toBe("無法載入工作空間");
    expect(document.getElementById("syncerror")!.hidden).toBe(false);
  });

  it("尚未登記 repo：顯示首次建組入口", async () => {
    const backend = mockBackend({
      getState: vi.fn(async () => makeState({ repos: [], groups: [], cards: [] })),
    });
    await mount(backend);
    expect(document.querySelector("#allclear h2")!.textContent).toBe("建立你的第一個工作範圍");
    expect(screen.getByRole("button", { name: "選擇資料夾，掃描 repo" })).toBeTruthy();
  });
});

describe("範圍", () => {
  it("點牌組切範圍並以該組重新採集", async () => {
    const { backend } = await mount();
    await act(async () =>
      fireEvent.click(screen.getByRole("button", { name: "選擇牌組：產品開發" })),
    );
    expect(backend.getScan).toHaveBeenLastCalledWith({ group: "產品開發" });
    // state 也以新範圍重抓（卡片／事件／關係的 lane 由引擎照範圍算）
    expect(backend.getState).toHaveBeenLastCalledWith({ group: "產品開發" });
    expect(document.getElementById("scopename")!.textContent).toBe("產品開發");
    expect(JSON.parse(localStorage.getItem("workbench:scope")!)).toEqual({
      group: "產品開發",
      repos: null,
    });
  });

  it("新建牌組：在牌組裡按＋不預勾；臨時組才預勾；編輯牌組預勾成員", async () => {
    const { store } = await mount();
    const checked = () =>
      [...document.querySelectorAll<HTMLInputElement>("#pickdialog input[type=checkbox]")]
        .filter((c) => c.checked)
        .map((c) => c.value || c.closest("label")!.textContent!.trim());
    await act(async () =>
      fireEvent.click(screen.getByRole("button", { name: "選擇牌組：產品開發" })),
    );
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "建立臨時組" })));
    expect(checked()).toEqual([]);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "關閉選擇 repo" })));

    await act(async () => store.setScope({ group: null, repos: ["a"] }));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "建立臨時組" })));
    expect(checked()).toHaveLength(1);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "關閉選擇 repo" })));

    await act(async () =>
      fireEvent.click(screen.getByRole("button", { name: "編輯牌組：產品開發" })),
    );
    expect(checked()).toHaveLength(2);
  });
});

describe("關係建議（F-47）", () => {
  const suggestion = {
    from: "a",
    to: "b",
    kind: "pm-of",
    reason: "推測 a pm-of b；12 份文件互相提到",
    files: ["a:plan.md"],
  };

  it("列出理由與依據；套用呼叫 relate，不相關呼叫 relation_dismiss", async () => {
    const backend = mockBackend({
      getState: vi.fn(async () =>
        makeState({
          rel_kinds: { "pm-of": { forward: "規劃", backward: "被規劃" } as never },
          relation_suggestions: [suggestion],
        }),
      ),
    });
    await mount(backend);
    await act(async () => fireEvent.click(document.querySelector('[data-tab="relations"]')!));
    const box = document.getElementById("relsuggest")!;
    expect(box.textContent).toContain("12 份文件互相提到");
    expect(box.textContent).toContain("依據：a:plan.md");
    const apply = [...box.querySelectorAll("button")].find((b) => b.textContent === "套用")!;
    await act(async () => fireEvent.click(apply));
    expect(backend.action).toHaveBeenCalledWith("relate", {
      a: "a",
      b: "b",
      kind: "pm-of",
      note: "",
    });
    const dismiss = [...box.querySelectorAll("button")].find((b) => b.textContent === "不相關")!;
    await act(async () => fireEvent.click(dismiss));
    expect(backend.action).toHaveBeenCalledWith("relation_dismiss", { a: "a", b: "b" });
  });

  it("沒有建議時不顯示建議區塊", async () => {
    await mount();
    await act(async () => fireEvent.click(document.querySelector('[data-tab="relations"]')!));
    expect(document.getElementById("relsuggest")).toBeNull();
  });

  it("套用失敗把錯誤顯示在那一筆建議上", async () => {
    const backend = mockBackend({
      getState: vi.fn(async () => makeState({ relation_suggestions: [{ ...suggestion }] })),
      action: vi.fn(async () => {
        throw new Error("registry 寫入失敗");
      }),
    });
    await mount(backend);
    await act(async () => fireEvent.click(document.querySelector('[data-tab="relations"]')!));
    const box = document.getElementById("relsuggest")!;
    const apply = [...box.querySelectorAll("button")].find((b) => b.textContent === "套用")!;
    await act(async () => fireEvent.click(apply));
    expect(box.querySelector(".error")!.textContent).toBe("registry 寫入失敗");
  });
});

describe("相關新料卡（F-48）", () => {
  const freshCard: StateCard = {
    key: "fresh:r7",
    kind: "fresh",
    lane: "scope",
    id: "r7",
    scope_kind: "group",
    scope_label: "產品開發",
    scope_repos: ["a", "b"],
    date: "2026-10-10",
    time: "06:30",
    title: "新料：notes:索引重建.md",
    lines: ["共同詞：增量、重建、水位線"],
    actions: [
      { label: "拿來試", action: "fresh_try", payload: { id: "r7" }, primary: true },
      { label: "不相關", action: "fresh_dismiss", payload: { id: "r7" } },
    ],
  };

  it("自成一段，主要動作是拿來試", async () => {
    const backend = mockBackend({ getState: vi.fn(async () => makeState({ cards: [freshCard] })) });
    await mount(backend);
    const card = (await screen.findByText("新料：notes:索引重建.md")).closest("article")!;
    expect(card.textContent).toContain("共同詞：增量、重建、水位線");
    expect(document.body.textContent).toContain("相關新料");
    const btn = [...card.querySelectorAll("button")].find((b) => b.textContent === "拿來試")!;
    await act(async () => fireEvent.click(btn));
    expect(backend.action).toHaveBeenCalledWith("fresh_try", { id: "r7" });
  });
});

describe("任務包開會話（F-42）", () => {
  it("先向引擎要任務包，再開預填任務的會話對話框", async () => {
    const { loopCard } = await import("./fixtures");
    const card = {
      ...loopCard,
      actions: [
        ...loopCard.actions,
        { label: "用任務包開會話", action: "brief_session" as const, payload: { num: "#3" } },
      ],
    } as StateCard;
    const backend = mockBackend({
      getState: vi.fn(async () => makeState({ cards: [card] })),
      action: vi.fn(async (name: string) =>
        name === "todo_brief"
          ? { ok: true, id: "#3", markdown: "# 任務包：#3 驗證 VDI\n## 目標" }
          : { ok: true },
      ) as unknown as Backend["action"],
    });
    await mount(backend);
    const article = (await screen.findByText("驗證 VDI")).closest("article")!;
    await act(async () => fireEvent.click(article.querySelector(".more")!));
    const dialog = document.getElementById("detaildialog")!;
    const btn = [...dialog.querySelectorAll("button")].find(
      (b) => b.textContent === "用任務包開會話",
    )!;
    await act(async () => fireEvent.click(btn));
    expect(backend.action).toHaveBeenCalledWith("todo_brief", { num: "#3" });
    await waitFor(() => expect(dialog.textContent).toContain("開啟 Agent 會話"));
    const task = dialog.querySelector("textarea") as HTMLTextAreaElement;
    expect(task.value).toContain("# 任務包：#3 驗證 VDI");
  });
});

describe("判斷回顧卡（F-23）", () => {
  it("主按鈕「記結果…」開表單，寫一句就呼叫 decision_outcome（帶 ref／組／repo）", async () => {
    const card: StateCard = {
      key: "retro:decision:2026-01-01-d1",
      kind: "retro",
      lane: "scope",
      ref: "decision:2026-01-01-d1",
      scope_kind: "group",
      scope_label: "產品開發",
      scope_repos: ["a", "b"],
      date: "2026-01-01",
      time: "09:00",
      title: "判斷回顧：快取用 LRU",
      lines: ["20 天前的判斷，還沒記結果——後來怎樣？"],
      actions: [
        {
          label: "記結果…",
          action: "decision_outcome",
          payload: { ref: "decision:2026-01-01-d1", group: "產品開發", repo: null },
          primary: true,
        },
        {
          label: "開會話回顧",
          action: "term_create",
          payload: { kind: "agent", group: "產品開發", task: "回顧" },
        },
        {
          label: "延 30 天",
          action: "reopen_defer",
          payload: { ref: "decision:2026-01-01-d1", days: 30 },
        },
      ],
    };
    const backend = mockBackend({ getState: vi.fn(async () => makeState({ cards: [card] })) });
    await mount(backend);
    const article = (await screen.findByText("判斷回顧：快取用 LRU")).closest("article")!;
    await act(async () =>
      fireEvent.click(within(article as HTMLElement).getByRole("button", { name: "記結果…" })),
    );
    const dialog = document.getElementById("detaildialog")!;
    expect(dialog.textContent).toContain("這個判斷後來怎樣？");
    // 空的不送
    await act(async () =>
      fireEvent.click(within(dialog).getByRole("button", { name: "記下結果" })),
    );
    expect(backend.action).not.toHaveBeenCalledWith("decision_outcome", expect.anything());
    fireEvent.change(dialog.querySelector("textarea")!, {
      target: { value: "照做了，查詢 450ms → 120ms" },
    });
    await act(async () =>
      fireEvent.click(within(dialog).getByRole("button", { name: "記下結果" })),
    );
    expect(backend.action).toHaveBeenCalledWith("decision_outcome", {
      ref: "decision:2026-01-01-d1",
      group: "產品開發",
      repo: null,
      text: "照做了，查詢 450ms → 120ms",
    });
  });
});

describe("該重看的判斷卡（F-46）", () => {
  it("排在需要判斷之後自成一段；延 14 天呼叫 reopen_defer", async () => {
    const card: StateCard = {
      key: "reopen:decision:2026-01-01-d1",
      kind: "reopen",
      lane: "scope",
      ref: "decision:2026-01-01-d1",
      scope_kind: "group",
      scope_label: "產品開發",
      scope_repos: ["a", "b"],
      date: "2026-01-01",
      time: "09:00",
      title: "該重看：先不做 Windows",
      lines: ["當初說的重看條件：2026-10-01 或 VDI 使用者回報"],
      actions: [
        {
          label: "改判斷…",
          action: "decision_supersede",
          payload: { ref: "decision:2026-01-01-d1", group: "產品開發", repo: null },
          primary: true,
        },
        {
          label: "開會話重看",
          action: "term_create",
          payload: { kind: "agent", group: "產品開發", task: "重看判斷" },
        },
        {
          label: "延 14 天",
          action: "reopen_defer",
          payload: { ref: "decision:2026-01-01-d1", days: 14 },
        },
      ],
    };
    const backend = mockBackend({ getState: vi.fn(async () => makeState({ cards: [card] })) });
    await mount(backend);
    const article = (await screen.findByText("該重看：先不做 Windows")).closest("article")!;
    expect(article.textContent).toContain("當初說的重看條件");
    expect(document.body.textContent).toContain("該重看的判斷");
    await act(async () => fireEvent.click(article.querySelector(".more")!));
    let dialog = document.getElementById("detaildialog")!;
    const defer = [...dialog.querySelectorAll("button")].find((b) => b.textContent === "延 14 天")!;
    await act(async () => fireEvent.click(defer));
    expect(backend.action).toHaveBeenCalledWith("reopen_defer", {
      ref: "decision:2026-01-01-d1",
      days: 14,
    });
    // 主按鈕「改判斷…」：寫新判斷就 decision_supersede（帶 ref／組／repo）
    await act(async () =>
      fireEvent.click(within(article as HTMLElement).getByRole("button", { name: "改判斷…" })),
    );
    dialog = document.getElementById("detaildialog")!;
    expect(dialog.textContent).toContain("現在怎麼判斷？");
    fireEvent.change(dialog.querySelector("textarea")!, {
      target: { value: "VDI 使用者變多，Windows 要做" },
    });
    await act(async () =>
      fireEvent.click(within(dialog).getByRole("button", { name: "記下新判斷" })),
    );
    expect(backend.action).toHaveBeenCalledWith("decision_supersede", {
      ref: "decision:2026-01-01-d1",
      group: "產品開發",
      repo: null,
      text: "VDI 使用者變多，Windows 要做",
    });
  });
});

describe("健康指標（F-50）", () => {
  it("活動分頁最上方一行；沒有 health 鍵（舊引擎）就不顯示", async () => {
    const health = {
      days: 14,
      handoff: { handoffs: 28, missed: 9, rate: 0.76 },
      fresh: { found: 45, used: 0, dismissed: 0, hit_rate: 0 },
      decisions: { with_outcome: 0, awaiting_retro: 0, outcome_rate: null },
      loops: { opened: 46, closed: 6, open_now: 40, overdue: 0 },
    };
    await mount(mockBackend({ getState: vi.fn(async () => makeState({ health })) }));
    await act(async () => fireEvent.click(document.querySelector('[data-tab="activity"]')!));
    const el = document.getElementById("health")!;
    expect(el.textContent).toContain("交接率 76%（沒交接 9）");
    expect(el.textContent).toContain("推薦命中 0%（0/45）");
    expect(el.textContent).toContain("判斷有結果 —");
    cleanup();
    await mount();
    expect(document.getElementById("health")).toBeNull();
  });
});

describe("健康指標的名片覆蓋（F-57）", () => {
  it("列出還沒寫介紹的 repo，點了在右側欄打開；全寫了只顯示比例", async () => {
    const base = {
      days: 14,
      handoff: { handoffs: 1, missed: 0, rate: 1 },
      fresh: { found: 0, used: 0, dismissed: 0, hit_rate: null },
      decisions: { with_outcome: 0, awaiting_retro: 0, outcome_rate: null },
      loops: { opened: 0, closed: 0, open_now: 0, overdue: 0 },
    };
    const profiles = {
      repos: 14,
      described: 2,
      with_role: 1,
      undescribed: ["a", "b"],
      undescribed_more: 10,
    };
    await mount(
      mockBackend({
        getState: vi.fn(async () => makeState({ health: { ...base, profiles } })),
        action: vi.fn(async (name: string) =>
          name === "repo_tree"
            ? { ok: true, id: "a", root: "/r/a", dir: "", more: 0, entries: [] }
            : name === "profile_draft_brief"
              ? { ok: true, text: "起草名片", repos: ["a", "b"] }
              : { ok: true, id: "a", path: "/r/a" },
        ) as never,
      }),
    );
    await act(async () => fireEvent.click(document.querySelector('[data-tab="activity"]')!));
    const el = document.getElementById("health")!;
    expect(el.textContent).toContain("名片 2/14 寫了介紹；還沒寫：");
    expect(el.textContent).toContain("，另 10 個");
    await act(async () => fireEvent.click(within(el).getByRole("button", { name: "a" })));
    expect(document.getElementById("repopanel")!.getAttribute("aria-label")).toBe("Repo：a");
    await act(async () =>
      fireEvent.click(within(el).getByRole("button", { name: "請 Agent 起草…" })),
    );
    expect(document.getElementById("detailtitle")!.textContent).toBe("開啟 Agent 會話");
    expect((document.querySelector("#detaildialog textarea") as HTMLTextAreaElement).value).toBe(
      "起草名片",
    );
    cleanup();
    await mount(
      mockBackend({
        getState: vi.fn(async () =>
          makeState({
            health: {
              ...base,
              profiles: { ...profiles, described: 14, undescribed: [], undescribed_more: 0 },
            },
          }),
        ),
      }),
    );
    await act(async () => fireEvent.click(document.querySelector('[data-tab="activity"]')!));
    expect(document.getElementById("health")!.textContent).toContain("名片 14/14 寫了介紹");
    expect(document.getElementById("health")!.textContent).not.toContain("還沒寫");
  });
});

describe("收件匣分段懶顯示（N-25）", () => {
  const many = async (n: number) => {
    const { loopCard } = await import("./fixtures");
    return Array.from({ length: n }, (_, i) => ({
      ...loopCard,
      key: "loop:#" + (i + 1),
      num: "#" + (i + 1),
      title: "待辦 " + (i + 1),
    })) as StateCard[];
  };

  it("一段先畫 50 張，其餘按鈕展開", async () => {
    const cards = await many(120);
    await mount(mockBackend({ getState: vi.fn(async () => makeState({ cards })) }));
    await screen.findByText("待辦 1");
    const count = () => document.querySelectorAll("#cards article.card.loop").length;
    expect(count()).toBe(50);
    const more = () => document.querySelector("#cards .showmore") as HTMLButtonElement | null;
    expect(more()!.textContent).toBe("再顯示 50 則（還有 70 則）");
    await act(async () => fireEvent.click(more()!));
    expect(count()).toBe(100);
    expect(more()!.textContent).toBe("再顯示 20 則（還有 20 則）");
    await act(async () => fireEvent.click(more()!));
    expect(count()).toBe(120);
    expect(more()).toBeNull();
    // 段標題仍是全部的數量
    expect(document.querySelector("#cards .secttl")!.textContent).toContain("120");
  });

  it("5000 張卡也只畫 50 張（規模）", async () => {
    const cards = await many(5000);
    const t0 = performance.now();
    await mount(mockBackend({ getState: vi.fn(async () => makeState({ cards })) }));
    await screen.findByText("待辦 1");
    const ms = performance.now() - t0;
    expect(document.querySelectorAll("#cards article.card.loop").length).toBe(50);
    console.log(`5000 張卡首次渲染 ${ms.toFixed(0)}ms`);
  });
});

describe("檔案分頁（像 IDE 的多根工作區）", () => {
  const tree = (name: string, dir: string) => {
    if (dir === "")
      return {
        ok: true,
        id: name,
        root: "/r/" + name,
        dir,
        more: 0,
        entries: [
          { name: "docs", path: "docs", abs: "/r/" + name + "/docs", dir: true },
          { name: "README.md", path: "README.md", abs: "/r/" + name + "/README.md", dir: false },
        ],
      };
    return {
      ok: true,
      id: name,
      root: "/r/" + name,
      dir,
      more: 0,
      entries: [
        { name: "plan.md", path: "docs/plan.md", abs: "/r/" + name + "/docs/plan.md", dir: false },
      ],
    };
  };
  const treeBackend = (extra: Partial<Backend> = {}) =>
    mockBackend({
      action: vi.fn(async (name: string, p: { id?: string; dir?: string }) => {
        if (name === "repo_tree") return tree(p.id!, p.dir || "");
        if (name === "group_add_folder")
          return { ok: true, id: "new", registered: true, group: "產品開發" };
        return { ok: true };
      }) as never,
      ...extra,
    });

  async function openFiles(backend: ReturnType<typeof mockBackend>) {
    const store = new WorkbenchStore(backend, { group: "產品開發", repos: null });
    render(<App store={store} autoStart={false} />);
    await act(async () => {
      await store.refreshAll();
    });
    await act(async () => fireEvent.click(document.querySelector('[data-tab="files"]')!));
    return store;
  }

  it("被 .gitignore 忽略的照列、畫灰色（跟 VS Code 一樣）", async () => {
    const backend = treeBackend({
      action: vi.fn(async (name: string, p: { id?: string; dir?: string }) => {
        if (name !== "repo_tree") return { ok: true };
        const t = tree(p.id!, p.dir || "");
        if (!p.dir)
          t.entries = [
            { name: "runs", path: "runs", abs: "/r/a/runs", dir: true, ignored: true },
            ...t.entries,
          ] as typeof t.entries;
        return t;
      }) as never,
    });
    await openFiles(backend);
    await act(async () =>
      fireEvent.click(
        within(document.getElementById("tab-files")!).getByRole("button", { name: "a" }),
      ),
    );
    const runs = await screen.findByRole("treeitem", { name: "runs" });
    expect(runs.classList.contains("ignored")).toBe(true);
    expect(runs.getAttribute("title")).toContain("忽略");
    const readme = screen.getByRole("treeitem", { name: "README.md" });
    expect(readme.classList.contains("ignored")).toBe(false);
  });

  it("範圍內每個 repo 是一個根；展開才讀下一層，可複製絕對／相對路徑", async () => {
    const backend = treeBackend();
    await openFiles(backend);
    const pane = document.getElementById("tab-files")!;
    expect(pane.querySelectorAll(".reporoot").length).toBe(2);
    expect(backend.action).not.toHaveBeenCalledWith("repo_tree", expect.anything());

    await act(async () =>
      fireEvent.click(
        within(document.getElementById("tab-files")!).getByRole("button", { name: "a" }),
      ),
    );
    expect(backend.action).toHaveBeenCalledWith("repo_tree", { id: "a", dir: "" });
    await act(async () => fireEvent.click(await screen.findByRole("treeitem", { name: "docs" })));
    expect(backend.action).toHaveBeenCalledWith("repo_tree", { id: "a", dir: "docs" });

    const write = vi.fn(async () => {});
    Object.assign(navigator, { clipboard: { writeText: write } });
    const plan = await screen.findByRole("treeitem", { name: "plan.md" });
    expect(plan.getAttribute("aria-level")).toBe("2");
    fireEvent.contextMenu(plan);
    await act(async () => fireEvent.click(screen.getByRole("menuitem", { name: /複製相對路徑/ })));
    expect(write).toHaveBeenCalledWith("docs/plan.md");
    // 鍵盤：⌥⌘C 複製路徑
    await act(async () =>
      fireEvent.keyDown(plan, { code: "KeyC", key: "ç", altKey: true, metaKey: true }),
    );
    expect(write).toHaveBeenCalledWith("/r/a/docs/plan.md");
  });

  it("檔案樹照 git 狀態上色（跟 VS Code 一樣）：檔案標字母、資料夾含變更標點、沒 push 標 ↑", async () => {
    const base = treeBackend();
    const backend = treeBackend({
      action: vi.fn(async (name: string, p: { id?: string; dir?: string }) => {
        if (name === "repo_changes")
          return {
            ok: true,
            id: p.id,
            changes: [{ path: "docs/plan.md", status: "modified" }],
            more: 0,
            unpushed: ["README.md"],
          };
        return base.action(name as never, p as never);
      }) as never,
    });
    await openFiles(backend);
    await act(async () =>
      fireEvent.click(
        within(document.getElementById("tab-files")!).getByRole("button", { name: "a" }),
      ),
    );
    const docs = await screen.findByRole("treeitem", { name: "docs（內有變更）" });
    expect(docs.classList.contains("git-modified")).toBe(true);
    expect(docs.querySelector(".tgit")!.textContent).toBe("●");
    const readme = screen.getByRole("treeitem", { name: "README.md（還沒 push）" });
    expect(readme.classList.contains("git-unpushed")).toBe(true);
    expect(readme.querySelector(".tgit")!.textContent).toBe("↑");
    await act(async () => fireEvent.click(docs));
    const plan = await screen.findByRole("treeitem", { name: "plan.md（已修改）" });
    expect(plan.classList.contains("git-modified")).toBe(true);
    expect(plan.querySelector(".tgit")!.textContent).toBe("M");
  });

  it("殼層有 copyText（Tauri）就走殼層寫剪貼簿，不碰 WebView 的 navigator.clipboard；殼層失敗才退回", async () => {
    const copyText = vi.fn(async () => {});
    const backend = treeBackend({ copyText });
    await openFiles(backend);
    await act(async () =>
      fireEvent.click(
        within(document.getElementById("tab-files")!).getByRole("button", { name: "a" }),
      ),
    );
    const write = vi.fn(async () => {});
    Object.assign(navigator, { clipboard: { writeText: write } });
    const docs = await screen.findByRole("treeitem", { name: "docs" });
    fireEvent.contextMenu(docs);
    await act(async () => fireEvent.click(screen.getByRole("menuitem", { name: /^複製路徑/ })));
    expect(copyText).toHaveBeenCalledWith("/r/a/docs");
    expect(write).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("已複製路徑：/r/a/docs");

    copyText.mockRejectedValueOnce(new Error("boom"));
    fireEvent.contextMenu(docs);
    await act(async () => fireEvent.click(screen.getByRole("menuitem", { name: /複製相對路徑/ })));
    expect(write).toHaveBeenCalledWith("docs");
  });

  it("挑檔案交給 Agent：⌘＋點挑、右鍵移出、引擎組任務；失敗顯示原因；記在這台電腦", async () => {
    let fail = true;
    const backend = treeBackend({
      action: vi.fn(async (name: string, p: { id?: string; dir?: string; files?: unknown[] }) => {
        if (name === "repo_tree") return tree(p.id!, p.dir || "");
        if (name === "pick_brief") {
          if (fail) {
            fail = false;
            throw new Error("檔案不存在：README.md");
          }
          return {
            ok: true,
            text: "先讀這幾份檔案…",
            mentions: "@/r/a/README.md\n@/r/b/README.md",
            repos: ["a", "b"],
            count: 2,
          };
        }
        return { ok: true };
      }) as never,
    });
    await openFiles(backend);
    const pane = document.getElementById("tab-files")!;
    for (const root of ["a", "b"])
      await act(async () =>
        fireEvent.click(within(pane).getByRole("button", { name: new RegExp("^" + root + "$") })),
      );
    const readmes = await within(pane).findAllByRole("treeitem", { name: "README.md" });
    expect(document.getElementById("picktray")).toBeNull();
    await act(async () => fireEvent.click(readmes[0], { metaKey: true }));
    await act(async () => fireEvent.click(readmes[1], { ctrlKey: true }));
    // 挑不會開檔
    expect(backend.action).not.toHaveBeenCalledWith("read_file", expect.anything());
    expect(within(pane).getAllByRole("treeitem", { name: "README.md（已加入脈絡）" })).toHaveLength(
      2,
    );
    const tray = document.getElementById("picktray")!;
    expect(tray.textContent).toContain("脈絡 2 個檔案");

    await act(async () =>
      fireEvent.click(within(tray).getByRole("button", { name: "交給新會話…" })),
    );
    expect(within(tray).getByRole("alert").textContent).toContain("檔案不存在");
    await act(async () =>
      fireEvent.click(within(tray).getByRole("button", { name: "交給新會話…" })),
    );
    expect(backend.action).toHaveBeenLastCalledWith("pick_brief", {
      files: [
        { id: "a", path: "README.md" },
        { id: "b", path: "README.md" },
      ],
    });
    expect(document.getElementById("detailtitle")!.textContent).toBe("開啟 Agent 會話");
    expect((document.querySelector("#detaildialog textarea") as HTMLTextAreaElement).value).toBe(
      "先讀這幾份檔案…",
    );
    expect(JSON.parse(localStorage.getItem("workbench:picks")!)).toHaveLength(2);

    // 右鍵移出挑選；清單展開可個別移除
    fireEvent.contextMenu(
      within(pane).getAllByRole("treeitem", { name: "README.md（已加入脈絡）" })[0],
    );
    await act(async () => fireEvent.click(screen.getByRole("menuitem", { name: /移出脈絡/ })));
    expect(tray.textContent).toContain("脈絡 1 個檔案");
    await act(async () =>
      fireEvent.click(within(tray).getByRole("button", { name: /脈絡 \d 個檔案/ })),
    );
    await act(async () =>
      fireEvent.click(within(tray).getByRole("button", { name: "不挑 b 的 README.md" })),
    );
    expect(document.getElementById("picktray")).toBeNull();
    expect(localStorage.getItem("workbench:picks")).toBe("[]");
  });

  it("這組的變更：只列有變更的 repo，查詢帶目前範圍，點了開差異；讀過之後失敗保留清單；沒有變更整段不顯示", async () => {
    let rows: unknown[] | null = [
      { id: "b", more: 0, changes: [{ path: "wip0.md", status: "untracked" }] },
    ];
    const backend = treeBackend({
      action: vi.fn(async (name: string, p: { id?: string; dir?: string }) => {
        if (name === "repo_tree") return tree(p.id!, p.dir || "");
        if (name === "scope_changes") {
          if (rows === null) throw new Error("git 跑不起來");
          return { ok: true, repos: rows };
        }
        if (name === "file_diff")
          return { ok: true, id: "b", path: "wip0.md", diff: "+草稿", truncated: false };
        return { ok: true };
      }) as never,
    });
    const store = await openFiles(backend);
    const sec = await screen.findByRole("region", { name: "這組還沒 commit 的變更" });
    expect(backend.action).toHaveBeenCalledWith("scope_changes", { group: "產品開發" });
    expect(sec.textContent).toContain("這組的變更（1 個 repo、1 個檔）");
    expect(sec.textContent).toContain("b（1）");
    // 讀過之後失敗：保留清單、顯示原因
    const ok = rows;
    rows = null;
    await act(async () => {
      await store.refreshState();
    });
    await waitFor(() =>
      expect(within(sec).getByRole("alert").textContent).toContain("git 跑不起來"),
    );
    expect(sec.textContent).toContain("b（1）");
    rows = ok;
    await act(async () => fireEvent.click(within(sec).getByRole("button", { name: /wip0\.md/ })));
    expect(backend.action).toHaveBeenCalledWith("file_diff", { id: "b", path: "wip0.md" });
    rows = [];
    await act(async () => {
      await store.refreshState();
    });
    await waitFor(() =>
      expect(
        screen.queryByRole("region", { name: "這組還沒 commit 的變更", hidden: true }),
      ).toBeNull(),
    );
  });

  it("讀取失敗顯示錯誤", async () => {
    const backend = mockBackend({
      action: vi.fn(async () => {
        throw new Error("repo 路徑不存在：/r/a");
      }) as never,
    });
    await openFiles(backend);
    await act(async () =>
      fireEvent.click(
        within(document.getElementById("tab-files")!).getByRole("button", { name: "a" }),
      ),
    );
    expect((await screen.findByRole("alert", { name: "" })).textContent).toContain(
      "repo 路徑不存在",
    );
  });

  it("加入資料夾：原生選資料夾 → group_add_folder 加進目前牌組", async () => {
    const pickFolder = vi.fn(async () => "/Users/me/new");
    const backend = treeBackend({ pickFolder });
    await openFiles(backend);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "＋ 加入資料夾" })));
    expect(pickFolder).toHaveBeenCalled();
    expect(backend.action).toHaveBeenCalledWith("group_add_folder", {
      path: "/Users/me/new",
      group: "產品開發",
    });
  });

  it("取消選資料夾不做事；從這組移除只改成員", async () => {
    const backend = treeBackend({ pickFolder: vi.fn(async () => null) });
    await openFiles(backend);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "＋ 加入資料夾" })));
    expect(backend.action).not.toHaveBeenCalledWith("group_add_folder", expect.anything());
    const removes = screen.getAllByRole("button", { name: "從這組移除" });
    await act(async () => fireEvent.click(removes[0]));
    expect(backend.action).toHaveBeenCalledWith("update_group", { name: "產品開發", repos: ["b"] });
  });
});

describe("repo 右側欄與右鍵選單", () => {
  const repoA = {
    id: "a",
    type: "mine",
    tier: "active",
    path: "/r/a",
    exists: true,
    dirty: 3,
    dirty_days: 1,
    last_commit_days: 2.2,
    ahead: 1,
    behind: 0,
    branch: "main",
    last_subject: "修好登入",
    worktrees: 1,
    commits_7d: 4,
    commits_30d: 9,
    recent_commits: [],
    agents: [],
    note: "",
  } as const;
  const backendWithRepo = () =>
    mockBackend({
      getScan: vi.fn(async () =>
        makeScan({ states: [{ ...repoA, recent_commits: [], agents: [] }] }),
      ),
      action: vi.fn(async (name: string) =>
        name === "repo_tree"
          ? { ok: true, id: "a", root: "/r/a", dir: "", more: 0, entries: [] }
          : { ok: true },
      ) as never,
    });

  async function mountGroup(backend: ReturnType<typeof mockBackend>) {
    const store = new WorkbenchStore(backend, { group: "產品開發", repos: null });
    render(<App store={store} autoStart={false} />);
    await act(async () => {
      await store.refreshAll();
    });
    await act(async () => fireEvent.click(document.querySelector('[data-tab="repos"]')!));
    return store;
  }
  const repoButton = () =>
    within(document.getElementById("deeptable")!).getByRole("button", { name: "a" });

  it("點 repo 開右側欄（不跳視窗），只放工作時看的；再點收起", async () => {
    const backend = backendWithRepo();
    await mountGroup(backend);
    await act(async () => fireEvent.click(repoButton()));
    const panel = document.getElementById("repopanel")!;
    expect(panel).toBeTruthy();
    expect(document.getElementById("detaildialog")!.hasAttribute("open")).toBe(false);
    expect(document.body.classList.contains("withpanel")).toBe(true);
    expect(panel.textContent).toContain("main");
    // 未提交跟「變更」段同一個來源：現場讀到 0 個變更，就不再用掃描快照的 3 檔
    expect(panel.textContent).toContain("未提交乾淨");
    expect(panel.textContent).not.toContain("3 檔");
    expect(panel.textContent).toContain("變更（0）");
    expect(panel.textContent).toContain("修好登入（2 天前）");
    expect(backend.action).toHaveBeenCalledWith("repo_tree", { id: "a", dir: "" });
    await act(async () => fireEvent.click(repoButton()));
    expect(document.getElementById("repopanel")).toBeNull();
    expect(document.body.classList.contains("withpanel")).toBe(false);
  });

  it("右鍵選單：更多細節才跳視窗；從這組移除改成員；Esc 關閉", async () => {
    const backend = backendWithRepo();
    await mountGroup(backend);
    await act(async () => fireEvent.contextMenu(repoButton(), { clientX: 50, clientY: 60 }));
    const menu = screen.getByRole("menu");
    const labels = within(menu)
      .getAllByRole("menuitem")
      .map((b) => b.textContent);
    expect(labels).toEqual([
      "在側邊欄開啟",
      "更多細節…",
      "開啟 Agent 會話…",
      "複製絕對路徑",
      "從這組移除",
      "移出工作台…",
    ]);
    await act(async () => fireEvent.keyDown(window, { key: "Escape" }));
    expect(screen.queryByRole("menu")).toBeNull();

    await act(async () => fireEvent.contextMenu(repoButton(), { clientX: 50, clientY: 60 }));
    await act(async () => fireEvent.click(screen.getByRole("menuitem", { name: "更多細節…" })));
    expect(document.getElementById("detaildialog")!.hasAttribute("open")).toBe(true);
    expect(document.getElementById("detailtitle")!.textContent).toBe("a");

    await act(async () => fireEvent.contextMenu(repoButton(), { clientX: 50, clientY: 60 }));
    await act(async () => fireEvent.click(screen.getByRole("menuitem", { name: "從這組移除" })));
    expect(backend.action).toHaveBeenCalledWith("update_group", { name: "產品開發", repos: ["b"] });
  });

  it("名片：顯示 README 推測；寫名片送三欄，存好換成新的；失敗顯示原因且不關表單", async () => {
    let fail = true;
    const backend = mockBackend({
      getScan: vi.fn(async () =>
        makeScan({ states: [{ ...repoA, recent_commits: [], agents: [] }] }),
      ),
      action: vi.fn(async (name: string, p: Record<string, string>) => {
        if (name === "repo_tree")
          return { ok: true, id: "a", root: "/r/a", dir: "", more: 0, entries: [] };
        if (name === "repo_profile")
          return {
            ok: true,
            id: "a",
            path: "/r/a",
            description: "登入服務。",
            description_source: "readme",
          };
        if (name === "describe_repo") {
          if (fail) {
            fail = false;
            throw new Error("寫不進 registry");
          }
          return {
            ok: true,
            id: "a",
            path: "/r/a",
            description: p.description,
            description_source: "registry",
            role: p.role,
            keywords: ["登入", "OAuth"],
            changed: true,
          };
        }
        return { ok: true };
      }) as never,
    });
    await mountGroup(backend);
    await act(async () => fireEvent.click(repoButton()));
    // 側欄分頁（UX-07）：名片在「資訊」
    await act(async () => fireEvent.click(screen.getByRole("tab", { name: "資訊" })));
    const card = within(document.getElementById("repopanel")!).getByRole("region", {
      name: "a 的名片",
    });
    expect(backend.action).toHaveBeenCalledWith("repo_profile", { id: "a" });
    expect(card.textContent).toContain("登入服務。");
    expect(card.textContent).toContain("從 README 推測");

    await act(async () => fireEvent.click(within(card).getByRole("button", { name: "寫名片" })));
    const desc = within(card).getByLabelText("介紹") as HTMLTextAreaElement;
    expect(desc.value).toBe("");
    expect(desc.placeholder).toBe("推測：登入服務。");
    await act(async () =>
      fireEvent.click(within(card).getByRole("button", { name: "用推測的介紹改" })),
    );
    expect(desc.value).toBe("登入服務。");
    fireEvent.change(within(card).getByLabelText(/角色/), { target: { value: "實作" } });
    fireEvent.change(within(card).getByLabelText(/關鍵字/), { target: { value: "登入、OAuth" } });
    await act(async () => fireEvent.click(within(card).getByRole("button", { name: "儲存名片" })));
    expect(within(card).getByRole("alert").textContent).toContain("寫不進 registry");
    expect(within(card).getByLabelText("介紹")).toBeTruthy();

    await act(async () => fireEvent.click(within(card).getByRole("button", { name: "儲存名片" })));
    expect(backend.action).toHaveBeenLastCalledWith("describe_repo", {
      id: "a",
      description: "登入服務。",
      role: "實作",
      keywords: "登入、OAuth",
    });
    expect(within(card).queryByLabelText("介紹")).toBeNull();
    expect(card.textContent).not.toContain("從 README 推測");
    expect(card.textContent).toContain("實作");
    expect(within(card).getByRole("list", { name: "關鍵字" }).textContent).toBe("登入OAuth");
  });

  it("名片：類別選單選了就存（鎖住、失敗顯示原因並回原值），存好顯示給 agent 的規則", async () => {
    let fail = true;
    const classes = [
      { id: "code", label: "程式", reliable: "測試通過" },
      { id: "reference", label: "參考", reliable: "不改，只讀" },
    ];
    const base = { ok: true, id: "a", path: "/r/a", classes };
    const backend = mockBackend({
      getScan: vi.fn(async () =>
        makeScan({ states: [{ ...repoA, recent_commits: [], agents: [] }] }),
      ),
      action: vi.fn(async (name: string, p: Record<string, string>) => {
        if (name === "repo_tree")
          return { ok: true, id: "a", root: "/r/a", dir: "", more: 0, entries: [] };
        if (name === "repo_profile") return base;
        if (name === "repo_class") {
          if (fail) {
            fail = false;
            throw new Error("類別需為 code|agent");
          }
          return {
            ...base,
            changed: true,
            class: { id: p.class, label: "參考", reliable: "不改，只讀", rule: "別人的專案，只讀" },
          };
        }
        return { ok: true };
      }) as never,
    });
    await mountGroup(backend);
    await act(async () => fireEvent.click(repoButton()));
    await act(async () => fireEvent.click(screen.getByRole("tab", { name: "資訊" })));
    const card = within(document.getElementById("repopanel")!).getByRole("region", {
      name: "a 的名片",
    });
    const select = (await within(card).findByLabelText("類別")) as HTMLSelectElement;
    expect(select.value).toBe("");
    await act(async () => fireEvent.change(select, { target: { value: "reference" } }));
    expect(within(card).getByRole("alert").textContent).toContain("類別需為");
    expect(select.value).toBe("");
    await act(async () => fireEvent.change(select, { target: { value: "reference" } }));
    expect(backend.action).toHaveBeenLastCalledWith("repo_class", { id: "a", class: "reference" });
    expect(select.value).toBe("reference");
    expect(card.textContent).toContain("給 agent 的規則：別人的專案，只讀");
    expect(within(card).queryByRole("alert")).toBeNull();
  });

  it("變更：列出還沒 commit 的檔，點了在主區開差異；差異模式不顯示讀檔錯誤", async () => {
    const diff = "diff --git a/src/a.ts b/src/a.ts\n@@ -1 +1 @@\n-old\n+new";
    const backend = mockBackend({
      getScan: vi.fn(async () =>
        makeScan({ states: [{ ...repoA, recent_commits: [], agents: [] }] }),
      ),
      action: vi.fn(async (name: string) => {
        if (name === "repo_tree")
          return { ok: true, id: "a", root: "/r/a", dir: "", more: 0, entries: [] };
        if (name === "repo_changes") {
          return {
            ok: true,
            id: "a",
            more: 0,
            changes: [
              { path: "src/a.ts", status: "modified" },
              { path: "new.md", status: "untracked" },
            ],
          };
        }
        if (name === "file_diff")
          return { ok: true, id: "a", path: "src/a.ts", diff, truncated: false };
        if (name === "review_brief") return { ok: true, id: "a", text: "審查 a 的變更", count: 2 };
        if (name === "read_file") throw new Error("檔案不存在：src/a.ts");
        return { ok: true, id: "a", path: "/r/a" };
      }) as never,
    });
    await mountGroup(backend);
    await act(async () => fireEvent.click(repoButton()));
    await act(async () => fireEvent.click(screen.getByRole("tab", { name: /變更/ })));
    expect(screen.getByRole("tab", { name: /變更/ }).getAttribute("aria-selected")).toBe("true");
    const sec = within(document.getElementById("repopanel")!).getByRole("region", {
      name: "a 還沒 commit 的變更",
    });
    await waitFor(() => expect(sec.textContent).toContain("變更（2）"));
    expect(within(sec).getByLabelText("修改").textContent).toBe("M");
    await act(async () =>
      fireEvent.click(within(sec).getByRole("button", { name: "請 Agent 審查…" })),
    );
    expect(backend.action).toHaveBeenCalledWith("review_brief", { id: "a" });
    expect(document.getElementById("detailtitle")!.textContent).toBe("開啟 Agent 會話");
    expect((document.querySelector("#detaildialog textarea") as HTMLTextAreaElement).value).toBe(
      "審查 a 的變更",
    );
    await act(async () => {
      (document.getElementById("detaildialog") as HTMLDialogElement).close();
    });
    await act(async () => fireEvent.click(within(sec).getByRole("button", { name: /src\/a\.ts/ })));
    expect(backend.action).toHaveBeenCalledWith("file_diff", { id: "a", path: "src/a.ts" });
    const view = await screen.findByRole("region", { name: "檢視：a/src/a.ts" });
    expect(within(view).getByRole("button", { name: "差異" }).getAttribute("aria-pressed")).toBe(
      "true",
    );
    const lines = [...view.querySelectorAll(".diffview .dl")].map((l) => l.className);
    expect(lines).toEqual(["dl meta", "dl hunk", "dl del", "dl add"]);
    // 差異模式不顯示讀檔錯誤（刪掉的檔讀不到，但差異看得到）
    expect(within(view).queryByText(/檔案不存在/)).toBeNull();
    // 切回內容：才看得到讀檔錯誤
    await act(async () => fireEvent.click(within(view).getByRole("button", { name: "內容" })));
    expect(within(view).getByRole("alert").textContent).toContain("檔案不存在");
  });

  it("移出工作台…：直接開確認，按確定才呼叫 remove_repo", async () => {
    const backend = backendWithRepo();
    await mountGroup(backend);
    await act(async () => fireEvent.contextMenu(repoButton(), { clientX: 50, clientY: 60 }));
    await act(async () => fireEvent.click(screen.getByRole("menuitem", { name: "移出工作台…" })));
    expect(document.getElementById("detailtitle")!.textContent).toBe("把 a 移出工作台？");
    expect(backend.action).not.toHaveBeenCalledWith("remove_repo", expect.anything());
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "確定移出" })));
    expect(backend.action).toHaveBeenCalledWith("remove_repo", { id: "a" });
  });
});

describe("唯讀檢視器（主區檔案分頁）", () => {
  const files: Record<string, string> = {
    "README.md": "# 說明\n\n看 [指南](docs/guide.md)，外部 [網站](https://example.com)。",
    "docs/guide.md": "## 指南\n內容",
    "src/main.rs": 'fn main() {\n    println!("hi");\n}\n',
  };
  const viewerBackend = () =>
    mockBackend({
      action: vi.fn(async (name: string, p: { id?: string; dir?: string; path?: string }) => {
        if (name === "repo_tree")
          return {
            ok: true,
            id: p.id,
            root: "/r/" + p.id,
            dir: "",
            more: 0,
            entries: [
              { name: "README.md", path: "README.md", abs: "/r/a/README.md", dir: false },
              { name: "main.rs", path: "src/main.rs", abs: "/r/a/src/main.rs", dir: false },
            ],
          };
        if (name === "palette") return { ok: true, query: "", repos: [], groups: [], files: [] };
        if (name === "read_file") {
          const text = files[p.path!];
          if (text == null) throw new Error("檔案不存在：" + p.path);
          return {
            ok: true,
            id: p.id,
            path: p.path,
            abs: "/r/a/" + p.path,
            size: text.length,
            kind: "text",
            text,
            truncated: false,
          };
        }
        return { ok: true };
      }) as never,
    });

  async function openTree(backend: ReturnType<typeof mockBackend>) {
    const store = new WorkbenchStore(backend, { group: "產品開發", repos: null });
    render(<App store={store} autoStart={false} />);
    await act(async () => {
      await store.refreshAll();
    });
    await act(async () => fireEvent.click(document.querySelector('[data-tab="files"]')!));
    await act(async () =>
      fireEvent.click(
        within(document.getElementById("tab-files")!).getByRole("button", { name: "a" }),
      ),
    );
  }

  it("檔案樹新增檔案：輸入檔名 Enter → create_entry → 直接開成分割編輯；清單按 Enter 自動延續", async () => {
    const disk: Record<string, string> = { "README.md": "# 說明" };
    const backend = mockBackend({
      action: vi.fn(async (name: string, p: { id?: string; dir?: string; path?: string }) => {
        if (name === "repo_tree")
          return {
            ok: true,
            id: p.id,
            root: "/r/a",
            dir: "",
            more: 0,
            entries: Object.keys(disk).map((k) => ({
              name: k,
              path: k,
              abs: "/r/a/" + k,
              dir: false,
            })),
          };
        if (name === "create_entry") {
          disk[p.path!] = "";
          return { ok: true, id: "a", path: p.path, abs: "/r/a/" + p.path, dir: false };
        }
        if (name === "save_image")
          return { ok: true, id: "a", path: p.path, abs: "/r/a/" + p.path };
        if (name === "read_file")
          return {
            ok: true,
            id: p.id,
            path: p.path,
            abs: "/r/a/" + p.path,
            size: disk[p.path!].length,
            kind: "text",
            text: disk[p.path!],
            truncated: false,
            editable: true,
            version: "v1",
          };
        return { ok: true };
      }) as never,
    });
    await openTree(backend);
    await screen.findByRole("treeitem", { name: "README.md" });
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "新增檔案到 a" })));
    const input = screen.getByRole("textbox", { name: "新檔名" });
    fireEvent.change(input, { target: { value: "notes.md" } });
    await act(async () => fireEvent.keyDown(input, { key: "Enter" }));
    expect(backend.action).toHaveBeenCalledWith("create_entry", {
      id: "a",
      path: "notes.md",
      kind: "file",
    });
    const box = (await screen.findByRole("textbox", {
      name: "編輯 notes.md",
    })) as HTMLTextAreaElement;
    // md 預設分割：右邊即時預覽
    expect(screen.getByRole("button", { name: "分割" }).getAttribute("aria-pressed")).toBe("true");
    await act(async () => fireEvent.change(box, { target: { value: "# 標題\n- 第一項" } }));
    expect(document.querySelector(".editorpreview h1")!.textContent).toBe("標題");
    box.setSelectionRange(box.value.length, box.value.length);
    await act(async () => fireEvent.keyDown(box, { key: "Enter" }));
    expect(box.value).toBe("# 標題\n- 第一項\n- ");
    // 工具列：粗體包住選取
    box.setSelectionRange(2, 4);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "粗體（⌘B）" })));
    expect(box.value.startsWith("# **標題**")).toBe(true);
    // HackMD 快捷鍵：⌘⌥E 只編輯、⌘⌥V 只預覽、⌘⌥B 分割（⌥ 會換字元，靠 code）
    await act(async () =>
      fireEvent.keyDown(window, { key: "´", code: "KeyE", metaKey: true, altKey: true }),
    );
    expect(screen.getByRole("button", { name: "編輯" }).getAttribute("aria-pressed")).toBe("true");
    await act(async () =>
      fireEvent.keyDown(window, { key: "√", code: "KeyV", metaKey: true, altKey: true }),
    );
    expect(screen.getByRole("button", { name: "預覽" }).getAttribute("aria-pressed")).toBe("true");
    await act(async () =>
      fireEvent.keyDown(window, { key: "∫", code: "KeyB", metaKey: true, altKey: true }),
    );
    expect(screen.getByRole("button", { name: "分割" }).getAttribute("aria-pressed")).toBe("true");
    // Windows 的 Ctrl+Alt（＝AltGr）在編輯區裡打字不能被搶走
    await act(async () =>
      fireEvent.keyDown(box, { key: "€", code: "KeyE", ctrlKey: true, altKey: true }),
    );
    expect(screen.getByRole("button", { name: "分割" }).getAttribute("aria-pressed")).toBe("true");
    // 編輯中的大綱：照草稿的標題，點了游標跳到那一行
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "大綱" })));
    await act(async () => fireEvent.click(screen.getByRole("menuitem", { name: "標題" })));
    expect(box.selectionStart).toBe(0);
    expect(document.querySelector(".editorstats")!.textContent).toMatch(/字 · \d+ 行/);
    // 編輯中 ⌘F：找草稿、全部取代（F-70）
    await act(async () => fireEvent.keyDown(window, { key: "f", metaKey: true }));
    const q = screen.getByRole("textbox", { name: "在草稿中尋找" });
    await act(async () => fireEvent.change(q, { target: { value: "標題" } }));
    expect(document.querySelector(".editfind .findcount")!.textContent).toBe("1 / 1 · 第 1 行");
    fireEvent.change(screen.getByRole("textbox", { name: "取代成" }), {
      target: { value: "大標" },
    });
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "全部取代" })));
    expect(box.value.startsWith("# **大標**")).toBe(true);
    await act(async () => fireEvent.keyDown(q, { key: "Escape" }));
    expect(screen.queryByRole("textbox", { name: "在草稿中尋找" })).toBeNull();
    // 貼上截圖：存到旁邊的 assets/、插入圖片連結
    const png = new File([new Uint8Array([137, 80, 78, 71])], "x.png", { type: "image/png" });
    box.setSelectionRange(box.value.length, box.value.length);
    await act(async () =>
      fireEvent.paste(box, {
        clipboardData: { items: [{ kind: "file", type: "image/png", getAsFile: () => png }] },
      }),
    );
    await waitFor(() =>
      expect(backend.action).toHaveBeenCalledWith("save_image", {
        id: "a",
        path: expect.stringMatching(/^assets\/notes-\d{8}-\d{9}\.png$/),
        base64: "iVBORw==",
      }),
    );
    await waitFor(() => expect(box.value).toMatch(/!\[notes-\d{8}-\d{9}\.png\]\(assets\/notes-/));
  });

  it("F2 改名：rename_entry、開著的分頁跟著換路徑", async () => {
    const disk: Record<string, string> = { "README.md": "# 說明" };
    const backend = mockBackend({
      action: vi.fn(
        async (name: string, p: { id?: string; path?: string; from?: string; to?: string }) => {
          if (name === "repo_tree")
            return {
              ok: true,
              id: p.id,
              root: "/r/a",
              dir: "",
              more: 0,
              entries: Object.keys(disk).map((k) => ({
                name: k,
                path: k,
                abs: "/r/a/" + k,
                dir: false,
              })),
            };
          if (name === "rename_entry") {
            disk[p.to!] = disk[p.from!];
            delete disk[p.from!];
            return { ok: true, id: "a", from: p.from, path: p.to, abs: "/r/a/" + p.to, dir: false };
          }
          if (name === "read_file") {
            const text = disk[p.path!];
            if (text == null) throw new Error("檔案不存在：" + p.path);
            return {
              ok: true,
              id: p.id,
              path: p.path,
              abs: "/r/a/" + p.path,
              size: text.length,
              kind: "text",
              text,
              truncated: false,
            };
          }
          return { ok: true };
        },
      ) as never,
    });
    await openTree(backend);
    const item = await screen.findByRole("treeitem", { name: "README.md" });
    await act(async () => fireEvent.click(item));
    expect(screen.getByRole("region", { name: "檢視：a/README.md" })).toBeTruthy();
    // 回工作區（檔案樹在那裡）再改名
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "工作區" })));
    await act(async () =>
      fireEvent.keyDown(screen.getByRole("treeitem", { name: "README.md" }), { key: "F2" }),
    );
    const input = screen.getByRole("textbox", { name: "新名稱：README.md" });
    fireEvent.change(input, { target: { value: "GUIDE.md" } });
    await act(async () => fireEvent.keyDown(input, { key: "Enter" }));
    expect(backend.action).toHaveBeenCalledWith("rename_entry", {
      id: "a",
      from: "README.md",
      to: "GUIDE.md",
    });
    await screen.findByRole("treeitem", { name: "GUIDE.md" });
    // Esc 取消後就算再失焦也不送出
    await act(async () =>
      fireEvent.keyDown(screen.getByRole("treeitem", { name: "GUIDE.md" }), { key: "F2" }),
    );
    const again = screen.getByRole("textbox", { name: "新名稱：GUIDE.md" });
    fireEvent.change(again, { target: { value: "NOPE.md" } });
    await act(async () => {
      fireEvent.keyDown(again, { key: "Escape" });
      fireEvent.blur(again);
    });
    expect(backend.action).not.toHaveBeenCalledWith(
      "rename_entry",
      expect.objectContaining({ to: "NOPE.md" }),
    );
    // 開著的分頁換成新路徑（分頁名也換）
    expect(screen.getByRole("button", { name: "關閉 GUIDE.md" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "關閉 README.md" })).toBeNull();
  });

  it("新增檔案失敗（同名）：錯誤留在輸入列，不開分頁", async () => {
    const backend = mockBackend({
      action: vi.fn(async (name: string, p: { id?: string }) => {
        if (name === "repo_tree")
          return { ok: true, id: p.id, root: "/r/a", dir: "", more: 0, entries: [] };
        if (name === "create_entry") throw new Error("已經有同名的檔案或資料夾：x.md");
        return { ok: true };
      }) as never,
    });
    await openTree(backend);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "新增資料夾到 a" })));
    const input = screen.getByRole("textbox", { name: "新資料夾名稱" });
    fireEvent.change(input, { target: { value: "x.md" } });
    await act(async () => fireEvent.keyDown(input, { key: "Enter" }));
    expect(screen.getByRole("alert").textContent).toContain("已經有同名");
    expect(document.querySelector(".viewer")).toBeNull();
  });

  it("⌘F 在檔案中尋找：數量、Enter 下一筆、Esc 關", async () => {
    const backend = viewerBackend();
    await openTree(backend);
    await act(async () =>
      fireEvent.click(await screen.findByRole("treeitem", { name: "README.md" })),
    );
    await act(async () => fireEvent.keyDown(window, { key: "f", metaKey: true }));
    const box = screen.getByRole("textbox", { name: "在檔案中尋找" });
    await act(async () => fireEvent.change(box, { target: { value: "指南" } }));
    expect(document.querySelector(".findcount")!.textContent).toBe("1 / 1");
    await act(async () => fireEvent.change(box, { target: { value: "沒有這個字" } }));
    expect(document.querySelector(".findcount")!.textContent).toBe("找不到");
    await act(async () => fireEvent.keyDown(box, { key: "Escape" }));
    expect(screen.queryByRole("textbox", { name: "在檔案中尋找" })).toBeNull();
  });

  it("預覽裡點待辦方塊：改原文存回去，帶 version（HackMD 式）", async () => {
    const disk = { text: "- [ ] 買牛奶\n- [x] 寫信\n", v: 1 };
    const file = () => ({
      ok: true,
      id: "a",
      path: "todo.md",
      abs: "/r/a/todo.md",
      size: disk.text.length,
      kind: "text",
      text: disk.text,
      truncated: false,
      editable: true,
      version: "v" + disk.v,
    });
    const backend = mockBackend({
      action: vi.fn(async (name: string, p: { text?: string; base?: string; id?: string }) => {
        if (name === "repo_tree")
          return {
            ok: true,
            id: p.id,
            root: "/r/a",
            dir: "",
            more: 0,
            entries: [{ name: "todo.md", path: "todo.md", abs: "/r/a/todo.md", dir: false }],
          };
        if (name === "read_file") return file();
        if (name === "write_file") {
          disk.text = p.text!;
          disk.v += 1;
          return file();
        }
        return { ok: true };
      }) as never,
    });
    await openTree(backend);
    await act(async () =>
      fireEvent.click(await screen.findByRole("treeitem", { name: "todo.md" })),
    );
    const boxes = screen.getAllByRole("checkbox", { name: "勾選或取消這個待辦" });
    expect(boxes.map((b) => (b as HTMLInputElement).checked)).toEqual([false, true]);
    await act(async () => fireEvent.click(boxes[0]));
    expect(backend.action).toHaveBeenCalledWith("write_file", {
      id: "a",
      path: "todo.md",
      text: "- [x] 買牛奶\n- [x] 寫信\n",
      base: "v1",
    });
    await waitFor(() =>
      expect(
        (screen.getAllByRole("checkbox", { name: "勾選或取消這個待辦" })[0] as HTMLInputElement)
          .checked,
      ).toBe(true),
    );
  });

  it("偏好「打開 md 時直接分割編輯」：開了之後點 md 直接進編輯", async () => {
    localStorage.setItem("workbench:mdOpenEdit", "true");
    const disk: Record<string, string> = { "README.md": "# 說明" };
    const backend = mockBackend({
      action: vi.fn(async (name: string, p: { id?: string; path?: string }) => {
        if (name === "repo_tree")
          return {
            ok: true,
            id: p.id,
            root: "/r/a",
            dir: "",
            more: 0,
            entries: [{ name: "README.md", path: "README.md", abs: "/r/a/README.md", dir: false }],
          };
        if (name === "read_file")
          return {
            ok: true,
            id: p.id,
            path: p.path,
            abs: "/r/a/" + p.path,
            size: 4,
            kind: "text",
            text: disk[p.path!],
            truncated: false,
            editable: true,
            version: "v1",
          };
        if (name === "palette") return { ok: true, query: "", repos: [], groups: [], files: [] };
        return { ok: true };
      }) as never,
    });
    await openTree(backend);
    await act(async () =>
      fireEvent.click(await screen.findByRole("treeitem", { name: "README.md" })),
    );
    expect(await screen.findByRole("textbox", { name: "編輯 README.md" })).toBeTruthy();
  });

  it("md 預覽下面列出連到這份的文件，點了開那份（F-69）", async () => {
    const base = viewerBackend();
    const inner = base.action;
    const backend = mockBackend({
      action: vi.fn(async (name: string, p: never) => {
        if (name === "backlinks")
          return {
            ok: true,
            id: "a",
            path: "README.md",
            items: [{ path: "docs/guide.md", line: 3, text: "回到 [說明](../README.md)" }],
            more: 0,
          };
        return inner(name as never, p);
      }) as never,
    });
    await openTree(backend);
    await act(async () =>
      fireEvent.click(await screen.findByRole("treeitem", { name: "README.md" })),
    );
    const sec = await screen.findByRole("region", { name: "連到這份的文件" });
    expect(sec.textContent).toContain("連到這份的文件（1）");
    await act(async () =>
      fireEvent.click(within(sec).getByRole("button", { name: /docs\/guide\.md/ })),
    );
    expect(backend.action).toHaveBeenCalledWith("read_file", { id: "a", path: "docs/guide.md" });
  });

  it("程式碼檔編輯時 Tab 縮排（照檔案習慣），不讓焦點跳走", async () => {
    const backend = mockBackend({
      action: vi.fn(async (name: string, p: { id?: string; path?: string }) => {
        if (name === "repo_tree")
          return {
            ok: true,
            id: p.id,
            root: "/r/a",
            dir: "",
            more: 0,
            entries: [{ name: "x.rs", path: "x.rs", abs: "/r/a/x.rs", dir: false }],
          };
        if (name === "read_file")
          return {
            ok: true,
            id: p.id,
            path: p.path,
            abs: "/r/a/x.rs",
            size: 10,
            kind: "text",
            text: "fn a() {\n}\n",
            truncated: false,
            editable: true,
            version: "v1",
          };
        if (name === "palette") return { ok: true, query: "", repos: [], groups: [], files: [] };
        return { ok: true };
      }) as never,
    });
    await openTree(backend);
    await act(async () => fireEvent.click(await screen.findByRole("treeitem", { name: "x.rs" })));
    // ⌘L 前往第幾行：捲過去並閃一下
    await act(async () => fireEvent.keyDown(window, { key: "l", metaKey: true }));
    const go = screen.getByRole("textbox", { name: "前往第幾行" });
    fireEvent.change(go, { target: { value: "2" } });
    await act(async () => fireEvent.keyDown(go, { key: "Enter" }));
    expect(document.querySelectorAll(".codeview .cl")[1].classList.contains("cl-flash")).toBe(true);
    expect(screen.queryByRole("textbox", { name: "前往第幾行" })).toBeNull();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "編輯" })));
    const box = screen.getByRole("textbox", { name: "編輯 x.rs" }) as HTMLTextAreaElement;
    box.setSelectionRange(9, 9);
    await act(async () => fireEvent.keyDown(box, { key: "Tab" }));
    expect(box.value).toBe("fn a() {\n    }\n");
    // Enter 保留縮排
    box.setSelectionRange(13, 13);
    await act(async () => fireEvent.keyDown(box, { key: "Enter" }));
    expect(box.value).toBe("fn a() {\n    \n    }\n");
  });

  it("md 大綱：列出標題，點了捲過去", async () => {
    const backend = viewerBackend();
    await openTree(backend);
    await act(async () =>
      fireEvent.click(await screen.findByRole("treeitem", { name: "README.md" })),
    );
    const h1 = document.querySelector(".viewer:not([hidden]) h1") as HTMLElement;
    const scroll = vi.fn();
    h1.scrollIntoView = scroll;
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "大綱" })));
    await act(async () => fireEvent.click(screen.getByRole("menuitem", { name: "說明" })));
    expect(scroll).toHaveBeenCalled();
  });

  it("看檔案時用 ⌘K 切分頁：回到工作區", async () => {
    const backend = viewerBackend();
    await openTree(backend);
    await act(async () =>
      fireEvent.click(await screen.findByRole("treeitem", { name: "README.md" })),
    );
    expect(document.getElementById("workspace")!.hidden).toBe(true);
    await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    const box = screen.getByRole("combobox", { name: "搜尋或執行指令" });
    fireEvent.change(box, { target: { value: ">活動" } });
    await act(async () => fireEvent.keyDown(box, { key: "Enter" }));
    expect(document.getElementById("workspace")!.hidden).toBe(false);
    expect(document.getElementById("tab-activity")!.classList.contains("on")).toBe(true);
  });

  it("檢視器「加入脈絡」：挑起來、按鈕變「已加入脈絡 ✓」，再按取消", async () => {
    const backend = viewerBackend();
    await openTree(backend);
    await act(async () =>
      fireEvent.click(await screen.findByRole("treeitem", { name: "README.md" })),
    );
    const viewer = () => document.querySelector(".viewer:not([hidden])") as HTMLElement;
    await act(async () =>
      fireEvent.click(within(viewer()).getByRole("button", { name: "加入脈絡" })),
    );
    expect(
      within(viewer()).getByRole("button", { name: "已加入脈絡 ✓" }).getAttribute("aria-pressed"),
    ).toBe("true");
    expect(document.getElementById("picktray")!.textContent).toContain("脈絡 1 個檔案");
    expect(JSON.parse(localStorage.getItem("workbench:picks")!)).toEqual([
      { repo: "a", path: "README.md", name: "README.md" },
    ]);
    await act(async () =>
      fireEvent.click(within(viewer()).getByRole("button", { name: "已加入脈絡 ✓" })),
    );
    expect(document.getElementById("picktray")).toBeNull();
  });

  it("點檔案開分頁：md 預設預覽、可切原始碼；相對連結開新分頁；關掉回到工作區", async () => {
    const backend = viewerBackend();
    await openTree(backend);
    await act(async () =>
      fireEvent.click(await screen.findByRole("treeitem", { name: "README.md" })),
    );
    expect(backend.action).toHaveBeenCalledWith("read_file", { id: "a", path: "README.md" });
    expect(document.getElementById("workspace")!.hidden).toBe(true);
    const viewer = () => document.querySelector(".viewer:not([hidden])")!;
    expect(viewer().querySelector("h1")!.textContent).toBe("說明");

    await act(async () =>
      fireEvent.click(within(viewer() as HTMLElement).getByRole("button", { name: "原始碼" })),
    );
    expect(viewer().querySelector(".codeview")!.textContent).toContain("# 說明");
    await act(async () =>
      fireEvent.click(within(viewer() as HTMLElement).getByRole("button", { name: "預覽" })),
    );

    const write = vi.fn(async () => {});
    Object.assign(navigator, { clipboard: { writeText: write } });
    await act(async () => fireEvent.click(within(viewer() as HTMLElement).getByText("網站")));
    expect(write).toHaveBeenCalledWith("https://example.com");

    await act(async () => fireEvent.click(within(viewer() as HTMLElement).getByText("指南")));
    expect(backend.action).toHaveBeenCalledWith("read_file", { id: "a", path: "docs/guide.md" });
    expect(document.querySelectorAll(".filetab:not(.home)").length).toBe(2);
    expect(viewer().querySelector("h2")!.textContent).toBe("指南");

    await act(async () => fireEvent.click(screen.getByRole("button", { name: "關閉 guide.md" })));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "關閉 README.md" })));
    expect(document.querySelector(".filetabs")).toBeNull();
    expect(document.getElementById("workspace")!.hidden).toBe(false);
  });

  it("程式碼有行號；讀取失敗顯示錯誤", async () => {
    const backend = viewerBackend();
    await openTree(backend);
    await act(async () =>
      fireEvent.click(await screen.findByRole("treeitem", { name: "main.rs" })),
    );
    const lines = document.querySelectorAll(".viewer:not([hidden]) .codeview .cl");
    expect(lines.length).toBe(3);
    expect(lines[1].querySelector(".ln")!.textContent).toBe("2");
  });
});

describe("側欄排序與寬度", () => {
  it("牌組標題 Alt+↓ 換順序並存檔；組員 Alt+↑ 也是", async () => {
    const backend = mockBackend({
      getState: vi.fn(async () =>
        makeState({
          groups: [
            { name: "產品開發", members: ["a", "b"], goal: "", goal_at: "" },
            { name: "研究", members: ["b"], goal: "", goal_at: "" },
          ],
        }),
      ),
    });
    await mount(backend);
    const head = document.querySelector('.gcard[data-key="產品開發"] .ghead')!;
    await act(async () => fireEvent.keyDown(head, { key: "ArrowDown", altKey: true }));
    expect(backend.action).toHaveBeenCalledWith("reorder", {
      kind: "groups",
      order: ["研究", "產品開發"],
    });

    await act(async () => fireEvent.click(screen.getByRole("button", { name: "展開所有 repo" })));
    const member = document.querySelector('.gcard[data-key="*"] .member:nth-child(2)')!;
    await act(async () => fireEvent.keyDown(member, { key: "ArrowUp", altKey: true }));
    expect(backend.action).toHaveBeenCalledWith("reorder", { kind: "repos", order: ["b", "a"] });
  });

  it("側欄把手：方向鍵調寬度並記住，雙擊回預設", async () => {
    await mount();
    const handle = screen.getByRole("separator", { name: "調整左側欄寬度" });
    document.documentElement.style.setProperty("--sidew", "264px");
    await act(async () => fireEvent.keyDown(handle, { key: "ArrowRight" }));
    expect(document.documentElement.style.getPropertyValue("--sidew")).toBe("280px");
    expect(localStorage.getItem("workbench:sideWidth")).toBe("280");
    await act(async () => fireEvent.doubleClick(handle));
    expect(document.documentElement.style.getPropertyValue("--sidew")).toBe("");
  });
});

describe("檔案樹跟著磁碟變", () => {
  it("展開的資料夾登記給殼監看；有變動就重讀那一層，開著的檔案也重讀", async () => {
    const subs = new Set<(id: string, paths: string[], rescan: boolean) => void>();
    const changed = (id: string, paths: string[], rescan = false) =>
      subs.forEach((cb) => cb(id, paths, rescan));
    const watchTree = vi.fn(async () => {});
    let round = 0;
    const backend = mockBackend({
      watchTree,
      onRepoChanged: (cb) => {
        subs.add(cb);
        return () => subs.delete(cb);
      },
      action: vi.fn(async (name: string, p: { id?: string; dir?: string; path?: string }) => {
        if (name === "repo_tree")
          return {
            ok: true,
            id: p.id,
            root: "/r/a",
            dir: p.dir,
            more: 0,
            entries: [
              { name: "README.md", path: "README.md", abs: "/r/a/README.md", dir: false },
              ...(round
                ? [{ name: "new.txt", path: "new.txt", abs: "/r/a/new.txt", dir: false }]
                : []),
            ],
          };
        if (name === "read_file")
          return {
            ok: true,
            id: "a",
            path: p.path,
            abs: "/r/a/" + p.path,
            size: 3,
            kind: "text",
            text: "v" + round,
            truncated: false,
          };
        return { ok: true };
      }) as never,
    });
    const store = new WorkbenchStore(backend, { group: null, repos: ["a"] });
    render(<App store={store} autoStart={false} />);
    await act(async () => {
      await store.refreshAll();
    });
    await act(async () => fireEvent.click(document.querySelector('[data-tab="files"]')!));
    // 只有一個 repo：根目錄預設展開
    await screen.findByRole("treeitem", { name: "README.md" });
    await waitFor(() => expect(watchTree).toHaveBeenLastCalledWith({ a: [""] }));

    await act(async () => fireEvent.click(screen.getByRole("treeitem", { name: "README.md" })));
    await screen.findByText("v0");
    round = 1;
    await act(async () => changed("a", ["new.txt", "README.md"]));
    // 主區正在看檔案（工作區藏起來），樹照樣更新
    expect(await screen.findByRole("treeitem", { name: "new.txt", hidden: true })).toBeTruthy();
    expect(await screen.findByText("v1")).toBeTruthy();
    // 子資料夾沒展開的變動不重讀
    const calls = (backend.action as ReturnType<typeof vi.fn>).mock.calls.length;
    await act(async () => changed("a", ["src/x.rs"]));
    expect((backend.action as ReturnType<typeof vi.fn>).mock.calls.length).toBe(calls);
    // OS 漏了事件（rescan）：樹跟開著的檔都重讀
    round = 2;
    await act(async () => changed("a", [""], true));
    expect(await screen.findByText("v2")).toBeTruthy();
  });

  it("同一層連續重讀：先送出、晚回來的舊清單不蓋掉新的", async () => {
    const subs = new Set<(id: string, paths: string[], rescan: boolean) => void>();
    const resolvers: ((v: unknown) => void)[] = [];
    const listing = (names: string[]) => ({
      ok: true,
      id: "a",
      root: "/r/a",
      dir: "",
      more: 0,
      entries: names.map((n) => ({ name: n, path: n, abs: "/r/a/" + n, dir: false })),
    });
    let first = true;
    const backend = mockBackend({
      watchTree: vi.fn(async () => {}),
      onRepoChanged: (cb) => {
        subs.add(cb);
        return () => subs.delete(cb);
      },
      action: vi.fn(async (name: string) => {
        if (name !== "repo_tree") return { ok: true };
        if (first) {
          first = false;
          return listing(["README.md"]);
        }
        return new Promise((res) => resolvers.push(res));
      }) as never,
    });
    const store = new WorkbenchStore(backend, { group: null, repos: ["a"] });
    render(<App store={store} autoStart={false} />);
    await act(async () => {
      await store.refreshAll();
    });
    await act(async () => fireEvent.click(document.querySelector('[data-tab="files"]')!));
    await screen.findByRole("treeitem", { name: "README.md" });
    await act(async () => subs.forEach((cb) => cb("a", ["old.txt"], false)));
    await act(async () => subs.forEach((cb) => cb("a", ["new.txt"], false)));
    expect(resolvers.length).toBe(2);
    await act(async () => resolvers[1](listing(["README.md", "new.txt"])));
    await act(async () => resolvers[0](listing(["README.md", "old.txt"])));
    expect(screen.getByRole("treeitem", { name: "new.txt" })).toBeTruthy();
    expect(screen.queryByRole("treeitem", { name: "old.txt" })).toBeNull();
  });
});

describe("檔案簡單編輯", () => {
  function editBackend(opts: { gate?: Promise<void> } = {}) {
    const subs = new Set<(id: string, paths: string[], rescan: boolean) => void>();
    const disk = { text: "原本\n", v: 1 };
    const backend = mockBackend({
      watchTree: vi.fn(async () => {}),
      onRepoChanged: (cb) => {
        subs.add(cb);
        return () => subs.delete(cb);
      },
      action: vi.fn(
        async (name: string, p: { id?: string; path?: string; text?: string; base?: string }) => {
          const file = () => ({
            ok: true,
            id: "a",
            path: "a.md",
            abs: "/r/a/a.md",
            size: disk.text.length,
            kind: "text",
            text: disk.text,
            truncated: false,
            editable: true,
            version: "v" + disk.v,
          });
          if (name === "repo_tree")
            return {
              ok: true,
              id: "a",
              root: "/r/a",
              dir: "",
              more: 0,
              entries: [{ name: "a.md", path: "a.md", abs: "/r/a/a.md", dir: false }],
            };
          if (name === "read_file") return file();
          if (name === "write_file") {
            if (p.base !== "v" + disk.v)
              return { ok: false, conflict: true, id: "a", path: "a.md" };
            disk.text = p.text!;
            disk.v += 1;
            // 真的殼：磁碟事件可能比存檔的回應先到（自己的回音）——測試自己決定何時放行
            if (opts.gate) await opts.gate;
            return file();
          }
          return { ok: true };
        },
      ) as never,
    });
    const agentWrites = (text: string) => {
      disk.text = text;
      disk.v += 1;
      subs.forEach((cb) => cb("a", ["a.md"], false));
    };
    const echo = () => subs.forEach((cb) => cb("a", ["a.md"], false));
    return { backend, disk, agentWrites, echo };
  }

  async function openEditor(backend: ReturnType<typeof mockBackend>) {
    const store = new WorkbenchStore(backend, { group: null, repos: ["a"] });
    render(<App store={store} autoStart={false} />);
    await act(async () => {
      await store.refreshAll();
    });
    await act(async () => fireEvent.click(document.querySelector('[data-tab="files"]')!));
    await act(async () => fireEvent.click(await screen.findByRole("treeitem", { name: "a.md" })));
    await act(async () => fireEvent.click(await screen.findByRole("button", { name: "編輯" })));
    return screen.getByRole("textbox", { name: "編輯 a.md" }) as HTMLTextAreaElement;
  }

  it("編輯區下緣顯示存檔狀態：未儲存 → 已儲存 時間", async () => {
    const { backend } = editBackend();
    const box = await openEditor(backend);
    await act(async () => fireEvent.change(box, { target: { value: "改了\n" } }));
    expect(document.querySelector(".editorstats")!.textContent).toContain("未儲存");
    await act(async () => fireEvent.keyDown(box, { key: "s", metaKey: true }));
    expect(document.querySelector(".editorstats")!.textContent).toMatch(/已儲存 \d\d:\d\d:\d\d/);
  });

  it("存檔的回音比回應先到、還在繼續打字：不誤報「磁碟上的檔被改了」", async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const { backend, echo } = editBackend({ gate });
    const box = await openEditor(backend);
    await act(async () => fireEvent.change(box, { target: { value: "第一段\n" } }));
    // 存檔送出（回應先卡住）
    await act(async () => fireEvent.keyDown(box, { key: "s", metaKey: true }));
    // 存檔途中又打了字，然後磁碟事件（自己的回音）先到
    await act(async () => fireEvent.change(box, { target: { value: "第一段\n第二段\n" } }));
    await act(async () => echo());
    // 回應還沒到的這段時間也不能閃出衝突橫幅
    expect(screen.queryByText(/磁碟上的檔在你編輯時被改了/)).toBeNull();
    await act(async () => release());
    expect(screen.queryByText(/磁碟上的檔在你編輯時被改了/)).toBeNull();
    expect(box.value).toBe("第一段\n第二段\n");
  });

  it("編輯後 ⌘S 存檔，帶讀檔時的 version；沒改東西時 agent 改檔會直接換上新內容", async () => {
    const { backend, disk, agentWrites } = editBackend();
    const box = await openEditor(backend);
    expect(box.value).toBe("原本\n");
    await act(async () => agentWrites("agent 改的\n"));
    await waitFor(() => expect(box.value).toBe("agent 改的\n"));

    await act(async () => fireEvent.change(box, { target: { value: "我改的\n" } }));
    expect(document.querySelector(".filetab .dirtymark")).toBeTruthy();
    await act(async () => fireEvent.keyDown(box, { key: "s", metaKey: true }));
    expect(backend.action).toHaveBeenCalledWith("write_file", {
      id: "a",
      path: "a.md",
      text: "我改的\n",
      base: "v2",
    });
    expect(disk.text).toBe("我改的\n");
    expect(document.querySelector(".filetab .dirtymark")).toBeNull();
  });

  it("有未存修改時 agent 改了檔：不洗掉輸入，提示後可以覆蓋；分頁關閉要按兩次", async () => {
    const { backend, disk, agentWrites } = editBackend();
    const box = await openEditor(backend);
    await act(async () => fireEvent.change(box, { target: { value: "我的版本\n" } }));
    await act(async () => agentWrites("agent 的版本\n"));
    await screen.findByText(/磁碟上的檔在你編輯時被改了/);
    expect(box.value).toBe("我的版本\n");

    // 直接存＝衝突，磁碟不動
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "儲存" })));
    expect(disk.text).toBe("agent 的版本\n");

    await act(async () => fireEvent.click(screen.getByRole("button", { name: "關閉 a.md" })));
    expect(document.querySelector(".viewer:not([hidden])")).toBeTruthy();

    await act(async () => fireEvent.click(screen.getByRole("button", { name: "用我的版本覆蓋" })));
    expect(disk.text).toBe("我的版本\n");
    expect(screen.queryByText(/磁碟上的檔在你編輯時被改了/)).toBeNull();
  });
});

describe("會話放進所屬牌組", () => {
  const term = (over: Record<string, unknown>) => ({
    sid: "t1",
    title: "claude · a",
    alive: true,
    kind: "agent",
    origin: null,
    scope: "a",
    scope_repos: ["a"],
    started: "2026-10-12 09:00",
    task: null,
    report: null,
    report_basis: "",
    agent_status: null,
    ...over,
  });

  it("範圍裡已有會話：主要按鈕是「回到會話」（等你的優先），新建降為次要（UX-01）", async () => {
    const backend = mockBackend({
      getState: vi.fn(async () =>
        makeState({
          terms: [
            term({ sid: "t1", home_group: "產品開發", started: "2026-10-12 10:00" }),
            term({
              sid: "t3",
              title: "等我的",
              home_group: "產品開發",
              started: "2026-10-12 08:00",
              agent_status: { status: "waiting", text: "等你", at: "x", event: "stop" },
            }),
            term({ sid: "t4", title: "結束了", alive: false, home_group: "產品開發" }),
          ] as never,
        }),
      ),
    });
    await mount(backend);
    const resume = screen.getByRole("button", { name: /回到會話（等你回覆） · 2/ });
    expect(resume.className).toContain("primary");
    expect(screen.getByRole("button", { name: "新建 Agent 會話…" }).className).not.toContain(
      "primary",
    );
    // 兩個以上：列出來選，等你的在前
    await act(async () => fireEvent.click(resume));
    const items = screen.getAllByRole("menuitem");
    expect(items[0].textContent).toContain("等我的（等你）");
    await act(async () => fireEvent.click(items[0]));
    expect(document.getElementById("termtitle")!.textContent).toContain("等我的");
  });

  it("Claude Code 停在權限提示：標「要你允許／回答」、列出提示、按「去處理」切過去，不給打字回答框", async () => {
    const backend = mockBackend({
      getState: vi.fn(async () =>
        makeState({
          terms: [
            term({
              sid: "t5",
              title: "要允許的",
              home_group: "產品開發",
              agent_status: {
                status: "blocked",
                text: "要你允許 Bash：rm -rf dist",
                at: "2026-10-14 01:05",
                event: "permission",
              },
            }),
            term({
              sid: "t6",
              title: "真卡住的",
              home_group: "產品開發",
              agent_status: { status: "blocked", text: "找不到測試資料", at: "2026-10-14 01:06" },
            }),
          ] as never,
        }),
      ),
    });
    await mount(backend);
    const row = screen.getByRole("button", { name: /要允許的/ }).closest(".session")!;
    expect(row.textContent).toContain("要你允許／回答");
    expect(row.textContent).toContain("要你允許 Bash：rm -rf dist");
    expect(within(row as HTMLElement).queryByRole("button", { name: "回答…" })).toBeNull();
    await act(async () =>
      fireEvent.click(within(row as HTMLElement).getByRole("button", { name: "去處理" })),
    );
    expect(document.getElementById("termtitle")!.textContent).toContain("要允許的");
    // agent 自己回報的 blocked 還是「卡住了」，可以打字回它
    const stuck = screen.getByRole("button", { name: /真卡住的/ }).closest(".session")!;
    expect(stuck.textContent).toContain("卡住了");
    expect(within(stuck as HTMLElement).getByRole("button", { name: "回答…" })).toBeTruthy();
  });

  it("做完一輪、還沒看的會話列有「看變更」：開那個 repo 的右側欄變更頁，不切會話", async () => {
    const backend = mockBackend({
      getState: vi.fn(async () =>
        makeState({
          terms: [
            term({
              sid: "t7",
              title: "做完的",
              home_group: "產品開發",
              agent_status: {
                status: "finished",
                text: "這一輪做完了",
                at: "2026-10-14 02:00",
                event: "stop",
              },
            }),
          ] as never,
        }),
      ),
    });
    await mount(backend);
    const row = screen.getByRole("button", { name: /做完的/ }).closest(".session")!;
    await act(async () =>
      fireEvent.click(within(row as HTMLElement).getByRole("button", { name: "看變更" })),
    );
    expect(document.getElementById("repopanel")!.textContent).toContain("a");
    expect(document.getElementById("panelpage-changes")!.hasAttribute("hidden")).toBe(false);
    expect(document.getElementById("termtitle")!.textContent).not.toContain("做完的");
  });

  it("記錄本次工作結果：存好後說記在哪、可以直接到活動看（UX 流程 C）", async () => {
    const backend = mockBackend({
      getState: vi.fn(async () =>
        makeState({ terms: [term({ sid: "t1", home_group: "產品開發" })] as never }),
      ),
      action: vi.fn(async (name: string) => {
        if (name === "session_draft") return { ok: true, text: "草稿", note: "", traces: 1 };
        if (name === "session_result") return { ok: true, date: "2026-10-15", time: "04:20" };
        return { ok: true };
      }) as never,
    });
    await mount(backend);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "回到會話" })));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "更多會話動作" })));
    await act(async () =>
      fireEvent.click(screen.getByRole("menuitem", { name: "記錄本次工作結果…" })),
    );
    const dialog = document.getElementById("detaildialog")!;
    await act(async () =>
      fireEvent.click(within(dialog).getByRole("button", { name: "確認並儲存" })),
    );
    expect(backend.action).toHaveBeenCalledWith("session_result", { sid: "t1", text: "草稿" });
    expect(within(dialog).getByRole("status").textContent).toContain(
      "已記在活動：2026-10-15 04:20",
    );
    await act(async () =>
      fireEvent.click(within(dialog).getByRole("button", { name: "到活動分頁看" })),
    );
    expect(document.getElementById("tab-activity")!.classList.contains("on")).toBe(true);
  });

  it("逐字稿（F-94）：從會話選單打開，篩「我說的」，錯誤結果標紅；讀失敗把原因寫出來", async () => {
    const frames = [
      { kind: "you", text: "幫我跑測試", at: "2026-10-21T01:00:00Z" },
      { kind: "thinking", text: "先看看", at: null },
      { kind: "reply", text: "好", at: null },
      { kind: "tool", text: "cargo test", at: null, tool: "Bash", result: "失敗", error: true },
    ];
    let fail = false;
    const backend = mockBackend({
      getState: vi.fn(async () =>
        makeState({ terms: [term({ sid: "t1", home_group: "產品開發" })] as never }),
      ),
      action: vi.fn(async (name: string) => {
        if (name === "session_transcript") {
          if (fail) throw new Error("這個會話還沒有逐字稿");
          return { ok: true, id: "x", frames, truncated: false, title: "跑測試", path: "/p.jsonl" };
        }
        return { ok: true };
      }) as never,
    });
    await mount(backend);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "回到會話" })));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "更多會話動作" })));
    await act(async () =>
      fireEvent.click(screen.getByRole("menuitem", { name: "逐字稿（完整對話與工具）" })),
    );
    expect(backend.action).toHaveBeenCalledWith("session_transcript", { sid: "t1" });
    const dialog = document.getElementById("detaildialog")!;
    const list = within(dialog).getByRole("list", { name: "逐字稿" });
    expect(list.querySelectorAll("li")).toHaveLength(4);
    expect(list.querySelector(".txerr")!.textContent).toContain("cargo test");
    await act(async () =>
      fireEvent.click(within(dialog).getByRole("button", { name: "我說的 1" })),
    );
    expect(list.querySelectorAll("li")).toHaveLength(1);
    expect(within(dialog).getByRole("button", { name: "貼回會話" })).toBeTruthy();
    fail = true;
    await act(async () =>
      fireEvent.click(within(dialog).getByRole("button", { name: "重新讀取" })),
    );
    expect(dialog.textContent).toContain("讀不到逐字稿：這個會話還沒有逐字稿");
  });

  it("牌組的會話在牌組區塊裡（寫 repo），其他的在下面；終端標題寫「組 › repo」", async () => {
    const backend = mockBackend({
      getState: vi.fn(async () =>
        makeState({
          terms: [
            term({
              home_group: "產品開發",
              agent_status: {
                status: "waiting",
                text: "這一輪做完了，等你回覆",
                at: "2026-10-12 09:30",
                event: "stop",
              },
            }),
            term({ sid: "t2", title: "shell", kind: "shell", scope: null, scope_repos: null }),
            term({ sid: "t3", title: "整組", scope: "產品開發", scope_repos: null }),
          ] as never,
        }),
      ),
    });
    await mount(backend);
    const card = document.querySelector('.gcard[data-key="產品開發"]')!;
    const inCard = card.querySelector(".gsessions")!;
    expect(inCard.textContent).toContain("claude · a");
    // 牌組裡的會話列：在等你的寫出來（Claude Code hook 自動寫的狀態）
    expect(inCard.textContent).toContain("等你回應 · a");
    expect(inCard.textContent).toContain("整組");
    expect(card.querySelector(".gmeta")!.textContent).toContain("2 個會話 · 1 個等你");
    const others = document.getElementById("sessions")!;
    expect(others.textContent).toContain("shell");
    expect(others.textContent).not.toContain("claude · a");

    await act(async () =>
      fireEvent.click(within(inCard as HTMLElement).getByRole("button", { name: /claude · a/ })),
    );
    expect(document.getElementById("termwhere")!.textContent).toBe("產品開發 › a");
    // 對單一 repo 開的會話：「看變更」在右側欄打開那個 repo
    await act(async () =>
      fireEvent.click(
        within(document.getElementById("termbar")!).getByRole("button", { name: "查看變更" }),
      ),
    );
    expect(document.getElementById("repopanel")!.getAttribute("aria-label")).toBe("Repo：a");
    // 直接切到「變更」頁籤
    expect(screen.getByRole("tab", { name: /變更/ }).getAttribute("aria-selected")).toBe("true");
  });

  it("agent 會話的終端標題能切到同目錄的 Terminal 再切回來；Terminal 不另列在會話列", async () => {
    let terms = [term({ home_group: "產品開發" })];
    const action = vi.fn(async (name: string) => {
      if (name === "term_create")
        terms = [
          ...terms,
          term({
            sid: "t9",
            title: "shell",
            kind: "shell",
            origin: "shell-of:t1",
            scope: null,
            scope_repos: null,
          }),
        ];
      return { ok: true, sid: "t9", title: "shell" };
    });
    const backend = mockBackend({
      getState: vi.fn(async () => makeState({ terms: terms as never })),
      action: action as never,
    });
    await mount(backend);
    const card = document.querySelector('.gcard[data-key="產品開發"]')!;
    await act(async () =>
      fireEvent.click(within(card as HTMLElement).getByRole("button", { name: /claude · a/ })),
    );
    const bar = document.getElementById("termbar")!;
    expect(within(bar).getByRole("button", { name: "Agent" }).getAttribute("aria-pressed")).toBe(
      "true",
    );

    await act(async () => fireEvent.click(within(bar).getByRole("button", { name: "開啟終端" })));
    expect(action).toHaveBeenCalledWith("term_create", { kind: "shell", of: "t1" });
    expect(within(bar).getByRole("button", { name: "Terminal" }).getAttribute("aria-pressed")).toBe(
      "true",
    );
    // 位置標籤跟著 agent；Terminal 不出現在會話列
    expect(document.getElementById("termwhere")!.textContent).toBe("產品開發 › a");
    expect(document.getElementById("sessions")!.textContent).not.toContain("shell");

    await act(async () => fireEvent.click(within(bar).getByRole("button", { name: "Agent" })));
    expect(within(bar).getByRole("button", { name: "Agent" }).getAttribute("aria-pressed")).toBe(
      "true",
    );
    // 再切過去：沿用已開的 Terminal，不再開新的
    await act(async () => fireEvent.click(within(bar).getByRole("button", { name: "Terminal" })));
    expect(action).toHaveBeenCalledTimes(1);
  });

  it("挑好的檔案貼到目前會話：@路徑以空白分隔、不按 Enter；沒有選會話時不出現按鈕", async () => {
    const writes: string[] = [];
    const stream = {
      onData() {},
      onOpen(cb: () => void) {
        cb();
      },
      onClose() {},
      write: (d: string) => writes.push(d),
      resize() {},
      close() {},
      connected: true,
    };
    const backend = mockBackend({
      getState: vi.fn(async () =>
        makeState({ terms: [term({ home_group: "產品開發" })] as never }),
      ),
      openTerminal: vi.fn(() => stream) as never,
      action: vi.fn(async (name: string) => {
        if (name === "repo_tree")
          return {
            ok: true,
            id: "a",
            root: "/r/a",
            dir: "",
            more: 0,
            entries: [{ name: "README.md", path: "README.md", abs: "/r/a/README.md", dir: false }],
          };
        if (name === "pick_brief")
          return {
            ok: true,
            text: "…",
            mentions: "@/r/a/README.md\n@/r/a/docs/x.md",
            repos: ["a"],
            count: 2,
          };
        return { ok: true };
      }) as never,
    });
    await mount(backend);
    localStorage.setItem(
      "workbench:picks",
      JSON.stringify([{ repo: "a", path: "README.md", name: "README.md" }]),
    );
    cleanup();
    await mount(backend);
    const tray = document.getElementById("picktray")!;
    expect(tray.textContent).toContain("脈絡 1 個檔案");
    expect(within(tray).queryByRole("button", { name: "貼到目前會話（不送出）" })).toBeNull();

    const card = document.querySelector('.gcard[data-key="產品開發"]')!;
    await act(async () =>
      fireEvent.click(within(card as HTMLElement).getByRole("button", { name: /claude · a/ })),
    );
    await waitFor(() => expect(backend.openTerminal).toHaveBeenCalledWith("t1"));
    await act(async () =>
      fireEvent.click(within(tray).getByRole("button", { name: "貼到目前會話（不送出）" })),
    );
    await waitFor(() => expect(writes.join("")).toBe("@/r/a/README.md @/r/a/docs/x.md "));
    expect(within(tray).queryByRole("alert")).toBeNull();
  });

  it("從檔案樹拖檔到終端：agent 會話貼 @路徑；沒選會話時提示", async () => {
    const writes: string[] = [];
    const stream = {
      onData() {},
      onOpen(cb: () => void) {
        cb();
      },
      onClose() {},
      write: (d: string) => writes.push(d),
      resize() {},
      close() {},
      connected: true,
    };
    const store = (
      await mount(
        mockBackend({
          getState: vi.fn(async () =>
            makeState({ terms: [term({ home_group: "產品開發" })] as never }),
          ),
          openTerminal: vi.fn(() => stream) as never,
        }),
      )
    ).store;
    const data = (abs: string) => {
      const m = new Map([[FILE_DRAG_TYPE, JSON.stringify({ repo: "a", path: "x.md", abs })]]);
      return { types: [...m.keys()], getData: (t: string) => m.get(t) ?? "", dropEffect: "" };
    };
    const panel = document.getElementById("termpanel")!;
    const toast = vi.spyOn(store, "toast");
    await act(async () => fireEvent.drop(panel, { dataTransfer: data("/r/a/x.md") }));
    expect(toast).toHaveBeenCalledWith(expect.stringContaining("先在下方終端選一個"));

    const card = document.querySelector('.gcard[data-key="產品開發"]')!;
    await act(async () =>
      fireEvent.click(within(card as HTMLElement).getByRole("button", { name: /claude · a/ })),
    );
    await waitFor(() => expect(store.backend.openTerminal).toHaveBeenCalledWith("t1"));
    fireEvent.dragOver(panel, { dataTransfer: data("/r/a/x.md") });
    expect(panel.classList.contains("dropping")).toBe(true);
    await act(async () => fireEvent.drop(panel, { dataTransfer: data("/r/a/x.md") }));
    expect(panel.classList.contains("dropping")).toBe(false);
    await waitFor(() => expect(writes.join("")).toBe("@/r/a/x.md "));
  });

  it("選單列「等你的會話」：切到卡住的那個會話", async () => {
    let next: (() => void) | null = null;
    const backend = Object.assign(
      mockBackend({
        getState: vi.fn(async () =>
          makeState({
            terms: [
              term({ sid: "t1", title: "claude · a" }),
              term({
                sid: "t2",
                title: "claude · b",
                scope: "b",
                scope_repos: ["b"],
                agent_status: { status: "blocked", text: "要你允許", at: "2026-10-12 09:30" },
              }),
            ] as never,
          }),
        ),
      }),
      {
        onNextWaiting: (cb: () => void) => {
          next = cb;
          return () => {
            next = null;
          };
        },
      },
    );
    await mount(backend);
    await act(async () => next!());
    expect(document.getElementById("termwhere")!.textContent).toBe("b");
  });

  it("從牌組裡對 repo 開會話：帶上所屬牌組", async () => {
    const store = new WorkbenchStore(mockBackend(), { group: "產品開發", repos: null });
    render(<App store={store} autoStart={false} />);
    await act(async () => {
      await store.refreshAll();
    });
    const card = document.querySelector('.gcard[data-key="產品開發"]')!;
    await act(async () =>
      fireEvent.click(within(card as HTMLElement).getByRole("button", { name: /展開/ })),
    );
    fireEvent.contextMenu(within(card as HTMLElement).getByRole("button", { name: /^a/ }));
    await act(async () =>
      fireEvent.click(screen.getByRole("menuitem", { name: "開啟 Agent 會話…" })),
    );
    const dialog = document.getElementById("detaildialog")!;
    expect(dialog.textContent).toContain("產品開發 › a");
  });

  it("開會話選權限模式、模型、隔離 worktree（F-96／F-108）：只帶有選的，下次記得", async () => {
    const backend = mockBackend({
      action: vi.fn(async (name: string) =>
        name === "term_create" ? { ok: true, sid: "t9", title: "x" } : { ok: true },
      ) as never,
    });
    const store = new WorkbenchStore(backend, { group: "產品開發", repos: null });
    render(<App store={store} autoStart={false} />);
    await act(async () => {
      await store.refreshAll();
    });
    const card = document.querySelector('.gcard[data-key="產品開發"]')!;
    await act(async () =>
      fireEvent.click(within(card as HTMLElement).getByRole("button", { name: /展開/ })),
    );
    fireEvent.contextMenu(within(card as HTMLElement).getByRole("button", { name: /^a/ }));
    await act(async () =>
      fireEvent.click(screen.getByRole("menuitem", { name: "開啟 Agent 會話…" })),
    );
    const dialog = document.getElementById("detaildialog")!;
    fireEvent.change(within(dialog).getByLabelText("權限"), { target: { value: "plan" } });
    fireEvent.change(within(dialog).getByLabelText("模型"), { target: { value: " opus " } });
    fireEvent.click(within(dialog).getByLabelText(/隔離的 git worktree/));
    await act(async () =>
      fireEvent.click(within(dialog).getByRole("button", { name: "開始會話" })),
    );
    expect(backend.action).toHaveBeenCalledWith(
      "term_create",
      expect.objectContaining({ permission_mode: "plan", model: "opus", worktree: true }),
    );
    expect(
      JSON.parse(
        localStorage.getItem(
          Object.keys(localStorage).find((k) => k.endsWith("permission_mode"))!,
        )!,
      ),
    ).toBe("plan");
  });
});

describe("牌組筆記", () => {
  const notesBackend = () => {
    let notes = [{ at: "2026-10-11 09:00", text: "問 PM **規格**" }];
    return mockBackend({
      action: vi.fn(async (name: string, p: { text?: string; at?: string }) => {
        if (name === "note_add") notes = [{ at: "2026-10-12 10:00", text: p.text! }, ...notes];
        if (name === "note_remove") notes = notes.filter((n) => n.at !== p.at);
        if (name.endsWith("notes") || name.startsWith("note_"))
          return { ok: true, group: "產品開發", path: "/sp/groups/產品開發/notes.md", notes };
        return { ok: true };
      }) as never,
    });
  };

  it("選單列「記一筆…」：切到筆記分頁、游標在輸入框；已在筆記分頁再按也回到輸入框", async () => {
    let capture: (() => void) | null = null;
    const backend = Object.assign(notesBackend(), {
      onCapture: (cb: () => void) => {
        capture = cb;
        return () => {
          capture = null;
        };
      },
    });
    const store = new WorkbenchStore(backend, { group: "產品開發", repos: null });
    render(<App store={store} autoStart={false} />);
    await act(async () => {
      await store.refreshAll();
    });
    expect(capture).not.toBeNull();
    await act(async () => capture!());
    await waitFor(() => expect(document.activeElement?.id).toBe("notebox"));
    expect(document.getElementById("tab-notes")!.classList.contains("on")).toBe(true);
    // 再按一次（已經在筆記分頁）：游標回到輸入框
    (document.activeElement as HTMLElement).blur();
    await act(async () => capture!());
    await waitFor(() => expect(document.activeElement?.id).toBe("notebox"));
  });

  it("「＋ 記一筆」打開筆記分頁；⌘Enter 記下、刪除要按兩次", async () => {
    const backend = notesBackend();
    const store = new WorkbenchStore(backend, { group: "產品開發", repos: null });
    render(<App store={store} autoStart={false} />);
    await act(async () => {
      await store.refreshAll();
    });
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "寫組筆記" })));
    expect(backend.action).toHaveBeenCalledWith("group_notes", { group: "產品開發" });
    const pane = document.getElementById("tab-notes")!;
    expect((await within(pane).findByText("規格")).tagName).toBe("STRONG");
    const box = document.getElementById("notebox") as HTMLTextAreaElement;
    expect(document.activeElement).toBe(box);

    // 清單按 Enter 自動延續（跟檔案編輯一樣）
    fireEvent.change(box, { target: { value: "- 第一件" } });
    box.setSelectionRange(5, 5);
    await act(async () => fireEvent.keyDown(box, { key: "Enter" }));
    expect(box.value).toBe("- 第一件\n- ");

    fireEvent.change(box, { target: { value: "  明天跟設計對齊  " } });
    expect(JSON.parse(localStorage.getItem("workbench:noteDraft:產品開發")!)).toBe(
      "  明天跟設計對齊  ",
    );
    await act(async () => fireEvent.keyDown(box, { key: "Enter", metaKey: true }));
    expect(backend.action).toHaveBeenCalledWith("note_add", {
      group: "產品開發",
      text: "明天跟設計對齊",
    });
    expect(await within(pane).findByText("明天跟設計對齊")).toBeTruthy();
    expect(box.value).toBe("");

    const items = within(pane).getAllByRole("listitem");
    await act(async () => fireEvent.click(within(items[1]).getByRole("button", { name: "刪除" })));
    expect(backend.action).not.toHaveBeenCalledWith("note_remove", expect.anything());
    await act(async () =>
      fireEvent.click(within(items[1]).getByRole("button", { name: "確定刪除" })),
    );
    expect(backend.action).toHaveBeenCalledWith("note_remove", {
      group: "產品開發",
      at: "2026-10-11 09:00",
      text: "問 PM **規格**",
    });
    await waitFor(() => expect(within(pane).queryByText("規格")).toBeNull());
  });

  it("所有 repo／臨時組：不能記，說明要先選牌組", async () => {
    await mount(notesBackend());
    expect((screen.getByRole("button", { name: "寫組筆記" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    await act(async () => fireEvent.click(document.querySelector('[data-tab="notes"]')!));
    expect(document.getElementById("tab-notes")!.textContent).toContain("筆記記在牌組上");
  });
});

describe("首次設定：建立或選既有 spine", () => {
  it("預設建在 ~/repo-spine；錯誤照原句顯示；也能選既有的", async () => {
    const createSpine = vi.fn(async () => {
      throw new Error("這個資料夾已經有東西、又不是 spine");
    });
    const setSpine = vi.fn(async () => ({ spine: "/s" }));
    render(<SpineSetup setSpine={setSpine} createSpine={createSpine} />);
    expect((screen.getByLabelText("spine 路徑") as HTMLInputElement).value).toBe("~/repo-spine");
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "建立新的 spine" })));
    expect(createSpine).toHaveBeenCalledWith("~/repo-spine");
    expect(screen.getByRole("alert").textContent).toContain("又不是 spine");

    fireEvent.change(screen.getByLabelText("spine 路徑"), { target: { value: " ~/old " } });
    expect(screen.queryByRole("alert")).toBeNull();
    await act(async () =>
      fireEvent.click(screen.getByRole("button", { name: "使用既有的 spine" })),
    );
    expect(setSpine).toHaveBeenCalledWith("~/old");
  });
});

describe("⌘K 找 repo／牌組", () => {
  const result = (query: string) => ({
    ok: true,
    query,
    repos: query.includes("zz")
      ? []
      : [
          {
            repo: "a",
            score: 50,
            why: ["名稱開頭"],
            role: "知識庫",
            description: "書摘與卡片",
            groups: ["產品開發"],
          },
          { repo: "b", score: 2, why: ["介紹命中"], groups: [] },
        ],
    groups: query ? [] : [{ name: "產品開發", members: 2, goal: "把登入做完" }],
    files:
      query === "plan"
        ? [{ repo: "a", path: "docs/plan.md", name: "plan.md", abs: "/r/a/docs/plan.md" }]
        : [],
  });
  const backendWithPalette = () =>
    mockBackend({
      action: vi.fn(async (name: string, p: { query?: string }) => {
        if (name === "palette") return result(p.query ?? "");
        if (name === "repo_tree")
          return { ok: true, id: "a", root: "/r/a", dir: "", more: 0, entries: [] };
        if (name === "repo_profile") return { ok: true, id: "a", path: "/r/a" };
        return { ok: true };
      }) as never,
    });
  async function mount(backend: ReturnType<typeof mockBackend>) {
    const store = new WorkbenchStore(backend, { group: null, repos: null });
    render(<App store={store} autoStart={false} />);
    await act(async () => {
      await store.refreshAll();
    });
    return store;
  }
  const box = () => screen.getByRole("combobox", { name: "搜尋或執行指令" });
  const opts = () => within(screen.getByRole("listbox", { name: "結果" })).getAllByRole("option");
  const noBox = () => screen.queryByRole("combobox", { name: "搜尋或執行指令" });

  it("「?」請助手：先給計畫、驗證不過的不能勾、執行走原本的 action、記一筆（F-63）", async () => {
    const plan = {
      ok: true,
      text: "a 加標籤 規格 然後切到產品開發",
      summary: "加標籤再切組",
      source: "agent",
      steps: [
        {
          op: "tag",
          args: { id: "a", add: ["規格"], remove: [] },
          text: "a：加標籤「規格」",
          why: "",
          danger: false,
          undo: { op: "tag", args: { id: "a", add: [], remove: ["規格"] } },
        },
        {
          op: "remove_group",
          args: {},
          text: "remove_group",
          why: "",
          danger: false,
          error: "助手不能做「remove_group」（不在白名單）",
        },
        {
          op: "switch_scope",
          args: { group: "產品開發" },
          text: "切到牌組「產品開發」",
          why: "",
          danger: false,
        },
      ],
    };
    const backend = mockBackend({
      action: vi.fn(async (name: string, p: { query?: string }) => {
        if (name === "palette") return result(p.query ?? "");
        if (name === "assist_plan") return plan;
        return { ok: true };
      }) as never,
    });
    const store = await mount(backend);
    await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    fireEvent.change(box(), { target: { value: "?a 加標籤 規格 然後切到產品開發" } });
    expect(opts()).toHaveLength(1);
    expect(opts()[0].textContent).toContain("請助手：a 加標籤 規格 然後切到產品開發");
    await act(async () => fireEvent.keyDown(box(), { key: "Enter" }));
    expect(backend.action).toHaveBeenCalledWith("assist_plan", {
      text: "a 加標籤 規格 然後切到產品開發",
    });
    const dialog = document.getElementById("detaildialog")!;
    await waitFor(() => expect(dialog.textContent).toContain("加標籤再切組"));
    const boxes = [...dialog.querySelectorAll<HTMLInputElement>("input[type=checkbox]")];
    expect(boxes.map((b) => b.disabled)).toEqual([false, true, false]);
    expect(dialog.textContent).toContain("不能執行：助手不能做");
    await act(async () =>
      fireEvent.click(within(dialog).getByRole("button", { name: "執行 2 步" })),
    );
    expect(backend.action).toHaveBeenCalledWith("tag", { id: "a", add: ["規格"], remove: [] });
    expect(store.getSnapshot().scope).toEqual({ group: "產品開發", repos: null });
    expect(backend.action).toHaveBeenCalledWith("assist_log", {
      text: "a 加標籤 規格 然後切到產品開發",
      group: "產品開發",
      done: ["a：加標籤「規格」", "切到牌組「產品開發」"],
      skipped: ["remove_group"],
    });
    // 復原：倒著做——先切回原本的範圍，再把標籤拿掉
    await act(async () =>
      fireEvent.click(within(dialog).getByRole("button", { name: "復原這次的變更" })),
    );
    expect(store.getSnapshot().scope).toEqual({ group: null, repos: null });
    expect(backend.action).toHaveBeenCalledWith("tag", { id: "a", add: [], remove: ["規格"] });
    expect(backend.action).toHaveBeenCalledWith("assist_log", {
      text: "復原：a 加標籤 規格 然後切到產品開發",
      done: ["切到牌組「產品開發」", "a：加標籤「規格」"],
      skipped: [],
    });
    expect(within(dialog).queryByRole("button", { name: "復原這次的變更" })).toBeNull();
  });

  it("助手改類別：走 repo_class，復原回原本的類別", async () => {
    const plan = {
      ok: true,
      text: "把 a 設成參考",
      summary: "a：類別 → 參考",
      source: "rules",
      steps: [
        {
          op: "set_class",
          args: { id: "a", class: "reference" },
          text: "a：類別 → 參考",
          why: "",
          danger: false,
          undo: { op: "set_class", args: { id: "a", class: "" } },
        },
      ],
    };
    const backend = mockBackend({
      action: vi.fn(async (name: string, p: { query?: string }) => {
        if (name === "palette") return result(p.query ?? "");
        if (name === "assist_plan") return plan;
        return { ok: true };
      }) as never,
    });
    await mount(backend);
    await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    fireEvent.change(box(), { target: { value: "?把 a 設成參考" } });
    await act(async () => fireEvent.keyDown(box(), { key: "Enter" }));
    const dialog = document.getElementById("detaildialog")!;
    await waitFor(() => expect(dialog.textContent).toContain("a：類別 → 參考"));
    await act(async () =>
      fireEvent.click(within(dialog).getByRole("button", { name: "執行 1 步" })),
    );
    expect(backend.action).toHaveBeenCalledWith("repo_class", { id: "a", class: "reference" });
    await act(async () =>
      fireEvent.click(within(dialog).getByRole("button", { name: "復原這次的變更" })),
    );
    expect(backend.action).toHaveBeenCalledWith("repo_class", { id: "a", class: "" });
  });

  it("助手改 App 偏好：寫進本機、復原回到預設", async () => {
    const backend = mockBackend({
      action: vi.fn(async (name: string, p: { query?: string }) => {
        if (name === "palette") return result(p.query ?? "");
        if (name === "assist_plan")
          return {
            ok: true,
            text: "關掉自動儲存",
            summary: "改 App 偏好",
            source: "rules",
            steps: [
              {
                op: "set_pref",
                args: { key: "autosave", value: "false" },
                text: "App 偏好「自動儲存」：關",
                why: "",
                danger: false,
              },
            ],
          };
        return { ok: true };
      }) as never,
    });
    await mount(backend);
    await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    fireEvent.change(box(), { target: { value: "?關掉自動儲存" } });
    await act(async () => fireEvent.keyDown(box(), { key: "Enter" }));
    const dialog = document.getElementById("detaildialog")!;
    await waitFor(() => expect(dialog.textContent).toContain("自動儲存」：關"));
    await act(async () =>
      fireEvent.click(within(dialog).getByRole("button", { name: "執行 1 步" })),
    );
    expect(localStorage.getItem("workbench:autosave")).toBe("false");
    await act(async () =>
      fireEvent.click(within(dialog).getByRole("button", { name: "復原這次的變更" })),
    );
    expect(localStorage.getItem("workbench:autosave")).toBeNull();
  });

  it("只打「?」：列出最近用過的指令（Enter 再跑）＋例子", async () => {
    const backend = mockBackend({
      action: vi.fn(async (name: string, p: { query?: string }) => {
        if (name === "palette") return result(p.query ?? "");
        if (name === "assist_recent")
          return { ok: true, texts: ["切到 產品開發", "a 加標籤 規格"] };
        return { ok: true };
      }) as never,
    });
    await mount(backend);
    await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    await act(async () => fireEvent.change(box(), { target: { value: "?" } }));
    await waitFor(() => expect(opts()).toHaveLength(2));
    expect(opts()[0].textContent).toContain("請助手：切到 產品開發");
    expect(document.querySelector(".paletteempty")!.textContent).toContain("最近用過的");
    expect(
      vi.mocked(backend.action).mock.calls.filter((c) => c[0] === "assist_recent"),
    ).toHaveLength(1);
  });

  it("引擎回的結果缺欄位（舊版殼、錯誤回應）：面板不炸，指令照樣能用", async () => {
    const backend = mockBackend({
      action: vi.fn(async (name: string) => {
        if (name === "palette") return { ok: true };
        return { ok: true };
      }) as never,
    });
    await mount(backend);
    await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    await waitFor(() => expect(backend.action).toHaveBeenCalledWith("palette", { query: "" }));
    await act(async () => {});
    expect(box()).toBeTruthy();
    fireEvent.change(box(), { target: { value: ">活動" } });
    expect(opts().some((o) => o.textContent?.includes("活動"))).toBe(true);
  });

  it("一般搜尋也列「請助手」但放最後，不搶 Enter", async () => {
    const backend = backendWithPalette();
    await mount(backend);
    await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    await waitFor(() => expect(opts()).toHaveLength(3));
    fireEvent.change(box(), { target: { value: "知識" } });
    await waitFor(() => expect(opts().length).toBeGreaterThan(1));
    const all = opts();
    expect(all[all.length - 1].textContent).toContain("請助手：知識");
    expect(all[0].textContent).not.toContain("請助手");
  });

  it("⌘K 開；空查詢列出 repo 與牌組；打字問引擎；Enter 在側欄開 repo；Esc 關", async () => {
    const backend = backendWithPalette();
    await mount(backend);
    await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    await waitFor(() => expect(opts()).toHaveLength(3));
    expect(backend.action).toHaveBeenCalledWith("palette", { query: "" });
    const first = opts();
    // 牌組在前
    expect(first[0].textContent).toContain("目標：把登入做完");
    expect(first[1].textContent).toContain("知識庫");
    expect(first[1].textContent).toContain("書摘與卡片");
    expect(first[0].getAttribute("aria-selected")).toBe("true");
    expect(box().getAttribute("writingsuggestions")).toBe("false");
    expect(box().getAttribute("autocomplete")).toBe("off");

    fireEvent.change(box(), { target: { value: "zz" } });
    await waitFor(() => expect(screen.getByText(/沒有符合「zz」/)).toBeTruthy());
    expect(backend.action).toHaveBeenCalledWith("palette", { query: "zz" });

    fireEvent.change(box(), { target: { value: "a" } });
    await waitFor(() => expect(opts()).toHaveLength(2));
    fireEvent.keyDown(box(), { key: "ArrowDown" });
    expect(opts()[1].getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(box(), { key: "ArrowUp" });
    await act(async () => fireEvent.keyDown(box(), { key: "Enter" }));
    expect(noBox()).toBeNull();
    expect(document.getElementById("repopanel")!.getAttribute("aria-label")).toBe("Repo：a");

    // 再選同一個不會把側欄收起（showRepo 不是 toggle）
    await act(async () => fireEvent.keyDown(window, { key: "k", ctrlKey: true }));
    await waitFor(() => expect(opts()).toHaveLength(3));
    await act(async () => fireEvent.keyDown(box(), { key: "Enter" }));
    expect(document.getElementById("repopanel")).toBeTruthy();

    await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    await act(async () => fireEvent.keyDown(box(), { key: "Escape" }));
    expect(noBox()).toBeNull();
  });

  it("選牌組切範圍；⌘Enter 對 repo 開會話對話框；失敗保留清單並顯示原因", async () => {
    const backend = backendWithPalette();
    const store = await mount(backend);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: /搜尋或執行指令/ })));
    await waitFor(() => expect(opts()).toHaveLength(3));
    await act(async () => fireEvent.click(opts()[0]));
    expect(store.getSnapshot().scope.group).toBe("產品開發");

    await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    await waitFor(() => expect(opts()).toHaveLength(3));
    (backend.action as ReturnType<typeof vi.fn>).mockImplementationOnce(async () => {
      throw new Error("registry 讀不到");
    });
    fireEvent.change(box(), { target: { value: "x" } });
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("registry 讀不到"));
    expect(opts()).toHaveLength(3);
    fireEvent.keyDown(box(), { key: "ArrowDown" });
    await act(async () => fireEvent.keyDown(box(), { key: "Enter", metaKey: true }));
    expect(document.getElementById("detaildialog")!.hasAttribute("open")).toBe(true);
    expect(document.getElementById("detailtitle")!.textContent).toBe("開啟 Agent 會話");
  });

  it("檔案：Enter 開檔、⌘Enter 加入挑選；查詢帶目前範圍", async () => {
    const backend = backendWithPalette();
    const store = await mount(backend);
    store.setScope({ group: "產品開發", repos: null });
    await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    fireEvent.change(box(), { target: { value: "plan" } });
    await waitFor(() =>
      expect(opts().some((o) => o.textContent!.includes("docs/plan.md"))).toBe(true),
    );
    expect(backend.action).toHaveBeenCalledWith("palette", { query: "plan", group: "產品開發" });
    const fileIdx = opts().findIndex((o) => o.textContent!.includes("docs/plan.md"));
    for (let i = 0; i < fileIdx; i++) fireEvent.keyDown(box(), { key: "ArrowDown" });
    await act(async () => fireEvent.keyDown(box(), { key: "Enter", metaKey: true }));
    expect(noBox()).toBeNull();
    expect(document.getElementById("picktray")!.textContent).toContain("脈絡 1 個檔案");

    await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    fireEvent.change(box(), { target: { value: "plan" } });
    await waitFor(() =>
      expect(opts().some((o) => o.textContent!.includes("docs/plan.md"))).toBe(true),
    );
    await act(async () =>
      fireEvent.click(opts().find((o) => o.textContent!.includes("docs/plan.md"))!),
    );
    expect(backend.action).toHaveBeenCalledWith("read_file", { id: "a", path: "docs/plan.md" });
  });

  it("會話：等你的排最前、打字依標題／任務／所在過濾，Enter 切到那個會話", async () => {
    const term = (sid: string, title: string, extra: Record<string, unknown> = {}) => ({
      sid,
      title,
      alive: true,
      kind: "agent",
      origin: null,
      scope: "a",
      scope_repos: ["a"],
      started: "2026-10-14 01:00",
      task: null,
      report: null,
      report_basis: "",
      agent_status: null,
      ...extra,
    });
    const backend = mockBackend({
      getState: vi.fn(async () =>
        makeState({
          terms: [
            term("t1", "claude · a", { task: "整理 README" }),
            term("t2", "claude · b", {
              scope: "b",
              scope_repos: ["b"],
              agent_status: { status: "blocked", text: "要你允許 Bash", at: "2026-10-14 01:05" },
            }),
            term("t3", "shell", { kind: "shell" }),
          ] as never,
        }),
      ),
      action: vi.fn(async (name: string, p: { query?: string }) =>
        name === "palette" ? result(p.query ?? "") : { ok: true },
      ) as never,
    });
    await mount(backend);
    await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    await waitFor(() => expect(opts().length).toBeGreaterThan(2));
    const first = opts();
    expect(first[0].textContent).toContain("claude · b");
    expect(first[0].textContent).toContain("卡住了");
    expect(first[0].textContent).toContain("要你允許 Bash");
    expect(first[1].textContent).toContain("claude · a");
    expect(first.some((o) => o.textContent!.includes("shell"))).toBe(false);
    fireEvent.change(box(), { target: { value: "README" } });
    await waitFor(() => expect(opts()[0].textContent).toContain("任務：整理 README"));
    expect(opts().filter((o) => o.textContent!.startsWith("會話"))).toHaveLength(1);
    await act(async () => fireEvent.keyDown(box(), { key: "Enter" }));
    expect(noBox()).toBeNull();
    expect(document.getElementById("termwhere")!.textContent).toBe("a");
  });

  it("指令：「>」只列指令；打字比對名稱或別名；Enter 執行", async () => {
    const backend = backendWithPalette();
    await mount(backend);
    await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    fireEvent.change(box(), { target: { value: ">" } });
    await waitFor(() => expect(opts().length).toBeGreaterThan(5));
    expect(opts().every((o) => o.textContent!.startsWith("指令"))).toBe(true);
    fireEvent.change(box(), { target: { value: ">筆記" } });
    await waitFor(() => expect(opts().map((o) => o.textContent)).toContain("指令切到「筆記」分頁"));
    // 一般查詢：別名也找得到（note）
    fireEvent.change(box(), { target: { value: "note" } });
    await waitFor(() =>
      expect(opts().some((o) => o.textContent === "指令寫組筆記（目前牌組）")).toBe(true),
    );
    const i = opts().findIndex((o) => o.textContent === "指令寫組筆記（目前牌組）");
    for (let k = 0; k < i; k++) fireEvent.keyDown(box(), { key: "ArrowDown" });
    await act(async () => fireEvent.keyDown(box(), { key: "Enter" }));
    expect(noBox()).toBeNull();
    expect(document.getElementById("tab-notes")!.classList.contains("on")).toBe(true);
  });

  it("焦點在終端裡時 Ctrl+K 留給 shell", async () => {
    await mount(backendWithPalette());
    const term = document.createElement("div");
    term.className = "xterm";
    const ta = document.createElement("textarea");
    term.appendChild(ta);
    document.body.appendChild(term);
    await act(async () => fireEvent.keyDown(ta, { key: "k", ctrlKey: true }));
    expect(noBox()).toBeNull();
    await act(async () => fireEvent.keyDown(ta, { key: "k", metaKey: true }));
    expect(box()).toBeTruthy();
    term.remove();
  });
});

describe("這組標為已讀（N-37）", () => {
  it("牌組範圍才出現；只送目前範圍；沒有未讀時停用", async () => {
    const backend = mockBackend({
      getState: vi.fn(async () =>
        makeState({
          cards: [{ ...interruptCard, key: "e1", kind: "event", lane: "scope" }] as never,
        }),
      ),
    });
    const store = new WorkbenchStore(backend, { group: "產品開發", repos: null });
    render(<App store={store} autoStart={false} />);
    await act(async () => {
      await store.refreshAll();
    });
    const btn = screen.getByRole("button", { name: "這組標為已讀" }) as HTMLButtonElement;
    expect(document.getElementById("worksummary")!.textContent).toContain("未讀 1");
    expect(btn.disabled).toBe(false);
    await act(async () => fireEvent.click(btn));
    expect(backend.action).toHaveBeenCalledWith("ack_scope", { group: "產品開發" });
  });
});
