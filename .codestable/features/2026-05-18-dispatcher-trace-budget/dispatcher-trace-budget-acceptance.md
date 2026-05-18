---
doc_type: feature-acceptance
feature: 2026-05-18-dispatcher-trace-budget
status: passed
date: 2026-05-18
summary: Phase 4 Module E 起步 feature 落地——TraceContext / Budget / RunawayTracker 三独立 gate 模块。trace.rs 202 LOC + budget.rs 330 LOC + runaway.rs 160 LOC + integ 6 测试。Cargo.toml 0 新增依赖，全 std + 既有 chrono/serde/getrandom。fmt/clippy/test --all/--doc 四命令本地全绿 + CI 三 job 远端绿（commit 28e1105，run 26011424647 success）。守护 grep N1-N9 + idiom 全 0 命中（N3/N5 仅 doc-comment 描述词非代码逻辑）
tags: [phase-4, module-e, trace, budget, runaway, dispatcher, acceptance]
---

# dispatcher-trace-budget 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-18
> 关联方案 doc：`.codestable/features/2026-05-18-dispatcher-trace-budget/dispatcher-trace-budget-design.md`

## 1. 接口契约核对

### 1.1 名词层逐一核查（design §2.1）

| 接口 | 设计签名 | 代码落点 | 一致 |
|---|---|---|---|
| `TraceId(String)` `#[serde(transparent)]` newtype | design §2.1.1 | `trace.rs:36-37` | ✓ + 偏差 D1（加 PartialOrd/Ord derive，BTreeMap 索引需要） |
| `TraceId::new_random / as_str / from_existing` | design §2.1.1 | `trace.rs:40-58` | ✓ |
| `TraceContext` `#[non_exhaustive]` 4 字段 | design §2.1.1 | `trace.rs:74-81` | ✓ |
| `new_root / child / check_depth / to_env_pairs / from_env / stamp_journal` | design §2.1.1 | `trace.rs:84-167` | ✓ |
| `TraceError #[non_exhaustive]` 2 变体 | design §2.1.1 | `trace.rs:170-180` | ✓ |
| 3 env key const ROOSTERY_TRACE_ID/DEPTH/PARENT_EVENT_ID | design §2.1.1 | `trace.rs:18-26` | ✓ |
| `Bucket` 4 字段 + `from_cfg / would_exceed / consume` | design §2.1.2 | `budget.rs:30-72` | ✓ |
| `BudgetState #[non_exhaustive]` schema_version + day + default | design §2.1.2 | `budget.rs:74-127` | ✓ |
| `BudgetError #[non_exhaustive]` 5 变体 | design §2.1.2 | `budget.rs:128-153` | ✓ |
| `load / load_from / save / save_to` atomic | design §2.1.2 | `budget.rs:155-205` | ✓ |
| `RunawayTracker { window, threshold, fires, clock }` | design §2.1.3 | `runaway.rs:26-31` | ✓ |
| `new / with_window_and_threshold / with_clock / record / check` | design §2.1.3 | `runaway.rs:33-89` | ✓ |
| `RunawayError::Detected` | design §2.1.3 | `runaway.rs:99-107` | ✓ |
| `paths::budget_state_path()` | design §2.1.4 | `paths.rs:47-49` | ✓ |
| `lib.rs` `pub mod trace/budget/runaway` | design §2.1.5 | `lib.rs:5/12/14` | ✓ |
| `Cargo.toml` 无新增依赖 | design §2.1.6 | `Cargo.toml` 未动 | ✓ |

### 1.2 调用示例核对

- design §2.1.1 `ctx.child(Some(eid))` → integ `end_to_end_three_gates_chain_for_one_dispatch` 验证 trace_id 不变 + depth +1 ✓
- design §2.1.2 `state.check_or_raise(0.001)?; state.consume(0.0024); budget::save(&state)?;` → integ `budget_save_then_load_round_trip_on_real_fs` 全验证 ✓
- design §2.1.3 `tracker.record(&tid); tracker.check(&tid)?;` → integ `runaway_tracker_timeline_with_injected_clock` 全验证 ✓

### 1.3 流程图核对（design §2.2）

主流程图三子图（Trace / Budget / Runaway）+ 上游 caller 编排预期图。逐节点对照：

