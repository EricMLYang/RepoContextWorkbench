import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { WorkbenchStore } from "../store/store";
import type { TidyItem, TidyResult } from "../api/types";
import { mockBackend } from "./fixtures";

const item = (id: string, bucket: TidyItem["bucket"], text: string): TidyItem => ({
  id,
  text,
  bucket,
  why: "理由",
  due: "2026-10-01",
  overdue: true,
  opened: "2026-09-17 10:00",
  age_days: 54,
  evidence:
    bucket === "done" ? [{ kind: "commit", ref: "a@1234567", at: "", text: "feat：做掉了" }] : [],
});

function tidy(items: TidyItem[]): TidyResult {
  const n = (b: string) => items.filter((it) => it.bucket === b).length;
  return {
    ok: true,
    scope: { group: null, repos: null, label: "全部" },
    counts: {
      done: n("done"),
      shelved: n("shelved"),
      ask: n("ask"),
      live: n("live"),
      total: items.length,
    },
    suggest_close: items
      .filter((it) => it.bucket === "done" || it.bucket === "shelved")
      .map((it) => it.id),
    items,
  };
}

beforeEach(() => localStorage.clear());
afterEach(() => cleanup());

describe("清理未結", () => {
  it("分類列出、預先勾好建議關的；改勾之後一次關掉，關完重讀清單並說關了幾項", async () => {
    let items = [
      item("#1", "done", "已經做掉的事"),
      item("#2", "shelved", "兩個月前的事"),
      item("#3", "ask", "使用者決定：要不要換"),
      item("#4", "live", "這週要做的事"),
    ];
    const action = vi.fn(async (name: string, p: { nums?: string[] }) => {
      if (name === "todo_tidy") return tidy(items);
      if (name === "close_loops") {
        const nums = p.nums || [];
        items = items.filter((it) => !nums.includes(it.id));
        return { ok: true, closed: nums.map((id) => ({ id, closed: "x" })), failed: [] };
      }
      return { ok: true };
    });
    const store = new WorkbenchStore(mockBackend({ action: action as never }), {
      group: null,
      repos: null,
    });
    render(<App store={store} autoStart={false} />);
    await act(async () => {
      await store.refreshAll();
    });
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "清理未結…" })));
    await screen.findByText(/共 4 項——可能已做完 1、擱置太久 1、等你決定 1、還在進行 1/);
    // 建議關的（可能已做完＋擱置太久）預先勾好，其他不勾
    const box = (text: string) =>
      screen.getByText(text).closest("label")!.querySelector("input") as HTMLInputElement;
    expect(box("已經做掉的事").checked).toBe(true);
    expect(box("兩個月前的事").checked).toBe(true);
    expect(box("使用者決定：要不要換").checked).toBe(false);
    expect(box("這週要做的事").checked).toBe(false);
    expect(screen.getByText(/證據：a@1234567 feat：做掉了/)).toBeTruthy();

    // 多勾一項、取消一項
    await act(async () => fireEvent.click(box("使用者決定：要不要換")));
    await act(async () => fireEvent.click(box("已經做掉的事")));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "關掉勾選的 2 項" })));
    await waitFor(() =>
      expect(action).toHaveBeenCalledWith("close_loops", { nums: ["#2", "#3"], note: undefined }),
    );
    await screen.findByText("已關閉 2 項");
    expect(screen.queryByText("兩個月前的事")).toBeNull();
    // 還勾著的只剩還開著的那一項
    expect(screen.getByRole("button", { name: "關掉勾選的 0 項" })).toHaveProperty(
      "disabled",
      true,
    );
  });

  it("讀不到清單：顯示原因，不是空白", async () => {
    const action = vi.fn(async (name: string) => {
      if (name === "todo_tidy") throw new Error("spine 讀不到");
      return { ok: true };
    });
    const store = new WorkbenchStore(mockBackend({ action: action as never }), {
      group: null,
      repos: null,
    });
    render(<App store={store} autoStart={false} />);
    await act(async () => {
      await store.refreshAll();
    });
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "清理未結…" })));
    await screen.findByText("讀不到未結清單：spine 讀不到");
  });
});
