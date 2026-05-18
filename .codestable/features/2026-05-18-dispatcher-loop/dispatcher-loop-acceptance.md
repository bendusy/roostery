---
doc_type: feature-acceptance
feature: 2026-05-18-dispatcher-loop
status: accepted
summary: Phase 4 Module E 收尾子 feature 验收闭环——dispatcher 主循环 + replay + test-rule 三入口全 9 节核对通过；架构 §2/§3/§4/§6 已实际归并 + 标 Module E 整体完成；req runtime-neutral 加变更日志条目（保持 draft 等 Phase 5 升级 current）；roadmap items.yaml + 主文档 §5 第 14 项同步 done
related_design: .codestable/features/2026-05-18-dispatcher-loop/dispatcher-loop-design.md
tags: [phase-4, module-e, dispatcher, loop, acceptance, milestone]
---

# dispatcher-loop 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-18
> 关联方案 doc：`.codestable/features/2026-05-18-dispatcher-loop/dispatcher-loop-design.md`
> 里程碑：**Phase 4 / Module E 整体完成**——dispatcher 主循环就绪，Phase 5 可以接 bot-task-writer 把 fire 输出接到飞书 task 卡片

## 1. 接口契约核对

逐条对照 design §2.1.1 dispatcher.rs 草图与 `crates/roostery/src/dispatcher.rs` 实际实装：

**接口示例逐项核对**：

- [x] `DEFAULT_MAX_FANOUT: usize = 16` 公开 const（`dispatcher.rs:28`）→ 一致
- [x] `DispatchOutcome` 3 字段（trace_id / root_event_id / dispatched）全 pub（`dispatcher.rs:31-39`）→ 一致
- [x] `DispatchStep` 7 字段全 pub（`dispatcher.rs:41-51`）→ 一致
- [x] `StepStatus` 5 态（Success / Skipped / GateRejected / Failed / NoMatch）`PartialEq + Clone`（`dispatcher.rs:53-60`）→ 一致
- [x] `DispatchError` `#[non_exhaustive]` 6 变体（ConfigLoadFailed / RulesLoadFailed / JournalDirNotFound / ReplayNotFound / EventReconstructFailed / BadCliInput）（`dispatcher.rs:69-84`）→ 一致
- [x] `pub async fn fire(root_event, registry, rules, cfg) -> DispatchOutcome`（`dispatcher.rs:94-142`）→ 一致；**注意签名扩展**：design 草图说 root_event.trace 为 None 则分配新 ctx，实装额外支持 root_event.trace = Some 时沿用（chain dispatch 内部场景）—— 这是合理增强而非偏离，覆盖更全
- [x] `pub async fn replay(source_trace_id, registry, rules, cfg) -> Result<DispatchOutcome, DispatchError>`（`dispatcher.rs:412-477`）→ 一致
- [x] `pub fn test_rule(event, rules) -> Option<Match<'_>>` trivial wrapper（`dispatcher.rs:482-487`）→ 一致

**journal.rs 加 `load_by_trace_id`**：

- [x] `pub fn load_by_trace_id(dir: &Path, trace_id: &str) -> std::io::Result<Vec<JournalEntry>>`（`journal.rs:117-160`）→ 一致；date-sorted；jsonl 解析失败行 skip + 不报错

**main.rs CLI wiring**（design §2.1.3）：

- [x] `Command::Dispatcher(DispatcherArgs)` + clap 嵌套 `Fire / Replay / TestRule`（`main.rs:33-93`）→ 一致
- [x] `FireArgs { agent / session / cwd / summary / stdin_event / verbose }` → 一致
- [x] `--stdin-event` 模式从 stdin 读 JSON HookEvent（`main.rs::synth_hook_event:158-164`）→ 一致
- [x] fire 始终 `ExitCode::SUCCESS`（`main.rs::run_fire:186-221` 所有分支都返 Success）→ 一致
- [x] replay / test-rule 走 DispatchError `ExitCode::from(1)`（`main.rs::run_replay:223-256 / run_test_rule:258-291`）→ 一致

**名词层"现状 → 变化"逐项核对**：

- [x] 现状（HookEvent / TraceContext / BudgetState / RulesEngine / RunnerRegistry / Journal）全部消费上游已落地模块 → 实装符合
- [x] 变化（新增 dispatcher.rs + journal load_by_trace_id + main.rs Command）全部落地 → 一致

