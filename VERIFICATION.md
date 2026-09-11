# VERIFICATION.md — 原型的驗證與回饋設計（先於程式碼寫成）

> 本檔是 loop engineering 的骨架：**驗證方向先定，程式碼照著驗證長**。
> 規格上游：`docs/20260901_工具開發規格_v2.md`（以下簡稱 v2）。
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
5. **引擎無私有字樣**：`repo_context/` 原始碼 grep 不到私有專名（引擎/私有分離鐵律的可證偽版）。
6. **靈感命中率算得出來**（v2 M2 驗收③）：`spine query --stats` 能從 `collision`/`outcome` 事件算出命中率——用 fixture 資料手算對照。
7. **chosen 可回連 presented**（v2 M2 驗收①）：`chosen` 事件帶 `ref:presented:HH:MM` 時，query 能解回原事件。

## 回饋迴路（用起來之後）

- **2026-09-01 L4「▶ 沒 work」**→ 兩個根因都實錘：① claude `--mcp-config` 吃多值，
  空格形式把帶料 prompt 吞成設定檔路徑（Invalid MCP configuration）→ 改等號形式；
  ② hover 才現身的 ▶ 隱形時不吃 click（穿透到列上、零回饋），5 秒輪詢重繪再打斷 hover
  → 改永遠可見。另修 pack 改走 git ls-files 尊重 .gitignore（clone 森林 1551 檔擠占預算）。
- **2026-09-01 L4「開 Agent 時料太長」**→ 設計錯位：全文打包是給 P7 headless 判定的
  （沒工具只能餵全文）；P16 互動會話的 agent 自己會讀檔 → 會話料改 `pack_index`
  **文件地圖**（路徑＋行數＋首標題，大檔在前）——實測 AgentCodingPM 1854 行 → 19 行；
  prompt 明說「先讀地圖挑檔細讀，不要整包吞」。全文 pack_group 保留給 collide。
- **2026-09-01 L4「agent 功能開啟都是失敗的」**→ 根因＝（全部）範圍時 `resolve_group(None)` 炸
  「組不存在: None」（工作台預設範圍，claude/codex 都死在打包前；簡報鈕同雷）；
  先補 failing test（resolve_group None＝全部／build 無範圍／假 agent 全路徑）再修；
  順手加固：agent 會話經 **登入 shell（$SHELL -lc）** spawn——PATH/profile 不受工作台
  怎麼被啟動影響，command not found 會顯示在終端裡而不是無聲失敗。

- **2026-09-02 UI 檢討「沒有達到我要的效果」**→ 定性為**設計錯位不是 bug**：畫面是「監控頁黏 terminal」
  （`docs/20260902_工作台UI_UX檢討.md`）。三個根因＝行動出口是字不是按鈕（F1）、
  三種待辦格式疊加（F2）、自己丟的想法變成待忽略的未讀（F3）；會話無主（F5）。全部先補 failing test 再改
  （見下「UI 第三輪」），並新增 `scripts/screenshot.py`——**功能測試通過 ≠ 體感通過**，每輪 UI 改動先截圖自看再進 L4。
- **每一次「這則不該吵我／怎麼沒告訴我」**→ 改 `config.yaml` thresholds → 立刻補一條 L1 閾值測試釘住新手感。
- **每一次 L4 發現的 bug** → 先寫 failing test（L1 或 L2）再修——原型期就維持紅綠循環。
- **每一次真實碰撞的判定不準** → 改 prompt（`collide.py` 內）→ L3 重跑；判定標準的措辭本來就留給前三次真實碰撞調（v2 §9-2）。

## L4 驗收腳本（給人）

原型可看之後，依序做（預計 20 分鐘）：

1. `python -m repo_context init <某個空資料夾>` — 建 spine repo scaffold
2. 編 `config.yaml`／用 `registry add` 登記 3–5 個真實 repo，建一個組
3. `python -m repo_context collect --group <g>` — 看採集結果對不對（dirty/天數）
4. `python -m repo_context brief --group <g>` — 看早晨簡報 md：一屏內？問句形狀對嗎？
5. `python -m repo_context collide submit "一個真想法" --group <g> --provider claude --wait` — 看判定五欄位可信度
6. `python -m repo_context unread` → `spine query --today` — 以上動作全部留痕了嗎
7. 回饋收進本檔「回饋迴路」，決定要不要移植成正式 repo

