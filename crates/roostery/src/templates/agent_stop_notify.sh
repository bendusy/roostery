#!/usr/bin/env bash
# CC / Codex SessionEnd hook 共用脚本：从 stdin 读 hook JSON，调 roostery dispatcher fire。
# Phase 3 hooks-merge 落地，Phase 4 dispatcher 起来后才能正常工作；
# Phase 3 期间触发会拿到 clap "unknown subcommand" 错误，末尾 `|| true` 吞掉
# 不阻塞 agent runtime。
#
# 设计依据（同 Python parity，sh 解析 stdin → 抽 summary → 调底层入口）：
# - CC `SessionEnd` 是每会话一次（agent 退出时）；stdin 没有 last_assistant_message
# - 拿最终 agent 回复要 tail transcript_path jsonl 最后一条 assistant message
set -u

HOOK_JSON="$(cat || true)"

extract() {
  local key="$1"
  if command -v jq >/dev/null 2>&1; then
    printf '%s' "$HOOK_JSON" | jq -r --arg k "$key" '.[$k] // empty' 2>/dev/null
  fi
}

CWD="$(extract cwd)"
SESSION="$(extract session_id)"
TRANSCRIPT="$(extract transcript_path)"

# 优先 transcript_path tail；其次 prompt_response（Gemini 风格）；再次空
SUMMARY=""
if [ -n "${TRANSCRIPT:-}" ] && [ -f "${TRANSCRIPT:-}" ] && command -v jq >/dev/null 2>&1; then
  SUMMARY="$(tac "$TRANSCRIPT" 2>/dev/null \
    | jq -r 'select(.type=="assistant") | .message.content[0].text // empty' 2>/dev/null \
    | head -n 1 \
    | head -c 200)"
fi
if [ -z "${SUMMARY:-}" ]; then
  SUMMARY="$(extract prompt_response | head -c 200)"
fi

[ -z "${CWD:-}" ] && CWD="$PWD"

AGENT="${ROOSTERY_AGENT:-unknown}"

# 调 Rust dispatcher 入口；任何错误吞掉不阻塞 agent。
roostery dispatcher fire \
  --agent "$AGENT" \
  --session "${SESSION:-no-session}" \
  --cwd "$CWD" \
  --summary "${SUMMARY:-}" \
  >/dev/null 2>&1 || true
