# Python → Rust 搬遷規約

> 2026-10-06 起在 `rust-desktop` 分支把 `repo_context`（Python）整份搬成 Rust＋Tauri＋React。
> 方向見 `AGENTS.md`。Python 版在搬完、對拍通過前**不刪**——它是規格，也是使用者正在用的 `ctx`。

## 版面

```
Cargo.toml                 workspace
crates/ctx-core/           引擎 lib（對應 repo_context/*.py，不依賴 Tauri、不依賴 CLI 框架）
crates/ctx-cli/            bin `ctx`：CLI（clap）＋MCP server（stdio）＋hooks 入口
desktop/                   Tauri v2 殼（src-tauri，依賴 ctx-core）＋React（Vite＋TS）前端
```

模組一對一：`repo_context/spine.py` → `crates/ctx-core/src/spine.rs`，`registry/` → `src/registry/`，
`webui/state.py`、`webui/actions.py` → `src/workbench/{state,actions}.rs`（給桌面 App 用，與 HTTP 無關）。

## 合約（不准改）

- spine 事件檔 `spine/events/YYYY-MM-DD.md` 的標頭文法、dead-letter 檔、`.engine.lock` 鎖檔協定（O_CREAT|O_EXCL＋stale 回收）。
- `registry.yaml`、`config.yaml` 的鍵與語意；寫回時**保留不認得的鍵**（Python 版寫什麼、Rust 版要讀得回，反之亦然）。
- `.state/` 底下各 JSON／txt 的路徑與欄位。
- 環境變數：`REPOENGINE_SPINE`、`REPOCTX_NO_BACKGROUND`、`XDG_CONFIG_HOME`；使用者設定檔 `~/.config/repo-context/config.json`。
- CLI 子命令、旗標、`--json` 輸出的欄位、exit code（0 成功／1 有紅字／2 用法錯或被拒／3 找不到 spine）。
- MCP 工具名、inputSchema、回傳 JSON 欄位、錯誤形狀 `{ok:false,error,message,hint}`。
- 給人看的中文訊息：測試有斷言的要逐字一致；其餘盡量照抄。

兩邊同時讀寫同一個 spine 是常態（搬遷期間全域 `ctx` 仍是 Python），所以「格式相容」比「Rust 寫法漂亮」優先。

## Rust 寫法

- 錯誤一律 `ctx_core::Result<T>`（`src/error.rs`）。Python 的 `ValueError` → `Error::Invalid`，
  `spine.ValidationError` → `Error::Validation`，`AgentUnsure` 之類外部失敗 → `Error::External`。
- 只有程式自己保證的不變式才 `unwrap`／`expect`；讀檔、解析、外部指令都要傳錯誤。
- 型別表達狀態（例：事件型別用 enum、tier 用 enum），但**資料檔的序列化字串與 Python 相同**。
- 可擴充的設定／registry 欄位用 typed struct＋`#[serde(flatten)] extra` 保留未知鍵；
  真的是自由形狀的才用 `serde_yaml::Value`／`serde_json::Value`。
- 時間：`chrono::Local`；凡是 Python 收 `when=`／`today=` 參數的函式，Rust 也收 `Option<NaiveDateTime>` 之類，方便測試。
- 外部指令（git／gh／claude）用 `std::process::Command`，參數與 Python 相同；不引入 git2。
- 依賴從嚴：要加 crate 先看 workspace `Cargo.toml` 有沒有；新增要有理由（寫在 commit message）。
- 每個 pub 函式第一行 doc comment 寫它對應哪個 Python 函式，例：`/// 對應 spine.append_event`。
- 保留 Python 檔頭 docstring 的「為什麼」（精簡成模組 `//!` 註解），不要只留「做什麼」。

## 測試

- 每個 `tests/test_<x>.py` 搬成 `crates/ctx-core/tests/<x>.rs`（CLI e2e 搬到 `crates/ctx-cli/tests/`），
  測試名稱照 Python 函式名，一個 Python 測試對一個 Rust 測試；搬不了的寫明原因（`// 未搬：…`）。