## 原型範圍（對照 v2 里程碑）

- **做**：M1 引擎核心（P1 含 audit／P2／P3／P6 簡版／P8／P10 三件套／P11 陽春＋stats）＋ M2 借調（P7 兩段式、P13 閾值過濾與未讀、P15 provider 介面：mock＋claude）＋ brief 產生器（樣貌 A，無 LLM、純事實版——同時充當 M1 Day1-3 假簡報的產生工具）。
- **不做**：薄殼全部（tray/hotkey/toast/S6 timer）、P4 上游（留 stub）、P5 digest、P9 spawn、P12 排程、P14 監控台、MCP server。這些等移植正式 repo 再做。
- pack 的選料規則採 v2 §9-7 最簡版：**只撞文件層**（README／*.md），超預算按檔案由小到大裝、裝不下就截斷並在輸出標注。

---

## UI 增補（2026-09-01 第二輪：S4 監控台雛形）

**範圍修訂**：原「不做」清單中的監控台改為**做網頁雛形**（P14-lite + S2-lite 碰撞輸入框 + S3-lite 組切換）。
形態＝`python -m repo_context ui`：stdlib http.server 綁 127.0.0.1、瀏覽器開頁，零新依賴（v2 §9-8 耗材原則）。
tray／全域 hotkey／OS 通知仍不做（那是真殼的範圍，VDI smoke test 過了才蓋）。

**殼原則的落實（可測）**：
- `/api/state` 輕量輪詢（5 秒）＝純脊椎檔案讀——「殼只 poll 脊椎未讀」的網頁化
- `/api/scan` 才跑 P3 採集（開頁／切組／手動按鈕），不在輪詢裡打 git——高頻輪詢不碰 subprocess

**UI 驗證（併入四層）**：
- L1：`build_state`／`build_scan` 純函數測試（脊椎+registry → dict，欄位齊全）
- L2：HTTP E2E——thread 起真 server（port 0），urllib 打：
  `GET /` 200 且含關鍵區塊；`GET /api/state` JSON 欄位齊；`POST /api/collide`（wait=true）後脊椎多兩筆 collision；
  `POST /api/ignore` 落 `chosen` 事件（忽略是 chosen 的負形）；`POST /api/close_loop` 後 open loops 少一條；
  `POST /api/ack` 後未讀歸零；server 綁定必須是 127.0.0.1（不變式：不對外暴露）
- L4（人）：瀏覽器開頁——空狀態是不是「一切正常」一行字？碰撞送出即關（fire-and-forget）回程有沒有出現在未讀？深看表格對不對？

---

## 補完輪（2026-09-01 第三輪：引擎原語 P1–P16 補齊＋MCP server）

**範圍修訂**：原「不做」清單中的 P4 上游、P5 digest、P9 spawn、P12 排程、MCP server 改為**做**；
真殼（tray／全域 hotkey／OS toast）仍不做（VDI Day 0 smoke test 過了才蓋，監控台網頁＋瀏覽器通知為等效替身）。

**新驗收條款（全部已釘成測試）**：
- **P4**（`test_upstream.py`）：新 release 落 `suggestion [upstream]` 事件；**同一筆不重報**（無事不報的上游版）；
  kinds/prerelease 由 config 驅動；fetch 可注入（L1 不打網路）
- **P5**（`test_digest.py`）：首輪產摘要檔＋事件；**二輪水位線擋住**（一律增量——水位線就在脊椎裡）;
  agent 失敗落 system-unsure 事件（失敗必須浮出）
- **P9**（`test_spawn.py`）：升格建 repo＋registry origin 血統＋`outcome` 回連 → **靈感命中率從真資料算得出來**
  （M2 驗收③的自動化版）；incubator 素材搬離＝升格完成；非空目標拒絕且不弄髒 registry；
  存活率視圖（`registry.survival`）＝品味量測第二視圖
- **P12**（`test_timer.py`）：due 判定純函數——到點才跑、**當日補課、同日不重跑、跨日不補**；
  失敗浮出且不重試轟炸；`commit` 排程任務＝git 單一提交者的排程形態（走 `spine.batch_commit` 同一入口）
