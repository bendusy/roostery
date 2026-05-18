//! Dispatcher loop — 串 trace/budget/runaway/rules/runners/journal 五模块为
//! `HookEvent in → DispatchOutcome out + journal` 主链路 + 三 CLI 入口
//! （`fire` / `replay` / `test_rule`）。
//!
//! Phase 4 Module E 收尾子 feature。本模块只做编排——所有飞书 IO 责任在
//! 具体 Runner impl 内部（如 CcHeadless 调 claude binary）或后续 Phase 5
//! `bot-task-writer` feature。dispatcher.rs 不消费 `LarkRunner` trait、不
//! 直接 `Command::new`、不引 `reqwest`——架构红线。
//!
//! See `.codestable/features/2026-05-18-dispatcher-loop/dispatcher-loop-design.md`
//! §2.1.1.
//!
//! 模块组织：Phase 4 / Module E 7 子模块全部聚在本目录下
//! （refactor `2026-05-18-module-e-subdir`，2026-05-18 acceptance 后落地）。

pub mod budget;
pub mod hook_event;
pub mod rules;
pub mod runaway;
pub mod runners;
pub mod trace;

use self::budget::{BudgetError, BudgetState};
use self::hook_event::HookEvent;
use self::runaway::{RunawayError, RunawayTracker};
use self::runners::{RunOutcome, RunnerRegistry, RunnerStatus};
use self::trace::{TraceContext, TraceId};
use crate::config;
use crate::config::BudgetCfg;
use crate::journal::{Journal, JournalEntry, JournalResult};
use crate::paths;
use std::collections::VecDeque;
use thiserror::Error;

/// 单次 fire 内单个 step 的 fanout 上限（防 runner 返巨量 emitted_events
/// 把队列撑爆）。`trace.max_depth` 守深度，本 const 守 width。
pub const DEFAULT_MAX_FANOUT: usize = 16;

/// dispatcher 单次 fire 编排结果总览（含链式分发的所有 step）。
#[derive(Debug)]
pub struct DispatchOutcome {
    pub trace_id: TraceId,
    pub root_event_id: String,
    /// 0 条 = 入口 gate 拒（trace.check_depth 失败 / rules 全不命中且根 event
    /// 自身不写 journal——但 design §3.2 C2.1 要求 no_match 也写 journal，所以
    /// dispatched 至少含 1 条）。
    pub dispatched: Vec<DispatchStep>,
}

#[derive(Debug)]
pub struct DispatchStep {
    pub event_id: String,
    pub hook_source: String,
    pub depth: u32,
    pub matched_rule: Option<String>,
    pub runner_kind: Option<String>,
    pub status: StepStatus,
    /// 该 step 触发并被消费（入队）的 emitted_events 个数（受 DEFAULT_MAX_FANOUT 截断）。
    pub fanout: usize,
}

#[derive(Debug, PartialEq, Clone)]
pub enum StepStatus {
    Success,
    Skipped { reason: String },
    GateRejected { reason: String },
    Failed { reason: String },
    NoMatch,
}

