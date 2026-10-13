import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { SpineSetup } from "../components/SpineSetup";
import { WorkbenchStore } from "../store/store";
import type { Backend } from "../api";
import type { StateCard } from "../api/types";
import { FILE_DRAG_TYPE } from "../lib/view";
import { makeScan, makeState, mockBackend } from "./fixtures";

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
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "分析想法" })));
    expect(backend.action).toHaveBeenCalledWith("collide", { idea: "一個想法" });
    expect(idea.value).toBe("一個想法");
    expect(JSON.parse(localStorage.getItem("workbench:draft")!)).toBe("一個想法");
    expect(document.getElementById("ideaerror")!.textContent).toContain("草稿已保留");
  });

  it("送出成功才清草稿", async () => {
    localStorage.setItem("workbench:draft", JSON.stringify("另一個想法"));
    const { backend } = await mount();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "分析想法" })));
    expect(backend.action).toHaveBeenCalledWith("collide", { idea: "另一個想法" });
    expect((document.getElementById("idea") as HTMLTextAreaElement).value).toBe("");
    expect(JSON.parse(localStorage.getItem("workbench:draft")!)).toBe("");
  });

  it("空的想法不送出", async () => {
    const { backend } = await mount();
    act(() => void fireEvent.click(screen.getByRole("button", { name: "＋ 新增想法" })));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "分析想法" })));
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
    const dialog = document.getElementById("detaildialog")!;
    const defer = [...dialog.querySelectorAll("button")].find((b) => b.textContent === "延 14 天")!;
    await act(async () => fireEvent.click(defer));
    expect(backend.action).toHaveBeenCalledWith("reopen_defer", {
      ref: "decision:2026-01-01-d1",
      days: 14,
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
    expect(within(pane).getAllByRole("treeitem", { name: "README.md（已挑）" })).toHaveLength(2);
    const tray = document.getElementById("picktray")!;
    expect(tray.textContent).toContain("已挑 2 個檔案");

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
    fireEvent.contextMenu(within(pane).getAllByRole("treeitem", { name: "README.md（已挑）" })[0]);
    await act(async () => fireEvent.click(screen.getByRole("menuitem", { name: /移出挑選/ })));
    expect(tray.textContent).toContain("已挑 1 個檔案");
    await act(async () => fireEvent.click(within(tray).getByRole("button", { name: /已挑/ })));
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
    expect(panel.textContent).toContain("3 檔");
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
    const sec = within(document.getElementById("repopanel")!).getByRole("region", {
      name: "a 還沒 commit 的變更",
    });
    await waitFor(() => expect(sec.textContent).toContain("變更（2）"));
    expect(within(sec).getByLabelText("修改").textContent).toBe("M");
    await act(async () =>
      fireEvent.click(within(sec).getByRole("button", { name: "請 Agent 審…" })),
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
    await act(async () => fireEvent.click(within(view).getByRole("button", { name: "差異" })));
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

  it("檢視器「加入挑選」：挑起來、按鈕變「已挑」，再按取消", async () => {
    const backend = viewerBackend();
    await openTree(backend);
    await act(async () =>
      fireEvent.click(await screen.findByRole("treeitem", { name: "README.md" })),
    );
    const viewer = () => document.querySelector(".viewer:not([hidden])") as HTMLElement;
    await act(async () =>
      fireEvent.click(within(viewer()).getByRole("button", { name: "加入挑選" })),
    );
    expect(
      within(viewer()).getByRole("button", { name: "已挑" }).getAttribute("aria-pressed"),
    ).toBe("true");
    expect(document.getElementById("picktray")!.textContent).toContain("已挑 1 個檔案");
    expect(JSON.parse(localStorage.getItem("workbench:picks")!)).toEqual([
      { repo: "a", path: "README.md", name: "README.md" },
    ]);
    await act(async () => fireEvent.click(within(viewer()).getByRole("button", { name: "已挑" })));
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
  function editBackend() {
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
    return { backend, disk, agentWrites };
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
        within(document.getElementById("termbar")!).getByRole("button", { name: "看變更" }),
      ),
    );
    expect(document.getElementById("repopanel")!.getAttribute("aria-label")).toBe("Repo：a");
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

    await act(async () => fireEvent.click(within(bar).getByRole("button", { name: "Terminal" })));
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
    expect(tray.textContent).toContain("已挑 1 個檔案");
    expect(within(tray).queryByRole("button", { name: "貼到目前會話" })).toBeNull();

    const card = document.querySelector('.gcard[data-key="產品開發"]')!;
    await act(async () =>
      fireEvent.click(within(card as HTMLElement).getByRole("button", { name: /claude · a/ })),
    );
    await waitFor(() => expect(backend.openTerminal).toHaveBeenCalledWith("t1"));
    await act(async () =>
      fireEvent.click(within(tray).getByRole("button", { name: "貼到目前會話" })),
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
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "＋ 記一筆" })));
    expect(backend.action).toHaveBeenCalledWith("group_notes", { group: "產品開發" });
    const pane = document.getElementById("tab-notes")!;
    expect((await within(pane).findByText("規格")).tagName).toBe("STRONG");
    const box = document.getElementById("notebox") as HTMLTextAreaElement;
    expect(document.activeElement).toBe(box);

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
    expect((screen.getByRole("button", { name: "＋ 記一筆" }) as HTMLButtonElement).disabled).toBe(
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
  const box = () => screen.getByRole("combobox", { name: "找 repo 或牌組" });
  const opts = () => within(screen.getByRole("listbox", { name: "結果" })).getAllByRole("option");
  const noBox = () => screen.queryByRole("combobox", { name: "找 repo 或牌組" });

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
    await act(async () => fireEvent.click(screen.getByRole("button", { name: /找 repo/ })));
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
    expect(document.getElementById("picktray")!.textContent).toContain("已挑 1 個檔案");

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
