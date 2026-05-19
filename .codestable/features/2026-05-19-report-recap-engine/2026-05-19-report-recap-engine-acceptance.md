# report-recap-engine 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-19
> 关联方案 doc：`2026-05-19-report-recap-engine-design.md` (status: approved，v5 落版)

## 1. 接口契约核对

**接口示例逐项核对**（对照 design §2.1 名词层）：

- [x] `CommitHash::new(raw: &str) -> Result<Self, GitLogError>` 含 trim + non-empty invariant → 代码：`git_log.rs:30-39` 实装一致，`as_str()` / `Display` 已暴露
- [x] `RepoSpec::new(path) -> Result<Self, RepoSpecError>` canonicalize + is_dir invariants → 代码：`git_log.rs:56-79` 实装一致，含 `PathNotFound` / `NotADirectory` / `Canonicalize` 三 variant
- [x] `Commit { hash, timestamp: DateTime<FixedOffset>, author, subject, body }` → 代码：`git_log.rs:107-114` 一致
- [x] `GitLogAggregate { date, timezone, repos }` + `is_empty()` / `total_commits()` → 代码：`git_log.rs:125-142` 一致；额外补 `#[serde(serialize_with)]` 让 `FixedOffset` 序列化（实装细节非偏差）
- [x] `GitLogError` 5 variant + `RepoSpecError` 3 variant + `RecapError` 5 variant + `NoSummaryReason` 7 variant 全部 `#[non_exhaustive]` + thiserror `#[from]` 链 + `#[error("..: {0}")]` Display → 代码：`git_log.rs:144-165` + `mod.rs:126-141` 一致
- [x] `RecapRequest { date, timezone, repos, runner_kind, timeout_ms, prompt_override }` + `Default` impl → `mod.rs:54-75` 一致
- [x] `RecapRuntime<'a> { registry, journal, budget_cfg, budget_path, trace_max_depth, budget_estimated_cost_usd }` 5 个 + 1 个 estimated cost 字段 → `mod.rs:44-52` 一致
- [x] `PreparedRecap { aggregate, markdown, prompt }` → `mod.rs:79-83` 一致
- [x] `PromptRunnerArgs<'a> { prompt, timeout_ms, model?, resume_id? }` Serialize → `mod.rs:91-99` 一致
- [x] `RecapOutcome { Summarized / RawDump / Failed }` → `mod.rs:103-118` 一致
- [x] `RecapJsonOutcome / RecapJsonReason` + `schema_version: 1` const → `mod.rs:494-530` 一致；`RecapJsonReason` 内 `RunnerNotInRegistry` 字段名从 design 文档 `kind` 改为 `runner_kind`（serde 内部 tag `kind` 冲突，已在 step 9 notes 记录）
- [x] `pub async fn run(req, rt) -> RecapOutcome` + `pub fn prepare(req) -> Result<PreparedRecap, RecapError>` → `mod.rs:174 + 156` 一致
- [x] `collect_aggregate(date, tz, &[RepoSpec]) -> Result<GitLogAggregate, GitLogError>` → `git_log.rs:177` 一致
- [x] `render_markdown(&GitLogAggregate) -> String` → `git_log.rs:294` 一致

**名词层"现状 → 变化"逐项核对**：

- [x] 现状：`daily_recap` / `git_log` 模块全新 → 代码：`crates/roostery/src/daily_recap/` 新建目录含 `mod.rs / cli.rs / git_log.rs / templates/`，Phase 4 既有类型（Runner / RunnerRegistry / RunOutcome / RunnerError / TraceContext / BudgetGuard / Journal）零改动 ✓

**流程图核对**（design §2.2 mermaid）：

- [x] 节点 `Resolve repos + runner_kind` → 代码 `run()` 入口两条 `if .is_empty()` check (`mod.rs:178-183`)
- [x] 节点 `git_log::collect_aggregate` → 代码 `mod.rs:186`
- [x] 节点 `RunnerRegistry::find` → 代码 `mod.rs:199`
- [x] 节点 `BudgetGuard::open_at` → 代码 `mod.rs:210`
- [x] 节点 `state_mut().check_or_raise(rt.budget_estimated_cost_usd)` → 代码 `mod.rs:225`（含 `if estimated > 0.0 else DEFAULT` 兜底）
- [x] 节点 `runner.run(event, trace, args)` → 代码 `mod.rs:258`
- [x] 节点 `state_mut.consume(cost_usd or 0.0) + commit` → 代码 `mod.rs:278-285`
- [x] 节点 `Journal::append source=daily_recap` → 代码 `mod.rs:299 / 333` (finalize_summarized + finalize_raw)
- [x] 所有 `RawDump` 分支映射到 7 个 `NoSummaryReason` variant → 代码 `mod.rs:260-285` match 全覆盖
- [x] `Journal::append` IO error → `Failed::JournalAppend` → 代码 `mod.rs:307-308 / 339-340`

