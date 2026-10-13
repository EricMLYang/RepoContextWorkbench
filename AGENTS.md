# AGENTS.md

## 目的
多 repo ＋ Agent ＋人協作的最佳工具：context 用 repo 管起來之後，讓人和 agent 都拿得到、用得上、留得下痕跡。
判斷取捨時以這三點為準：**更快拿到對的脈絡、判斷有留痕、人和 agent 走同一個介面**。

## 架構方向（2026-10-06 定）
- 已全面搬到 **Rust 引擎（`crates/ctx-core`）＋CLI／MCP（`crates/ctx-cli`，單一執行檔 `ctx`）＋Tauri 殼＋React 前端（`desktop/`）**，在 `rust-desktop` 分支。
- Python 版（`repo_context/`、`tests/`）只當對拍規格（`scripts/parity.sh`），合併回 main 後刪除；合併前不刪、不改它的行為。
- **資料格式就是合約**：spine（事件 md）、registry（YAML）、`.state/`、CLI、MCP 工具介面不隨語言變。規約與刻意差異見 `docs/RUST_PORTING.md`。
- 寫程式的準則（分層、錯誤、並行寫入、前端非同步、MCP 設計、開源缺口）見 `docs/DEVELOPMENT.md`。

## Rust
- 錯誤用 `Result` 往上傳；只有程式裡寫死的不變式才用 `unwrap`／`expect`。
- 型別表達狀態，不用字串或布林旗標拼湊。
- 引擎 crate 不依賴 Tauri；殼只做視窗、IPC、選單列、通知、PTY 轉接。
- 依賴從嚴：標準庫或主流 crate（serde、clap、portable-pty）能做的就不加新的。

## React
- 只負責畫面和互動，不放業務邏輯；資料一律從引擎取得，不在前端算。
- 每個非同步動作都要有：執行中鎖住按鈕、失敗時把錯誤顯示出來、斷線時保留上次的資料。
- 元件依一級物件拆：牌／收件匣／會話／深看。

## 桌面 App
- 輕：用系統內建 WebView，不用 Electron；閒置時不輪詢、不吃 CPU。
- 從 Finder 或 Dock 打開也找得到 `git`／`gh`／`claude`（啟動時自己補 PATH）。
- 本機服務只綁 127.0.0.1，並帶隨機 token。
- 會話（terminal）的生命週期獨立於視窗：關掉視窗不殺掉正在跑的 agent。
- macOS 先做，Windows（VDI）每個功能都要註明測過了沒有。
