# 構想池（backlog）

> 規則：ID 永不重編；新構想加在表尾；只改「狀態」「最後更新」。
> 狀態：構想 → 候選 → 排入（已開 todo，註記 `#N`）→ 進行 → 完成｜放棄（寫理由）｜併入（寫併入哪個 ID）
> 準則：脈絡／留痕／同介面（可多選）｜P：P0 會掉資料或天天在痛、P1、P2

## 功能（F）——依「repo 抽象化成什麼」分類

| ID | 類別 | 構想 | 準則 | 對標／依據 | P | 狀態 | 首次 | 最後更新 |
|---|---|---|---|---|---|---|---|---|
| F-01 | 專家 | Repo 名片：宣告我是誰、懂什麼、怎麼問、不懂什麼；`ask_repo` 依名片自動選要問誰；R14：R14：search_knowledge 的 repo 排名已能當分派依據；名片的邊際價值低 | 脈絡、同介面 | DeepWiki 研究 agent | P1 | 併入 F-57 | R01 | R28 |
| F-02 | 專家 | 常駐專家：每位專家有自己的記憶檔，記下被問過什麼、怎麼答的 | 脈絡、留痕 | Letta 記憶區塊 | P1 | 候選 | R01 | R01 |
| F-03 | 專家 | 專家會議：一題丟給多個 repo 專家，附引用，人裁決後落成判例 | 留痕 | BMAD 多角色 | P2 | 構想 | R01 | R01 |
| F-04 | 專家 | 專家可信度：用引用紀錄加結果統計誰常被採用、常被推翻 | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-05 | 專家 | 反方專家：專找判斷跟既有知識矛盾的地方 | 留痕 | — | P2 | 構想 | R01 | R01 |
| F-06 | 知識庫 | card_notes 編譯成 LLM Wiki（概念頁＋反向連結＋對哪些 repo 有用）；R02：已有可安裝的 karpathy-llm-wiki skill，整合不重做，ctx 只負責把 raw 對到 repo | 脈絡 | Karpathy LLM Wiki | P1 | 候選 | R01 | R02 |
| F-07 | 知識庫 | 有時效的判斷：decision 帶成立時間與被誰推翻，預設只給目前有效的；R02：純檔案做法已有人驗證（DiffMem、MarkMem：valid_until＋successor；AgDR 的 supersedes）；R03：ADR 實務：舊紀錄要在自己身上看得到「已被取代」，不能只靠索引；R04：抄 shadowgraph：判斷可帶「取代哪一筆」與「何時重看」；被取代的在查詢／開場自動標出、預設只給有效的；R05：迭代 3（a15a9ba）：`ctx log --supersedes --reopen-when`、MCP 兩個選填參數；被取代的退出最近事件／反芻；開場「該重看」／「有條件的判斷」 | 脈絡、留痕 | Graphiti 雙時間軸 | P1 | 完成 | R01 | R05 |
| F-08 | 知識庫 | 跨 repo 矛盾偵測 → 碰撞卡 | 留痕 | — | P2 | 構想 | R01 | R01 |
| F-09 | 知識庫 | 術語表：跨 repo 共用詞統一定義並注入 | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-10 | 知識庫 | 知識週報：新知識影響了哪些專案（反芻從「推一筆」升級成「整理一份」） | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-11 | 知識庫 | 每組匯出靜態頁，人可讀、可當連結給其他 agent | 同介面 | DeepWiki | P2 | 構想 | R01 | R01 |
| F-12 | 流程 | Playbook repo：把 SOP 寫成 skill＋檢查表，開場依任務類型帶入 | 脈絡 | spec-kit、agent-skills | P1 | 構想 | R01 | R01 |
| F-13 | 流程 | 交接把關：依流程定義檢查（沒測、沒更新 README 就提醒）；R02：可參考 Anthropic Outcomes 的評分 rubric | 留痕 | — | P2 | 構想 | R01 | R02 |
| F-14 | 流程 | 規格驅動模板：spec→plan→tasks 落成 spine 事件 | 留痕 | spec-kit、BMAD | P2 | 構想 | R01 | R01 |
| F-15 | 流程 | 從重複操作序列建議出 playbook | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-16 | 技能包 | 把組的 export 自動打包成 Claude Code plugin（skills＋MCP＋hooks）；R14：R14：Anthropic 已把 tribal knowledge 交給 plugin；但 ctx setup 已一次裝好 MCP＋hooks＋skill，單一使用者邊際價值低 | 同介面 | plugin marketplace | P1 | 延後 | R01 | R14 |
| F-17 | 技能包 | 跨 repo 技能目錄：偵測重複與過時 | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-18 | 技能包 | 同類 decision 累積到 N 筆 → 建議寫成 skill（把品味固化） | 留痕 | — | P2 | 構想 | R01 | R01 |
| F-19 | 任務圖 | todo 加相依關係（blocks／related／parent），可跨 repo，next_work 照相依排；R02：Tasks 管執行、Beads 管捕捉；ctx 的 todo 屬於捕捉層。抄「只浮出沒被擋的工作」；Beads 改用 Dolt 後出現穩定性問題，所以維持 md 格式；R13：R13：每組未結約 10 項、相依很少，延後 | 脈絡 | Beads | P1 | 候選 | R01 | R13 |
| F-20 | 任務圖 | todo 帶完成條件；close 時附證據（commit／測試） | 留痕 | — | P2 | 構想 | R01 | R01 |
| F-21 | 任務圖 | 未結事項自動清理：agent 提議哪些可能已完成並附證據；R03：體驗缺口第一名：開場未結 10 項、其中 6 項回填的 due 今天，明天全變逾期雜訊；做法抄 Stale Issue Auditor／Next.js：提議關閉＋附證據（commit、檔案、後續事件），人或 agent 一鍵確認；R04：迭代 2（ae4b359）：`ctx todo review`＋開場體檢一行；覆蓋率判準（連續片段），第一版 7/10 誤判→0；模擬明天標出 #10 #11 #12 #14 | 留痕 | — | P1 | 完成 | R01 | R04 |
| F-22 | 判斷品味 | 判例搜尋：新判斷前列出過去類似 decision 與結果；R02：AI Agentree 已做判例搜尋＋結果追蹤（向量）；我們的差異是跨 repo 而且人和 agent 同一份紀錄；R09：R09 回到原始目標：`search_knowledge` 只搜 md／程式碼、搜不到 spine 裡的判斷——留痕了卻找不回來（痛點 4）；做法：判斷併進搜尋結果（被取代的標出取代者、帶後續 outcome）；R10：迭代 8（69c2677）：search 帶出過去的判斷（取代者、結果、重看條件）；「為什麼選 Tauri」找得到 10-06 定案＋理由 | 脈絡、留痕 | — | P1 | 完成 | R01 | R10 |
| F-23 | 判斷品味 | 判斷回顧：30／90 天後追問「後來對嗎」，累積命中率；R04：shadowgraph 的 reopenWhen＋review() 是同一件事，可併進 F-07 的「何時重看」；R05：日期型條件到期會提醒；非日期條件交給 agent 開場時判斷；R15：選為迭代 14——迭代 8 讓判斷搜尋會帶出結果，但沒人記結果（沒提醒、沒入口）。做法：滿 14 天、沒被取代、還沒記結果的判斷，開場提一行、App 成卡（開會話回顧／延 30 天）；log_decision 加 outcome_of 寫結果事件；**迭代 14 完成**：outcome_of 記結果＋開場回顧行＋App 回顧卡；只回顧人／agent／回填的判斷（第一版選到〔registry〕工具紀錄、ref 只有時間會撞，已修）| 留痕 | — | P1 | 完成 | R01 | R15 |
| F-24 | 判斷品味 | 品味檔案：從 decision 提煉取捨原則，注入給 agent 自我審查；R02：Anthropic Dreaming（排程整理記憶、抽出重複錯誤與偏好）屬於同一類，單 agent 的部分交給平台 | 脈絡 | — | P2 | 構想 | R01 | R02 |
| F-25 | 學習路徑 | 能力地圖：對照目標（AI 產品 PM／工程／前端部署）的證據與缺口 | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-26 | 學習路徑 | 書摘讀完 → 在相關專案開一個「拿來試」的 todo；R09：R09：痛點 3「知識吸收後讓其他 repo 得到好處」目前只有推送（反芻），沒有「拿來試」的行動出口；R10：跟 F-48 一起做：反芻推薦可以「拿來試」→ 在該組開一個帶出處的待辦，推薦標成用上了（Forte：just-in-time，專案需要時才把知識混進產出）；R11：迭代 9（5722ded）：`ctx fresh --try`、App 拿來試；開出帶出處的待辦 | 脈絡 | — | P2 | 完成 | R01 | R11 |
| F-27 | 學習路徑 | 從 decision 和書摘出題，間隔複習 | — | — | P2 | 構想 | R01 | R01 |
| F-28 | 執行層 | 同一題分給多個 agent 比較，選定結果連同理由落成 decision | 留痕 | Orca fan-out | P2 | 構想 | R01 | R01 |
| F-29 | 執行層 | 會話成本與時間帳（每組、每任務的 token 與耗時）；R24：agentsview／tuneloop／agentacct 已在本機讀 Claude Code 會話紀錄算 token 與成本；本工具不重做，需要時建議使用者裝其一 | 脈絡 | — | P2 | 整合不重做 | R01 | R24 |
| F-30 | 執行層 | 會話重播：交接附關鍵步驟摘要和 diff | 留痕 | Conductor diff 審查 | P2 | 構想 | R01 | R01 |
| F-31 | 執行層 | 當其他工具的脈絡層：Orca／Conductor 開的會話也自動掛 ctx 的 MCP＋hooks | 同介面 | Orca、Conductor | P1 | 候選 | R01 | R01 |
| F-32 | 跨 repo | 上游一改，下游自動起 PR 草稿（drift 從提醒進一步到行動） | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-33 | 跨 repo | 相關 repo 建議（共同詞／引用／commit 時間點）；R02：mra 用 5 種掃描器（docker-compose、共用 DB、gateway、共用套件、API 呼叫）自動偵測相依，可以直接抄；R06：做法：四種可解釋訊號——文件裡直接提到對方 repo（Obsidian 未連結提及）、引用紀錄、同一段時間一起改（co-change）、共同主題詞；只建議、附理由，人確認後才 relate；R07：迭代 5（6253123）：`ctx registry suggest`；真資料 15 repo 零關係 → 10 筆建議，PM_Head 推測為 8 個專案的 pm-of；App 還看不到 → F-47；R22：R22：Adamic-Adar（共同鄰居、少見的鄰居權重高）適合「已有關係後再推更多」；目前零關係，延後 | 脈絡 | — | P2 | 完成（CLI＋開場） | R01 | R22 |
| F-34 | 跨 repo | Incubator 主題一鍵轉 repo（建立＋登記＋關係＋初始脈絡）；R09：R09：痛點 1「Incubator 主題轉 repo」至今沒有任何迭代碰到，排在 F-22 之後；R10：R10：spawn --from-incubator 已有；spine incubator 目前是空的，邊際價值低 | 脈絡 | — | P2 | 延後 | R01 | R10 |
| F-35 | 觀測 | 脈絡命中率：開場注入的內容 agent 實際用了幾成 | 脈絡 | — | P1 | 併入 N-03 | R01 | R01 |
| F-36 | 觀測 | 健康指標：未交接率、跨組被拒次數；R17：`ctx health`：使用者真用幾天後，用數字判斷 15 個迭代有沒有用——交接率（會話有沒有交接）、推薦命中率、判斷結果記錄率、未結關閉速度、工具用量（MCP 呼叫紀錄）；**迭代 16 完成**：`ctx health`；使用前基準：交接率 76%、推薦命中率 0%（45 筆）、未結開 46 關 6| 留痕 | — | P2 | 完成 | R01 | R17 |
| F-37 | 入口 | 選單列隨手捕捉一句想法或判斷，直接寫進 spine | 留痕、同介面 | — | P2 | 構想 | R01 | R01 |
| F-38 | 入口 | 外部收件匣：ClickUp／Gmail connector 拉進或推出；R02：Paseo 做了跨裝置控制層，可參考 | 同介面 | — | P2 | 構想 | R01 | R02 |
| F-39 | 入口 | Windows／VDI 入口 | 同介面 | — | P2 | 併入 N-10 | R01 | R01 |
| F-40 | 判斷品味 | decision 可匯出成 AgDR 相容格式（Y 句型、考慮過的選項、agent／model 中繼資料），讓別的工具也讀得到；R22：Claude Code Projects 9/23 起可在本機跑 thread；不掛 ctx MCP 的 agent 也一定讀得到 repo 檔——`ctx decisions export --agdr <dir>` 把判斷（含取代、重看、結果）寫成 AgDR md，是最通用的介面；**迭代 21 完成**：`ctx decisions export <dir>`；真資料副本工作台 12 份| 留痕、同介面 | AgDR | P2 | 完成 | R02 | R22 |
| F-41 | 跨 repo | 跨 repo 分支：同一功能在多個 repo 開同名分支，依相依順序開 PR、等 CI 再合併 | 脈絡 | mra | P2 | 構想 | R02 | R02 |
| F-42 | 流程 | 任務包：替每個 todo 產生一張乾淨的脈絡卡，交給全新 context 的 subagent（state 放在磁碟、不放在模型裡）；R11：`ctx todo brief #N`：目標／背景（組目標、相關判斷＋取代與重看、上次交接）／輸入（相關檔 ref）／限制（範圍、不能改的）／完成條件——全由引擎現有資料組；App 未結卡「用任務包開會話」；R12：迭代 10（125c227）：`ctx todo brief`＋App「用任務包開會話」；判斷蓋住 ≥20% 才放 | 脈絡 | GSD、Ralph loop | P1 | 完成 | R02 | R12 |
| F-43 | 入口 | 手機或遠端檢視收件匣與會話狀態 | 同介面 | Paseo、Superset 遠端工作區 | P2 | 構想 | R02 | R02 |
| F-44 | 定位 | 平台整合：spine 摘要可匯出成 Claude Managed Agents 的檔案式記憶；單 agent 的記憶整理交給平台，ctx 專注跨 repo 與人的判斷；R03：Claude Code Projects（09-17）已做「每個 repo 一個 thread＋共享記憶」，但只在雲端、官方未說記憶可編輯或匯出 → 本工具差異收斂為「本機、可讀、可改、在 git 裡的判斷留痕，涵蓋知識 repo＋人的收件匣」；R22：Projects 已有本機版（🔗 官方 X 9/23）：本機 thread 若是一般 Claude Code 會話，hook／MCP 照樣掛上；先用 F-40 的 repo 檔對接 | 同介面 | Anthropic Memory、Dreaming、Projects | P1 | 構想 | R02 | R22 |