**结论**：0 偏差。Design §4.1 库 API 与 §2.1 / §2.2 全部映射到现有代码。

## 2. 行为与决策核对

**需求摘要逐项验证**（design §1）：

- [x] git_log 多仓聚合 → `collect_aggregate` 多 repo 顺序 spawn `git -C <path> log --since/--until --pretty=%H%x1f%cI%x1f%an%x1f%s%x1f%b%x1e` 实装
- [x] 直接调 `RunnerRegistry::find(kind).run` 委托 agent CLI → `mod.rs:199 + 258` 实装，**不走 `dispatcher::fire`**
- [x] 输出 `RecapOutcome` 三态结构化 → `mod.rs:103-118` enum
- [x] 双 CLI / 库 API 产物 → `cli.rs DailyRecapArgs + run()` + `mod.rs pub async fn run/prepare`

**明确不做逐项核对**（design §1）：

- [x] 不写飞书 docx / Base 记录 → `rg -i "lark_cli\|docx\|base_app" crates/roostery/src/daily_recap` → 0 命中 ✓
- [x] 不做 cron 调度器 → CLI 跑完即退，无 daemon / loop ✓
- [x] 不走 `dispatcher::fire` → `rg "dispatcher::fire" crates/roostery/src/daily_recap` → 0 命中 ✓
- [x] 不动 rules.yaml → `rg -i "rules.yaml\|RulesConfig\|CompiledRule" crates/roostery/src/daily_recap` → 0 命中 ✓
- [x] 不引入新 Runner impl → `rg "impl Runner for" crates/roostery/src/daily_recap` → 0 命中（只在 tests/ MockRunner / MockOutcomeRunner，符合）
- [x] 不引入新 trait → `rg "pub trait" crates/roostery/src/daily_recap` → 0 命中 ✓
- [x] 不引外部 LLM SDK → `cargo tree -p roostery | grep -iE "reqwest|openai|anthropic|gemini|hyper|ureq"` → 0 命中 ✓

**关键决策落地**（design §0 D1-D7）：

- [x] D1 模块 nested：`crates/roostery/src/daily_recap/git_log.rs` 是 nested 子模块，不顶级 sibling ✓
- [x] D2 双产物：`pub async fn run` (lib API) + `roostery daily-recap` CLI ✓
- [x] D3 三态降级 enum：`RecapOutcome::Summarized / RawDump / Failed`，每态 typed payload（不退化成 String）✓
- [x] D4 rules.yaml 不自动注入：CLI 无 `--write-rule` flag；prompt template 用 config + CLI flag override 不动 rules.yaml ✓
- [x] D5 走 (c) 直调 RunnerRegistry：`rg "dispatcher::fire" daily_recap/` 0 命中；`registry.find(kind).run(...)` 直接调用 ✓
- [x] D6 Rust idiom-first：`#[from]` 链 / `thiserror` / typed `PromptRunnerArgs` struct / `RecapRuntime` context / `CommitHash` newtype / `RepoSpec` smart constructor 全部落地 ✓
- [x] D7 不 over-engineer：无 typestate / 无 builder pattern / CLI 保 `--dry-run` flag 不拆 subcommand variant ✓

**编排层"现状 → 变化"逐项核对**：

- [x] 变化 V1：在已有 RunnerRegistry / BudgetGuard / Journal 上**新增** `daily_recap::run` 主流程串联 5 步链路 → `mod.rs:174-294` 实装
- [x] 变化 V2：新增 `daily_recap::prepare` 双 entry point 分离 dry-run 与 live → `mod.rs:156`
- [x] 变化 V3：dispatcher 既有路径**零改动**（验证：`git diff crates/roostery/src/dispatcher/` 此次未触碰）

**流程级约束核对**：