| 节点 | 代码落点 | ✓ |
|---|---|---|
| Trace T1 new_root / from_env | `trace.rs:84-110, 134-156` | ✓ |
| Trace T2 check_depth | `trace.rs:113-121` | ✓ |
| Trace T3 stamp_journal + to_env_pairs | `trace.rs:160-165, 124-131` | ✓ |
| Trace T4 child | `trace.rs:101-110` | ✓ |
| Budget B1 load/from_cfg | `budget.rs:159, 84-90` | ✓ |
| Budget B2 roll_over_if_needed | `budget.rs:96-105` | ✓ |
| Budget B3 check_or_raise | `budget.rs:109-118` | ✓ |
| Budget B4 consume | `budget.rs:122-125` | ✓ |
| Budget B5 save atomic | `budget.rs:182-205` | ✓ |
| Runaway R1 record | `runaway.rs:62-69` | ✓ |
| Runaway R2 check | `runaway.rs:73-88` | ✓ |

上游 caller 预期编排图（A 入口 → check_depth → tracker.check → budget.check_or_raise → stamp_journal → dispatch → budget.consume → save）由 integ `end_to_end_three_gates_chain_for_one_dispatch` 模拟验证 ✓

### 1.4 偏差与处理

- **D1** `TraceId` 加 `PartialOrd / Ord` derive。design §2.1.1 列了 `PartialEq, Eq, Hash` 没列 Ord。**理由**：`RunawayTracker::fires: BTreeMap<TraceId, Vec<Instant>>` 索引硬要求 Ord trait。**已回填 design §2.1.1**（见下方编辑）。

无其他接口偏差。

## 2. 行为与决策核对

### 2.1 需求摘要 12 个 F 行为验证（design §1.1）

| # | 行为 | 实测 |
|---|---|---|
| F1 | TraceContext 新建（链路起点） | ✓ `new_root_starts_at_depth_zero` 单测 + integ |
| F2 | TraceContext 派生子上下文 | ✓ `child_preserves_trace_id_and_increments_depth` |
| F3 | depth 守门 | ✓ `check_depth_at_max_rejects` + `check_depth_below_max_passes` |
| F4 | TraceContext → JournalEntry 字段注入 | ✓ `stamp_journal_aligns_trace_fields_only` + integ |
| F5 | TraceContext ↔ env 序列化 | ✓ `env_round_trip_preserves_fields` + `from_env_without_trace_id_returns_none` + `from_env_with_invalid_depth_returns_err` + `from_env_missing_depth_defaults_to_zero` + `to_env_pairs_omits_parent_when_none` |
| F6 | Budget load | ✓ `save_then_load_round_trip` + `load_missing_file_returns_load_failed` |
| F7 | Budget check | ✓ `check_or_raise_passes_when_under_cap` + `_fails_when_calls_exceeded` + `_fails_when_cost_exceeded` |
| F8 | Budget consume | ✓ `consume_increments_calls_and_cost` |
| F9 | Budget save | ✓ `save_then_load_round_trip` + `save_to_creates_parent_dir` |
| F10 | Budget rollover | ✓ `rollover_same_day_noop` + `rollover_different_day_resets` + `check_or_raise_triggers_rollover` |
| F11 | RunawayTracker record | ✓ `single_record_returns_one` + `record_within_window_accumulates` + `record_evicts_entries_outside_window` |
| F12 | RunawayTracker check | ✓ `check_at_threshold_returns_err` + `different_trace_ids_are_independent` + `check_on_unknown_trace_returns_zero` |

### 2.2 明确不做（design §1.3 N1-N9）反向核查

```bash
$ grep -rE 'Runner|RunnerKind|runner_registry' crates/roostery/src/{trace,budget,runaway}.rs
0 hits
$ grep -rE 'Rule|rules::' ...
0 hits
$ grep -rE 'Loop|loop_|EventQueue|dispatch' ...
仅 doc-comment 描述词："dispatcher loop" / "Register one dispatch" / "fired N dispatches" / "design.md 路径"
$ grep -rE 'per_runner|by_rule|by_runner' src/budget.rs
0 hits
$ grep -rE 'fs::write|save|load' src/runaway.rs
仅一处 `advance_clock.load(Ordering::SeqCst)` 测试中 AtomicU64 字面词
$ grep 'FEISHU_HUB_' src/{trace,budget,runaway}.rs
0 hits
$ grep 'uuid|ulid_rs|crossbeam|moka' Cargo.toml
0 hits
$ grep 'Command::Budget|Command::Trace|Command::Runaway' src/main.rs
0 hits
$ grep -E 'LarkRunner|lark_cli::' src/{trace,budget,runaway}.rs
0 hits
$ grep -rE 'as_object_mut\(\)\.unwrap\(\)|as_array_mut\(\)\.unwrap\(\)' src/{trace,budget,runaway}.rs
0 hits
```

