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
- 不輪詢原則（AGENTS.md「閒置不吃 CPU」）：引擎寫入脊椎／registry、agent 回報狀態、會話結束時發 `workbench://changed` 事件，前端收到就重抓 `get_state`；有這個事件就**不輪詢 state**，只在視窗回到前景時補抓一次（2026-10-13）。沒有事件的 HTTP 後端才每 5 秒輪詢。`get_scan`（打 git）照舊在視窗可見時每 2 分鐘一次。

## 命令

| 命令 | 參數（JS） | 回傳 | 對應 Python |
|---|---|---|---|
| `get_state` | `{ group: string \| null, repos: string[] \| null }`（畫面目前的範圍；同 `get_scan`，但空陣列不拒絕＝空範圍） | `WorkbenchState` | `GET /api/state?group=…\|repos=a,b`：`build_state(group, repos)` ＋ `terms`（`annotate_terms(TermManager.list())`）＋ `agents`（config `agents`） |
| `get_scan` | `{ group: string \| null, repos: string[] \| null }` | `ScanResult` | `GET /api/scan?group=…\|repos=a,b`：`build_scan()`。`repos` 優先於 `group`；兩者皆 null＝全部 |
| `run_action` | `{ name: ActionName, payload: object }` | 該 action 的結果物件 | `POST /api/<name>`：`handle_action()` |
| `term_open` | `{ sid: string, onEvent: Channel<TermChannelMessage> }` | `{ attach_id: string }` | `GET /ws/term/<sid>`（WebSocket 升級） |
| `term_write` | `{ sid: string, data: string }` | `null` | WS 訊息 `{"t":"i","d":…}` |
| `term_resize` | `{ sid: string, cols: number, rows: number }` | `null` | WS 訊息 `{"t":"r","c":…,"r":…}` |
| `term_close` | `{ sid: string, attachId: string }` | `null` | 關掉 WebSocket |
| `get_spine` | — | `{ spine: string \| null }` | （無；HTTP 版 server 啟動時就綁好 spine） |
| `set_spine` | `{ path: string }` | `{ spine: string }`（解析後的絕對路徑） | （無） |
| `create_spine` | `{ path: string }` | `{ spine: string }`（同 `set_spine`） | （無；在新路徑或空資料夾跑 `ctx init` 同一份 scaffold，已是 spine 就直接用） |
| `pick_folder` | — | `string \| null`（取消＝null） | （無；Rust 端用 `tauri-plugin-dialog` 開原生「選擇資料夾」，前端不開 dialog 權限） |
| `watch_tree` | `{ repos: { [id]: string[] } }`（每個 repo 展開中的資料夾，`""`＝根；空物件＝全部停） | `null` | （無；查不到的 repo 略過。變動用事件 `repo://changed` 通知） |

### spine 定位（桌面殼才有）

- 啟動時殼用 `ctx_core::config::find_spine`：`--spine <path>` ＞ `REPOENGINE_SPINE` ＞ 使用者設定檔
  `~/.config/repo-context/config.json` 的 `spine`。
- 前端（`src/main.tsx`）先呼叫 `get_spine`：`spine` 是 null → 顯示首次設定（`SpineSetup`），
  使用者填路徑 → 既有的用 `set_spine`、要新建的用 `create_spine`。殼驗證是 spine（有 `config.yaml` 或 `registry.yaml`）後寫回使用者設定檔
  （保留其他鍵）、啟用會話清冊與檔案監看。
- `set_spine` 錯誤：`"不是 spine 目錄（找不到 config.yaml 或 registry.yaml）：<path>"`；
  已有活著的會話時切換到別的 spine：`"還有會話在跑；先關掉所有會話再切換 spine"`。
- `create_spine` 錯誤：資料夾已有東西又不是 spine →
  `"這個資料夾已經有東西、又不是 spine；為了不動到你的檔案，請換一個新路徑或空資料夾：<path>"`（不往既有資料夾塞檔）。
- spine 未設定時 `get_state`／`get_scan`／`run_action` 回 `"尚未設定 spine：請先選擇 spine 目錄"`。

Rust 端簽名建議：

