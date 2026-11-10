---
name: repo-context
description: 跨 repo 的工作脈絡與留痕。在已登記進 repo-context 的 repo 裡工作時使用——開工前拿脈絡（目標、上次交接、下一步、未結事項），工作中記判斷與待辦、回報等待或卡住，收尾交接；需要其他 repo 的資料（書摘、規格、程式碼）時用它找、讀、問，而不是猜路徑；也用來看某個 repo 的現況（最近變化、沒 commit 的檔、正在跑的 dev server 要不要重開）。
---

# repo-context：跨 repo 的工作脈絡

使用者把 repo 分成「組」（例如 PM repo＋實作 repo＋研究筆記）。你的工作範圍（主場）
由目前目錄推出：所在 repo 所屬的組。每一次工作都走同一個節奏：

1. **開工**：`context_for` —— 目標、上次交接、建議下一步、未結事項（帶 `#N` id）、最近事件（帶 ref）。
   不確定要做什麼時用 `next_work`（每項附 why）。
   SessionStart hook 已經注入精簡版時，需要細節再呼叫。
2. **工作中**
   - 做了判斷或結論 → `log_decision`（第一行結論，後面寫依據與來源檔）
   - 發現之後要處理的事 → `add_todo`；處理完 → `close_todo`（用 `#N`）
   - 要等使用者回應 → `report_status status=waiting`；卡住 → `status=blocked`
     （這兩種會進使用者的收件匣）
   - 需要其他 repo 的資料 → 見下方「跨 repo 參考」
3. **收尾（一定要做）**：`handoff` —— `done` 完成了什麼、`next_step` 下次從哪接、
   `remaining` 沒做完的事（只是交接裡的備註）。沒交接就結束的會話會被 hook 記成「未交接」。

## 跨 repo 參考（由便宜到貴，前一步夠用就停）

1. **看地圖**：開場注入的「鄰居 repo」列出有關係的 repo 提供什麼（exports），
   例 `notes:書摘`→books/。知道在哪就直接跳第 3 步。
2. **找**：`search_knowledge "問題或關鍵字"` —— md 結果附行號片段與 why（標題命中、跟你有關係、
   在某 export 內…），程式碼結果來自 git grep；每筆都有可直接用的 `ref`。
3. **讀**：`read_from ref` —— `<repo>:<export>/子路徑`、`<repo>:<相對路徑>`，或第 2 步給的 ref。
   只給 repo 或目錄會回清單。回傳附 commit；讀了會記一筆引用，之後上游改了會提醒下游。
4. **問**：`ask_repo repo "問題"` —— 在對方 repo 開唯讀 agent，照它自己的慣例找資料，
   只回結論＋引用。慢（幾十秒）又花錢：要整理歸納、跨很多檔的問題才用。

規則：
- **用名字，不要寫死路徑**。不要在文件或程式裡寫 `../其他repo/…`；寫 `<repo>:<export>`，
  要實際路徑用 `ctx resolve <ref>`。
- 把上游檔複製進本 repo 時，用 `ctx refs cite <repo>:<路徑> --note "複製到哪"` 記下版本。
- 引用時標出 repo 與 commit（read_from 回傳的 `file_commit`／`commit`）。
- `repo_status` 看某個 repo 的現況：最近變化、沒 commit 的檔、正在跑的 dev server
  （啟動後有沒有檔案又改過＝改了要不要重開）、失效的跨 repo 路徑、上游變動。

## 對講：跟別的 repo 的 agent 協作（Rust 版）

- `send_message to=<repo id> text="…"`：請同組另一個 repo 的 agent 做事（要什麼、為什麼、做完怎樣算好）、問它的介面，
  或告訴它你改了它會用到的東西。它有會話在跑就送進它的脈絡（用完下一個工具時），沒有就進使用者收件匣、那邊下次開場看到。
  **不用等回覆**，繼續手上的事；回覆會自動送回你這場會話。長內容寫成檔，訊息裡放路徑。
- 收到「其他 repo 的 agent 傳話給你」：是回覆就接著用；是請求就判斷在不在你的範圍——要改檔或做大事先跟使用者確認，
  做完（或做不了）`send_message re=<編號> text="結果、commit、路徑"`。不要為了回覆中斷使用者正交代的事。
