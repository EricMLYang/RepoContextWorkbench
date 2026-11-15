# R41｜2026-11-15｜三個執行台比較：Orca、Better Agent Terminal、Herdr，以及跟本 repo 的定位

> 起因：使用者把 herdr.dev 加進 MyProjects 當參考 repo（跟 orca、better-agent-terminal 並列），
> 要求把 orca、BAT 拉到最新後比較三者，再跟本 repo 的定位做總比較。
> 這一輪只比較、不實作；新構想 F-128～F-131 進 `backlog.md`，狀態都是候選。

## 結論（先講）

1. **三個都是「執行台」**：解的是「同時跑很多 agent，怎麼看、怎麼管、怎麼不斷線」。三個都不管
   「agent 開工前該知道什麼」和「判斷留在哪」。本 repo 的定位（跨 repo 脈絡＋判斷留痕＋人的收件匣）沒有被任何一個蓋掉。
2. **執行台這一層我們追不上，也不該追**。Orca 一個月進了 7,450 個 commit（其中 593 個 feat），原生 Chat、雲端 VM、手機 App、Linear、plugin 市集都有；
   Herdr 把終端機做成獨立 server、多機、手機直連。我們的內嵌 terminal 是三者之下最弱的一塊，再投資報酬最低。
3. **該做的是 F-31：當它們的脈絡層**。`ctx setup --claude／--codex` 掛的是 user scope 的 MCP＋hook，
   所以在 Orca／BAT／Herdr 裡開的 Claude Code、Codex 會話照理已經有開場注入與留痕工具——這件事還沒驗過（F-129）。
4. **Herdr 有兩樣東西直接補我們的洞**：
   - **會話跑在獨立 server（herdr-core）**：我們的 AGENTS.md 寫「關掉視窗不殺掉正在跑的 agent」，但 ⌘Q 或換版時會話會結束
     （每次重裝 App 都遇到）。herdr-core 是現成、Apache-2.0、有 OpenRPC 的解法（F-128）。
   - **OSC 7501 狀態協定**：agent 自己用一行 escape sequence 說「處理中／做完／等人」。我們剛為 Codex 掛了 7 個 hook、
     還要使用者手動信任；讀 PTY 裡的 OSC 7501 是不分 agent 品牌的第二條路（F-130）。
5. **重疊最多的是「agent 互叫」**：Orca 的 orchestration、Herdr 的 `hc run／send／wait`、我們的對講與接力。
   差別在我們的對講會留痕、過收件匣、照 repo 組別限範圍；它們是純執行通道。值得抄的細節是 Herdr 的 draft wait（F-131）。

## 查證範圍

| 對象 | 讀了什麼 | 版本 | 標記 |
|---|---|---|---|
| Herdr App | 本機 clone 的 README、`defaults/`、`Cargo.toml`；herdr.dev 首頁 | `5216e6f`（2026-11-14，v0.1.15） | ✅ |
| herdr-core | 本機 clone 的 README、`skills/`、`schema/` 目錄 | `c84d6e7`（2026-11-14，v0.1.21） | ✅ |
| Herdr TUI | 本機 clone 的 README、`docs/next/website` 的 agents、session-state 兩頁 | `ee5bfb50`（2026-10-29） | ✅ |
| Orca | GitHub 上最新 README（2026-11-15）；`package.json`、目錄結構；R33／R34 讀過原始碼 | `67dc1ad8a23`（2026-11-15；pull 前 `3879c6d96f`，2026-10-14） | ✅ README｜原始碼是 10 月讀的 |
| Better Agent Terminal | GitHub 上最新 README；`package.json`；R33／R37 讀過原始碼 | `2ed1cc6`（2026-11-14，v3.2.19-pre.1；pull 前 `9b468b4`，2026-10-12） | ✅ README｜原始碼是 10 月讀的 |

沒做的：三個都沒有實際安裝操作；Orca 與 BAT 這一個月新增的程式沒有逐檔讀；orca.dev 的 docs 站沒有讀。
下表凡是功能描述，來源都是各家自己的 README，等於它們的自我宣稱。

## 三者比較

