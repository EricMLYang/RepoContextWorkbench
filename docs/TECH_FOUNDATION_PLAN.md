# 技術基礎升級：開發規劃

> 2026-11-17 定稿。依據是 `ideation/rounds/` 的 R41（三個執行台比較）與 R42（技術基礎檢討）。
> 這份只寫「做什麼、照什麼順序、怎樣算做完」；為什麼做看 R42。
> 每做完一階段回來更新「進度」一欄；全部做完後，把結果收斂進 `VISION.md` §5 並刪掉這份。

## 1. 目標

把四個拖累日常使用的底層換掉，另外做兩項讓之後改動變便宜的基礎工程。資料格式（git 裡的純文字 spine 與 registry）不動。

| 做完之後 | 現在 |
|---|---|
| 任何 agent 在 spine 開場就知道有哪些 repo、在哪、先讀什麼 | 沒掛 hook 的 agent 什麼都看不到（2026-11-14 Codex 自己 `find .git`） |
| Codex 與其他 agent 在 App 裡零設定就有「處理中／等你／做完」 | Codex 要掛 7 個 hook 並手動信任 |
| 重裝、當掉、⌘Q 都不斷會話 | PTY 掛在 App 行程，⌘Q 就全斷 |
| 搜尋暖查詢 150 ms 內，索引只更新變動的檔 | 111 MB 的 JSON 整份重寫，0.6–1.7 秒 |

## 2. 總覽

| 階段 | 項目 | backlog | 估計 | 先決條件 | 進度 |
|---|---|---|---|---|---|
| 0 | 量基準、驗兩個假設 | F-129 | 半天 | 無 | 完成（2026-11-18）：兩家都實測會送 OSC 7501；search 暖 1.6–1.9s、索引 111 MB；別家開的會話還沒驗 |
| 1 | spine 靜態入口：生成 AGENTS.md 與每組的 REPOS.md | F-132 | 1–2 天 | 無 | 完成（2026-11-18）：`ctx home sync`；沒掛 hook／MCP 的 Codex 與 Claude Code 在 spine 副本都答出 23 個 repo；真 spine 還沒跑 |
| 2 | 狀態雙通道：hook 優先、OSC 7501 補位 | F-130 | 2 天 | 階段 0 驗過 Codex 會送 | 完成（2026-11-18）：真的 Claude Code 2.1.326 與 Codex 0.169 經引擎的 PTY 都記到 working → finished；App 畫面還沒測 |
| 3 | 會話跑在 App 之外 | F-128 | 試作 1 天＋實作 3–5 天 | 決定 2 | 完成（2026-11-18，路線 A）：`session.backend: herdr-core`（預設仍是 pty）；真 hc 0.1.21 上，真的 Claude Code 跑到一半、丟掉 manager 再重建，接回後跑完；App 畫面、全螢幕程式、Windows 沒驗 |
| 4a | 合併回 main、刪 Python 版與對拍 | VISION §6 第 2 項 | 1–2 天 | 使用者真用驗收 | 沒做：等使用者決定（4b、4c、5a 後來確認不必等它） |
| 4b | 知識索引改存 SQLite（排序不變、增量） | N-51 | 3 天 | 無（沒等 4a） | 完成（2026-11-18）：副本上暖查詢 760→280–400 ms、改一檔後 800→300 ms，20 個查詢結果與改前相同；代價是 db 172 MB（原 97 MB）、冷建 2→4.5 秒；真 spine 還沒量 |
| 4c | 會話索引（Claude Code＋Codex） | F-134 | 3 天 | 4b | 完成（2026-11-18）：兩家各一個掃描器；Codex 補上逐字稿、digest、backfill、分岔；`ctx sessions --grep` 與 ⌘K「&」。副本上索引 113 場（Claude 110、Codex 3）、建 4.1 秒、db 23 MB、查詢 16–24 ms；App 畫面沒測 |
| 5a | 介面合約由方法表生成 | N-52 | 3–5 天，可分批 | 無（沒等 4a） | 完成（2026-11-18）：MCP 參數由型別生成、單一方法表、tools/list 逐位元組不變；MCP／CLI 旗標／`--json` 形狀三份「上次發版」快照擋破壞性變更；前端只做契約測試，沒有生成 TS |
| 5b | 工作台 skill 進 spine | F-133 | 盤點半天＋實作 1 天 | 階段 1、決定 3 | 機制完成（2026-11-18，同日改版）：`spine/skills/<名稱>/` 只在納管 repo 的開場列出、`skill_get` 讀，不發成檔案；盤點了 102 個 skill，搬哪些等使用者決定 |

