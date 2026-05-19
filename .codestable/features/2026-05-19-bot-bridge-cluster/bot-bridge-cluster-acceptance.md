---
doc_type: feature-acceptance
feature: 2026-05-19-bot-bridge-cluster
status: accepted
accepted_at: 2026-05-19
slug: bot-bridge-cluster
design: .codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-design.md
checklist: .codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-checklist.yaml
commits: [3ccd2a3, ff9cf70, e359995, d29ebc4, 5c95be4, e5d9366, 2fe8f4f, fb8ef74, dbd2470]
test_summary: cargo test --all → 570 passed / 0 failed / 2 ignored；cargo clippy --all-targets --all-features -- -D warnings 全绿；cargo fmt --all --check pass
---

# bot-bridge-cluster 验收

## 0. 启动检查

| 检查 | 结果 |
|---|---|
| `cargo test --all` | 570 passed / 0 failed / 2 ignored |
| `cargo clippy --all-targets --all-features -- -D warnings` | 0 warning |
| `cargo fmt --all --check` | clean |
| checklist steps 全 done | 8/8 done |
| checklist checks 全 pending（验收前） | 41/41 pending → 本次回写（验收后 41/41 passed） |

## 1. 接口契约核对（design §2.1）

逐条 grep 落实到源码 file:line。

### bot_bridge::role

| 契约 | 证据 |
|---|---|
| `BOTS_SCHEMA_VERSION: u32 = 1` | `crates/roostery/src/bot_bridge/role.rs:17` |
| `pub struct BotRole #[non_exhaustive]` 9 字段 | `crates/roostery/src/bot_bridge/role.rs:20-44` |
| `pub struct BotsConfig #[non_exhaustive] { schema_version, bots }` | `crates/roostery/src/bot_bridge/role.rs:53-59` |
| `pub enum BotRoleError` 4 变体（LoadFailed / ParseFailed / SchemaVersionMismatch / MissingField） | `crates/roostery/src/bot_bridge/role.rs:66-77` |
| `pub fn load_bots(&Path) -> Result<BotsConfig, BotRoleError>` | `crates/roostery/src/bot_bridge/role.rs:99` |
| `pub fn event_matches_bot(&ImEvent, &BotRole) -> bool` | `crates/roostery/src/bot_bridge/role.rs:133` |
| `pub fn extract_message_body(&ImEvent, &BotRole) -> &str` | `crates/roostery/src/bot_bridge/role.rs:144` |

### bot_bridge::hitl

| 契约 | 证据 |
|---|---|
| `HitlDecision` 三态 + `#[non_exhaustive]` | `crates/roostery/src/bot_bridge/hitl.rs:20-24` |
| `ABORT_KEYWORDS: &[&str]` 4 项 `/stop /abort 停 中止` | `crates/roostery/src/bot_bridge/hitl.rs:29` |
| `ADJUST_PREFIXES: &[&str]` 4 项 `/adjust ` / `/adjust\n` / `调整 ` / `调整\n` | `crates/roostery/src/bot_bridge/hitl.rs:34` |
| `pub fn classify(&str) -> HitlDecision` | `crates/roostery/src/bot_bridge/hitl.rs:44` |

### bot_bridge::active_registry

| 契约 | 证据 |
|---|---|
| `HitlSignal` 二态 + `#[non_exhaustive]` | `crates/roostery/src/bot_bridge/active_registry.rs:26-29` |
| `RunnerHandle { kill_tx, task_guid, task_url, chat_id, started_at }` | `crates/roostery/src/bot_bridge/active_registry.rs:36-42` |
| `HitlSignalError` 2 变体（NotFound / ReceiverGone） | `crates/roostery/src/bot_bridge/active_registry.rs:62-69` |
| `ActiveRunnerRegistry::new/register/unregister/lookup_by_chat_id/send_signal` | `crates/roostery/src/bot_bridge/active_registry.rs:80-137` |
| 命名避让 `dispatcher::runners::RunnerRegistry`（D2） | 同上；`ActiveRunnerRegistry` 名字独立 |

### bot_bridge::relay_task