| | Orca | Better Agent Terminal（BAT） | Herdr（App＋core；另有 TUI） |
|---|---|---|---|
| 一句話 | 平行跑一群 agent 的 IDE，每個任務一個 worktree | 多工作區終端聚合器，內建 Claude／Codex 面板 | 終端機 server＋可被 agent 改造的原生 App |
| 規模 | 12.5 萬星、MIT、repo 2.9 GB、6.4 萬個檔、v1.4.214 | 523 星、MIT、個人專案、v3.2.19-pre.1 | TUI 5.1 萬星、App 2,918、core 695；Apache-2.0；App 與 core 2026-10-23 才開始 |
| 技術棧 | Electron＋React＋node-pty＋xterm.js；另有 `cloud/`、`mobile/` | Tauri 2＋React＋xterm.js＋Node sidecar（跑 SDK） | Rust＋GPUI（Zed 的 UI 框架）＋ghostty-vt；mod 用 QuickJS 跑 JavaScript |
| 工作單位 | workspace＝一個 worktree | workspace＝一個資料夾，可分 group、profile | space＝某台機器上的一個 repo → tab → pane |
| 怎麼接 agent | 原生 Chat 或原始 CLI，兩者可互轉；號稱任何 CLI agent | Claude 走 Agent SDK、Codex 走 app-server，畫自己的面板；也能開原始終端 | 只跑真終端，不包裝 agent |
| 狀態從哪來 | 各家 hook（R34 讀到 working／waiting／done／blocked＋衰減） | SDK 事件流 | App／core：agent 自報 OSC 7501，core 不猜；TUI：讀畫面最下方＋偵測規則，另有 integration |
| 關掉前端後 | 可配 Remote Orca Server、SSH、雲端 VM；筆電睡了照跑 | 會話可持久化、重開後 resume；rest／wake | 終端機本來就跑在 core，關 App、更新、當掉都不斷；重開機後只還原版面與目錄 |
| agent 叫 agent | orchestration：一個 agent 拆工、開 worker、收結果（Orca CLI） | 只有 SDK 的 subagent 追蹤 | `hc run --bg`、`hc send`、`hc wait --children`；父子關係、子的回報進父的輸入 |
| 多機與手機 | 遠端 server、SSH、雲端 VM；iOS／Android App | profile 可連遠端；iOS（TestFlight）／Android | SSH 機器併入同一側欄；手機直連自己的機器，不經第三方 |
| 審查產出 | diff 逐行留言送回 agent、起草 commit／PR、CI 失敗與 review 送回 agent | git log／diff 檢視、GitHub PR／issue 面板 | 沒有內建，靠 mod |
| 任務來源 | GitHub、Linear 原生整合；排程 automations | 無 | 無 |
| 擴充方式 | plugin 市集（主題、編輯器、面板） | statusline 可排、snippet；沒有 plugin | 版面、按鍵、功能全是 `~/.config/herdr-app` 的檔案，附 AGENTS.md 教 agent 改；312 個版面、576 個 mod 的分享區 |
| 給外部程式的介面 | Orca CLI（agent 可開 worktree、開 worker、操作瀏覽器） | IPC 內部用 | `hc` CLI、`hc schema` 印 OpenRPC、`hc connect` 可走 SSH；可自己做 client |
| 計費與帳號 | 用你自己的訂閱；看用量、切帳號 | SDK 路徑（R34 記過計費差異）；看 5h／7d 用量、cache 成本、切帳號 | 不碰，agent 自己管 |
| 跨 repo 脈絡 | 無（跳轉能跨 repo，內容不跨） | 無 | 無 |
| 判斷留痕 | 無 | 無 | 無 |

### 三者各自的重心

- **Orca：全包**。從任務（Linear）到 worktree、agent、diff 審查、PR、雲端、手機都自己做，每天發版。
  它正在變成平台，使用者的工作流越來越綁在它裡面。
- **BAT：把 agent 的對話畫得更好看、更省錢**。重心在單一會話的品質：權限模式、cache 成本與 TTL 提醒、context 用量、
  fork、statusline。跟我們同棧（Tauri＋React），所以實作細節最能直接參考。
- **Herdr：把「終端機活著」這件事做成基礎建設**，其餘交給使用者和 agent 自己拼。core 不畫畫面、不認 agent 品牌、
  狀態只收自報。這是三者裡唯一刻意做小、而且歡迎別人疊在上面的。

### 這一個月的變化（對照 10 月的紀錄）

- **Orca**：`3879c6d96f` → `67dc1ad8a23`，一個月 7,450 個 commit。10 月的紀錄只寫了「每任務一個 worktree 的平行執行台」
  與會話狀態相關的細節；原生 Chat、orchestration、雲端、手機 App、Linear、plugin、automations、computer use
  在 10 月的 commit 紀錄裡就有了，是當時只看了跟我們相關的部分、沒記到。這一個月各塊都還在大量加碼
  （commit 主旨提到 mobile 的 1,025 個、plugin 275、chat 235、orchestration 215）。
  用 commit 主旨搜 memory／context／handoff／decision／knowledge，沒有跨 repo 記憶或判斷留痕的功能
  （搜到的 memory 是 relay 的記憶體 cell 與終端機記憶體上限）。
- **BAT**：3.0 起拿掉 Electron、只剩 Tauri；README 主打手機 App、cx 語意導覽（讓 agent 先看檔案概覽再決定讀哪份）、
  Codex 會話索引與 resume 快取。`9b468b4` → `2ed1cc6` 只有 27 個 commit，feat 有 5 個：更新 Claude runtime 與模型清單、
  聊天裡畫 Mermaid、Fast mode 下一輪生效、5h／7d 用量上限重置後自動接續 agent。步調是 Orca 的幾百分之一。
