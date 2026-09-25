"""S4 工作台（三個一級物件：牌／收件匣／會話；2026-09-02 UI 檢討後重排）。

2026-09-01 裁定：要工作台不要玩具監控頁，內嵌 terminal 直接與 agent 對話。
2026-09-02 檢討（`docs/20260902_工作台UI_UX檢討.md`）：畫面曾是
「監控頁黏 terminal」——行動出口是字不是按鈕、三種待辦格式疊加、會話無主。本輪重排：
- 牌（sidebar）：組是一張卡；臨時組合＝未命名卡；repo 微標 icon 化
- 收件匣（main）：統一卡片 {key, kind, title, lines, actions}，出口是真按鈕→呼叫引擎原語
  →落 chosen 後卡消失；自己丟的碰撞＝「處理中」卡，回程原地換判定卡；
  自己出手的留痕（presented/chosen/monitor decision）不進收件匣
- 會話（sidebar 下段＋terminal 面板）：每個會話有主（origin＝哪張卡／哪次碰撞），title＝任務摘要
- 早晨簡報＝收件匣的三段折疊視圖；「存成今日簡報」落 presented

殼原則（v2 §1）不變：
- /api/state  輕量輪詢（純脊椎/registry 檔案讀）＝「殼只 poll 脊椎未讀」
- /api/scan   才跑 P3 採集（開頁/切組/勾選/手動），高頻輪詢不打 git subprocess
- 所有行動按鈕都只是呼叫引擎原語再落脊椎；terminal 會話生命週期獨立於視窗

安全（檢討④ VDI multi-session：localhost 埠跨 session 共享）：
- 只綁 127.0.0.1 ＋ 每次啟動隨機 token——URL 帶 token 才服務（/static 除外）；
  pywebview 視窗拿完整 URL，其他本機使用者猜不到
- WS（terminal）同樣驗 token；WS 實作為 stdlib 手寫 RFC6455 最小子集（零依賴）

2026-09-12 UX 檢討落地（工作連續性）：
- /api/scan 多帶 `summary`（本組工作摘要：目標／上次進度／變化／可接續的一步，每格帶來源）
- /api/state 的會話清冊多帶 `report`：會話開始後、範圍內的最後一筆脊椎留痕——
  沒有留痕就是「尚無回報」，程序存活不當成進度（§5）
- 新增 action：set_goal（組目標）、session_draft（交接草稿）、scan_dir／register_repos（首次建組）


2026-09-25 架構檢查：拆成 state（畫面資料）／actions（行動分派）／ws／server，
registry 與組的寫入改走操作表（ops.py）。這裡只把公開名稱接出去，呼叫端不用改。
"""
from .actions import ACTIONS, handle_action
from .actions import handle_action as _handle_action  # 舊名（測試與既有呼叫端）
from .server import make_server, serve, serve_window
from .state import (PAGE_PATH, STATIC_DIR, annotate_terms, build_cards, build_scan,
                    build_state, event_scopes, normalize_dest, session_draft, session_report)
from .ws import ws_accept_key, ws_encode, ws_read_frame
