/**
 * 畫面預覽（開發用，不進 App）：真的 App 元件＋固定的假資料，用網址 `?scene=` 選場景。
 * 給沒辦法開 App 實測時（使用者的 App 開著）用 headless 瀏覽器截圖看排版：
 *   npx vite --port 5199  →  http://localhost:5199/dev/preview.html?scene=editor
 * 場景：workspace（工作區＋右欄變更頁）、editor（md 分割編輯）、code（程式碼上色＋尋找）、
 *       assist（⌘K 請助手的計畫）、assist-empty（只打「?」）、tree-new（檔案樹新增檔案的輸入列）、
 *       panel-info（右欄資訊頁）、term-menu（終端列的「⋯」）、md-view（md 預覽＋反向連結）、
 *       edit-find（編輯中的尋找取代）、no-session（沒有會話、所有 repo）、palette（一般搜尋）、composer（請 Agent 分析想法）、
 *       rename（檔案樹 F2 改名）、tablist（開啟的檔案清單）
 */
import { createRoot } from "react-dom/client";
import "../src/styles.css";
import type { Backend, TermStream } from "../src/api/backend";
import { App } from "../src/App";
import { EMPTY_SCAN, EMPTY_STATE, WorkbenchStore } from "../src/store/store";

const scene = new URLSearchParams(location.search).get("scene") || "workspace";
localStorage.clear();
const put = (k: string, v: unknown) => localStorage.setItem("workbench:" + k, JSON.stringify(v));

const NOTES = `# 夜間迭代筆記

## 待辦
- [x] UI/UX 檢討落地
- [ ] 重裝 App 實測
  - [ ] 分割編輯

## 程式碼
\`\`\`rust
fn main() {
    let s = "hello"; // 註解
    println!("{s} {}", 42);
}
\`\`\`

| 項目 | 狀態 |
| --- | --- |
| 助手 | 完成 |
`;
const RS = `//! 範例
use std::path::Path;

/// 讀一個檔
pub fn read(p: &Path) -> Result<String, std::io::Error> {
    let text = std::fs::read_to_string(p)?; /* 區塊
    註解 */
    Ok(format!("{} bytes: {}", text.len(), 'a'))
}
`;
const files: Record<string, string> = {
  "docs/notes.md": NOTES,
  "src/main.rs": RS,
  "README.md": "# repo-a\n",
};

const term = {
  sid: "t1",
  title: "claude · repo-a",
  alive: true,
  kind: "agent",
  origin: null,
  scope: "repo-a",
  scope_repos: ["repo-a"],
  started: "2026-10-15 01:00",
  task: "整理 UI",
  report: null,
  report_basis: "",
  // F-80：做完一輪、還沒看（亮綠點、牌組「1 個做完沒看」）
  agent_status: { status: "finished", text: "這一輪做完了", at: "01:30", event: "stop" },
  home_group: "產品開發",
  // F-71：Claude Code 取的標題＋最後回覆
  ai_title: "整理登入頁的 UI",
  last_reply: "改好了：按鈕對齊、錯誤訊息改成紅字。要我順便補測試嗎？",
};
const card = (key: string, kind: string, title: string) => ({
  key,
  kind,
  lane: "scope",
  scope_kind: "repo",
  scope_label: "repo-a",
  scope_repos: ["repo-a"],
  time: "01:00",
  date: "2026-10-15",
  title,
  lines: [],
  actions: [],
});
const state = {
  ...EMPTY_STATE,
  repos: ["repo-a", "repo-b"],
  groups: [{ name: "產品開發", members: ["repo-a", "repo-b"], goal: "把登入做完", goal_at: "" }],
  tiers: { "repo-a": "active", "repo-b": "active" },
  tags: { "repo-a": ["開發"], "repo-b": [] },
  agents: ["claude"],
  // F-83：額度餘光（狀態列右下）
  usage: {
    at: "2026-10-16 20:50",
    five_hour: { used: 62, resets: "23:10" },
    seven_day: { used: 31, resets: "10-20 09:00" },
    context: 41,
    model: "Opus",
    warn: false,
  },
  terms: [
    term,
    {
      ...term,
      sid: "t2",
      alive: false,
      ai_title: "快取改成分片",
      last_reply: null,
      agent_status: { status: "exited", text: "程序已結束", at: "00:40", event: "exit" },
      claude_session: "02908d1e-7843",
      resume_cwd: "/Users/me/repo-a",
    },
  ],
  cards: [
    card("loop:#1", "loop", "驗證 Windows 版"),
    card("e1", "event", "交接：夜間迭代完成"),
    card("e2", "event", "判斷：助手一律先確認"),
  ],
  scope: { group: "產品開發", repos: null, members: ["repo-a", "repo-b"] },
};
const repoState = (id: string) => ({
  id,
  repo_type: "mine",
  tier: "active",
  path: "/Users/me/" + id,
  exists: true,
  dirty: 2,
  dirty_days: 0,
  last_commit_days: 0.2,
  ahead: 1,
  behind: 0,
  branch: "main",
  last_subject: "feat：助手",
  worktrees: 1,
  commits_7d: 12,
  commits_30d: 40,
  recent_commits: [],
  agents: [],
  note: "",
});
const scan = {
  ...EMPTY_SCAN,
  group: "產品開發",
  states: [repoState("repo-a"), repoState("repo-b")],
  quiet: 0,
};
const plan = {
  ok: true,
  text: "把 repo-b 移出產品開發，repo-a 加標籤 規格，然後打開筆記",
  summary: "調整組員與標籤",
  source: "agent",
  steps: [
    {
      op: "set_members",
      args: { group: "產品開發", repos: ["repo-a"] },
      text: "牌組「產品開發」：移出 repo-b",
      why: "使用者要移出",
      danger: true,
    },
    {
      op: "tag",
      args: { id: "repo-a", add: ["規格"], remove: [] },
      text: "repo-a：加標籤「規格」",
      why: "",
      danger: false,
    },
    {
      op: "remove_group",
      args: {},
      text: "remove_group",
      why: "",
      danger: false,
      error: "助手不能做「remove_group」（不在白名單）",
    },
    { op: "open_tab", args: { tab: "notes" }, text: "打開「筆記」分頁", why: "", danger: false },
  ],
};