## 非功能（N）——每項附證據

| ID | 品質屬性 | 構想 | 證據（R01 量測或程式位置） | 準則 | P | 狀態 | 首次 | 最後更新 |
|---|---|---|---|---|---|---|---|---|
| N-01 | 資料安全 | handoff 或每天自動 commit spine＋私有 remote 自動推送；**開發 2026-10-09**：SessionEnd hook 自動 commit（`spine::autocommit`，上次 commit 超過 6 小時、可設 `thresholds.spine_autocommit_hours`），有 upstream 就背景 push | 3 份事件檔沒 commit、無 remote（＝#8） | 留痕 | P0 | 進行（commit 完成；remote 待使用者決定 #8） | R01 | R02 |
| N-02 | 效能 | search 降到 500ms 以內：有型別的反序列化 → MCP 常駐時把索引留在記憶體做增量 → 合併後換 bincode／SQLite FTS5；**開發 2026-10-09**：search 2.1–3.0s → 約 0.55s（只要 md 0.38s），結果相同；順手修掉浮點解析不精確導致 631 檔每次重算的舊 bug | `ctx search` 2.2–3.0s；索引 54MB 每次整份解析（`knowledge.rs:104`） | 脈絡 | P0 | 完成 | R01 | R02 |
| N-03 | agent 端 token 效能 | 推送降噪：bigram 停用詞、不跨詞邊界、相關度門檻；量注入 token 數與 read_from 轉換率；R02：做法：比照 Lucene CJKBigram＋StopFilter，用哈工大停用詞表，先不加 jieba-rs；同 3 筆雜訊連兩輪都被推送；**開發 2026-10-09**：共同詞去掉數字日期與跨詞邊界假詞（黏度 ≥0.08）、待處理中不重推；真資料上共同詞變乾淨，但被選的檔沒變——推得準不準要看 `ctx fresh --stats` 的命中率；R03：迭代 1：一般用語過濾（repo 散布 >2/3）、並行反芻去重、fresh 同一份只列一次；真資料模擬 16 筆→3 筆 | 開場新料共同詞為「用後、登記、際使」等雜訊 | 脈絡 | P0 | 完成（第二步） | R01 | R03 |
| N-04 | 可維護 | GitHub Actions：test／clippy／fmt／parity／前端測試，加 Windows 編譯；**開發 2026-10-09**：`.github/workflows/ci.yml`：macOS 全套（前端、fmt、clippy、test、對拍），Windows 只編 ctx（不擋） | 無 `.github/workflows`；312 個 Rust 測試只能手動跑 | — | P0 | 完成（待第一次 push 驗證） | R01 | R02 |
| N-05 | 併發一致 | 所有寫入路徑走鎖＋暫存檔改名；寫入帶 opId 防重複；timer 單例；R02：Beads 用雜湊 ID 避免多 agent 撞號，可參考；R18 稽核：新增的 .state 檔裡，reopen_snooze.json、relation_suggest_dismissed.json 是讀改寫且非原子、無鎖——兩個行程同時按會丟掉人的選擇；review.json 已 tmp＋rename（快取，少一筆可重算）；metrics.jsonl 是 O_APPEND 單行（<4KB，安全）。修法：讀改寫一律持 spine 鎖＋暫存檔改名；**迭代 17**：update_json_locked；舊寫法 20 條執行緒同時延後只剩 1 筆，新寫法 20 筆| 常駐 4 支 `ctx mcp`＋App＋timer 共寫 spine | 留痕 | P1 | 完成（新檔） | R01 | R18 |
| N-06 | 閒置耗能 | 量 App 開一小時的能耗；改用 FSEvents（`notify` crate）取代輪詢 | 未量 | — | P1 | 構想 | R01 | R01 |
| N-07 | 可觀測 | 每次 MCP 呼叫記到 `.state/metrics.jsonl`；`ctx doctor`；錯誤附下一步；R02：mra 有 mra_doctor，可對照；R17：MCP 每次呼叫記一行到 `.state/metrics.jsonl`（工具、毫秒、成功與否），給 `ctx health` 算工具用量與延遲；**迭代 16 完成**：MCP 呼叫記 `.state/metrics.jsonl`| 目前沒有工具層計時 | 脈絡 | P1 | 完成 | R01 | R17 |
| N-08 | 發佈與升級 | codesign＋notarize、Tauri updater 或 Homebrew cask；R02：notarytool＋.dmg 首裝＋updater 讀 GitHub Releases 上的 latest.json | 重 build 後要手動重新裝到 /Applications | 同介面 | P1 | 構想 | R01 | R02 |
| N-09 | 前端體驗 | ⌘K 指令面板、慢動作顯示秒數並可取消、無障礙稽核（#12） | — | 同介面 | P1 | 構想 | R01 | R01 |
| N-10 | 可攜 | Windows／VDI：ConPTY、補 PATH、路徑大小寫、多使用者 token 隔離 | AGENTS.md 要求註明 Windows 有沒有測，目前全空白（#12） | 同介面 | P2 | 構想 | R01 | R01 |
| N-11 | 隱私安全 | ask_repo 唯讀沙盒確認；spine 推送前先過洩密哨兵；R02：dev.to 提出用 symlink＋chmod 做「核心層強制」的唯讀，見 N-15 | — | — | P2 | 構想 | R01 | R02 |
| N-12 | 規模 | 合成 100 repo／1 萬筆事件，測 context 與 search 的上限；R14：12 個迭代在 build_state（App 每次檔案變動都重算）與開場 hook 加了取代／重看／新料／體檢；用合成 1 萬筆事件、多組多 repo 的 spine 量，定預算並修熱點；**迭代 13 完成**：`scripts/perf_spine.py` 合成 1 萬筆；開場 hook 0.60→0.33s（體檢 0.41→0.13s：事件只斷詞一次、未結文字只處理一次）；build_state 145ms| 目前 spine 事件 8 份、.state 71MB | 脈絡 | P2 | 完成 | R01 | R14 |
| N-13 | 可維護 | 合併後刪 Python 版與 `pyjson.rs`／`pystr.rs` 相容層；資料格式加 `schema_version`；MCP 輸出 JSON Schema＋合約測試 | — | 同介面 | P2 | 構想 | R01 | R01 |
| N-14 | 資料安全 | hook 收到沒有 session_id 的輸入時要拒絕或提供 `--dry-run`；不然量測或誤觸會在 spine 留下假會話（被記成未交接）；**開發 2026-10-09**：沒 session_id 照樣注入、不記 marker；量測不再留下假會話 | R02 量測時 `ctx hook session-start </dev/null` 產生 `hook_sessions/unknown.json` | 留痕 | P1 | 完成 | R02 | R02 |
| N-15 | 隱私安全 | 寫入範圍改由檔案系統強制：ask_repo 或跨組讀取走唯讀 view 目錄（symlink＋chmod），不靠 prompt 也不只靠工具層 | `cross_scope` 只擋 MCP／CLI，agent 用 shell 還是寫得進去 | 同介面 | P2 | 構想 | R02 | R02 |
| N-16 | 資料安全 | 沒走 SessionEnd 的會話 marker 會一直留著（`hook_sessions/` 6 個，最舊 10-05）；開場時清掉超過 N 天的，必要時補記「未交接」；R12：跟 N-18 一起做：開場時清掉超過 N 天沒結束的會話 marker；R13：迭代 11：超過 7 天的 marker 開場時照 SessionEnd 規則處理並清掉 | R03 量測 | 留痕 | P2 | 完成 | R03 | R13 |
| N-17 | 效能 | 有檔改過就整份 54MB 知識快取重寫（約 0.5s）：改成依 repo 分檔或追加式；R13：R13 量測：改檔後 search 1.15s、沒改 0.59s；做法：少數檔變動（<50）不重寫整份快取，只留在行程記憶與下次重算；**迭代 12 放棄**：對照實驗（新增一個 md 後搜尋）新舊版都是 0.58s——整份快取重寫的成本可以忽略，R04／R13 看到的 1.1s 是剛 build 完檔案系統快取被擠掉的冷啟動，前提不成立 | R04：工作中改檔後 `ctx search` 1.09s（未改 0.56s） | 脈絡 | P2 | 放棄 | R04 | R13 |
| F-45 | 入口 | 收件匣「之後再說」改成間隔遞增（1→3→7 天），不是固定延 7 天；清理不再有壓力；R19：讀 Matuschak 原文 ✅——「之後再說」取代刪除，間隔遞增再出現、夠久就淡出。做法：未結卡的延期依已延次數 7→14→30 天（第一次維持 7 天＝跟 Python 一致），延 3 次以上卡上提醒「還要做嗎」；**迭代 18 完成**| 脈絡 | Andy Matuschak〈Spaced repetition can lower the stakes around destructive inbox-maintenance〉 | P2 | 完成 | R05 | R19 |
| F-46 | 同介面 | 桌面 App 收件匣看得到 agent 看到的：未結卡帶體檢結論＋證據、到期的「該重看」判斷成卡；R06：迭代 4（e1c2043）：未結卡帶「可能已處理／擱置沒動靜」標籤＋證據；該重看判斷的卡片還沒做（收件匣要新卡種）；R13：補「該重看」卡：條件日期到期的判斷在收件匣成卡（純事件讀），出口＝記一筆新判斷取代它／延後；**迭代 12 完成**：「該重看」卡＋延 14 天（App 與 agent 開場共用延後狀態）| 同介面、留痕 | — | P1 | 完成 | R05 | R13 |
| N-18 | 可維護 | parity「mcp 管理層」notify 未讀跨分鐘偶發不同（last_seen 是 HH:MM）；R12：第二次發生（迭代 10 對拍 mcp 兩項偶發失敗、重跑過）；不穩定的對拍會讓人習慣性重跑、忽略真問題；R13：迭代 11（f715838）：對拍 align_minute，連跑 3 次全過 | R05：迭代 3 對拍一次失敗、重跑兩次皆過 | — | P2 | 完成 | R05 | R13 |
| N-19 | 可維護 | 每次迭代的檢查清單要包含 `parity_state.py`（迭代 2 誤刪 `examples/workbench_dump.rs` 兩輪後才發現）；暫存範例用唯一檔名、只刪自己建的檔 | R06：誤刪事件 | — | P1 | 完成（流程） | R06 | R06 |
| F-47 | 同介面 | App「關係」分頁列出關係建議（理由＋證據），有推測的一鍵套用、沒推測的開預填表單、「不相關」打掉後不再出現；R08：迭代 6（270e73c）：App 關係分頁建議＋套用／設定…／不相關；真資料工作台範圍 1 筆、全部 10 筆 | 同介面、脈絡 | Microsoft HAX（說明為什麼、容易略過、記住使用者的選擇） | P1 | 完成 | R07 | R08 |
| N-20 | agent 端 token 效能 | 推薦帶規則版本：舊規則產生的不再推進開場（保留資料，`ctx fresh --all` 看得到）；開場推薦再加時間衰減（太舊的不推）；R09：迭代 7（fb597e3）：rules 版本＋7 天衰減＋一般用語門檻 1/2；開場注入 2,549→1,881 字元，舊雜訊消失；未結 #46 不刪資料即解 | R08：新版開場仍推 r23–r25（舊規則雜訊），要等 30 天才淡出；對應未結 #46 | 脈絡 | P0 | 完成 | R08 | R09 |
| F-48 | 同介面 | 桌面 App 看得到反芻推薦（目前完全看不到）：範圍內的新料＋共同詞，「拿來試」「不相關」兩個出口；R11：迭代 9：收件匣「相關新料」段；真 spine 副本 7 張卡 | 同介面、脈絡 | — | P1 | 完成 | R10 | R11 |
| N-21 | 前端體驗 | GUI 實測（App 視窗截圖驗收）：迭代 4、6、9 改了畫面只有單元測試；要在使用者沒開 App 時、用 spine 副本開 bundle 實測 | R11：使用者正開著 target/release/bundle 版 App（單一實例），重 bundle 會覆寫、另開會搶焦點 → 本輪不做 | 同介面 | P1 | 候選（等使用者） | R11 | R11 |
| N-22 | agent 端 token 效能／同介面 | 開場提醒集中成一段「提醒」（體檢、該重看、判斷回顧、關係建議），MCP context_for 也回 `hints`（同一份），工作約定補一行新能力（supersedes／outcome_of／todo brief／registry suggest）；**迭代 15 完成**：hints 一份兩用；開場 2,807→2,633 字元（去回填尾巴）| R16：迭代 2/3/5/7/14 各加一行，散在三處；MCP context_for 只有 reopen、拿不到體檢／關係建議／回顧；工作約定沒提新能力 | 同介面、脈絡 | P1 | 完成 | R16 | R16 |
| F-49 | 發佈與升級 | 一鍵安裝＋驗收清單：`scripts/install-app.sh`（App 開著就停、build、驗 ctx、備份、ditto、清 quarantine、刪 bundle 那份、驗證）＋自動迭代的驗收清單（每項在真實使用中怎麼看、`ctx health` 前後比）；**迭代 19 完成**：`scripts/install-app.sh`（含 --dry-run；修掉 BSD head 與 bash 把全形括號吃進變數名兩個 bug）＋`docs/VISION.md` §6（原驗收清單已收斂進去）| R20：18 個迭代都沒裝；使用者開的是 target/release/bundle 那份、Spotlight 有兩個；安裝步驟靠手動 | 同介面 | P0 | 完成 | R20 | R20 |
| N-23 | 同介面（agent 文件） | agent 看的說明跟上新能力：skill（`ctx setup --claude` 裝的 repo-context）加一段「判斷沿革、回顧、交辦」；`ctx todo --help` 等說明補 review／brief；**迭代 20 完成**；驗收清單補「重跑 ctx setup --claude 更新 skill」| R21：skill 與 `ctx todo` 說明仍只寫 add／close；agent 靠 skill 決定用哪些工具 | 同介面 | P1 | 完成 | R21 | R21 |
| N-24 | 可維護 | agentapi.rs 裡 Rust 才有的段落搬進 `agentapi/` 子模組（review／brief／judgment／hints／agdr／fresh_try），行為不變；`agentapi.rs` 回到對應 Python 的那部分；**迭代 22 完成**：mod.rs 2,319→1,407 行，7 個子模組；測試未改、全過| R23：agentapi.rs 1,150→2,319 行，新增 1,175 行全是 Rust 才有的功能疊在後面 | — | P2 | 完成 | R23 | R23 |
| F-50 | 同介面 | App「活動」分頁最上方顯示健康指標（交接率、推薦命中率、判斷結果記錄率、未結開／關／逾期）——跟 `ctx health` 同一份；**迭代 23 完成**| 同介面、留痕 | — | P1 | 完成 | R24 | R24 |
| N-25 | 前端體驗／效能 | 收件匣每段先畫 50 張，其餘「再顯示 N 則」展開（純畫面處理，不放業務邏輯）；**迭代 24 完成**：jsdom 5,000 張卡首次渲染 1,990→105ms| R25：迭代 13 的 1 萬筆規模下 build_state 給 5,389 張卡，App 全部畫進 DOM 可能卡死 WebView | 同介面 | P1 | 完成 | R25 | R25 |
| N-26 | agent 端 token 效能 | agent 拿到的資料設上限：`context_for` 的未結超過 30 項只給最該處理的 30（逾期→到期近→其餘），另給 `open_loops_more`；`todo review --json` 進行中又沒證據的只給編號（`--all` 才全給）；**迭代 25 完成**：context 51→33KB、review 157→34KB（真資料副本 8→2.1KB）| R26：1 萬筆規模 context_for 51KB（104 項未結全列）、todo review 157KB | 脈絡 | P1 | 完成 | R26 | R26 |
| N-27 | 可維護 | CLI 輸出被 `head` 等管線截斷時 panic「failed printing to stdout: Broken pipe」——應安靜結束（SIGPIPE）；R27：收尾前最後一個小項，agent 與人都常截斷看輸出；**迭代 26 完成**：main 一開始 SIGPIPE→SIG_DFL；截斷時 exit 141、stderr 空 | R26 量測時發現（既有行為，非這次迭代造成） | — | P2 | 完成 | R26 | R27 |
| N-28 | agent 端 token 效能 | 開場注入回到 ≤ 3,000 字元：提醒段（體檢／重看／回顧／關係建議）跟新料、未結搶同一個預算，該排優先序或摺疊成「另 N 項，用 todo review 看」 | R28：新版副本注入 4,316 字元（R08 為 2,549） | 脈絡 | P1 | 候選（等真用後看哪段是雜訊） | R28 | R28 |
| F-51 | 入口 | App「檔案」分頁：目前牌組的 repo 當多根工作區（像 IDE 的 Add Folder to Workspace），逐層展開、複製絕對／相對路徑；「＋ 加入資料夾」登記並加進牌組、「從這組移除」、repo 詳情「移出工作台」（不刪檔）；**2026-10-11 完成**；同日改成點 repo 開右側欄、右鍵選單才跳視窗（使用者要求；GUI 未實測） | 使用者 2026-10-11：要能從本機資料夾加 repo、刪除，並像 IDE 側邊欄看檔案、複製路徑，以群組一起看 | 脈絡、同介面 | P1 | 完成 | R28 | R28 |
| F-52 | 入口 | 側欄體驗：左右側欄拖曳調寬、牌組與組員拖曳排序（存 registry 順序）、牌組改成區塊外觀；點檔案在主區開唯讀分頁（md 預覽／原始碼、程式碼行號、圖片），**2026-10-11 完成**（使用者要求；GUI 未實測） | 使用者 2026-10-11：要能調寬、排序、群組像區塊、順暢地看檔案內容（不需編輯） | 脈絡、同介面 | P1 | 完成 | R28 | R28 |
| F-53 | 入口 | 會話放進所屬牌組區塊、終端標題「組 › repo」；牌組筆記（`groups/<組>/notes.md`，筆記分頁＋「＋ 記一筆」）；檔案樹 VS Code 風格並跟著磁碟變動自動更新，**2026-10-12 完成**（使用者要求；GUI 未實測） | 使用者 2026-10-12：會話要看得出屬於哪組哪個 repo；想快速記組層級的事但不知道存哪；右側欄資料夾要像 VS Code、檔案變動要跟著動 | 脈絡、留痕、同介面 | P1 | 完成 | R28 | R28 |
| F-54 | 入口 | 檢視器簡單編輯（只改既有文字檔、textarea、⌘S；帶 version 防止蓋掉 agent 剛寫的內容、保留 CRLF／BOM）＋檔案樹即時更新補強（OS 漏事件時全部重讀、舊結果不蓋新的），**2026-10-13 完成**（使用者要求；GUI 未實測） | 使用者 2026-10-13：文件要能簡單編輯；agent 改檔很快，檔案狀態即時更新很重要（推翻 F-52 的「不需編輯」） | 脈絡、同介面 | P1 | 完成 | R28 | R28 |
| F-55 | 入口 | Agent 會話的 Terminal：終端標題列 [Agent]／[Terminal] 切換、共用同一塊版面（Ctrl+Shift+`），shell 開在該會話目錄（`term_create` 的 `of`），**2026-10-13 完成**（GUI 未實測） | 使用者 2026-10-13：Agent 對話裡打不了 terminal 指令，要能切過去再切回來 | 同介面 | P1 | 完成 | R28 | R28 |
| F-56 | 入口 | App 首次設定可直接「建立新的 spine」（只建在新路徑或空資料夾）；README 開頭說明 spine 是什麼、VISION 加「只讀不寫使用者 repo」原則，**2026-10-13 完成**（GUI 未實測） | 使用者 2026-10-13：開源後新使用者第一步就卡在要先去終端機 `ctx init` | 同介面 | P1 | 完成 | R28 | R28 |
| F-57 | 專家 | Repo 名片＋組員名冊（repo 50 個以上的維護）：registry 加 `description`／`role`／`keywords`（存 spine、不寫進 repo），沒填就從 README 推一句（標推測）；`context_for` 的 `roster`＋開場「同組成員」段（角色、絕對路徑、介紹、有沒有會話在跑）；`search_knowledge` 的 `repo_profiles` 先回答是哪個 repo；`describe_repo`（MCP，只限主場）／`ctx registry describe` 寫名片。**P1 2026-10-13 完成**；P2 待做：App Repo 面板顯示與編輯名片、`ctx health` 標出沒介紹或介紹過時、agent 批次起草名片（出 patch 給人審）、組層級角色覆寫（等真實案例） | 使用者 2026-10-13：repo 到 50 個以上就難管，要像 skill 的短描述一樣可被搜尋，群組內的 agent 要拿得到同組的角色、路徑等；機制要好維護、可開源、最小打擾 | 脈絡、同介面 | P1 | 進行 | R28 | R28 |
