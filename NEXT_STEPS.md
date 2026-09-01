# NEXT_STEPS — 原型看過後的下一步（2026-09-01，工作台輪後更新）

> 原型現況：**引擎原語 P1–P16＋MCP server＋IDE 工作台（內嵌 terminal）**；L1＋L2 全綠（98 tests）；L3/L4 待人。
> 詳見 `VERIFICATION.md`＋設計檔 `../personal_agent_design/20260901_工作台設計_v1.md`。
> 入口：`python -m repoengine --spine <spine> app`（桌面視窗；需 `pip install pywebview`）。

## 1. 跑 L4 驗收腳本（約 30 分鐘，`VERIFICATION.md` 有完整版）

```powershell
$env:PYTHONUTF8 = "1"
cd repo_engine_prototype
python -m repoengine init <某個空資料夾>                    # 建 spine repo
python -m repoengine --spine <spine> registry add <id> <路徑>  # 登記 3–5 個真實 repo
python -m repoengine --spine <spine> registry add <外部id> <路徑> --type external --upstream owner/repo
python -m repoengine --spine <spine> group add g1 <id1>,<id2>
python -m repoengine --spine <spine> collect --group g1     # 採集對不對？
python -m repoengine --spine <spine> upstream --group g1    # 上游查得到嗎（需 gh 登入）
python -m repoengine --spine <spine> digest  --group g1     # mock 摘要形狀對嗎（真摘要換 --provider claude）
python -m repoengine --spine <spine> brief   --group g1     # 簡報一屏內？問句形狀對？
python -m repoengine --spine <spine> ui                     # 監控台＋碰撞台＋深看
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

MCP 也可直接掛給 Claude Code 驗（P16 會自動產 `.mcp.json`）：

```powershell
python -m repoengine --spine <spine> session --group g1 --task "隨便問脊椎一件事"
# 或手動：在 spine repo 目錄開 claude，MCP tools（spine_append/collide_submit/…）應該掛上
```

## 3. 排程與通知的手感驗證（新）

- `config.yaml` 開 `schedule:`（範例在 scaffold 註解裡）→ `ui` 開著＝殼內 timer 生效；
  隔天早上看：簡報有沒有準時出現在 briefs/？錯過時段有沒有當日補跑、跨日不補？
- 弄壞一個排程任務（組名寫錯）→ 監控台該出現紅色 ⚠ system-unsure 事件＋瀏覽器通知（best-effort）。
- 每一次「這則不該吵我／怎麼沒告訴我」→ 修 `config.yaml` thresholds → 補一條 L1 閾值測試釘住。

## 4. 回饋收斂 → 決定移植

- 體驗回饋（該吵沒吵／不該吵卻吵、簡報形狀、判定可信度）記進 `VERIFICATION.md`「回饋迴路」段；每條回饋先補 failing test 再修。
- 滿意後移植成獨立正式 repo，屆時**只剩真殼**：tray 常駐、全域 hotkey、OS toast（S1/S2/S5）——
  引擎、MCP、timer、通知分類都已在原型驗過，殼只做「顯示與引信」。
- 移植前置動作別忘 v2 §3.5／M1 Day 0：spine repo 搬離雲同步資料夾、GitHub 政策確認、VDI 環境 smoke test
  （tray 常駐存活／hotkey 攔得到／toast 出得來／autostart 可設）。

---

## 歷史增補記錄

- **2026-09-01 監控台**：`ui` 子命令（事件列＋碰撞框＋組切換＋深看；僅綁 localhost；雙擊啟動 `監控台.cmd`）
- **2026-09-01 第二輪**：repo 勾選（P2 臨時組合進介面）＋ `registry remove`
- **2026-09-01 補完輪**：P4 upstream／P5 digest／P9 spawn／P12 timer／P13 notify／P16 session／MCP server；
  品味量測補存活率；open-loop 自動補預設 due；監控台 interrupt 置頂＋瀏覽器通知
