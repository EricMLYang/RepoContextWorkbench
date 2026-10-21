# R37｜2026-10-21｜逆向 better-agent-terminal：它怎麼接 coding agent，以及 20 項值得做的功能

> 聚焦：使用者 2026-10-21 23:27「仔細逆向 better-agent-terminal 利用 Coding Agent SDK 的方式；這樣算有用 Claude Code／Codex 的 harness 嗎？依它的設計理念列 20 項本 repo 適合、值得開發的功能，一個一個實現（含編輯、瀏覽程式或資訊），做到早上 6 點」｜上一輪：`rounds/20261021_R36_夜間打磨_介面體驗與agent契合度.md`｜skill：pm-ideation
> 這輪是**自動迭代**：不開子 agent，逐項做、各自 commit、定時 push。
> 讀的版本：`reference_repo/better-agent-terminal` @ `9b468b4`（2026-10-12）。

## A. 逆向：BAT 怎麼接 coding agent

BAT 是 Tauri（Rust 殼）＋React＋一個小的 Node sidecar。Agent 不是自己寫的迴圈，而是**四條接法**，全部轉成同一套 `claude:*` 事件（`claude:message／tool-use／stream／status／result／turn-end`），畫面不分來源（`plans/claude-channel-agent-plan.md` 目標 2「One unified message contract」）。

| # | 接法 | 程式位置 | 怎麼做 | 計費（BAT 自己的判斷） |
|---|---|---|---|---|
| 1 | **Claude Agent SDK**（預設） | `node-sidecar/src/handlers/claude-send.mjs`、`lib/live-query.mjs` | `sdk.query({prompt: AsyncIterable, options})`，一場會話一個長駐 query（streaming-input），後續回合不用再冷啟動 `claude`（3～4 秒）；`options`：`systemPrompt/tools: {preset:'claude_code'}`、`settingSources: ['user','project','local']`、`includePartialMessages`、`thinking: adaptive`、`canUseTool`（權限改由 BAT 畫面回答）、`pathToClaudeCodeExecutable`、`resume=<sdkSessionId>`、`plugins`、`stderr` 收尾巴解釋錯誤 | API 計量 |
| 2 | **Codex app-server** | `src-tauri/src/codex_app_server.rs`（1 萬行 Rust） | 起 `codex app-server`，JSONL 上跑 JSON-RPC（`thread/start`、`thread/resume`、`turn/start`、`item/*` 通知），翻成同一套 `claude:*` 事件 | ChatGPT 登入或 API key |
| 3 | **互動式 CLI＋逐字稿**（實驗，BAT_DEBUG） | `runtimes/claude-cli-runtime.mjs`、`claude-cli-transcript.mjs`、`claude-cli-frames.mjs` | 在 PTY 跑**互動式** `claude`（畫面就是它原本的 TUI），用 `--settings` 掛 HTTP hooks 收即時訊號（工具開始、回合結束、權限轉送），內容則**尾隨** `~/.claude/projects/**/<session>.jsonl` 逐字稿，分類成 you／message／tool／thinking／usage | 訂閱（Pro/Max） |
| 4 | **Channel**（實驗） | `runtimes/claude-channel-*.mjs` | 互動式 `claude` 掛一個 BAT 寫的 MCP channel server，訊息從 channel 推進去，觀測同樣走 hooks；權限走官方 `claude/channel/permission` 協定 | 訂閱 |