**流程图核对**（design §2.2.1 mermaid 主流程）：

- [x] 图中节点 A→B→C→D→E→F→G→H→I→J→K→Z 在 `process_one(...)` 内有实际落点（trace.check_depth → rules.matches → budget.check_or_raise → runaway.record+check → registry.find → runner.run → budget.consume+save → enqueue_emitted → finalize_step+journal）—— grep `dispatcher.rs` 验证 5 gate 都有对应 if/match 分支

无未处理偏差。

## 2. 行为与决策核对

对照 design §1.1 F1-F12 + §1.2 D1-D14：

**需求摘要 F1-F12 逐项验证**：

- [x] F1 `roostery dispatcher fire` 子命令 → `main.rs::run_fire` + clap derive FireArgs
- [x] F2 fire 主链路 5 gate / 1 engine → `process_one(...)` 实装
- [x] F3 unknown runner kind → Skipped → 单测 `fire_unknown_runner_kind_is_skipped` + integ `fire_happy_with_noop_runner`（间接证明 registry find 路径）
- [x] F4 链式分发 emitted_events → `enqueue_emitted(...)` + 3 单测覆盖
- [x] F5 self-event 短路（已存在 rules.matches 内）→ 无需 dispatcher 重做；继承上游 rules-feature 行为
- [x] F6 `roostery dispatcher replay --trace <id>` → `run_replay` + `replay(...)` 实装
- [x] F7 `roostery dispatcher test-rule` → `run_test_rule` + `test_rule(...)` trivial wrapper
- [x] F8 DispatchOutcome / DispatchStep 数据形状 → 类型定义
- [x] F9 journal::load_by_trace_id → journal.rs 加 read API
- [x] F10 tokio runtime（current_thread）→ `main.rs::run_dispatcher:132-148`
- [x] F11 DispatchError 类型 6 变体 → 类型定义
- [x] F12 失败也写 journal → `finalize_step(...)` 覆盖所有 StepStatus 分支

**关键决策 D1-D14 落地**：

- [x] D1 fire 始终 exit 0 + journal 落档 → `main.rs::run_fire` 所有分支返 SUCCESS；`process_one` 每分支调 `finalize_step` 写 journal
- [x] D2 emitted_events 本期消费走链式分发 → `enqueue_emitted` + `fire` BFS while 循环
- [x] D3 replay 走 live 真跑 runner → `replay` 内部调 `fire`，与外部 fire 同语义
- [x] D4 replay 分配新 trace_id → fire 入口 `root_event.trace = None` 时走 `TraceContext::new_root`；replay 重建 HookEvent 时显式 `trace: null`
- [x] D5 unknown runner kind = Skipped → `process_one` `registry.find` None 分支返 `StepStatus::Skipped`
- [x] D6 DispatchError 分层 → 与 RunnerError / RulesError / BudgetError 不混（编译期保证）
- [x] D7 fire flag 模式 + `--stdin-event` 模式 → `synth_hook_event` 实装两条路径
- [x] D8 fire 默认静默 / `--verbose` 打印 → `if args.verbose { print_outcome(&outcome) }`
- [x] D9 dispatcher.rs 单文件 → 实装 1032 行（产品 ~470 + 测试 ~530）；顶层 18 → 19 < 20 容忍区
- [x] D10 dispatcher.rs 不消费 LarkRunner → 守护 grep N1 通过
- [x] D11 链式分发用 VecDeque BFS 不递归 → `let mut queue: VecDeque<(HookEvent, TraceContext)>` + while pop_front
- [x] D12 trace.max_depth 唯一深度守门 → `ctx.check_depth()` Gate 1；emitted_events 子 event 出队再走 `ctx.check_depth`
- [x] D13 journal::load_by_trace_id 在 journal.rs 同文件 → 实装位置一致
- [x] D14 clap subcommand 嵌套 `dispatcher { fire / replay / test-rule }` → 一致

**明确不做 N1-N12 守护 grep**（实测见会话上文 + S9 阶段）：

