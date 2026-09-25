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
   `remaining` 新的待辦（各自開成未結事項）。沒交接就結束的會話會被 hook 記成「未交接」。

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