### 這樣算有用 Claude Code／Codex 的 harness 嗎？——**算，而且是完整的 harness。**
- **Claude Agent SDK 本身就是 Claude Code 的 harness 打包成函式庫**：`query()` 會用 `pathToClaudeCodeExecutable` 起一個 `claude` 子程序，用 stream-json 在 stdio 上對話。Agent 迴圈、內建工具（Read／Edit／Bash…）、context 壓縮、權限規則、hooks、CLAUDE.md、skills、MCP、plugins 都是 Claude Code 在跑；`settingSources` 三層都開，連使用者的 `~/.claude/settings.json` 都吃。BAT 換掉的只有**畫面**（不用 TUI，用自己的 React 面板），以及**權限提問**（`canUseTool` 回呼，改成 BAT 的對話框）。
- **Codex app-server** 是 OpenAI 官方給 IDE 擴充（VS Code Codex）用的同一個 harness 介面：thread／turn／item 都是 Codex 自己的迴圈，BAT 只做轉譯。
- 所以 BAT 沒有自己的 agent 迴圈，它是「**多個官方 harness 的統一外殼**」。真正自己寫的是：事件正規化、權限 UI、會話持久化／續接、用量與快取成本的觀測、worktree 隔離、遠端／手機。

### 為什麼還要做 3、4 兩條？
BAT 的計畫書寫明：依它讀到的方案政策（文件記載 2026-06-15 起），**互動式** Claude Code 用訂閱額度，**程式化**（`claude -p`、Agent SDK）走另外計量的額度；很多使用者要繼續用訂閱，所以要一條「互動式 CLI＋讀本機檔案＋hooks」的路，把 SDK 面板的體驗（工具列、thinking、用量、權限）補回來。（這是 BAT 文件的說法，計費細節以 Anthropic 官方為準。）

### 跟我們的關係
**我們已經走在 BAT 第 3 條路上**：App 在 PTY 開互動式 `claude`，`--settings` 掛 hooks（Stop／Notification／PermissionRequest／PreToolUse／PostToolUse／UserPromptSubmit＋statusLine），寫 `.state/session_status/`，`transcript.rs` 讀逐字稿頭尾取標題與最後回覆。差別是：BAT 把逐字稿**整份**分類成時間軸、把 hook 訊號做成控制（auto-continue、中斷卡、子 agent 樹），我們只拿了標題和最後一句。這輪的 agent 類功能大多是「**把已經在手上的 hooks／逐字稿用滿**」，不換執行方式、不碰計費路徑。

## B. BAT 的設計理念（萃取）
1. **用官方 harness，不自己寫迴圈**；自己只做外殼與觀測。
2. **一套事件合約**，多個執行來源接上去，畫面不分來源。
3. **主機持有狀態**（會話、auto-continue 計數、權限待答），視窗只是檢視；遠端、手機看到同一份。
4. **成本看得見**：context %、快取命中、快取 TTL 倒數、過期警告、5h／7d 額度。
5. **安全的平行**：worktree 隔離、權限模式分級（default／acceptEdits／plan／bypass）。
6. **長任務不中斷**：auto-continue、中斷回合卡（一鍵繼續）、fork、rest／wake。
7. **工作就在旁邊**：檔案瀏覽、git log／diff、GitHub PR／issue、snippet、Procfile worker、md／csv／html 預覽、輸出裡的路徑可點。

我們的準則（AGENTS.md）：**更快拿到對的脈絡、判斷有留痕、人和 agent 走同一個介面**。以下 20 項是 B 跟這三點的交集，只選不違反「引擎算、前端畫」「閒置不輪詢」「依賴從嚴」的做法。

## C. 20 項功能（ID 接 backlog.md）