估計天數是我的推估，沒有實測依據；階段 3 的不確定性最大。

依賴關係：0 → 1、2 可以平行；3 獨立；4a 擋住 4b、4c、5a；5b 接在 1 後面。
階段 1 和 2 不動既有合約，可以在合併回 main 之前做。

## 3. 每一項都要守的

- 能力先做成 `ctx-core` 的函式，CLI／MCP／App 各接一層薄殼；引擎不依賴 Tauri。
- 型別表達狀態；錯誤用 `Result`。新依賴只在 4b 加一個（`rusqlite`）；5a 另加 `schemars`（使用者在 N-52 指定可加，成本見 `RUST_PORTING.md`）。
- 既有的 spine、registry、`.state/`、CLI、MCP 欄位只增不改；新增的寫進 `RUST_PORTING.md`。
- 4a 之前新增的東西標「Rust 才有」，在 `scripts/parity.py` 加允許差異。
- **Claude Code 與 Codex 都要接、都要測**；做不到的那一家在回報裡明說。
- 每個階段的完成定義：
  1. `bash scripts/check.sh` 通過。
  2. 要人點的項目補進 `TESTING.md`，註明 macOS 測過沒有、Windows 測過沒有。
  3. 量測結果寫進 `ideation/baseline.md` 新的一欄。
  4. 各自 commit；會影響 App 的重 build 並裝好，commit 訊息寫重開後要測什麼。

## 4. 階段 0：量基準、驗假設（半天）

不改程式。產出是 `baseline.md` 的一欄和兩個是／否的答案。

1. 量現況：`ctx search` 暖／冷、改一個檔後第一次查詢、`knowledge_index.json` 大小、SessionStart hook 耗時與注入字元數。
2. **驗 Codex 0.169 會不會送 OSC 7501**：在 PTY 裡跑 `codex`，把原始輸出存檔，找 `ESC ] 7501 ;`。
   同樣驗 Claude Code 2.1.326（它要終端機先回應 `?` 探詢才送，要照規格回）。這一步決定階段 2 能不能做。
3. **驗在別家開的會話有沒有 ctx 的脈絡**（F-129）：Herdr App 裝得起來的話，各開一場 Claude Code 與 Codex，
   看開場注入、MCP 工具、交接。Orca、BAT 等使用者決定要不要裝。

## 5. 階段 1：spine 靜態入口（F-132）

**做什麼**

- 新模組 `ctx-core/src/home.rs`：純函式，`RegistryData`＋名片 → 三種檔案的內容。
  - `AGENTS.md`：工作台是什麼（三到五行）；有 MCP 用哪些工具、沒有就用 `ctx … --json`；每組一行指向它的 `REPOS.md`；
    `registry.yaml` 與 `config.yaml` 哪些欄位可以改。
  - `groups/<組>/REPOS.md`：每個 repo 一行——名稱、名片的一句話、本機路徑、remote、提示。
    提示由類別推：參考類「只讀」、程式類「改之前先讀它的 AGENTS.md」。沒分組的 repo 另放一檔。
  - `CLAUDE.md`：一行指向 `AGENTS.md`。
- CLI：`ctx home sync` 寫檔；`ctx home sync --check` 只報告哪些檔過期（給 `ctx health` 用）。
- 觸發：`registry::store::save` 之後、名片變更之後、`ctx setup` 結束時。
- 生成檔第一行帶標記。檔案存在但沒有標記，視為使用者手寫的：不覆蓋，回報衝突。

