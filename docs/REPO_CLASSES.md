# Repo 類別：依「怎樣才算可靠」決定怎麼對待一個 repo

> 2026-10-18 定（使用者提出：參考外部專案、PM 專案、知識專案…要被不同對待）。程式在 `crates/ctx-core/src/registry/class.rs`。

## 為什麼要分

工作台原本把每個登記的 repo 都當成「你在開發的程式」：採集 git 狀態、搜程式碼、提醒沒 commit。
但 orca 這種 clone 來參考的大專案、PM 文件、讀書筆記，被這樣對待只會拖慢、吵、而且 agent 會用錯規矩
（對筆記說「改完跑測試」、對參考專案順手改檔）。

## 六類

分類的主軸：**這個 repo 的東西怎樣才算可靠**。判斷新 repo 歸哪類就問這一句。

| 類別（`class`） | 例子 | 怎樣算可靠 | 給 agent 的規則（開場注入） |
|---|---|---|---|
| 程式 `code` | 工作台、爬蟲、資料管線 | 測試通過 | 改完跑測試，照 AGENTS.md 規矩 commit |
| 領域 Agent `agent` | department-agent-engineering、ForwardDeployedEngineer | eval／範例跑過、prompt 改動有理由 | prompt／skill／流程是產品本體：改措辭說理由、保留術語；改了就跑 eval 並記結果 |
| 數據探索 `data` | notebook、分析 | 重跑得出同樣數字、每個數字有出處 | 數字附出處、不確定別補、大資料先抽樣、資料列不進留痕 |
| 文件・PM `docs` | PM_Head、各產品 PM | 事實正確、決策有記錄 | 不確定就問、不補細節、不用跑測試、別外傳 |
| 知識 `knowledge` | card_notes、讀書會 | 有出處，可以自由整理 | 可以重組改寫，引用附出處 |
| 參考 `reference` | orca、better-agent-terminal | 不改，只讀 | 只讀、不改不 commit，引用寫明專案與檔 |

沒寫 `class`＝未分類，照程式對待。舊的 `type: external` 沒寫 class 時當成參考。
資料管線、爬蟲歸「程式」（產出是一直跑的程式）；探索分析才歸「數據探索」。

## 第一版：軟區分

| 做了 | 在哪 |
|---|---|
| registry 的 `class` 欄（新欄位，舊欄位不動；Python 讀得回） | `registry::store::Repo::class()` |
| CLI `ctx registry class <id> [<類別>]`（不給＝清掉）；App 名片卡的「類別」選單；都走 op `registry_class`（驗證＋〔registry〕留痕） | `ops.rs`、`workbench/actions/namecard.rs`、`RepoProfileCard.tsx` |
| App 助手（⌘K「?把 orca 設成參考」）與 agent（MCP `workbench_apply` 的 `set_class`）：同一份白名單、先預覽、可復原 | `assist.rs`、`agentapi/apply.rs`、`AssistDialog.tsx` |
| 開場注入：所在 repo 一行「這個 repo 是「X」類：規則」；名冊每列標類別，非程式類帶短規則（「參考：只讀」） | `hooks.rs` |
| `context_for.repo_class`、名冊與名片的 `class`（含 label／reliable／rule／short） | `agentapi/mod.rs`、`profile.rs` |
| **參考**類的行為：不採集 git 狀態、不進程式碼搜尋（md 知識索引、read_from 照常） | `collect.rs`、`crossref/code.rs` |

其他五類這一版只有「標示＋告訴 agent」，不擋任何事。

## 之後要加共同限制時

規則只寫在 `RepoClass`：加一個方法（例 `fn app_session_denies(self) -> &[&str]`），各處照類別問它。
候選（都還沒做，要做先問使用者）：

- **App 會話權限**（硬擋）：參考類擋 Edit／Write；含工作資訊的擋 WebFetch／WebSearch。工作台已用 `--settings` 掛 hook，可一起帶 `permissions.deny`。
- **含工作資訊**（可疊在任何類別上的標記，文件・PM 預設開）：反芻不往外推、spine 只記引用不記摘錄、`ask_repo` 先確認、遮蔽更嚴。
  **待決定**：spine 推到個人 GitHub，工作 repo 的判斷要不要完全不進遠端。
- **領域 Agent**：registry 可設 `eval` 指令；體檢新增「改了 prompt／skill 檔卻沒有 log_decision」。
- **數據探索**：notebook 只索引 markdown 格、不搜輸出；「發現」的判斷回顧問「資料更新後還成立嗎」。
- **知識**：反芻的主要來源、權重較高；「介紹過時」門檻放寬。
- **路徑層級覆寫**（一個 repo 同時有 prompt 與程式時）：真的不夠用再做。
