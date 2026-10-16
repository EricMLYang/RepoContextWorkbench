# Repo Context Workbench — repo-context 工作台

> **這是什麼**：把一群 repo 分成組，每組組成一份 context——餵給 agent，也餵給自己。
> 人和 agent 走同一個介面：CLI、MCP server、桌面工作台三個入口，背後是同一個引擎、同一份 spine。
>
> **現況（2026-10-14，`rust-desktop` 分支）**：Rust 引擎＋單一執行檔 `ctx`（CLI／MCP／hooks）＋Tauri 桌面 App。
> Python 版（`repo_context/`、`tests/`）在合併回 main 前**仍保留**，只當對拍規格；合併後刪除。
>
> 文件：為什麼做、往哪走 → `docs/VISION.md`｜寫程式的準則 → `docs/DEVELOPMENT.md`｜合約與刻意差異 → `docs/RUST_PORTING.md`。

## 先認識 spine：工作台自己的資料 repo

用之前要先有一個 **spine**——一個專門給工作台用的 git repo，跟你平常工作的 repo 分開。
整台電腦一個就夠，登記再多 repo 都存在同一個 spine 裡。

```
~/repo-spine/
  registry.yaml    登記了哪些 repo（id、路徑、tags、關係）、怎麼分組
  spine/events/    事件，一則一個 md：判斷、待辦、交接、狀態回報（agent 的 log_decision／add_todo／handoff 寫在這）
  groups/          各組的筆記與彙整產物
  config.yaml      工作台設定（可開哪些 agent 等）
  .state/          可重算的快取：搜尋索引、會話標記、已讀水位（不進 git）
```

- **工作台只讀你的 repo，不寫進去。** 所有輔助資料（登記、分組、判斷、交接、索引）都在 spine，
  你的 repo 內容、git 歷史、`.gitignore` 不會因為這個工具多一行。
- spine 本身是 git repo：會話結束自動 commit，判斷與交接都有歷史可查；要換電腦就把它 clone 過去。
- 建法二選一：桌面 App 第一次打開時按「建立新的 spine」（只會建在新路徑或空資料夾），
  或在終端機跑 `ctx init ~/repo-spine`。接著 `ctx setup --claude` 記住位置、掛上 agent 的 MCP 與 hooks。
- spine 等於程式碼（見下方「信任模型」）：不要直接拿別人的 spine 來用。

## 架構

```
crates/ctx-core/   引擎 lib：registry／spine／採集／脈絡／知識／碰撞／會話 PTY／工作台 state 與 actions
                   （不依賴 Tauri、不依賴 CLI 框架；對應 repo_context/*.py，一模組對一檔）
crates/ctx-cli/    bin `ctx`：CLI（clap）＋MCP server（stdio）＋Claude Code hooks 入口
desktop/           桌面工作台：src-tauri/（Tauri v2 殼：視窗、IPC、PTY 轉接、選單列、通知）
                   ＋ src/（React＋Vite＋TS 前端，只管畫面；IPC 合約見 desktop/IPC.md）
```

資料格式就是合約：spine（`spine/events/*.md`）、`registry.yaml`、`config.yaml`、`.state/`、CLI 旗標與 `--json` 欄位、
MCP 工具介面都與 Python 版相同，兩版可以讀寫同一個 spine（`scripts/parity.sh` 逐指令對拍）。

## 建置

```bash
# 需要 Rust（stable）與 Node 20+
cargo build --release -p ctx-cli                 # → target/release/ctx（CLI＋MCP＋hooks，單一執行檔）
cd desktop && npm install && npm run bundle      # → target/release/bundle/macos/Repo Context.app（及 .dmg），內含 ctx
                                                 #   只要 .app：npm run bundle -- --bundles app

# 驗證（改完必跑）
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
cargo build -p ctx-cli && bash scripts/parity.sh          # 與 Python 版逐指令對拍
python3 scripts/parity_state.py                           # 工作台 state／scan JSON 對拍
cd desktop && npm run build && npm test && npm run lint && npm run format:check
```

## 安裝

重裝桌面 App（含 ctx）最省事：先從選單列結束 App，再 `bash scripts/install-app.sh`（`--dry-run` 先看步驟；
會備份舊版到 `~/.cache/repo-context/app-backups/`、用 ditto 整包替換、刪掉 build 目錄那份）。

```bash
cargo install --path crates/ctx-cli      # ctx 裝到 ~/.cargo/bin/ctx
# 桌面 App：把 target/release/bundle/macos/Repo Context.app 拖到 /Applications
#   .app 自帶 ctx（Contents/MacOS/ctx）；App 的背景判定與會話 .mcp.json 用的是這一支，不看 PATH。
#   不想另外 cargo install，也可以直接連過去：
#   ln -s "/Applications/Repo Context.app/Contents/MacOS/ctx" ~/.local/bin/ctx
ctx app                                  # 找 Repo Context.app（ctx 同目錄、/Applications、~/Applications、target/）並帶 --spine 開啟
```

桌面 App 目前只做了 macOS（Windows／VDI 未做、未測）。關視窗＝隱藏，會話照跑；選單列「結束」或 ⌘Q 才收掉會話。