**這一版不做**：不縮短 hook 注入的同組成員段。那會動到跟 Python 對拍的輸出，留到 4a 之後。

**驗收**

- 單元測試：固定的 registry → 固定的檔案內容；手寫檔不被覆蓋；組改名後舊檔清掉。
- 真 agent：spine 副本、隔離的設定目錄（沒有 hook、沒有 MCP、沒有 skill），
  `codex exec` 與 `claude -p` 各問一次「現在登記了幾個 repo」，都要答出 registry 的數量而且沒有掃檔案系統。
- `ctx health` 在生成檔過期時有一行提醒。

**風險**：AGENTS.md 太長反而佔脈絡。上限定 60 行，超過就把內容往每組的檔推。

## 6. 階段 2：狀態雙通道（F-130）

**做什麼**

- `ctx-core/src/term.rs`：`reader_loop` 讀到的位元組多經過一個 OSC 7501 解析器。
  解析器是獨立的純狀態機（序列會被切在兩次讀取之間），認 `ESC ] 7501 ; … BEL` 或 `ST` 結尾。
  輸出照常往前端送，不吃掉。
- 回應探詢：收到 `OSC 7501 ; ?` 時寫回規格要求的回應。回應格式照規格與 Orca 的 `src/shared/program-status/`，不自己猜。
- 新型別 `StatusSource { Hook, Program }`；`session_status` 的狀態檔多一個 `source` 欄位（只增）。
- 優先規則做成純函式：hook 有紀錄就用 hook；沒有 hook 紀錄，或 hook 停在某個狀態超過 2 分鐘而程式回報已離開那個狀態，用程式回報的。
  門檻放 `config.yaml` 的 `thresholds`。
- 7501 的 `blocked` 原因對應到現有的「要你允許」（permission）與「等你」（question、auth）。

**範圍**：只有 App 開的會話有（PTY 在我們手上）。在外部終端機開的會話仍然靠 hook。

**驗收**

- 解析器測試：完整序列、切成兩段、夾在一般輸出中間、格式錯的要忽略。
- 優先規則測試：四種組合（有無 hook × 有無程式回報）加上靜默逾時。
- 真 agent：沒跑過 `ctx setup --codex` 的環境，在 App 開 Codex 會話，狀態會從處理中變成做完。
  Claude Code 的會話行為與現在相同（hook 仍是主通道）。

**風險與退路**

- 回應探詢後 Claude Code 的輸出行為可能改變。加一個設定可以整個關掉 7501，預設開。
- Codex 只送狀態、不送原因，所以「要你允許」仍然要靠 hook 才分得出來。這一點寫進 `TESTING.md`，不當成缺陷。

## 7. 階段 3：會話跑在 App 之外（F-128）

**先試作一天**（決定 2 選「願意依賴 herdr-core」才做）

只接三件事：開一個 agent 會話、在畫面上接回它、⌘Q 再開 App 會話仍在而且可以輸入。試作不合併，只回答：

| 問題 | 過關條件 |
|---|---|
| 輸入延遲與畫面重畫 | 跟現在分不出差別 |
| 安裝 | `hc` 能隨 App 一起裝、版本固定，使用者不用另外裝 |
| 結束偵測 | 會話結束時我們收得到（沒交接的卡片靠這個） |
| Windows（VDI） | 至少裝得起來、開得了一個會話；測不到就明寫 |

四項都過就走 A，否則走 B。

**路線 A：herdr-core 當後端**

- 引擎加一個會話後端的介面（開、寫入、改大小、訂閱輸出、結束通知、列出現存的），現有的 portable-pty 實作是第一個實作，
  `hc` 的 client 是第二個。`Terms` 不變。
- client 走 JSON-RPC over Unix socket；attach 用分幀模式。
- App 啟動時列出現存的會話並重新接上；會話的中繼資料（哪個 repo、哪個 agent、帶了什麼料）存在 `.state/`，用終端機位址對回來。
- 階段 2 的 7501 解析在這條路上改成直接讀 core 的狀態事件，自己的解析器留給 portable-pty 後端。
- `rebuild.sh` 的「等 ⌘Q」那段仍然要等（換的是 App 本體），但文案改成「會話不受影響」。

