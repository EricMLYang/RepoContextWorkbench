import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { scopeMemoryKey } from "../lib/view";
import { WorkbenchStore } from "../store/store";
import { makeScan, makeState, mockBackend } from "./fixtures";

const GROUPS = [
  { name: "產品開發", members: ["a"], goal: "", goal_at: "" },
  { name: "知識", members: ["b"], goal: "", goal_at: "" },
];

/** state 照請求的範圍回成員；read_file 回一行字。 */
function backend(over: Parameters<typeof mockBackend>[0] = {}) {
  return mockBackend({
    getState: vi.fn(async (scope?: { group?: string }) =>
      makeState({
        groups: GROUPS,
        scope: {
          group: scope?.group ?? null,
          repos: null,
          members: GROUPS.find((g) => g.name === scope?.group)?.members ?? ["a", "b"],
        },
      }),
    ) as never,
    action: vi.fn(async (name: string, p: { id?: string; path?: string }) =>
      name === "read_file"
        ? {
            ok: true,
            id: p.id,
            path: p.path,
            abs: "/r/" + p.id + "/" + p.path,
            size: 1,
            kind: "text",
            text: "內容",
            truncated: false,
          }
        : { ok: true },
    ) as never,
    ...over,
  });
}

async function mount(b = backend()) {
  const store = new WorkbenchStore(b, { group: null, repos: null });
  render(<App store={store} autoStart={false} />);
  await act(async () => {
    await store.refreshAll();
  });
  return store;
}

const switchTo = async (store: WorkbenchStore, group: string | null) => {
  await act(async () => {
    store.setScope({ group, repos: null });
    await store.refreshAll();
  });
};
const fileTabs = () =>
  [...document.querySelectorAll(".filetab .tabname")].map((t) => t.textContent || "");

beforeEach(() => localStorage.clear());
afterEach(() => cleanup());

describe("各牌組記住自己看到哪", () => {
  it("scopeMemoryKey：牌組各一份、所有 repo 一份、臨時範圍共用一份", () => {
    expect(scopeMemoryKey({ group: "產品開發", repos: null })).toBe("g:產品開發");
    expect(scopeMemoryKey({ group: null, repos: null })).toBe("*");
    expect(scopeMemoryKey({ group: null, repos: ["a"] })).toBe("adhoc");
    expect(scopeMemoryKey({ group: null, repos: ["a", "b"] })).toBe("adhoc");
  });

  it("換牌組：檔案分頁各記各的，切回去還原；存進本機", async () => {
    localStorage.setItem(
      "workbench:view:g:產品開發",
      JSON.stringify({
        files: [{ repo: "a", path: "docs/plan.md" }],
        active: "a:docs/plan.md",
        panelRepo: "a",
        tab: "files",
        activeSid: null,
      }),
    );
    localStorage.setItem(
      "workbench:openFiles",
      JSON.stringify([{ repo: "b", path: "notes/card.md" }]),
    );
    const store = await mount();
    expect(fileTabs().join()).toContain("card.md");

    await switchTo(store, "產品開發");
    await waitFor(() => expect(fileTabs().join()).toContain("plan.md"));
    expect(fileTabs().join()).not.toContain("card.md");
    expect(document.getElementById("repopanel")!.getAttribute("aria-label")).toBe("Repo：a");
    // 離開的那個範圍（所有 repo）存下來了
    const all = JSON.parse(localStorage.getItem("workbench:view:*")!);
    expect(all.files).toEqual([{ repo: "b", path: "notes/card.md" }]);

    // 沒去過的牌組：檔案分頁是空的
    await switchTo(store, "知識");
    await waitFor(() => expect(fileTabs()).toHaveLength(0));

    // 切回去：還原
    await switchTo(store, null);
    await waitFor(() => expect(fileTabs().join()).toContain("card.md"));
    await switchTo(store, "產品開發");
    await waitFor(() => expect(fileTabs().join()).toContain("plan.md"));
  });

  it("⌘1～⌘9 切到第 n 個牌組、⌘0 回所有 repo；沒有那麼多牌組就不動", async () => {
    const store = await mount();
    await act(async () => fireEvent.keyDown(window, { key: "2", metaKey: true }));
    expect(store.getSnapshot().scope).toEqual({ group: "知識", repos: null });
    await act(async () => fireEvent.keyDown(window, { key: "1", ctrlKey: true }));
    expect(store.getSnapshot().scope.group).toBe("產品開發");
    await act(async () => fireEvent.keyDown(window, { key: "7", metaKey: true }));
    expect(store.getSnapshot().scope.group).toBe("產品開發");
    await act(async () => fireEvent.keyDown(window, { key: "0", metaKey: true }));
    expect(store.getSnapshot().scope).toEqual({ group: null, repos: null });
    // 牌組的提示帶快捷鍵
    expect(screen.getByRole("button", { name: "選擇牌組：知識" }).title).toMatch(/[⌘+]2）$/);
  });

  it("切回採集過的範圍：先顯示上次的結果，背景再更新", async () => {
    // quiet 用來認是哪一次採集：所有 repo＝1、知識＝2；hold 之後的採集卡住，回來時是 9
    let hold = false;
    let release: (() => void) | null = null;
    const b = backend({
      getScan: vi.fn(async (scope?: { group?: string }) => {
        if (hold) {
          await new Promise<void>((r) => (release = r));
          return makeScan({ quiet: 9 });
        }
        return makeScan({ quiet: scope?.group === "知識" ? 2 : 1 });
      }) as never,
    });
    const store = await mount(b);
    expect(store.getSnapshot().scan.quiet).toBe(1);
    await switchTo(store, "知識");
    expect(store.getSnapshot().scan.quiet).toBe(2);
    // 切回所有 repo：新的採集還沒回來，畫面已經是上次那份（不是空的）
    hold = true;
    act(() => store.setScope({ group: null, repos: null }));
    const snap = store.getSnapshot();
    expect(snap.scanReady).toBe(true);
    expect(snap.scan.quiet).toBe(1);
    await waitFor(() => expect(release).toBeTruthy());
    await act(async () => release!());
    await waitFor(() => expect(store.getSnapshot().scan.quiet).toBe(9));
    // 沒採集過的範圍照舊從空的開始
    act(() => store.setScope({ group: "產品開發", repos: null }));
    expect(store.getSnapshot().scanReady).toBe(false);
    await act(async () => release?.());
  });
});
