# 開發重點（Rust＋Tauri＋React＋MCP）

> 給人和 agent 共用的寫程式準則：只寫**這個產品**特有的重點，通用常識不重複。
> 架構方向與硬規定在 `AGENTS.md`；搬遷規約與刻意差異在 `docs/RUST_PORTING.md`；IPC 合約在 `desktop/IPC.md`。
> 2026-10-13 依現況（`rust-desktop` 分支）整理，改架構時一起改這份。

## 1. 產品特性決定的四條底線

1. **資料格式就是合約。** spine（事件 md）、`registry.yaml`、`.state/`、CLI 旗標與 `--json` 欄位、MCP 工具介面都是對外 API：
   人會手改、agent 會讀、git 會 diff。改合約要升版並寫進 `RUST_PORTING.md`；只加欄位、不改既有欄位的意思。
2. **人和 agent 走同一個引擎。** 每個能力都先做成 `ctx-core` 的函式，再由 CLI／MCP／桌面三個入口各接一層薄殼。
   入口裡只做參數轉換與輸出格式，不准有業務判斷。
3. **agent 會同時改東西。** 任何寫入都要假設磁碟在你讀完之後被改過：寫 spine／registry 走 `locking`；寫使用者檔案要帶
   讀取時的版本、對不上就回衝突（例：`filetree::write_file`）；先寫暫存檔再 rename。畫面上的資料隨時可能過期，
   要靠事件即時更新，而不是靠輪詢。
4. **留痕優先。** 判斷寫 spine、改檔交給 git，兩邊不重複記。新功能要先回答「這件事事後查得到嗎」。

## 2. 分層與依賴方向

```
ctx-cli（clap／MCP stdio／hooks）  ─┐
desktop/src-tauri（視窗、IPC、PTY）─┼─▶ ctx-core（引擎，純 lib）─▶ 檔案系統、git、gh、claude
desktop/src（React，只管畫面）── IPC ┘
```

- 依賴只能往右指。`ctx-core` 不依賴 Tauri、clap 或任何 UI；要用殼的能力（例如會話清冊）就定義 trait 由外面注入（例：`Terms`）。
- 一個領域一個模組（`spine`、`registry`、`filetree`…），對外只露少數函式；檔案超過約 800 行時，依職責拆成子模組
  （`workbench/`、`agentapi/` 已經這樣拆；`workbench/actions/` 再依一級物件分 `sessions`／`files`／`finder`／`briefs`／`namecard`，
  `ctx-cli/src/cli/` 的 `health` 也獨立一檔）。目前仍超過的：`agentapi/mod.rs`、`cli/agent.rs`、`filetree.rs`、`assist.rs`、
  `backfill.rs`（後兩個是對拍 Python 的規格，合併回 main 後再拆）。
- 外部指令（git／gh／claude）集中在少數模組呼叫並包裝錯誤，會跑很久的（claude）一定要設逾時；測試用 `testkit` 的假資料，
  不打真的網路。

## 3. Rust

- **錯誤**：引擎用 `thiserror` 的 `ctx_core::Error`，每個變體都對應呼叫端要做的不同處理；不要再加只裝字串的通用變體。
  錯誤訊息要寫給使用者看（說明是哪個檔、哪個 repo、該怎麼辦），因為它會直接顯示在 App 和 agent 那邊。
- **型別表達狀態**：「成功／衝突」「文字／圖片／二進位」這種分支用 enum（例：`WriteOutcome`、`FileView`），不用布林加字串。
  桌面 action 的參數用 `parse::<T>()` 解析成 `#[derive(Deserialize)]` 的 struct／enum（例：`ReadFileArgs`、`ReorderArgs`）；
  照抄 Python 的舊 action 還用 `raw`／`s`／`int`（Python 的 truthy 語意），合併回 main 後再換。
  拒絕給 agent 的錯誤用 `RejectCode` enum，字串形式是 MCP 合約。
