# 桌面 App IPC 合約（React ⇄ Tauri v2 殼）

> 前端實作：`src/api/tauri.ts`（`createTauriBackend`）；型別：`src/api/types.ts`。
> 對照組：Python 版 HTTP API（`repo_context/webui/`），前端 `src/api/http.ts` 打的就是它。
> **兩邊回傳的 JSON 必須一致**——Rust 端照 `webui/state.py`、`webui/actions.py` 的欄位序列化（snake_case，不改名）。
> 改這份合約要同時改 `tauri.ts`、`types.ts` 與 `src/test/api.test.ts`。

前端在執行時偵測 `window.__TAURI_INTERNALS__`：有就走這份 IPC，沒有就走 HTTP（`/api/*`、`/ws/term/*`）。

## 通則

- 參數命名：JS 端 camelCase（`attachId`、`onEvent`），Rust 端 snake_case（`attach_id`、`on_event`）——Tauri v2 `#[tauri::command]` 預設轉換。
- 回傳值是 `serde_json::Value`（或等價 struct），欄位名與 Python 版完全相同。
- 錯誤：`Err(String)`，字串＝Python 版回的 `{"error": "..."}` 那句話（給人看的中文，**逐字照抄**）。前端把它當 `Error.message` 顯示在卡片／對話框／橫幅上。
- 範圍驗證：任何參數裡 `repos` 是**空陣列**一律拒絕：`"至少選擇一個 repo；空範圍不代表全部"`（`get_scan` 與 `run_action` 都要）。
- 不輪詢原則（AGENTS.md「閒置不吃 CPU」）：引擎寫入脊椎／registry 後發 `workbench://changed` 事件，前端收到就重抓 `get_state`。前端仍保留 5 秒輪詢當保險（視窗隱藏時停）。

## 命令

| 命令 | 參數（JS） | 回傳 | 對應 Python |
|---|---|---|---|
| `get_state` | — | `WorkbenchState` | `GET /api/state`：`build_state()` ＋ `terms`（`annotate_terms(TermManager.list())`）＋ `agents`（config `agents`） |
| `get_scan` | `{ group: string \| null, repos: string[] \| null }` | `ScanResult` | `GET /api/scan?group=…\|repos=a,b`：`build_scan()`。`repos` 優先於 `group`；兩者皆 null＝全部 |
| `run_action` | `{ name: ActionName, payload: object }` | 該 action 的結果物件 | `POST /api/<name>`：`handle_action()` |
| `term_open` | `{ sid: string, onEvent: Channel<TermChannelMessage> }` | `{ attach_id: string }` | `GET /ws/term/<sid>`（WebSocket 升級） |
| `term_write` | `{ sid: string, data: string }` | `null` | WS 訊息 `{"t":"i","d":…}` |
| `term_resize` | `{ sid: string, cols: number, rows: number }` | `null` | WS 訊息 `{"t":"r","c":…,"r":…}` |
| `term_close` | `{ sid: string, attachId: string }` | `null` | 關掉 WebSocket |
| `get_spine` | — | `{ spine: string \| null }` | （無；HTTP 版 server 啟動時就綁好 spine） |
| `set_spine` | `{ path: string }` | `{ spine: string }`（解析後的絕對路徑） | （無） |

### spine 定位（桌面殼才有）

- 啟動時殼用 `ctx_core::config::find_spine`：`--spine <path>` ＞ `REPOENGINE_SPINE` ＞ 使用者設定檔
  `~/.config/repo-context/config.json` 的 `spine`。
- 前端（`src/main.tsx`）先呼叫 `get_spine`：`spine` 是 null → 顯示首次設定（`SpineSetup`），
  使用者填路徑 → `set_spine`。殼驗證是 spine（有 `config.yaml` 或 `registry.yaml`）後寫回使用者設定檔
  （保留其他鍵）、啟用會話清冊與檔案監看。
- `set_spine` 錯誤：`"不是 spine 目錄（找不到 config.yaml 或 registry.yaml）：<path>"`；
  已有活著的會話時切換到別的 spine：`"還有會話在跑；先關掉所有會話再切換 spine"`。
- spine 未設定時 `get_state`／`get_scan`／`run_action` 回 `"尚未設定 spine：請先選擇 spine 目錄"`。

Rust 端簽名建議：

