# repo_engine_prototype — 個人 repo 引擎原型

> **性質**：build-to-delete 原型，驗證 v2 規格（`../personal_agent_design/20260901_工具開發規格_v2.md`）的引擎層組得出來、驗證迴路轉得動。看過原型後才移植成獨立正式 repo；本資料夾不是最終家。
> **驗證設計先於程式碼**：見 `VERIFICATION.md`（四層驗證＋七條不變式＋回饋迴路）。

## 範圍（2026-09-01 拆機落地輪後：P1–P16 全數落地＋外部工具設計課補完）

> 拆機落地輪＝把 `../personal_agent_design/20260901_外部工具比對_逐一介紹.md` 點名「可抄的設計決策」
> 而原型尚缺的五項補上：gitpane 欄位（worktree＋agent 偵測）、mani 掃描混合模式、
> Repomix 的 token 預算函數與 Secretlint 洩密哨兵、second-brain 的 lint 衛生迴圈。
> 全部是 v2 既有原語（P1/P3/P6/P11）的延伸，未開新子系統。

- **有**：P1 registry+audit+**scan**（mani「init 自動掃描＋手寫補語意」混合模式）｜P2 組（含臨時組合）｜P3 採集（欄位抄 gitpane 補完：worktree 數＋**agent 偵測**）｜P4 上游（gh api，無事不報）｜P5 增量 digest（便宜模型）｜P6 打包簡版＋**estimate 預算函數**＋**洩密哨兵**（Repomix/Secretlint 課）｜P7 兩段式碰撞｜P8 落｜P9 生（升格＋出生登記＋outcome 回連）｜P10 脊椎三件套｜P11 查詢＋品味量測（命中率＋存活率）＋**lint 衛生迴圈**（second-brain 課）｜P12 排程（timer 住殼內、當日補課跨日不補）｜P13 通知（interrupt 白名單分類）｜P15 provider（mock＋claude，judge＋自由文字）｜P16 會話（帶料開終端＋MCP 掛載）｜**MCP server**（stdio 零依賴，agent 的介面＝人的介面）｜早晨簡報產生器（樣貌 A 純事實版）｜S4 監控台網頁雛形（`ui`，有 schedule 時內建 S6 timer）。
- **S4 是 IDE 風格工作台桌面 APP**（2026-09-01 兩輪使用者裁定；設計見 `../personal_agent_design/20260901_工作台設計_v1.md`）：`app` 子命令用 pywebview 開原生視窗——sidebar（組＋repo 勾選＋audit）｜事件流/簡報/深看分頁｜**內嵌 terminal 面板**（xterm.js＋PTY＋手寫 RFC6455 WS，shell 與帶料 agent 會話都開在這，會話生命週期獨立於視窗）｜狀態列量測。取代 v2 §7「不內嵌 terminal」舊裁定。`ui` 瀏覽器模式降為過渡替代。啟動隨機 token 防 VDI 多使用者（127.0.0.1＋URL token）。
- **無（留給正式 repo，VDI Day 0 smoke test 過了才蓋）**：真殼的常駐件——tray、全域 hotkey、OS toast、開機自啟；Windows terminal 需 pywinpty（未實測）。視窗內 badge＋瀏覽器通知是常駐件的等效替身。
- 對 v2 的小偏差：kv token 多一個 `id:`（collision 回連需要，v2 的 kv 清單為例示性）；pack 用內建文件層選料（正式版換 Repomix）。

## 快速開始

```powershell
$env:PYTHONUTF8 = "1"   # 必設（v2 §7：脊椎全中文，cp950 會炸）
cd repo_engine_prototype

# 自我驗證（agent 每次改完必跑；等同 scripts/check.ps1）
python -m pytest tests -q -m "not agent"

# 建 spine repo → 登記 → 建組
python -m repoengine init D:\some\spine-repo
python -m repoengine --spine D:\some\spine-repo registry add my-repo C:\path\to\repo
python -m repoengine --spine D:\some\spine-repo registry add ext-lib C:\x --type external --upstream owner/lib
python -m repoengine --spine D:\some\spine-repo registry scan D:\Repo --apply  # 掃目錄批次登記（mani 混合模式；不加 --apply 只列候選）
python -m repoengine --spine D:\some\spine-repo group add g1 my-repo

# 日常
python -m repoengine --spine ... collect --group g1     # P3 採集
python -m repoengine --spine ... upstream --group g1    # P4 上游（新 release 落 suggestion 事件）
python -m repoengine --spine ... digest  --group g1     # P5 增量摘要（水位線在脊椎裡）
python -m repoengine --spine ... brief   --group g1     # 早晨簡報（落 presented 事件）
python -m repoengine --spine ... collide submit "想法" --group g1          # fire-and-forget（detached）
python -m repoengine --spine ... collide submit "想法" --group g1 --wait --provider claude  # 同步真判定
python -m repoengine --spine ... spawn new-repo D:\path --origin collision:2026-09-01-a  # P9 升格
python -m repoengine --spine ... unread                 # 殼 poll 的同一來源
python -m repoengine --spine ... notify                 # P13：interrupt 優先的通知文字
python -m repoengine --spine ... query --today -v       # P11
python -m repoengine --spine ... query --stats          # 靈感命中率＋repo 存活率
python -m repoengine --spine ... registry audit         # P1 稽核紅字
python -m repoengine --spine ... lint                   # 脊椎衛生迴圈（ref 斷鏈/逾期 loop/碰撞無回程/dead-letter）
python -m repoengine --spine ... pack --estimate --group g1  # 挑的預算函數：先算 token 再挑
python -m repoengine --spine ... timer --once           # P12：跑一輪到期任務（schedule 見 config.yaml）
python -m repoengine --spine ... commit                 # git 單一提交者（批次）
python -m repoengine --spine ... session --group g1 --task "寫文"  # P16 帶料開 claude 終端
python -m repoengine --spine ... mcp                    # MCP server（stdio；.mcp.json 由 session 產生）
python -m repoengine --spine ... app                    # S4 工作台桌面視窗（IDE 風格＋內嵌 terminal）
python -m repoengine --spine ... ui                     # 瀏覽器模式（app 的過渡替代）
```