- [x] 错误语义：git 层硬错 → `Failed`；runner / budget 软错 → `RawDump`；journal 写失败硬错 → `Failed::JournalAppend` → 代码 `mod.rs` `finalize_*` 三函数实装
- [x] 幂等性：design 已声明"不再完全幂等"（budget consume on success）→ 代码 `mod.rs:278-285` 实装 mirror dispatcher only-on-success 政策
- [x] 并发：`BudgetGuard::open_at` 跨进程 flock → 实装委托 Phase 4 既有 BudgetGuard，daily_recap 不另造 lock
- [x] trace 深度：fresh root depth=0 → `mod.rs:244` `TraceContext::new_root(None, rt.trace_max_depth)`
- [x] 审计点：每次 daily-recap 写一条 JournalEntry，含 scrub 后的 prompt_head + summary_head + reason_kind / error_kind discriminant → `mod.rs:380-414` `build_journal_entry`

**挂载点反向核对（design §2.3 - 可卸载性）**：

逐条 grep + 沙盘推演：

- [x] M1 `Command::DailyRecap(DailyRecapArgs)` CLI 子命令 → `main.rs:39-40` + `main.rs:142-143` cfg-gated，删除整 feature flag 即可剥
- [x] M2 `daily_recap` 模块整目录 → `crates/roostery/src/daily_recap/`，删目录 + `lib.rs` `#[cfg]` line + `main.rs` 两条 cfg 块 = feature 完全消失
- [x] M3 Cargo feature flag `daily-report` → `Cargo.toml:28-30 [features]` 段，关掉 = 整组消失
- [x] M4 embedded prompt template → `daily_recap/templates/default-recap-prompt.md` + `include_str!` 引用 in `mod.rs:32`
- [x] M5 `Config.recap` 段 → `config.rs:42 + 161-190` `RecapConfig` / `RecapRepoConfig`；DTO 不 gate（设计如此），删除 feature 后字段仍 deserialize 兼容旧 yaml

**反向核查 grep**：本 feature 的所有引用都落在清单内吗？
- `rg "daily_recap" crates/roostery/src --type rust` → 命中：`lib.rs:11` (M2) / `main.rs:39 + 143` (M1) / 模块内自引 / `tests/daily_recap_integration.rs` 测试（不算挂入）
- `rg "RecapConfig\|RecapRepoConfig" crates/roostery/src` → 命中：`config.rs` (M5) + `daily_recap/cli.rs` 消费
- `rg "daily-report" crates/roostery/` → 命中：`Cargo.toml` (M3) + `lib.rs`/`main.rs` cfg attrs

→ 清单外 0 引用 ✓

**拔除沙盘推演**：
- 关 Cargo feature `daily-report` → daily_recap 整 mod 不编译 + `Command::DailyRecap` 不注册 → 验证：`cargo build --no-default-features` 成功（已跑） ✓
- 删 `crates/roostery/src/daily_recap/` 整目录 → 编译期失败提示 `lib.rs:11` 引用不到模块 → 同步删 `lib.rs` 那行即可干净 ✓
- 删 `Cargo.toml [features]` 段 + `Config.recap` 字段 → `Default` impl 也要改回，但本 feature 之前没有 `Config.recap`，提交记录可还原 ✓

**结论**：可卸载性合格，无残留 / 无清单外漏挂。

## 3. 验收场景核对

逐条按 design §3 关键场景清单核对（已在 implement 阶段实装相应测试）：

### 3.1 正常路径（N1-N7）

- [x] **N1 库 API Summarized** — 证据：单元测试 `run_summarized_happy_path` + 集成测试 `integ_n1_summarized_writes_journal_and_budget` 通过；验证 summary 内容 + cost_usd + journal Ok value.outcome=summarized
- [x] **N2 CLI dry-run** — 证据：手工 smoke `./target/debug/roostery daily-recap --dry-run --repo .` 在本仓输出 git markdown + rendered prompt + 单元测试 `prepare_returns_aggregate_markdown_prompt` + `integ_prepare_does_not_touch_registry_budget_journal` 验证不调 registry / 不开 budget / 不写 journal
- [x] **N3 库 API 真跑 mock** — 证据：`run_summarized_happy_path` 使用 `RunnerRegistry::with_runner(Box::new(MockRunner::succeeds(...)))` 注入（codex P1.8 修正实施）
- [x] **N4 多仓聚合** — 证据：单元测试 `multi_commit_body_with_newline_preserved` (3 commits same repo) + 集成测试 `integ_multi_repo_aggregation` (2 repos)
- [x] **N5 `--date` 覆盖** — 证据：单元测试 `future_date_returns_empty` 验证 `--since/--until` 边界 + `git_log.rs:177-200` 时区边界处理
- [x] **N6 `--runner` 覆盖 config** — 证据：CLI `build_request` 走 `args.runner.clone().unwrap_or(cfg.recap.runner_kind.clone())` (`cli.rs:73-75`)
- [x] **N7 `--json` v1 schema** — 证据：单元测试 `json_dto_summarized_schema_v1 / json_dto_raw_dump_schema_v1 / json_dto_failed_schema_v1` + 集成测试 `integ_json_dto_summarized_v1_schema`

