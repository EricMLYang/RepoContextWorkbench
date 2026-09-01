# VERIFICATION.md — 原型的驗證與回饋設計（先於程式碼寫成）

> 本檔是 loop engineering 的骨架：**驗證方向先定，程式碼照著驗證長**。
> 規格上游：`../personal_agent_design/20260901_工具開發規格_v2.md`（以下簡稱 v2）。
> 原則：coding agent 先把 L1/L2 跑到全綠才交人；人只做 L3/L4（機器測不了的部分）。

## 驗證四層

| 層 | 誰跑 | 內容 | 指令 | 通過判準 |
|---|---|---|---|---|
| **L1 單元** | agent，每次改完必跑 | validator 文法、file lock、registry audit、組解析、閾值過濾、路由、pack 選料 | `python -m pytest tests -q -m "not e2e and not agent"` | 全綠 |
| **L2 CLI E2E** | agent，收工前必跑 | 以 subprocess 跑真 CLI（`PYTHONUTF8=1`），fixture＝temp 假 git repo 群＋temp spine repo；走完「init → register → group → collect → append → query → brief → collide(mock) → route → commit」全流程；產出的脊椎檔要能被 validator 重新 parse 回來（往返驗證） | `python -m pytest tests -q -m e2e` | 全綠 |
| **L3 真 agent 冒煙** | 人觸發、agent 執行 | 把 provider 從 mock 換成 `claude -p --output-format json`，跑一次真實 collide；驗內層 JSON schema 驗證與重試一次的路徑 | `python -m pytest tests -q -m agent`（需 claude CLI 登入） | 判定五欄位齊、raw log 落檔 |
| **L4 人工驗收** | **只有這層請人** | 用真實 repo 資料跑一輪（見下方腳本）；體驗題：簡報一屏內讀完？問句≤3條？碰撞回程內容可信？ | 見「L4 驗收腳本」 | 人說 OK |

## 不變式（每條都有對應測試，測試名前綴 `test_inv_`）

v2 裡「現在不驗、三個月後才痛」的東西，全部釘成不變式測試：

1. **脊椎 append-only**：任何指令執行後，既有事件行的內容不得改變（E2E 裡對比執行前後檔案前綴）。
2. **不合格拒寫**：壞標頭經任何入口都寫不進 `spine/events/`；人的入口（`--dead-letter`）原文落 dead-letter 檔且未讀數 +1；程式入口回傳錯誤訊息（給 agent 自我修正）。
3. **git 單一提交者**：`append`/`route`/`collide` 等指令**絕不**產生 git commit；只有 `spine commit` 會（E2E 數 commit 數）。
4. **lock 範圍含 registry**：兩個行程同時寫（spine append × registry edit）不產生撕裂寫入（併發測試：多進程各寫 N 筆，事後全部 parse 得回來、registry YAML 仍合法）。
5. **引擎無私有字樣**：`repoengine/` 原始碼 grep 不到私有專名（引擎/私有分離鐵律的可證偽版）。
6. **靈感命中率算得出來**（v2 M2 驗收③）：`spine query --stats` 能從 `collision`/`outcome` 事件算出命中率——用 fixture 資料手算對照。
7. **chosen 可回連 presented**（v2 M2 驗收①）：`chosen` 事件帶 `ref:presented:HH:MM` 時，query 能解回原事件。

## 回饋迴路（用起來之後）

- **每一次「這則不該吵我／怎麼沒告訴我」**→ 改 `config.yaml` thresholds → 立刻補一條 L1 閾值測試釘住新手感。
- **每一次 L4 發現的 bug** → 先寫 failing test（L1 或 L2）再修——原型期就維持紅綠循環。
- **每一次真實碰撞的判定不準** → 改 prompt（`collide.py` 內）→ L3 重跑；判定標準的措辭本來就留給前三次真實碰撞調（v2 §9-2）。

## L4 驗收腳本（給人）

原型可看之後，依序做（預計 20 分鐘）：

1. `python -m repoengine init <某個空資料夾>` — 建 spine repo scaffold
2. 編 `config.yaml`／用 `registry add` 登記 3–5 個真實 repo，建一個組
3. `python -m repoengine collect --group <g>` — 看採集結果對不對（dirty/天數）
4. `python -m repoengine brief --group <g>` — 看早晨簡報 md：一屏內？問句形狀對嗎？
5. `python -m repoengine collide submit "一個真想法" --group <g> --provider claude --wait` — 看判定五欄位可信度
6. `python -m repoengine unread` → `spine query --today` — 以上動作全部留痕了嗎
7. 回饋收進本檔「回饋迴路」，決定要不要移植成正式 repo

## 原型範圍（對照 v2 里程碑）

- **做**：M1 引擎核心（P1 含 audit／P2／P3／P6 簡版／P8／P10 三件套／P11 陽春＋stats）＋ M2 借調（P7 兩段式、P13 閾值過濾與未讀、P15 provider 介面：mock＋claude）＋ brief 產生器（樣貌 A，無 LLM、純事實版——同時充當 M1 Day1-3 假簡報的產生工具）。
- **不做**：薄殼全部（tray/hotkey/toast/S6 timer）、P4 上游（留 stub）、P5 digest、P9 spawn、P12 排程、P14 監控台、MCP server。這些等移植正式 repo 再做。
- pack 的選料規則採 v2 §9-7 最簡版：**只撞文件層**（README／*.md），超預算按檔案由小到大裝、裝不下就截斷並在輸出標注。