- 共用 fixture 在 `ctx_core::testkit`（feature `testkit`）：temp spine、假 git repo（照 `tests/conftest.py`）。
- 測試裡設 `REPOCTX_NO_BACKGROUND=1`，不碰使用者真的 `~/.config`（測試用 temp `XDG_CONFIG_HOME`）。
- 完成條件：`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all --check` 全過。
- CLI e2e（`crates/ctx-cli/tests/`）用 `env!("CARGO_BIN_EXE_ctx")` 跑真 binary；temp `XDG_CONFIG_HOME`、
  `REPOCTX_NO_BACKGROUND=1` 只設在子進程上，不改測試進程的全域環境。MCP 的單元測試在行程內呼叫
  `ctx_cli::mcp::handle_message`（ctx-cli 同時是 lib＋bin）。

## 對拍

`scripts/parity.sh`（bash 包一層，比較邏輯在 `scripts/parity.py`）：用 Python CLI 建一份 fixture
（init、3 個假 git repo、兩組、關係、export、決策／todo／handoff／碰撞、回填候選、一場 Claude Code 會話），
複製兩份，各跑 Python `python3 -m repo_context` 與 `target/debug/ctx` 的同一組指令，比較：

- exit code；
- stdout：`--json` 解析後比（浮點數容許小誤差——天數是現算的），人話逐字比；路徑換成 `<R>`、時間戳記
  （`YYYY-MM-DD HH:MM`、`HH:MM`、打包／dead-letter 檔名裡的時間）換成佔位符；
- 每個指令跑完後整個 spine 目錄（`.git` 除外）：事件檔、dead-letter、`registry.yaml`（解析成 YAML 物件比）、
  `.state/` 底下的 JSON（解析後比）、其他檔逐字比；
- MCP：同一串 JSON-RPC（initialize、notifications、tools/list、ping、未知 method／tool、工作層與管理層的
  tools/call，含錯誤）餵兩邊 stdin，逐則比回應（tool 回的 text 是 JSON 就解析後比）。

涵蓋所有不打外部網路／不呼叫真 claude 的子命令（provider 一律 mock；`upstream` 只測沒有 upstream 欄位的情形；
`ask`／`collide`／`digest` 走 mock）。沒涵蓋：`session`（不帶 `--print` 會開終端機）、`app`、`timer`（常駐）、
`backfill run`（呼叫 claude）、`ui`（已停用，Rust 測試另驗）。跨午夜跑可能因日期不同而失敗，重跑即可。

允許差異（表上列出、比較前只拿掉那一欄或那一檔，不是整個指令跳過）見下節。
2026-10-06 第一次跑：153 項（CLI 151＋MCP 工作層／管理層各一串）全過；用到的允許差異只有
`.agent_logs/`（provider 除錯 log）與 `.mcp.json`（MCP 指令改指 Rust ctx）。全過時自動刪 fixture，
要留著看設 `PARITY_KEEP=1`。
全部指令對拍通過才合併回 main、才把 hooks／MCP 改指向 Rust 版。

## 刻意差異（Rust 版與 Python 版行為不同之處）

引擎（ctx-core，各 agent 搬遷時回報）：

- **collect dirty_days 修正**：Python 版算 dirty 檔放了幾天時只看前幾個檔，會漏掉較舊的檔；Rust 看全部
  dirty 檔。對拍時 `collect --json` 的 `dirty_days` 列為允許差異（人話表格的「dirty N 天」也正規化後比）。
- **MockProvider 呼叫計數改成實例層**：Python 是類別屬性（整個行程共用，`REPOENGINE_MOCK_FAIL` 的計數跨呼叫累積）；
  Rust 每個 provider 實例自己算。`.agent_logs/` 底下的 provider 原始紀錄（除錯 log）不對拍。
