# 非功能量測基準（baseline）

> 每輪加一欄，量法見 skill `pm-ideation` 步驟 1。量不到寫「未量」，不估。
> 目標欄是現在的預算，可以隨輪次調整（要寫理由）。

| 項目 | 目標 | R01 2026-10-09 | R02 2026-10-09 |
|---|---|---|---|
| `ctx where --json` | < 50ms | 10ms | 10ms |
| `ctx next --json` | < 100ms | 10ms | 10ms |
| `ctx context --json` | < 300ms | 150ms | 110ms |
| SessionStart hook | < 200ms | 130ms | 120ms |
| `ctx search` | < 500ms | **2.2–3.0s** ❌ | **2.1–2.8s** ❌（沒變） |
| `.state/knowledge_index.json` | — | 54MB | 54MB |
| `.state` 總量 | — | 71MB（archive 16MB） | 71MB（archive 16MB） |
| spine 沒 commit 的檔 | 0 | **3** ❌ | **3** ❌（沒變） |
| spine remote | 有 | **無** ❌ | **無** ❌ |
| spine 事件檔數 | — | 8 | 8 |
| .app 大小 | < 30MB | 16MB（殼 10.8＋ctx 5.5） | 16MB |
| 常駐 `ctx mcp` | — | 4 支，各 6–10MB RSS，CPU 0% | 4 支，同 R01 |
| App 閒置 1 小時耗能 | 低 | 未量 | 未量 |
| CI | 有 | **無** ❌ | **無** ❌ |
| Rust 測試數 | 增加 | 312 | 312 |
| 開場推送準度（有意義的新料／全部） | ≥ 2/3 | **0/3**（共同詞是雜訊 bigram）❌ | **0/3** ❌（同樣 3 筆連兩輪被推，沒消失） |
| 開場注入大小 | ≤ 3,000 字元 | 未量 | 2,219 字元（中文 605 字） |
| Windows／VDI 測過的功能數 | 全部 | 0 | 0 |
| 量測時留下的假會話檔 | 0 | — | **2**（已手動刪除，見 N-14） |
