#!/usr/bin/env bash
# 改完必跑的全套檢查（docs/DEVELOPMENT.md §7）：Rust fmt／clippy／test／Python 對拍，前端 build／test／lint／prettier。
# 用法：scripts/check.sh [rust|front|all]（預設 all）。任一步失敗就印那一步的尾巴、最後 exit 1；全過印 exit=0。
# 每步輸出先收進暫存檔，過了就不印（一次約 10 分鐘，大多是 cargo test 與對拍）。
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2
what=${1:-all}
fail=0
log=$(mktemp "${TMPDIR:-/tmp}/ctx-check.XXXXXX")
trap 'rm -f "$log"' EXIT
step() {
  local name=$1
  shift
  echo "== $name"
  if ! "$@" >"$log" 2>&1; then
    echo "FAIL（${name}）"
    tail -40 "$log"
    fail=1
  fi
}
if [ "$what" != front ]; then
  step fmt cargo fmt --all --check
  step clippy cargo clippy -q --workspace --all-targets -- -D warnings
  step test cargo test -q --workspace
  step parity scripts/parity.sh
fi
if [ "$what" != rust ]; then
  cd desktop || exit 2
  step build npm run -s build
  step vitest npm test --silent
  step lint npm run -s lint
  step prettier npm run -s format:check
fi
# 磁碟衛生（docs/DEVELOPMENT.md §7）：build 產物只留最近 7 天用到的；沒裝 cargo-sweep 就提醒一句、不算失敗。
if [ "$what" != front ]; then
  cd "$(dirname "$0")/.." || exit 2
  if command -v cargo-sweep >/dev/null 2>&1; then
    cargo sweep --time 7 >/dev/null 2>&1 || echo "（cargo sweep 失敗，略過）"
  else
    echo "（沒裝 cargo-sweep：target/ 會越長越大，cargo install cargo-sweep 後這裡會自動掃）"
  fi
fi
echo "exit=$fail"
exit $fail
