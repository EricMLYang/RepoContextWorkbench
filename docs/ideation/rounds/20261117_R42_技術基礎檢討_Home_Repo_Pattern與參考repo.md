# R42｜2026-11-17｜技術基礎檢討：對照 Home Repo Pattern 與 Orca、Better Agent Terminal、Herdr

> 起因：使用者貼了 Matt Pocock 的〈The Home Repo Pattern〉摘要，要求連同參考 repo 再做一版檢討。
> 這一版不比定位（那是 R41），問的是：**這是我自己每天用的工具，看完這些之後，技術基礎和設計哪裡該升級，
> 升級後用起來會差在哪**。只檢討、不實作；新構想進 `backlog.md`，狀態都是候選。

## 結論（先講）

目前最拖累日常使用的不是缺功能，是四個底層設計。每一個在參考 repo 裡都有現成做法：

| # | 現在的底層 | 你實際遇到的事 | 參考誰 | 升級後 |
|---|---|---|---|---|
| 1 | 脈絡只靠 hook 注入，spine 根目錄沒有 AGENTS.md | 11-14 在 spine 開 Codex，它不知道有工作台，自己 `find .git` | Home Repo Pattern | 任何 agent 在 spine 開場就讀得到「有哪些 repo、在哪、先讀什麼」，不用先裝東西 |
| 2 | PTY 開在 App 行程裡 | 每次重裝 App 都要等你 ⌘Q，會話跟著斷 | herdr-core 的 holder、Orca 的 daemon | 重裝、當掉、關 App 都不斷會話 |
| 3 | 會話狀態只靠各家 hook | Codex 要掛 7 個 hook 還要手動信任；其他 agent 沒有狀態 | Orca 的雙通道、Herdr 的 OSC 7501 | Codex 與其他 agent 零設定就有「處理中／等你／做完」 |
| 4 | 搜尋索引是一份 111 MB 的 JSON，有檔案變動就整份重寫 | `ctx search` 0.6–1.7 秒；索引從 54 MB 長到 111 MB | Orca 的 SQLite FTS5 增量索引 | 搜尋與開場更快，還能順便搜過去的會話內容 |

另外兩項是讓之後每次改動變便宜的基礎工程（第 5、6 節）：介面合約由一張方法表生成、共享 skill 集中到 spine。

建議順序：**1 → 3 → 2 → 4**。前兩項各一到兩天、彼此獨立、不動資料合約；第 2 項要先做一天的試作再決定路線；
第 4 項等 Python 版刪掉之後做最省事。

## 查證範圍

| 對象 | 這次讀了什麼 | 標記 |
|---|---|---|
| Home Repo Pattern | 使用者貼的摘要（沒有讀原文） | 🔗 |
| herdr-core `c84d6e7` | `AGENTS.md`、`schema/README.md`、`holder.rs`／`status.rs`／`notes.rs` 的檔頭與型別 | ✅ |
| Orca `67dc1ad` | `src/main` 的模組清單與行數、`daemon/AGENTS.md`、`agent-hooks/README.md`、`ai-vault-search` 的 schema | ✅ |
| Better Agent Terminal `2ed1cc6` | 目錄結構、`CLAUDE.md` 大綱、README | ✅ |
| 本 repo | `DEVELOPMENT.md` §1–§4、`baseline.md`、`knowledge.rs` 檔頭、`term.rs`、真 spine 的 `.state/` | ✅ |

沒做的：沒有實際安裝任何一家；參考 repo 的程式只讀了設計文件與檔頭，沒有逐檔讀實作；
OSC 7501 的支援情況來自 Orca 的實測表（2026-10-17），本機只確認 Claude Code 2.1.326 的執行檔裡有 7501 字串，Codex 沒有驗到。

## 1. spine 就是 Home Repo，但少了靜態入口

Home Repo Pattern 的做法：開一個 repo 當所有 agent 的啟動點，裡面放共享 skill、根目錄 AGENTS.md（每個 repo 一行：是什麼、路徑、
沒 clone 就從哪 clone、動它之前先讀什麼）、共享文件（規範、詞彙、ADR）。超過約 20 個 repo 就按領域拆成索引檔，AGENTS.md 只指向索引。