```rust
#[tauri::command] async fn get_state(app: State<'_, Workbench>, group: Option<String>, repos: Option<Vec<String>>) -> Result<serde_json::Value, String>;
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
| `term_create` | `{kind: "agent"\|"shell", agent?, task?, origin?, group?, repos?, repo?, cwd?, home_group?}`（`home_group` Rust 才有：所屬牌組，只給畫面歸類——前端只送目前選的組當提示，對單一 repo 開時由引擎判斷（提示的組有這個 repo 就用它，否則只有一個組有它才算）；沒給用 `group`；`state.terms[]` 有它才多一個 `home_group` 鍵） | `sid`, `title` |
| `term_kill` | `{sid}` | — |
| `session_seen` | `{sid}`（Rust 才有，F-80：你看過這個會話了——做完沒看→看過了；引擎在 `.state/session_status/<sid>.json` 記 `seen: true`，下一個 hook 事件就消失） | `changed`（沒狀態檔或已記過＝false） |
| `session_result` | `{sid, text}` | `date`, `time` |
| `repo_tree`（Rust 才有） | `{id, dir?}`（`dir` 相對 repo 根目錄，空＝根） | `root`, `dir`, `entries: [{name, path, abs, dir, ignored}]`（`ignored`＝被 .gitignore 忽略，照列、App 畫灰色；`.git` 不列）, `more` |
| `group_add_folder`（Rust 才有） | `{path, group?}` | `id`, `registered`, `group` |
| `reorder`（Rust 才有） | `{kind: "groups"\|"members"\|"repos", order, group?}`（只換順序，不是同一批就拒絕） | — |
| `read_file`（Rust 才有） | `{id, path}` | `path`, `abs`, `size`, `kind: "text"\|"image"\|"binary"\|"too_large"`（text 帶 `text`、`truncated`、`editable`（沒截斷且是合法 UTF-8）、`version`（內容版本，存檔時帶回）；image 帶 `mime`、`base64`） |
| `write_file`（Rust 才有） | `{id, path, text, base}`（`base`＝讀檔時的 `version`） | 寫成功：同 `read_file` 的結果（新 `version`）；讀檔之後磁碟上已被改過：`{ok: false, conflict: true, id, path}`（沒有寫）。只改既有的 UTF-8 文字檔（≤1MB、不新增不刪除），先寫暫存檔再改名，保留原檔 CRLF／BOM；不記 spine 事件 |
| `create_entry`（Rust 才有） | `{id, path, kind: "file"\|"dir"}` | `path`（相對 repo 根目錄）, `abs`, `dir`。新增空檔或資料夾：已存在就拒絕（不覆蓋），上層沒有就一路建（最近一個已存在的上層要在 repo 裡），不准碰 `.git`；不記 spine 事件 |
| `ack_scope`（Rust 才有） | `{group?, repos?}`（目前範圍；全部範圍會被拒，那是 `ack`） | `at`。只把這個範圍的一般未讀（event 卡）標為已讀，記在 `.state/seen_scopes.json`；全域 `last_seen.txt` 不動，別組照舊；打斷、待跟進、回顧不受影響 |
| `save_image`（Rust 才有） | `{id, path, base64}` | `path`, `abs`。md 編輯區貼上的圖片存成新檔：只收 png／jpg／gif／webp／bmp（svg 可帶腳本不收）、≤ 8MB、不覆蓋、路徑規則同 `create_entry`；不記 spine 事件。App 存到那份 md 旁邊的 `assets/<主檔名>-<本機時間到毫秒>.<副檔名>` |
| `backlinks`（Rust 才有，唯讀） | `{id, path}` | `items: [{path, line, text}]`, `more`。repo 裡哪些 md 連到這個檔：照相對連結解析（`/` 開頭＝repo 根、去掉 #／?）、跳過程式碼區塊與自己、最多看 3000 份 md、列 50 則 |
| `rename_entry`（Rust 才有） | `{id, from, to}` | `from`, `path`（新的相對路徑）, `abs`, `dir`。改名或搬移：`from` 要存在（符號連結改連結本身），`to` 同 `create_entry` 的規則（不覆蓋、上層一路建、不碰 .git、不跳出 repo）；不刪任何東西、不記 spine 事件 |
| `assist_plan`（Rust 才有，唯讀） | `{text, group?, repos?}`（目前範圍） | `text`, `summary`, `source: "rules"\|"agent"`, `steps: [{op, args, text, why, danger, error?}]`。一句話 → 白名單內的步驟（見 `docs/APP_ASSISTANT.md`）；常見說法本機規則直接翻，其他問 config `provider.assist`（沒設＝claude、model 預設 haiku）。**不執行**：App 照步驟呼叫原本的 action |
| `assist_recent`（Rust 才有，唯讀） | `{}` | `texts`：最近用過、真的做了事的助手指令（新的在前、同一句只留一次、最多 5 句；復原不算） |
| `assist_log`（Rust 才有） | `{text, group?, done: [人話], skipped: [人話], failed?}` | 落 presented [assist] 事件（指令、做了哪些、略過哪些、在哪步失敗） |
| `repo_profile`（Rust 才有，唯讀） | `{id}` | 名片：`id`, `path`, `role?`, `description?`, `description_source?: "registry"\|"readme"`（沒填介紹時從 README 截的推測）, `keywords?`, `tags?` |
| `describe_repo`（Rust 才有） | `{id, description, role, keywords}`（表單三欄的完整內容；`keywords` 是一串字，逗號、頓號、全形逗號或換行分隔） | 同 `repo_profile` ＋ `changed`。只寫跟 registry 不同的欄位（走 `registry_describe`，同 CLI／MCP 的留痕）；都沒變＝不寫、`changed: false` |
| `palette`（Rust 才有，唯讀） | `{query, group?, repos?}`（query 空＝全部；group／repos＝找檔案的範圍，都沒有＝所有沒封存的 repo） | `query`, `repos: [{repo, score, why, role?, description?, groups}]`（名稱完全相同＞開頭＞包含＞詞首縮寫，加上名片欄位包含整串查詢、名片詞命中；最多 30 個）, `groups: [{name, members, goal}]`（名稱或目標包含查詢）, `files: [{repo, path, name, abs}]`（兩個字以上才找；git repo 用 `git ls-files`（含沒被忽略的未追蹤檔），否則往下走；檔名相同＞開頭＞包含＞路徑包含；最多 20 個）。App 的 ⌘K；排名在 `profile::quick_find`、`filetree::find_files` |
| `pick_brief`（Rust 才有，唯讀） | `{files: [{id, path}]}`（從檔案樹挑的，最多 50 個） | `text`（會話任務：依 repo 分段、段落標題帶名片）, `mentions`（`@絕對路徑` 一行一個）, `repos`, `count`。每個路徑都要在它的 repo 裡（同 `read_file` 的檢查），重複的只算一次 |
| `profile_draft_brief`（Rust 才有，唯讀） | `{ids?}`（沒給＝還沒寫介紹、沒封存的 repo，最多 10 個） | `text`（請 agent 起草名片的任務：只讀 repo、先列草稿給人確認、確認後才 `ctx registry describe`；附已用過的角色）, `repos`。都寫了＝錯誤「沒有要起草的 repo（都寫了介紹）」 |
| `repo_changes`（Rust 才有，唯讀） | `{id}` | `id`, `changes: [{path, status: "modified"\|"added"\|"deleted"\|"renamed"\|"untracked"\|"conflicted"}]`, `more`（超過 500 個沒列的數量）。`git --no-optional-locks status --porcelain=v1 -z --untracked-files=all`（不拿 index.lock、不回寫 index）；不是 git repo＝錯誤「不是 git repo：<path>」 |
| `scope_changes`（Rust 才有，唯讀） | `{group?, repos?}`（畫面範圍；都沒有＝所有沒封存的 repo） | `repos: [{id, changes, more, error?}]`——只回有變更的 repo（照範圍順序，各 repo 平行跑 `repo_changes` 同一套）；不是 git 的略過。真資料 18 個 repo 約 70ms |
| `review_brief`（Rust 才有，唯讀） | `{id}` | `text`（請 agent 審查還沒 commit 的變更：檔案清單＋先不改檔不 commit、摘要、風險、建議怎麼切 commit、用 log_decision 記結論）, `count`。沒有變更＝錯誤「<id> 沒有還沒 commit 的變更」 |
| `file_diff`（Rust 才有，唯讀） | `{id, path}`（相對 repo，只能往下走；檔案可以已刪除） | `id`, `path`, `diff`（`git diff HEAD`；還沒追蹤的跟空檔比；repo 還沒 commit 就對暫存區；沒有差異＝空字串）, `truncated`（超過 512 KB） |
| `group_notes`（Rust 才有） | `{group}`（已儲存的牌組） | `group`, `path`（`<spine>/groups/<組>/notes.md`）, `notes: [{at, text}]`（新的在前） |
| `note_add`（Rust 才有） | `{group, text}` | 同 `group_notes` |
| `note_remove`（Rust 才有） | `{group, at, text}`（兩個都要對得上，檔案被改過就拒絕） | 同 `group_notes` |

未知 name → `Err("未知 action: <name>")`。

**Rust 殼**（`src-tauri/src/workbench.rs`）：`get_state`／`get_scan`／`run_action` 全部接到 `ctx_core::workbench`
（`api_state`／`build_scan`／`handle_action`，會話清冊以 `Terms` trait 傳入 `TermManager`）。
`term_create` 的 agent 分支：`session::build` 組好 cwd＋指令 → `TermManager::create_agent`；
`session_draft`／`session_result` 從 `terms.get(sid).info()` 拿會話資料。
`collide`／`collide_rerun` 的背景判定：找得到 `ctx`（`ctx_core::exe::ctx_exe`：自己→同目錄→PATH）就起 detached 進程（同 Python，關 App 也跑得完），
沒有就在 App 行程裡開執行緒跑（App 結束前沒跑完 → lint 會報「碰撞無回程」，可按〔重跑判定〕）。
`register_repos` 部分成功時 Python 回 `{error, added}`；IPC 只回 `Err(error)`（前端不用 `added`）。
寫入型 action 成功後發 `workbench://changed`（`reason`＝action 名）；唯讀的（`session_draft`／`scan_dir`／`repo_tree`／`read_file`／`group_notes`／`repo_changes`／`scope_changes`／`file_diff`／`review_brief`／`repo_profile`／`palette`／`pick_brief`／`profile_draft_brief`，清單在引擎 `workbench::is_read_only`）不發——否則展開資料夾、開檔都會讓前端重抓整份 state。

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
| `ui://next-waiting` | `null` | 選單列「等你的會話：N（點了切過去）」（Rust 才有，2026-10-14；N=0 時不能點）：殼先叫出視窗，前端切到最該先處理的會話（卡住的優先，同類取最早開始的；`lib/view.firstNeedingYou`）。 |
| `ui://capture` | `null` | 選單列「記一筆…」（Rust 才有，2026-10-14）：殼先叫出視窗，前端切到工作區的筆記分頁、游標放進輸入框（記到目前牌組的 `notes.md`；沒選牌組時筆記分頁會說要先選）。 |
| `repo://changed` | `{ id: string, paths: string[], rescan: boolean }`（相對 repo 根目錄） | `watch_tree` 登記的資料夾裡有檔案新增／刪除／改名／改內容（合併 300ms）。父資料夾沒登記的、`.git/` 裡的不報。前端重讀受影響的那一層、開著的檔案重讀（有未存修改的檔不重讀，改成提示）。`rescan: true`＝OS 漏了事件（溢出或監看錯誤）：`paths` 是這個 repo 所有登記中的資料夾，前端全部重讀。實作：`src-tauri/src/tree_watch.rs`（遞迴 OS 檔案事件，沒有樹開著就不監看任何 repo）。 |

