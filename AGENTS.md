# AGENTS.md

## 目的
多 repo ＋ Agent ＋人協作的最佳工具：context 用 repo 管起來之後，讓人和 agent 都拿得到、用得上、留得下痕跡。
判斷取捨時以這三點為準：**更快拿到對的脈絡、判斷有留痕、人和 agent 走同一個介面**。

## 架構方向（2026-10-06 定）
- 最終形態：**Rust 引擎＋Tauri 殼＋React 前端**，產出單一執行檔（CLI／MCP／桌面 App 同一支）。
- 過渡期：Python 引擎當 sidecar，依模組穩定度逐塊搬到 Rust，**穩定的先搬，常改的後搬**。
- **資料格式就是合約**：spine（JSONL）、registry（YAML）、MCP 工具介面不隨語言變；搬遷期間兩邊讀寫同一份資料。
- 每搬一個模組，把對應測試一起搬過去，兩邊都跑過同樣的資料再切換。

## Rust
- 錯誤用 `Result` 往上傳；只有程式裡寫死的不變式才用 `unwrap`／`expect`。
- 型別表達狀態，不用字串或布林旗標拼湊。
- 引擎 crate 不依賴 Tauri；殼只做視窗、IPC、選單列、通知、PTY、管理 sidecar。
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