- **Herdr**：R35 記的是「終端 runtime、無 GUI」。現在有獨立的桌面 App（GPUI），core 從 TUI 拆出來重寫；
  TUI 與 App 是兩套程式，不共用 agent。

## 跟本 repo 的定位比

本 repo 的定義（`docs/VISION.md` §2）：context 用 repo 管起來之後，讓人和 agent 都拿得到、用得上、留得下痕跡。
北極星是「回顧時看得見判斷與品味變好」，不是省時間。

| 問題 | 三個執行台 | 本 repo |
|---|---|---|
| 操作單位 | 一個任務的 worktree／資料夾／終端 | 一手牌＝repo 組合（組） |
| agent 開場知道什麼 | 該 repo 自己的 AGENTS.md，其餘靠人貼 | 開場注入：組別、同組成員、上次交接、下一步、未結 |
| 需要別的 repo 的資料 | 自己找路徑 | `search_knowledge`／`repo_entry`／`read_from`，引用有紀錄 |
| 做了判斷 | 留在逐字稿裡 | `log_decision` 進 spine，可搜尋、可取代、可回看結果 |
| 會話結束 | 逐字稿可 resume | `handoff` 寫下完成了什麼、下次從哪接；沒交接會進收件匣 |
| 等人決定 | 側欄亮燈，人切過去看終端 | 決策卡：問題、選項、建議、證據；可設放行規則 |
| agent 之間 | 開 worker、傳字串、收回報 | 對講與接力：照組別限範圍、過收件匣、留痕 |
| 資料在哪 | 各自的 app 狀態 | 使用者自己的 spine repo，純文字、在 git 裡 |
| 跑 agent 的地方 | 主力，三家都遠比我們強 | 內嵌 terminal，夠用但不是重點 |

**不重疊的（我們獨有）**：registry 與組、spine、開場注入、交接、決策卡與放行規則、跨 repo 搜尋與引用、反芻、總管。

**重疊而我們較弱的**：會話存活（⌘Q 就斷）、狀態來源（只認有 hook 的兩家）、多機與手機（沒有）、diff 審查（有逐行留言，沒有 PR 流程）、
agent 互叫（對講要對方在同組且登記過）。

**重疊而不該追的**：原生 Chat 面板、雲端 VM、手機 App、plugin 市集、Linear 整合。這些沒有一項對應到三條取捨準則。

### 定位上的風險

- **Orca 往上吃**：它已有 GitHub／Linear 任務、orchestration、automations。只要再加「跨 workspace 的共用記憶」，
  就踩到我們的範圍。目前 README 沒有這一項（💭 推論：它的方向是執行與協調，不是留痕）。
- **我們把時間花在 terminal**：R33 以來抄了不少執行台功能（會話列、Needs You、過去的會話、接續、Dock 徽章）。
  這些讓 App 能當座艙，但每一項三家都做得更完整。之後 terminal 相關的構想要先問：使用者改用 Herdr 或 Orca 開會話時，這個功能還有沒有用。
- **F-31 一直是候選**：從 R01 排到現在沒做。三家都長大之後，這是定位上最該先確認的一件事。

## 新增構想（都是候選，進 `backlog.md`）

| ID | 構想 | 準則 | 來源 |
|---|---|---|---|
| F-128 | **會話跑在 App 之外**：App ⌘Q、換版、當掉都不中斷會話。兩條路：自己拆出常駐的 PTY 行程，或直接用 herdr-core 當後端（`hc run`／`hc attach`，OpenRPC）。先評估第二條：多一個外部依賴，換來不用自己維護 | 同介面 | Herdr |
| F-129 | **驗 F-31**：在 Orca、BAT、Herdr 各開一場 Claude Code 與 Codex，確認開場注入、MCP 工具、交接都正常；不正常的列出原因（例如 BAT 走 SDK 時 hook 有沒有觸發）。只驗證與寫文件，不改程式 | 脈絡、留痕 | 三家 |
| F-130 | **狀態多一個來源：OSC 7501**。App 的 terminal 讀 PTY 輸出裡的 OSC 7501，讓沒有 hook 的 agent（`AgentKind::Other`）也有「處理中／等你／做完」。要先查 Claude Code、Codex 目前有沒有發這個序列（💭 還沒查） | 同介面 | Herdr |
| F-131 | **對講送達前等草稿**：使用者正在打字時，不把對講訊息貼進那場會話，等送出或清空、最多 60 秒（Herdr 的 `draft_wait`）。現在是 agent 用完工具就送 | 同介面 | herdr-core |

排序建議：F-129 最先（半天、不改程式、直接回答定位問題）→ F-128（補上 AGENTS.md 已經寫了但沒做到的事）→ F-131 → F-130。

## 對標清單的更新

- Orca、BAT：更新「最後查證」與本輪看到的變化。
- Herdr：類型從「終端 runtime（無 GUI）」改成「終端 server＋原生 App」，狀態從觀察升為參考，查證 🔗 → ✅。