- **P13**（`test_notify.py`）：interrupt 白名單分類（body 含 system-unsure → 打斷；其餘未讀累積）；
  空狀態＝「一切正常」一行字
- **P16**（`test_session.py`）：指令組裝純函數——cwd=spine 靠 `.mcp.json` 自動掛載、cwd=某 repo 改 `--mcp-config`；
  `.mcp.json` 產生 idempotent
- **MCP**（`test_mcpserver.py`＋E2E stdio 往返）：tools 覆蓋全部原語；**validator 拒寫錯誤原樣回 agent**
  （isError，不落 dead-letter——dead-letter 是人的入口專屬）；notification 不回；stdio 真跑 initialize→list→call
- **open-loop 預設 due**：未帶 `due:` 的 opened 事件自動補 `openloop_default_due_days`（closed 不補）
- **webui**：`/api/state` 的 unread interrupt 置頂帶標記；stats 併入存活率；有 schedule 時 `ui` 殼內起 timer

---

## 工作台輪（2026-09-01 第四輪：「要 IDE 風格工作台＋terminal 跟 Agent 溝通，目前太像玩具」）

**裁定**：S4 從監控頁升級為 IDE 工作台（設計檔＝`docs/20260901_工作台設計_v1.md`）；
內嵌 terminal 取代 v2 §7「不內嵌 terminal」舊裁定。桌面視窗＝pywebview（前一輪裁定 ①）。

**新驗收條款（全部已釘成測試，98 tests）**：
- PTY 會話：echo 迴路｜晚到訂閱者拿 replay（會話生命週期獨立於視窗）｜kill 收乾淨（`test_term.py` L1）
- WS codec：RFC6455 官方向量＋frame 編解碼往返（含 70KB、中文）（L1 純函數）
- 真 WS 往返：HTTP 建 shell 會話 → 握手 → JSON 輸入 → PTY 輸出以 binary frame 回來（L2）
- token 防護：無 token 一律 403、/static 除外（VDI multi-session localhost 共享，檢討④）（L2 不變式）
- 簡報從 UI 產生且 presented 留痕；工作台頁關鍵區塊齊（碰撞台/事件流/簡報/深看/終端）（L2）
- L4（人）：`app` 開工作台 → ＋agent 會話直接跟 claude 對話、MCP tools 掛上；關視窗重開會話還在；
  瀏覽器實測已過（zsh 會話 echo 中文往返 OK），claude 會話與一週手感待人

---

## UI 第二輪（2026-09-01 L4 回饋：「無法在介面選取/去除 repo、看不到監控資訊」）

**回饋定性**：規格未錯——P2「臨時組合免建組」與 P14 深看全量表都在，是原型 UI 未實作；
另 `registry remove` 連 CLI 都缺（真缺口）。規格修訂建議（v3 候選）：repo 清單屬**操作面板**
（選取介面），監控四判準只管通知與事件流，操作面板進預設面不算違反「無事不報」。

**新驗收條款**：
- L1：`collide.submit(repos=...)` 落 `group:臨時(a,b)` token（照 §3.2 文法範例）；
  `run_judgement` 無 group 參數時能從事件的 臨時(...) token 解回 repos（detached 進程也拿得到料）；
  `registry.remove_repo` 移除 repo 並同步清掉所有組的 membership，不存在的 id 要報錯
- L2 HTTP：`/api/scan?repos=a` 只回選取的 repo 且問句同步過濾；`POST /api/collide {repos:[...]}` 可撞臨時組合；
  `POST /api/save_group` 把勾選存成命名組；CLI `registry remove` E2E
- L4（人）：預設面直接看到 repo 狀態表＋勾選框；取消勾選後「需要你判斷的」與碰撞範圍同步縮小；
  勾選可一鍵存成組

---

## 拆機落地輪（2026-09-01 第五輪：外部工具比對檔的「可抄設計決策」補完）

**範圍裁定**：依 `docs/20260901_外部工具比對_逐一介紹.md`，把報告點名
「可抄」而原型尚缺的五項設計課落地；全部是 v2 既有原語（P1/P3/P6/P11）的延伸，
與 v2 無新偏差（規格收斂檢查：未動分層、未動鐵律、未加依賴）。