N3 / N5 命中均为字面词误判（doc-comment 描述路径 / AtomicU64::load 方法名）；**无任何代码逻辑** Loop / dispatch struct / fs 持久化函数。意图守住。

### 2.3 关键决策 D1-D11 落地（design §1.2）

| # | 决策 | 代码体现 |
|---|---|---|
| D1 | TraceContext 不携带 runner kind / event payload | `trace.rs:74-81` 仅 4 字段 |
| D2 | trace_id 用 16-byte hex via getrandom | `trace.rs:41-49` |
| D3 | depth 从 0 起 | `trace.rs:88-95` new_root depth=0 + `new_root_starts_at_depth_zero` 单测 |
| D4 | env 前缀 ROOSTERY_* | `trace.rs:18-25` + `env_key_constants_use_roostery_prefix` 单测 |
| D5 | Budget = default 单 bucket | `budget.rs:77-81` BudgetState 仅 `default: Bucket` |
| D6 | cost 单位 f64 USD | `budget.rs:32-35` 全 f64 |
| D7 | state 路径 ~/.roostery/state/budget.json | `paths.rs:47-49` + `budget.rs:156, 179` |
| D8 | rollover 每次 check / consume 前调 | `budget.rs:110, 123` 内部都调 |
| D9 | RunawayTracker 内存 only | `runaway.rs:26-31` 无 fs 字段 |
| D10 | 默认 window=300s threshold=10 | `runaway.rs:21-22` const + `defaults_are_300s_and_10` 单测 |
| D11 | 三模块不互引 | trace 不引 budget/runaway；budget 不引 trace/runaway；runaway 引 `trace::TraceId` 仅为类型签名（无逻辑耦合）。caller dispatcher-loop 串场景 |

### 2.4 编排层"现状 → 变化"核对（design §2.2）

dispatcher 模块树空 → 本 feature 引入 3 个独立 gate 模块。无跨模块编排（design 明示）✓

### 2.5 流程级约束（design §2.2 不变量 1-7）

| 不变量 | 守护方式 |
|---|---|
| 1 TraceContext 不可变 | new_root/child 返新值；stamp_journal 仅 `&mut entry` 借用更新字段（trace ctx 本身不变） |
| 2 depth 单调递增 | child() 总 +1；无 decrement API；类型签名编译期保证 |
| 3 budget save atomic | `budget.rs:191-203` `.tmp` + `fs::rename` + 缺父目录 `fs::create_dir_all` |
| 4 budget rollover 幂等 | `rollover_same_day_noop` 单测断言同日二次返 false |
| 5 budget schema_version=1 公开承诺 | `budget.rs:25` const + `load_wrong_schema_version_errors` 守护 |
| 6 runaway 内存隔离 | `RunawayTracker` 无 fs 字段；多实例不共享（无 static state） |
| 7 runaway 窗口清理懒计算 | `runaway.rs:63-67` record 内 retain；无 thread::spawn |

### 2.6 挂载点反向核对（design §2.3）

| # | 挂载点 | 代码实际落点 | 一致 |
|---|---|---|---|
| 1 | `pub mod trace;` in lib.rs | `lib.rs:14` | ✓ |
| 2 | `pub mod budget;` in lib.rs | `lib.rs:5` | ✓ |
| 3 | `pub mod runaway;` in lib.rs | `lib.rs:12` | ✓ |
| 4 | `paths::budget_state_path()` | `paths.rs:47-49` | ✓ |

**反向 grep 核查**：