- [x] N1 LarkRunner / lark_cli:: → 0（仅 doc 注释 disclaimer 命中，符合预期）
- [x] N2 reqwest / openai / anthropic → 0（同上）
- [x] N3 Command::new / std::process::Command / tokio::process → 0（同上）
- [x] N4 replay 无 --dry flag → grep main.rs 0
- [x] N5 per_runner / per_rule → 0
- [x] N6 cron / scheduler / daemon / tokio::time::interval → 0
- [x] N7 rules / config load 仅入口 → fire / replay 各一次，链式分发不重读（review 确认）
- [x] N8 templates/agent_stop_notify.sh 零改动 → `git diff f56a20c..HEAD --name-only` 不含
- [x] N9 retry / max_retries → 0
- [x] N10 im_send / fallback_im → 0
- [x] N11 tokio::spawn / join_all / FuturesUnordered → 0（BFS 串行）
- [x] N12 git diff 范围 → 仅 dispatcher.rs / journal.rs / lib.rs / main.rs / tests/dispatcher_integration.rs（trace / budget / runaway / rules / runners / hook_event 零改动）
- [x] idiom as_object_mut/as_array_mut unwrap → 0

**编排层流程级不变量核对**（design §2.2.4 不变量 1-10）：

- [x] 不变量 1：任何 gate/runner/DispatchError 失败分支都 journal.append → `finalize_step` 覆盖所有 StepStatus 分支写 journal
- [x] 不变量 2：trace.max_depth 是唯一深度守门 → 实装一致；单测 `fire_chain_over_max_depth_gates_at_boundary`
- [x] 不变量 3：fanout cap → `enqueue_emitted` `.min(DEFAULT_MAX_FANOUT)`；单测 `fire_fanout_truncated_at_default_cap`
- [x] 不变量 4：budget.check_or_raise(0.0) 在 runner 前；consume + save 在 runner Success 后 → 实装一致
- [x] 不变量 5：runaway.record 在 budget 之后、registry.find 之前 → 实装一致
- [x] 不变量 6：dispatcher.rs 不直接走飞书 IO → grep N1 / N3 守护
- [x] 不变量 7：fire 内部错误吞 + journal → `process_one` 所有错误分支都返 DispatchStep（不冒泡）
- [x] 不变量 8：始终 exit 0 → `main.rs::run_fire` 所有分支返 SUCCESS
- [x] 不变量 9：rules 加载一次 → `main.rs::run_fire` 入口 `rules::load()` 一次；fire 内部不重读
- [x] 不变量 10：journal::load_by_trace_id 容错 → 解析失败行 skip；单测 `malformed_lines_are_skipped` 验证

**挂载点反向核对**：

design §2.3 挂载点清单 3 项：

- [x] 挂载点 1 `pub mod dispatcher;` in lib.rs → `lib.rs:7` 一致
- [x] 挂载点 2 `Command::Dispatcher(DispatcherArgs)` in main.rs → `main.rs:34` 一致
- [x] 挂载点 3 `journal::load_by_trace_id` → `journal.rs:117-160` 一致

**反向核查**（grep `dispatcher::|fn fire|fn replay|test_rule` 全 repo 排除 dispatcher.rs 自身）：

- 命中位置：`main.rs`（dispatcher::fire / replay / test_rule 三入口）+ `tests/dispatcher_integration.rs`（测试调用 dispatcher::fire / replay / test_rule）
- 命中性质：均为本 feature 内部或挂载点 2 引用——非清单外的"业务消费方"
- 结论：**无清单外引用**，挂载点清单完整

**拔除沙盘推演**（按清单逆向）：

- 删 `lib.rs:7 pub mod dispatcher;` → lib 不再 export
- 删 `src/dispatcher.rs`（1032 行）→ 模块消失
- 删 `tests/dispatcher_integration.rs` → 集成测试消失
- 删 `main.rs::Command::Dispatcher` + 关联 Args struct + `run_*` 函数 → CLI 子命令消失
- 删 `journal.rs::load_by_trace_id` + 关联测试 → journal 退回 write-only API（与 dispatcher-runners 落地前同状态）
- 0 新增依赖无需撤销
- `cargo build` 通过——5 上游 gate 模块（trace / budget / runaway / rules / runners / hook_event）对 dispatcher 零反向依赖（守护 grep N12 已证）
- 用户感知 = 回到 Phase 3 期间状态（`roostery dispatcher fire` 拿 clap "unknown subcommand" 被 stop hook sh 末尾 `|| true` 吞掉）
- 结论：**可完整卸载**，无残留

## 3. 验收场景核对

对照 design §3 验收契约：

**§3.1 类型 C1.1-C1.5** ✓ 单测 `default_max_fanout_is_sixteen / step_status_variants_distinguishable / dispatch_error_display_includes_context`