殼的實作（`src-tauri/src/watch.rs`）：用 `notify` crate 監看 `<spine>/registry.yaml`、`<spine>/config.yaml`、
`<spine>/spine/events/*.md`、`<spine>/.state/last_seen.txt`（未讀；CLI `ack` 也會改）、
`<spine>/.state/agent_status/*.json`（agent 經 MCP 回報的階段，會話清冊顯示）、`<spine>/.state/session_status/*.json`（App 開的 Claude Code 會話由 hook 自動寫的「做完等你／處理中」，見下）——OS 檔案事件，不輪詢，
250ms 內的一波寫入合併成一次；誰寫的都算（工作台、CLI、MCP）。每次發事件時殼也重算選單列的未讀數
（`notify.pending` 的 count；`get_state` 回來時也以 `unread.length` 更新），視窗藏起來時選單列照樣是新的。
選單列也顯示在等你的會話數（活著、階段 waiting／blocked；標題如「3 · 等你 2」，選單「等你的會話：N」）：
事件來時由引擎 `workbench::sessions_needing_you` 算（只讀階段檔），`get_state` 回來時從 `terms[].agent_status` 數。
另外 `run_action` 成功（`reason` = action 名）、`set_spine`（`spine`）、會話程序結束（`term-exit`）也會發。

