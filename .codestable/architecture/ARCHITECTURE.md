# 🪺 Roostery 架构总入口

> 状态：active（Rust 重写期更新）
> 创建日期：2026-05-15
> 末次刷新：2026-05-15（rust-scaffold feature 落地时）

## 1. 项目简介

**Roostery** — vendor-neutral, Feishu-native agent broker。本地 daemon，将任意 agent runtime（Claude Code / Codex / Gemini / OpenClaw / 自定义 Python）桥接到飞书（Lark）作为**跨设备 vibecoding 协作面**。核心动机见 `.codestable/brainstorms/v0.x-direction/`。

**阶段**：Rust 重写中（自 2026-05-15）。仓库未发布任何版本——首个 0.1.0 等到 Rust 达到"可用"形态（roadmap Phase 5 完成）。

**目录布局**：

- `crates/roostery/` — Rust workspace 单 member crate，**活跃代码**（Phase 0 起逐步搭建）
- `legacy/python/` — prior `feishu_hub` baseline 归档（M3.C → M5.A，~7339 LOC），**仅作 reference，不维护**；Phase 7 `legacy-removal` 删
- `.codestable/` — CodeStable 规范体系（attention / req / arch / roadmap / brainstorm / feature / compound）
- `.github/workflows/ci.yml` — fmt / clippy / test 三 job

## 2. 核心概念 / 术语表

> **Feishu 是共享 state machine。`lark-cli` 是 agent 对 Feishu 的 syscall 面。Roostery 只是执行桥 + 本地审计缓存。**

| 概念 | 含义 |
|------|------|
| Agent runtime | Claude Code / Codex / Gemini / OpenClaw / 自定义 Python 等本地 agent 进程 |
| `lark-cli` | 与飞书通信的唯一 sanctioned subprocess wrapper（pin 在 1.0.28） |
| `LarkRunner` trait | Rust 期 lark-cli wrapper 的抽象接口，下游所有模块依赖 trait 而非具体 struct（见 roadmap §4.1） |
| Dispatcher | 本地事件 → 规则匹配 → runner 执行的桥接层（Module E，Phase 4） |
| Journal | 本地 jsonl 审计日志（`~/.feishu_hub/`），仅作 replayable audit + portable data |
| `JournalEntry` schema | journal 单行结构，是 `portable-by-default` req 的公开契约（见 roadmap §4.2） |
| Trace | `trace_id` / `depth` / `parent_event_id` 链，loop 保护用（见 roadmap §4.5） |
| Budget | 调用次数与成本上限 |
| Roost | 项目名含义——agent 来此栖息，离开时带着自己的痕迹走（不锁定在某一协作平面） |

### State ownership

| State | Owner |
|---|---|
| Work-item lifecycle、agent step stream | Feishu Task (`lark-cli task +create` / `append_task_steps`) |
| 跨 agent live context | Feishu IM thread (`lark-cli im +messages-reply --thread`) |
| Comments / collab traces | Feishu Docs comments、group chat |
| Index / stats / dashboard | Feishu Base（索引层，**非** source of truth） |
| 云侧路由（@mention / cron） | Feishu Base Workflow（`LarkMessageTrigger` / `TimerTrigger`） |
| 本地进程 / 模型调用 / budget | Local（Rust：`dispatcher::runners` / `dispatcher::budget`，Phase 4） |
| Audit / replay | 本地 journal jsonl（Rust：`journal` 模块，Phase 1） |

## 3. 子系统 / 模块索引

按 roadmap rust-rewrite §3 聚成 8 个模块。详细 feature 拆解和接口契约见 `.codestable/roadmap/rust-rewrite/`。

> Phase 0（rust-scaffold，本 feature）落地时 `crates/roostery/src/` 仅有 `main.rs` + `lib.rs`。下表是 **target architecture**，每个 Phase 的 feature 落地时实际 Rust 文件才出现。

### Module A · 基础工具（Phase 1）
纯数据操作。`schema` 常量、`redact`（敏感字段脱敏）、`remoterefs`（regex 抽 `doc_token` / `record_id`）。
- 子 feature：`rust-scaffold` / `core-redact` / `core-remoterefs`

### Module B · 本地审计 / Journal（Phase 1）
本地 jsonl audit / replay。`JournalEntry` schema 是 `portable-by-default` req 的契约载体（公开、稳定、可移植）。
- 子 feature：`journal-core`

