import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import axe from "axe-core";
import { afterEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { WorkbenchStore } from "../store/store";
import { interruptCard, loopCard, makeScan, makeState, mockBackend } from "./fixtures";

/**
 * 無障礙稽核（#12 的自動化部分）：把 App 用假資料掛起來，跑 axe-core，serious／critical 的違規要是 0。
 * jsdom 算不了顏色對比，color-contrast 關掉（那要真瀏覽器）；其餘規則照 WCAG 2.1 AA 預設。
 */
async function violations(
  scope: { group: string | null; repos: string[] | null },
  prepare: () => Promise<void> = async () => {},
) {
  const backend = mockBackend({
    getState: vi.fn(async () =>
      makeState({
        cards: [interruptCard, loopCard],
        terms: [
          {
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
            home_group: "產品開發",
            agent_status: {
              status: "blocked",
              text: "要你允許 Bash",
              at: "2026-10-14 01:05",
              event: "permission",
            },
          },
        ] as never,
      }),
    ),
    getScan: vi.fn(async () =>
      makeScan({
        states: [
          {
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
          },
        ] as never,
      }),
    ),
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
      if (name === "repo_changes")
        return { ok: true, id: "a", changes: [{ path: "README.md", status: "modified" }], more: 0 };
      return { ok: true };
    }) as never,
  });
  const store = new WorkbenchStore(backend, scope);
  render(<App store={store} autoStart={false} />);
  await act(async () => {
    await store.refreshAll();
  });
  await prepare();
  const r = await axe.run(document.body, {
    rules: { "color-contrast": { enabled: false } },
  });
  return r.violations
    .filter((v) => v.impact === "serious" || v.impact === "critical")
    .map(
      (v) =>
        `${v.id}（${v.impact}）：${v.help}\n  ` +
        v.nodes
          .slice(0, 3)
          .map((n) => n.target.join(" "))
          .join("\n  "),
    );
}

describe("無障礙稽核（axe-core）", () => {
  afterEach(() => cleanup());
  it("工作區（全部範圍）沒有 serious／critical 違規", async () => {
    const found = await violations({ group: null, repos: null });
    expect(found, found.join("\n")).toEqual([]);
  });
  it("牌組範圍（有會話、卡片）沒有 serious／critical 違規", async () => {
    const found = await violations({ group: "產品開發", repos: null });
    expect(found, found.join("\n")).toEqual([]);
  });
  it("開著「新建 Agent 會話…」對話框時沒有違規", async () => {
    const found = await violations({ group: "產品開發", repos: null }, async () => {
      await act(async () =>
        fireEvent.click(screen.getByRole("button", { name: "新建 Agent 會話…" })),
      );
    });
    expect(found, found.join("\n")).toEqual([]);
  });
  it("⌘K 開著時沒有違規", async () => {
    const found = await violations({ group: "產品開發", repos: null }, async () => {
      await act(async () => fireEvent.keyDown(window, { key: "k", metaKey: true }));
    });
    expect(found, found.join("\n")).toEqual([]);
  });
  it("右側欄開著（檔案／變更／資訊）時沒有違規", async () => {
    const found = await violations({ group: "產品開發", repos: null }, async () => {
      // 跟 app.test 一樣：Repo 分頁的表格點 repo 名開右側欄
      await act(async () =>
        fireEvent.click(
          within(document.getElementById("tabs")!).getByRole("button", { name: "Repo" }),
        ),
      );
      await act(async () =>
        fireEvent.click(
          within(document.getElementById("deeptable")!).getByRole("button", { name: "a" }),
        ),
      );
      expect(document.getElementById("repopanel")).not.toBeNull();
    });
    expect(found, found.join("\n")).toEqual([]);
  });
});
