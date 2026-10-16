#!/bin/bash
# 畫面預覽截圖（開發用）：起 dev server、用 headless Chrome 把 desktop/dev/preview.html 的每個場景截圖、關掉 server。
# 使用者的 App 開著、或只想看排版時用；看不到 Tauri 才有的東西（終端、檔案監看、原生對話框）。
#
# 用法：
#   bash scripts/preview-shots.sh [輸出目錄（預設 $TMPDIR/ctx-preview-shots）] [寬,高（預設 1440,900）] [場景…]
#
# 不碰使用者的瀏覽器：--headless=new、獨立的 --user-data-dir（暫存）、不開視窗、不送全域按鍵。
# 找不到 Chrome 就設 CHROME=<執行檔路徑>。
set -uo pipefail

HERE=$(cd "$(dirname "$0")/.." && pwd)
OUT=${1:-${TMPDIR:-/tmp}/ctx-preview-shots}
SIZE=${2:-1440,900}
shift $(( $# > 2 ? 2 : $# ))
SCENES=("$@")
if [ ${#SCENES[@]} -eq 0 ]; then
  SCENES=(workspace editor code md-view assist assist-empty palette composer tree-new rename panel-info term-menu tablist edit-find no-session past-sessions diff-notes ended-session reply-box)
fi
CHROME=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
[ -x "$CHROME" ] || { echo "找不到 Chrome：$CHROME（設 CHROME=…）"; exit 2; }
PORT=5199

mkdir -p "$OUT"
PROFILE=$(mktemp -d "${TMPDIR:-/tmp}/ctx-preview-chrome.XXXXXX")
cd "$HERE/desktop" || exit 2
npx vite --port $PORT --strictPort > "$OUT/vite.log" 2>&1 &
VITE=$!
trap 'kill $VITE 2>/dev/null; rm -rf "$PROFILE"' EXIT

# 等 dev server 起來（最多 20 秒）
for _ in $(seq 1 40); do
  curl -s -o /dev/null "http://localhost:$PORT/dev/preview.html" && break
  sleep 0.5
done

fail=0
for sc in "${SCENES[@]}"; do
  png="$OUT/$sc.png"
  "$CHROME" --headless=new --disable-gpu --no-first-run --user-data-dir="$PROFILE" \
    --hide-scrollbars --window-size="$SIZE" --virtual-time-budget=6000 \
    --screenshot="$png" "http://localhost:$PORT/dev/preview.html?scene=$sc" > /dev/null 2>&1
  if [ -s "$png" ]; then echo "✓ $png"; else echo "✗ $sc 沒截到"; fail=1; fi
done
exit $fail