```rust
#[tauri::command] async fn get_state(app: State<'_, Workbench>) -> Result<serde_json::Value, String>;
#[tauri::command] async fn get_scan(app: State<'_, Workbench>, group: Option<String>, repos: Option<Vec<String>>) -> Result<serde_json::Value, String>;
#[tauri::command] async fn run_action(app: State<'_, Workbench>, name: String, payload: serde_json::Value) -> Result<serde_json::Value, String>;
#[tauri::command] fn term_open(app: State<'_, Workbench>, sid: String, on_event: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>) -> Result<TermOpenResult, String>; // TermOpenResult { attach_id: String }
#[tauri::command] fn term_write(app: State<'_, Workbench>, sid: String, data: String) -> Result<(), String>;
#[tauri::command] fn term_resize(app: State<'_, Workbench>, sid: String, cols: u16, rows: u16) -> Result<(), String>;
#[tauri::command] fn term_close(app: State<'_, Workbench>, sid: String, attach_id: String) -> Result<(), String>;
```

`get_scan` 會打 git（數秒），請用 async／`spawn_blocking`，不要卡住主執行緒。

## 動作（`run_action` 的 `name`）

與 `webui/actions.py` 的 `ACTIONS` 一一對應；payload／回傳形狀見 `src/api/types.ts` 的 `ActionMap`。

| name | payload | 成功回傳（皆含 `ok: true`） |
|---|---|---|
| `ack` | `{}` | — |
| `relate` | `{a, b, kind, note?}` | `relation` |
| `unrelate` | `{a, b, kind?}` | op 結果 |
| `collide` | `{idea, group?, repos?, wait?}` | `cid`（`wait` 時多 `judgement`） |
| `collide_rerun` | `{cid}` | `cid` |
| `ignore` | `{type, time, header?, cid?}` | — |
| `close_loop` | `{num}` | — |
| `loop_defer` | `{num, days?}` | `due` |
| `defer` | `{id, qkind?, days?, text?}` | `until`, `days` |
| `tier` | `{id, tier, resume_when?}` | `id`, `tier` |
| `remove_repo` | `{id}` | op 結果 |
| `route_collision` | `{cid, dest}` | `cid`, `dest`, `file` |
| `tag` | `{id, add?, remove?}` | op 結果 |
| `save_group` | `{name, repos}` | `name` |
| `update_group` | `{name, repos, new_name?}` | `name`, `members` |
| `remove_group` | `{name}` | op 結果 |
| `set_goal` | `{group, text}` | op 結果 |
| `session_draft` | `{sid}` | `text`, `traces`, `loops`, `note` |
| `scan_dir` | `{dir}` | `dir`, `candidates: [{path, id}]` |
| `register_repos` | `{repos: [{path, id}], group?}` | `added`, `group` |
| `brief` | `{group?, repos?}` | `text`, `file` |
| `term_create` | `{kind: "agent"\|"shell", agent?, task?, origin?, group?, repos?, repo?, cwd?}` | `sid`, `title` |
| `term_kill` | `{sid}` | — |
| `session_result` | `{sid, text}` | `date`, `time` |

未知 name → `Err("未知 action: <name>")`。

**Rust 殼目前進度**（`src-tauri/src/workbench.rs`）：只接上 `term_create`（`kind: "shell"`）與 `term_kill`；
`get_state`／`get_scan` 回 `Err("尚未實作")`，其餘 action 回 `Err("尚未實作：<name>")`，
`term_create` 的 `kind: "agent"` 回 `Err("尚未實作：agent 會話（等 session 模組搬到 Rust）")`。
第三波 `ctx_core::workbench::{state,actions}` 搬完後補上。`register_repos` 部分成功時 Python 回 `{error, added}`；IPC 只回 `Err(error)`（前端不用 `added`）。

## 終端串流（`term_open` 的 Channel）

`onEvent` 收到的訊息（`TermChannelMessage`）：

| 訊息 | Rust 送法 | 意義 |
|---|---|---|
| `ArrayBuffer` | `InvokeResponseBody::Raw(Vec<u8>)` | PTY 輸出原始位元組（**主要格式**；可能切在 UTF-8 中間，xterm 自己接） |
| `{ "t": "closed", "reason": "detached" \| "exited" \| "error", "message"?: string }` | `InvokeResponseBody::Json(...)` | 這條連線結束（程序結束、被 `term_kill`、或出錯）。前端顯示「連線已中斷」與〔重新連線〕 |

