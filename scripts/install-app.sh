#!/usr/bin/env bash
# 一鍵重裝桌面 App（ideation F-49）：build → 驗 ctx → 備份舊版 → 用 ditto 整包換到 /Applications → 清 quarantine
# → 刪掉 build 目錄裡那份（Spotlight 只剩一個）→ 驗證。
#
# 用法：
#   bash scripts/install-app.sh            # 正式裝
#   bash scripts/install-app.sh --dry-run  # 只列會做什麼，不 build、不動任何檔
#   bash scripts/install-app.sh --prebuilt # 已經 build 好（App 開著時先 build）：跳過 build，只驗證與替換
#
# 規則：
# - App 開著就停下來請你先結束（選單列「結束」或 ⌘Q），不替你關——關掉會結束正在跑的 agent 會話。
# - 用 ditto 整包替換：cp 改 .app 裡的檔會讓下次啟動因簽章不符崩潰。
# - 舊版備份在 ~/.cache/repo-context/app-backups/<時間>/，保留最近 3 份；要回退就把它 ditto 回 /Applications。
# - 已經開著的 Claude 會話仍用舊的 ctx mcp（行程沒重開）；開新會話才會用新版。
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="Repo Context.app"
DEST="/Applications/$APP_NAME"
BUILT="target/release/bundle/macos/$APP_NAME"
BACKUPS="$HOME/.cache/repo-context/app-backups"
DRY=0
PREBUILT=0
[ "${1:-}" = "--dry-run" ] && DRY=1
[ "${1:-}" = "--prebuilt" ] && PREBUILT=1

say() { printf '%s\n' "$*"; }
run() {
  if [ "$DRY" = 1 ]; then say "  （dry-run）$*"; else "$@"; fi
}

if pgrep -f "MacOS/repo-[c]ontext-desktop" >/dev/null 2>&1; then
  say "Repo Context App 正在執行（$(pgrep -f "MacOS/repo-[c]ontext-desktop" | head -1)）。"
  say "請先從選單列「結束」或按 ⌘Q 關掉它，再重跑這個腳本。（關掉會結束 App 裡正在跑的會話。）"
  [ "$DRY" = 1 ] || exit 1
fi

export PATH="$HOME/.cargo/bin:$PATH"
if [ "$PREBUILT" = 1 ] && [ -d "$BUILT" ]; then
  say "1. build：用已經 build 好的 ${BUILT}（--prebuilt）"
else
  say "1. build（ctx＋桌面 App）"
  run bash -c "cd desktop && npm run bundle -- --bundles app"
fi

say "2. 驗證 bundle 裡的 ctx 就是剛 build 的"
if [ "$DRY" = 0 ]; then
  cmp -s "$BUILT/Contents/MacOS/ctx" target/release/ctx || { say "bundle 裡的 ctx 跟 target/release/ctx 不同，停止"; exit 1; }
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

say "4. 安裝到 ${DEST}（ditto 整包替換）"
run rm -rf "$DEST"
run ditto "$BUILT" "$DEST"
run xattr -dr com.apple.quarantine "$DEST" 2>/dev/null || true

say "5. 刪掉 build 目錄裡那份（Spotlight 只會找到 /Applications 這一個）"
run rm -rf "$BUILT"

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
