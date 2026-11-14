#!/usr/bin/env bash
# rebuild-app skill 的腳本：build → 裝到 /Applications（App 開著就留一個脫離會話的程序，等 ⌘Q 自動換好重開）
# → ctx setup --claude → 清掉只佔空間的 build 產物 → 印出裝了哪版、多了哪些 commit。
# 安裝本身交給 scripts/install-app.sh（備份、ditto、驗證都在那裡）；這裡只管順序、脫離會話與清理。
#
# 用法：bash .claude/skills/rebuild-app/rebuild.sh [--clean-only]
set -euo pipefail
root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$root"
export PATH="$HOME/.cargo/bin:$PATH"

CACHE="$HOME/.cache/repo-context"
DEST="/Applications/Repo Context.app"
BUILT="target/release/bundle/macos/Repo Context.app"
APP_PATTERN="MacOS/repo-[c]ontext-desktop"
mkdir -p "$CACHE"

say() { printf '%s\n' "$*"; }
size() { du -sh "$1" 2>/dev/null | cut -f1; }
# -a：在 App 的終端裡跑時 App 是上層行程，macOS 的 pgrep 預設不算上層
app_running() { pgrep -af "$APP_PATTERN" >/dev/null 2>&1; }

clean() {
  local before after
  before="$(size target)"
  if command -v cargo-sweep >/dev/null 2>&1; then
    cargo sweep --time 2 >/dev/null 2>&1 || say "（cargo sweep 失敗，略過）"
  else
    say "（沒裝 cargo-sweep：cargo install cargo-sweep）"
  fi
  # sweep 不管 incremental：2 天沒動的整個刪掉
  for d in target/debug/incremental target/release/incremental; do
    [ -d "$d" ] && find "$d" -mindepth 1 -maxdepth 1 -type d -mtime +1 -exec rm -rf {} + 2>/dev/null
  done
  # 裝完之後 bundle 目錄只剩空殼；還在等 ⌘Q 時那份要留著（install-app.sh 換好會自己刪）
  if [ ! -d "$BUILT" ] && [ -d target/release/bundle ]; then rm -rf target/release/bundle; fi
  # 快取目錄裡的一次性 log／腳本：7 天沒動就刪；App 會讀的三個檔與這次的 build.log 留著
  find "$CACHE" -maxdepth 1 -type f -mtime +7 \
    ! -name install.log ! -name install-status.json ! -name installed.json ! -name build.log \
    -delete 2>/dev/null || true
  after="$(size target)"
  say "清理：target ${before} → ${after}；App 舊版備份 $(size "$CACHE/app-backups")（install-app.sh 只留 3 份）"
}

if [ "${1:-}" = "--clean-only" ]; then
  clean
  exit 0
fi

PREV="$(sed -n 's/.*"commit":"\([0-9a-f]*\)".*/\1/p' "$CACHE/installed.json" 2>/dev/null || true)"
HEAD_C="$(git rev-parse --short=7 HEAD)"
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  say "注意：有還沒 commit 的改動，會一起 build 進去，但版本號仍標 ${HEAD_C}。"
fi

# 上一次還在等 ⌘Q 的安裝程序：這次會重新 build，舊的不要了
old="$(pgrep -f "install-app.sh.*--wait" || true)"
if [ -n "$old" ]; then
  # shellcheck disable=SC2086
  kill $old 2>/dev/null || true
  say "停掉上一次還在等的安裝程序（pid $(echo $old)）"
fi

say "1. build（ctx＋桌面 App）……"
if ! (cd desktop && npm run -s bundle -- --bundles app) >"$CACHE/build.log" 2>&1; then
  say "build 失敗，尾巴："
  tail -30 "$CACHE/build.log"
  exit 1
fi
cmp -s "$BUILT/Contents/MacOS/ctx" target/release/ctx || { say "bundle 裡的 ctx 跟 target/release/ctx 不同，停止"; exit 1; }
VER="$("$BUILT/Contents/MacOS/ctx" --version)"
say "   build 好了：${VER}"

if app_running; then
  # App 開著：不替使用者關（會結束 App 裡所有會話，包含可能正在跑這支腳本的那場）。
  # 留一個自成 session 的程序（App 結束時的 SIGHUP 打不到它）：等 ⌘Q → 換 → 重開 → ctx setup --claude。
  CTX_INSTALL_STATUS="$CACHE/install-status.json" perl -MPOSIX -e 'exit 0 if fork; POSIX::setsid(); exec @ARGV' \
    /bin/bash -c 'bash scripts/install-app.sh --prebuilt --wait --reopen && "$0/Contents/MacOS/ctx" setup --claude' "$DEST" \
    </dev/null >"$CACHE/install.log" 2>&1
  sleep 3
  if ! pgrep -f "install-app.sh.*--wait" >/dev/null; then
    say "安裝程序沒留住，log："
    cat "$CACHE/install.log"
    exit 1
  fi
  MODE=waiting
  say "2. App 開著：新版已備好，⌘Q 結束 App 後會自動換好並重新打開（log：${CACHE}/install.log）"
else
  say "2. 裝到 /Applications"
  bash scripts/install-app.sh --prebuilt >"$CACHE/install.log" 2>&1 || { cat "$CACHE/install.log"; exit 1; }
  "$DEST/Contents/MacOS/ctx" setup --claude >>"$CACHE/install.log" 2>&1 || say "（ctx setup --claude 失敗，看 ${CACHE}/install.log）"
  MODE=installed
  say "   裝好了：$("$DEST/Contents/MacOS/ctx" --version)；打開 App 就是新版"
fi

say "3. 清理"
clean

say ""
say "RESULT mode=${MODE} version=${HEAD_C} previous=${PREV:-?}"
if [ -n "$PREV" ] && git cat-file -e "${PREV}^{commit}" 2>/dev/null; then
  say "這版比上次裝的（${PREV}）多了："
  git log --format='--- %h %s%n%b' "${PREV}..HEAD" | grep -v '^Co-Authored-By' | grep -v '^$' || true
fi