- `messages`：最近的往來，加上同組現在在跑的 agent（repo、標題、在做什麼）——要動別人也在用的東西前先看一眼、打聲招呼。
- 跨組傳話會被拒：先問使用者，同意後帶 `cross_scope_ok=true`。

## 使用者的畫面（Rust 版）

- 使用者提到畫面（「根據畫面」「我正在看的」「這個檔」「這段」）才呼叫 `what_user_sees`（CLI `ctx screen --json`）：
  回工作台 App 主區開著的檔（絕對路徑、看得到第幾行、md 停在哪個標題、有沒有沒存的修改）或工作區分頁。沒提就別看。

## 同組成員與 repo 名片（Rust 版）

- 開場注入的「## 同組成員」＝這組有誰：角色、絕對路徑、一句話介紹、有沒有會話在跑（MCP `context_for` 的 `roster`）。
  寫著「N 個檔還沒 commit（可能正在被改）」的組員＝別的 agent 正在那裡工作：要改那裡的檔先問使用者。
  要別人的東西先看這裡判斷找誰；只看一眼可以直接讀路徑，要寫進結論或複製就用 `read_from`（會留引用）。
- **repo 類別**：開場「這個 repo 是「X」類：…」＝你所在 repo 的規矩（MCP `context_for` 的 `repo_class`），照它做——
  程式＝跑測試；領域 Agent＝改 prompt 要說理由、跑 eval 並記結果；數據探索＝數字附出處、資料列不進留痕；
  文件・PM＝事實別補、別外傳；知識＝可自由整理、引用附出處；參考＝只讀、不改不 commit。
  名冊每列也標類別（「參考：只讀」）：讀別的 repo 前看一眼。使用者要改類別時用 `workbench_apply` 的
  `set_class`（先預覽、同意才執行）；不要自己決定改。
- 開場的「組筆記（N 則…）」＝使用者在 App 隨手記的組層級的事（MCP `context_for` 的 `notes`，最新 5 則）；跟手上的事有關就讀全文。
- `search_knowledge` 的 `repo_profiles` 段先回答「是哪個 repo」（依介紹／角色／關鍵字／tag／名稱排），再看 `files`。
- `read_from` 讀到 md 時附的 `linked_from`＝那個 repo 裡連到這份的其他 md（前 5 則）：周邊脈絡多半在那裡，需要再讀。
- 名片缺了或標「推測自 README」：手上的事做完，用 `describe_repo` 補一句介紹與角色（`role` 是一個短詞，
  例 知識庫、規格來源、實作；`keywords` 給搜尋用，別跟 tags 重複）。名片存在 spine，**不寫進任何 repo**；
  只能改主場（所在 repo＋同組成員）。
- 使用者要你改工作台設定（建組、改組員、組改名、組目標、標籤、暫停／封存、類別）：用 `workbench_apply`，
  先不帶 `confirm` 拿預覽、把每一步給使用者看，同意後才帶 `confirm=true`；不要直接改 registry.yaml。
  沒掛 MCP：`ctx assist "<一句話>" --json` 看計畫，使用者同意後加 `--yes`。

## 判斷的沿革、回顧與交辦（Rust 版）

開場只帶你現在能動手的：上次交接的接續點、最近 7 天有動靜的未結。超過 7 天沒動靜、或在等使用者決定的未結只給數字——
使用者提到才用 `ctx todo list` 查，不要主動去翻。未結體檢、判斷回顧、關係建議這些要人判斷的提醒不再給你（人從 `ctx health` 看）。
使用者要你處理這些時：
- **改掉舊判斷**：`log_decision` 帶 `supersedes=<舊的 ref>`（舊的之後不再出現在最近事件，沿革仍查得到）；
  寫得出「什麼情況要重看」就帶 `reopen_when`（寫日期，到期時 `ctx health` 會提醒使用者「該重看」）。