**新驗收條款（全部已釘成測試，117 tests）**：
- **P3 gitpane 欄位補完**（`test_collect_brief.py`）：worktree 數（`git worktree list`，
  主 worktree 不算）；**agent 偵測**——工作台 spawn 的 agent 會話寫存活標記
  （`agentmark.py`，marker 落 `.state/agent_sessions/`），採集時 cwd 落在哪個 repo 就標誰；
  pid 已死的殘骸 marker 自動清（工作台被硬殺不留幽靈）；不帶 spine_dir 的舊呼叫路徑不變
- **P1 registry scan**（`test_registry.py`＋CLI E2E）：mani「init 自動掃描＋手寫補語意」
  混合模式——預設只列候選、`--apply` 才登記；下半身預設 mine/active，上半身留人；
  已登記不重列（冪等）、同名資料夾建議 id 加序號不覆蓋
- **P6 洩密哨兵**（`test_pack_route_collide.py`）：命中憑證樣式（private key／AKIA／
  ghp_／xox／generic secret）的檔案不進包、內容不出 repo，「疑似機密」段列名讓判定知情
- **P6 estimate 預算函數**（同上＋MCP）：per-repo token 成本先算再挑；estimate 合計
  ＝真打包全收錄的預算（兩個函數不說兩套話）
- **P11 spine lint 衛生迴圈**（`test_spine.py`＋CLI E2E）：ref 斷鏈／open-loop 逾期／
  碰撞無回程（opened 隔天判定沒回）／dead-letter 積壓四類紅字；乾淨脊椎零紅字；
  exit code 與 audit 同形態；工作台 `/api/scan` 的 audit 欄位＝registry 稽核＋spine lint
  併同一個紅字面
- **MCP parity**（`test_mcpserver.py`）：`registry_scan`／`pack_estimate`／`spine_lint`
  三工具入表——agent 的介面＝人的介面，新原語不例外

---

## UI 第三輪（2026-09-02：UI/UX 檢討落地——「牌／收件匣／會話」三個一級物件）

**觸發**：使用者裁定「沒有達到我要的效果」→ 先做檢討檔再動手。檢討方法＝用真 spine 跑起來截圖，對照
收斂檔 §7 操作單位／§8 四判準／素材檔 §2 Orca 判斷逐項核對。**診斷：監控頁黏 terminal，不是工作台。**

**引擎側（小改、結構化）**：
- `brief.build_question_cards`：問句改結構化卡 `{kind, qkind, repo, key, title, actions[{label, action, payload}]}`；
  `build_questions`／簡報 md 由 `card_text` 渲染而來，**文字版與按鈕版同源不分岔**。
- `webui.build_cards`：收件匣＝一種卡。順序 interrupt → 處理中 → 碰撞回程 → 其他未讀 → open loops。
  規則：自己出手的留痕（presented／chosen／monitor·route·spawn 的 decision）不進收件匣；
  被 `chosen ref:` 到的事件＝已處理，**ack 前就消失**；collision `opened` 未回程＝「處理中」卡（無出口），
  回程到了原地換判定卡；open-loop 事件只以 loop 卡呈現。
- 新 action（全部只是呼叫既有原語再落脊椎）：`tier`（set_field＋decision）、`defer`（chosen `snooze:<qkind> until:<date>`，
  scan 據此濾問句）、`remove_repo`、`route_collision`（P8 route＋chosen ref:collision）、`loop_defer`（同 # 新 due 再 opened）、
  `collide_rerun`；`ignore` 對判定卡改 ref `collision:<cid>`；`term_create` 帶 `origin`。
- `term.agent_title`：會話 title＝任務摘要（≤40 字），`TermManager.list` 帶 `kind/origin/scope/task`。

**前端**（`repo_context/workbench.html`，從 webui.py 內嵌字串抽出成檔）：
- 牌：組是卡（成員數＋異常數＋🤖），點卡切範圍、▶ 展開成員（勾選＝臨時子集、tier 點、icon 微標、▶ 開會話）；
  「＋臨時組」對話框（tag 篩選＋勾選＋可存成組）取代常駐 tag 篩選列與 13 個 checkbox。