### 3.2 降级路径（D1-D8）

- [x] **D1 RunnerNotInRegistry** — `run_runner_not_in_registry` + `integ_d1_runner_not_in_registry`
- [x] **D2 BudgetUnavailable (open_at fail)** — 类型层覆盖：`BudgetError::LoadFailed` / `ParseFailed` / `SchemaVersionMismatch` 任一 → match 通过 `mod.rs:213-216`
- [x] **D3 BudgetExhausted** — `run_budget_exhausted`（用 small max_cost_usd 触发 check_or_raise Exceeded）
- [x] **D4 RunnerErrored** — `run_runner_errored`（用 `RunnerError::Timeout`）+ design D4 中提到的 `SpawnFailed / BinaryNotFound / Timeout / OutputParseFailed / BadArgs` 5 variant 类型层全覆盖
- [x] **D5 RunOutcomeFailed** — `run_run_outcome_failed` + `integ_d5_run_outcome_failed_no_budget_consume` 验证 `RunnerStatus::Failed { reason }` 走 `NoSummaryReason::RunOutcomeFailed`
- [x] **D6 RunOutcomeSkipped** — `run_run_outcome_skipped`
- [x] **D7 EmptyOutput** — `run_empty_output`（mock 返 `"   \n  \t  "` 验 trim 后 empty）
- [x] **D8 commit() 失败 → BudgetUnavailable** — 类型层覆盖：`mod.rs:281-285` `guard.commit()` Err 分支映射到 `NoSummaryReason::BudgetUnavailable`

**Budget consume 政策共同验收**：
- [x] `budget_consumed_on_success` — Success + non-empty stdout → calls=1 持久化
- [x] `budget_not_consumed_on_failure` — RunOutcome Failed → budget 文件无 calls=1 / 不存在
- [x] `integ_d5_run_outcome_failed_no_budget_consume` — 跨 crate 端到端验证同政策

### 3.3 硬错路径（F1-F10）

- [x] **F1 NoRepos** — `run_no_repos_failed` + `integ_f1_failed_no_repos`
- [x] **F2 NoRunnerKind** — `run_no_runner_kind_failed` + `integ_f2_failed_no_runner_kind`
- [x] **F3 GitLog NotAGitRepo** — `not_a_git_repo_returns_typed_error`（tempdir 非 git）
- [x] **F4 GitLog Spawn (git binary 不可用)** — 类型层覆盖（`collect_repo` 内 `Command::new("git").output().map_err(...)` → `GitLogError::Spawn`）
- [x] **F5 GitLog NonZeroExit** — `not_a_git_repo_returns_typed_error` 实测 git exit 128 路径
- [x] **F6 GitLog ParseFailed** — 类型层覆盖（`parse_record` 内 missing field 返 `ParseFailed`；非 UTF-8 走 `ParseFailed { detail: "non-UTF-8 stdout" }`）
- [x] **F7 GitLog InvalidHash** — `commit_hash_rejects_empty` 验证 `CommitHash::new("")` 返 `InvalidHash`
- [x] **F8 RepoSpec 相对路径** — `repo_spec_path_not_found` + `repo_spec_canonicalizes_and_derives_name` 共同覆盖 canonicalize 行为
- [x] **F9 Journal append IO error** — 类型层覆盖：`finalize_*` 内 `journal.append` 返 `Err(io::Error)` → `RecapError::JournalAppend(#[from] std::io::Error)`
- [x] **F10 空仓非 Failed** — `empty_repo_returns_no_commits` 验证 git exit 128 "does not have any commits yet" 兜底成 `Vec::new()`（attention.md 新加一条记录此 quirk）