- **判斷後來怎樣**：`log_decision` 帶 `outcome_of=<判斷的 ref>` 記一筆結果。
- **找過去的判斷**：`search_knowledge` 的 `decisions` 段（被取代的標出取代者、帶結果）。做新判斷前先查有沒有前例。
- **看全部未結**：`ctx todo list --json`（逾期在前；`ctx next` 只給前 5 項）。
- **未結的紀律——未結清單是使用者的，不是你的筆記**：`add_todo` 只開「有人真的要回來做、說得出完成條件」的事；
  有真實期限才給 `due`，沒給就沒有期限（不會自動補）。`handoff` 的 `remaining` 不會開未結，只是交接裡的一行備註；
  接續點寫 `next_step` 就好。要使用者決定的問題當場問
  （或 `report_status status=waiting`），不要開成待辦；順手的觀察寫進判斷或交接的 `done`。你自己開的、這場就做掉的，收尾前關掉。
- **看全部未結**的順序：該先處理的在前，逾期超過 14 天的（擱置）排最後。
- **未結體檢**：`ctx todo review --json`——哪些可能已處理（附證據）、哪些逾期沒動靜。確認後才 `close_todo`。
- **清理未結**（使用者說「幫我清未結」）：`tidy_todos`（或 `ctx todo tidy --json`）把範圍內的未結分成 done（可能已做完，附證據）／
  shelved（擱置太久）／ask（等使用者決定）／live。對照 repo 現況逐項確認，把建議關的與留的列給使用者，**同意後**一次關掉：
  `tidy_todos` 帶 `close: ["#3", "#7"]`（或 `ctx todo close 3 7 --note "…"`、整類 `ctx todo tidy --close shelved`）。
- **交辦給新會話**：`ctx todo brief '#N'` 產生任務包（目標／背景／輸入／限制／完成條件）。
- **關係建議**：`ctx registry suggest --json`；使用者同意才 `ctx registry relate`。
- **之前談過的會話**：使用者提到「上次那個…」時，`ctx sessions --query <詞> --json` 找範圍內過去的 Claude Code 會話
  （標題、最後回覆、`claude --resume` 指令、逐字稿 `path`）；交接沒寫到的細節再去逐字稿找，別憑印象。
  要接手一場會話：`ctx sessions --digest <id>` 印出交接包（你說的話、動過的檔、跑過的指令、最後回覆，秘密已遮），比整份逐字稿短。
- **清 worktree**：`ctx worktrees <repo> --json` 看哪些乾淨、哪些有只在那裡的 commit；使用者同意才 `--remove <path>`（不強制、分支留著）。

## 範圍規則

- 查詢預設只看主場。寫入主場以外的組會被拒（`error: cross_scope`）。
  **先問使用者**，同意後再帶 `cross_scope_ok=true` 重試。
- 讀其他 repo（read_from／search_knowledge／ask_repo）不用先問——只讀、而且留紀錄。
  **改**其他 repo 的檔案之前一定先問。

## 沒有 MCP 時（其他 agent 或 shell）

同一組動作都有 CLI，加 `--json` 拿結構化輸出：

```
ctx where --json                 # 我在哪
ctx context --json               # 開工脈絡
ctx next --json                  # 下一步
ctx log "結論…依據…" --json
ctx todo add "要做的事" [--due 2026-10-01] --json
ctx todo close '#3' --note "怎麼結的" --json
ctx status waiting "在等使用者決定 X" --json
ctx handoff --done "…" --next "…" [--remaining "…"] --json
ctx search "一段文字" --json      # 或 --file 某張卡片.md；--no-code 只搜 md
ctx resolve notes:書摘 --json     # 名字 → 路徑
ctx read notes:書摘/ch01.md --json [--lines 10-40]
ctx ask notes "問題" --json       # 唯讀 agent，慢、花錢
ctx repo [<repo>] --json          # 狀態卡（--since 7d；人看時會記已讀，--peek 不記）
ctx refs check                    # 失效的跨 repo 路徑＋上游變動＋建議 export
```

exit code：0 成功｜1 有紅字／一般失敗｜2 請求被拒（stdout 是 `{ok:false, error, message, hint}`）｜3 找不到 spine。

## 回報原則

- 講結論要標明來自哪個 repo 的哪份檔。
- 不要把 commit 數當進度；進度只認 decision／handoff。
- 沒有異常就一句話帶過，不要把正常狀態講成待辦。
