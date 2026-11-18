---
name: repo-context
description: 跨 repo 的工作脈絡與留痕。在已登記進 repo-context 的 repo 裡工作時使用——開工拿脈絡、工作中記判斷與待辦、收尾交接；要別的 repo 的資料時用它找與讀，不要猜路徑。不在登記的 repo 裡就用不到。
---

# repo-context

只在登記進 repo-context 的 repo 裡有用（開場注入有「repo-context：你在 …」那段）。每次工作照這個節奏：

1. **開工**：開場注入已有上次交接、未結、同組成員，夠用就做事；要細節才 `context_for`，不知道做什麼用 `next_work`。
   沒有開場注入就先 `context_for`；回說這裡沒登記＝這個工具用不到，照平常做。
2. **工作中**：做了判斷 → `log_decision`；之後真的要有人回來做的事 → `add_todo`（說得出完成條件才開）；
   等使用者 → `report_status status=waiting`，卡住 → `status=blocked`；要使用者決定 → 先 `request_decision`。
3. **收尾一定 `handoff`**：`done` 做了什麼、`next_step` 下次從哪接；在等使用者決定就帶 `ask`。

## 規則

- 寫入主場以外的組會被拒（`cross_scope`）：先問使用者，同意後帶 `cross_scope_ok=true`。
- 讀別的 repo 不用問；**改**別的 repo 的檔一定先問。不要掃 `.git` 找 repo、不要寫死別的 repo 的路徑。
- 特定情境的做法（跨 repo 查資料、接手工作…）在開場的「工作台 skill」段，用 `skill_get` 讀；各工具的細節看工具自己的說明。
- 沒有異常就一句話帶過；進度只認判斷與交接，不認 commit 數。

## 沒掛 MCP 時

同名功能都有 CLI，加 `--json` 拿結構化輸出：`ctx <where|context|next|log|todo|status|handoff|search|read|repo|skill> --json`，
全部見 `ctx --help`。exit code：0 成功｜2 請求被拒（stdout 是 `{ok:false, error, message, hint}`）｜3 找不到 spine。