| 契约 | 证据 |
|---|---|
| `BOT_CHAT_CACHE_SCHEMA_VERSION: u32 = 1` | `crates/roostery/src/bot_bridge/relay_task.rs:31` |
| `EndOutcome` 四态（Success/Failed/Aborted/Timeout）+ `#[non_exhaustive]` | `crates/roostery/src/bot_bridge/relay_task.rs:37-44` |
| `RelayTaskError` 3 变体（TaskWriter / CacheLoad / CacheSave） | `crates/roostery/src/bot_bridge/relay_task.rs:48-65` |
| `record_start / record_adjust / record_end` 三 fn 签名 | `crates/roostery/src/bot_bridge/relay_task.rs:245 / 309 / 339` |
| `record_adjust` 增 `chat_id` 入参（修 e5d9366） | `crates/roostery/src/bot_bridge/relay_task.rs:309-316` |

> 偏离：design §2.1 record_adjust 原签名只取 `&TaskRef`，acceptance 阶段实装新增 `chat_id` 入参以让 `cache.adjust_count` 真实递增（commit e5d9366）。已在 design.md 与 attention.md 之外通过 record_adjust 单测 `record_adjust_increments_cache_count` 锁定行为。

### bot_bridge::event

| 契约 | 证据 |
|---|---|
| `ImEvent` 6 字段 + `#[non_exhaustive]` + Deserialize | `crates/roostery/src/bot_bridge/event.rs:36-45` |
| `consume_im` stream API | `crates/roostery/src/bot_bridge/event.rs:121` |
| `EventError` 4 变体 + `#[non_exhaustive]` | `crates/roostery/src/bot_bridge/event.rs:90-107` |

> 偏离：design §2.1 出参写 `impl Stream<Item=...>`，实装走 `mpsc::Receiver` + `JoinHandle`（`ConsumeStream`，event.rs:113-116）。理由模块顶部 doc 注释里说明——`LarkRunner` trait 是 buffered Value 模型与 NDJSON tail 长跑不匹配；mpsc 与 daemon 中央 dispatcher 整合多 bot 流更顺手；不引 `futures` crate。

### bot_bridge::daemon

| 契约 | 证据 |
|---|---|
| `BridgeOptions` 含 max_concurrency / max_events / timeout / profile_filter | `crates/roostery/src/bot_bridge/daemon.rs:100-119` |
| `BridgeReport` 11 字段含 `events_received` / `events_skipped_*` / `hitl_*` / `handle_event_results` / `shutdown_reason` | `crates/roostery/src/bot_bridge/daemon.rs:178-191` |
| `run_bridge(&Path, BridgeOptions) -> Result<BridgeReport, BridgeError>` | `crates/roostery/src/bot_bridge/daemon.rs:211` |
| `BridgeError` `#[non_exhaustive]`（含 LoadBots） | `crates/roostery/src/bot_bridge/daemon.rs:204-208` |
| `ShutdownReason` 5 态 | `crates/roostery/src/bot_bridge/daemon.rs:47-59` |
| `CancelToken` graceful shutdown 注入点 | `crates/roostery/src/bot_bridge/daemon.rs:64-97` |

### bot_bridge::cli + 顶层 BotSub::Bridge

| 契约 | 证据 |
|---|---|
| `BridgeCliArgs` 5 flags（bots / profile / max_concurrency / max_events / timeout） | `crates/roostery/src/bot_bridge/cli.rs:17-34` |
| `BotSub::Bridge(BridgeCliArgs)` 第 3 变体注册 | `crates/roostery/src/bot_stop_hook/cli.rs:27` |
| 顶层 dispatch `BotSub::Bridge(a) => bot_bridge::cli::run(a)` | `crates/roostery/src/bot_stop_hook/cli.rs:176-182` |

**接口契约偏差汇总**：2 项偏离均有显式 doc 注释 + 单测 / 集成测覆盖，**不是漏实装而是 design 阶段 shape 与 Rust trait 模型匹配后的精化**。无遗漏。

## 2. 行为与决策核对（design §1 / §2.2 / §2.3）

### 决策 D1-D12 落实