- **agentsetup／session 掛的 MCP 與 hooks 指令改指 Rust ctx**：`ctx setup --claude` 註冊
  `claude mcp add … -- <ctx> mcp`、hooks 是 `<ctx> hook session-start|session-end`；`ctx session` 寫的
  `.mcp.json` 是 `<ctx> mcp`。已經有 `repo_context` 的 `.mcp.json` 不動（使用者可能還在用 Python 版）。
  `is_our_hook` 兩種都認，重跑 setup 不會重複掛。
- **collide.spawn_detached 尊重 `REPOCTX_NO_BACKGROUND`**：Python 版只有反芻的背景進程看這個變數；
  Rust 版碰撞判定的 detached 進程也不開（測試／對拍不會留下非同步寫入）。
- **timer.run_loop 睡到下一個排程點**（最多 300 秒重看一次 config），不是固定每 30 秒 tick，閒置不吃 CPU；
  可由 channel 停止。
- **term kill 先送 SIGHUP＋SIGTERM**（給 shell／agent 收尾的機會），逾時才 SIGKILL。
- **SessionEnd hook 補一次 spine 批次 commit**（2026-10-09，ideation N-01）：timer 住在桌面 App 裡，App 沒開時
  21:00 的排程 commit 不會跑。`spine::autocommit`：spine 已有 git 歷史、有變更、上次 commit 超過
  `thresholds.spine_autocommit_hours`（預設 6；0＝關）才 commit；分支有 upstream 就背景 `git push`。
  一個 commit 都沒有的 spine 不動（不替使用者開始 git 歷史）。`batch_commit` 改成持 `.engine.lock` 才 add＋commit。
- **陳舊會話 marker**（N-16）：開場時把開了超過 7 天、沒收到 SessionEnd 的 marker 當成已結束——照 SessionEnd 的規則
  判斷（有事發生又沒交接才記一筆，理由寫「超過 7 天沒有收到結束訊號」），然後清掉。門檻保守，會話可能一開好幾天。
- **hook 沒有 session_id 不記 marker**（N-14）：Python 寫成 `hook_sessions/unknown.json`；Rust 照樣輸出注入內容，
  但不記會話（手動試跑、量測不會留下假會話，之後也不會被記成「未交接」）。
- **知識索引快取**（N-02，格式不變）：不展開成整棵 JSON 樹，只解析用到的檔；沒有新算的檔就不重寫快取
  （Python 每次查詢整份重寫）；常駐行程（MCP、App）記住上次讀到的快取；列檔與逐檔解析分給多條執行緒；
  `serde_json` 開 `float_roundtrip`（預設的浮點解析不精確，mtime 對不上 → 曾有 631 份檔每次查詢都重算）。
  程式碼搜尋（`git grep`）各 repo 同時跑。真資料 `ctx search` 2.1–3.0s → 約 0.55s，結果與舊版相同。
- **反芻推送降噪**（N-03）：共同詞不收純數字／日期；中文 bigram 要「像一個詞」——含它的文件數 ÷ 含其中較少見
  那個字的文件數 ≥ 0.08（`CharStats::word_like`），擋掉「者決」「用後」這類跨詞邊界切出來的假詞；
  同一組同一份料還有沒處理的或被打掉過的推薦，檔案改了也不重推。
  2026-10-10 第二步：共同詞不收「幾乎每個 repo 都有」的一般用語（repo 數 ≥6 時，出現在超過三分之二的 repo；
  `CharStats::generic`——真資料「尚未」「交付」「等待」在 15 個 repo 裡出現在 14–15 個）；寫入前持鎖重看一次狀態、
  背景反芻先佔 `.state/ruminate.running`（並行開場曾寫出兩份一模一樣的推薦）；`fresh` 同一份料只列最新一筆。
  2026-10-10 第三步（N-20）：推薦多 `rules`（規則版本，目前 3；一般用語門檻收緊到「超過一半的 repo」，
  repo 分布只用 repo 裡的檔算）。開場（`context_for.fresh`）只推目前規則、`PUSH_DAYS`（7）天內找到的
  （`ruminate::push_links`）；舊規則的推薦視為被取代——不推、不擋新規則重推、資料保留（`ctx fresh --all` 看得到）。
  對拍時比對前拿掉推薦的 `rules` 鍵，記為允許差異。

