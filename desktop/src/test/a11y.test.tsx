import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import axe from "axe-core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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
            // R37：會話列小量表
            claude_session: "8d110180-5810",
            resume_cwd: "/r/a",
            context_pct: 84,
            model: "Opus",
            subagents: 2,
            auto_continue: { prompt: "繼續", max: 3, count: 1 },
          },
          {
            // R37 F-99：出錯中斷（「繼續」按鈕）＋快取已冷
            sid: "t2",
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
              status: "waiting",
              text: "這一輪出錯中斷：rate limit",
              at: "2026-10-14 01:05",
              event: "stop-failure",
            },
            cache_cold: true,
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
      if (name === "read_file")
        return {
          ok: true,
          id: "a",
          path: "README.md",
          abs: "/r/a/README.md",
          kind: "text",
          text: "# 說明\n\n- [ ] 待辦\n",
          editable: true,
          version: "v1",
          size: 20,
        };
      if (name === "backlinks") return { ok: true, id: "a", path: "README.md", links: [] };
      if (name === "session_transcript")
        return {
          ok: true,
          id: "x",
          truncated: false,
          title: "t",
          path: "/p.jsonl",
          frames: [
            { kind: "you", text: "跑測試", at: "2026-10-14T01:00:00Z" },
            { kind: "thinking", text: "嗯", at: null },
            { kind: "tool", tool: "Bash", text: "npm test", result: "fail", error: true, at: null },
            { kind: "reply", text: "好了", at: null },
          ],
        };
      if (name === "prompts")
        return {
          ok: true,
          dir: "/sp/prompts",
          prompts: [{ name: "下一步", text: "照計畫做下一步", updated: "2026-10-22 00:30" }],
        };
      if (name === "agent_commands")
        return {
          ok: true,
          cwd: "/r/a",
          commands: [{ name: "lint", description: "跑 lint", kind: "command", source: "project" }],
        };
      if (name === "repo_worktrees")
        return {
          ok: true,
          id: "a",
          worktrees: [
            {
              path: "/r/a",
              branch: "main",
              head: "0123abcd",
              main: true,
              dirty: 0,
              unique: 0,
              locked: false,
              prunable: false,
              removable: false,
            },
            {
              path: "/r/a/wt",
              branch: "x",
              head: "fedcba98",
              main: false,
              dirty: 0,
              unique: 0,
              locked: false,
              prunable: false,
              removable: true,
            },
          ],
        };
      if (name === "repo_github")
        return {
          ok: true,
          id: "a",
          prs: [
            {
              number: 1,
              title: "修登入",
              url: "https://github.com/o/a/pull/1",
              author: "me",
              updated: null,
              branch: "fix",
              draft: false,
              review: null,
              labels: [],
            },
          ],
          issues: [],
        };
      if (name === "repo_log")
        return {
          ok: true,
          id: "a",
          commits: [
            {
              sha: "a1b2c3d4",
              short: "a1b2c3d",
              subject: "修好登入",
              author: "me",
              at: "2026-10-14T01:00:00Z",
            },
          ],
        };
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
  // A11Y_ALL=1 npx vitest run src/test/a11y.test.tsx：連 minor／moderate 都印出來看
  if (process.env.A11Y_ALL)
    console.log(
      r.violations
        .map(
          (v) =>
            `${v.id}（${v.impact}）${v.help}：` + v.nodes.map((n) => n.target.join(" ")).join("、"),
        )
        .join("\n"),
    );
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
  // 偏好（分頁、開著的檔）記在 localStorage：每個案例都從乾淨的狀態開始
  beforeEach(() => localStorage.clear());
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
  it("開著一份 md（檢視器＋編輯工具列）時沒有違規", async () => {
    const found = await violations({ group: "產品開發", repos: null }, async () => {
      await act(async () =>
        fireEvent.click(
          within(document.getElementById("tabs")!).getByRole("button", { name: "檔案" }),
        ),
      );
      await act(async () =>
        fireEvent.click(
          within(document.getElementById("tab-files")!).getByRole("button", { name: "a" }),
        ),
      );
      const readme = await screen.findByRole("treeitem", { name: /README\.md/ });
      await act(async () => fireEvent.click(readme));
      const viewer = document.querySelector(".viewer:not([hidden])") as HTMLElement | null;
      expect(viewer).not.toBeNull();
      await act(async () => fireEvent.click(within(viewer!).getByRole("button", { name: "編輯" })));
      expect(viewer!.querySelector("textarea")).not.toBeNull();
    });
    expect(found, found.join("\n")).toEqual([]);
  });
  const openTermMenuItem = async (label: string) => {
    // 側欄第一場會話（有小量表的那場）
    await act(async () => fireEvent.click(document.querySelector<HTMLElement>(".session button")!));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "更多會話動作" })));
    await act(async () => fireEvent.click(screen.getByRole("menuitem", { name: label })));
  };
  it("R37：逐字稿對話框開著時沒有違規", async () => {
    const found = await violations({ group: "產品開發", repos: null }, () =>
      openTermMenuItem("逐字稿（完整對話與工具）"),
    );
    expect(found, found.join("\n")).toEqual([]);
  });
  it("R37：常用 prompt、插入 /指令、自動繼續對話框開著時沒有違規", async () => {
    for (const label of ["常用 prompt…", "插入 /指令…", "自動繼續（1/3）…"]) {
      const found = await violations({ group: "產品開發", repos: null }, () =>
        openTermMenuItem(label),
      );
      expect(found, label + "\n" + found.join("\n")).toEqual([]);
      cleanup();
      localStorage.clear();
    }
  });
  it("R37：右側欄展開 Worktree、GitHub、最近的 commit 時沒有違規", async () => {
    const found = await violations({ group: "產品開發", repos: null }, async () => {
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
      for (const sel of [".panelworktrees", ".panelgithub", ".panelcommits"]) {
        const d = document.querySelector<HTMLDetailsElement>(sel)!;
        expect(d, sel).not.toBeNull();
        await act(async () => {
          d.open = true;
          d.dispatchEvent(new Event("toggle"));
        });
      }
      expect(document.querySelector(".wtrow")).not.toBeNull();
      expect(document.querySelector(".ghrow")).not.toBeNull();
      expect(document.querySelector(".commitrow")).not.toBeNull();
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
