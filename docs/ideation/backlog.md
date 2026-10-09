# 構想池（backlog）

> 規則：ID 永不重編；新構想加在表尾；只改「狀態」「最後更新」。
> 狀態：構想 → 候選 → 排入（已開 todo，註記 `#N`）→ 進行 → 完成｜放棄（寫理由）｜併入（寫併入哪個 ID）
> 準則：脈絡／留痕／同介面（可多選）｜P：P0 會掉資料或天天在痛、P1、P2

## 功能（F）——依「repo 抽象化成什麼」分類

| ID | 類別 | 構想 | 準則 | 對標／依據 | P | 狀態 | 首次 | 最後更新 |
|---|---|---|---|---|---|---|---|---|
| F-01 | 專家 | Repo 名片：宣告我是誰、懂什麼、怎麼問、不懂什麼；`ask_repo` 依名片自動選要問誰 | 脈絡、同介面 | DeepWiki 研究 agent | P1 | 候選 | R01 | R01 |
| F-02 | 專家 | 常駐專家：每位專家有自己的記憶檔，記下被問過什麼、怎麼答的 | 脈絡、留痕 | Letta 記憶區塊 | P1 | 候選 | R01 | R01 |
| F-03 | 專家 | 專家會議：一題丟給多個 repo 專家，附引用，人裁決後落成判例 | 留痕 | BMAD 多角色 | P2 | 構想 | R01 | R01 |
| F-04 | 專家 | 專家可信度：用引用紀錄加結果統計誰常被採用、常被推翻 | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-05 | 專家 | 反方專家：專找判斷跟既有知識矛盾的地方 | 留痕 | — | P2 | 構想 | R01 | R01 |
| F-06 | 知識庫 | card_notes 編譯成 LLM Wiki（概念頁＋反向連結＋對哪些 repo 有用）；R02：已有可安裝的 karpathy-llm-wiki skill，整合不重做，ctx 只負責把 raw 對到 repo | 脈絡 | Karpathy LLM Wiki | P1 | 候選 | R01 | R02 |
| F-07 | 知識庫 | 有時效的判斷：decision 帶成立時間與被誰推翻，預設只給目前有效的；R02：純檔案做法已有人驗證（DiffMem、MarkMem：valid_until＋successor；AgDR 的 supersedes） | 脈絡、留痕 | Graphiti 雙時間軸 | P1 | 候選（R02 升為功能首位） | R01 | R02 |
| F-08 | 知識庫 | 跨 repo 矛盾偵測 → 碰撞卡 | 留痕 | — | P2 | 構想 | R01 | R01 |
| F-09 | 知識庫 | 術語表：跨 repo 共用詞統一定義並注入 | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-10 | 知識庫 | 知識週報：新知識影響了哪些專案（反芻從「推一筆」升級成「整理一份」） | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-11 | 知識庫 | 每組匯出靜態頁，人可讀、可當連結給其他 agent | 同介面 | DeepWiki | P2 | 構想 | R01 | R01 |
| F-12 | 流程 | Playbook repo：把 SOP 寫成 skill＋檢查表，開場依任務類型帶入 | 脈絡 | spec-kit、agent-skills | P1 | 構想 | R01 | R01 |
| F-13 | 流程 | 交接把關：依流程定義檢查（沒測、沒更新 README 就提醒）；R02：可參考 Anthropic Outcomes 的評分 rubric | 留痕 | — | P2 | 構想 | R01 | R02 |
| F-14 | 流程 | 規格驅動模板：spec→plan→tasks 落成 spine 事件 | 留痕 | spec-kit、BMAD | P2 | 構想 | R01 | R01 |
| F-15 | 流程 | 從重複操作序列建議出 playbook | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-16 | 技能包 | 把組的 export 自動打包成 Claude Code plugin（skills＋MCP＋hooks） | 同介面 | plugin marketplace | P1 | 候選 | R01 | R01 |
| F-17 | 技能包 | 跨 repo 技能目錄：偵測重複與過時 | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-18 | 技能包 | 同類 decision 累積到 N 筆 → 建議寫成 skill（把品味固化） | 留痕 | — | P2 | 構想 | R01 | R01 |
| F-19 | 任務圖 | todo 加相依關係（blocks／related／parent），可跨 repo，next_work 照相依排；R02：Tasks 管執行、Beads 管捕捉；ctx 的 todo 屬於捕捉層。抄「只浮出沒被擋的工作」；Beads 改用 Dolt 後出現穩定性問題，所以維持 md 格式 | 脈絡 | Beads | P1 | 候選 | R01 | R02 |
| F-20 | 任務圖 | todo 帶完成條件；close 時附證據（commit／測試） | 留痕 | — | P2 | 構想 | R01 | R01 |
| F-21 | 任務圖 | 未結事項自動清理：agent 提議哪些可能已完成並附證據 | 留痕 | — | P1 | 構想 | R01 | R01 |
| F-22 | 判斷品味 | 判例搜尋：新判斷前列出過去類似 decision 與結果；R02：AI Agentree 已做判例搜尋＋結果追蹤（向量）；我們的差異是跨 repo 而且人和 agent 同一份紀錄 | 脈絡、留痕 | — | P1 | 候選 | R01 | R02 |
| F-23 | 判斷品味 | 判斷回顧：30／90 天後追問「後來對嗎」，累積命中率 | 留痕 | — | P1 | 候選 | R01 | R01 |
| F-24 | 判斷品味 | 品味檔案：從 decision 提煉取捨原則，注入給 agent 自我審查；R02：Anthropic Dreaming（排程整理記憶、抽出重複錯誤與偏好）屬於同一類，單 agent 的部分交給平台 | 脈絡 | — | P2 | 構想 | R01 | R02 |
| F-25 | 學習路徑 | 能力地圖：對照目標（AI 產品 PM／工程／前端部署）的證據與缺口 | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-26 | 學習路徑 | 書摘讀完 → 在相關專案開一個「拿來試」的 todo | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-27 | 學習路徑 | 從 decision 和書摘出題，間隔複習 | — | — | P2 | 構想 | R01 | R01 |
| F-28 | 執行層 | 同一題分給多個 agent 比較，選定結果連同理由落成 decision | 留痕 | Orca fan-out | P2 | 構想 | R01 | R01 |
| F-29 | 執行層 | 會話成本與時間帳（每組、每任務的 token 與耗時） | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-30 | 執行層 | 會話重播：交接附關鍵步驟摘要和 diff | 留痕 | Conductor diff 審查 | P2 | 構想 | R01 | R01 |
| F-31 | 執行層 | 當其他工具的脈絡層：Orca／Conductor 開的會話也自動掛 ctx 的 MCP＋hooks | 同介面 | Orca、Conductor | P1 | 候選 | R01 | R01 |
| F-32 | 跨 repo | 上游一改，下游自動起 PR 草稿（drift 從提醒進一步到行動） | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-33 | 跨 repo | 相關 repo 建議（共同詞／引用／commit 時間點）；R02：mra 用 5 種掃描器（docker-compose、共用 DB、gateway、共用套件、API 呼叫）自動偵測相依，可以直接抄 | 脈絡 | — | P2 | 候選 | R01 | R02 |
| F-34 | 跨 repo | Incubator 主題一鍵轉 repo（建立＋登記＋關係＋初始脈絡） | 脈絡 | — | P2 | 構想 | R01 | R01 |
| F-35 | 觀測 | 脈絡命中率：開場注入的內容 agent 實際用了幾成 | 脈絡 | — | P1 | 併入 N-03 | R01 | R01 |
| F-36 | 觀測 | 健康指標：未交接率、跨組被拒次數 | 留痕 | — | P2 | 構想 | R01 | R01 |
| F-37 | 入口 | 選單列隨手捕捉一句想法或判斷，直接寫進 spine | 留痕、同介面 | — | P2 | 構想 | R01 | R01 |
| F-38 | 入口 | 外部收件匣：ClickUp／Gmail connector 拉進或推出；R02：Paseo 做了跨裝置控制層，可參考 | 同介面 | — | P2 | 構想 | R01 | R02 |
| F-39 | 入口 | Windows／VDI 入口 | 同介面 | — | P2 | 併入 N-10 | R01 | R01 |
| F-40 | 判斷品味 | decision 可匯出成 AgDR 相容格式（Y 句型、考慮過的選項、agent／model 中繼資料），讓別的工具也讀得到 | 留痕、同介面 | AgDR | P2 | 構想 | R02 | R02 |
| F-41 | 跨 repo | 跨 repo 分支：同一功能在多個 repo 開同名分支，依相依順序開 PR、等 CI 再合併 | 脈絡 | mra | P2 | 構想 | R02 | R02 |
| F-42 | 流程 | 任務包：替每個 todo 產生一張乾淨的脈絡卡，交給全新 context 的 subagent（state 放在磁碟、不放在模型裡） | 脈絡 | GSD、Ralph loop | P1 | 構想 | R02 | R02 |
| F-43 | 入口 | 手機或遠端檢視收件匣與會話狀態 | 同介面 | Paseo、Superset 遠端工作區 | P2 | 構想 | R02 | R02 |
| F-44 | 定位 | 平台整合：spine 摘要可匯出成 Claude Managed Agents 的檔案式記憶；單 agent 的記憶整理交給平台，ctx 專注跨 repo 與人的判斷 | 同介面 | Anthropic Memory、Dreaming、Projects | P1 | 構想 | R02 | R02 |

