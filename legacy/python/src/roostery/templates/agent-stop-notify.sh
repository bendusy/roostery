#!/usr/bin/env bash
# CC / Codex SessionEnd hook 共用脚本：从 stdin 读 hook JSON，调 python 写飞书 Task。
# 详见 roostery/README.md + roostery/stop_hook.py。
#
# 设计依据（memory agent_hooks_facts.md）：
# - CC `Stop` 是 per-turn（每回合 fire 一次，10 turn = 10 次刷屏），不是"任务完成"
# - CC `SessionEnd` 才是每会话一次（agent 真正退出时）
# - SessionEnd stdin 没有 `last_assistant_message`，只有 session_id / transcript_path / cwd
# - 想拿最终 agent 回复，要 tail `transcript_path` 的 jsonl 最后一条 assistant message
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

# 优先 transcript_path tail；其次 prompt_response（Gemini CLI 风格）；再次空
SUMMARY=""
if [ -n "${TRANSCRIPT:-}" ] && [ -f "${TRANSCRIPT:-}" ] && command -v jq >/dev/null 2>&1; then
  # 倒序扫 jsonl，找第一条 type=assistant 的 message.content[0].text
  SUMMARY="$(tac "$TRANSCRIPT" 2>/dev/null \
    | jq -r 'select(.type=="assistant") | .message.content[0].text // empty' 2>/dev/null \
    | head -n 1 \
    | head -c 200)"
fi
if [ -z "${SUMMARY:-}" ]; then
  SUMMARY="$(extract prompt_response | head -c 200)"
fi

[ -z "${CWD:-}" ] && CWD="$PWD"

AGENT="${FEISHU_HUB_AGENT:-unknown}"

# 调 python 入口；任何错误吞掉不阻塞 agent。
# assignee 默认走 identity.resolve_user_open_id()——FEISHU_NOTIFY_TO env 优先，
# 缺则 lark-cli active profile 的 user_open_id，最后兜底 config.yaml.notify_receive_id。
python3 -m roostery.stop_hook \
  --agent "$AGENT" \
  --session "${SESSION:-no-session}" \
  --cwd "$CWD" \
  --summary "${SUMMARY:-}" \
  --assignee-open-id "${FEISHU_NOTIFY_TO:-}" \
  >/dev/null 2>&1 || true
