#!/bin/bash
# 真 Claude Code 端到端煙霧測試（未結 #7）：開場注入 → context_for → handoff → 結束，全部在 spine 副本上。
#
# 用法：
#   bash scripts/e2e_claude.sh <spine 路徑> [repo 目錄（預設本 repo）] [ctx 執行檔（預設 target/release/ctx）]
#
# 會花一次很小的 API 呼叫（haiku）。隔離做法——不碰真的 spine、不跑使用者其他 hook：
#   --setting-sources project   不載使用者設定（~/.claude/settings.json 的 hook、其他工具的 hook 都不跑）
#   --settings <檔>             只掛 ctx 的 hook，帶 --spine <副本>
#   --strict-mcp-config         只連 --mcp-config 給的 ctx mcp（--spine <副本>）
#   --allowedTools              只開 context_for、handoff 兩個 MCP 工具（agent 不能改檔）
# repo 目錄只當 cwd（開場注入看它在哪個範圍），不會被寫入。
set -uo pipefail

SPINE=${1:?用法：bash scripts/e2e_claude.sh <spine 路徑> [repo 目錄] [ctx 執行檔]}
HERE=$(cd "$(dirname "$0")/.." && pwd)
REPO=${2:-$HERE}
CTX=${3:-$HERE/target/release/ctx}
command -v claude >/dev/null || { echo "找不到 claude 指令"; exit 2; }
[ -x "$CTX" ] || { echo "找不到 ctx：${CTX}（先 cargo build --release -p ctx-cli）"; exit 2; }

WORK=$(mktemp -d "${TMPDIR:-/tmp}/ctx-e2e.XXXXXX")
COPY="$WORK/spine"
rsync -a "$SPINE"/ "$COPY"/ || exit 2
# 副本不能推到真的 remote
for r in $(git -C "$COPY" remote 2>/dev/null); do git -C "$COPY" remote remove "$r"; done
STATUS="$COPY/.state/session_status/e2e.json"
count_markers() { ls "$COPY/.state/hook_sessions" 2>/dev/null | wc -l | tr -d ' '; }
count_missed() { cat "$COPY"/spine/events/*.md 2>/dev/null | grep -c '沒有交接'; }
before=$(count_markers)
missed_before=$(count_missed)
count_handoffs() { grep -c '"tool":"handoff".*"ok":true' "$COPY/.state/metrics.jsonl" 2>/dev/null; }
handoffs_before=$(count_handoffs)

python3 - "$CTX" "$COPY" "$WORK" <<'EOF'
import json, sys
c, e, w = sys.argv[1:]
h = lambda ev: [{"hooks": [{"type": "command", "command": f"'{c}' --spine '{e}' hook {ev}"}]}]
json.dump({"hooks": {"SessionStart": h("session-start"), "SessionEnd": h("session-end"),
                     "Stop": h("stop"), "UserPromptSubmit": h("prompt")}},
          open(f"{w}/settings.json", "w"))
json.dump({"mcpServers": {"repo-context": {"type": "stdio", "command": c, "args": ["--spine", e, "mcp"]}}},
          open(f"{w}/mcp.json", "w"))
EOF

NEXT="E2E 下一步：$(date +%H%M%S)"
reply=$(cd "$REPO" && REPO_CONTEXT_SESSION_STATUS="$STATUS" claude -p \
  "你在 repo-context 的端到端測試裡。請依序：1) 呼叫 mcp__repo-context__context_for（不帶參數），2) 呼叫 mcp__repo-context__handoff，done='E2E 測試', next_step='$NEXT'。最後只回覆你在開場注入裡看到的工作範圍名稱。" \
  --model haiku --setting-sources project --settings="$WORK/settings.json" \
  --mcp-config="$WORK/mcp.json" --strict-mcp-config \
  --allowedTools "mcp__repo-context__context_for,mcp__repo-context__handoff" </dev/null 2>&1)
echo "agent 回覆：$reply"

fail=0
check() { if eval "$2"; then echo "PASS $1"; else echo "FAIL $1"; fail=1; fi; }
check "交接寫進 spine" "grep -rq '下次從這裡接：$NEXT' '$COPY/spine/events'"
check "MCP handoff 成功（多一筆 ok）" "[ \"\$(count_handoffs)\" -gt ${handoffs_before:-0} ]"
check "SessionEnd 清掉開場標記" "[ \"\$(count_markers)\" -eq $before ]"
check "沒誤記「沒交接」" "[ \"\$(count_missed)\" -eq $missed_before ]"
check "Stop hook 寫了會話狀態" "grep -q '\"event\":\"stop\"' '$STATUS'"
check "agent 有回覆" "[ -n \"\$reply\" ]"
echo "副本與設定在：${WORK}（看完可刪）"
[ $fail -eq 0 ] && echo "E2E PASS" || echo "E2E FAIL"
exit $fail