- 收件匣：碰撞台輸入框置頂最亮；卡片依 kind 分段、出口是真按鈕（紫＝處理中含轉圈、藍＝碰撞回程含判定色）；
  「改落點…」卡內展開 select；空狀態一行「一切正常」＋「其餘 N 個 repo 無異常」。
- 會話：sidebar 清單（存活點、title、來源）＋ terminal 面板只顯示選中的會話；沒會話時面板自動收合。
- 簡報：分頁移除；「存成今日簡報」在 topbar，產出落 presented 並在收件匣下方抽屜顯示。
- 色彩語意表：紅只給准打斷；琥珀＝要判斷；藍＝資訊；綠＝正常；紫＝處理中。狀態列量測**有樣本才顯示**。
- 輪詢改 **key-based diff**（卡與段標題都有 key，內容 sig 不變不重繪；正在改落點的卡不被重繪打斷）。

**驗證（122 tests）**：
| 條款 | 測試 |
|---|---|
| 問句卡結構化＋文字版同源 | `test_collect_brief.py::test_question_cards_structured` |
| 處理中卡→判定卡；opened 不再是可忽略未讀；incubator 出口不重複 | `test_cards_collision_pending_then_judged` |
| 自己的留痕不進收件匣；chosen ref 到的卡 ack 前消失 | `test_cards_hide_own_records_and_handled` |
| interrupt 置頂；loop 卡三出口；open-loop 不重複成未讀 | `test_cards_interrupt_first_and_loops` |
| 問句卡出口＝term_create／defer；defer 7 天內不再浮出且留 chosen | `test_scan_question_cards_and_defer` |
| tier／route_collision／loop_defer 三個新 action 的 HTTP 全路徑與拒收 | `test_tier_action`／`test_route_collision_action`／`test_loop_defer_action` |
| 會話 title＝任務摘要 | `test_agent_session_title_has_task` |
| 頁面關鍵區塊（牌／收件匣／會話／臨時組／存成今日簡報） | `test_page_and_state` |
| 既有 flaky（09:00 事件在 09:00 前 ack 不掉）改用 now-1min | `test_close_loop_and_ack` |

**L4（人）**：真 spine 開 `app` → ① 三條問句能不離開畫面答掉（按下→卡消失→深看「最近事件」有 chosen）；
② 丟一個想法，看它從 ◐ 處理中變 ✔ 判定卡，按〔照建議落〕檔案真的落到建議位置；
③ 從判定卡〔深撞：開會話〕開的會話，sidebar 顯示「碰撞 <cid>」來源、title 是任務摘要；
④ 「＋臨時組」挑 3 個 repo → 範圍 pill 變「3 個 repo（臨時）」→ 問句／碰撞／會話帶料都跟著縮。
截圖自看：`python scripts/screenshot.py <spine> [--demo]`（headless Edge，30 秒）。

## 組為單位輪（2026-09-08：「預設工作單位是一組 repo」——agent 融入、活動脈動、關係設定）

**觸發**：使用者三個希望——① 預設 agent 要更融入「一組 repo」的工作方式（組內互動、資訊彙整更順）；
② 組的監控專注在**活動頻繁度與近期 commit 內容**；③ repo 間的**關係**要能很友善地設定（例：A 是 B 的 PM）。
設計檔：`docs/20260908_組為工作單位_Agent融入與關係設定_v1.md`。

**引擎側**：
- P1 關係層：`registry.relate/unrelate/relations/relations_of/related_ids`——關係存在來源 repo 的 `relations:` 欄
  （`{to, kind, note?}`，v1 §3.1 草案落地）；詞彙表 `RELATION_KINDS`（pm-of／feeds／derived-from／upstream-of／sibling-topic）
  可由 config `relations.kinds` 擴充；同一 (a,b,kind) 冪等；自指、未登記 id、未知 kind 拒寫；`remove_repo` 連帶清關係；
  audit 加 `[關係斷鏈]`。CLI `registry relate|unrelate|relations`；MCP `registry_relate`／`registry_relations`；
  工作台深看「關係」段＝列表＋新增表單＋✕，臨時組對話框〔＋相關 repo〕一鍵帶上一跳鄰居。
