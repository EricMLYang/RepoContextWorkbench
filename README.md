# repo_engine_prototype — 個人 repo 引擎原型

> **性質**：build-to-delete 原型，驗證 v2 規格（`../personal_agent_design/20260901_工具開發規格_v2.md`）的引擎層組得出來、驗證迴路轉得動。看過原型後才移植成獨立正式 repo；本資料夾不是最終家。
> **驗證設計先於程式碼**：見 `VERIFICATION.md`（四層驗證＋七條不變式＋回饋迴路）。

## 範圍

- **有**：M1 引擎核心（P1 registry+audit／P2 組／P3 採集／P6 打包簡版／P8 落／P10 脊椎三件套／P11 查詢+stats）＋借調 M2 的 P7 兩段式碰撞、P13 未讀、P15 provider（mock＋claude）＋早晨簡報產生器（樣貌 A 純事實版）。
- **無**：薄殼（tray/hotkey/toast/timer）、P4 上游、P5 digest、P9 spawn、P12 排程、MCP server——移植正式 repo 再做。P14 已有網頁雛形（`ui` 子命令，見 VERIFICATION.md UI 增補）。
- 對 v2 的小偏差：kv token 多一個 `id:`（collision 回連需要，v2 的 kv 清單為例示性）；pack 用內建文件層選料（正式版換 Repomix）。

## 快速開始

```powershell
$env:PYTHONUTF8 = "1"   # 必設（v2 §7：脊椎全中文，cp950 會炸）
cd repo_engine_prototype

# 自我驗證（agent 每次改完必跑；等同 scripts/check.ps1）
python -m pytest tests -q -m "not agent"

# 建 spine repo → 登記 → 建組
python -m repoengine init D:\some\spine-repo
python -m repoengine --spine D:\some\spine-repo registry add my-repo C:\path\to\repo
python -m repoengine --spine D:\some\spine-repo group add g1 my-repo

# 日常
python -m repoengine --spine ... collect --group g1     # P3 採集
python -m repoengine --spine ... brief   --group g1     # 早晨簡報（落 presented 事件）
python -m repoengine --spine ... collide submit "想法" --group g1          # fire-and-forget（detached）
python -m repoengine --spine ... collide submit "想法" --group g1 --wait --provider claude  # 同步真判定
python -m repoengine --spine ... unread                 # 殼 poll 的同一來源
python -m repoengine --spine ... query --today -v       # P11
python -m repoengine --spine ... query --stats          # 靈感命中率
python -m repoengine --spine ... registry audit         # P1 稽核紅字
python -m repoengine --spine ... commit                 # git 單一提交者（批次）
python -m repoengine --spine ... ui                     # S4 監控台雛形（localhost 網頁，事件列＋碰撞框＋深看）
```

`--spine` 可用環境變數 `REPOENGINE_SPINE` 取代。

## 驗證分層（詳見 VERIFICATION.md）

| 指令 | 層 | 誰跑 |
|---|---|---|
| `python -m pytest tests -q -m "not e2e and not agent"` | L1 單元 | agent |
| `python -m pytest tests -q -m e2e` | L2 CLI E2E | agent |
| `REPOENGINE_RUN_AGENT_TESTS=1` ＋ `python -m pytest tests -q -m agent` | L3 真 claude 冒煙 | 人觸發 |
| VERIFICATION.md「L4 驗收腳本」 | L4 真實資料驗收 | 人 |

## 架構速覽

```
repoengine/
  spine.py     P10 append（validator 拒寫＋跨行程 lock＋dead-letter）＋ P11 query/stats/unread
  registry.py  P1 registry+audit ＋ P2 組（含臨時組合）；寫入同一把 lock
  collect.py   P3 git 狀態（dirty 天數、末 commit 天數；external 只留 P4 stub 註記）
  pack.py      P6 文件層選料（由小到大裝預算，未納入列清單）
  brief.py     樣貌 A 三段結構產生器（閾值過濾、>3 條問句浮出警語）
  collide.py   P7 兩段式（submit 即落脊椎 → detached 進程判定 → 未讀回程）
  agents.py    P15 provider 介面（mock／claude -p --output-format json；內層驗證＋重試一次＋raw log）
  route.py     P8（group materials／repo 01_inbox／incubator）
  cli.py       全部原語的人用介面（正式版同組原語再開 MCP）
  webui.py     S4 監控台雛形（stdlib http.server、127.0.0.1 限定；/api/state 輕量輪詢、/api/scan 才打 git）
```

鐵律對應：引擎零私有字樣（有測試釘住）；git 只有 `commit` 子命令會動（單一提交者）；壞事件程式入口回錯誤、人的入口落 dead-letter（想法永不丟）。