| 順序 | ID | 功能 | 對應 BAT | 我們怎麼做（引擎／畫面） | 準則 | 大小 |
|---|---|---|---|---|---|---|
| 1 | F-94 | **會話逐字稿檢視**：你／回覆／工具／thinking 時間軸，可看到終端捲掉的內容 | CLI runtime 的 frames 分類 | `transcript::frames` 讀整份 JSONL（略過 harness 雜訊、壓縮摘要另標）；action `session_transcript`；終端列「逐字稿」開對話框 | 留痕、脈絡 | M |
| 2 | F-95 | **終端輸出裡的檔案路徑可點**（`src/x.rs:12` → 檢視器開檔並捲到那行） | PathLinker、clickable file paths | xterm link provider；路徑相對會話目錄解析；檢視器支援跳行 | 同介面 | M |
| 3 | F-96 | **開會話選項：權限模式、模型** | permission modes、/model | `term_create` 多 `permission_mode`／`model`，插 `--permission-mode`（Claude Code 2.1.306 的選項：acceptEdits／auto／bypassPermissions／manual／dontAsk／plan）／`--model`（只認白名單值） | 同介面 | S |
| 4 | F-97 | **會話分岔（fork）**：從過去或執行中的會話岔出一場新的，原會話不動 | Session fork | `claude --resume <id> --fork-session`；會話選單與「過去的會話」都有 | 脈絡 | S |
| 5 | F-98 | **自動繼續**：這一輪做完就自動再送一句，最多 N 次；你自己送出就歸零 | /auto-continue | 狀態放在主機（殼）：設定存 `.state/auto_continue/<sid>.json`；殼看到 Stop（finished）就寫入 PTY；會話列顯示「自動 2/5」 | 同介面 | M |
| 6 | F-99 | **這一輪出錯中斷 → 一鍵繼續**（API 錯誤、額度） | InterruptedTurnCard、StopFailure | 多掛 `StopFailure` hook → 狀態 `failed`＋原因；會話列「繼續」送一句 | 留痕 | S |
| 7 | F-100 | **快取過期提示**：閒置超過 1 小時的會話標「快取已過期，下一句會整份重算」 | Cache TTL countdown／expiry warning | 狀態檔時間＋現在時間由引擎算 `cache_cold`；會話列小字 | 脈絡 | S |
| 8 | F-101 | **每場會話的 context 用量 %** | Context % statusline | statusLine 本來就收到 `context_window`；改成也寫進該會話的狀態檔；會話列「情境 62%」，≥80% 變色 | 脈絡 | S |
| 9 | F-102 | **子 agent 在跑幾個** | Subagent tracking | 多掛 `SubagentStart`／`SubagentStop` hook 計數；會話列「子 agent 2」 | 留痕 | S |
| 10 | F-103 | **把一場會話整理成交接包，交給新會話**（去敏、只留你說的、它的回覆、改過的檔） | agent-context-transfer（portable transcript＋redaction） | `transcript::digest`＋`redact`；action `session_digest`；「帶這場的脈絡開新會話」 | 脈絡、留痕 | M |
| 11 | F-104 | **你說過的話（prompt 歷史）**：逐字稿裡你送出的每一句，可再貼回會話 | Per-terminal prompt history | 逐字稿檢視的「只看我說的」篩選＋「貼回」 | 同介面 | S |
| 12 | F-105 | **可用的 skills／指令清單**：這個 repo 的會話裡能打哪些 `/指令` | Skills & Agents panels | 引擎掃 `~/.claude/skills`、`<repo>/.claude/skills`、`commands`（只讀 front matter）；終端選單「插入 /指令」 | 脈絡 | S |
| 13 | F-106 | **常用 prompt 庫**：存在 spine 的 `prompts/*.md`（人可讀、有版本），⌘K 插入目前會話 | Snippet manager | 引擎 `prompts` 讀寫；⌘K「/」前綴列出 | 留痕、同介面 | M |
| 14 | F-107 | **worktree 管理**：列出每個 repo 的 worktree、乾淨與否、獨有 commit，一鍵移除 | WorktreeMergedChip、worktree handler | `git worktree list --porcelain`＋status；右側欄 repo 詳情多一段；移除走 `git worktree remove`（不加 --force） | 脈絡 | M |
| 15 | F-108 | **在隔離的 worktree 開 agent 會話** | Claude Code (worktree) preset | `term_create` 多 `worktree: true` → `claude --worktree <name>`（Claude Code 內建） | 同介面 | S |
| 16 | F-109 | **最近的 commit 與單一 commit 的差異** | Git viewer（commit log＋diff） | action `repo_log`／`commit_diff`；右側欄「變更」上方加「最近 commit」 | 脈絡、留痕 | M |
| 17 | F-110 | **CSV／TSV 表格預覽** | CsvPreview | 檢視器依副檔名用表格畫（前 500 列、引號欄位） | 同介面 | S |
| 18 | F-111 | **終端內搜尋（⌘F）** | Ctrl+F in previews | xterm search addon（官方）；終端列搜尋框 | 同介面 | S |
| 19 | F-112 | **GitHub PR／issue 清單**（gh） | GitHubPanel | action `repo_github`（`gh pr list`／`gh issue list --json`，沒裝 gh 就說明）；右側欄 repo 詳情 | 脈絡 | M |
| 20 | F-113 | **圖片貼進 agent 會話**（截圖 → 存檔 → 貼路徑） | Image attachment | 終端拖放／貼上圖片 → 現有 `save_image` 存到 spine materials → 貼 `@路徑` | 同介面 | S |