/// dispatcher 编排层错误（与 `RunnerError` / `RulesError` / `BudgetError` 分层）。
///
/// `fire` 入口加载阶段（`config::load` / `rules::load`）的错误走这里直接返给
/// caller；`fire` 主循环内的 gate / runner 错误**不**冒泡——全部走 `journal.append`
/// 落档 + `StepStatus` 反映。
///
/// `replay` / `test_rule` 走 `DispatchError`（直接显示给用户）。
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DispatchError {
    #[error("config load failed: {0}")]
    ConfigLoadFailed(#[source] config::ConfigError),
    #[error("rules load failed: {0}")]
    RulesLoadFailed(#[source] rules::RulesError),
    #[error("journal dir not found: {0}")]
    JournalDirNotFound(std::path::PathBuf),
    #[error("replay: no journal entry for trace_id {0}")]
    ReplayNotFound(String),
    #[error("replay: failed to reconstruct HookEvent from journal: {reason}")]
    EventReconstructFailed { reason: String },
    #[error("bad CLI input: {0}")]
    BadCliInput(String),
}

// --- fire 主入口 ----------------------------------------------------------

/// fire 主入口——消费 HookEvent，走 trace / rules / budget / runaway / runner
/// 主链路 + 链式分发 emitted_events（BFS 队列），每 step 写 journal，永不冒泡
/// 错误（错误全部走 journal + `StepStatus` 反映）。
///
/// `root_event.trace` 若为 `Some`，沿用其 `trace_id` / `depth` / `max_depth`（chain
/// dispatch 内部场景）；为 `None` 时分配新 root TraceContext（外部 hook 入口场景）。
pub async fn fire(
    root_event: HookEvent,
    registry: &RunnerRegistry,
    rules: &[rules::CompiledRule],
    cfg: &config::Config,
) -> DispatchOutcome {
    let max_depth = cfg.trace.max_depth;
    // 入口分配 / 复用 TraceContext。
    let root_ctx = match &root_event.trace {
        Some(t) => t.clone(),
        None => TraceContext::new_root(None, max_depth),
    };

    // 准备外部状态：budget / runaway 跨 step 共享。
    let mut budget_state = load_or_init_budget(&cfg.budgets.default);
    let mut runaway = RunawayTracker::new();
    let journal = Journal::open(paths::journal_dir());

    // BFS 队列：(event, ctx)。
    let mut queue: VecDeque<(HookEvent, TraceContext)> = VecDeque::new();
    queue.push_back((root_event, root_ctx.clone()));

    let mut dispatched: Vec<DispatchStep> = Vec::new();
    let mut root_event_id: Option<String> = None;

    while let Some((event, ctx)) = queue.pop_front() {
        let step = process_one(
            event,
            ctx,
            registry,
            rules,
            &mut budget_state,
            &mut runaway,
            &journal,
            &mut queue,
        )
        .await;
        if root_event_id.is_none() {
            root_event_id = Some(step.event_id.clone());
        }
        dispatched.push(step);
    }

    DispatchOutcome {
        trace_id: root_ctx.trace_id,
        root_event_id: root_event_id.unwrap_or_default(),
        dispatched,
    }
}

/// 单个 event 的处理 step——5 gate / 1 engine 顺序串场景。
/// 任何分支都 journal.append 一条 entry + 返 DispatchStep。
#[allow(clippy::too_many_arguments)]
async fn process_one(
    event: HookEvent,
    ctx: TraceContext,
    registry: &RunnerRegistry,
    rules: &[rules::CompiledRule],
    budget_state: &mut BudgetState,
    runaway: &mut RunawayTracker,
    journal: &Journal,
    queue: &mut VecDeque<(HookEvent, TraceContext)>,
) -> DispatchStep {
    let base = StepBase {
        event_id: crate::journal::new_event_id(),
        hook_source: event.hook_source.clone(),
        depth: ctx.depth,
    };
    let mut entry = JournalEntry::new("dispatcher", base.hook_source.clone());
    entry.event_id = base.event_id.clone();
    ctx.stamp_journal(&mut entry);
    entry.params = serde_json::json!({
        "session_id": event.session_id,
        "workspace": event.workspace,
        "trigger_meta": event.trigger_meta,
    });

    // Gate 1: trace.check_depth
    if let Err(e) = ctx.check_depth() {
        return reject(
            journal,
            entry,
            &base,
            None,
            None,
            StepStatus::GateRejected {
                reason: e.to_string(),
            },
        );
    }

    // rules.matches（NoMatch 也写 journal）
    let m = match rules::matches(rules, &event) {
        Some(m) => m,
        None => return reject(journal, entry, &base, None, None, StepStatus::NoMatch),
    };
    let matched_rule_name = m.rule_name.as_str().to_string();
    let runner_kind = m.runner.to_string();

    // Gate 2: budget.check_or_raise(0.0) — 守"是否已超额"
    // 其他 BudgetError 变体（LoadFailed / SaveFailed / ParseFailed /
    // SchemaVersionMismatch）不会从 check_or_raise 出来——只有 Exceeded。
    if let Err(BudgetError::Exceeded { reason, .. }) = budget_state.check_or_raise(0.0) {
        return reject(
            journal,
            entry,
            &base,
            Some(matched_rule_name),
            Some(runner_kind),
            StepStatus::GateRejected {
                reason: format!("budget: {reason}"),
            },
        );
    }

    // Gate 3: runaway.record + check
    runaway.record(&ctx.trace_id);
    if let Err(RunawayError::Detected {
        count,
        window_secs,
        threshold,
        ..
    }) = runaway.check(&ctx.trace_id)
    {
        return reject(
            journal,
            entry,
            &base,
            Some(matched_rule_name),
            Some(runner_kind),
            StepStatus::GateRejected {
                reason: format!("runaway: {count} fires in {window_secs}s (threshold {threshold})"),
            },
        );
    }

    // registry.find
    let runner = match registry.find(&runner_kind) {
        Some(r) => r,
        None => {
            return reject(
                journal,
                entry,
                &base,
                Some(matched_rule_name),
                Some(runner_kind.clone()),
                StepStatus::Skipped {
                    reason: format!("unknown runner kind: {runner_kind}"),
                },
            );
        }
    };

    // runner.run（async）
    let args = m.args.clone();
    let outcome: RunOutcome = match runner.run(&event, &ctx, &args).await {
        Ok(o) => o,
        Err(e) => {
            return reject(
                journal,
                entry,
                &base,
                Some(matched_rule_name),
                Some(runner_kind),
                StepStatus::Failed {
                    reason: format!("RunnerError: {e}"),
                },
            );
        }
    };

    let (status, fanout) = match &outcome.status {
        RunnerStatus::Success => {
            // budget.consume + save
            if let Some(cost) = outcome.cost_usd {
                budget_state.consume(cost);
            }
            // 即便 cost 是 None，本次成功仍计 1 次 call（consume(0.0)）以维持 max_calls 守门
            if outcome.cost_usd.is_none() {
                budget_state.consume(0.0);
            }
            let _ = budget::save(budget_state);

            // 链式分发 emitted_events（S4 实现：受 DEFAULT_MAX_FANOUT 截断 + ctx.child）
            let fanout = enqueue_emitted(queue, &ctx, &base.event_id, outcome.emitted_events);
            (StepStatus::Success, fanout)
        }
        RunnerStatus::Failed { reason } => (
            StepStatus::Failed {
                reason: reason.clone(),
            },
            0,
        ),
        RunnerStatus::Skipped { reason } => (
            StepStatus::Skipped {
                reason: reason.clone(),
            },
            0,
        ),
    };

    finalize_step(
        journal,
        entry,
        DispatchStep {
            event_id: base.event_id,
            hook_source: base.hook_source,
            depth: base.depth,
            matched_rule: Some(matched_rule_name),
            runner_kind: Some(runner_kind),
            status,
            fanout,
        },
    )
}

/// `process_one` 内每个 gate 拒绝路径共享的 step 头部字段。打包传递避免在
/// 5 处 rejection 调用站点各 clone 一遍。
struct StepBase {
    event_id: String,
    hook_source: String,
    depth: u32,
}

/// Gate-rejected / NoMatch / Skipped / Failed 共用的 step 构造 + journal 收尾。
/// `fanout` 恒为 0（这些路径不会链式分发 emitted_events）；成功路径走
/// `finalize_step` 直调。
fn reject(
    journal: &Journal,
    entry: JournalEntry,
    base: &StepBase,
    matched_rule: Option<String>,
    runner_kind: Option<String>,
    status: StepStatus,
) -> DispatchStep {
    finalize_step(
        journal,
        entry,
        DispatchStep {
            event_id: base.event_id.clone(),
            hook_source: base.hook_source.clone(),
            depth: base.depth,
            matched_rule,
            runner_kind,
            status,
            fanout: 0,
        },
    )
}

/// 把 step 写进 journal 并返还；journal append 失败仅吞（journal 是 best-effort
/// 审计，不阻塞 dispatch 主流程）。
fn finalize_step(journal: &Journal, mut entry: JournalEntry, step: DispatchStep) -> DispatchStep {
    entry.result = match &step.status {
        StepStatus::Success => JournalResult::Ok {
            value: serde_json::json!({
                "outcome": "success",
                "matched_rule": step.matched_rule,
                "runner_kind": step.runner_kind,
                "fanout": step.fanout,
            }),
        },
        StepStatus::NoMatch => JournalResult::Ok {
            value: serde_json::json!({"outcome": "no_match"}),
        },
        StepStatus::Skipped { reason } => JournalResult::Err {
            kind: "skipped".to_string(),
            message: reason.clone(),
        },
        StepStatus::GateRejected { reason } => JournalResult::Err {
            kind: "gate_rejected".to_string(),
            message: reason.clone(),
        },
        StepStatus::Failed { reason } => JournalResult::Err {
            kind: "failed".to_string(),
            message: reason.clone(),
        },
    };
    let _ = journal.append(&entry);
    step
}

/// 把 runner 返的 emitted_events 入队走链式分发；超 DEFAULT_MAX_FANOUT 截断。
/// 返实际入队个数。
fn enqueue_emitted(
    queue: &mut VecDeque<(HookEvent, TraceContext)>,
    parent_ctx: &TraceContext,
    parent_event_id: &str,
    emitted: Vec<HookEvent>,
) -> usize {
    let take = emitted.len().min(DEFAULT_MAX_FANOUT);
    for child_event in emitted.into_iter().take(take) {
        let child_ctx = parent_ctx.child(Some(parent_event_id.to_string()));
        queue.push_back((child_event, child_ctx));
    }
    take
}

/// 加载或初始化 BudgetState。文件不存在 / 加载失败时 fallback 到 from_cfg。
fn load_or_init_budget(cfg: &BudgetCfg) -> BudgetState {
    budget::load().unwrap_or_else(|_| BudgetState::from_cfg(cfg))
}

/// replay 入口——读 journal 找 trace_id 根 entry → 重建 HookEvent → 调 fire；
/// 分配新 trace_id（不沿用），journal 加 `replay_of: <source_trace_id>` 关联。
///
/// "根 entry" = 该 trace_id 内 `depth == 0` 的最早 entry；无则 ReplayNotFound。
pub async fn replay(
    source_trace_id: &str,
    registry: &RunnerRegistry,
    rules: &[rules::CompiledRule],
    cfg: &config::Config,
) -> Result<DispatchOutcome, DispatchError> {
    let dir = paths::journal_dir();
    if !dir.exists() {
        return Err(DispatchError::JournalDirNotFound(dir));
    }
    let entries = crate::journal::load_by_trace_id(&dir, source_trace_id).map_err(|e| {
        DispatchError::EventReconstructFailed {
            reason: format!("journal read failed: {e}"),
        }
    })?;
    if entries.is_empty() {
        return Err(DispatchError::ReplayNotFound(source_trace_id.to_string()));
    }
    let root_entry = entries
        .iter()
        .find(|e| e.depth == 0)
        .ok_or_else(|| DispatchError::ReplayNotFound(source_trace_id.to_string()))?;

    // 从 entry.params 重建 HookEvent 字段（fire 写入约定见 process_one）。
    let params = &root_entry.params;
    let session_id = params
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DispatchError::EventReconstructFailed {
            reason: "params.session_id missing".to_string(),
        })?
        .to_string();
    let workspace = params
        .get("workspace")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DispatchError::EventReconstructFailed {
            reason: "params.workspace missing".to_string(),
        })?
        .to_string();
    let trigger_meta = params
        .get("trigger_meta")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    // hook_source 在 entry.action（process_one 写入约定）
    let hook_source = root_entry.action.clone();

    let raw = serde_json::json!({
        "schema_version": self::hook_event::HOOK_EVENT_SCHEMA_VERSION,
        "hook_source": hook_source,
        "session_id": session_id,
        "workspace": workspace,
        "trigger_meta": trigger_meta,
        // 关联源 trace_id 给审计；fire 入口看到 trace=None 走新 root trace_id 分配
        "trace": null,
    });
    let mut event: HookEvent =
        serde_json::from_value(raw).map_err(|e| DispatchError::EventReconstructFailed {
            reason: format!("HookEvent deserialize: {e}"),
        })?;
    // 把 replay_of 塞进 trigger_meta（journal 自然带上）
    if let serde_json::Value::Object(ref mut map) = event.trigger_meta {
        map.insert(
            "replay_of".to_string(),
            serde_json::Value::String(source_trace_id.to_string()),
        );
    } else {
        // 非 Object 的 trigger_meta：转 Object 包一层
        event.trigger_meta = serde_json::json!({
            "original": event.trigger_meta,
            "replay_of": source_trace_id,
        });
    }
    Ok(fire(event, registry, rules, cfg).await)
}