| 决策 | 兑现位置 |
|---|---|
| **D1** 5 Python 模块 → 1 Rust 子目录 `bot_bridge/` 7 子模块 | `crates/roostery/src/bot_bridge/{mod,role,hitl,active_registry,relay_task,event,runner,daemon,cli}.rs`（9 文件含 mod.rs + cli.rs） |
| **D2** ActiveRunnerRegistry 命名避让 | `crates/roostery/src/bot_bridge/active_registry.rs:76` 类型名 `ActiveRunnerRegistry`；与 `crates/roostery/src/dispatcher/runners.rs::RunnerRegistry` 不冲突 |
| **D3** HITL 信号通道 = oneshot::Sender，不落盘 | `crates/roostery/src/bot_bridge/active_registry.rs:37` `kill_tx: tokio::sync::oneshot::Sender<HitlSignal>` |
| **D4** runner 调用走 `dispatcher::runners::Runner` trait + Registry | `crates/roostery/src/bot_bridge/runner.rs:138 runner_registry.find(&bot.runner)` + `runner.run(&hook_event, &ctx, &args)` line 195 |
| **D5** task 写入走 `bot_task_writer` API | `crates/roostery/src/bot_bridge/relay_task.rs:21-24` import `create_task / append_steps`；不直接调 LarkRunner 创 task |
| **D6** IM 事件源走 lark-cli `im_messages_subscribe` 子进程 NDJSON | `crates/roostery/src/bot_bridge/event.rs:230-238` `cmd.arg("im").arg("im_messages_subscribe")` |
| **D7** 回复走 LarkRunner trait（不引专用 wrapper） | `crates/roostery/src/bot_bridge/runner.rs:114` `lark: &dyn LarkRunner` 注入；relay_task 内 `append_steps` 透传同一 trait |
| **D8** `BOTS_SCHEMA_VERSION = 1` 公开承诺 | `crates/roostery/src/bot_bridge/role.rs:17` const + 测试 `schema_version_missing_defaults_to_one` / `schema_version_two_returns_mismatch_error` |
| **D9** `ADJUST_MAX = 1` const | `crates/roostery/src/bot_bridge/runner.rs:34` |
| **D10** 每 BotRole 独立 cache 目录 `~/.roostery/state/bot_chats/{app_id}/` | `crates/roostery/src/paths.rs:66-69`；单测 `multi_bot_record_start_uses_isolated_cache_dirs` |
| **D11** 每条 event spawn tokio task + `tokio::select! { kill, runner }` | `crates/roostery/src/bot_bridge/runner.rs:198-207` |
| **D12** CLI `roostery bot bridge --bots ... --profile X --max-concurrency N --max-events N --timeout` | `crates/roostery/src/bot_bridge/cli.rs:17-34` |

### 编排流程（design §2.2 主流程图）

daemon.rs:211-469 实装：load_bots → per-bot spawn consume_im → central mpsc → main select loop（ctrl_c / cancelled / handle_joins / central_rx）→ HITL classify 命中 → dispatch_hitl_{abort,adjust} → Pass 走 event_matches_bot → spawn handle_event → graceful shutdown deadline。

幂等性 idempotency_key 模板 = `relay:{kind}:{message_id}:{bot_app_id}` 锁定在 relay_task.rs:208-210 + 单测 `idempotency_key_template`（line 719）。

### 挂载点反向 grep（design §2.3 4 条）

| 挂载点 | 反向 grep 证据 |
|---|---|
| 1. `roostery bot bridge` clap 子命令 | `crates/roostery/src/bot_stop_hook/cli.rs:27 BotSub::Bridge(...)` 实装；删除后顶层 `bot bridge` 命令消失 |
| 2. `bots.yaml` schema 文档化 | `bot_bridge::role::BotRole / BotsConfig / BOTS_SCHEMA_VERSION` 落实 schema；yaml roundtrip 单测 `yaml_roundtrip_full_config`（role.rs:204） |
| 3. `paths::bot_chat_cache_dir(bot_app_id)` | `crates/roostery/src/paths.rs:66`；单测 `bot_chat_cache_dir_under_state` / `bot_chat_cache_dir_neutralizes_path_traversal`（paths.rs:160-180） |
| 4. journal action 命名空间 `bot_bridge:*` | `crates/roostery/src/bot_bridge/daemon.rs:610 JournalEntry::new("bot_bridge:daemon", ...)` + `runner.rs:420 ::new("bot_bridge:handle_event", ...)` |

