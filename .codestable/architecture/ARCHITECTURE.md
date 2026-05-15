# 🪺 Roostery 架构总入口

> 状态：骨架（从 CLAUDE.md 抽取）
> 创建日期：2026-05-15

## 1. 项目简介

**Roostery** — vendor-neutral, Feishu-native agent broker。本地 daemon，将任意 agent runtime（Claude Code / Codex / Gemini / OpenClaw / 自定义 Python）桥接到飞书（Lark）作为协作面。

当前阶段：planning（v0.0.0），代码来自 prior `feishu_hub` baseline（M3.C → M5.A，~7339 LOC，681 tests upstream）。

Python 包布局，`package.json` + `index.js` 占住 npm namespace；npm 侧是占位，所有真实代码在 `src/roostery/` 下的 Python。

## 2. 核心概念 / 术语表

> **Feishu 是共享 state machine。`lark-cli` 是 agent 对 Feishu 的 syscall 面。Roostery 只是执行桥 + 本地审计缓存。**

| 概念 | 含义 |
|------|------|
| Agent runtime | Claude Code / Codex / Gemini / OpenClaw / 自定义 Python 等本地 agent 进程 |
| lark-cli | 与飞书通信的唯一 sanctioned subprocess wrapper（pin 在 1.0.28） |
| Dispatcher | 本地事件 → 规则匹配 → runner 执行的桥接层 |
| Journal | 本地 jsonl 审计日志（`~/.feishu_hub/`），仅作 replayable audit |
| Trace | `trace_id` / `depth` / `parent_event_id` 链，loop 保护用 |
| Budget | 调用次数与成本上限 |

### State ownership

| State | Owner |
|---|---|
| Work-item lifecycle、agent step stream | Feishu Task (`lark-cli task +create` / `append_task_steps`) |
| 跨 agent live context | Feishu IM thread (`lark-cli im +messages-reply --thread`) |
| Comments / collab traces | Feishu Docs comments、group chat |
| Index / stats / dashboard | Feishu Base（索引层，**非** source of truth） |
| 云侧路由（@mention / cron） | Feishu Base Workflow（`LarkMessageTrigger` / `TimerTrigger`） |
| 本地进程 / 模型调用 / budget | Local（`dispatcher.runners`, `dispatcher.budget`） |
| Audit / replay | 本地 journal jsonl (`journal.py`) |

## 3. 子系统 / 模块索引

源码在 `src/roostery/`，详细 red-line 见 `src/roostery/README.md`。

- **`lark_cli.py`** — 稳定的 `lark-cli` subprocess wrapper（JSON 解析、异常归一化）。与飞书通信的唯一 sanctioned 入口。
- **Shim & audit**
  - `shim.py` — PATH-prefix shim，透明代理真 `lark-cli` 并写 journal
  - `journal.py`、`redact.py`、`remoterefs.py`（从 stdout 抽取 `doc_token` / `record_id` 等）
- **本地 config / 安装**
  - `config.py` — `~/.feishu_hub/config.yaml`
  - `hooks_merge.py` — 合并 Stop hooks 到 `~/.claude/settings.json` / `~/.codex/hooks.json`
  - `onboarding.py`、`identity.py`、`templates/`
- **Bot bridge（M3.B 主路径）**
  - `task_writer.py` — 创建 Feishu task + append step stream + session cache
  - `stop_hook.py` — shell→python 桥，task_writer 优先 IM 兜底
  - `bot_runner.py`、`bot_bridge.py`、`bot_relay_task.py`、`bot_role.py`、`hitl_router.py`
- **`dispatcher/`** — 本地执行桥（M3.A 后已轻量化）
  - `cli.py` — `fire` / `replay` / `test-rule`
  - `loop.py` — event → match rules → trace/budget gate → run runner → emit
  - `rules.py` — 本地 hook → runner 匹配
  - `runners.py` — `cc_headless` / `codex_exec` / `gemini_headless` / `noop`
  - `trace.py` — loop 保护链
  - `budget.py` — call-count + 成本上限
- **Reporting**
  - `git_log.py`（多仓聚合）
  - `llm_summary.py`（**唯一**允许 import GA-style llmcore client 的模块）
  - `daily_report.py`、`record_writer.py`
- **其他**：`agent_detect.py`、`base_config.py`、`base_indexer.py`、`base_intent_router.py`、`event_bridge.py`、`runner_registry.py`

## 4. 关键架构决定

1. **vendor-neutral 桥而非 SDK**。Roostery 不替代 agent runtime，也不替代 Feishu，它只做转换 + 审计。
2. **Feishu = source of truth**。本地是 cache / audit，不是 canonical 协作记录。
3. **lark-cli 是唯一飞书入口**。不允许新增 HTTP client 直连 `open.feishu.cn`。
4. **dispatcher hook-agnostic**。新 hook 源（Codex / Gemini / Cursor）通过 `hooks_merge.py` + `templates/` 扩展，loop 不感知 provider。
5. **`llm_summary.py` 是 LLM provider 集成的唯一白名单**。其他模块保持 vendor-neutral。

## 5. 已知约束 / 硬边界

1. **禁止重实现 lark-cli**。飞书有 API 就走 `lark_cli.py`，不准 `requests` 打 `open.feishu.cn`。
2. **本地 state 是 cache 不是真相**。`~/.feishu_hub/` 下任何东西都只是可重放的审计。若发现某段代码靠读本地 state 来回答"任务 X 现在状态如何"——那是 bug。
3. **`llm_summary.py` 是 GA-style llmcore / mykey client import 的唯一允许位置。**
4. **lark-cli 版本 pin 在 1.0.28**（特别是 `task agent_task_step_info append_task_steps` timestamp schema 的兼容）。升级需先跑 smoke。
5. **`python -m roostery smoke` 是升级后的 gate**。它跑验证过的命令矩阵（`im +messages-send`、`docs +create v2`、`docs +update overwrite`、`drive files list / +create-folder / move`）。任意 probe 失败，`init` 和 `daily_report` 拒绝运行。
