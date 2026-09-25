# Repo Context Workbench — repo-context 工作台

> **這是什麼**：把一群 repo 分成組，每組組成一份 context——餵給 agent，也餵給自己。
> 三層＝**引擎**（P1–P16 原語，套件 `repo_context`）＋**工作台**（桌面殼，`ctx app`）＋**脊椎**（留痕資料，住 spine repo）。
> **來歷**：2026-09-01 起在 `AgentCodingPM/repo_engine_prototype` 以 build-to-delete 原型開發，2026-09-11 獨立成本 repo（原型 15 個 commit 的歷史一併搬來）。
> 規格＝`docs/20260901_工具開發規格_v2.md`（§9 開放題 1「工具命名」已於本次搬家裁定：工具 repo-context／殼 工作台／指令 `ctx`）。
> **驗證設計先於程式碼**：見 `VERIFICATION.md`（四層驗證＋七條不變式＋回饋迴路）。

## 範圍（2026-09-01 拆機落地輪後：P1–P16 全數落地＋外部工具設計課補完）

> 拆機落地輪＝把 `docs/20260901_外部工具比對_逐一介紹.md` 點名「可抄的設計決策」
> 而原型尚缺的五項補上：gitpane 欄位（worktree＋agent 偵測）、mani 掃描混合模式、
> Repomix 的 token 預算函數與 Secretlint 洩密哨兵、second-brain 的 lint 衛生迴圈。
> 全部是 v2 既有原語（P1/P3/P6/P11）的延伸，未開新子系統。

- **有**：P1 registry+audit+**scan**（mani「init 自動掃描＋手寫補語意」混合模式）｜P2 組（含臨時組合）｜P3 採集（欄位抄 gitpane 補完：worktree 數＋**agent 偵測**）｜P4 上游（gh api，無事不報）｜P5 增量 digest（便宜模型）｜P6 打包簡版＋**estimate 預算函數**＋**洩密哨兵**（Repomix/Secretlint 課）｜P7 兩段式碰撞｜P8 落｜P9 生（升格＋出生登記＋outcome 回連）｜P10 脊椎三件套｜P11 查詢＋品味量測（命中率＋存活率）＋**lint 衛生迴圈**（second-brain 課）｜P12 排程（timer 住殼內、當日補課跨日不補）｜P13 通知（interrupt 白名單分類）｜P15 provider（mock＋claude，judge＋自由文字）｜P16 會話（帶料開終端＋MCP 掛載）｜**MCP server**（stdio 零依賴，agent 的介面＝人的介面）｜早晨簡報產生器（樣貌 A 純事實版）｜S4 監控台網頁雛形（`ui`，有 schedule 時內建 S6 timer）。
- **S4 是工作台桌面 APP，三個一級物件＝牌／收件匣／會話**（2026-09-01 裁定要 IDE 風格＋內嵌 terminal；2026-09-02 UI 檢討後重排，見 `docs/20260902_工作台UI_UX檢討.md`）：`app` 子命令用 pywebview 開原生視窗——**牌**（組是卡，點卡切範圍、展開勾成員；「＋臨時組」對話框挑一手牌）｜**收件匣**（統一卡片：准打斷／需要你判斷／處理中／碰撞回程／未結 loops，**每個出口都是真按鈕**→呼叫引擎原語→落 chosen→卡消失；碰撞台輸入框置頂）｜**會話**（sidebar 清單帶來源與任務摘要 title＋底部 terminal 面板：xterm.js＋PTY＋手寫 RFC6455 WS，會話生命週期獨立於視窗，沒會話自動收合）｜深看（全量表＋最近事件）。前端＝`repo_context/workbench.html`＋`repo_context/static/workbench.css`＋`repo_context/static/workbench.js`（2026-09-08 第四輪拆檔）。`ui` 瀏覽器模式為過渡替代。啟動隨機 token 防 VDI 多使用者（127.0.0.1＋URL token）。
- **組為單位輪（2026-09-08）**：① **關係層**——`registry relate A B --kind pm-of`（A 是 B 的 PM；詞彙 pm-of／feeds／derived-from／
  upstream-of／sibling-topic，config `relations.kinds` 可擴充），CLI／MCP／深看「關係」段三入口同源，臨時組〔＋相關 repo〕一鍵帶鄰居；
  ② **活動脈動**——採集加 7d／30d commit 數＋最近 5 則主旨，`pulse` 子命令、牌組卡 ⟳ 徽章、深看「近期 commit」表、簡報「脈動」行，
  audit 補反向 tier 漂移（dormant 卻有新 commit）；③ **組管家**——會話料改「組情境卡＋文件地圖」（成員角色含關係／脈動／本組未結／
  本組最近事件／角色說明），`group context`、MCP `group_context`、牌組卡〔組會話〕；沒給任務預設「3 行現況再問我」。
  設計：`docs/20260908_組為工作單位_Agent融入與關係設定_v1.md`。