## 非功能（N）——每項附證據

| ID | 品質屬性 | 構想 | 證據（R01 量測或程式位置） | 準則 | P | 狀態 | 首次 | 最後更新 |
|---|---|---|---|---|---|---|---|---|
| N-01 | 資料安全 | handoff 或每天自動 commit spine＋私有 remote 自動推送；**開發 2026-10-09**：SessionEnd hook 自動 commit（`spine::autocommit`，上次 commit 超過 6 小時、可設 `thresholds.spine_autocommit_hours`），有 upstream 就背景 push | 3 份事件檔沒 commit、無 remote（＝#8） | 留痕 | P0 | 進行（commit 完成；remote 待使用者決定 #8） | R01 | R02 |
| N-02 | 效能 | search 降到 500ms 以內：有型別的反序列化 → MCP 常駐時把索引留在記憶體做增量 → 合併後換 bincode／SQLite FTS5；**開發 2026-10-09**：search 2.1–3.0s → 約 0.55s（只要 md 0.38s），結果相同；順手修掉浮點解析不精確導致 631 檔每次重算的舊 bug | `ctx search` 2.2–3.0s；索引 54MB 每次整份解析（`knowledge.rs:104`） | 脈絡 | P0 | 完成 | R01 | R02 |
| N-03 | agent 端 token 效能 | 推送降噪：bigram 停用詞、不跨詞邊界、相關度門檻；量注入 token 數與 read_from 轉換率；R02：做法：比照 Lucene CJKBigram＋StopFilter，用哈工大停用詞表，先不加 jieba-rs；同 3 筆雜訊連兩輪都被推送；**開發 2026-10-09**：共同詞去掉數字日期與跨詞邊界假詞（黏度 ≥0.08）、待處理中不重推；真資料上共同詞變乾淨，但被選的檔沒變——推得準不準要看 `ctx fresh --stats` 的命中率 | 開場新料共同詞為「用後、登記、際使」等雜訊 | 脈絡 | P0 | 完成（第一步） | R01 | R02 |
| N-04 | 可維護 | GitHub Actions：test／clippy／fmt／parity／前端測試，加 Windows 編譯；**開發 2026-10-09**：`.github/workflows/ci.yml`：macOS 全套（前端、fmt、clippy、test、對拍），Windows 只編 ctx（不擋） | 無 `.github/workflows`；312 個 Rust 測試只能手動跑 | — | P0 | 完成（待第一次 push 驗證） | R01 | R02 |
| N-05 | 併發一致 | 所有寫入路徑走鎖＋暫存檔改名；寫入帶 opId 防重複；timer 單例；R02：Beads 用雜湊 ID 避免多 agent 撞號，可參考 | 常駐 4 支 `ctx mcp`＋App＋timer 共寫 spine | 留痕 | P1 | 構想 | R01 | R02 |
| N-06 | 閒置耗能 | 量 App 開一小時的能耗；改用 FSEvents（`notify` crate）取代輪詢 | 未量 | — | P1 | 構想 | R01 | R01 |
| N-07 | 可觀測 | 每次 MCP 呼叫記到 `.state/metrics.jsonl`；`ctx doctor`；錯誤附下一步；R02：mra 有 mra_doctor，可對照 | 目前沒有工具層計時 | 脈絡 | P1 | 構想 | R01 | R02 |
| N-08 | 發佈與升級 | codesign＋notarize、Tauri updater 或 Homebrew cask；R02：notarytool＋.dmg 首裝＋updater 讀 GitHub Releases 上的 latest.json | 重 build 後要手動重新裝到 /Applications | 同介面 | P1 | 構想 | R01 | R02 |
| N-09 | 前端體驗 | ⌘K 指令面板、慢動作顯示秒數並可取消、無障礙稽核（#12） | — | 同介面 | P1 | 構想 | R01 | R01 |
| N-10 | 可攜 | Windows／VDI：ConPTY、補 PATH、路徑大小寫、多使用者 token 隔離 | AGENTS.md 要求註明 Windows 有沒有測，目前全空白（#12） | 同介面 | P2 | 構想 | R01 | R01 |
| N-11 | 隱私安全 | ask_repo 唯讀沙盒確認；spine 推送前先過洩密哨兵；R02：dev.to 提出用 symlink＋chmod 做「核心層強制」的唯讀，見 N-15 | — | — | P2 | 構想 | R01 | R02 |
| N-12 | 規模 | 合成 100 repo／1 萬筆事件，測 context 與 search 的上限 | 目前 spine 事件 8 份、.state 71MB | 脈絡 | P2 | 構想 | R01 | R01 |
| N-13 | 可維護 | 合併後刪 Python 版與 `pyjson.rs`／`pystr.rs` 相容層；資料格式加 `schema_version`；MCP 輸出 JSON Schema＋合約測試 | — | 同介面 | P2 | 構想 | R01 | R01 |
| N-14 | 資料安全 | hook 收到沒有 session_id 的輸入時要拒絕或提供 `--dry-run`；不然量測或誤觸會在 spine 留下假會話（被記成未交接）；**開發 2026-10-09**：沒 session_id 照樣注入、不記 marker；量測不再留下假會話 | R02 量測時 `ctx hook session-start </dev/null` 產生 `hook_sessions/unknown.json` | 留痕 | P1 | 完成 | R02 | R02 |
| N-15 | 隱私安全 | 寫入範圍改由檔案系統強制：ask_repo 或跨組讀取走唯讀 view 目錄（symlink＋chmod），不靠 prompt 也不只靠工具層 | `cross_scope` 只擋 MCP／CLI，agent 用 shell 還是寫得進去 | 同介面 | P2 | 構想 | R02 | R02 |