對照我們：

| | Home Repo Pattern | 本 repo 現況 |
|---|---|---|
| 中央 repo | 手建的 home repo | spine（`repo-spine`），已經存在 |
| repo 清單 | 手寫在 AGENTS.md | `registry.yaml`，23 個 repo、6 個組，有名片、類別、關係 |
| agent 怎麼知道 | 開場自動讀 AGENTS.md | SessionStart hook 注入；沒掛 hook 的什麼都看不到 |
| 漸進揭露 | 按領域拆索引檔 | 開場注入只給本組；細節靠 MCP 工具 |
| 共享 skill | 放 home repo、用 git 版控 | 只有 `repo-context` 一個由 `ctx setup` 發；其餘散在各 repo（card_notes 15 個、language-learning 6、draftagent 5、character-play-studio 3），`~/.codex/skills` 是空的 |
| 共享文件 | 規範、詞彙表、ADR | 判斷在 spine，可用 `ctx decisions` 匯出 AgDR；沒有詞彙表與共用規範 |
| 時間軸 | 沒有 | spine 事件：交接、判斷、未結 |
| 平行隔離 | worktree | 開會話時可選隔離的 worktree；`ctx worktrees` 看現況與清理 |

我們多的是動態那一半（上次交接、下一步、未結、留痕），少的是靜態那一半。靜態入口的好處是**零依賴**：
任何會讀 AGENTS.md 的 agent 都拿得到，hook 沒裝、沒被信任、壞掉時仍然成立。11-14 的事就是只有動態、沒有靜態造成的。

**建議（F-132）**：由 registry 生成，不手寫。

- `spine/AGENTS.md`：三到五行介紹工作台是什麼、有 MCP 就用哪些工具、沒有就用 `ctx … --json`；接著每組一行，指向該組的索引檔。
  23 個 repo 已經超過文章說的 20 個門檻，所以直接用兩層。
- `spine/groups/<組>/REPOS.md`：每個 repo 一行，欄位照文章的格式（名稱、是什麼、本機路徑、remote、提示）。
  「是什麼」用名片，提示由類別推（參考類寫「只讀」、程式類寫「先讀它的 AGENTS.md」）。
- `spine/CLAUDE.md`：一行指向 AGENTS.md。
- registry 或名片有變更時重新生成；檔頭註明是生成檔。這些檔在 spine 裡，不違反「只讀使用者的 repo」。

做完之後，hook 注入可以縮短：靜態的同組成員清單改成一句「成員見 `groups/<組>/REPOS.md`」，開場注入的 3,000 字元預算留給交接與未結。

**建議（F-133）**：共享 skill 進 spine。`spine/skills/<名稱>/SKILL.md` 由 git 版控，`ctx setup` 發到 `~/.claude/skills` 與
`~/.codex/skills`（發送機制這一輪已經做好，現在只發 `repo-context` 一個）。哪些 skill 該從各 repo 搬出來要你決定，
照文章 Step 2 的分法先盤點成「共享／重複／repo 專屬」三類即可，repo 專屬的留在原地。

## 2. 會話要跑在 App 之外

現況：`ctx-core/term.rs` 用 portable-pty 開 PTY，行程掛在 Tauri 殼底下。AGENTS.md 寫「關掉視窗不殺掉正在跑的 agent」，
關視窗確實不殺，但 ⌘Q、換版、App 當掉都會殺。這是每次裝新版都卡住你的那一步。

兩家的做法：

- **herdr-core**：分成 holder 與 core 兩個行程。holder 擁有所有 PTY、是所有程式的父行程，只做開、讀、改大小、送訊號、收屍，
  不解析也沒有任何規則，每個終端機留約 1 MB 的輸出尾巴。core 當掉時 holder 把它重啟，core 再從尾巴重建畫面。
  因為 holder 幾乎沒有邏輯，它很少需要更新，更新 core 時終端機不受影響。整個 core 約 2.4 萬行 Rust。