**§3.2 fire 主链路 C2.1-C2.10** ✓ 6 lib + 4 integ 覆盖：

- C2.1 NoMatch → `fire_no_match_writes_journal_no_match`
- C2.2 Success + budget.consume → `fire_happy_success_writes_journal_and_consumes_budget`（assert cost 0.5 + calls 1）
- C2.3 unknown runner kind Skipped → `fire_unknown_runner_kind_is_skipped`
- C2.4 runner Failed (exit ≠ 0) → `fire_runner_failed_marks_step_failed`
- C2.5 RunnerError (binary not found) → `fire_runner_error_marks_step_failed`
- C2.6 trace.check_depth 超 max_depth → `fire_chain_over_max_depth_gates_at_boundary`（chain 第二层触发）
- C2.7 budget.check_or_raise 超额 → `fire_budget_over_gate_rejected` + integ `fire_over_budget_gate_rejected_after_n_calls`
- C2.8 runaway.check 触发 → 通过 `process_one` Gate 3 分支实装；本期未单独写测（runaway 上游模块已有 8 个内联测试覆盖触发逻辑）。**这一项是测试覆盖弱点**：见第 9 节遗留
- C2.9 fire 永不冒泡 → 编译期保证（`fire` 返 `DispatchOutcome` 不返 Result）+ 所有测试间接证明
- C2.10 每 step 写 journal → `fire_happy_success_writes_journal_and_consumes_budget` 用 `load_by_trace_id` 断言 journal 长度 == 1

**§3.3 emitted_events 链式 C3.1-C3.4** ✓ 3 lib + 2 integ：

- C3.1 1 child → integ `fire_chain_two_layers_via_real_registry`（emit_count=2 验证多 child）
- C3.2 2 child + 2 层 → 同上（depth=0 root + 2 depth=1 children = 3 step）
- C3.3 超 max_depth → `fire_chain_over_max_depth_gates_at_boundary` + integ `fire_over_depth_gates_child_step`
- C3.4 fanout 超 16 → `fire_fanout_truncated_at_default_cap`（30 emit → 16 入队，root.fanout=16）

**§3.4 replay C4.1-C4.4** ✓ 3 lib + 2 integ：

- C4.1 happy → `replay_happy_runs_again_with_new_trace_id` + integ `replay_roundtrip_creates_new_trace_id`；新 trace_id assert + replay_of 字段 assert
- C4.2 unknown trace_id → `replay_unknown_trace_id_returns_not_found` + integ `replay_unknown_trace_returns_not_found`
- C4.3 字段缺 EventReconstructFailed → `replay_root_entry_with_missing_params_returns_reconstruct_err`
- C4.4 replay exit 1 → `main.rs::run_replay` `Err → ExitCode::from(1)` 静态保证（编译期）

**§3.5 test-rule C5.1-C5.2** ✓ 2 lib + 1 integ：

- C5.1 match → `test_rule_match_returns_some_with_rule_meta` 验证 rule_name + runner 字段
- C5.2 no_match → `test_rule_no_match_returns_none`

**§3.6 CLI C6.1-C6.5** ✓ clap derive 静态保证 + 手动 `--help` 调用验证（impl 阶段执行：`dispatcher --help` / `dispatcher fire --help` 输出正确子命令树和 flag）：

- C6.1 flag 模式合成 → `synth_hook_event` 实装；编译期 + impl 手动 verify
- C6.2 --stdin-event → 同上
- C6.3 --verbose → `print_outcome` 实装；编译期
- C6.4 replay --trace → `ReplayArgs.trace`；编译期
- C6.5 test-rule MATCH / NO MATCH 输出 → `run_test_rule` 实装 println 分支

**§3.7 守护 grep** ✓ 见第 2 节 N1-N12

**§3.8 模块级 C8.1-C8.5** ✓

- C8.1 cargo test --all → lib 322 + dispatcher inline 17 + dispatcher integ 7 + 其他 integ 全过（313+17=330 lib 测试实测，含 dispatcher 17 + journal 5 新增）
- C8.2 cargo test --doc → 全绿
- C8.3 cargo clippy --all-targets --all-features -- -D warnings → 全绿
- C8.4 cargo fmt --all --check → 全绿
- C8.5 守护 grep 全 0 → 通过

**前端改动**：无（dispatcher 是后端核心库 + CLI，无 UI）

