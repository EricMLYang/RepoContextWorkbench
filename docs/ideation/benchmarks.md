# 對標清單（benchmarks）

> 用法：每輪先跑「下一輪查詢」，再處理「待查對標」，然後用**延伸法**——對「標竿」「參考」各搜 `<名稱> alternatives`／`<名稱> vs`，找更新更好的對象。
> 狀態：**標竿**（該類最值得學）｜**參考**（有可抄的設計）｜**零件**（直接接，不重做）｜**觀察**｜**反面**（死法）｜**淘汰**（停更或被取代）
> 查證：✅ 讀過官方原文｜🔗 只看到搜尋結果｜💭 推論
> 2026-09-01 那輪的 11＋6 個工具（gitpane、mani、Repomix…）已拆在 `docs/20260901_外部工具比對_逐一介紹.md`，下表只列摘要。

## 清單

| 名稱 | 類型 | 把 repo 當成什麼 | 可抄 | 不抄／差異 | 狀態 | 查證 | 最後查證 | 發現途徑 | 連結 |
|---|---|---|---|---|---|---|---|---|---|
| Orca | 平行執行台 | 工作區（每任務一個 worktree） | 同一題分給多個 agent 比較、合併；每個 worktree 內嵌瀏覽器 Design Mode | 不管跨 repo 脈絡、不留判斷；終端機不跟它拼 → F-28、F-31 | 標竿 | 🔗 | 2026-10-09 | 使用者點名 | [onorca.dev](https://onorca.dev/) |
| Conductor | 平行執行台 | 工作區（macOS、worktree） | diff／PR 審查流程、串 Linear／GitHub | 同上 → F-30 | 參考 | 🔗 | 2026-10-09 | R01 平行 agent 比較文 | [nimbalyst 比較](https://nimbalyst.com/compare/nimbalyst-vs-conductor-vs-vibe-kanban/) |
| Vibe Kanban | 平行執行台 | issue 看板＋工作區 | 看板形狀的規劃與審查 | bloop 2026-04 關門，改社群維護 | 觀察 | 🔗 | 2026-10-09 | 同上 | [aq.dev](https://aq.dev/alternatives/vibe-kanban/) |
| Sculptor | 平行執行台 | Docker 容器 | 隔離本機 repo，核准後才把改動搬進來 | — | 觀察 | 🔗 | 2026-10-09 | 同上 | [augmentcode 整理](https://www.augmentcode.com/tools/open-source-agent-orchestrators) |
| Nimbalyst（前身 Crystal） | 平行執行台 | 工作區＋視覺編輯 | — | Crystal 2026-02 停更 | 觀察 | 🔗 | 2026-10-09 | 同上 | [nimbalyst.com](https://nimbalyst.com/blog/best-agent-management-tools-2026/) |
| Beads | git 裡的任務記憶 | 任務相依圖（`.beads/` JSONL） | 四種相依關係、跟著分支合併、影響了 Claude Code Tasks；雜湊 ID 防撞 | R02：v50 起強制用 Dolt，有穩定性抱怨（只抄設計，不接） → F-19 | 標竿 | 🔗 | 2026-10-09 | R01 查詢 | [morphllm 介紹](https://www.morphllm.com/beads-agent-memory) |
| Backlog.md | git 裡的任務記憶 | md 任務＋看板 | 純 md、CLI 加網頁、人和 AI 共用 | — | 參考 | 🔗 | 2026-10-09 | R01 查詢 | [README](https://cdn.jsdelivr.net/gh/MrLesk/Backlog.md@main/README.md) |
| DeepWiki（Cognition） | repo 轉 wiki | 百科＋研究 agent | 預先整理成 wiki，網址一換就能用；有 MCP | 只看單一 repo、公開 repo 為主 → F-01、F-11 | 標竿 | 🔗 | 2026-10-09 | R01 查詢 | [指南](https://codersera.com/blog/deepwiki-complete-guide-2026/amp/) |
| Karpathy LLM Wiki | 知識編譯 | 原始資料逐步編譯成 wiki | 不每次重新 RAG，持續累積；Obsidian 當前端 | — → F-06 | 標竿 | 🔗 | 2026-10-09 | R01 查詢 | [教學](https://datasciencedojo.com/blog/llm-wiki-tutorial/) |
| addyosmani/agent-skills | 技能包 | 開發生命週期 skills＋slash 指令 | /spec /plan /build /test /review /ship | — → F-12、F-16 | 參考 | 🔗 | 2026-10-09 | R01 查詢 | [GitHub](https://github.com/addyosmani/agent-skills) |
| Agent-Skills-for-Context-Engineering | 技能包 | context engineering 的 skill 集合（plugin marketplace） | 依任務自動啟用 | — | 參考 | 🔗 | 2026-10-09 | R01 查詢 | [GitHub](https://github.com/muratcankoylan/agent-skills-for-context-engineering) |
| GitHub spec-kit | 流程即規格 | spec→plan→tasks→實作 | 模板＋CLI，跨 agent 通用 | — → F-14 | 參考 | 🔗 | 2026-10-09 | R01 查詢 | [比較文](https://medium.com/@visrow/a-tale-of-two-frameworks-bmad-method-vs-github-spec-kit-c021ab0ad037) |
| BMAD Method | 流程即規格 | 21 個角色 agent 模擬敏捷團隊 | 角色分工、50+ 流程 | 太重 → F-03 | 觀察 | 🔗 | 2026-10-09 | R01 查詢 | 同上 |
| Graphiti／Zep | Agent 記憶 | 雙時間軸知識圖 | 事實何時成立、何時被推翻 | 不引入圖資料庫，只抄欄位 → F-07 | 標竿 | 🔗 | 2026-10-09 | R01 查詢 | [比較文](https://codepointer.substack.com/p/agent-memory-systems-and-knowledge) |
| Letta | Agent 記憶 | 可編輯的文字記憶區塊 | 記憶就是純文字檔 → F-02 | 是完整 agent 框架，不接 | 參考 | 🔗 | 2026-10-09 | 同上 | 同上 |
| Mem0 | Agent 記憶 | 向量庫加實體提示 | — | 向量庫不符「純檔案」 | 觀察 | 🔗 | 2026-10-09 | 同上 | 同上 |
| Cognee | Agent 記憶 | 實體抽取管線 | — | — | 觀察 | 🔗 | 2026-10-09 | 同上 | 同上 |
| gitpane／mani／GitKraken WS | 多 repo 狀態 | 見 0901 文件 | 採集欄位、registry 骨架 | — | 參考（已落地） | ✅ | 2026-09-01 | 0901 輪 | `docs/20260901_…` §二 |
| Repomix／gitingest | repo 打包 | 見 0901 文件 | token 預算、洩密哨兵 | — | 零件（已落地） | ✅ | 2026-09-01 | 0901 輪 | `docs/20260901_…` §四 |
| second-brain／MemoryOS | 知識層 | 見 0901 文件 | typed records、lint 衛生迴圈 | 就地更新 | 參考（已落地） | ✅ | 2026-09-01 | 0901 輪 | `docs/20260901_…` §五 |
| NewReleases.io | 訊號源 | 見 0901 文件 | release webhook | — | 零件 | ✅ | 2026-09-01 | 0901 輪 | `docs/20260901_…` §三 |
| GitHub Repository Dashboard | 多 repo 狀態 | 官方全量檢視 | — | 官方免費做了，不做這塊 | 觀察 | 🔗 | 2026-09-01 | 0901 輪 | `docs/20260901_…` §六 |
| **multi-repo-agent（mra）** | 跨 repo 脈絡（**最接近的同類**） | 有相依的 repo 群組 | 5 種掃描器自動偵測相依；`--with-deps` 一起載入；依相依順序開 PR；MCP 9 個工具（含 doctor）；workspace allowlist；「專案知識庫」有 [DECISION] 標記 | 只有 2 顆星；偏程式碼相依，沒有人的判斷、交接、收件匣 → F-33、F-41 | 參考 | ✅ | 2026-10-09 | R02 查詢 3 | [GitHub](https://github.com/hanfour/multi-repo-agent) |
| The context problem nobody solved（dev.to） | 跨 repo 脈絡（論述） | View Directory：symlink 指到 worktree | 寫入範圍用 chmod 由核心強制，不靠 prompt；整理了 5 種現有做法的缺點 | 文章，不是工具 → N-15 | 參考 | ✅ | 2026-10-09 | R02 查詢 3 | [dev.to](https://dev.to/mnzs/the-context-problem-nobody-solved-ai-agents-across-multiple-repos-4533) |
| Repo Lens MCP／multi-repo harness 研究 | 跨 repo 脈絡 | 跨 repo 程式碼搜尋（AST） | 結構化搜尋 | 只管程式碼 | 觀察 | 🔗 | 2026-10-09 | R02 查詢 3 | [mcpservers](https://mcpservers.org/servers/yohannhommet/mcp-repo-search-server)、[深入研究](https://aixsoftware.substack.com/p/ai-research-multi-repo-agentic-harnesses) |
| **AgDR（Agent Decision Records）** | 判斷留痕 | `docs/agdr/` 的 md＋YAML | Y 句型、考慮過的選項、agent／model／trigger 中繼資料、proposed→executed→superseded、`/decide` skill | 單一 repo、沒有搜尋 → F-40、F-07 | 標竿 | ✅ | 2026-10-09 | R02 查詢 4 | [GitHub](https://github.com/me2resh/agent-decision-record) |
| AI Agentree | 判斷留痕 | 決策軌跡平台 | 判例搜尋（向量）＋結果追蹤 | 雲端平台，不是純檔案 → F-22、F-23 | 參考 | 🔗 | 2026-10-09 | R02 查詢 4 | [ai-agentree.com](https://www.ai-agentree.com/) |
| DeepWiki Open | repo 轉 wiki | 自架 wiki 產生器 | 私有 repo、Ollama 本機模型、Mermaid 圖、FAISS 純檔案 | Python＋Next＋Docker，太重 → F-11 | 觀察 | 🔗 | 2026-10-09 | R02 查詢 5 | [介紹](https://www.hostinger.com/applications/deepwiki-open) |
| DiffMem | Agent 記憶 | md 只存「現在」，歷史交給 git diff／log | 不用向量庫；歷史用 git 查 | — → F-07 | 參考 | 🔗 | 2026-10-09 | R02 查詢 6 | [landscape](https://landscape.jimmysong.io/projects/diffmem) |
| MarkMem | Agent 記憶 | YAML 雙時間事實＋git | 舊事實不刪：寫 valid_until＋successor；git log 就是稽核紀錄 | — → F-07 | 參考 | 🔗 | 2026-10-09 | R02 查詢 6 | [PyPI](https://pypi.org/project/markmem/) |
| **Anthropic 平台（Managed Agents Memory、Dreaming、Outcomes、Claude Code Projects）** | 平台（**會吃掉單 agent 記憶**） | 記憶以檔案存放；Dreaming 排程整理；Projects 協調多個 thread 共享記憶（2026-09-17） | 檔案式記憶、排程整理、評分 rubric | 平台不管跨 repo 和人的判斷；我們整合，不重做 → F-44、F-24、F-13 | 標竿（平台風險） | ✅（Code with Claude 文）／🔗（Projects） | 2026-10-10（R03：讀官方頁 ✅——Projects 每個 repo 一個 thread、共享記憶；只在雲端，未說記憶可編輯／匯出） | R02 延伸（Tasks） | [mindstudio](https://www.mindstudio.ai/blog/code-with-claude-2026-new-agent-features)、[Projects 官方](https://claude.com/resources/articles/projects-redesigned) |
| Claude Code Tasks | 任務記憶 | session 內的執行步驟（可存檔延續） | — | 管執行；Beads／ctx todo 管捕捉，兩者互補 → F-19 | 觀察 | 🔗 | 2026-10-09 | R02 查詢 2 | [keepcoding](https://keepcoding.io/blog-frr/tasks-vs-beads-memoria-claude-code/) |
| Kilo Agent Manager | 平行執行台 | VS Code 面板 | — | 綁 Kilo 擴充 | 觀察 | 🔗 | 2026-10-09 | R02 查詢 1（Orca 延伸） | [Kilo blog](https://blog.kilo.ai/p/orca-vs-kilo-agent-manager) |
| Claude Squad | 平行執行台 | 終端機多工 | 暫停／審查／出貨 | — | 觀察 | 🔗 | 2026-10-09 | 待查 | [review](https://vibecodinghub.org/blog/claude-squad-review) |
| Superset | 平行執行台 | worktree 桌面工作區 | MCP、遠端／雲端工作區 | — → F-43 | 觀察 | 🔗 | 2026-10-09 | 待查 | [vs Claude Squad](https://superset.sh/compare/superset-vs-claude-squad) |
| Paseo | 入口 | 跨裝置控制層（網頁、桌面、手機、CLI） | 自架、手機看 agent | — → F-43 | 參考 | 🔗 | 2026-10-09 | 待查 | [amux 整理](https://amux.io/guides/best-claude-code-session-managers-2026/) |
| Kiro（AWS） | 流程即規格 | spec 是真相，程式碼是產物 | steering files（常駐專案知識）＋事件 hooks | IDE 綁定 → F-12 | 參考 | 🔗 | 2026-10-09 | 待查 | [指南](https://thepromptshelf.dev/blog/amazon-kiro-ide-complete-guide-2026/) |
| GSD（Get Shit Done） | 流程／context engineering | spec→研究→原子任務包 | 每個任務交給全新 200K context 的 subagent，防 context rot | — → F-42 | 參考 | 🔗 | 2026-10-09 | 待查 | [mindstudio](https://www.mindstudio.ai/blog/gsd-framework-prevent-context-rot-claude-code) |
| Ralph loop | 流程 | 固定 prompt 的無限迴圈，state 存在磁碟 | 每輪全新 context，從檔案讀狀態 | token 成本高 → F-42 | 觀察 | 🔗 | 2026-10-09 | 待查 | [Thoughtworks Radar](https://www.thoughtworks.com/radar/techniques/ralph-loop) |
| karpathy-llm-wiki skill（astro-han） | 知識編譯 | raw/ → wiki/，做成可安裝的 skill | 直接拿來用 | — → F-06 整合不重做 | 零件 | 🔗 | 2026-10-09 | R02 延伸（LLM Wiki） | [skills.sh](https://www.skills.sh/astro-han/karpathy-llm-wiki/karpathy-llm-wiki) |
| ADK Stale Issue Auditor（google/adk-python PR #3546） | 任務記憶清理 | 每天排程審 issue 時間軸 | 標 stale → 通知 → 二次門檻才關；依時間軸判斷是否還活著 | GitHub issue 限定 → F-21 | 參考 | 🔗 | 2026-10-10 | R03 查詢 1 | [PR](https://github.com/google/adk-python/pull/3546) |
| Next.js〈How we closed 1500 GitHub issues〉 | 任務記憶清理 | 大量積壓的整理實務 | 關閉要附理由、可重開 | 文章 → F-21 | 參考 | 🔗 | 2026-10-10 | R03 查詢 1 | [blog](https://nextjs.org/blog/how-we-closed-1500-github-issues) |
| 〈The ADR your agent never read〉（tianpan） | 判斷留痕（論述） | ADR 實務 | 不改舊紀錄、寫新的取代它，並在**舊紀錄自己身上**標已被取代；可取回「目前有效」與整條沿革 | — → F-07 | 參考 | 🔗 | 2026-10-10 | R03 查詢 2 | [blog](https://tianpan.co/blog/2026/07/02/the-adr-your-agent-never-read) |
| **shadowgraph（LiLara-AI）** | 判斷留痕（**直接同類**） | `.shadowgraph/data.json`（或 SQLite）的決策圖 | 選了／放棄了什麼與理由、**reopenWhen**（何時重看）＋`review()` 對事實比對、outcome 回饋信心、來源分級（agent 自稱／人確認／正式環境驗證）；MCP＋CLI＋本機 HTTP | 單一專案、Node、3 星；不是 md／git 可讀 → F-07、F-23 | 標竿 | ✅ | 2026-10-10 | R04 查詢（decision log MCP） | [GitHub](https://github.com/LiLara-AI/shadowgraph) |
| linklore-mcp | 判斷留痕 | 專案記憶 MCP | 記決策、踩坑、規格，下次開場帶回 | — | 觀察 | 🔗 | 2026-10-10 | R04 查詢 | [enterprisedna](https://enterprisedna.co/directories/mcp/linklore-linklore-mcp) |
| DEG（Decision-Evidence Graph，`degraph`） | 判斷留痕 | 跨會話的決策記憶 | 為什麼、放棄了什麼、何時重看＋完整出處 | Python 套件 → F-07 | 觀察 | 🔗 | 2026-10-10 | R05 延伸（shadowgraph alternatives） | [PyPI](https://pypi.org/project/degraph/) |
| Andy Matuschak〈Spaced repetition … inbox-maintenance〉 | 入口（UX 論述） | 收件匣整理 | 把「刪掉」改成「之後再說」，間隔遞增再出現，降低清理的壓力 | — → F-45 | 參考 | 🔗 | 2026-10-10 | R05 查詢 2 | [notes](https://notes.andymatuschak.org/zJ5Yzvba2729XKXivBBZ91J) |
| Qodo cross-repo review | 跨 repo 連動 | 自動發現 repo 間關係＋跨 repo 審 PR | 從技術訊號自動推關係，也可手動定義 | 商業、程式碼為主 → F-33 | 參考 | 🔗 | 2026-10-10 | R06 查詢 1 | [docs](https://docs.qodo.ai/governance/cross-repo-code-review) |
| co-change mining（Kagdi 等） | 跨 repo 連動（研究） | 版本歷史 | 一起被 commit 的檔／repo＝隱性關係，靜態分析看不到 | — → F-33 | 參考 | 🔗 | 2026-10-10 | R06 查詢 1 | [Kagdi07](https://www.cs.kent.edu/~jmaletic/cs63902/Papers/Kagdi07.pdf) |
| Obsidian 未連結提及／Atomic Insights（Adamic-Adar） | 知識庫連結建議 | 筆記庫 | 「提到了卻沒連」一鍵建連結；共同鄰居（Adamic-Adar）推薦 | 單一 vault → F-33 | 參考 | 🔗 | 2026-10-10 | R06 查詢 2 | [Atomic Insights](https://community.obsidian.md/plugins/atomic-insights) |
| Microsoft HAX Toolkit（G7／G11／G13） | 入口（AI 建議 UX 準則） | — | 建議要能輕鬆略過、說得出為什麼、記住使用者的選擇並說明會怎麼影響之後 | — → F-47 | 參考 | 🔗 | 2026-10-10 | R07 查詢 | [HAX](https://www.microsoft.com/en-us/haxtoolkit/application-type/prediction/page/5) |
| Google Discover SDF（Supersession-Decay Filtering） | 推薦過期處理 | — | 被取代（新內容／新規則取代舊的）與自然衰減兩道濾網；上線後過期回報 -54.9% | 學習式模型不抄，抄「兩種過期分開處理」 → N-20 | 參考 | 🔗 | 2026-10-10 | R08 查詢 2 | [alphaxiv](https://alphaxiv.org/abs/2608.15780v1) |
| supermemory〈How to Preserve Decisions an AI Agent Should Not Reopen〉 | 判斷留痕（論述） | — | 記選擇＋理由＋放棄的選項＋什麼情況值得重看；逐字稿不是決策紀錄 | — → F-07 驗證方向對 | 參考 | 🔗 | 2026-10-10 | R08 查詢 1 | [blog](https://supermemory.ai/blog/agent-decision-records/) |
| Claude Code auto memory | 平台 | `~/.claude/projects/<路徑>/memory/*.md` | 本機 md 記憶；匯出／匯入功能請求被關成不做 | 單一路徑、不跨 repo → F-44 可對接 | 觀察 | 🔗 | 2026-10-10 | R04 查詢 | [docs](https://code.claude.com/docs/en/memory)、[issue #67369](https://github.com/anthropics/claude-code/issues/67369) |

## 待查對標（下一輪處理）

| 名稱 | 為什麼值得查 | 從哪延伸 |
|---|---|---|
| agentsroom.dev、Abralo | Orca alternatives 頁（R01 留下，R02 沒查） | Orca |
| Mux（Coder） | 平行開發，Coder 出的 | 平行 agent 比較文 |
| Ticket、Tracer、Honcho | 列為 Beads 的替代品 | Beads alternatives |
| rufuspollock〈issue tracking dilemma〉 | 談 agent 時代的 issue 追蹤取捨 | Beads alternatives |
| Anthropic Agent Teams、Ruflo、ccpm | Claude 多 agent 生態整理文 | Claude Squad |
| WolfDB、StudioMeyer Memory | 雙時間記憶，宣稱可像 git 一樣檢視 | DiffMem／MarkMem |
| lucasastorian/llmwiki、Pratiyush/llm-wiki、memory-compiler | LLM Wiki 的其他實作，有一個會自動抓對話 | karpathy-llm-wiki |
| AgenticAKM（arXiv 2602.04445） | 用 agent 管架構知識的研究 | AgDR |
| Decision Log MCP（decisionlog.ai） | 判斷留痕做成 MCP，可能是直接同類 | R03 查詢 2 |
| agdr-aki（Rust crate） | 封存式 AgDR，Rust 實作 | R03 查詢 2 |

## 搜尋線索

### 用過的查詢
- R01：`Orca agent IDE parallel coding agents worktrees`（standard）
- R01：`2026 tools orchestrate multiple coding agents Conductor Crystal Vibe Kanban Sculptor comparison`（extended）
- R01：`repo as knowledge base for AI agents skills plugins domain expert agent git-backed context engineering 2026`（extended）
- R01：`agent memory tools Letta Mem0 Zep Graphiti Cognee knowledge graph for coding agents`
- R01：`DeepWiki … spec-kit BMAD Task Master spec-driven development agents`
- R01：`Backlog.md git-native task management …; Beads issue tracker …`
- R01：`Karpathy LLM wiki pattern Obsidian vault agent knowledge base 2026 Claude Code`
- R02：`Orca vs Kilo Agent Manager vs Claude Squad 2026`
- R02：`Claude Code Tasks vs Beads dependency graph`
- R02：`cross-repo context MCP server multi-repo agent 2026`（extended）
- R02：`decision log ADR tool for AI agents precedent search`（extended）
- R02：`self-hosted DeepWiki open source local repo wiki`
- R02：`bi-temporal facts markdown git agent memory`
- R02：`Tauri app auto update notarize best practice 2026`
- R02：`Chinese BM25 tokenization bigram stopwords Rust`
- R02：`Claude Squad Superset Paneflow Mux Paseo …`、`Kiro …; GSD …; Ralph loop`、`Anthropic Claude Code agent teams memory 2026`（extended）、`Beads alternatives …`、`Karpathy LLM wiki implementation …`
- R03：`AI agent stale issues cleanup propose close with evidence from commits 2026`
- R03：`AgDR agent decision records supersede search across repositories`
- R03：`Claude Code Projects threads shared memory multi-repo September 2026`（extended）＋讀官方頁
- R04：`decisionlog.ai MCP server decision log for AI agents`（→ shadowgraph ✅）
- R04：`Claude Code Projects local execution memory export October 2026`（extended）
- R05：`decision memory for AI agents reopen conditions alternatives shadowgraph 2026`（→ DEG）
- R05：`desktop app inbox design triage stale tasks evidence suggestions UX 2026 agent`（→ Matuschak）
- R06：`automatically infer relationships between repositories cross-repo references co-change analysis`
- R06：`personal knowledge base suggest links between notes unlinked mentions Obsidian algorithm`

- R07：`explainable suggestions UI accept dismiss "why am I seeing this" pattern design 2026`

- R08：`supermemory agent decision records blog`
- R08：`recommendation staleness versioning re-score old suggestions after algorithm change best practice`

### 下一輪查詢
1. `multi-repo-agent mra alternatives polyrepo AI agent orchestration`（R02 留下）
2. `supermemory agent decision records`
3. `MCP server latency caching long-running index Rust`（N-17）
4. `Claude Code Projects local threads release`（盯平台本機版）
5. `Adamic-Adar link prediction small graph explainable suggestions`（F-33 做完後要不要加共同鄰居）