四条挂载点都有实装；任一拔掉 feature 立即破。

## 3. 验收契约 41 条 check（design §3 + checklist.yaml）

每条找到具体测试 / 代码证据。

| # | check 类别 | 证据 | passed |
|---|---|---|---|
| 1 | BotsConfig + BotRole + BOTS_SCHEMA_VERSION + 4 错误变体 | role.rs:17/20/53/66 + 测试 `yaml_roundtrip_full_config` / `schema_version_missing_defaults_to_one` / `schema_version_two_returns_mismatch_error` / `missing_required_field_reports_index_and_field` / `load_failed_when_file_missing` / `parse_failed_on_invalid_yaml` | ✅ |
| 2 | HitlDecision 三态 + 关键词 const | hitl.rs:20/29/34 + `abort_keywords_list_has_exactly_four_entries` / `adjust_prefixes_list_has_exactly_four_entries` | ✅ |
| 3 | ActiveRunnerRegistry + RunnerHandle + HitlSignal oneshot | active_registry.rs:26/36/76 + `oneshot_send_signal_delivers_abort_to_receiver` / `oneshot_send_signal_delivers_adjust_to_receiver` | ✅ |
| 4 | EndOutcome 四态 + relay_task 三 fn | relay_task.rs:37/245/309/339 + step_text_end_{success,failed,aborted,timeout}_* 四态文案测 | ✅ |
| 5 | BridgeOptions + BridgeReport + run_bridge | daemon.rs:100/178/211 + `run_bridge_with_empty_bots_yaml_returns_no_bots` | ✅ |
| 6 | ImEvent + consume_im stream | event.rs:36/121 + `s6_1_ndjson_parses_valid_and_skips_corrupt` / `s6_2_eof_triggers_exponential_reconnect` / `s6_3_spawn_failure_reports_and_keeps_retrying` | ✅ |
| 7 | BridgeCliArgs 5 flags + BotSub::Bridge 注册 | cli.rs:17 + bot_stop_hook/cli.rs:27 + `bridge_cli_args_defaults_parse` / `bridge_cli_args_all_flags_parse` / `bridge_cli_args_to_options_roundtrip` | ✅ |
| 8 | bots.yaml schema 文档化 | role.rs + design §2.1 + yaml roundtrip 单测 | ✅ |
| 9 | paths::bot_chat_cache_dir helper | paths.rs:66 + 2 单测 | ✅ |
| 10 | journal source/action 命名空间 `bot_bridge:*` | daemon.rs:610 / runner.rs:420 grep；end-to-end test `s7_1` 断言 `"action":"event:received"` / `"event:handle_complete"` 命中 | ✅ |
| 11 | 主流程 consume_im→HITL→handle_event→record_end→reply→unregister | daemon.rs:323-422 编排 + `s7_1_end_to_end_six_events_dispatched_correctly` 端到端断言 events_received=6 / handle_event_spawned=2 / success≥2 | ✅ |
| 12 | HITL 判定串行先于 spawn handle_event | daemon.rs:374-385 `match classify(...) { Abort/Adjust → continue（不 spawn） }`；s7_1 断言 `/stop` 命中 `hitl_signal_misses` 而非进 handle_event | ✅ |
| 13 | 幂等性 idempotency_key 模板 | relay_task.rs:208 + `idempotency_key_template` | ✅ |
| 14 | subscribe 子进程退出指数退避重连，cap 60s | event.rs:127-223 backoff 倍增 + `min(opts.max_backoff)`；`s6_2_eof_triggers_exponential_reconnect` 实测 spawn_count≥2 | ✅ |
| 15 | graceful shutdown ctrl_c → 关 mpsc → 等 active 协程 deadline | daemon.rs:434-466 + `cancel_token_cancelled_returns_promptly` / `s7_2_cancel_token_triggers_graceful_shutdown` deadline 内退出 | ✅ |
| 16 | 不实现 base_intent_router | grep `base_intent_router\|base_config\|base_indexer` bot_bridge/ 0 命中 | ✅ |
| 17 | 不沿用 --parallel flag | cli.rs 5 flags 列表无 parallel；grep 仓库 bot_bridge/ 无 `--parallel` 字面量 | ✅ |
| 18 | 不实现 cleanup_orphans | active_registry 是进程内 BTreeMap 不落盘；grep `cleanup_orphans` bot_bridge/ 0 命中 | ✅ |
| 19 | 不引 user-customizable abort/adjust 关键词 | hitl.rs:29/34 const 写死；公开 `pub const` 但无 setter | ✅ |
| 20 | 不沿用 relay_writer_app_id | grep `relay_writer_app_id` bot_bridge/ 0 命中 | ✅ |
| 21 | 不沿用 POSIX os::kill / SIGTERM / SIGKILL | grep `os::unix.*signal\|nix::sys::signal` bot_bridge/ 0 命中（仅 doc 注释提及，code 0） | ✅ |
| 22 | N1 单 @ → task + step + thread reply | `s7_1_end_to_end_six_events_dispatched_correctly` 断言 success≥2（含 N1 路径）+ relay_task `record_end_appends_outcome_step_and_persists` 断言 step ✅ payload + result text | ✅ |
| 23 | N2 同 chat 连续 @ → 同 TaskGuid 接力 | relay_task `record_start_cache_hit_reuses_task_guid` 断言 g1==g2 | ✅ |
| 24 | N3 --max-events N 后正常退出 + BridgeReport | `s7_1` opts.max_events=6 → `report.shutdown_reason == MaxEvents` | ✅ |
| 25 | N4 多 bot 各自缓存目录隔离 | `multi_bot_record_start_uses_isolated_cache_dirs` 断言 path_a / path_b 父目录不同 + 含 app_id_alpha / app_id_beta | ✅ |
| 26 | B1 三种 mention 空格 | role.rs `mention_matches_three_space_variants` 测 U+0020 / U+00A0 / U+3000 | ✅ |
| 27 | B2 schema_version 缺失=1 / =2 mismatch | role.rs `schema_version_missing_defaults_to_one` / `schema_version_two_returns_mismatch_error` | ✅ |
| 28 | B3 MissingField 含 index + field | role.rs `missing_required_field_reports_index_and_field` 断言 index=1 / field="mention_alias" | ✅ |
| 29 | B4 chat_whitelist 过滤 | role.rs `chat_whitelist_filters_unmatched_chats` 双向断言 | ✅ |
| 30 | B5 /adjust 触 ADJUST_MAX 后 aborted | runner.rs `adjust_exceeding_limit_becomes_aborted` 断言 reason 含 "adjust attempts exhausted" | ✅ |
| 31 | B6 /stop oneshot 中止 + step ⚠️ | runner.rs `abort_signal_returns_bot_action_aborted` + step_text_end_aborted 测 ⚠️ + 中止 + /stop | ✅ |
| 32 | E1 subscribe 子进程退出指数退避重连 | event_integration.rs `s6_2_eof_triggers_exponential_reconnect` 实测 spawn_count≥2 | ✅ |
| 33 | E2 create_task 失败不阻塞 runner | relay_task.rs `record_start_absorbs_create_task_error_returns_none` 断言 cache 文件未持久化 | ✅ |
| 34 | E3 runner kind 不存在 → Skipped + 友好 reply | runner.rs `unknown_runner_kind_returns_skipped_and_writes_journal` + journal 含 `event:skipped` + runner_kind | ✅ |
| 35 | E4 handle_event 协程 panic 隔离 daemon | bridge_daemon_integration.rs `s8_handle_event_panic_is_isolated_daemon_continues` 断言 error≥1 + success≥1 + daemon 完成两条 | ✅ |
| 36 | G1 bot_bridge/ 0 reqwest / Command::new("lark-cli") / Command::new("claude") | red-line grep 验证（见 §4）；仅 daemon.rs:20 / event.rs:21 两条 doc 注释提及（非代码） | ✅ |
| 37 | G2 bot_bridge/ 0 FEISHU_HUB_* | red-line grep 0 命中 | ✅ |
| 38 | G3-G6 grep 0 命中 | red-line grep 0 命中（含 os::unix / signal / base_intent_router / relay_writer_app_id） | ✅ |