- P3 活動脈動：`collect` 每 repo 加 `commits_7d`／`commits_30d`／`recent_commits[{date,hash,subject}]`（最新 5 則）；
  `collect.group_pulse(states)` 組級彙總（7d/30d 總數、活躍／沉默 repo、最新一則）。`/api/scan` 帶 `pulse`；
  CLI `pulse`；牌組卡 ⟳7d 徽章、成員列 ⟳N 微標（hover 看最近 commit）、深看加「近期 commit」表；
  簡報加「脈動（一行）」段（樣貌 A v1.1，仍是資訊不是問句）。
- P16 組情境卡：新模組 `context.build_context(spine, group|repos)`＝成員與角色（含關係）＋脈動＋本組未結 loops
  ＋本組最近事件＋**組管家角色說明**（可由 config `session.role_prompt` 覆寫）。`session.build` 的會話料改為
  「情境卡＋文件地圖」同一檔，prompt 先讀情境卡；沒給任務時預設任務＝「3 行現況摘要再問我要做什麼」；
  開會話落 `presented [session] group:<g>`。CLI `group context`；MCP `group_context`；牌組卡〔組會話〕按鈕。

**不變式維持**：引擎零私有字樣（角色說明是通用措辭）；關係寫入走同一把 lock；presented 是自己的留痕不進收件匣；
無事不報——脈動只是一行資訊，不產生問句卡。

**驗證**：
| 條款 | 測試 |
|---|---|
| relate 冪等／拒自指／拒未登記／拒未知 kind；unrelate；relations_of 兩向標籤；related_ids 一跳 | `test_relations.py::test_relate_roundtrip_and_rejects`／`test_relations_of_labels_and_neighbors` |
| config 擴充 kind；remove_repo 清關係；audit 關係斷鏈 | `test_relations.py::test_custom_kind_from_config`／`test_remove_repo_strips_relations_and_audit_dangling` |
| collect 帶 7d/30d 計數與最近 commit；group_pulse 彙總 | `test_collect_brief.py::test_collect_activity_and_pulse` |
| 簡報有脈動一行且不增加問句 | `test_collect_brief.py::test_brief_has_pulse_line` |
| 情境卡：成員角色（含關係）、脈動、本組 loops、角色說明；config 覆寫角色說明 | `test_context.py::test_context_card_sections`／`test_context_role_prompt_override` |
| session.build 料檔含情境卡＋地圖，prompt 先讀情境卡，落 presented [session] | `test_session.py::test_build_includes_context_card_and_leaves_trace` |
| CLI relate/relations/context/pulse 全路徑 | `test_cli_e2e.py::test_relations_context_pulse_cli` |
| audit 反向 tier 漂移：dormant 但 7 天內有 commit 紅字、真沉默不報 | `test_registry.py::test_audit_dormant_but_recently_active` |
| MCP registry_relate／registry_relations／group_context | `test_mcpserver.py::test_relation_and_context_tools` |
| /api/state 帶 relations＋rel_kinds；relate/unrelate action；/api/scan 帶 pulse；頁面有「關係」「組會話」「近期 commit」 | `test_webui.py::test_relations_api_and_pulse`／`test_page_and_state` |

**L4（人）**：真 spine 開 `app` → ① 深看「關係」段設 `MI_PM pm-of MiCrawler`，牌展開 MI 組在 MI_PM 列 hover ⇄ 看得到「規劃（PM）→ MiCrawler」；
② 牌組卡看得到 ⟳7d 數字、成員列 hover ⟳ 看到最近 commit 主旨、深看「近期 commit」表按日期排；
③ 牌組卡〔組會話〕開 claude，第一句回覆是這組的 3 行現況（成員角色、活動、未結）而不是「請問要做什麼」；
④ 臨時組對話框勾 MI_PM 按〔＋相關 repo〕自動帶上 MiCrawler。
截圖自看：`python scripts/screenshot.py <spine> --demo`。

## UI 第四輪（2026-09-08：成熟度檢討落地——範圍契約／可信狀態／工作區四分頁）

**觸發**：檢討檔 `docs/20260908_工作台UI_UX成熟度檢討.md`——「骨架可用，但仍是工程原型感；缺口在工作焦點、
資訊取捨、狀態可信度」。P0 兩條已實際重現：切組後收件匣仍出現他組任務；失敗／等待／恢復狀態不完整。
落地記錄（含截圖）：`docs/20260908_工作台UI_UX成熟度檢討_落地記錄.md`。

