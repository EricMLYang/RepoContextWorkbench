# NEXT_STEPS — 原型看過後的下一步（2026-09-01）

> 原型現況：L1＋L2 全綠（49 tests）；L3/L4 待人。詳見 `VERIFICATION.md`。

## 1. 跑 L4 驗收腳本（約 20 分鐘，`VERIFICATION.md` 有完整版）

```powershell
$env:PYTHONUTF8 = "1"
cd repo_engine_prototype
python -m repoengine init <某個空資料夾>                    # 建 spine repo
python -m repoengine --spine <spine> registry add <id> <路徑>  # 登記 3–5 個真實 repo
python -m repoengine --spine <spine> group add g1 <id1>,<id2>
python -m repoengine --spine <spine> collect --group g1     # 採集對不對？
python -m repoengine --spine <spine> brief   --group g1     # 簡報一屏內？問句形狀對？
python -m repoengine --spine <spine> unread                 # 動作有留痕嗎
```

## 2. 開 L3 真 agent 冒煙（會花 claude 額度，預設 skip）

```powershell
$env:REPOENGINE_RUN_AGENT_TESTS = "1"
python -m pytest tests -q -m agent
```

或直接對真實組撞一次看判定五欄位可信度：

```powershell
python -m repoengine --spine <spine> collide submit "一個真想法" --group g1 --wait --provider claude
```

## 3. 回饋收斂 → 決定移植

- 體驗回饋（該吵沒吵／不該吵卻吵、簡報形狀、判定可信度）記進 `VERIFICATION.md`「回饋迴路」段；每條回饋先補 failing test 再修。
- 滿意後移植成獨立正式 repo，屆時才做：MCP server、薄殼（tray/hotkey/toast/S6 timer）、P4 上游、P5 digest、P9 spawn、P12 排程、P14 監控台。
- 移植前置動作別忘 v2 §3.5／M1 Day 0：spine repo 搬離雲同步資料夾、GitHub 政策確認、VDI 環境 smoke test。