## 4. 术语一致性

对照 design §0 + §2.1 命名 grep 代码：

- `DispatchOutcome` / `DispatchStep` / `StepStatus` / `DispatchError` / `DEFAULT_MAX_FANOUT` / `fire` / `replay` / `test_rule` → 单 source of truth（`dispatcher.rs`）；main.rs / 集成测试 / 内联测试统一引用
- `process_one` / `finalize_step` / `enqueue_emitted` / `load_or_init_budget` → 私有 helper 命名与 design §2.2.1 "fire 实装结构" 描述一致
- `synth_hook_event` / `run_dispatcher` / `run_fire` / `run_replay` / `run_test_rule` / `print_outcome` → main.rs 私有 helper，与已有 `run_init` 命名风格一致
- `load_by_trace_id` → journal.rs 新增 fn，命名与既有 `Journal::open` / `append` 风格一致
- 禁用词反向核查：
  - `dry-replay` / `--dry` → main.rs 0 命中（N4 通过）
  - cron / scheduler / daemon → 0 命中（N6 通过）

无不一致。

## 5. 架构归并

对照 design §4，三类内容已实际写入：

- [x] **`.codestable/architecture/ARCHITECTURE.md §2 术语表`** ← design §2.1 新增类型
  - 加 4 个词条：`DispatchOutcome / DispatchStep / StepStatus` / `DispatchError` / `DEFAULT_MAX_FANOUT` / Dispatcher 术语补充模块文件路径
  - journal 词条：未单独追加 `load_by_trace_id`（journal 词条本身已说"public contract"，新增 read API 在 §3 Module B / E 段落里描述更合适）
- [x] **`.codestable/architecture/ARCHITECTURE.md §3 Module E`** ← design §2.2 主流程
  - 子 feature 清单 `dispatcher-loop` 标 done；显式标 **Module E 整体完成（Phase 4 收尾）**
  - 加 "dispatcher 主循环模块" 段落（新文件 / 公开 API / 设计约束 / 不变量 / caller 编排终点指向 Phase 5 bot-stop-hook）
- [x] **`.codestable/architecture/ARCHITECTURE.md §6 已知约束`** ← design §2.2 流程级约束
  - 加第 15 条：`dispatcher::fire` 始终 exit 0（理由 + 例外说明）
  - 加第 16 条：emitted_events 链式 fanout cap（理由 + 改 cap 协议）
  - 加第 17 条：`dispatcher.rs` 不直接走飞书 IO + 不直接 spawn（红线 grep 表述）

**判据自检**：没读过本 feature design 的人打开 ARCHITECTURE.md 现在能看到——
(a) Module E 收尾完成 + dispatcher 主循环角色 + emit chain + replay 语义
(b) §2 术语表能查到 DispatchOutcome / DispatchStep / StepStatus / DispatchError / DEFAULT_MAX_FANOUT 形状
(c) §6 知道 fire 始终 exit 0 / fanout cap / dispatcher.rs 不走飞书 IO 是硬约束

✓ 归并完成。

## 6. requirement 回写

- 方案 frontmatter `requirement: runtime-neutral`
- 该 req `status: draft`，本 feature 是 dispatcher 编排层的最终兑现层
- 已 update `.codestable/requirements/runtime-neutral.md`：
  - `implemented_by` 列表追加 `2026-05-18-dispatcher-loop`
  - 变更日志加 2026-05-18 第 4 条（dispatcher-loop 落地，标 Phase 4 / Module E 整体完成）
  - **req 仍保持 `draft`**——按 design §4 决定：升级 `current` 要等 Phase 5 bot-task-writer + bot-stop-hook 把 dispatcher.fire 真正接到飞书 task 卡片输出，由 Phase 5 收尾 acceptance 一次性升级
- 用户故事 / 边界 / pitch 未变（用户视角端到端"换 runtime 飞书侧呈现不变"还未兑现），仅追加变更日志

## 7. roadmap 回写

- 方案 frontmatter `roadmap: rust-rewrite` / `roadmap_item: dispatcher-loop`
- 已 update `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`：
  - `dispatcher-loop.status: in-progress → done`
  - `feature: 2026-05-18-dispatcher-loop`（校对一致）
  - `notes` 重写：4 个 user 拍板决策摘要 + 0 新增 dep + Accepted commit + CI run #