- **Orca**：terminal daemon 約 4.1 萬行 TypeScript。它的 `daemon/AGENTS.md` 記了一段教訓：光是「誰可以動 socket 路徑」
  就經過七輪審查、23 個缺陷，最後定成一套先 link、驗證對方已死、再 rename 的協定。

這兩份都說明同一件事：**常駐的終端機行程自己寫很貴，貴在行程交接與殘留清理，不在 PTY 本身**。

兩條路（F-128）：

| | 用 herdr-core 當後端 | 自己做最小 holder |
|---|---|---|
| 做法 | App 改成 `hc` 的 client（JSON-RPC over Unix socket，attach 走分幀串流） | 新增一個只管 PTY 的小行程，照 holder 的切法；規則留在引擎 |
| 工作量 | 小：`Terms` trait 已經把會話清冊抽出來，換一個實作 | 中到大：交接、殘留、Windows 都要自己處理 |
| 風險 | 多一個要安裝的執行檔；herdr-core 是 0.1.x，API 標示 preview（但有「只增不改」的相容測試） | Orca 踩過的那類缺陷要自己踩一遍 |
| 附帶 | 多機（SSH）、agent 互叫、OSC 7501 狀態都現成 | 完全自己掌握，沒有外部依賴 |

建議先花一天用 herdr-core 試作：只接「開會話、接回、關 App 再開仍在」三件事，確認體驗與 Windows（VDI）裝不裝得起來。
`Terms` 留成可換的介面，試作不滿意再走第二條，屆時照 holder 的原則做（不解析、不放規則、留尾巴）。

## 3. 狀態改成雙通道：hook 優先，OSC 7501 補位

OSC 7501（Program Status Protocol）是 agent 自己寫進終端機輸出的一行狀態：`idle／working／done／blocked／error`，
blocked 還能帶原因（permission／question／auth）、一行訊息、子 agent 紀錄。

Orca 的 `agent-hooks/README.md` 有一張實測表（2026-10-17，每一列都是在 PTY 裡跑真的 CLI）：

| agent | 版本 | 行為 |
|---|---|---|
| Claude Code | 2.1.295 起支援 | 終端機要先回應 `?` 探詢才會送；五種狀態、三種原因、子 agent 都有 |
| Codex | 0.163 | 不用探詢直接送；只有狀態，沒有原因與訊息 |
| Grok、pi、oh-my-pi | — | 都有 |

本機是 Claude Code 2.1.326、Codex 0.169，都在支援範圍內。

Orca 的規則值得直接照做：**hook 有講話就用 hook；hook 沒裝、被關掉、或在某個狀態靜默超過 2 分鐘，就用 OSC 7501**。
hook 帶的東西比較多（prompt、工具名稱、session id），所以仍是主通道；7501 的價值是不用任何設定。

對我們的意義（F-130，上一輪留的問題「Claude Code、Codex 有沒有發」在這裡有答案）：

- Codex 會話在 App 裡開出來就有狀態，不必先跑 `ctx setup --codex` 也不必在 `/hooks` 逐項信任。
- `AgentKind::Other` 的 agent 只要有發就有狀態。
- 實作位置在引擎：`term.rs` 的 `reader_loop` 已經讀過所有輸出，在那裡認 OSC 7501、回應探詢，寫進現有的 `session_status`。
  要新增的是「來源」這個型別（hook 或 program），以及誰優先的規則。

要先驗的：Codex 0.169 在我們的 PTY 裡是不是真的會送（Orca 測的是 0.163）。

## 4. 索引與搜尋換成增量的

現況（真 spine，2026-11-17）：`.state/knowledge_index.json` 111 MB。它是 BM25 索引，依檔案 mtime 快取，
有檔案變動時整份重寫。歷次量測：

| 時間 | 索引大小 | `ctx search` |
|---|---|---|
| 2026-10-09（R01） | 54 MB | 2.2–3.0 秒 |
| 2026-10-21（R36） | 65 MB | 0.6 秒（暖）、1.09 秒（冷） |
| 2026-11-17 | 111 MB | 這次沒量 |

從 65 MB 到 111 MB 的這段期間加了三個 herdr repo，orca 也多了 7,450 個 commit。每次查詢要把整份 JSON 讀進來解析，
索引越大越慢，而且跟查的東西多寡無關。

