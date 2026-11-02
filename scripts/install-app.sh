#!/usr/bin/env bash
# 一鍵重裝桌面 App（ideation F-49）：build → 驗 ctx → 備份舊版 → 用 ditto 整包換到 /Applications → 清 quarantine
# → 刪掉 build 目錄裡那份（Spotlight 只剩一個）→ 驗證。
#
# 用法：
#   bash scripts/install-app.sh            # 正式裝
#   bash scripts/install-app.sh --dry-run  # 只列會做什麼，不 build、不動任何檔
#   bash scripts/install-app.sh --prebuilt # 已經 build 好（App 開著時先 build）：跳過 build，只驗證與替換
#   bash scripts/install-app.sh --wait --reopen
#                                          # App 開著也先 build；build 好之後等你結束 App（⌘Q）才換，換好自動重開
#                                          # （App 的「N 個改動沒裝」→「背景 build…」就是這樣叫的，F-118）
#
# 環境變數 CTX_INSTALL_STATUS＝狀態檔路徑：每個階段寫一行 JSON（building／waiting／installing／done／failed／expired），
# App 讀它顯示進度。
# 規則：
# - App 開著就停下來請你先結束（選單列「結束」或 ⌘Q），不替你關——關掉會結束正在跑的 agent 會話。
# - 用 ditto 整包替換：cp 改 .app 裡的檔會讓下次啟動因簽章不符崩潰。
# - 舊版備份在 ~/.cache/repo-context/app-backups/<時間>/，保留最近 3 份；要回退就把它 ditto 回 /Applications。
# - 已經開著的 Claude 會話仍用舊的 ctx mcp（行程沒重開）；開新會話才會用新版。
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="Repo Context.app"
# 後兩個環境變數只給測試用（裝到暫存目錄，不碰 /Applications）
DEST=${CTX_INSTALL_DEST:-"/Applications/$APP_NAME"}
BUILT="target/release/bundle/macos/$APP_NAME"
BACKUPS=${CTX_INSTALL_BACKUPS:-"$HOME/.cache/repo-context/app-backups"}
DRY=0
PREBUILT=0
WAIT=0
REOPEN=0
for a in "$@"; do
  case "$a" in
    --dry-run) DRY=1 ;;
    --prebuilt) PREBUILT=1 ;;
    --wait) WAIT=1 ;;
    --reopen) REOPEN=1 ;;
    *) printf '不認得的參數：%s\n' "$a"; exit 2 ;;
  esac
done
# 等你結束 App 最多等多久（秒）；測試可改
WAIT_MAX=${CTX_INSTALL_WAIT_MAX:-43200}
APP_PATTERN=${CTX_INSTALL_APP_PATTERN:-"MacOS/repo-[c]ontext-desktop"}
STATUS=${CTX_INSTALL_STATUS:-}
COMMIT="$(git rev-parse --short=7 HEAD 2>/dev/null || echo "?")"

say() { printf '%s\n' "$*"; }
# 狀態檔（給 App 看進度）：先寫暫存再改名，讀的人不會看到寫一半的
status() {
  [ -n "$STATUS" ] && [ "$DRY" = 0 ] || return 0
  mkdir -p "$(dirname "$STATUS")"
  printf '{"phase":"%s","at":"%s","pid":%s,"commit":"%s","detail":"%s"}\n' \
    "$1" "$(date '+%Y-%m-%d %H:%M')" "$$" "$COMMIT" "${2:-}" > "$STATUS.tmp" && mv "$STATUS.tmp" "$STATUS"
}
# 背景更新（--wait）時的系統通知：build 好了、失敗了——build 要幾分鐘，那時人多半在別的 App
notify() {
  [ "$WAIT" = 1 ] && [ "$DRY" = 0 ] || return 0
  osascript -e "display notification \"$1\" with title \"Repo Context\"" >/dev/null 2>&1 || true
}
DONE=0
trap 'rc=$?; [ "$DONE" = 1 ] || { status failed "第 ${STEP:-?} 步失敗（exit ${rc}）"; notify "背景更新失敗（第 ${STEP:-?} 步）：細節在 ~/.cache/repo-context/install.log"; }' EXIT
STEP=0
app_running() { pgrep -af "$APP_PATTERN" >/dev/null 2>&1; }
run() {
  if [ "$DRY" = 1 ]; then say "  （dry-run）$*"; else "$@"; fi
}

# -a：macOS 的 pgrep 預設不算自己的上層行程；在 App 的終端裡跑這個腳本時，App 正是上層，
# 不加就偵測不到、照樣把開著的 App 換掉（2026-10-22 發生過：檔案換成新版，跑的仍是舊版）。
# --wait：App 開著沒關係，build 完再等
if [ "$WAIT" = 0 ] && app_running; then
  say "Repo Context App 正在執行（$(pgrep -af "$APP_PATTERN" | head -1)）。"
  say "請先從選單列「結束」或按 ⌘Q 關掉它，再重跑這個腳本。（關掉會結束 App 裡正在跑的會話。）"
  say "或用 --wait：先 build，build 好了等你結束 App 再換。"
  [ "$DRY" = 1 ] || exit 1
fi

# 從 App 叫的背景更新（--wait）：原始碼有沒 commit 的改動就不 build——可能是 agent 改到一半，
# 裝進去的東西跟「N 個改動沒裝」列的 commit 對不起來。自己在終端跑（沒有 --wait）不擋。
if [ "$WAIT" = 1 ] && [ -n "$(git status --porcelain --untracked-files=no 2>/dev/null)" ]; then
  say "原始碼有還沒 commit 的改動，背景更新不 build（先 commit，或自己在終端跑 bash scripts/install-app.sh）。"
  status failed "原始碼有還沒 commit 的改動（先 commit 再更新）"
  notify "背景更新沒開始：原始碼有還沒 commit 的改動"
  DONE=1
  exit 1
