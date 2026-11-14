#!/usr/bin/env bash
# 漸進揭露題組（F-126）：只在真資料的 spine 副本上手動跑，不進 check.sh。
# 每題跑完整模式與精簡模式，印「期望 repo｜完整 files 第一名｜完整 repos 第一名｜精簡 answer_repo｜精簡字元數」，最後印命中數；
# 再用舊版 ctx（PATH 上的，或 CTX_OLD）跑一次完整模式當基準。
#
# 用法：bash scripts/search_eval.sh <spine 副本>
#   CTX_NEW=target/debug/ctx（預設）  CTX_OLD=$(command -v ctx)（預設；沒有就略過基準）
# 只讀：search 只寫 .state/ 的索引快取。不要對真 spine 跑（它會寫快取）。
set -euo pipefail
SPINE="${1:?用法：bash scripts/search_eval.sh <spine 副本>}"
cd "$(dirname "$0")/.."
NEW="${CTX_NEW:-target/debug/ctx}"
OLD="${CTX_OLD:-$(command -v ctx || true)}"
export REPOCTX_NO_BACKGROUND=1

# 題目｜期望 repo（逗號分隔＝都算對）
QUESTIONS=(
  "golden set 會議紀錄 Agent 15 題|card_notes"
  "排水縱斷面圖 verifier 高程|draftagent"
  "主角館 字庫 識字動畫|character-play-studio"
  "ISO 42001 稽核 起始會議腳本|iso-ai-audit"
  "ir_calendar 爬蟲分類 IT Brand TV Brand|databricks_coding,MI_Crawler"
  "週會 Forecast 參考頁提案|PM_Head"
  "SAP-C02 考題 Agentic AI Lens|AWS_StudyGroup"
  "mob 腳本 harness 讀書會 kickoff|AgentStudyGroup"
  "SRS 卡 旅館 check-in 日文情境|language-learning"
  "風險分層 品質門檻 Agent Coding 推動策略|AgentCodingPM"
  "會議紀錄 呼應 Sales Brief Databricks Job|MI3Agents"
  "PTY 多工作區 terminal aggregator|better-agent-terminal"
)

# 從 JSON 取欄位（stdin）：files 第一名、repos 第一名、answer_repo
pick() {
  python3 -c '
import json, sys
mode = sys.argv[1]
try:
    v = json.load(sys.stdin)
except Exception:
    print("（壞 JSON）"); sys.exit()
if mode == "files":
    f = v.get("files") or []
    print(f[0]["repo"] if f else "—")
elif mode == "repos":
    r = v.get("repos") or []
    print(r[0]["repo"] if r else "—")
else:
    a = v.get("answer_repo")
    print(a["repo"] if a else "—")
' "$1"
}

hit() { # $1=答案 $2=期望（逗號）
  local IFS=,
  for e in $2; do [ "$1" = "$e" ] && { echo 1; return; }; done
  echo 0
}

run_set() { # $1=binary $2=標籤 $3=要不要跑精簡（1/0）
  local bin="$1" label="$2" brief="$3"
  local n_files=0 n_repos=0 n_brief=0
  echo "## ${label}（${bin}）"
  if [ "${brief}" = 1 ]; then
    echo "期望｜完整 files 第一名｜完整 repos 第一名｜精簡 answer_repo｜精簡字元數"
  else
    echo "期望｜完整 files 第一名｜完整 repos 第一名"
  fi
  local i=0
  for item in "${QUESTIONS[@]}"; do
    i=$((i + 1))
    local q="${item%%|*}" want="${item#*|}"
    local full
    full=$("${bin}" --spine "${SPINE}" search "${q}" --json 2>/dev/null || echo '{}')
    local f r
    f=$(printf '%s' "${full}" | pick files)
    r=$(printf '%s' "${full}" | pick repos)
    n_files=$((n_files + $(hit "${f}" "${want}")))
    n_repos=$((n_repos + $(hit "${r}" "${want}")))
    if [ "${brief}" = 1 ]; then
      local b a chars
      b=$("${bin}" --spine "${SPINE}" search "${q}" --brief --json 2>/dev/null || echo '{}')
      a=$(printf '%s' "${b}" | pick answer)
      chars=$(printf '%s' "${b}" | python3 -c 'import json,sys; print(len(json.dumps(json.load(sys.stdin), ensure_ascii=False, separators=(",", ":"))))')
      n_brief=$((n_brief + $(hit "${a}" "${want}")))
      echo "${i}. ${want}｜${f}｜${r}｜${a}｜${chars}"
    else
      echo "${i}. ${want}｜${f}｜${r}"
    fi
  done
  if [ "${brief}" = 1 ]; then
    echo "命中：完整 files ${n_files}/${#QUESTIONS[@]}、完整 repos ${n_repos}/${#QUESTIONS[@]}、精簡 answer_repo ${n_brief}/${#QUESTIONS[@]}"
  else
    echo "命中：完整 files ${n_files}/${#QUESTIONS[@]}、完整 repos ${n_repos}/${#QUESTIONS[@]}"
  fi
  echo
}

if [ -n "${OLD}" ]; then
  run_set "${OLD}" "舊版基準（完整模式）" 0
fi
run_set "${NEW}" "新版" 1