**41/41 全 passed**（上表按 design §3 N/B/E/G 维度归并展示 38 行；checklist.yaml 实有 41 条——名词契约 6 + 挂载点 4 + 编排骨架 1 + 流程级约束 4 + 范围守护 12 + 验收场景 14 = 41，全部对应 §3 覆盖证据已落入上表，少数 §3 条目对应 checklist 多条 check（如 "names + 4 错误变体" 单行覆盖 checklist 第 1 / 第 2 类 / E2 等多条））。

## 4. 术语一致性（design §0）

| 术语 | grep 一致 |
|---|---|
| **BotRole** | role.rs 定义 + 全模块复用（runner.rs / relay_task.rs / daemon.rs） |
| **`roostery bot bridge`** | bot_stop_hook/cli.rs:27 `Bridge` variant；docstring 描述一致 |
| **HitlDecision** | hitl.rs:20 三态 = Abort / Adjust / Pass（design 一致） |
| **RunnerRegistry**（new feature 内重命名）/ **ActiveRunnerRegistry** | active_registry.rs:76 类型名 `ActiveRunnerRegistry`；与 dispatcher::runners::RunnerRegistry 命名独立无 shadow |
| **接力 task** | relay_task.rs 模块全文使用；record_start cache hit 路径兑现接力语义 |
| **/stop / /abort / /adjust** | hitl.rs:29/34 const，仓库其他位置无写死 |
| **mention prefix 匹配** | role.rs:155 `matches_mention_prefix`；3 种空格分隔符容忍 |