## 型別摘要（細節見 `src/api/types.ts`）

- `WorkbenchState`：`groups[{name, members, goal, goal_at}]`、`repos[]`、`tiers{}`、`tags{}`、`tag_vocab[]`、`relations[{from,to,kind,note?,exports?,in_scope,task}]`、`rel_kinds{kind:{forward,inverse,symmetric?}}`、`unread[]`、`loops[]`、`cards[]`、`recent[]`（事件＋`scope_kind/scope_label/scope_repos`＋`lane`，依事件時間新→舊，最多 200）、`stats{collisions, collisions_with_outcome, hit_rate, spawned, alive, survival_rate}`、`scope{group, repos, members, gone}`（這份 state 是照哪個範圍算的；`gone`＝範圍已不存在，Rust 才有）、`terms[]`、`agents[]`。
- 卡片 `cards[]`：共同欄位 `key, kind, title, lines, actions[{label, action, payload, primary?}]（`primary: true`＝主按鈕，一張卡最多一個，Rust 才有）, date, time, scope_kind, scope_label, scope_repos, lane`；`collision` 多 `cid, verdict, judgement{}`；`interrupt/event` 多 `etype, source`；`pending` 多 `cid`；`loop` 多 `num, due`。
- `lane`（卡片與 `recent[]` 事件）：`scope`＝屬於目前範圍、`global`＝全域與未分類（卡片另含：範圍外的打斷）、`other`＝不顯示。前端照它分欄，不自己判斷範圍。
- `attention{needs_you, finished_unseen, working}`（Rust 才有，F-80／F-86）：活著的 agent 會話裡「在等你」（waiting／blocked）與「做完沒看」（finished 且沒 `seen`、沒 `faded`）各幾個——殼的選單列與 Dock 徽章直接讀這個，不自己比狀態字串（分類只在引擎 `workbench::Attention`）。
- `usage{at, five_hour{used,resets}|null, seven_day{…}|null, context|null, model|null, warn}|（沒收過就沒有這個鍵）`（Rust 才有，F-83）：Claude Code 額度餘光，從 App 會話的 statusLine 來（`ctx hook statusline` → `.state/usage.json`）。
- `terms[]`：`sid, title, alive, kind, origin, scope, scope_repos, started, task, report{text,type,source,at}|null, report_basis`，agent 會話另有 `agent_status{status,text,at,…}|null`、`ai_title`、`last_reply`、`claude_session`、`resume_cwd`（F-71／F-78）。
  Rust 才有（2026-10-14）：`agent_status` 也可能來自 Claude Code hook——`term_create` 開 Claude Code（config `mcp: mcp-config`）時多帶
  `--settings=<spine>/.state/session_hooks.json`（只掛 `Stop` → `ctx hook stop`、`UserPromptSubmit` → `prompt`、`Notification` 依 matcher：`permission_prompt`／`elicitation_dialog`／`elicitation_url_dialog`／`agent_needs_input` → `permission`、`idle_prompt` → `notification`，
  不動使用者的 `~/.claude/settings.json`）與環境變數 `REPO_CONTEXT_SESSION_STATUS=<spine>/.state/session_status/<sid>.json`；
  （2026-10-16 起另掛 `PermissionRequest` → `permission-request`、`PreToolUse`／`PostToolUse` → `tool-start`／`tool-end`、`statusLine` → `statusline`；事件名只在引擎 `session_status::StatusHook` 定義一次。）
  `terms[]` 另有 `tool{label, minutes, stalled}|null`（F-82：處理中正在用的工具、跑了幾分、超過 10 分＝可能卡住；引擎算好）。
  hook 照這個路徑寫 `{status, text, at, event}`（做完一輪＝**finished**（F-80，不算等你；App 看過後多 `seen: true`）、要你允許工具＝blocked＋「Bash：cargo test」這種一句（F-81）、閒著等輸入＝waiting＋訊息、你送出＝working；程序結束＝exited）。讀端另依 `at` 衰減（N-41，不改檔）：做完沒看超過 24 小時加 `faded: true`（當成看過）；處理中、沒工具在跑、30 分鐘以上沒事件加 `quiet_minutes`（畫面說「沒動靜 N 分」）。
  階段字串只在引擎 `status::AgentStatus` 定義（MCP `report_status` 仍只收 working／waiting／blocked／done）。
  `blocked` 的（agent 停住了）殼會在視窗不在前面時發系統通知「<會話> 要你允許」，同一次請求只發一次（`workbench::fresh_permission_asks`）。這種的多一個 `event` 鍵。
  跟 agent 經 MCP 回報的取新的（同一分鐘取 MCP 的）；會話結束刪檔、App 重開清空（sid 從 t1 重編）。