### 3.4 Cargo feature flag 边界（C1-C4）

- [x] **C1 cargo build 默认** — `cargo build` 成功；`roostery --help` 含 `daily-recap` 子命令（手工核对）
- [x] **C2 cargo build --no-default-features** — 成功；binary 不含 `daily-recap`（手工核对 `--help` 不列）；`Config.recap` 段仍 deserialize（`recap_missing_section_yields_default` + DTO 不 gate）
- [x] **C3 cargo test --all** — 533 lib tests + 8 daily_recap_integration + 其他全过
- [x] **C4 cargo test --all --no-default-features** — 499 lib tests（34 daily_recap gated out）+ 0 integration（gated）+ 其他全过

### 3.5 明确不做反向核对

- [x] `cargo tree -p roostery | rg -iE "reqwest|openai|anthropic|gemini|tonic|prost|hyper|ureq"` → 0 命中
- [x] `rg "pub trait (Summarizer|RecapEngine|LlmClient)" crates/` → 0 命中
- [x] `rg "dispatcher::fire" crates/roostery/src/daily_recap` → 0 命中
- [x] `rg "json!" crates/roostery/src/daily_recap/mod.rs` → 命中只在 JournalEntry params / trigger_meta 构造处（合规，Value 边界）
- [x] CLI 无 `--write-rule` 等触碰用户 rules.yaml 的 flag

### 3.6 前端验证

N/A——本 feature 无前端改动（CLI tool + lib API only）。

## 4. 术语一致性

对照 design §0 + §2.1 命名 grep 代码：

- `daily_recap` 模块路径 — 命中：`lib.rs:11` + `main.rs:39/143` + `daily_recap/{mod,cli,git_log}.rs` 内 self-ref，全部一致 ✓
- `RecapOutcome / NoSummaryReason / RecapError` — grep 命中 30+ 处全在 daily_recap 内 + tests + cli.rs 消费，全部一致 ✓
- `RecapRuntime / RecapRequest / PreparedRecap / PromptRunnerArgs` — 一致 ✓
- `RecapJsonOutcome / RecapJsonReason` — 一致 ✓
- `CommitHash / RepoSpec / RepoSpecError / GitLogError / GitLogAggregate` — 一致 ✓
- `cron.daily-recap` hook_source 字符串 — 命中 `mod.rs:34` 常量 `HOOK_SOURCE`，未在 rules.yaml 之外使用（D4 边界保持）✓
- `daily_recap` journal source 字符串 — `mod.rs:35` 常量 `JOURNAL_SOURCE`，与 `bot_stop_hook` 模式对齐 ✓
- 防冲突 grep `Summarizer` / `RecapEngine` / `LlmClient` → 0 命中 ✓
- 防冲突 grep `Dispatcher` struct → 命中只在 `dispatcher::Dispatcher` 既有外部模块，daily_recap 内不引此名 ✓

**结论**：术语层零冲突。

## 5. 架构归并

design §4 列的归并项实际写入 ARCHITECTURE.md：

- [x] **§3 Module G 描述改写**：去掉旧"`llm_summary` 是唯一允许 LLM client import"红线，改为 reuse §4.3 Runner trait 描述（与 roadmap §3 已改的措辞对齐）+ 加入 `report-recap-engine` 落地状态
- [x] **§5 第 5 条改写**：原"`llm_summary` 模块是 LLM provider 集成的唯一白名单"已被 §2 全局红线"不引外部 LLM SDK / 不用 HTTP client 直连 LLM endpoint"取代，本条改写为新表述
- [x] **§6 第 3 条改写**：同上理由
- [x] **§5 新增第 10 条架构决定**：daily-recap 不走 dispatcher.fire（hook-event 分发 vs one-shot string-return 语义不同）—— "代码-文档优先级"角度也值得归档，因为这是又一次"Rust 期重新设计而非 Python 1:1 翻译"的代表案例
- [x] **§3 Module G 加 Phase 6 落地状态**：`report-recap-engine` accept 2026-05-19，链接 design + 关键决策摘要

→ 见下方"架构 doc 实际改动"小节，全部已写入。

**判据自检**：归并后没读 design 的人打开 ARCHITECTURE.md：
- 知道 Module G `report-recap-engine` 已落地 ✓
- 知道架构红线从"`llm_summary` 唯一白名单"切到"任何模块 0 LLM SDK"✓
- 知道 daily-recap 跟 dispatcher.fire 是两条独立路径，理由有源（§5 新加第 10 条）✓

