---
name: repo-context
description: 跨 repo 的工作脈絡與留痕。在已登記進 repo-context 的 repo 裡工作時使用——開工前拿脈絡（目標、上次交接、下一步、未結事項），工作中記判斷與待辦、回報等待或卡住，收尾交接；也用來找其他 repo 裡跟目前主題相關的文件。
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
   - 需要其他 repo 的相關資料 → `search_knowledge`（回傳檔案、行號片段、repo 排名）
3. **收尾（一定要做）**：`handoff` —— `done` 完成了什麼、`next_step` 下次從哪接、
   `remaining` 新的待辦（各自開成未結事項）。沒交接就結束的會話會被 hook 記成「未交接」。

## 範圍規則

- 查詢預設只看主場。寫入主場以外的組會被拒（`error: cross_scope`）。
  **先問使用者**，同意後再帶 `cross_scope_ok=true` 重試。
- 範圍是寫入規則，不是檔案權限——讀寫組外 repo 的檔案之前一樣先問。

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
ctx search "一段文字" --json      # 或 --file 某張卡片.md
```

exit code：0 成功｜1 有紅字／一般失敗｜2 請求被拒（stdout 是 `{ok:false, error, message, hint}`）｜3 找不到 spine。

## 回報原則

- 講結論要標明來自哪個 repo 的哪份檔。
- 不要把 commit 數當進度；進度只認 decision／handoff。
- 沒有異常就一句話帶過，不要把正常狀態講成待辦。
