#!/usr/bin/env bash
# Python ↔ Rust 對拍：同一份 fixture 各跑 `python3 -m repo_context` 與 `target/debug/ctx`，
# 比 exit code、stdout（--json 解析後比）、寫出的 spine 檔與 registry.yaml。全部通過才 exit 0。
# 用法：scripts/parity.sh            （先 cargo build -p ctx-cli 再對拍）
#       CTX_BIN=/path/to/ctx scripts/parity.sh
set -euo pipefail
cd "$(dirname "$0")/.."
if [ -z "${CTX_BIN:-}" ]; then
  cargo build -q -p ctx-cli
fi
exec python3 scripts/parity.py "$@"