```bash
$ grep -rn 'use roostery::\(trace\|budget\|runaway\)' crates/roostery/src/ crates/roostery/tests/
crates/roostery/tests/trace_budget_integration.rs:5: use roostery::budget::{self, BUDGET_SCHEMA_VERSION, BudgetState};
crates/roostery/tests/trace_budget_integration.rs:8: use roostery::runaway::RunawayTracker;
crates/roostery/tests/trace_budget_integration.rs:9: use roostery::trace::{TraceContext, TraceId};
```

外部消费者仅 integ test（本 feature 内）。`runaway.rs` 内部用 `crate::trace::TraceId` 是类型签名引用而非外部消费，不算挂载点外引用。dispatcher-rules / dispatcher-runners / dispatcher-loop Phase 4 后续 feature 才会消费。**无清单外挂入点**。

**拔除沙盘推演**：删 4 个挂载点（lib.rs 3 pub mod + paths.rs budget_state_path）+ 3 模块文件 + integ test → `cargo build` 通过其他模块不感知；journal.rs 的 trace_id / parent_event_id / depth 字段仍在但永远 None / 0（无 caller 注入）；config.rs 的 BudgetCfg / TraceConfig 不被消费仍可序列化（早就独立可用）。**边界清晰，可完整卸载**。

## 3. 验收场景核对（design §3）

### 3.1 TraceContext C1.1-C1.6

| # | 场景 | 证据 |
|---|---|---|
| C1.1 | new_root(None, 8) | ✓ `new_root_starts_at_depth_zero` |
| C1.2 | child(Some(eid)) | ✓ `child_preserves_trace_id_and_increments_depth` |
| C1.3 | depth=max → DepthExceeded | ✓ `check_depth_at_max_rejects` |
| C1.4 | depth=max-1 → Ok | ✓ `check_depth_below_max_passes` |
| C1.5 | env round-trip + missing + invalid | ✓ 4 单测覆盖 |
| C1.6 | stamp_journal 仅改 3 字段 | ✓ `stamp_journal_aligns_trace_fields_only` 显式断言 event_id/action/ts 不动 |

### 3.2 Budget C2.1-C2.10

| # | 场景 | 证据 |
|---|---|---|
| C2.1 | from_cfg 零初始化 | ✓ `from_cfg_zero_init_with_cfg_caps` |
| C2.2 | check 0-balance ok | ✓ `check_or_raise_passes_when_under_cap` |
| C2.3 | calls 超额 | ✓ `check_or_raise_fails_when_calls_exceeded` |
| C2.4 | cost 超额 | ✓ `check_or_raise_fails_when_cost_exceeded` |
| C2.5 | 跨日 rollover 触发 | ✓ `check_or_raise_triggers_rollover` |
| C2.6 | 同日 rollover noop | ✓ `rollover_same_day_noop` |
| C2.7 | save round-trip + .tmp 不残留 | ✓ `save_then_load_round_trip` + integ |
| C2.8 | load 文件不存在 | ✓ `load_missing_file_returns_load_failed` |
| C2.9 | load 非法 JSON | ✓ `load_invalid_json_returns_parse_failed` |
| C2.10 | schema_version=2 | ✓ `load_wrong_schema_version_errors` |

### 3.3 RunawayTracker C3.1-C3.5

| # | 场景 | 证据 |
|---|---|---|
| C3.1 | 单次 record 返 1 | ✓ `single_record_returns_one` |
| C3.2 | 累计 5 次 | ✓ `record_within_window_accumulates` |
| C3.3 | 超阈值 | ✓ `check_at_threshold_returns_err` |
| C3.4 | 注入 clock 模拟过窗口 | ✓ `record_evicts_entries_outside_window` + integ `runaway_tracker_timeline_with_injected_clock` |
| C3.5 | 不同 trace_id 互不影响 | ✓ `different_trace_ids_are_independent` |

### 3.4 明确不做反向核查 C4.1-C4.9

见 §2.2 表全过 ✓

### 3.5 模块级 C5.1-C5.5