### 桌面 App 常用操作

- **⌘K**：找會話（切過去）、repo（名稱片段、詞首縮寫如 `rcw`、名片的角色／介紹／關鍵字）、牌組、檔案（目前範圍，用檔名）；
  Enter 開，⌘Enter 對 repo 開 Agent 會話或把檔案加入脈絡。打「>」只列指令（寫組筆記、請 Agent 分析想法、開 Shell、重新採集、切分頁）。
- **App 助手（⌘K 打「?」）**：一句話改工作台設定——「切到 PM Group」「VMS_PM 加標籤 規格」「把 a 加進 MI3.0」「暫停 X，等年底再看」
  「關掉自動儲存」…；先給計畫、勾選確認才做、做完可以復原。常見說法本機直接翻，其他問 Claude（幾秒）。
  終端裡用 `ctx assist "<一句話>"`，agent 用 MCP `workbench_apply`（同一份白名單）。
- **看與改檔案（HackMD 式）**：md 預設預覽，「編輯」左寫右看（⌘⌥E／⌘⌥B／⌘⌥V 切編輯／分割／預覽）、格式工具列、清單延續、
  自動儲存（下緣看得到存了沒）、預覽裡勾待辦、貼截圖存進 `assets/`、從檔案樹拖檔插相對連結；⌘F 找（編輯中可取代）、⌘L 前往第幾行、
  「大綱」跳標題、程式碼上色；md 最下面列出「連到這份的文件」。檔案樹「＋」「⊞」新增檔案／資料夾、F2 改名。
- **挑幾份檔案交給 agent**：檔案樹 ⌘＋點、右鍵「加入脈絡」、檢視器「加入脈絡」，或從檔案樹把檔拖到下方終端；
  主區下緣的「脈絡 N 個檔案」可以「交給新會話」、「貼到目前會話（不送出）」或「複製 @路徑」。
- **哪個 agent 在等你**：App 開的 Claude Code 會話會自動回報（做完一輪＝等你、要你允許工具＝卡住，視窗不在前面時發通知）；
  會話列紅點、牌組卡「N 個等你」、工作區「回到會話」（等你的優先）、選單列「等你 N」（點了切過去）。
  不用另外設定（只掛在 App 開的會話，不動 `~/.claude/settings.json`）。會話列的標題是 Claude Code 自己取的對話標題，
  在等你或做完沒看時多一行最後回覆；小點紅＝要你允許／回答、亮綠＝做完沒看、暗綠＝看過了；Dock 圖示的數字＝這兩種的會話數。
- **額度餘光**：狀態列右下「5h 62%（23:10 重置） · 7d 31%」＝Claude Code 訂閱額度（App 開的會話每輪回報，不花額度），
  過 80% 變琥珀。活動分頁的健康段與 `ctx health` 另列「程式化呼叫」（碰撞判定、ask_repo、助手、backfill 用的 `claude -p`，
  走另計額度）次數；不想花就在 spine `config.yaml` 設 `provider: {programmatic: "off"}`。
- **有 agent 在跑就不讓機器睡**（macOS）：App 開的會話處理中時自動 `caffeinate -i`，都做完就收掉；
  不要的話在 `~/.config/repo-context/config.json` 加 `"keep_awake": false`。
- **在等你的會話直接回**：會話列紅點那列有「回答…」，打一句 Enter 就切到該會話並貼進輸入框（不送出，看一眼再按 Enter）；
  處理中的會話寫出正在用的工具與跑了幾分，超過 10 分沒結束標「可能卡住」。
- **過去的會話**：會話區「過去的…」列出這個範圍 Claude Code 談過的會話（標題、時間、分支、最後回覆），「接續」在原目錄開
  `claude --resume`，「複製指令」給外面的終端。終端裡用 `ctx sessions`。
- **看 agent 改了什麼**：右側欄「變更」頁籤（還沒 commit 的檔，點了看差異；「請 Agent 審…」先讓 agent 摘要風險）、
  檔案分頁上方「這組的變更」、終端標題列「查看變更」。差異裡滑過一行按「＋」留言，留好幾則再「貼給會話」一次送出。
- **repo 名片**：右側欄「資訊」頁籤可直接寫介紹／角色／關鍵字；活動分頁的健康段列出還沒寫的（可「請 Agent 起草…」）。
- **選單列「記一筆…」**：記到目前牌組的筆記（agent 開場也看得到）；待處理分頁「這組標為已讀」只清這組的未讀。

## 信任模型（資安）

- **登記的 repo 內容視為不受信任**：App 只顯示、不執行裡面的 md／html；但 agent 讀到其他 repo 的文字時仍可能被
  prompt injection 影響——只登記你信任的 repo。
- **spine 等於程式碼**：`config.yaml` 的 agent 指令與 `.mcp.json` 會在開會話時執行；不要直接使用別人的 spine。
- **App 的範圍＝登記的 repo**：檔案檢視、編輯、終端都限制在登記的 repo 裡；不能把根目錄或家目錄登記成 repo。
- 細節與寫程式的規則見 `docs/DEVELOPMENT.md` §8。