Orca 的 `ai-vault-search`：SQLite＋FTS5，表分成 sessions／files／messages，另一個行程做增量索引，查詢有期限。
它索引的是各家 agent 的會話逐字稿（`ai-vault` 那 2.3 萬行是各家的掃描器），所以 Orca 能全文搜尋「以前跟 agent 講過什麼」。

**建議（N-51）**：索引改用 SQLite FTS5。

- 只重建有變動的檔，不整份重寫；查詢只讀命中的頁。
- 中文沿用現在的字元 bigram（寫入時自己切，或用 FTS5 的 trigram tokenizer，要實測哪個準）。
- 依賴 `rusqlite`（bundled），符合「主流 crate」；二進位會變大，要量。
- `.state/` 不進 git，是可重建的快取，不是資產。但 `knowledge.rs` 註明格式跟 Python 共用，所以這項排在 Python 版刪除之後。

**建議（F-134）**：同一個庫加一張會話表，Claude Code 與 Codex 各一個掃描器。這同時補掉上一輪沒做的
「Codex 會話的逐字稿／digest／backfill」：現在是 Claude 的逐字稿直接讀檔、Codex 另寫一套讀 rollout，
改成兩家都先掃進同一份索引，上層功能（過去的會話、digest、backfill、搜尋）只面對索引。

## 5. 介面合約由一張方法表生成

現況：同一個能力要在四個地方各寫一次——CLI（clap）、MCP 工具的 schema、Tauri IPC 與前端的 `types.ts`、Python 對拍。
同步靠 `scripts/parity.sh` 和手寫的 `desktop/IPC.md`。這一輪替會話加一個 `agent` 欄位，就動了引擎、CLI、MCP、relay、
前端型別、五個前端元件和對拍的允許差異表。

herdr-core 的做法（`AGENTS.md` 與 `schema/README.md`）：

- 一張方法表。參數與回傳的 Rust 型別同時就是 CLI 參數與對外 schema；CLI 每個指令就是一次 API 呼叫。
- schema（OpenRPC）由型別生成，過期時測試會失敗。
- `released.openrpc.json` 是上次發版的 API。任何會讓舊 client 壞掉的變更（刪方法、刪欄位、新增必填、回傳多一種值）測試都會擋下。
  規則是「API 只增不改」，要破壞就明確升一代。

BAT 也守「IPC 只加不改」（R33 讀原始碼時記的），但靠人守。

**建議（N-52）**：`ctx-core` 的請求與回應改成具名型別（serde），由它們生成 MCP 的 `inputSchema`、`--json` 輸出的 schema、
前端的 `types.ts`；加一份「上次發版的 schema」快照測試。Python 版刪掉之後，對拍正好需要一個接手守合約的機制，這個就是。
這項不會直接改變你用起來的感覺，它讓之後每個功能少改幾個地方、兩家 agent 看到的介面不會各自漂移。

## 6. 其他值得帶走的設計

- **「回報觀察到的，不回報假設的」**（herdr-core 的規則）。我們的工具回傳偶爾會寫預期結果；交接、狀態、setup 的輸出可以照這條檢查一遍。
- **原始檔不超過約 32 KB**（herdr-core）：理由是 agent 的讀檔工具在 50 KB 附近會截斷。我們是「約 800 行」，目前
  `agentapi/mod.rs`、`cli/agent.rs`、`filetree.rs`、`assist.rs`、`backfill.rs` 超過。用位元組當門檻比行數貼近 agent 實際的限制。
- **兩層 skill**（herdr-core、Orca 都這樣）：裝進 agent 的是很短的 stub，只說何時用、去哪拿完整說明（`hc --skill`）；
  完整說明跟著執行檔走，不會過期。我們的 SKILL.md 145 行整份裝進去，所以才需要「skill 過時」檢查。改成 stub＋`ctx instructions` 可以拿掉那個檢查。
- **父子會話與等待**（herdr-core 的 `notes.rs`）：子的消息只在父「接得下」而且「還沒看過」時才貼進去；
  使用者正在打字時等最多 60 秒。這是對講送達可以抄的細節（F-131，上一輪已記）。