**路線 B：自己做最小 holder**

- 新執行檔模式 `ctx term-host`：只擁有 PTY，開、讀、寫、改大小、送訊號、收屍，每個會話留 1 MB 尾巴，不解析、不放規則。
- App 與它之間用 Unix socket。socket 路徑的所有權照 Orca `daemon/AGENTS.md` 的協定做（先 link、證明對方已死、再 rename），不自己發明。
- 估計 5 天以上，Windows 另計。

**驗收（兩條路相同）**

- 開會話 → ⌘Q → 重開：會話在、畫面還原、可輸入、狀態正確。
- 開會話 → 強制殺掉 App → 重開：同上。
- 重裝新版（`rebuild.sh`）：跑到一半的 agent 沒有中斷。
- 會話結束且沒交接：收件匣照常出現卡片。
- App 閒置時 CPU 不因為多一個常駐行程而上升（量 10 分鐘）。

**風險**：這是六項裡唯一可能做到一半要換路的。所以先試作，而且後端介面先定好，兩個實作可以並存、用設定切換。

## 8. 階段 4：索引

### 4a 合併回 main、刪 Python（既有計畫）

這是 `VISION.md` §6 已經排的事，條件是使用者真用驗收通過。它擋住 4b、4c、5a，
因為這三項都會改到目前跟 Python 共用格式或對拍的東西。這一項的時機由使用者決定。

### 4b 知識索引改存 SQLite（N-51，2026-11-18）

原本寫「用 FTS5、等 Python 刪掉後做」；實際做法改成現在就做、而且**搜尋結果一個位元都不變**
（`ctx search` 跟 Python 對拍，排序、分數、片段、why 都是合約）：

- SQLite 只當「可增量更新、可局部讀取的倒排索引儲存」：`.state/knowledge.db` 的 `docs`（repo、絕對路徑、rel、mtime、
  secret、長度、該檔的詞清單）、`terms`、`body`／`head` posting（詞 → 檔、tf；內文與標題＋frontmatter＋檔名各一份）。
  格式版本放 `PRAGMA user_version`，不符整個重建。schema、並行、損毀處理見 `crates/ctx-core/src/knowledge/store.rs` 檔頭。
- **不用 FTS5**，也不用它的 bm25()：斷詞（英數小寫單字＋中文字元 bigram）、BM25 參數、標題加權、boosts、片段全是
  `knowledge` 原本的算法，浮點累加順序也照舊。查詢只撈查詢詞的 posting 來算分。
- 更新：比對 mtime，只重讀新增／變動的檔；刪掉的檔、registry 移除的 repo 一併清掉；沒變動不寫。
- 多個行程（CLI、多支 `ctx mcp`、App）：WAL、busy timeout 3 秒、寫入交易裡逐列重查；拿不到寫入鎖就不寫、這次查詢用記憶體裡剛算的。
  壞了刪掉重建，再不行退回在記憶體裡從頭建——搜尋不會失敗。
- Python 版不動、繼續用它自己的 `knowledge_index.json`；Rust 不讀不寫，第一次建好 db 後刪掉。對拍列為允許差異。
- 原本「在記憶體裡從頭建再算分」的路留著當測試的對照組（`tests/knowledge_store.rs`：20 個查詢、增量、改名、切分支、
  並行、版本不符、損毀，結果都要跟對照組逐位元組相同）。
- 依賴：`rusqlite`（bundled）。
- 量完發現索引之後的大頭是程式碼搜尋：`git grep -I` 要先把圖片讀進來才知道是二進位（card_notes 1 萬多張圖，
  佔程式碼搜尋的大半）。改成 png／jpg／jpeg／gif／webp／ico 不交給 git grep（正常的圖本來就不會命中；
  只有副檔名是圖、內容是純文字的檔會搜不到——登記的 repo 裡一個都沒有）。