/// test-rule 入口——`rules.matches` dry-run；不调 runner / 不写 journal /
/// 不消费 budget。
pub fn test_rule<'a>(
    event: &'a HookEvent,
    rules: &'a [rules::CompiledRule],
) -> Option<rules::Match<'a>> {
    rules::matches(rules, event)
}

#[cfg(test)]
#[allow(clippy::await_holding_lock)] // test ENV_LOCK serializes ROOSTERY_HOME mutation (attention.md pattern)
mod tests {
    use super::runners::Runner;
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Mutex;

    // Lock for env vars touched by paths::roostery_home / journal_dir / budget_state_path.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn dummy_event(hook_source: &str) -> HookEvent {
        let raw = json!({
            "schema_version": 1,
            "hook_source": hook_source,
            "session_id": "s_test",
            "workspace": "/tmp/test_workspace",
            "trigger_meta": {},
        });
        serde_json::from_value(raw).unwrap()
    }

    fn dummy_cfg(max_depth: u32, max_calls: u32, max_cost_usd: f64) -> config::Config {
        let mut c = config::Config::default();
        c.trace.max_depth = max_depth;
        c.budgets.default.max_calls = max_calls;
        c.budgets.default.max_cost_usd = max_cost_usd;
        c
    }

    fn isolate_home(tmp: &tempfile::TempDir) {
        // Force paths::roostery_home → tmp.path()
        unsafe { std::env::set_var("ROOSTERY_HOME", tmp.path()) };
    }

    fn restore_home() {
        unsafe { std::env::remove_var("ROOSTERY_HOME") };
    }

    /// 单字符 prefix rule，匹配 hook_source 字面量。
    fn simple_rules_yaml(hook_source: &str, runner: &str) -> Vec<u8> {
        format!(
            "schema_version: 1\nrules:\n  - name: r1\n    when:\n      hook_source: {hook_source}\n    action:\n      runner: {runner}\n      args: {{}}\n"
        )
        .into_bytes()
    }

    fn load_simple_rules(hook_source: &str, runner: &str) -> Vec<rules::CompiledRule> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), simple_rules_yaml(hook_source, runner)).unwrap();
        rules::load_from(tmp.path()).unwrap()
    }

    // --- Mock runners ----------------------------------------------------

    struct CostyRunner {
        kind: &'static str,
        cost: Option<f64>,
    }
    #[async_trait]
    impl Runner for CostyRunner {
        fn kind(&self) -> &'static str {
            self.kind
        }
        async fn run(
            &self,
            _: &HookEvent,
            _: &TraceContext,
            _: &serde_json::Value,
        ) -> Result<RunOutcome, super::runners::RunnerError> {
            Ok(RunOutcome {
                status: RunnerStatus::Success,
                stdout: String::new(),
                stderr: String::new(),
                emitted_events: Vec::new(),
                cost_usd: self.cost,
            })
        }
    }

    struct FailingRunner;
    #[async_trait]
    impl Runner for FailingRunner {
        fn kind(&self) -> &'static str {
            "failing"
        }
        async fn run(
            &self,
            _: &HookEvent,
            _: &TraceContext,
            _: &serde_json::Value,
        ) -> Result<RunOutcome, super::runners::RunnerError> {
            Ok(RunOutcome {
                status: RunnerStatus::Failed {
                    reason: "exit code 42".to_string(),
                },
                stdout: String::new(),
                stderr: String::new(),
                emitted_events: Vec::new(),
                cost_usd: None,
            })
        }
    }

    struct ChainRunner {
        kind: &'static str,
        emit: Vec<HookEvent>,
    }
    #[async_trait]
    impl Runner for ChainRunner {
        fn kind(&self) -> &'static str {
            self.kind
        }
        async fn run(
            &self,
            _: &HookEvent,
            _: &TraceContext,
            _: &serde_json::Value,
        ) -> Result<RunOutcome, super::runners::RunnerError> {
            Ok(RunOutcome {
                status: RunnerStatus::Success,
                stdout: String::new(),
                stderr: String::new(),
                emitted_events: self.emit.clone(),
                cost_usd: None,
            })
        }
    }

    struct ErrorRunner;
    #[async_trait]
    impl Runner for ErrorRunner {
        fn kind(&self) -> &'static str {
            "errorer"
        }
        async fn run(
            &self,
            _: &HookEvent,
            _: &TraceContext,
            _: &serde_json::Value,
        ) -> Result<RunOutcome, super::runners::RunnerError> {
            Err(super::runners::RunnerError::BinaryNotFound {
                kind: "errorer",
                path: PathBuf::from("/nowhere/claude"),
            })
        }
    }

    // --- S1 type tests ---------------------------------------------------

    #[test]
    fn default_max_fanout_is_sixteen() {
        assert_eq!(DEFAULT_MAX_FANOUT, 16);
    }

    #[test]
    fn step_status_variants_distinguishable() {
        let success = StepStatus::Success;
        let no_match = StepStatus::NoMatch;
        let skipped = StepStatus::Skipped {
            reason: "x".to_string(),
        };
        let gate = StepStatus::GateRejected {
            reason: "y".to_string(),
        };
        let failed = StepStatus::Failed {
            reason: "z".to_string(),
        };
        assert_ne!(success, no_match);
        assert_ne!(skipped, gate);
        assert_ne!(gate, failed);
        // Same-payload equality.
        assert_eq!(
            StepStatus::Skipped {
                reason: "a".to_string()
            },
            StepStatus::Skipped {
                reason: "a".to_string()
            },
        );
    }

    #[test]
    fn dispatch_error_display_includes_context() {
        let err = DispatchError::ReplayNotFound("abc123".to_string());
        let msg = err.to_string();
        assert!(msg.contains("abc123"));
        let err2 = DispatchError::EventReconstructFailed {
            reason: "missing hook_source".to_string(),
        };
        assert!(err2.to_string().contains("missing hook_source"));
        let err3 = DispatchError::BadCliInput("no --agent given".to_string());
        assert!(err3.to_string().contains("no --agent given"));
    }

    // --- S3 fire main-path tests -----------------------------------------

    #[tokio::test]
    async fn fire_happy_success_writes_journal_and_consumes_budget() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);
        let cfg = dummy_cfg(8, 100, 10.0);
        let rules = load_simple_rules("cc-stop", "costy");
        let registry = RunnerRegistry::new().with_runner(Box::new(CostyRunner {
            kind: "costy",
            cost: Some(0.5),
        }));
        let event = dummy_event("cc-stop");
        let outcome = fire(event, &registry, &rules, &cfg).await;
        assert_eq!(outcome.dispatched.len(), 1);
        assert_eq!(outcome.dispatched[0].status, StepStatus::Success);
        assert_eq!(outcome.dispatched[0].matched_rule.as_deref(), Some("r1"));
        assert_eq!(outcome.dispatched[0].runner_kind.as_deref(), Some("costy"));
        let entries = crate::journal::load_by_trace_id(
            &crate::paths::journal_dir(),
            outcome.trace_id.as_str(),
        )
        .unwrap();
        assert_eq!(entries.len(), 1);
        let bs = budget::load().unwrap();
        assert!((bs.default.cost_usd - 0.5).abs() < 1e-9);
        assert_eq!(bs.default.calls, 1);
        restore_home();
    }

    #[tokio::test]
    async fn fire_no_match_writes_journal_no_match() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);
        let cfg = dummy_cfg(8, 100, 10.0);
        let rules = load_simple_rules("cc-stop", "noop");
        let registry = RunnerRegistry::with_defaults();
        let event = dummy_event("other-stop");
        let outcome = fire(event, &registry, &rules, &cfg).await;
        assert_eq!(outcome.dispatched.len(), 1);
        assert_eq!(outcome.dispatched[0].status, StepStatus::NoMatch);
        assert!(outcome.dispatched[0].matched_rule.is_none());
        restore_home();
    }

    #[tokio::test]
    async fn fire_unknown_runner_kind_is_skipped() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);
        let cfg = dummy_cfg(8, 100, 10.0);
        let rules = load_simple_rules("cc-stop", "nonexistent_kind");
        let registry = RunnerRegistry::with_defaults();
        let event = dummy_event("cc-stop");
        let outcome = fire(event, &registry, &rules, &cfg).await;
        assert_eq!(outcome.dispatched.len(), 1);
        match &outcome.dispatched[0].status {
            StepStatus::Skipped { reason } => {
                assert!(reason.contains("unknown runner kind"));
                assert!(reason.contains("nonexistent_kind"));
            }
            other => panic!("expected Skipped, got {other:?}"),
        }
        restore_home();
    }

    #[tokio::test]
    async fn fire_budget_over_gate_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);
        let cfg = dummy_cfg(8, 0, 0.0);
        let rules = load_simple_rules("cc-stop", "noop");
        let registry = RunnerRegistry::with_defaults();
        let event = dummy_event("cc-stop");
        let outcome = fire(event, &registry, &rules, &cfg).await;
        assert_eq!(outcome.dispatched.len(), 1);
        match &outcome.dispatched[0].status {
            StepStatus::GateRejected { reason } => {
                assert!(reason.contains("budget"));
            }
            other => panic!("expected GateRejected, got {other:?}"),
        }
        restore_home();
    }

    #[tokio::test]
    async fn fire_runner_failed_marks_step_failed() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);
        let cfg = dummy_cfg(8, 100, 10.0);
        let rules = load_simple_rules("cc-stop", "failing");
        let registry = RunnerRegistry::new().with_runner(Box::new(FailingRunner));
        let event = dummy_event("cc-stop");
        let outcome = fire(event, &registry, &rules, &cfg).await;
        assert_eq!(outcome.dispatched.len(), 1);
        match &outcome.dispatched[0].status {
            StepStatus::Failed { reason } => {
                assert!(reason.contains("42"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        restore_home();
    }

    // --- S4 emitted_events 链式分发 tests --------------------------------

    #[tokio::test]
    async fn fire_chain_two_layers_propagates_trace_and_depth() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);
        let cfg = dummy_cfg(8, 100, 10.0);
        // 第一层匹配 cc-stop → chain_runner；chain_runner 返 1 个 emitted_event
        // hook_source="downstream-step"；第二层匹配 downstream-step → noop。
        let yaml = "schema_version: 1\nrules:\n  - name: r1\n    when: {hook_source: cc-stop}\n    action: {runner: chain1, args: {}}\n  - name: r2\n    when: {hook_source: downstream-step}\n    action: {runner: noop, args: {}}\n";
        let tmp_rules = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp_rules.path(), yaml).unwrap();
        let rules = rules::load_from(tmp_rules.path()).unwrap();
        let child_event = dummy_event("downstream-step");
        let registry = RunnerRegistry::new()
            .with_runner(Box::new(ChainRunner {
                kind: "chain1",
                emit: vec![child_event],
            }))
            .with_runner(Box::new(super::runners::NoopRunner));
        let root = dummy_event("cc-stop");
        let outcome = fire(root, &registry, &rules, &cfg).await;
        assert_eq!(outcome.dispatched.len(), 2);
        assert_eq!(outcome.dispatched[0].depth, 0);
        assert_eq!(outcome.dispatched[0].status, StepStatus::Success);
        assert_eq!(outcome.dispatched[0].fanout, 1);
        assert_eq!(outcome.dispatched[1].depth, 1);
        assert_eq!(outcome.dispatched[1].status, StepStatus::Success);
        assert_eq!(outcome.dispatched[1].runner_kind.as_deref(), Some("noop"));
        restore_home();
    }

    #[tokio::test]
    async fn fire_chain_over_max_depth_gates_at_boundary() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);
        // max_depth=1：root depth=0 走，child depth=1 应 GateRejected
        let cfg = dummy_cfg(1, 100, 10.0);
        let yaml = "schema_version: 1\nrules:\n  - name: r1\n    when: {hook_source: cc-stop}\n    action: {runner: chain1, args: {}}\n  - name: r2\n    when: {hook_source: downstream-step}\n    action: {runner: noop, args: {}}\n";
        let tmp_rules = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp_rules.path(), yaml).unwrap();
        let rules = rules::load_from(tmp_rules.path()).unwrap();
        let child_event = dummy_event("downstream-step");
        let registry = RunnerRegistry::new()
            .with_runner(Box::new(ChainRunner {
                kind: "chain1",
                emit: vec![child_event],
            }))
            .with_runner(Box::new(super::runners::NoopRunner));
        let root = dummy_event("cc-stop");
        let outcome = fire(root, &registry, &rules, &cfg).await;
        assert_eq!(outcome.dispatched.len(), 2);
        assert_eq!(outcome.dispatched[0].status, StepStatus::Success);
        match &outcome.dispatched[1].status {
            StepStatus::GateRejected { reason } => {
                assert!(reason.contains("depth"));
            }
            other => panic!("expected GateRejected, got {other:?}"),
        }
        restore_home();
    }

    #[tokio::test]
    async fn fire_fanout_truncated_at_default_cap() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);
        let cfg = dummy_cfg(8, 1000, 100.0);
        let yaml = "schema_version: 1\nrules:\n  - name: r1\n    when: {hook_source: cc-stop}\n    action: {runner: chain1, args: {}}\n  - name: r2\n    when: {hook_source: downstream-step}\n    action: {runner: noop, args: {}}\n";
        let tmp_rules = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp_rules.path(), yaml).unwrap();
        let rules = rules::load_from(tmp_rules.path()).unwrap();
        // 发 30 条 child（超 DEFAULT_MAX_FANOUT=16）
        let emit: Vec<HookEvent> = (0..30).map(|_| dummy_event("downstream-step")).collect();
        let registry = RunnerRegistry::new()
            .with_runner(Box::new(ChainRunner {
                kind: "chain1",
                emit,
            }))
            .with_runner(Box::new(super::runners::NoopRunner));
        let root = dummy_event("cc-stop");
        let outcome = fire(root, &registry, &rules, &cfg).await;
        // 1 root + 16 children = 17
        assert_eq!(outcome.dispatched.len(), 17);
        assert_eq!(outcome.dispatched[0].fanout, DEFAULT_MAX_FANOUT);
        restore_home();
    }

    #[tokio::test]
    async fn fire_runner_error_marks_step_failed() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);
        let cfg = dummy_cfg(8, 100, 10.0);
        let rules = load_simple_rules("cc-stop", "errorer");
        let registry = RunnerRegistry::new().with_runner(Box::new(ErrorRunner));
        let event = dummy_event("cc-stop");
        let outcome = fire(event, &registry, &rules, &cfg).await;
        assert_eq!(outcome.dispatched.len(), 1);
        match &outcome.dispatched[0].status {
            StepStatus::Failed { reason } => {
                assert!(reason.contains("RunnerError"));
                assert!(reason.contains("binary not found"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        restore_home();
    }

    // --- S5 replay tests ----------------------------------------------------

    #[tokio::test]
    async fn replay_happy_runs_again_with_new_trace_id() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);
        let cfg = dummy_cfg(8, 100, 10.0);
        let rules = load_simple_rules("cc-stop", "noop");
        let registry = RunnerRegistry::with_defaults();
        let event = dummy_event("cc-stop");
        let first = fire(event, &registry, &rules, &cfg).await;
        let original_trace = first.trace_id.as_str().to_string();
        // 现在 replay
        let replayed = replay(&original_trace, &registry, &rules, &cfg)
            .await
            .unwrap();
        assert_eq!(replayed.dispatched.len(), 1);
        assert_eq!(replayed.dispatched[0].status, StepStatus::Success);
        // 新 trace_id
        assert_ne!(replayed.trace_id.as_str(), original_trace);
        // journal 现含 2 条不同 trace_id 的 success entry
        let orig_entries =
            crate::journal::load_by_trace_id(&crate::paths::journal_dir(), &original_trace)
                .unwrap();
        let new_entries = crate::journal::load_by_trace_id(
            &crate::paths::journal_dir(),
            replayed.trace_id.as_str(),
        )
        .unwrap();
        assert_eq!(orig_entries.len(), 1);
        assert_eq!(new_entries.len(), 1);
        // replay_of 标记
        let trigger_meta = &new_entries[0].params["trigger_meta"];
        assert_eq!(
            trigger_meta["replay_of"].as_str(),
            Some(original_trace.as_str())
        );
        restore_home();
    }

    #[tokio::test]
    async fn replay_unknown_trace_id_returns_not_found() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);
        // 先建一下 journal_dir 让 JournalDirNotFound 不先返
        std::fs::create_dir_all(crate::paths::journal_dir()).unwrap();
        let cfg = dummy_cfg(8, 100, 10.0);
        let rules: Vec<rules::CompiledRule> = Vec::new();
        let registry = RunnerRegistry::with_defaults();
        match replay("nonexistent_trace_id_xyz", &registry, &rules, &cfg).await {
            Err(DispatchError::ReplayNotFound(tid)) => {
                assert_eq!(tid, "nonexistent_trace_id_xyz");
            }
            other => panic!("expected ReplayNotFound, got {other:?}"),
        }
        restore_home();
    }

    // --- S6 test_rule tests -------------------------------------------------

    #[test]
    fn test_rule_match_returns_some_with_rule_meta() {
        let rules = load_simple_rules("cc-stop", "noop");
        let event = dummy_event("cc-stop");
        let m = test_rule(&event, &rules).expect("should match");
        assert_eq!(m.rule_name.as_str(), "r1");
        assert_eq!(m.runner, "noop");
    }

    #[test]
    fn test_rule_no_match_returns_none() {
        let rules = load_simple_rules("cc-stop", "noop");
        let event = dummy_event("totally-different-source");
        assert!(test_rule(&event, &rules).is_none());
    }

    #[tokio::test]
    async fn replay_root_entry_with_missing_params_returns_reconstruct_err() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        isolate_home(&tmp);
        // 手工写一条 trace_id="t1" 的 root entry 但 params 缺 session_id
        std::fs::create_dir_all(crate::paths::journal_dir()).unwrap();
        let entry = JournalEntry {
            schema_version: 1,
            event_id: "evt1".to_string(),
            trace_id: Some("t1".to_string()),
            parent_event_id: None,
            depth: 0,
            ts: chrono::Utc::now(),
            source: "dispatcher".to_string(),
            action: "cc-stop".to_string(),
            // 缺 session_id 字段
            params: serde_json::json!({"workspace": "/tmp", "trigger_meta": {}}),
            result: JournalResult::Ok {
                value: serde_json::Value::Null,
            },
            duration_ms: 0,
        };
        let j = Journal::open(crate::paths::journal_dir());
        j.append(&entry).unwrap();
        let cfg = dummy_cfg(8, 100, 10.0);
        let rules: Vec<rules::CompiledRule> = Vec::new();
        let registry = RunnerRegistry::with_defaults();
        match replay("t1", &registry, &rules, &cfg).await {
            Err(DispatchError::EventReconstructFailed { reason }) => {
                assert!(reason.contains("session_id"));
            }
            other => panic!("expected EventReconstructFailed, got {other:?}"),
        }
        restore_home();
    }
}