- **設定目錄自帶 AGENTS.md**（Herdr App）：告訴 agent 哪些檔可以改、格式是什麼。第 1 節的 `spine/AGENTS.md`
  可以順便說明 `registry.yaml`、`config.yaml` 的欄位，讓 agent 能直接幫你維護 spine。

不建議帶走的：Herdr 的 mod 系統（你本來就用 agent 直接改這個 repo，不需要另一層擴充機制）；
Orca 的原生 Chat、雲端、手機、plugin；BAT 的 SDK 面板與計費顯示。理由同 R41。

## 技術差異總表

| | 本 repo | herdr-core／App | Orca | Better Agent Terminal |
|---|---|---|---|---|
| 語言與 UI | Rust 引擎＋Tauri＋React | Rust＋GPUI，mod 用 QuickJS | Electron＋React（TypeScript） | Tauri＋React＋Node sidecar |
| 終端機在哪個行程 | App 行程 | 獨立的 holder，core 可重啟 | 獨立 daemon（4.1 萬行） | App 行程；SDK 會話可持久化 |
| 終端機模擬 | xterm.js（前端） | ghostty-vt（引擎內，重連時能重畫畫面） | xterm.js | xterm.js |
| 狀態來源 | 各家 hook | 只收 OSC 7501 | hook 優先、OSC 7501 補位 | SDK 事件 |
| 持久資料 | Markdown 事件＋YAML，在 git 裡 | 設定檔＋core 的狀態 | 大量 app 狀態，`persistence` 5.5 萬行 | SQLite（snippet）＋設定 |
| 索引 | 單一 JSON，整份重寫 | 無 | SQLite FTS5，增量 | Codex 會話本地快取 |
| 對外 API | CLI＋MCP，各自手寫 | 一張方法表 → CLI＋OpenRPC，有相容測試 | Orca CLI | 無（IPC 內部用） |
| 給 agent 的說明 | 整份 SKILL.md 裝進去 | 短 stub＋`hc --skill` | stub＋版本相符的指南 | — |
| 跨 repo 清單 | registry＋組（23 repo） | 無 | 無（有巢狀 repo 探索） | workspace 群組 |

本 repo 在「持久資料」這一列的選擇（純文字、在 git 裡、人能改）是三家都沒有的，也是不該動的；
要升級的是它周邊的東西：會話行程、狀態通道、索引、合約。

## 新增與更新的構想

| ID | 構想 | 準則 | P |
|---|---|---|---|
| F-132 | spine 當 Home Repo：由 registry 生成 `AGENTS.md`、每組的 `REPOS.md`、`CLAUDE.md`；開場注入的成員清單改指向它 | 脈絡、同介面 | P1 |
| F-133 | 共享 skill 進 `spine/skills/`，`ctx setup` 發到兩家；先盤點共享／重複／repo 專屬 | 同介面 | P2 |
| F-134 | 會話索引：Claude Code 與 Codex 各一個掃描器掃進同一份索引，過去的會話、digest、backfill、全文搜尋共用 | 脈絡、留痕 | P2 |
| N-51 | 知識索引改 SQLite FTS5，增量更新（Python 版刪除後做） | 脈絡 | P1 |
| N-52 | 介面合約由一張方法表生成 schema 與前端型別，加發版快照的相容測試（接手 Python 對拍） | 同介面 | P2 |
| F-128（更新） | 補上兩條路的比較與「先試作 herdr-core 一天」的建議 | 同介面 | P1 |
| F-130（更新） | 補上 Orca 的實測表與「hook 優先、7501 補位」規則；實作位置在 `term.rs` 的 `reader_loop` | 同介面 | P1（原 P2） |

## 要你決定的

1. F-132 生成的 `AGENTS.md` 要不要進 spine 的 git（建議要：換機器 clone 下來就有）。
2. F-128 願不願意多依賴一個外部執行檔（herdr-core）。不願意的話直接走自己做 holder 那條，不必試作。
3. F-133 哪些 skill 算共享。這個我可以先盤點出三類的表給你勾。