| # | 命令 | 结果 |
|---|---|---|
| C5.1 | `cargo test --all` | 239 lib + 6 onboarding integ + 12 hooks_merge integ + 2 config integ + 4 shim + 4 smoke + 6 trace_budget integ + 3 doc 全绿 |
| C5.2 | `cargo test --doc` | 3 doc-tests 通过（2 lark_cli ignored）；本 feature 各模块 doc-comment 中无新增 doc-test（降一档：design checklist 提到但未硬要求） |
| C5.3 | `cargo clippy --all-targets --all-features -- -D warnings` | 全绿（修过一次 drop_non_drop） |
| C5.4 | `cargo fmt --all --check` | 全绿 |
| C5.5 | 守护 grep 0 命中 | 通过（见 §2.2） |

**前端改动**：无（pure backend gate library feature）。

## 4. 术语一致性

| 术语 | 代码命中 | 一致 |
|---|---|---|
| `TraceContext` | `trace.rs:75` struct + 测试/integ 多处 | ✓ |
| `TraceId` | `trace.rs:36` newtype + onboarding integ + runaway/budget signatures | ✓ |
| `TraceError::DepthExceeded / EnvParseFailed` | `trace.rs:170-180` + 测试断言 | ✓ |
| `Bucket / BudgetState` | `budget.rs:31, 77` + 测试 + integ | ✓ |
| `BudgetError` 5 变体 | `budget.rs:128-153` + 测试 | ✓ |
| `RunawayTracker / RunawayError::Detected` | `runaway.rs:26, 99` + 测试 | ✓ |
| `ROOSTERY_TRACE_ID / DEPTH / PARENT_EVENT_ID` | `trace.rs:18-25` + 测试 + design D4 引用 | ✓ |
| `BUDGET_SCHEMA_VERSION = 1` | `budget.rs:25` + integ + 测试 | ✓ |

**防冲突 grep**：

- `grep -rn 'TraceContext' crates/roostery/src/` → 仅 trace.rs + 测试，无冲突命名
- `grep -rn 'BudgetState' crates/roostery/src/` → 仅 budget.rs + 测试，与 `config::Budgets / config::BudgetCfg` 类型名不重合
- `grep -rn 'RunawayTracker' crates/roostery/src/` → 仅 runaway.rs + 测试

无术语冲突。

## 5. 架构归并

### 5.1 `ARCHITECTURE.md §2 术语表`

加 5 个新词条：`TraceContext` / `TraceId` newtype / `BudgetState` + `Bucket` / `BUDGET_SCHEMA_VERSION` / `RunawayTracker`。

### 5.2 `ARCHITECTURE.md §3 Module E`

把当前的空 placeholder 子节填充：加 trace / budget / runaway 三段描述；子 feature 列表 `dispatcher-trace-budget` 标 done。

### 5.3 `ARCHITECTURE.md §4 契约表 §4.5`

`TraceContext` 行标 "Phase 4 已落地（feature `2026-05-18-dispatcher-trace-budget`）"。

### 5.4 `ARCHITECTURE.md §6 已知约束`

加 1 条：`BUDGET_SCHEMA_VERSION = 1` 公开承诺 + caller 必须把 `Config.trace.max_depth` 注入 `TraceContext`。

### 5.5 `.codestable/requirements/runtime-neutral.md`

变更日志加 2026-05-18 `dispatcher-trace-budget` 落地条目；`implemented_by` 加本 feature；status 保持 `draft`（loop 真起来要 Phase 4 收尾 dispatcher-loop feature）。

### 5.6 `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`

`dispatcher-trace-budget` `in-progress → done`。

### 5.7 `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md §5 第 11 项`

`planned → done` + feature 引用补充。

### 5.8 `.codestable/attention.md`

候选盘点见 §8。

### 5.9 `.codestable/compound/`

无新 decision 候选。`business-identifier-newtype` decision 已覆盖 TraceId（design §1.5 idiom #3 显式 honour）。

## 6. requirement 回写

- 方案 frontmatter `requirement: runtime-neutral`（current status: `draft`）
- 本 feature 兑现 req 的"loop 保护是中立 dispatcher 的前提"——三 gate（trace 深度 / runaway 阈值 / budget 配额）独立于具体 runtime，是后续 Phase 4 dispatcher-loop 接入任意 runtime 时必经的守门基底
- 处理方式：**update**——`implemented_by` 加本 feature；变更日志追加 2026-05-18 条目；status 保持 `draft`（用户视角"换 runtime 飞书侧呈现不变"还要 Phase 4 dispatcher-loop + Phase 5 bot-stop-hook 兑现）