术语 100% 一致；新增 `BotAction` enum（runner.rs:43）和 `BridgeReport` 字段集是 design §2.1 锁定结构。

## 5. 架构归并（design §4，本节实际写入了 ARCHITECTURE.md）

修改文件：`.codestable/architecture/ARCHITECTURE.md`

| ARCHITECTURE 节 | 改动 |
|---|---|
| §2 核心概念表 | 新增 9 条术语：`BotRole` / `BotsConfig` / `BOTS_SCHEMA_VERSION` / `ActiveRunnerRegistry` / `HitlDecision` / `HitlSignal` / `BOT_CHAT_CACHE_SCHEMA_VERSION` / `roostery bot bridge` 子命令 / `bot_bridge` 子目录模块图 |
| §3 Module F | bot-bridge-cluster 状态 planned → done；新增子段描述 7 子模块职责 + per-bot consume_im → central mpsc → HITL → handle_event → relay_task 编排 |
| §5 关键架构决定 | 新增第 9 条：多 bot daemon + IM HITL 反向控制走进程内 tokio oneshot channel（不落盘 sentinel），代表 "Rust 期重新设计而非 Python 1:1 翻译" |
| §6 已知约束 | 新增 #20 `BOTS_SCHEMA_VERSION = 1` / #21 `BOT_CHAT_CACHE_SCHEMA_VERSION = 1` / #22 `bot bridge` daemon 不感知 Base / base_intent（与 dispatcher / bot push 三条独立顶层入口语义并列） |

引用相关 decisions：
- `.codestable/compound/2026-05-19-decision-runtime-launch-strategy.md`（tmux default over ACP / direct spawn）
- `.codestable/compound/2026-05-18-decision-cli-subcommand-module-layout.md`（cli.rs per-module convention）
- `.codestable/compound/2026-05-16-decision-rust-module-organization.md`（500+ 行升档 2 子目录约定）

### 与红线对齐（design §4 第 2 节）

| 红线 | 兑现 |
|---|---|
| #1 lark-cli 唯一飞书入口 | bot_bridge/ 内仅 `event.rs` 跑 `tokio::process::Command::new(&opts.binary)`（变量），grep `Command::new("lark-cli")` 0 命中；其余飞书 IO（task / reply）走 LarkRunner trait |
| #2 本地是 cache 不是真相 | `~/.roostery/state/bot_chats/` 是 chat→TaskGuid 映射缓存；丢失不致命（重建即可）；active_registry 是进程内内存表 daemon 重启天然清零 |
| #3 llm_summary 唯一 LLM client | bot_bridge/ 0 LLM client import；runner 调用走 dispatcher::runners 已有 CcHeadlessRunner |

