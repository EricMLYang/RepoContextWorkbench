#!/bin/bash
# 對講（F-121）真 Claude Code 端到端：兩個 repo 各一場 agent，A 請 B 做事 → B 在 hook 收到、先問人 →
# 人同意（同一場 --continue）→ B 做完帶 re 回覆 → 回覆送回 A 那場（to_sid）、A 用完工具時收到。
#
# 用法：bash scripts/e2e_mailbox.sh [ctx 執行檔（預設 target/debug/ctx）]
#
# 花四次很小的 API 呼叫（haiku）。全部在暫存目錄：新 spine、兩個假 repo；不碰真 spine、不跑使用者的 hook
# （--setting-sources project＋--settings 只掛 ctx 的 hook，--strict-mcp-config 只連這個 spine 的 ctx mcp）。
# 會話 sid 用 REPO_CONTEXT_SESSION_STATUS 指定（跟 App 開的會話一樣），MCP server 繼承它＝知道是哪場在發話。
set -uo pipefail

HERE=$(cd "$(dirname "$0")/.." && pwd)
CTX=${1:-$HERE/target/debug/ctx}
command -v claude >/dev/null || { echo "找不到 claude 指令"; exit 2; }
[ -x "$CTX" ] || { echo "找不到 ctx：${CTX}（先 cargo build -p ctx-cli）"; exit 2; }

W=$(mktemp -d "${TMPDIR:-/tmp}/ctx-e2e-mail.XXXXXX")
trap 'rm -rf "$W"' EXIT
for r in ra rb; do
  mkdir "$W/$r" && git -C "$W/$r" init -q && echo "# $r" >"$W/$r/README.md"
  git -C "$W/$r" add -A && git -C "$W/$r" -c user.email=e2e@local -c user.name=e2e commit -qm init
done
"$CTX" init "$W/spine" >/dev/null || exit 2
"$CTX" --spine "$W/spine" registry add ra "$W/ra" >/dev/null
"$CTX" --spine "$W/spine" registry add rb "$W/rb" >/dev/null
"$CTX" --spine "$W/spine" group add g ra,rb >/dev/null
mkdir -p "$W/spine/.state/session_status"
h() { echo "[{\"hooks\":[{\"type\":\"command\",\"command\":\"'$CTX' hook $1\"}]}]"; }
echo "{\"hooks\":{\"PostToolUse\":$(h tool-end),\"UserPromptSubmit\":$(h prompt),\"Stop\":$(h stop)}}" >"$W/settings.json"
echo "{\"mcpServers\":{\"repo-context\":{\"command\":\"$CTX\",\"args\":[\"--spine\",\"$W/spine\",\"mcp\"]}}}" >"$W/mcp.json"

run() { # <repo> <sid> <tools> <prompt> [--continue]
  (cd "$W/$1" && echo "$4" | env -u CLAUDECODE -u CLAUDE_CODE_CHILD_SESSION -u CLAUDE_CODE_SESSION_ID \
    -u CLAUDE_CODE_ENTRYPOINT REPO_CONTEXT_SESSION_STATUS="$W/spine/.state/session_status/$2.json" \
    claude -p ${5:-} --model haiku --setting-sources project --settings "$W/settings.json" \
    --strict-mcp-config --mcp-config "$W/mcp.json" --allowedTools "$3" 2>&1 | grep -v '^Warning')
}
SEND=mcp__repo-context__send_message
echo "== A 請 B 做事"
run ra a1 "$SEND" "請用 repo-context 的 send_message 工具請 rb 這個 repo 的 agent 在它的 repo 根目錄建立 hello.txt，內容寫 hi from rb，做完回覆你。送出後就結束，不用等。"
echo "== B 用完工具時收到"
run rb b1 "Bash(ls:*) Write $SEND" "先用 Bash 執行 ls 看看目錄。"
echo "== 人同意，B 做完回覆"
run rb b1 "Bash(ls:*) Write $SEND" "好，照做。" --continue
echo "== A 用完工具時收到回覆"
out=$(run ra a1 "Bash(ls:*)" "用 Bash 執行 ls，然後告訴我有沒有收到 rb 的回覆、內容是什麼。")
echo "$out"

fail=0
check() { if eval "$2"; then echo "PASS $1"; else echo "FAIL $1"; fail=1; fi; }
book="$W/spine/.state/mailbox.json"
check "B 建了檔" '[ "$(cat "$W/rb/hello.txt" 2>/dev/null)" = "hi from rb" ]'
check "請求經 hook 送到 b1" 'python3 -c "import json,sys;m=json.load(open(sys.argv[1]))[\"messages\"][0];sys.exit(0 if m[\"delivered\"][\"sid\"]==\"b1\" and m[\"delivered\"][\"via\"]==\"hook\" else 1)" "$book"'
check "回覆帶 re、指定回 a1、送到 a1" 'python3 -c "import json,sys;ms=json.load(open(sys.argv[1]))[\"messages\"];r=[m for m in ms if m.get(\"re\")];sys.exit(0 if r and r[0][\"to_sid\"]==\"a1\" and r[0][\"delivered\"][\"sid\"]==\"a1\" else 1)" "$book"'
check "spine 留痕兩筆〔對講〕" '[ "$(cat "$W"/spine/spine/events/*.md | grep -c "〔對講〕")" = 2 ]'
exit $fail