- **一種狀態、一個 enum、一個地方**：會話階段用 `status::AgentStatus`（含 hook 才會寫的 `Finished`／`Exited`），
  App 會話掛的 hook 用 `session_status::StatusHook`（`ctx hook <arg>` 的字串只在 `arg()` 出現一次），「誰在等你」的分類只在
  `workbench::Attention`——殼讀 `get_state` 的 `attention`，CLI 把 clap 的 enum 對到引擎的 enum，誰都不自己比字串。
  （2026-10-16 檢查時殼裡曾多一份一樣的分類，已拆掉。）
- **回傳用 struct**：引擎函式回 `#[derive(Serialize)]` 的 struct（例：`agentapi::Logged`、`TodoAdded`、`HandedOff`），
  到 MCP／CLI 的邊界才轉成 JSON（`mcp/work.rs` 的 `json()`、CLI 的 `agent_call`）。欄位順序和名稱就是合約，
  改了要過 `parity.sh`。舊函式還有不少回 `serde_json::Value` 的（讀取類工具、`ops`），碰到時順手換。
- **`unwrap`／`expect`** 只用在程式裡寫死的不變式；鎖中毒用 `unwrap_or_else(|e| e.into_inner())`。
- **依賴從嚴**：版本放在 workspace 的 `[workspace.dependencies]` 統一管理；標準庫能做的就不加 crate；每加一個依賴，
  都要在 commit 訊息說明為什麼。
- **路徑**：一律用 `Path`／`PathBuf`，不手拼 `/`；從外面來的相對路徑都要經過 `filetree::resolve_in` 這種檢查（擋 `..` 和指到 repo 外的符號連結）。
- **版本號**：從 `env!("CARGO_PKG_VERSION")` 取，不寫死。
- **全域常數**：用標準庫的 `std::sync::LazyLock`（不用 once_cell）。

## 4. 桌面殼（Tauri v2）

- **權限採最小化**：`capabilities/default.json` 只開前端實際用到的；新功能優先寫成自訂 `#[tauri::command]`，
  不要把 plugin 權限整包開給前端。CSP 維持不准 inline script、不連外網。
- **不信任 repo 內容**：repo 裡的 md／html 不能在 App 裡執行（Markdown 自己轉成 React 元素，不用 `innerHTML`），
  外部連結只複製、不開啟。
- **閒置不吃 CPU**：檔案監看只看展開中的資料夾（`tree_watch`）；OS 漏掉事件時要整批重讀，不能直接丟掉。
- **會話生命週期獨立於視窗**；本機服務只綁 127.0.0.1 並帶隨機 token；啟動時自己補 PATH（`env_path`）。
- 殼裡的阻塞工作放進 `spawn_blocking`，不要卡住 IPC 執行緒。

## 5. 前端（React＋TS）

- **只負責畫面和互動**：資料一律從引擎取，不在前端計算業務結果。要新增一種資料，先在引擎加 action，
  更新 `api/types.ts` 的 `ActionMap`，再寫畫面。
- **非同步三件事**：執行中鎖住按鈕（`AsyncButton`）、失敗時把錯誤顯示出來、斷線或失敗時保留上次的資料。
  連續發出的請求，只採用最後一次的結果（例：`FileTree` 的 `seq`）。
- **元件依一級物件拆**：deck（牌）／inbox（收件匣）／sessions（會話）／deep（深看）／viewer；跨元件的狀態放在 `store/`，
  不用 prop 一層層傳下去。
- 需要給非同步回呼看最新值的 ref，在 effect 裡更新，不要在 render 時更新（`react-hooks/refs` 會擋）。
- **畫面操作和畫面狀態分開**：`useUi()` 只有操作（整個 App 生命週期同一個物件），`useUiState()` 是會變的狀態
  （草稿、展開的牌組、目前的終端…）。只在點擊當下需要值的，用操作端的 getter（例：`ui.getDraft()`），不要訂閱狀態，
  否則每打一個字整個畫面都會重畫。
- 不用瀏覽器原生的 `alert`／`confirm`，危險操作改成「按兩次」或站內提示。