桌面視窗模式需要 `pip install pywebview`（引擎本體零 GUI 依賴；缺了會講人話退場）。

`--spine` 可用環境變數 `REPOENGINE_SPINE` 取代。

## 驗證分層（詳見 VERIFICATION.md）

| 指令 | 層 | 誰跑 |
|---|---|---|
| `python -m pytest tests -q -m "not e2e and not agent"` | L1 單元 | agent |
| `python -m pytest tests -q -m e2e` | L2 CLI E2E | agent |
| `REPOENGINE_RUN_AGENT_TESTS=1` ＋ `python -m pytest tests -q -m agent` | L3 真 claude 冒煙 | 人觸發 |
| VERIFICATION.md「L4 驗收腳本」 | L4 真實資料驗收 | 人 |

## 架構速覽

```
repoengine/
  spine.py     P10 append（validator 拒寫＋跨行程 lock＋dead-letter）＋ P11 query/stats/unread
               ＋ lint 衛生迴圈（ref 斷鏈/逾期 loop/碰撞無回程/dead-letter 積壓）
               ＋ batch_commit（git 單一提交者唯一入口）＋ open-loop 預設 due
  registry.py  P1 registry+audit+scan（掃目錄自動登記下半身）＋ P2 組（含臨時組合）
               ＋ 存活率視圖；寫入同一把 lock
  collect.py   P3 git 狀態（欄位抄 gitpane：dirty 天數、末 commit 天數、ahead/behind、
               worktree 數、agent 偵測）
  agentmark.py 工作台 agent 會話存活標記（P3 agent 偵測的資料源；pid 死了殘骸自動清）
  upstream.py  P4 上游（gh api release/commit；.state 記已見，同一筆不重報）
  digest.py    P5 增量摘要（水位線＝脊椎最後 digest 事件；便宜模型；失敗浮出）
  pack.py      P6 文件層選料（由小到大裝預算，未納入列清單）＋ estimate 預算函數
               ＋ 洩密哨兵（憑證樣式檔不進包——公私分界，Repomix/Secretlint 課）
  brief.py     樣貌 A 三段結構產生器（閾值過濾、>3 條問句浮出警語）
  collide.py   P7 兩段式（submit 即落脊椎 → detached 進程判定 → 未讀回程）
  route.py     P8（group materials／repo 01_inbox／incubator）
  spawn.py     P9 升格（建 repo＋出生登記 origin 血統＋outcome 回連命中率）
  timer.py     P12（S6 引信：due 判定純函數；當日補課跨日不補；失敗浮出）
  notify.py    P13（interrupt 白名單分類：system-unsure 打斷、其餘未讀累積）
  agents.py    P15 provider 介面（mock／claude；judge 五欄位驗證＋run_text；重試一次＋raw log）
  session.py   P16 帶料備會話（.mcp.json 產生；cwd 掛載或 --mcp-config）
  mcpserver.py MCP server（stdio JSON-RPC 零依賴；validator 錯誤原樣回 agent）
  term.py      內嵌 terminal 的 PTY 會話管理（POSIX pty／Windows pywinpty；
               ring buffer replay，會話生命週期獨立於視窗）
  cli.py       全部原語的人用介面
  webui.py     S4 工作台（127.0.0.1＋啟動 token；/api/state 輕量輪詢、/api/scan 才打 git；
               interrupt 置頂＋通知 best-effort；schedule 時殼內起 timer；
               手寫 RFC6455 WS 供 terminal；serve_window()＝桌面視窗／serve()＝瀏覽器過渡）
  static/      vendor 前端資產（xterm.js 5.5.0＋addon-fit 0.10.0，MIT，離線可用）
```

鐵律對應：引擎零私有字樣（有測試釘住）；git 只有 `commit` 子命令與排程的 `commit` 任務會動（都走 `spine.batch_commit` 單一入口）；壞事件程式/MCP 入口回錯誤、人的入口落 dead-letter（想法永不丟）。