## 從 Python 版切換

切換前的檢查清單：

1. `which -a ctx`：確認 `~/.cargo/bin/ctx`（Rust）排在 Python 的 `ctx` 前面，或把 Python 版 `pip uninstall repo-context-workbench`。
   注意：引擎找 `ctx` 的順序是「自己 → 同目錄 → PATH」。桌面 App 自帶 ctx，所以 App 不受影響；但你在終端機打的
   `ctx`、以及 `ctx setup --claude` 註冊給 Claude Code 的 MCP／hooks 路徑，取決於你跑的是哪一支——用 Rust 版跑 setup。
2. 先停掉正在跑的 `ctx timer`／`ctx app`（Python 版）與開著的 agent 會話，避免兩版同時寫同一個 spine 的排程。
3. spine 先 `git commit`（或備份），切換後有問題可回退。
4. **移除舊的 MCP 註冊**：`claude mcp remove repo-context -s user`——`ctx setup` 看到已註冊會略過，不會自動換成 Rust 版。
5. **移除舊的 hooks**：`~/.claude/settings.json` 裡 `SessionStart`／`SessionEnd` 中指令含 `repo_context hook` 的兩筆刪掉——
   `ctx setup` 把 Python 版的 hook 也認成「已掛」，不刪就不會換成 `ctx hook …`。
6. spine 裡已有的 `.mcp.json`（`mcpServers.repo_context` 指向 `python -m repo_context`）刪掉，讓 `ctx session` 重寫——
   只要已有 `repo_context` 這個 server，Rust 版就不改寫。

然後：

```bash
ctx setup --spine <spine> --claude --dry-run   # 先看會做什麼
ctx setup --spine <spine> --claude             # 用 Rust ctx 掛 MCP（user scope）＋hooks＋skill，記住 spine
claude mcp get repo-context                    # 確認指令是 <…>/ctx mcp
```

`ctx ui`（瀏覽器模式）在 Rust 版停用，改用 `ctx app`。其他刻意差異見 `docs/RUST_PORTING.md`「刻意差異」。

## 常用指令

`--spine` 可省略：依序找 `$REPOENGINE_SPINE` → `ctx setup` 記住的 → 目前目錄。每個查詢指令都可加 `--json`。

```bash
# 開始用
ctx init ~/repo-spine                                  # 建 spine repo（桌面 App 首次打開也能建）
ctx setup --spine ~/repo-spine --claude --dry-run      # 記住 spine＋掛 MCP（user scope）＋hooks＋skill；先看會改什麼
ctx registry add my-repo ~/Coding/my-repo              # 登記；registry scan <目錄> --apply 批次登記
ctx group add 產品 my-repo,my-notes                     # 建組（add-member／remove-member／rename／remove）
ctx registry describe my-notes --role 知識庫 --description "書摘與主題閱讀" --keywords 書摘,閱讀
                                                       # repo 名片：給 agent 開場的組員名冊與搜尋用（存 spine，不寫進 repo）
ctx registry relate my-pm my-code --kind pm-of         # 關係：A 是 B 的 PM；registry suggest 看建議
ctx app                                                # 桌面工作台
ctx instructions                                       # 給沒有 skill 機制的 agent：貼進 AGENTS.md

# agent 的節奏（在任何已登記 repo 裡）
ctx where                        # 我在哪：repo／組／主場
ctx context                      # 開工脈絡：目標／上次交接／下一步／未結／最近事件
ctx next                         # 下一步（每項附依據）
ctx log "結論…依據…" [--supersedes <ref>] [--reopen-when "…"] [--outcome-of <ref>]
ctx todo add "要做的事" ／ close '#3' --note "…" ／ review ／ brief '#3'
ctx status waiting "在等使用者決定 X"
ctx handoff --done "…" --next "…" --remaining "…"
ctx sessions [--query 快取]      # 這個範圍過去的 Claude Code 會話（標題、最後回覆、claude --resume 指令）

# 跨 repo 與知識
ctx search "快取失效"             # 知識 ↔ repo 相關度＋過去的判斷
ctx registry export notes 書摘 books/ --note "逐章書摘"   # 宣告 export（repo 的 API）
ctx read notes:書摘/ch01.md       # 讀別的 repo（附 commit、記引用）；ask 開唯讀 agent 問（慢、花錢）
ctx repo [<id>]                   # repo 狀態卡：上次看過後的變化
ctx refs check                    # 失效的跨 repo 路徑＋上游變動
ctx fresh [--run]                 # 反芻：別組新進的知識／判斷中，跟本組手上工作相關的

# 回顧與維護
ctx health                        # 交接率、推薦命中率、判斷結果記錄率、未結、名片覆蓋；skill 不是這版的會提醒
ctx decisions export docs/agdr    # 判斷寫成 AgDR md
ctx backfill scan|run|review|accept   # 新 spine 冷啟動：從過去的 Claude Code 會話撈判斷／待辦／交接
ctx pulse --group 產品            # 活動脈動；brief／collide／spawn／timer 等原語見 ctx --help
```