## 6. MCP／CLI 介面（給 agent 用的 API）

- 工具依「使用者要完成的事」設計（`context_for`、`handoff`），不要把引擎的函式一個個直接開出去；工具和參數的描述
  要寫清楚必填、選填和預設值，agent 只看得到這些描述。
- stdio 模式的 stdout 只能輸出協定訊息；log 寫到 stderr 或檔案。
- 錯誤要回「發生什麼事＋可以怎麼補救」（例：跨組寫入被拒時附上 hint），設定錯誤時不能讓 server 崩潰。
- CLI 每個查詢指令都要支援 `--json`，欄位一旦發佈就不改意思。
- **hook 回應紀律**（N-42）：觀測類 hook（Stop／Notification／PreToolUse／PostToolUse／UserPromptSubmit／statusLine）
  必須 < 100ms 回、不等網路、不印會擋住 Claude Code 的東西；讀逐字稿只讀尾巴固定位元組。PreToolUse 每次工具呼叫都會
  spawn 一次 `ctx`，任何會讓它變慢的改動（讀事件、讀 registry、git）都不准放進這條路徑。核准類 hook 若將來要回決定，
  一定帶 timeout、逾時交回 TUI。2026-10-18 實測（release、7MB 逐字稿）：tool-start／tool-end／stop／prompt 都約 10ms。
  SessionStart 是例外（要組脈絡），預算 < 500ms。

## 7. 測試與驗證

- 改完必跑（CI 也跑同一套）：**`scripts/check.sh [rust|front|all]`** 一次跑完下面這些，過的步驟不印、失敗印尾巴——
  `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`＋`scripts/parity.sh`，
  前端在 `desktop/` 跑 `npm run build && npm test && npm run lint && npm run format:check`（格式化用 `npm run format`）。
  改到一半不要跑：對拍會把半成品編進去。
- 引擎邏輯寫單元測試；action 和合約寫 `crates/ctx-core/tests/` 的整合測試；畫面行為寫 vitest（用 mock backend）。
- 和 Python 版對拍（`scripts/parity.sh`）只到合併回 main 為止；之後由整合測試接手守住合約。
- 每個功能的 commit 都要註明：macOS GUI 實測過沒有、Windows 測過沒有。
- **真 agent 端到端**（2026-10-14）：`bash scripts/e2e_claude.sh <spine>` 用真的 Claude Code（haiku，花一次小額 API）
  在 spine **副本**上走開場注入 → `context_for` → `handoff` → 結束，自動檢查交接、標記清理、會話狀態。
  隔離：`--setting-sources project`（不跑使用者的其他 hook）＋`--settings`／`--strict-mcp-config` 只掛 `--spine <副本>` 的 ctx。
  改 hook、MCP 或開場注入後跑一次。
- **GUI 實測**：使用者的 App 沒開時，`npx tauri build --no-bundle` 的執行檔帶 `--spine <副本>`（副本先拿掉 git remote），
  截圖只拍該 pid 的視窗（`screencapture -l`）、按鍵只送該 pid（`CGEventPostToPid`）；不要全域點擊或打字。
- **無障礙稽核（自動化，#12 的 macOS 部分；2026-10-21）**：`desktop/src/test/a11y.test.tsx` 用 axe-core 對掛起來的 App
  （工作區、牌組範圍、新建會話對話框、⌘K、右側欄）跑 WCAG 規則，serious／critical 要是 0；`npm test` 就會跑。
  jsdom 算不了顏色對比（`color-contrast` 關掉），對比與真鍵盤流程仍要在真 App 看。新畫面加進這個測試的場景清單。
- **一次性 shell 腳本的寫法**（2026-10-18 踩到，#100）：變數名後面緊貼全形標點或中文時一律 `${VAR}`（`"${BUILT}（"` 而不是
  `"$BUILT（"`）——背景或非 UTF-8 locale 下 bash 會把後面的位元組當成變數名的一部分，炸出 unbound variable；
  `set -u` 的腳本更是直接停。`install-app.sh`、`check.sh` 都中過。
