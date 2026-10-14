# docs

| 文件 | 內容 | 什麼時候看 |
|---|---|---|
| `VISION.md` | 一開始的目的、現在的定義、核心概念與護欄、走到哪、接下來的方向 | 判斷要不要做某個功能、排優先序之前 |
| `DEVELOPMENT.md` | 寫程式的準則：分層、Rust／Tauri／React／MCP 重點、測試、資訊安全、開源缺口 | 寫程式之前 |
| `RUST_PORTING.md` | Python → Rust 的搬遷規約、合約、每一條刻意差異（合併回 main 後縮成「合約」一節） | 改到 spine／registry／CLI／MCP 的格式時 |
| `TESTING.md` | 要人點才測得到的手動測試清單（GUI、選單列、通知、互動式會話），每項分 macOS／Windows | 裝上新版後實測、合併回 main 之前 |
| `UIUX_REVIEW.md` | App 的 UI/UX 檢討（內容待補） | 檢討畫面與互動、排前端體驗改進之前 |
| `APP_ASSISTANT.md` | App 助手設計：一句話 → 計畫 → 確認 → 走同一組 action 改設定（換組、標籤…） | 做助手或開放設定類 MCP 工具之前 |
| `ideation/` | 構想池（`backlog.md`）、對標（`benchmarks.md`）、非功能基準（`baseline.md`）、最近一輪盤點（`rounds/`） | 跑 pm-ideation、挑下一個要做的功能時 |

IPC 合約在 `../desktop/IPC.md`；硬規定在 `../AGENTS.md`。

慣例：這裡只放**往後還會用到**的文件。單次檢討、落地記錄、截圖寫完就收斂回上面幾份，原始過程留在 git 歷史。