### Module C · 飞书 Syscall（Phase 2）
飞书通信的唯一 sanctioned 通道。`LarkRunner` trait + 默认 subprocess 实现 + `roostery smoke` + `bin/shim` 二进制。
- 子 feature：`lark-cli-wrapper` / `roostery-smoke` / `lark-cli-shim`

### Module D · 本地配置与安装（Phase 3）
bootstrap `~/.feishu_hub/`、merge Stop hooks 进 `~/.claude/settings.json` / `~/.codex/hooks.json`、装 shim、识别 agent runtime、嵌入模板。
- 子 feature：`config-yaml` / `hooks-merge` / `roostery-init`

### Module E · Dispatcher（Phase 4）
本地执行桥。event → 规则匹配 → trace/budget gate → runner → emit。`runtime-neutral` req 的执行机制（通过 `Runner` trait 调度，不感知具体 runtime）。
- 子 feature：`dispatcher-trace-budget` / `dispatcher-rules` / `dispatcher-runners` / `dispatcher-loop`

### Module F · Bot Bridge（Phase 5）
agent run → Feishu task card + step stream + IM thread。**`agent-work-in-feishu` req 的直接兑现层**。`bot-stop-hook` feature 完成 = "Rust 可用" milestone = 0.1.0 触发判据。
- 子 feature：`bot-task-writer` / `bot-stop-hook` / `bot-bridge-cluster`

### Module G · Reporting（Phase 6）
日报：git log 聚合 + LLM 摘要 + 写飞书 docx + Base 记录。`llm_summary` 是**唯一**允许 import 外部 LLM client 的模块（架构红线）。Cargo feature flag 控制。
- 子 feature：`report-git-llm` / `report-daily`

### Module H · Base Index（Phase 7）
Feishu Base 作为索引层（**非** source of truth）。
- 子 feature：`base-indexer`

### 终态切换（Phase 7）
- 子 feature：`legacy-removal`（删 `legacy/python/`、重写 README、crates.io 准备）

## 4. 跨模块接口契约

7 个契约在 `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §4 定义，是 feature-design 的硬约束输入：

| # | 契约 | 方向 | Phase 落地 |
|---|---|---|---|
| 4.1 | `LarkRunner` trait | E/F/G/H → C | Phase 2 |
| 4.2 | `JournalEntry` schema | C/E/F 写 → 用户/社区读 | Phase 1 |
| 4.3 | `Runner` trait | E → 具体 runner | Phase 4 |
| 4.4 | `HookEvent` schema | D/E → E | Phase 3-4 |
| 4.5 | `TraceContext` | E → F → C | Phase 4 |
| 4.6 | Config schema | D 写 → 所有读 | Phase 3 |
| 4.7 | 模板嵌入约定 | D → 用户文件系统 | Phase 3 |

## 5. 关键架构决定

1. **vendor-neutral 桥而非 SDK**。Roostery 不替代 agent runtime，也不替代 Feishu，它只做转换 + 审计
2. **Feishu = default view，不是 lock-in**。本地是 cache / audit，journal 是 portable 数据形态——飞书出问题 / 想换前端，能基于 journal 重建（兑现 `portable-by-default` req）
3. **`lark-cli` 是唯一飞书入口**。不允许新增 HTTP client 直连 `open.feishu.cn`
4. **dispatcher hook-agnostic**。新 hook 源（Codex / Gemini / Cursor）通过 `hooks_merge` + 模板嵌入扩展，loop 不感知 provider
5. **`llm_summary` 模块是 LLM provider 集成的唯一白名单**。其他模块保持 vendor-neutral

## 6. 已知约束 / 硬边界

> 完整 9 条硬约束见 `.codestable/attention.md`——每次 CodeStable 子技能启动自动加载。

1. **禁止重实现 lark-cli**。飞书 API 必经 `lark_cli` wrapper；不准 `reqwest` / `requests` 打 `open.feishu.cn`，也不引 Feishu SDK
2. **本地 state 是 cache 不是真相**。`~/.feishu_hub/` 下任何东西都只是可重放的审计，不回答"任务 X 现在状态如何"
3. **`llm_summary` 是外部 LLM client import 的唯一允许位置**
4. **lark-cli 版本 pin 在 1.0.28**（`task append_task_steps` timestamp schema 兼容）。升级需先跑 smoke
5. **smoke 是升级后的 gate**。任意 probe 失败 `roostery init` 和 `daily_report` 拒绝运行
6. **代码-文档优先级**：Python baseline 与最新文档冲突时**以文档为准**（见 attention.md）。Rust port 不机械 1:1 翻译，失配点记观察项