- **UI 第四輪（2026-09-08，成熟度檢討落地）**：① **範圍契約**——每筆事件／卡片帶歸屬（repo → 組 → 碰撞回程繼承開場 → 判不出標「未分類」，
  不默默歸目前組），切組後待處理／活動／關係一致，跨組警示獨立「全域警示」段，「所有組標為已讀…」顯示影響則數；空 `repos:[]` 後端拒收。
  ② **可信狀態**——送出成功才清草稿（草稿本機保留）、斷線紅點＋橫幅保留上次資料、空狀態四種、靜默行用 `scan.quiet`、動作執行中鎖鈕。
  ③ **工作區重排**——WORKSPACE 標題＋一句摘要＋「開啟組會話」主鈕，四分頁（待處理／Repo／活動／關係），卡片一主一次＋「更多」，
  原生 `<dialog>`（Esc／焦點回觸發鈕），Repo 表 6 核心欄＋進階欄，活動依事件時間排序，「記錄結果」落 decision（不是 outcome）。
  檢討：`docs/20260908_工作台UI_UX成熟度檢討.md`；落地記錄同名 `_落地記錄.md`（含截圖）。
- **Agent 友善輪（2026-09-25）**：agent 從任何 repo 打開都接得上——① `agentapi.py` 工作層動作
  （where_am_i／context_for／next_work／log_decision／add_todo／close_todo／report_status／handoff／search_knowledge），
  MCP 預設只開這 9 個（JSON 回傳、結構化錯誤），維護工具移到 `ctx mcp --admin`；② 主場依 cwd 推出，寫到別組被拒（`cross_scope`），
  使用者同意才放行；③ Claude Code hooks：SessionStart 注入脈絡、SessionEnd 記下沒交接的會話；④ CLI `--json`＋exit code＋skill；
  ⑤ `knowledge.py` 知識 ↔ repo 相關度（BM25、中文 bigram）；⑥ `ctx setup --claude` 一次掛好 MCP／hooks／skill。
  順手修掉中文檔名 md 被 `git ls-files` 跳脫而全部略過的舊 bug。記錄：`docs/20260925_Agent友善輪.md`。
- **跨 repo 參考輪（2026-09-25 同日第二輪）**：agent 在 A repo 要又快又準地拿到 B repo 的東西——
  ① **exports**：repo 宣告對外提供什麼（`registry export <repo> 書摘 books/`），agent 用名字
  `<repo>:<export>/子路徑` 引用、`ctx resolve` 換路徑，不再寫死 `../x/y`；關係可標用到哪些 export；
  ② 四層由便宜到貴：開場注入**鄰居地圖** → `search_knowledge`（標題加權、鄰居加分、why、程式碼 git grep）
  → `read_from`（附 commit）→ `ask_repo`（對方 repo 開唯讀 agent，只回結論＋引用）；
  ③ **引用紀錄**（`refs/cites.jsonl`，不進收件匣）→ 上游改了提醒下游（drift）、常被引用的目錄建議宣告 export；
  ④ **repo 狀態卡** `ctx repo <id>`：上次看過後的變化、改動集中的目錄、沒 commit 的檔與天數、
  正在跑的 dev server 與它啟動後改過的檔（要不要重開）、最後交接、失效的跨 repo 路徑；
  ⑤ `refs check`／`registry audit` 掃各 repo agent 文件裡指向其他 repo 卻已失效的路徑。
  記錄：`docs/20260925_跨repo參考輪.md`。
- **未做（VDI Day 0 smoke test 過了才蓋）**：真殼的常駐件——tray、全域 hotkey、OS toast、開機自啟；Windows terminal 需 pywinpty（未實測）。視窗內 badge＋瀏覽器通知是常駐件的等效替身。
- 對 v2 的小偏差：kv token 多一個 `id:`（collision 回連需要，v2 的 kv 清單為例示性）；pack 用內建文件層選料（正式版換 Repomix）。

## 快速開始