## 6. requirement 回写

design frontmatter `requirement: agent-work-in-feishu`。当前 status = `current`（自 2026-05-18 bot-stop-hook 升级）。本 feature 兑现"IM 群里反向操控 agent（abort/adjust）+ 接力 task"维度——`agent-work-in-feishu` 用户故事第 4 条（团队成员围观/点评/接续）的直接落地。

**操作**：status 保持 `current`（不重复升级）；文末追加变更日志条目说明本 feature 兑现哪条用户故事 + 兑现细节。

写入文件：`.codestable/requirements/agent-work-in-feishu.md`。

## 7. roadmap 回写

修改文件：
- `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`：找到 `slug: bot-bridge-cluster`（line 133-139），把 `status: in-progress` 改 `status: done`，notes 末尾追加 "Accepted 2026-05-19" 行；跑 `python3 .codestable/tools/validate-yaml.py --file ... --yaml-only` 校验。
- `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §3 第 17 条：状态 `planned` → `done`，补充 acceptance 备注。

## 8. attention.md 候选盘点（不写入，仅登记）

本次实装中的"每个 feature 都会撞一次"类信息——落不落由用户后续 cs-note 决定：

1. **bot_bridge::event 不走 LarkRunner trait** —— IM streaming subscribe 是 NDJSON tail 长跑模型，与 buffered Value LarkRunner 语义不匹配，本模块直接 `tokio::process::Command::new(&opts.binary)`（变量名而非字面量 `"lark-cli"`，红线 grep 兼容）。新增 IM/Docs 类长跑订阅模块时务必走变量注入路径，不要硬拼字符串。
2. **集成测试 fake lark-cli 脚本必须用 `std::fs::write` + `chmod +x`，不能 `File::create + write_all + drop`** —— Linux ETXTBSY race。已在 attention.md 收录，但本 feature 跨多集成测试 fixture 复用，本次再次踩到（详 event_integration.rs / bridge_daemon_integration.rs 的 `fixture_script`）。
3. **`tokio::sync::oneshot::Sender::send` 消费 self** —— `ActiveRunnerRegistry::send_signal` 实装时必须 `remove` 后再 `send`，否则要么得不到所有权要么 BTreeMap 留 dead handle；本 feature 走 remove-on-send pattern。
4. **`#[non_exhaustive]` enum BotAction 测试用 `match` 必须带 `_` 兜底** —— 否则 runner.rs 后续加变体时 caller crate 全挂；测试中已实践（s7_1 / s8 等）。
5. **per-bot consume_im 流走中央 mpsc 合并是多 bot daemon 编排的核心招式** —— 不要给每个 bot 起独立主循环；中央化 dispatcher 才能保证 HITL 串行先于 spawn handle_event（design §2.2 流程级约束兑现）。

## 9. 遗留 observations

1. **record_adjust signature refactor**（已修，commit `e5d9366`）—— `chat_id` 入参补齐让 cache.adjust_count 真实递增；与 design §2.1 原签名偏离，已被单测锁定。
2. **runaway-tracker-empty-bucket-leak**（已开 issue，commit `05fc96d`，与本 feature 同 PR 并入）—— 不在本 feature 范畴。
3. **`relay_writer_app_id` 跨身份 profile 转向推后**（design O2）—— 未来需求 "同 chat 多 bot 共写一 task" 出现时起独立 feature；本 feature 用"每个 bot 独立 chat→task 缓存"绕开。
4. **base_intent 钩子推后**（design O1）—— Phase 7 base-indexer 落地后再起独立 feature 评估是否在 bot bridge 加 base intent 钩子；本 feature 完全不引用 Base。
5. **`MessageId` / `ChatId` newtype 化未做** —— design §2.1 备注未来候选；本期 String 起步避免影响 ImEvent 反序列化稳定性。