- **未結事項體檢 `ctx todo review`**（2026-10-10，ideation F-21；Python 沒有）：每項看 opened 之後範圍內的 commit 與
  判斷／交接的「完成：」行，算證據蓋住事項文字的比例（中文用連續命中的片段，不數零散 bigram）；≥45%＝可能已處理，
  逾期或開了 14 天以上、期間沒人提到 `#N` 也沒相關動靜＝擱置。提到 `#N` 的只算「有人提到」不算證據，開出這項的交接
  （同一分鐘）也不算。只提議不自動關。SessionStart 注入在「未結」段尾多一行「體檢：…」（有可清的才出現）。
- **判斷取代＋何時重看**（2026-10-10，ideation F-07；Python 沒有）：`log_decision` 可帶 `supersedes`（`decision:<id>`，要存在）
  與 `reopen_when`。寫進 body 的結構化行 `取代：decision:<id>`、`何時重看：<條件>`，被取代的同時寫成 `ref:` token——
  標頭文法不變，Python 讀得回（只是不懂它的意思）。被取代的判斷不再出現在 `context_for` 的最近事件、反芻的手上工作
  與推給別組的判斷候選；帶重看條件的有效判斷列在 `context_for.reopen`（只在有的時候出現），開場注入「該重看」（條件裡
  的日期已到）／「有條件的判斷」各一行。MCP `log_decision` 多兩個選填參數（對拍時拿掉後比，記為允許差異）。
- **桌面 App 收件匣帶體檢結果**（2026-10-10，ideation F-46）：`review_todos` 每次算完併進 `.state/review.json`；
  `build_scan`（本來就跑 git 的重層）順便逐組更新；`build_state` 只讀這個檔（仍是純檔案讀），把「可能已處理」「擱置沒動靜」
  掛到未結卡的 `verdict`（卡片標籤）與 `lines`（描述＋證據），進行中的不加註。前端不用改。對拍時 `.state/review.json` 列為允許差異。
- **相關 repo 建議 `ctx registry suggest`**（2026-10-10，ideation F-33；Python 沒有，新模組 `relsuggest`）：
  主要訊號＝A 的 md 提到 B 的 repo id 或目錄名（≥3 份）；引用紀錄（≥2 筆）；只出現在這兩個 repo 的主題詞只當佐證。
  一個 repo 當主要提到者的建議 ≥4 筆＝推測它是 PM（`pm-of`）。只建議不寫入；已有關係的不列。
  反芻跑完順便算進 `.state/relation_suggest.json`，開場只讀檔、跟主場有關的提一行。對拍時這個快取列為允許差異。
  App（F-47）：`build_state` 多一個 `relation_suggestions`（碰到目前範圍、還沒設關係的；沒有建議時不給這個鍵），
  「關係」分頁列建議＋理由＋依據，有推測的「套用」、沒推測的「設定…」開預填表單、「不相關」＝新動作
  `relation_dismiss`（記在 `.state/relation_suggest_dismissed.json`，ctx `registry suggest --dismiss A B` 同一份）。
- **判斷可搜尋**（2026-10-10，ideation F-22；Python 沒有）：`search_knowledge`／`ctx search` 多一段 `decisions`
  （有命中才給）：範圍內的 decision（含交接）BM25，查詢詞先去含虛字的 bigram；被取代的打五折並標 `superseded_by`、
  帶 `reopen_when` 與之後的 `outcomes`（`ref:` 指回它的結果事件）；只留分數 ≥ 最高分 30% 的。
  對拍時比對前拿掉 `decisions` 鍵／終端機輸出的「過去的判斷」段。