- **畫面預覽（App 開著、或只想看排版時）**：`desktop/dev/preview.html` 用真的 App 元件＋固定假資料，`?scene=` 選場景
  （workspace／editor／code／assist／assist-empty）。`cd desktop && npm run preview:screens`，再用 headless 瀏覽器
  （獨立的 `--user-data-dir`、`--headless=new`，不開視窗、不碰使用者的瀏覽器）截圖：
  `"…/Google Chrome" --headless=new --user-data-dir=<暫存> --window-size=1440,900 --virtual-time-budget=5000 --screenshot=<png> "http://localhost:5199/dev/preview.html?scene=editor"`。
  一次截全部場景：`bash scripts/preview-shots.sh [輸出目錄] [寬,高] [場景…]`（自己起、關 dev server）。
  看得到排版與上色，看不到 Tauri 才有的東西（終端、檔案監看、原生對話框），那些仍要真 App 實測。
  滑鼠事件送 pid 不生效（視窗不是 key window），點擊流程要人來點。
- **磁碟衛生（2026-10-17）**：只留有用的產物，不累積。`target/debug` 曾長到 39 GB（相依升級、切分支留下的舊版 `.rlib`，cargo 從不回收）。
  規則：`scripts/check.sh` 跑完自動 `cargo sweep --time 7`（要先 `cargo install cargo-sweep`；沒裝只提醒），
  只刪 7 天沒被任何 build 用到的產物，不影響增量編譯速度。全清（`cargo clean --profile dev`）只在 sweep 之後還是太大時才做，
  代價是下一次重編相依約 5～10 分鐘。App 舊版備份在 `~/.cache/repo-context/app-backups/`，`scripts/install-app.sh` 自動只留 3 份（每份約 19 MB）。

## 8. 資訊安全

### 信任模型（寫程式時要守住的邊界）

| 來源 | 信任程度 | 守法 |
|---|---|---|
| 使用者自己下的 CLI／App 操作 | 信任 | — |
| 登記的 repo 的**檔案內容**（md、程式碼） | **不信任**：可能是別人的 repo | 前端不執行（不用 innerHTML、CSP 不准 inline script、外部連結只複製）；路徑一律經 `filetree::resolve_in` |
| 其他 repo 給 agent 的內容（`read_from`、`ask_repo`、開場注入） | **不信任**：可能夾帶 prompt injection | 當成資料、不是指令；`ask_repo` 的子 agent 只開唯讀工具；憑證樣式的內容要遮掉 |
| spine 的 `config.yaml`（`agents.<名>.cmd`）、`.mcp.json` | **等於程式碼**：開會話時會被執行 | 只用自己的 spine；別人的 spine 要先讀過 |
| App 的 webview | 一旦被攻破就等於本機權限（能開終端、讀寫登記 repo 裡的檔） | 所以上面三列都要守住；capabilities 只開事件監聽，新能力寫成自訂 command |

### 寫程式的規則

- **外部指令**：參數一律分開傳給 `Command::args`，不准用 `sh -c` 拼字串。非用 shell 不可時（PTY 的 `sh -lc`、
  macOS Terminal 的 AppleScript）每個參數都要經過 `term::shlex_quote`，再依外層語法跳脫（例：`session::osascript_for`）。
  Windows `cmd /k` 擋不住 `& | < > ^ % !`，有這些字元就不開視窗、改印出指令。
- **路徑**：從前端或 agent 來的路徑都要 canonicalize 後確認在範圍內（符號連結跳出去也擋）；
  不能把根目錄或家目錄登記成 repo（`registry::add_repo` 會拒絕）。
- **寫檔**：同資料夾暫存檔（名稱不可預測、`create_new`）→ fsync → rename；保留原檔權限。
- **本機服務**：只綁 127.0.0.1、隨機 token、timing-safe 比對；桌面版走 Tauri IPC，不開任何埠。
- **機密**：不 commit 任何 token 和個人資料；測試用的假憑證放在 `pack` 測試裡，並標明是假的。
  `docs/` 只放可以公開的內容，個人或公司的素材放在 repo 外。
