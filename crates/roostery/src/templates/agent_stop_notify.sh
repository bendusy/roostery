#!/usr/bin/env bash
# CC / Codex / Gemini SessionEnd 极简 wrapper：stdin JSON 直透给
# `roostery bot stop-hook`，Rust 端原生处理（解析 / transcript tail / push
# 到飞书 / IM 兜底）。
#
# 安装：`roostery init` 写到 `~/.roostery/scripts/agent_stop_notify.sh`
# 触发：CC/Codex/Gemini stop hook JSON 里命令 `ROOSTERY_AGENT=cc <path>`
# 退出：始终 0，不阻塞 agent runtime（错误观察走 ~/.roostery/journal/）
set -u
ROOSTERY_AGENT="${ROOSTERY_AGENT:-unknown}" exec roostery bot stop-hook