**驗收**（真 spine 副本、release、`ctx search "會話 狀態 hook" --json`、cwd 本 repo；中位數，兩輪）

| 指標 | 原訂目標 | 改之前（同版 build） | 改之後 |
|---|---|---|---|
| `ctx search` 暖 | < 150 ms | 756–838 ms | 283–302 ms |
| `ctx search` 暖、`--no-code` | — | 684–703 ms | 205–214 ms |
| `ctx search` 冷（沒有索引） | < 500 ms | 1.9–2.6 s | 4.5–5.4 s ❌（建庫寫 800 萬列 posting） |
| 改一個 md 後第一次查詢 | < 300 ms | 780–830 ms（整份 JSON 重寫） | 294–317 ms（寫入約 12 ms） |
| 排序 | 20 個固定查詢，前 10 名至少重疊 8 個 | — | 20/20 完全相同（files、repos、code 逐位元組） |
| 索引檔 | — | JSON 97 MB | db 172 MB |
| `ctx` 執行檔 | — | 8.9 MB | 10.7 MB |

暖查詢裡索引本身只剩約 25 ms（stat、讀 docs 列、撈 posting）；`--no-code` 剩下的大多是 `git ls-files`（orca 的未追蹤檔掃描約 100 ms）。
要整份 doc 的呼叫端（反芻、相關 repo 建議）比 JSON 時約慢 0.6 s（3.3 → 4.0 s）。

### 4c 會話索引（F-134）

- 同一個庫加會話表與訊息表。掃描器一家一個：Claude Code 讀 `~/.claude/projects`，Codex 讀 rollout（`codex_sessions.rs` 已有讀法）。
- 「過去的會話」、`ctx sessions --digest`、backfill 改成面對索引，Codex 因此補齊這三樣。
- 新增全文搜尋：`ctx sessions --grep`，App 的 ⌘K 多一個前綴。
- 寫入前照常過秘密遮蔽。

**驗收**：Codex 會話看得到逐字稿、digest、可以 backfill；兩家的搜尋結果都能一鍵接續。

## 9. 階段 5：讓之後的改動變便宜

### 5a 介面合約由方法表生成（N-52）

分三批，每批獨立可用。2026-11-18 實際做的（流程寫在 `DEVELOPMENT.md` §6「介面只增不改」）：

1. **做了。** MCP 每個工具的參數改成具名的 Rust 型別（`crates/ctx-cli/src/mcp/work.rs`、`admin.rs`），`inputSchema`
   由型別生成（新依賴 `schemars`，`mcp/params.rs` 把輸出整理成原本的形狀）；方法表 `mcp/tools.rs` 的 `TOOLS` 是唯一一份清單，
   tools/list、分派、`readOnlyHint`、`--admin` 分層都從它來。`tools/list` 跟改之前逐位元組相同（對拍照過）。
   快照 `crates/ctx-cli/schema/mcp.tools.json`＋上次發版的 `mcp.tools.released.json`，相容測試擋刪工具、改名、刪參數、
   選填變必填、新增必填、型別或 enum 收得更少、唯讀變非唯讀、工作層移到管理層。操作表（`ctx_core::ops`）來的管理層工具
   本來就是宣告式、跟 CLI／工作台共用，沒改。回傳沒有改成具名型別（大多已是 `Serialize` 的 struct，其餘是手組 JSON）。
2. **改成擋破壞，沒有改成共用型別。** 把 `--json` 全部改成具名型別是幾萬行的重構；改做形狀快照：在固定 fixture 上跑
   agent 會用的 31 個指令，`--json` 化成「鍵路徑 → 型別」（`cli.json.shape.json`／`.released.json`）；另由 clap 定義生成
   全部子指令的旗標表面（`cli.flags.json`／`.released.json`）。測試在 `crates/ctx-cli/tests/cli_schema.rs`。