const stream: TermStream = {
  onData() {},
  onOpen() {},
  onClose() {},
  write() {},
  resize() {},
  close() {},
  connected: false,
};
// no-session：沒有會話、所有 repo 範圍
if (scene === "no-session") {
  (state as { terms: unknown[] }).terms = [];
  (state as { scope: unknown }).scope = { group: null, repos: null, members: ["repo-a", "repo-b"] };
}
const backend: Backend = {
  kind: "http",
  getState: async () => state as never,
  getScan: async () => scan as never,
  openTerminal: () => stream,
  action: (async (name: string, p: Record<string, unknown>) => {
    if (name === "repo_tree")
      return {
        ok: true,
        id: p.id,
        root: "/Users/me/" + p.id,
        dir: p.dir || "",
        more: 0,
        entries: p.dir
          ? Object.keys(files)
              .filter((f) => f.startsWith(p.dir + "/"))
              .map((f) => ({ name: f.split("/").pop(), path: f, abs: "/x/" + f, dir: false }))
          : [
              { name: "docs", path: "docs", abs: "/x/docs", dir: true },
              { name: "src", path: "src", abs: "/x/src", dir: true },
              { name: "README.md", path: "README.md", abs: "/x/README.md", dir: false },
            ],
      };
    if (name === "read_file") {
      const text = files[String(p.path)] ?? "";
      return {
        ok: true,
        id: p.id,
        path: p.path,
        abs: "/x/" + p.path,
        size: text.length,
        kind: "text",
        text,
        truncated: false,
        editable: true,
        version: "v1",
      };
    }
    if (name === "repo_changes")
      return {
        ok: true,
        id: p.id,
        changes: [
          { path: "docs/notes.md", status: "modified" },
          { path: "src/new.rs", status: "untracked" },
        ],
        more: 0,
      };
    if (name === "palette")
      return {
        ok: true,
        query: p.query,
        repos: [
          {
            repo: "repo-a",
            score: 100,
            why: ["名稱開頭"],
            role: "實作",
            description: "登入服務",
            groups: ["產品開發"],
          },
        ],
        groups: [],
        files: [
          { repo: "repo-a", path: "docs/notes.md", name: "notes.md", abs: "/x/docs/notes.md" },
        ],
      };
    if (name === "assist_plan") return plan;
    if (name === "file_diff")
      return {
        ok: true,
        id: p.id,
        path: p.path,
        truncated: false,
        diff: [
          "diff --git a/src/main.rs b/src/main.rs",
          "--- a/src/main.rs",
          "+++ b/src/main.rs",
          "@@ -1,4 +1,5 @@",
          " //! 範例",
          "-fn main() {}",
          "+fn main() {",
          '+    println!("hi");',
          "+}",
          " ",
        ].join("\n"),
      };
    if (name === "past_sessions")
      return {
        ok: true,
        sessions: [
          {
            id: "8d110180-5810",
            repo: "repo-a",
            cwd: "/Users/me/repo-a",
            title: "整理登入頁的 UI",
            prompt: "情境卡＋文件地圖已備好…",
            reply: "改好了：按鈕對齊、錯誤訊息改成紅字。要我順便補測試嗎？",
            branch: "main",
            started: "2026-10-16T01:00:00Z",
            updated: "2026-10-16 01:30",
            live: true,
            path: "/x.jsonl",
          },
          {
            id: "02908d1e-7843",
            repo: "repo-b",
            cwd: "/Users/me/repo-b",
            title: "快取改成分片",
            prompt: "幫我看一下快取",
            reply: "已經 commit：快取拆成 16 片，查詢 450ms → 120ms。",
            branch: "cache",
            started: "2026-10-15T20:00:00Z",
            updated: "2026-10-15 22:37",
            live: false,
            path: "/y.jsonl",
          },
        ],
      };
    if (name === "backlinks")
      return {
        ok: true,
        id: p.id,
        path: p.path,
        items: [
          { path: "README.md", line: 12, text: "夜裡做了什麼見 [筆記](docs/notes.md)" },
          { path: "docs/plan.md", line: 4, text: "- [ ] 照 [筆記](notes.md#待辦) 重裝實測" },
        ],
        more: 0,
      };
    if (name === "assist_recent")
      return { ok: true, texts: ["切到 產品開發", "repo-a 加標籤 規格"] };
    if (name === "repo_profile")
      return { ok: true, id: p.id, path: "/x", role: "實作", description: "登入服務" };
    if (name === "group_notes") return { ok: true, group: "產品開發", path: "/x", notes: [] };
    return { ok: true };
  }) as never,
};

