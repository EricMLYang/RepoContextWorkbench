import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { WorkbenchStore } from "../store/store";
import { makeState, mockBackend } from "./fixtures";

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
    const backend = mockBackend({ getState: vi.fn(async () => makeState({ repos: [], groups: [], cards: [] })) });
    await mount(backend);
    expect(document.querySelector("#allclear h2")!.textContent).toBe("建立你的第一個工作範圍");
    expect(screen.getByRole("button", { name: "選擇資料夾，掃描 repo" })).toBeTruthy();
  });
});

describe("範圍", () => {
  it("點牌組切範圍並以該組重新採集", async () => {
    const { backend } = await mount();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "選擇牌組：產品開發" })));
    expect(backend.getScan).toHaveBeenLastCalledWith({ group: "產品開發" });
    expect(document.getElementById("scopename")!.textContent).toBe("產品開發");
    expect(JSON.parse(localStorage.getItem("workbench:scope")!)).toEqual({
      group: "產品開發",
      repos: null,
    });
  });
});