```powershell
cd RepoContextWorkbench
pip install -e .        # 裝完就有短指令 ctx；下面每個 `python -m repo_context` 都可換成 `ctx`
                        # （輸出已自行設為 UTF-8，不必再先設 PYTHONUTF8）

# 讓 agent 從任何 repo 都接得上（一次性）
ctx setup --spine D:\some\spine-repo --claude --dry-run   # 先看會做什麼
ctx setup --spine D:\some\spine-repo --claude             # 記住 spine＋掛 MCP（user scope）＋hooks＋skill
ctx instructions                                           # 給沒有 skill 機制的 agent：貼進 AGENTS.md

# 自我驗證（agent 每次改完必跑；等同 scripts/check.ps1）
python -m pytest tests -q -m "not agent"

# 建 spine repo → 登記 → 建組
python -m repo_context init D:\some\spine-repo
python -m repo_context --spine D:\some\spine-repo registry add my-repo C:\path\to\repo
python -m repo_context --spine D:\some\spine-repo registry add ext-lib C:\x --type external --upstream owner/lib
python -m repo_context --spine D:\some\spine-repo registry scan D:\Repo --apply  # 掃目錄批次登記（mani 混合模式；不加 --apply 只列候選）
python -m repo_context --spine D:\some\spine-repo group add g1 my-repo
python -m repo_context --spine D:\some\spine-repo group add-member g1 repo-x,repo-y   # 加入組員（repo 需已登記）
python -m repo_context --spine D:\some\spine-repo group remove-member g1 repo-x        # 移出組（登記保留；整個移出 registry 用 registry remove）
python -m repo_context --spine D:\some\spine-repo group rename g1 產品研究              # 改名（groups/<組>/ 的料跟著搬）
python -m repo_context --spine D:\some\spine-repo group remove g1                       # 刪組（repo 登記與 groups/<組>/ 的料保留）
# 工作台：牌組旁的 ✎ ＝ 同一套（改成員／改名／刪除）；MCP 管理層：group_update／group_remove
python -m repo_context --spine D:\some\spine-repo registry relate my-pm my-code --kind pm-of --note "PM 規劃 code"  # 關係：A 是 B 的 PM
python -m repo_context --spine D:\some\spine-repo registry relations my-code    # 站在 my-code 讀：PM 是 my-pm

# 日常
python -m repo_context --spine ... collect --group g1     # P3 採集
python -m repo_context --spine ... pulse   --group g1     # 活動脈動：7d/30d commit 數＋近期 commit 主旨（組的監控焦點）
python -m repo_context --spine ... group context g1       # 組情境卡（會話開場料同源；agent 用 MCP group_context 拿同一份）
python -m repo_context --spine ... upstream --group g1    # P4 上游（新 release 落 suggestion 事件）
python -m repo_context --spine ... digest  --group g1     # P5 增量摘要（水位線在脊椎裡）
python -m repo_context --spine ... brief   --group g1     # 早晨簡報（落 presented 事件）
python -m repo_context --spine ... collide submit "想法" --group g1          # fire-and-forget（detached）
python -m repo_context --spine ... collide submit "想法" --group g1 --wait --provider claude  # 同步真判定
python -m repo_context --spine ... spawn new-repo D:\path --origin collision:2026-09-01-a  # P9 升格
python -m repo_context --spine ... unread                 # 殼 poll 的同一來源
python -m repo_context --spine ... notify                 # P13：interrupt 優先的通知文字
python -m repo_context --spine ... query --today -v       # P11
python -m repo_context --spine ... query --stats          # 靈感命中率＋repo 存活率
python -m repo_context --spine ... registry audit         # P1 稽核紅字
python -m repo_context --spine ... lint                   # 脊椎衛生迴圈（ref 斷鏈/逾期 loop/碰撞無回程/dead-letter）
python -m repo_context --spine ... pack --estimate --group g1  # 挑的預算函數：先算 token 再挑
python -m repo_context --spine ... timer --once           # P12：跑一輪到期任務（schedule 見 config.yaml）
python -m repo_context --spine ... commit                 # git 單一提交者（批次）
python -m repo_context --spine ... session --group g1 --task "寫文"  # P16 帶料開 claude 終端
python -m repo_context --spine ... mcp                    # MCP server（stdio；.mcp.json 由 session 產生）
python -m repo_context --spine ... app                    # S4 工作台桌面視窗（牌／收件匣／會話＋內嵌 terminal）

# agent 的節奏（在任何已登記 repo 裡；都可加 --json）
ctx where                        # 我在哪：repo／組／主場
ctx context --json               # 開工脈絡：目標／上次交接／下一步／未結／最近事件
ctx next                         # 下一步（每項附依據）
ctx log "結論…依據…"             # 記判斷 → 回傳 ref
ctx todo add "要做的事"  ／  ctx todo close '#3' --note "…"
ctx status waiting "在等使用者決定 X"
ctx handoff --done "…" --next "…" --remaining "…"
ctx search --file 某張卡片.md     # 知識 ↔ repo 相關度

# 跨 repo 參考（在任何已登記 repo 裡；都可加 --json）
ctx registry export notes 書摘 books/ --note "逐章書摘"   # 宣告 export（repo 的 API）
ctx registry relate notes course --kind feeds --export 書摘
ctx resolve notes:書摘/ch01.md    # 名字 → 路徑
ctx read notes:書摘/ch01.md [--lines 10-40]   # 讀＋附 commit＋記引用
ctx ask notes "快取失效怎麼處理？"            # 對方 repo 開唯讀 agent（慢、花錢）
ctx repo [<id>] [--since 7d] [--peek]        # 狀態卡（人看會記已讀；--peek 不記）
ctx refs check                    # 失效的跨 repo 路徑＋上游變動＋建議 export
ctx refs cite notes:書摘/ch01.md --note "複製進講義"   # 手動複製時記下版本
python scripts/screenshot.py <spine> --demo             # 改 UI 後先截圖自看（headless Edge；--demo 灌示範事件）
python -m repo_context --spine ... ui                     # 瀏覽器模式（app 的過渡替代）
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
repo_context/
  spine.py     P10 append（validator 拒寫＋跨行程 lock＋dead-letter）＋ P11 query/stats/unread
               ＋ lint 衛生迴圈（ref 斷鏈/逾期 loop/碰撞無回程/dead-letter 積壓）
               ＋ batch_commit（git 單一提交者唯一入口）＋ open-loop 預設 due
  registry.py  P1 registry+audit+scan（掃目錄自動登記下半身）＋ P2 組（含臨時組合）
               ＋ 存活率視圖＋關係層（relate/unrelate/relations_of/related_ids/relation_lines；
               詞彙 config 可擴充；audit 含關係斷鏈與反向 tier 漂移）；寫入同一把 lock
  collect.py   P3 git 狀態（欄位抄 gitpane：dirty 天數、末 commit 天數、ahead/behind、
               worktree 數、agent 偵測）＋活動脈動（commits_7d/30d、recent_commits、
               group_pulse／recent_across／pulse_line）
  context.py   組情境卡（成員與角色含關係／脈動／本組未結／本組最近事件／組管家角色說明，
               config session.role_prompt 可覆寫）——P16 會話開場料、CLI group context、MCP group_context 同源
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
  agentapi.py  agent 工作層（主場推定、範圍檢查、留痕／交接／狀態；MCP・CLI・hooks 同源）
  knowledge.py 知識 ↔ repo 相關度（BM25、中文 bigram、標題另建索引加權、boosts＋why、洩密檔不索引、mtime 快取）
  crossref.py  跨 repo 參考（resolve／read＋引用紀錄／drift／建議 export／鄰居地圖／程式碼 git grep／
               ask_repo 唯讀委派／agent 文件失效路徑 lint）
  repocard.py  repo 狀態卡（已讀水位線、目錄彙總、dirty 天數、正在跑的程序與啟動後改過的檔、交接、drift）
  hooks.py     Claude Code SessionStart／SessionEnd（注入脈絡、記下沒交接的會話）
  agentsetup.py ctx setup（使用者設定、claude mcp add、hooks 合併、skill 安裝）
  mcpserver.py MCP server（stdio JSON-RPC 零依賴；工作層預設、管理層 --admin）
  term.py      內嵌 terminal 的 PTY 會話管理（POSIX pty／Windows pywinpty；
               ring buffer replay，會話生命週期獨立於視窗）
  cli.py       全部原語的人用介面
  webui.py     S4 工作台（127.0.0.1＋啟動 token；/api/state 輕量輪詢、/api/scan 才打 git；
               interrupt 置頂＋通知 best-effort；schedule 時殼內起 timer；
               手寫 RFC6455 WS 供 terminal；serve_window()＝桌面視窗／serve()＝瀏覽器過渡）
  static/      vendor 前端資產（xterm.js 5.5.0＋addon-fit 0.10.0，MIT，離線可用）
```

鐵律對應：引擎零私有字樣（有測試釘住）；git 只有 `commit` 子命令與排程的 `commit` 任務會動（都走 `spine.batch_commit` 單一入口）；壞事件程式/MCP 入口回錯誤、人的入口落 dead-letter（想法永不丟）。