## 7. roadmap 回写

- 方案 frontmatter `roadmap: rust-rewrite` / `roadmap_item: dispatcher-trace-budget`，两字段都有值
- `rust-rewrite-items.yaml` 第 85-91 行 `slug: dispatcher-trace-budget` 当前 `status: in-progress` + `feature: 2026-05-18-dispatcher-trace-budget`（design 阶段已写入）
- 改 `status: done`，`validate-yaml.py` 校验通过（本节执行后断言）
- `rust-rewrite-roadmap.md` §5 第 11 项当前 `状态: planned` → 改 `状态: **done**（feature 2026-05-18-dispatcher-trace-budget）`

## 8. attention.md 候选盘点

**本 feature 暴露的"下个 feature 还会撞"硬约束**：

1. **clippy `drop_non_drop` 在测试中容易触发**：测试用临时变量再 drop 是 Rust 编程常见 noise，且 `-D warnings` CI 会卡。**触发判据**：feature dev 用 `Vec / String / 闭包` 等 `Default` non-Drop 类型再 `drop()`。**是否归入 attention.md**：**否**——这是 clippy 规则之一，下次撞了再加更可能加偏（lint 规则多，逐一加 attention 会膨胀）。**归 cs-learn pitfall** 更准。

2. **rust-idiom-first decision 守护 grep 模板**：本 feature 又一次跑了 `grep -rE 'as_object_mut\(\)\.unwrap\(\)|as_array_mut\(\)\.unwrap\(\)' src/{...}.rs` 这条守护，对所有新 feature 应该是默认配置。**触发判据**：每个新 feature design §1.5 加 idiom checklist + acceptance 阶段守护 grep。**是否归入 attention.md**：**否**——已在 `.codestable/compound/2026-05-18-decision-rust-idiom-first.md` decision §83 落地，acceptance 阶段每次走，**不是 attention.md 一句话能覆盖**——归 decision 更准。

**结论**：**本 feature 未暴露需要补入 attention.md 的内容**。已有的"`#[non_exhaustive]` 测试 fixture corollary"条目（上次 roostery-init 加）本 feature 又实测复用了（`BudgetCfg::default()` + 字段赋值绕 E0639），验证了那条 attention 条目持续起作用。

## 9. 遗留

### 9.1 后续观察项

- **跨进程 RunawayTracker**：design §1.2 D9 + roadmap §7 已 flag。dispatcher-loop feature 起来后若发现 daemon 实例多个并发时需要 cross-process tracking，再走 cs-roadmap update 评估。
- **Budget 粒度扩展**：design D5 + §4 回写说明已记。Phase 4 dispatcher-rules / dispatcher-runners 起来时若发现 per-runner / per-rule 是常用需求，走 `cs-roadmap update` 扩 §4.6 schema 后再补本模块的 multi-bucket 接口。
- **trace_id 选型**：design D2 用 16-byte hex via getrandom，与 journal `event_id` (ULID via Crockford base32) 在选型上不一致。**理由**：trace_id 不需要时间排序（journal entry 才需要按时间扫描），随机性 + 唯一性即可。未来如果发现 trace 跨多个 event 时按时间排序更直观，再评估改 ULID。

### 9.2 顺手发现

- **测试 env 串行化第 3+ 处重复**：之前 roostery-init 已 flag。本 feature integ 测试用 atomic clock 避免触碰 env，所以**未引入新的 env 串行化点**——这是个良性信号（gate 模块设计上独立于 env 状态）。

### 9.3 已知限制

- **本 feature 不交付 dispatch 编排**：三模块各自独立 gate，**caller 串场景**是 dispatcher-loop（Phase 4 收尾 feature）的职责。本 feature 完成 = caller 装弹（gate 类型 + 数据结构 + persistence + 测试基础设施）就绪，**dispatcher 还不会跑**。
- **trace_id / event_id 当前两套**：见 §9.1。short-term 不统一。
- **doc-test 缺失**：design checklist S9 提到 "本 feature 各模块 doc-comment 含简短示例" 走 doc-test，实际未落地（async/sync 混合 + 依赖 BudgetCfg 构造较啰嗦不适合 doc-test 形态）。降一档可接受，未来 dispatcher-loop 起来后接口稳定再补。