- **反芻推薦拿來試＋App 可見**（2026-10-10，ideation F-26／F-48；Python 沒有）：`ctx fresh --try <id>`／
  `agentapi::try_fresh` 在推薦所屬的組開待辦「試用：<檔名>（<ref>，反芻 <id>）」並把推薦標成 used（`used_at`）。
  App 收件匣多一種卡 `fresh`（`push_links` 那一批＝跟 agent 開場看到的同一批），動作 `fresh_try`／`fresh_dismiss`。
- **任務包 `ctx todo brief #N`**（2026-10-10，ideation F-42；Python 沒有）：把一項未結組成給全新會話的 markdown——
  目標（待辦＋期限）／背景（範圍、組目標、上次交接、蓋住待辦文字 ≥20% 的有效判斷）／輸入（知識搜尋前 5 個 ref）／
  限制（範圍、相關判斷的重看條件、卡住 report_status）／完成條件（close_todo／handoff）。App 未結卡多「用任務包開會話」
  （前端動作 `brief_session`：先呼叫 `todo_brief`，再開預填任務的會話對話框）；`parity_state.py` 比對前拿掉這個動作。

CLI／MCP（ctx-cli）：

- **`ctx ui` 不搬**：瀏覽器模式（本機 HTTP＋WebSocket）在 Rust 版沒有對應物；`ctx ui` 印一行
  「已停用：請改用 `ctx app`」到 stderr，exit 2。`--port`／`--no-browser` 仍收（不報用法錯）。
- **`ctx app` 改成啟動 Tauri 桌面 App**：依序找 `Repo Context.app`——ctx 同目錄（或 ctx 所在的 .app 包）、
  `/Applications`、`~/Applications`、`target/{release,debug}/bundle/macos/`；找到就 `open <app> --args --spine <d>`
  （spine 找不到就不帶，讓 App 自己問；明確給了錯的 `--spine` 照樣報錯）。找不到 App 時說明怎麼 build
  （`cd desktop && npm install && npm run bundle`），exit 1。`--port` 收但忽略。目前只支援 macOS（Windows 未做、未測）。
- **`ctx timer`**：前景跑 `timer::run_loop`，Ctrl+C 結束（印「timer 已停」、exit 0）。用 `ctrlc` crate：
  跨平台（macOS＋Windows VDI 都要）、只需要「收到中斷 → 送停止訊號給 run_loop」一件事，
  比 signal-hook 少寫平台分支。`--interval` 為相容保留但不再使用（見上面 run_loop），啟動訊息改成
  「睡到下一個排程點，最多 300s 重看一次」。
- **`--help` 的排版是 clap 的**（子命令順序、說明文字照 Python；argparse 與 clap 的版面不同，不對拍）。
  長旗標可縮寫（`infer_long_args`，同 argparse 的 prefix matching）。用法錯誤一律 exit 2（同 argparse）。
- **沒接住的錯誤**：Python 會印 traceback、exit 1；Rust 印一行「錯誤：<訊息>」到 stderr、exit 1（exit code 相同）。
  例：`collide run <不存在的 cid>`、`append` 不帶 `--dead-letter` 卻被 validator 拒。
- **`ctx search` 不給查詢文字**：Python 因參數被濾掉而 TypeError（traceback、exit 1）；Rust 走 agentapi 的
  `bad_request`「query 不能是空的」（exit 2，`--json` 時是 `{ok:false,...}`）。
- **`collide submit` 不給想法文字**：Python 炸 traceback；Rust 印「錯誤：collide submit 需要想法文字」、exit 1。
- **MCP 工作層缺必填參數**：Python 是 `TypeError` 經 `except Exception` 變 `{ok:false,error:"failed",message:"<fn>() missing 1 required positional argument: 'text'"}`；
  Rust 照抄同一句訊息（不是依賴語言行為），形狀相同。