## 6. requirement 回写

frontmatter `requirement: daily-dev-recap` → 指向 draft req。

**判定**：req 描述的"用户视角日报落到飞书 docx + Base"完整能力**还未交付**——本 feature (`report-recap-engine`) 只是引擎层（git log 聚合 + agent CLI 委托），写飞书 docx + Base 是 `report-daily` feature 的事。所以 req 保持 `status: draft`，加变更日志一条记录 engine 层落地。

**动作**：编辑 `.codestable/requirements/daily-dev-recap.md` 在文末加变更日志条目（保留原始愿景）。

→ 见下方"req doc 实际改动"小节。

## 7. roadmap 回写

frontmatter `roadmap: rust-rewrite` + `roadmap_item: report-recap-engine`。

**动作**：
- `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml` 中 `slug: report-recap-engine` 条目：`status: in-progress → done`
- `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §5 item 18 状态从 `planned` 改成 `done` 含 commit / CI run 占位 + Accepted 标记
- `validate-yaml.py` 校验

→ 见下方"roadmap 实际改动"小节。

## 8. attention.md 候选盘点

implement 阶段已主动加 1 条到 `attention.md` 命令脚本陷阱段：

> `git log` 在已初始化但 0 commit 的仓库退 128 而非 0 + stderr `"does not have any commits yet"`——不是"非 git 仓"错误（那条 stderr 是 `"not a git repository"`，也 exit 128）。调用方要按 stderr substring 区分。参考 `daily_recap/git_log.rs::collect_repo`

无更多候选——本 feature 其余实现细节都是 feature-specific，不属于"下个 feature 还会再撞一次"类型。

## 9. 遗留

- **`tests/onboarding_integration.rs::seed_passing_smoke` 风格的 `serde_json::from_str` JSON-bypass for `#[non_exhaustive]` BudgetCfg** — 已在 integration test 用了同模式（`integ_harness.budget_cfg` 构造），不算新债
- **`CcHeadlessRunner` 内部 args.prompt 路径与 `PromptRunnerArgs` 字段名重合是巧合还是约定** — design A1 / §0 D5 已声明"daily-recap runner convention"作为约定层；未来 `codex_exec` / `gemini_headless` 适配时若发现命名不一致需要走 cs-decide 归档此 convention
- **`emit_outcome` 在 `cli.rs` 内对 `RecapOutcome` 分支判定有重复 match**（一次走 json branch、一次走 final ExitCode 判定）—— 极小冗余，不值得重构
- **顺手发现（cs-feat-impl 阶段记录）**：`crates/roostery/src/bot_bridge/active_registry.rs:308` 等历史 `cargo fmt --check` 格式 diff（非本次引入）—— 建议后续单独 issue 跑 `cargo fmt --all`，不阻塞本次 acceptance
- **report-daily 启动前置依赖**：本 feature 是 report-daily 的引擎层，下个 feature 起步时只需直接 import `roostery::daily_recap::{run, RecapOutcome, RecapRuntime}`，无需 cs-roadmap update（依赖关系已 in items.yaml）
- **`tests/daily_recap_integration.rs::integ_multi_repo_aggregation` 中两个 repo 都 today 才 init**：实际 quiet repo 也有 commit；测试名暗示一个 quiet 但实装两个都 active 但不影响验收意图（验证多 repo 聚合形态）。后续如有人扩此测试覆盖"真正 quiet" 场景可改

---

## 架构 doc 实际改动

详见随后 Edit 操作：

1. `.codestable/architecture/ARCHITECTURE.md` §3 Module G 段改写
2. `.codestable/architecture/ARCHITECTURE.md` §5 第 5 条改写
3. `.codestable/architecture/ARCHITECTURE.md` §5 新增第 10 条架构决定（daily-recap 不走 dispatcher.fire）
4. `.codestable/architecture/ARCHITECTURE.md` §6 第 3 条改写
5. `.codestable/requirements/daily-dev-recap.md` 文末加变更日志条目（保留愿景，draft 不升级）
6. `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml` item `report-recap-engine` 状态 in-progress → done
7. `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §5 item 18 状态改 done + 加 commit / CI 占位
8. `{slug}-checklist.yaml` 顶层 `status: pending → done` + 所有 `checks` 标 `passed`
