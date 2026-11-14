---
name: rebuild-app
description: 重新 build 桌面 App 並直接處理到 /Applications，讓使用者重開 App 就是新版；要 commit 時把「重開後要測什麼」寫進 commit；build 完清掉只佔空間的產物。使用者說「重新 build」「裝新版」「更新 App」「build 一版給我測」「重 build 並 commit」時使用；改完會影響 App 或 ctx 的程式、使用者要實際看到結果時也用。
---

# rebuild-app：build → 裝到 /Applications → 清理

目標只有一個：**使用者重開 App 就看到新版，而且知道要測什麼**。不要停在「build 好了，請自己跑安裝腳本」。

## 順序

1. **要 commit 就先 commit，再 build。** ctx 的版本號帶 commit（`ctx 0.2.0 (abc1234 …)`），App 的「已更新：多了什麼」也照 commit 列；
   先 build 再 commit，裝進去的版本就對不上。使用者沒說要 commit 就不 commit，照樣 build（腳本會提醒有未 commit 的改動）。
2. **跑腳本**（約 1～3 分鐘，timeout 給 10 分鐘）：

   ```bash
   bash .claude/skills/rebuild-app/rebuild.sh
   ```

   它做完整件事：build → 驗 ctx → 裝到 `/Applications` → `ctx setup --claude` → 清理 → 印出這版比上次裝的多了哪些 commit。
3. **照腳本最後一行 `RESULT mode=…` 回報**（見下）。

## App 開著的時候

不要替使用者關 App：關掉會結束 App 裡所有會話，你自己這場多半也在裡面。腳本會自己判斷：

- `mode=installed`：App 沒開，已經換好。告訴使用者打開 App 就是新版。
- `mode=waiting`：App 開著。新版已 build 好，背景留了一個脫離會話的程序，**使用者按 ⌘Q 之後自動備份、換版、重開、跑 `ctx setup --claude`**。
  告訴使用者：先收好其他會話，按 ⌘Q，App 會自己重開成新版。
  這種情況**先 `handoff` 再回報**——⌘Q 之後這場會話就沒了。

沒換成功時看 `~/.cache/repo-context/install.log`；build 好的那份還在，`bash scripts/install-app.sh --prebuilt` 可以重跑。

## commit 要寫「重開後要測」

這次有 commit 的話，訊息本文照這個形狀（標題與其他慣例照 repo 既有的）：

```
feat(relay)：……一句話講做了什麼

- 改了什麼、為什麼（照舊）

重開後要測：
- <操作> → <應該看到什麼>
- <操作> → <應該看到什麼>
macOS GUI：<實測過／vitest 過，真 App 未實測>。Windows：未測。
```

- 每一項是使用者**在新版 App 或終端裡做得到的一個動作**＋看得到的結果；不寫「確認功能正常」這種無法照做的。
- 只列這個 commit 改到的；沒有畫面或行為變化（純重構、文件）就寫「重開後要測：無（行為不變）」。
- 超過 5 項，或要準備資料才測得了的：寫進 `docs/TESTING.md` 新的一節，commit 裡留最要緊的 2～3 項＋「其餘見 TESTING.md §X」。
- 已經自動測過的不列（單元測試、vitest、對拍），那是給 `macOS GUI：` 那一行講的。

## 回報給使用者

三件事，短：

1. 裝了哪版（commit）、現在是「已換好」還是「⌘Q 後自動換」。
2. **重開後要測什麼**：把腳本列出的「這版比上次裝的多了」各 commit 的「重開後要測」合成一張清單（沒寫的舊 commit 就從它的說明與
   `docs/TESTING.md` 對應節挑）。這是使用者下一步要照著做的，不要只丟 commit 列表。
3. 清理結果一行（target 前後大小）；沒清到東西就不特別講。

## 清理：留什麼、刪什麼

腳本每次都做，不用另外跑；只想清不想 build 時用 `rebuild.sh --clean-only`。

| 刪 | 留（刪了下次要重編或回不去） |
|---|---|
| 2 天沒被任何 build 用到的 `target/` 產物（`cargo sweep --time 2`） | 這兩天用到的 `target/debug`、`target/release`（增量編譯靠它） |
| 2 天沒動的 `target/*/incremental/*` | `desktop/node_modules`、`desktop/dist` |
| 裝完後的 `target/release/bundle/`（已經在 /Applications 了） | 等 ⌘Q 期間的那份 bundle（換好會自己刪） |
| `~/.cache/repo-context/` 裡 7 天沒動的一次性 log 與腳本 | `install.log`、`install-status.json`、`installed.json`、最近 3 份 App 備份 |

不要 `cargo clean`（下次重編相依 5～10 分鐘）。清完 `target` 還超過 15 GB 才跟使用者提 `cargo clean --profile dev`，由他決定。

## 不做的事

- 不直接 `cp`／`ditto` 蓋掉開著的 App（簽章不符下次啟動會崩；檔案換了跑的仍是舊版）。
- 不自己寫另一支等 App 結束的一次性腳本丟在 `~/.cache`——用這支。
- Windows 沒有這條路（`install-app.sh` 只有 macOS）；照 README 手動裝，並在 commit 註明 Windows 未測。