### 刻意不做
- **自己用 Agent SDK 跑會話**：會把 App 會話從互動式變成程式化（BAT 文件說計費不同），也會讓人和 agent 走兩套介面；我們維持 PTY＋hooks。
- **Codex app-server 轉譯**：1 萬行等級；先等有人真的要在工作台跑 Codex。
- **遠端／手機**：已有 F-43 構想，這輪不碰。
- **Mermaid**：前端多一個大依賴，違反依賴從嚴；先不做。

## D. 進度（每做完一項更新）
| ID | 狀態 | commit |
|---|---|---|
| F-94＋F-104 | 完成：`timeline.rs`（真逐字稿 57MB 只讀尾 8MB、48ms）、action `session_transcript`、終端 ⋯「逐字稿」與過去的會話「逐字稿」、「我說的」可貼回 | f51e940 |
| F-96＋F-97＋F-108 | 完成：`session::LaunchFlags`（`PermissionMode` 列舉、名稱白名單、一律等號形式免得吞掉開場 prompt）；開會話表單「權限／模型／隔離 worktree」；終端 ⋯ 與過去的會話「分岔」（`--fork-session`） | 956ba27 |
| F-95 | 完成：前端 `lib/termLinks.ts` 找候選（網址、and/or 不算；全形字換算格子）、xterm link provider；引擎 `locate_path`（相對路徑先照 hook 記的 cwd、再照會話目錄；只開登記 repo 裡的檔）；檢視器 `line` 捲到那行（md 先切原始碼） | 2bc0a74 |
| F-99＋F-100＋F-101＋F-102 | 完成：多掛 `StopFailure`（→ 等你「這一輪出錯中斷：原因」＋會話列「繼續」貼進輸入框）、`SubagentStart／Stop`（`<sid>.sub/<agent_id>` 標記檔，免讀改寫競爭；2 小時沒收尾不算）；statusLine 另寫 `<sid>.meter.json`（context %、模型，變了才寫）；停下超過 60 分＝`cache_cold` | e2417aa |
| F-98 | 完成：引擎 `auto_continue.rs`（`.state/auto_continue/<sid>.json`；`take` 只認順利做完的 Stop、每次 Stop 帶 `stop_id` 只送一次、到上限停；你自己送別的話 `UserPromptSubmit` 歸零）；殼的監看回呼呼叫 `tick` 寫進 PTY（字與 Enter 分兩次）；終端 ⋯「自動繼續…」、會話列「自動 1/3」 | （本筆） |

## 來源
- BAT 原始碼：`node-sidecar/src/handlers/claude-send.mjs`（buildQueryOptions L944–1010）、`lib/live-query.mjs`、`handlers/claude-permission.mjs`、`src-tauri/src/codex_app_server.rs`、`runtimes/claude-cli-*.mjs`、`plans/claude-cli-transcript-agent-plan.md`、`plans/claude-channel-agent-plan.md`、`lib/transcript-transfer.mjs`、`lib/harness-noise.mjs`、README（Auto-Continue、Cache Cost Awareness、Statusline）。