**引擎側**：
- `webui.event_scopes(events, registry)`：每筆事件算歸屬 `{scope_kind, scope_label, scope_repos}`——`repo:` → repo、`group:` → 組
  （含 `臨時(a,b)`）、碰撞回程繼承開場事件、同 ref 同日的決策繼承被引用事件、開場本身無範圍＝`global`、判不出＝`unclassified`
  （**只讀 token，不從 body 猜**）。`build_cards` 與 `build_state.recent` 都帶歸屬；`recent` 改依 (date, time) 排序取 200 筆。
- `close_loop`／`loop_defer` 保留原事件的 `repo:`／`group:` token；`build_scan` 與 `_handle_action` 對 `repos: []` 一律拒收
  （「至少選擇一個 repo；空範圍不代表全部」），`/api/scan` 解析 query 保留空值以便拒收。
- 新 action `session_result`：把會話結果落 `decision`（`ref:collision:<cid>`＋`group:`／`repo:`／`臨時(...)`），**不是 outcome**。
  `term.create_agent` 加 `scope_repos` 供臨時組會話寫回歸屬。

**前端**（`workbench.html` 拆成 HTML＋`static/workbench.css`＋`static/workbench.js`，既有 `/static/` handler 服務）：
- 範圍契約：`scopedCards()` 只留 `scope_repos` 交集範圍的卡；跨組 interrupt 進「全域警示」段、其餘未分類進收合段；活動分頁同規則。
  「所有組標為已讀…」確認框顯示影響則數。
- 可信狀態：送出成功才清草稿（`localStorage` 保留）、`isComposing` 檢查；`stateError`／`scanError` → 頂列紅點＋橫幅；空狀態四種；
  靜默行用 `scan.quiet`；`busyActions` 鎖鈕；`scanVersion` 防舊回應蓋新範圍。
- 工作區：WORKSPACE 標題＋摘要＋「開啟組會話」；四分頁（待處理／Repo／活動／關係）＋搜尋＋類型篩選；卡片一主一次＋「更多」；
  原生 `<dialog>`（焦點限制／Esc／回觸發鈕）；「選擇 repo」對話框（篩選只影響顯示、零選取停用、取消不動 registry）；
  Repo 表 6 核心欄＋進階欄、可排序；牌組卡／會話列穩定節點更新；文案改白話（准打斷→需要立即處理 等）。

**驗證（146 tests）**：
| 條款 | 測試 |
|---|---|
| 碰撞回程繼承開場組；repo 事件歸 repo；無 token 的 interrupt 標未分類；route 後的 chosen 帶組 | `test_webui.py::test_event_scope_inherits_collision_and_keeps_unknown_unclassified` |
| 臨時子集歸子集；無範圍碰撞＝global | `test_adhoc_scope_and_global_collision` |
| loop 延期／關閉後歸屬不丟 | `test_loop_scope_survives_defer_and_close` |
| recent 依事件時間排序（回填不亂序） | `test_recent_sorts_by_event_time_not_append_order` |
| `repos: []` 在 collide／brief／term_create／scan 全部 400，且脊椎無新事件 | `test_empty_explicit_scope_never_expands_to_everything` |
| session_result 落 decision 帶 group、拒空文字與未知 sid、不產生 outcome | `test_session_note_records_result_without_claiming_adoption` |
| 頁面關鍵字改白話；`/static/workbench.css`／`.js` 可取得 | `test_page_and_state` |

**瀏覽器走查（Chrome 1280×860，暫存 spine，5 repo／2 組／seed_demo＋組事件）**：切組內容一致、對話框 Esc 與焦點、
四分頁、草稿保留、斷線橫幅——七項全過，console 無錯誤。截圖在 `docs/assets/20260908_ui_ux_implementation/`。

**未做（不宣稱）**：中文 IME 選字 Enter 實測、Windows/VDI pywebview、螢幕閱讀器、真 Agent 生命週期、完整 WCAG、
檢討 §5 第三輪、L4 真用（檢討 §5 的人工驗收任務）。