- `ScanResult`：`group, states[RepoState], summary(WorkSummary), pulse, recent_commits[{repo,date,hash,subject}], question_cards[]（含 provenance：scope_kind="repo"）, questions[], quiet, session_tasks[{label,text}], audit[]`。
- `RepoState`（`collect.collect_repo`）：`id, type, tier, path, exists, dirty, dirty_days, last_commit_days, ahead, behind, branch, last_subject, worktrees, commits_7d, commits_30d, recent_commits[], agents[], note`。
- `WorkSummary`（`summary.build_summary`）：`group, saved_group, repos, at, goal, goal_gap, progress, progress_gap, since, changes[], changes_window, next, next_gap, changes_gap, gaps[], resume_task`；每個 fact 是 `{text, source, at}`。

## 已移進引擎的前端業務邏輯（2026-10-06）

AGENTS.md「前端不放業務邏輯」：下列原本在 `src/lib/view.ts` 的判斷改由引擎（`ctx_core::workbench::state`，
Python `webui/state.py` 同步加了同樣欄位，兩邊 JSON 仍一致，`scripts/parity_state.py` 對拍）給，前端只讀欄位：

| 原本前端 | 現在 |
|---|---|
| `relationTask()`（用這條關係開工作的任務文案） | `get_state` 的 `relations[].task` |
| `taskEntries()`（接續上次工作／整理本組現況／組內第一條關係） | `get_scan` 的 `session_tasks[]`（沒照 IPC 舊建議放 `summary.status_task`：`summary` 是 `ctx group summary --json` 的合約，不動） |
| `matchesScope`／`isUnscoped`：卡片、事件依範圍篩選 | `get_state({group, repos})` 的 `cards[].lane`、`recent[].lane` |
| 關係依範圍篩選（from 或 to 在範圍內） | `relations[].in_scope` |
| 問句卡以 repo 補 provenance | `question_cards[]` 直接帶 `scope_kind/scope_label/scope_repos` |
| `primaryAction()`：哪個是主按鈕（2026-10-13） | `actions[].primary`（`state` 的 `mark_primary`） |
| 範圍已不存在（組被刪或改名）（2026-10-13） | `scope.gone` |
| 單一 repo 會話屬於哪個牌組（2026-10-13） | `term_create` 照 `home_group` 提示＋registry 判斷（`home_group_for`） |
| 哪些 action 之後要重新採集（2026-10-13） | action 結果帶 `rescan: true`（`tier`、`defer`、`remove_repo`） |

前端仍保留的只有顯示層：搜尋（`matchesSearch`，只比對看得到的文字）、類型篩選、分段標題與順序、範圍名稱（`scopeLabel`）、
牌組卡的 commit 加總（`groupMeta`）；組員用 `state.scope.members`，只有剛換範圍、state 還沒回來時才先用本地推算頂著。