- **CI**：`permissions: contents: read`、第三方 actions 釘 commit SHA；依賴更新時在有網路的環境跑一次
  `cargo audit`／`npm audit`。
- **回報管道**：`SECURITY.md`（GitHub 私下回報；repo 設定要打開 Private vulnerability reporting）。

### 開源前的待辦（2026-10-13 檢查）

- [ ] git 歷史要處理：個人路徑與 Email 在 `docs/工具想法.md`（2026-10-13 已從目前的檔案移除，歷史裡還在），
  commit 作者也有公司租戶信箱（6 筆）和本機主機名信箱（9 筆）→ 用新 repo 單一 commit 釋出最乾淨；
  要保留歷史就用 `git filter-repo` 刪檔＋`--mailmap` 改作者
- [x] `docs/` 收斂（2026-10-13）：只留 `VISION.md`、`DEVELOPMENT.md`、`RUST_PORTING.md`、`ideation/`；
  過程紀錄與截圖移除。開源前再看一次 `ideation/` 的對標與輪次是否提到個人 repo 名稱
- [x] `LICENSE`（MIT）
- [x] macOS 開終端的引號注入（`session.rs`）
- [x] CI 權限最小化、actions 釘 SHA
- [x] 拒絕登記根目錄／家目錄；存檔暫存檔不可預測
- [x] 加 `SECURITY.md`（公開 repo 後要在設定打開 Private vulnerability reporting）
- [x] `cargo audit`／`npm audit`（2026-10-13）：沒有漏洞。`cargo audit` 有 3 個警告：`unsafe-libyaml`
  無人維護（`serde_yaml` 帶進來，`serde_yaml` 本身也已封存）、`proc-macro-error` 無人維護與 `glib` unsound
  （都是 Tauri 在 Linux 才用的 GTK 依賴，macOS／Windows 不編譯）。`serde_yaml` 要換成維護中的分支，
  但會影響 registry.yaml 的輸出格式，要等合併回 main 後跟對拍一起處理

## 9. 開源後要讓別人裝得起來（目前的缺口）

| 項目 | 現況 | 要做的 |
|---|---|---|
| 授權 | 已補 `LICENSE`（MIT） | — |
| 安裝 | 要自己 build；`install-app.sh` 是給開發者重裝用的 | 用 GitHub Releases 發 `.dmg` 和 `ctx` 執行檔；README 第一段寫三步驟安裝＋`ctx init` |
| macOS 簽章 | 沒簽章、沒公證 → 別人下載後會被 Gatekeeper 擋住 | 發佈時用 Developer ID 簽章並公證（Tauri 有內建流程），在 CI 上做 |
| Windows | CI 只確認 `ctx` 編得過 | 先發 CLI＋MCP，桌面版之後再補（每個功能都要註明測過沒有） |
| 個人化設定 | bundle id 是 `dev.ericmlyang.*` | 換成中性的 id |
| 語言 | 只有中文 | 至少 README 要有英文版 |
| 第一次使用 | `SpineSetup` 要手動填 spine 路徑 | `ctx init` 一步建好 spine，並附一個範例 |

## 參考

- Tauri v2 安全模型（預設拒絕、capabilities、CSP）：<https://www.oflight.co.jp/en/columns/tauri-v2-security-model>
- Tauri 發佈與簽章：<https://v2.tauri.app/distribute>
- 函式庫用 thiserror、應用程式用 anyhow：<https://oneuptime.com/blog/post/2026-01-25-error-types-thiserror-anyhow-rust/view>
- MCP server 實務（stdout 乾淨、錯誤可補救、工具描述）：<https://nordicapis.com/8-tips-and-best-practices-for-mcp-server-development/>、<https://steipete.me/posts/2025/mcp-best-practices>