fi

export PATH="$HOME/.cargo/bin:$PATH"
STEP=1
if [ "$PREBUILT" = 1 ] && [ -d "$BUILT" ]; then
  say "1. build：用已經 build 好的 ${BUILT}（--prebuilt）"
else
  say "1. build（ctx＋桌面 App）"
  status building
  run bash -c "cd desktop && npm run bundle -- --bundles app"
fi

STEP=2
say "2. 驗證 bundle 裡的 ctx 就是剛 build 的"
if [ "$DRY" = 0 ]; then
  cmp -s "$BUILT/Contents/MacOS/ctx" target/release/ctx || { say "bundle 裡的 ctx 跟 target/release/ctx 不同，停止"; exit 1; }
fi

if [ "$WAIT" = 1 ] && [ "$DRY" = 0 ]; then
  waited=0
  while app_running; do
    if [ "$waited" = 0 ]; then
      say "build 好了，等你結束 App（⌘Q）再換……"
      status waiting
      notify "新版 build 好了：⌘Q 結束 App 後會自動換好並重新打開"
    fi
    if [ "$waited" -ge "$WAIT_MAX" ]; then
      say "等了 $((WAIT_MAX / 3600)) 小時 App 都還開著，不換了（build 好的留在 ${BUILT}，之後可以 --prebuilt）。"
      status expired
      DONE=1
      exit 0
    fi
    sleep 2
    waited=$((waited + 2))
  done
fi
status installing

STEP=3
# 換掉之前記下舊版是哪個 commit（App 第一次打開新版時用它列「已更新：多了什麼」）：
# 新的 ctx --version 帶 commit；更舊的沒有，就用它的 build 時間找當時的 commit
PREV=""
if [ -x "$DEST/Contents/MacOS/ctx" ]; then
  PREV="$("$DEST/Contents/MacOS/ctx" --version 2>/dev/null | sed -n 's/.*(\([0-9a-f]\{7,\}\) .*/\1/p')"
  if [ -z "$PREV" ]; then
    built="$(stat -f %m "$DEST/Contents/MacOS/ctx" 2>/dev/null || true)"
    [ -n "$built" ] && PREV="$(git rev-list -1 --before="@$built" HEAD 2>/dev/null | cut -c1-7 || true)"
  fi
fi
TS="$(date +%Y%m%d-%H%M%S)"
say "3. 備份舊版 → $BACKUPS/$TS/"
if [ -d "$DEST" ]; then
  run mkdir -p "$BACKUPS/$TS"
  run ditto "$DEST" "$BACKUPS/$TS/$APP_NAME"
  # 只留最近 3 份
  if [ "$DRY" = 0 ]; then
    ls -1d "$BACKUPS"/*/ 2>/dev/null | sort -r | tail -n +4 | while read -r old; do rm -rf "$old"; done
  fi
else
  say "  （/Applications 還沒有舊版，略過）"
fi

STEP=4
# 剛好又被打開（等的期間你結束後馬上又開）：別把跑著的 App 刪掉
if [ "$DRY" = 0 ] && app_running; then
  say "App 又開著了，不換（build 好的留在 ${BUILT}，之後可以 --prebuilt）。"
  status failed "App 又開著了"
  DONE=1
  exit 1
fi
say "4. 安裝到 ${DEST}（ditto 整包替換）"
run rm -rf "$DEST"
run ditto "$BUILT" "$DEST"
run xattr -dr com.apple.quarantine "$DEST" 2>/dev/null || true

STEP=5
say "5. 刪掉 build 目錄裡那份（Spotlight 只會找到 /Applications 這一個）"
run rm -rf "$BUILT"

STEP=6
say "6. 驗證"
if [ "$DRY" = 0 ]; then
  "$DEST/Contents/MacOS/ctx" --version
  "$DEST/Contents/MacOS/ctx" where --json >/dev/null && say "  ctx 可以跑"
  readlink "$HOME/.local/bin/ctx" 2>/dev/null | grep -q "$DEST" \
    && say "  ~/.local/bin/ctx 指向新版" \
    || say "  注意：~/.local/bin/ctx 沒有指向 ${DEST}（終端機裡的 ctx 可能是別支）"
fi
say "完成。接著跑 ctx setup --claude（skill 與 hook 指令換成這版；ctx health 沒更新會一直提醒）。"
say "開新的 Claude 會話才會用新版 MCP；用幾天後跑 ctx health 跟 docs/VISION.md §6 的使用前基準比。"
status done
# 裝了哪版、換掉的是哪版（App 的「已更新」在沒有「上次看過」時用 previous）
RECORD=${CTX_INSTALL_RECORD:-"$HOME/.cache/repo-context/installed.json"}
if [ "$DRY" = 0 ]; then
  mkdir -p "$(dirname "$RECORD")"
  # 裝進去的是哪個 commit 以裝好的 ctx 自己說的為準（--prebuilt 時可能比現在的 HEAD 舊）
  NEW="$("$DEST/Contents/MacOS/ctx" --version 2>/dev/null | sed -n 's/.*(\([0-9a-f]\{7,\}\) .*/\1/p')"
  printf '{"commit":"%s","previous":"%s","at":"%s"}\n' "${NEW:-$COMMIT}" "$PREV" "$(date '+%Y-%m-%d %H:%M')" > "$RECORD"
fi
DONE=1
if [ "$REOPEN" = 1 ] && [ "$DRY" = 0 ]; then
  say "重新打開 App"
  open "$DEST"
fi
