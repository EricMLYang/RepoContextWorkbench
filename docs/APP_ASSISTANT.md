# App 助手：用一句話操作整個工作台（設計）

> 2026-10-15。狀態：第一版已實作（2026-10-15：引擎 `assist_plan`／`assist_log`、⌘K「?」請助手、計畫對話框）；MCP `workbench_apply` 還沒做。GUI 未實測。來源：使用者需求「對整個介面的 agent 對話框，可以做整體 App 設定操作（換群組、設 tag…）」。

## 1. 結論

**做得到，而且不用另開一套權限或 API。** App 裡改設定的每個動作，都已經是引擎的 action（`save_group`、`update_group`、`tag`、`tier`、`set_goal`、`reorder`、`describe_repo`…），前端只是呼叫它們。助手只需要把一句話翻成「一串 action＋參數」，**給人看過、按確定**，再走同一組 action 執行。這剛好符合三條取捨準則：

- **同介面**：助手、按鈕、（之後的）MCP 走同一組 action，不會出現「只有助手能做」或「助手繞過檢查」的路。
- **留痕**：每個 action 本來就有留痕（registry 的變更記 decision、標籤進 registry）；助手只多記一筆「誰用哪句話、執行了哪些步驟」。
- **更快拿到對的脈絡**：「切到 PM 那組、把 VMS 加 tag 規格」不用知道按鈕在哪。

不建議的做法：讓一個常駐的 Claude Code 會話直接改 `registry.yaml`，或讓 agent 透過畫面自動點擊。前者跳過引擎的驗證與留痕；後者脆弱，也違反「GUI 自動化只鎖定受測 App」的約定。

## 2. 現況缺口

| 能力 | App 按鈕 | 引擎 action | MCP（給 agent） |
|---|---|---|---|
| 換目前範圍（牌組／臨時組） | 有 | 不需要（純畫面狀態） | 沒有 |
| 建組、改組員、改名、刪組 | 有 | `save_group`／`update_group`／`remove_group` | 沒有（管理層只有讀） |
| 標籤、分層（暫停／封存） | 有 | `tag`／`tier` | 沒有 |
| 組目標、名片 | 有 | `set_goal`／`describe_repo` | `describe_repo` 有 |
| 排序 | 拖曳 | `reorder` | 沒有 |

也就是說，**agent 今天改不了工作台的設定**，只有人能。助手補上這個缺口時，順便把同一份「可做的設定動作清單」開給 MCP，Claude Code 會話裡說「幫我把這三個 repo 編成一組」也能做。

## 3. 流程：一句話 → 計畫 → 人確認 → 執行

```text
⌘K 打字（或「助手」輸入框）
  └─ 「切到 PM Group，把 VMS_PM 加標籤 規格、從 Knowledge 移掉 old-notes」
       │ action assist_plan {text, scope}
       ▼
引擎：帶「現況快照」（組與成員、repo 與標籤／分層、目前範圍）＋「可用動作清單」問 provider
       │ 回 JSON：{說明, steps:[{op, args, 理由}]}
       ▼
引擎驗證每一步：op 在白名單、引用的 repo／組存在、參數型別對；壞的步驟標出原因，不丟掉整份
       ▼
App 顯示計畫：每步一行人話（「VMS_PM：加標籤『規格』」），可逐步取消勾選；危險步驟標紅
       │ 人按「執行 N 步」
       ▼
前端依序呼叫原本的 action（畫面類步驟如換範圍由前端自己做）；任何一步失敗就停，前面做過的列出來
       ▼
落一筆事件：presented [assist] 「指令：…／執行：…／略過：…」
```

**一律先確認、不自動執行**——即使只有一步。理由：自然語言常有歧義（「移掉」是移出組還是移出工作台？），而設定改錯的成本是使用者要自己找回來。確認畫面就是消歧義的地方。

## 4. 可用動作（白名單，第一版）

| op | 參數 | 對應 | 危險 |
|---|---|---|---|
| `switch_scope` | `{group}` 或 `{repos}` | 前端 `setScope` | |
| `open_tab` | `{tab}` | 前端 `showTab` | |
| `open_repo` | `{id}` | 前端 `showRepo` | |
| `create_group` | `{name, repos}` | `save_group` | |
| `set_members` | `{group, repos}` | `update_group` | 移掉成員時標紅 |
| `rename_group` | `{group, new_name}` | `update_group` | |
| `set_goal` | `{group, text}` | `set_goal` | |
| `tag` | `{id, add?, remove?}` | `tag` | |
| `set_tier` | `{id, tier, resume_when?}` | `tier` | 封存標紅 |
| `open_session` | `{repo? group? task}` | 開「新建 Agent 會話」對話框（不直接開） | |

第一版**不收**：`remove_group`、`remove_repo`（刪除類先只給按鈕）、`write_file`、任何碰 spine 事件內容的動作。之後要加，先看真用資料。

## 5. 分層與規則（照 AGENTS.md）

- **引擎**：`assist::plan(spine, text, scope) -> Plan`（組 prompt、呼叫 provider、驗證）。白名單、驗證、人話描述都在引擎，前端只畫與執行。provider 走既有的 `agents::provider_by_name`（測試用 `mock`）。
- **前端**：⌘K 加「請助手：〈打的字〉」一列（或 `?` 開頭）；計畫用既有的 detail 對話框；執行中鎖按鈕、失敗就地顯示、斷線保留計畫。
- **MCP**：同一個白名單開成管理層工具 `workbench_apply {steps}`（不經 LLM，agent 自己就是 LLM），同樣先回計畫、要 `confirm: true` 才執行。
- **成本與速度**：一次規劃約一個短 prompt（現況快照壓在 2–3 千字內）；provider 沒設定或失敗時，助手那一列寫原因，不影響 ⌘K 其他功能。
- **閒置不吃 CPU**：只在按下時呼叫，沒有背景輪詢。

## 6. 風險與還不知道的

- **模型把名字對錯**：快照裡給完整的 repo id 與組名，驗證時找不到就標出「找不到 VMS」並列出最接近的名字，不猜。
- **一句話做很多事**：上限 10 步；超過要人拆。
- **離線／沒裝 claude**：助手停用並說明；按鈕照常可用。
- **還不知道**：使用者最常想用一句話做的設定是什麼——先做第一版、看 `assist` 事件裡的指令再調白名單。

## 7. 建議下一步

1. 引擎 `assist_plan`＋白名單驗證＋mock provider 測試（不碰前端）。
2. ⌘K「請助手」→ 計畫對話框 → 依序執行。
3. MCP `workbench_apply`（同一份白名單）。
4. 用一週後看 `assist` 事件：哪些指令被取消、哪些步驟驗證不過，調整白名單與 prompt。