put(
  "scope",
  scene === "no-session" ? { group: null, repos: null } : { group: "產品開發", repos: null },
);
if (scene === "workspace" || scene === "panel-info") {
  put("panelRepo", "repo-a");
  put("panelTab", scene === "workspace" ? "changes" : "info");
}
if (scene === "tree-new" || scene === "rename") {
  put("panelRepo", "repo-a");
  put("panelTab", "files");
}
if (scene === "editor" || scene === "edit-find" || scene === "tablist") {
  put("openFiles", [
    { repo: "repo-a", path: "docs/notes.md", edit: true },
    { repo: "repo-b", path: "docs/notes.md" },
  ]);
  put("activeFile", "repo-a\u0000docs/notes.md");
  put("editLayout", "split");
}
if (scene === "diff-notes") {
  put("openFiles", [{ repo: "repo-a", path: "src/main.rs", diff: true }]);
  put("activeFile", "repo-a\u0000src/main.rs");
}
if (scene === "md-view") {
  put("openFiles", [{ repo: "repo-a", path: "docs/notes.md" }]);
  put("activeFile", "repo-a\u0000docs/notes.md");
}
if (scene === "code") {
  put("openFiles", [{ repo: "repo-a", path: "src/main.rs" }]);
  put("activeFile", "repo-a\u0000src/main.rs");
}

const store = new WorkbenchStore(
  backend,
  scene === "no-session" ? { group: null, repos: null } : { group: "產品開發", repos: null },
);
createRoot(document.getElementById("root")!).render(<App store={store} />);

// 需要互動的場景：等畫面好了再送鍵盤事件（只送給這一頁）
const later = (ms: number, f: () => void) => setTimeout(f, ms);
const typeInto = (el: HTMLInputElement, v: string) => {
  const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  set.call(el, v);
  el.dispatchEvent(new Event("input", { bubbles: true }));
};
if (scene === "assist" || scene === "assist-empty") {
  later(600, () => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "k", metaKey: true, bubbles: true }));
    later(200, () => {
      const box = document.querySelector<HTMLInputElement>("#palette input")!;
      typeInto(box, scene === "assist" ? "?" + plan.text : "?");
      if (scene === "assist")
        later(200, () =>
          box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })),
        );
    });
  });
}
if (scene === "code") {
  later(800, () => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "f", metaKey: true, bubbles: true }));
    later(200, () => typeInto(document.querySelector<HTMLInputElement>(".findbar input")!, "text"));
  });
}

const click = (sel: string) => document.querySelector<HTMLElement>(sel)?.click();
if (scene === "tree-new")
  later(800, () => click('#repopanel button[aria-label="新增檔案到 repo-a"]'));
if (scene === "term-menu")
  later(600, () => {
    click(".gsessions button");
    later(1200, () => click("#termmore"));
  });
if (scene === "edit-find")
  later(900, () => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "f", metaKey: true, bubbles: true }));
    later(200, () =>
      typeInto(document.querySelector<HTMLInputElement>(".editfind input")!, "待辦"),
    );
  });
if (scene === "palette")
  later(600, () => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "k", metaKey: true, bubbles: true }));
    later(200, () => typeInto(document.querySelector<HTMLInputElement>("#palette input")!, "repo"));
  });
if (scene === "composer") later(700, () => click("#ideatoggle"));
if (scene === "rename")
  later(900, () => {
    const row = document.querySelector<HTMLElement>(
      '#repopanel [role="treeitem"][data-path="README.md"]',
    );
    row?.dispatchEvent(new KeyboardEvent("keydown", { key: "F2", bubbles: true }));
  });
if (scene === "tablist") later(900, () => click('button[aria-label="開啟的檔案清單"]'));
if (scene === "past-sessions") later(700, () => click("#pastbtn"));
if (scene === "diff-notes")
  later(900, () => {
    document.querySelectorAll<HTMLElement>(".dlnote")[2]?.click();
    later(200, () => {
      const ta = document.querySelector<HTMLTextAreaElement>(".dlcomment textarea")!;
      const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
      set.call(ta, "改成回傳 Result，錯誤往上丟");
      ta.dispatchEvent(new Event("input", { bubbles: true }));
      later(100, () => click(".dlcommentacts button"));
      later(300, () => document.querySelectorAll<HTMLElement>(".dlnote")[4]?.click());
    });
  });
if (scene === "ended-session") later(700, () => click(`button[aria-label="展開產品開發"]`));