前端也接受兩種備用格式（Rust 端 Raw 不好送時再用）：`number[]`（`Vec<u8>` 走 JSON）、`{ "t": "data", "b64": "<base64>" }`。

順序保證（照 Python `server._handle_ws`）：

1. `term_open` 先把 **replay**（程序至今的輸出緩衝）送進 channel，再接 live 輸出——同一條 channel、順序不亂。
2. `term_open` 回傳 `attach_id` 後前端才會送 `term_write`／`term_resize`；回傳後前端立刻送一次 `term_resize`（目前 xterm 尺寸）。
3. 同一個 `sid` 可以有多個 attach（重新連線、多視窗）；`term_close` 只拆掉那一個 attach。
4. **`term_close` 不殺程序**（會話生命週期獨立於視窗）。殺程序走 `run_action("term_kill")`。關掉視窗＝所有 attach 失效，程序照跑。
5. 未知 `sid` → `Err("未知會話: <sid>")`。

## 事件

| 事件名 | payload | 何時發 |
|---|---|---|
| `workbench://changed` | `{ reason?: string }`（前端不讀內容） | 引擎寫入脊椎事件、registry、組、會話清冊有變（含 agent 從 MCP 寫入、會話程序結束）時。前端收到就呼叫 `get_state`（不會觸發 `get_scan`）。 |

殼的實作（`src-tauri/src/watch.rs`）：用 `notify` crate 監看 `<spine>/registry.yaml`、`<spine>/config.yaml`、
`<spine>/spine/events/*.md`（OS 檔案事件，不輪詢），250ms 內的一波寫入合併成一次；誰寫的都算（工作台、CLI、MCP）。
另外 `run_action` 成功（`reason` = action 名）、`set_spine`（`spine`）、會話程序結束（`term-exit`）也會發。

## 型別摘要（細節見 `src/api/types.ts`）

- `WorkbenchState`：`groups[{name, members, goal, goal_at}]`、`repos[]`、`tiers{}`、`tags{}`、`tag_vocab[]`、`relations[{from,to,kind,note?,exports?}]`、`rel_kinds{kind:{forward,inverse,symmetric}}`、`unread[]`、`loops[]`、`cards[]`、`recent[]`（事件＋`scope_kind/scope_label/scope_repos`，依事件時間新→舊，最多 200）、`stats{collisions, collisions_with_outcome, hit_rate, spawned, alive, survival_rate}`、`terms[]`、`agents[]`。
- 卡片 `cards[]`：共同欄位 `key, kind, title, lines, actions[{label, action, payload}], date, time, scope_kind, scope_label, scope_repos`；`collision` 多 `cid, verdict, judgement{}`；`interrupt/event` 多 `etype, source`；`pending` 多 `cid`；`loop` 多 `num, due`。
- `terms[]`：`sid, title, alive, kind, origin, scope, scope_repos, started, task, report{text,type,source,at}|null, report_basis`，agent 會話另有 `agent_status{status,text,at,…}|null`。
- `ScanResult`：`group, states[RepoState], summary(WorkSummary), pulse, recent_commits[{repo,date,hash,subject}], question_cards[], questions[], quiet, audit[]`。
- `RepoState`（`collect.collect_repo`）：`id, type, tier, path, exists, dirty, dirty_days, last_commit_days, ahead, behind, branch, last_subject, worktrees, commits_7d, commits_30d, recent_commits[], agents[], note`。
- `WorkSummary`（`summary.build_summary`）：`group, saved_group, repos, at, goal, goal_gap, progress, progress_gap, since, changes[], changes_window, next, next_gap, changes_gap, gaps[], resume_task`；每個 fact 是 `{text, source, at}`。

## 搬遷時要一起移進引擎的前端文案

目前前端（照舊版 `workbench.js`）還自己組兩段任務文字：`src/lib/view.ts` 的 `relationTask()`（「用這條關係開工作」）與 `taskEntries()` 的「整理本組現況」。Rust 版建議由 `get_state` 的 `relations[]` 帶 `task` 欄位、`get_scan` 的 `summary` 帶 `status_task`，前端改讀欄位後刪掉這兩個函式。