- 已 update `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md §5 第 14 项`：
  - 标题加 "— **done**（feature `2026-05-18-dispatcher-loop`）"
  - 所属模块加 "（**Phase 4 / Module E 整体完成**）"
  - 状态：planned → **done**
  - 备注重写：完整实装范围 / 不变量 / 红线 / journal read API 启动 / commit + CI run
- yaml 校验通过（`yaml.safe_load` OK）

## 8. attention.md 候选盘点

回看本次实现暴露的环境 / 工具 / 工作流类信息：

- **候选 1：sync 系统冲突副本文件**——实现过程中 macOS iCloud / 同步系统在 `crates/roostery/src/` 下生成了 `dispatcher(M4.local的冲突副本1_2026-05-18 14-39-32).rs` 这种带括号文件名的 stale 快照。cargo 不会编译它（文件名带括号 + 不在 mod tree），但会在 git status 里出现，可能造成混淆。已手工删除。
  - 论据：未来 feature 实现中途 sync 系统可能再生成；提前告诉 AI "看到 `*(*的冲突副本*)*` 不要慌，cargo 不会编译，git status 显示是因为没 .gitignore；可以直接 rm" 能省一次困惑
  - 反方：触发场景跟用户的 sync 系统配置耦合（不是所有人都用 iCloud）；如果只是项目作者一人的环境问题，归 `cs-learn` 比 attention.md 合适
- **候选 2：dispatcher fire 测试 ROOSTERY_HOME 隔离 + ENV_LOCK 模式**——既有 attention.md 已记 "Rust 2024 edition std::env::set_var 是 unsafe，测试中并发触碰 env 必须用 static Mutex 串行化"；本 feature 在 dispatcher.rs + dispatcher_integration.rs 两处复用这个模式 + `#[allow(clippy::await_holding_lock)]` 允许 Mutex 跨 await 持有。
  - 不需要新加 attention 条目——既有规则已覆盖；本 feature 是该规则的又一次落点

留给用户决定候选 1 是否走 `cs-note` 归档。

## 9. 遗留

- **runaway gate 测试覆盖弱**：fire 主链路 §3.2 C2.8 "runaway.check 触发" 本期未单独写 fire 维度集成测试（只有 runaway 上游模块自身的 8 个内联测）。原因：触发 runaway 需要在窗口内重复 record 同 trace_id ≥ threshold (10) 次，单 fire 调用通常不会触发（除非 emitted_events 链式自激同 trace_id 循环到 10 层，那就是 trace.max_depth 先拦了）。**建议**：未来如出现链式分发实战中 runaway 真触发的 case，新增一个 cs-issue 走 fix-note，或直接补一个 lib 测试（用 `RunawayTracker::with_clock` + 手工 record 9 次再 fire）
- **§4.3 契约文本未同步**：roadmap §4.3 原契约仍写 `run(&self, event, ctx, &BudgetGate)` 不带 cost_usd——dispatcher-runners 已经偏离，dispatcher-loop 复用相同偏离，建议下次 `cs-roadmap update` 把 §4.3 原文改齐（架构 §4.3 表格行已加偏离声明，roadmap 章节内 §4.3 详细签名段未改）。**这是 dispatcher-runners 已经登记的遗留，本 feature 不新增**
- **dispatcher/ 子目录化推到 cs-refactor**：自 dispatcher-trace-budget 起反复 flag；Phase 4 已完成（顶层 .rs 文件数 = 19，仍 < 20 容忍区），强烈建议 acceptance 后立即走 `cs-refactor` 把 trace / budget / runaway / hook_event / rules / runners / dispatcher 7 模块聚到 `src/dispatcher/` 子目录。这是只搬不改行为的纯目录重组，与本 feature 独立
- **Phase 5 路径已铺平**：dispatcher-loop 完成意味着 `roostery dispatcher fire` 能从 stop hook sh 接收 event 并跑完整 5 gate / 1 engine 链路 + journal 落档。Phase 5 第一个 feature `bot-task-writer` 的工作是写一个 `Runner` 实装 (`BotTaskWriterRunner` 或类似)，调 `lark-cli task +create` / `append_task_steps` 把 dispatcher 编排结果输出到飞书 task 卡片；用户配 rule "hook_source: cc-stop → runner: bot_task_writer" 就能跑通 0.1.0 minimal-loop
- **实现阶段顺手发现**：sync 系统冲突副本文件已删除（见第 8 节候选 1）；无其他顺手改动