3. **只做了契約測試，沒有生成 TypeScript。** `types.ts` 對應的引擎輸出大多是手組 `Value`；已是具名 struct 的（TermInfo、
   會話、決策卡…）生成 TS 要在 `ctx-core` 加 derive 依賴，生成的寫法（`T | null`、封閉的字串聯集）也跟手寫的
   （`field?:`、`(string & {})`）不同，換掉會牽動一串元件——風險大於收益，先不做。改做：`crates/ctx-core/tests/ipc_contract.rs`
   產生 `get_state`／`get_scan` 的 fixture，`desktop/src/test/contract.test.ts` 用 TypeScript 編譯器讀 `types.ts` 驗欄位與型別。
   `desktop/IPC.md` 只加了指向契約測試的一句，手抄的段落沒動。

原本的驗收「加一個欄位只改引擎的型別一處」：MCP 參數做到了（改參數 struct 一處，schema、解析、說明都跟著走）；
CLI、前端仍要各改各的，但改壞既有介面會被測試擋下。

### 5b 工作台 skill 進 spine（F-133）

**現況（2026-11-18）：機制已做（同日改版），搬哪些等使用者決定。**

使用者定的需求：這些 skill 是教 agent 用本工具（搜尋、拿別的 repo 的 context…）的，**repo 納管才用得到；沒納管、別處 clone、
移出列管都不能有任何檔案或痕跡**。所以不發成檔案，改由工具在會話裡提供，漸進揭露跟原生 skill 一樣三層：

1. 盤點各 repo 與 `~/.claude/skills` 的 skill，分成共享／重複／repo 專屬，給使用者勾。只讀不搬。（已完成：102 個 skill）
2. 使用者勾完後，要的放進 `spine/skills/<名稱>/`；原 repo 裡的副本由使用者決定刪不刪（動別的 repo 要先問）。（**等使用者決定**，一個都還沒搬）
3. 提供方式（**已做**，`shared_skills` 模組；規約見 `docs/RUST_PORTING.md`，手測見 `docs/TESTING.md` §8q）：
   - 第 1 層：開場 hook 只在納管 repo 注入「工作台 skill」段（名稱＋一行說明，有上限）；
   - 第 2、3 層：MCP `skill_get`／`ctx skill` 讀 SKILL.md 與附檔，只在納管 repo 回應；
   - 早上那版發到 `~/.claude/skills`、`~/.codex/skills` 的副本由 `ctx setup` 收回，`ctx health` 提醒。
   考慮過但沒採用：發到每個 repo 的 `.claude/skills`／`.agents/skills`＋`.git/info/exclude`（repo 工作目錄裡還是有檔）；
   Claude Code 外掛照專案開關（多則回報不可靠、Codex 要另寫一套）。
4. 還沒驗：Codex 壓縮脈絡後會不會重跑 SessionStart（不會的話清單會在長會話裡被摘要掉；`skill_get` 不帶名稱仍列得出來）。
5. `repo-context` 自己的 skill 還裝在使用者層（沒納管的地方也看得到），改成同樣的方式或短 stub＋`ctx instructions`。（沒做）

## 10. 要你決定的

| # | 問題 | 我的建議 | 擋住什麼 |
|---|---|---|---|
| 1 | 生成的 AGENTS.md、REPOS.md 要不要進 spine 的 git | 要：換機器 clone 下來就有 | 階段 1 可以先做，這個只影響 `.gitignore` |
| 2 | 願不願意依賴 herdr-core 這個外部執行檔 | 願意，先試作一天再定 | 階段 3 的路線 |
| 3 | 哪些 skill 算共享 | 我先出盤點表 | 5b |
| 4 | 什麼時候合併回 main | 你真用驗收之後 | 4b、4c、5a |

沒有回答的話：階段 0、1、2 不需要任何決定，我會照順序先做。

## 11. 不在這份規劃裡

- 原生 Chat 面板、雲端、手機 App、plugin 或 mod 系統、Linear 整合（R41、R42 都寫了不做的理由）。
- `ask_repo`、App 助手、碰撞判定改成能用 Codex 跑：另外排。
- F-131 對講送達前等草稿：小項，哪一階段順手就做。
- 2026-11-14 那批 Codex 同步功能的實測（`TESTING.md` §8l）：等使用者測，不擋這份規劃。
