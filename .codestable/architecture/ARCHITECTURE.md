# 🪺 Roostery 架构总入口

> 状态：active（Rust 重写期更新）
> 创建日期：2026-05-15
> 末次刷新：2026-05-19（feature `bot-bridge-cluster` accepted——Module F 第 3 子 feature 落地：bot_bridge 9 子模块簇 + per-bot daemon + IM HITL oneshot 通道 + chat→task 接力缓存；新增 §5 决定 9 / §6 #20-22 / §2 7 条术语 / §3 Module F bot_bridge 段）

## 1. 项目简介

**Roostery** — vendor-neutral, Feishu-native agent broker。本地 daemon，将任意 agent runtime（Claude Code / Codex / Gemini / OpenClaw / 自定义 Python）桥接到飞书（Lark）作为**跨设备 vibecoding 协作面**。核心动机见 `.codestable/brainstorms/v0.x-direction/`。

**阶段**：Rust **0.1.0 已 tag**（2026-05-18）。"Rust 可用" 判据 = Phase 5 minimal-loop closing 已达成（feature `bot-stop-hook` 2026-05-18 落地），首个 release 形态完成；0.2.0 待 crates.io publish 决策。

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
| `LarkRunner` trait | Rust 期 lark-cli wrapper 的抽象接口（async + Send + Sync），下游所有模块依赖 trait 而非具体 struct；已落地 `crates/roostery/src/lark_cli/`（feature `2026-05-16-lark-cli-wrapper`，commit `cc44dfa`）。三实现：`LarkCli`（subprocess）/ `MockLarkRunner`（测试替身）/ `Journaled<R>`（journal 装饰器）。`run` 和 `run_with_options` 双 method 见 roadmap §4.1 |
| `LarkError` | `#[non_exhaustive]` rich enum + thiserror，4 变体 `Spawn` / `NonZeroExit` / `OutputParse` / `Timeout`，每变体携带专有数据；`retriable()` method（非字段）。caller 必经 `match` + `_ =>` 处理（外部 crate E0004 守护）。见 roadmap §4.1 |
| Dispatcher | 本地事件 → 规则匹配 → runner 执行的桥接层（Module E，Phase 4 完成）。落地于 `crates/roostery/src/dispatcher.rs` + 5 上游 gate / engine 模块（trace / budget / runaway / rules / runners） |
| Journal | 本地 jsonl 审计日志（默认 `~/.roostery/journal/`，可 `$ROOSTERY_HOME` 覆盖），仅作 replayable audit + portable data |
| `JournalEntry` schema | journal 单行结构，11 字段；`schema_version=1` 自 journal-core 落地起对外公开承诺，破坏性改动需 bump + 旧版兼容反序列化 + cs-roadmap update（见 roadmap §4.2） |
| `ROOSTERY_HOME` | Roostery 本地 state 根目录的环境变量覆盖；未设时默认 `~/.roostery/`。**不再读** Python 期 `FEISHU_HUB_HOME`（一次性切换） |
| Trace | `trace_id` / `depth` / `parent_event_id` 链，loop 保护用（见 roadmap §4.5） |
| Budget | 调用次数与成本上限 |
| Roost | 项目名含义——agent 来此栖息，离开时带着自己的痕迹走（不锁定在某一协作平面） |
| `MASK` | redact 模块的脱敏占位字符串，固定为 `"***"`（`crates/roostery/src/redact.rs`，Phase 1 落地） |
| `SENSITIVE_KEYS` | redact 模块默认敏感字段名列表（11 个：7 个 Python parity + 4 个业界扩展 `password` / `secret` / `cookie` / `private_key`），归一化后比较 |
| Logging-boundary scrubber | redact 模块的定位：对**已 flow 到 logging 边界的数据**做脱敏。与 `redact::Secret<T>` / `secrecy::SecretString` 等 in-memory wrapper crate 不同层——本项目本 phase 不引入这类 wrapper |
| Newtype token | remoterefs 模块的 9 个独立类型（`MessageId` / `DocToken` / `FolderToken` / `RecordId` / `ChatId` / `AppToken` / `WikiToken` / `TaskId` / `ThreadId`），全部 `#[serde(transparent)]`——JSON 形态裸字符串与 Python 版兼容，Rust 类型互不兼容，下游 cross-wiring bug 编译期拦截。`RemoteRefs` 容器 `#[non_exhaustive]` 防外部 struct literal 构造 |
| `shim` 二进制 | Phase 2 落地的 `bin/shim`（`src/bin/shim.rs`，feature `2026-05-17-lark-cli-shim`）。装到 `~/.local/bin/lark-cli` 作为 PATH-prefix 拦截点；agent runtime 调 `lark-cli` 时被透明截获，shim 流式 tee stdout/stderr 给用户 + 写一条 source="shim" 的 `JournalEntry`，最后 `exec()` / 等待 real lark-cli。与 `Journaled<LarkCli>` 装饰器是两条独立 I/O 路径：shim 是 **streaming bytes**（std::thread + std::process + tee），`Journaled<LarkCli>` 是 **buffered Value**（tokio + serde_json::Value）；语义不同所以不强行抽公共 trait |
| `ROOSTERY_REAL_LARK_CLI` | shim 读取的 env，指向 real lark-cli 二进制路径；不设 shim 退 127。由 Phase 3 `roostery init` 装机时写入。**不再读** Python 期 `FEISHU_HUB_REAL_LARK_CLI` |
| `ROOSTERY_NOJOURNAL` | shim 读取的 env，设为 `"1"` 时跳过写完整 journal entry（仍跑 real lark-cli 并 tee 给用户，只追加一条 `action="lark-cli:{verb}:skipped"` + `params.reason="nojournal"` 的标记 entry）。**不再读** Python 期 `FEISHU_HUB_NOJOURNAL` |
| `Smoke` 模块 | Phase 2 落地的 `crates/roostery/src/smoke.rs`（feature `2026-05-17-roostery-smoke`）。`PROBE_MATRIX` 6 条 `lark-cli {sub} ... --dry-run` 命令（im / docs / drive）顺序跑，分类 `Dry Run` marker + rc==0 = ok；写状态快照 `~/.roostery/state/smoke.json`（`SmokeReport` `schema_version=1` + `lark_cli_version` 字段助升级漂移诊断 + atomic `.tmp` + rename）。公开 `pub fn run() -> SmokeReport` + `pub fn ensure_ready() -> Result<(), SmokeError>` 两条 API。`SmokeError` `#[non_exhaustive]` 4 变体（NeverRun / LastFailed / StateLoadFailed / BinaryNotFound）。**不调 LarkRunner trait**——raw bytes 模型（检 stdout 文本 marker）vs buffered Value 模型语义不同（同 shim 决定） |
| `roostery smoke` 子命令 | clap derive 主 bin 的第一个真正子命令（feature `2026-05-17-roostery-smoke` 引入 clap 4 derive 作为项目首个 CLI 解析器，后续 init / dispatch 复用）。退 0 = `all_ok` / 退 1 = 至少一条 probe 失败；`--version` 锁定 `roostery 0.0.0 (rust)`（`#[command(version = concat!(env!("CARGO_PKG_VERSION"), " (rust)"))]`）|
| `Config` | Phase 3 落地的 `crates/roostery/src/config.rs`（feature `2026-05-17-config-yaml`）。顶层 6 字段 `#[non_exhaustive]`：`schema_version: u32` (1) / `identity: Identity { user_id, default_chat_id, default_task_app_token }` / `runners: BTreeMap<String, serde_yml::Value>`（开放结构——加新 runner kind 不动 schema 顶层）/ `budgets: Budgets { default: BudgetCfg { max_calls=100, max_cost_usd=1.0 } }` / `trace: TraceConfig { max_depth=8 }` / `journal: JournalConfig { dir, rotation="daily" }`。所有字段 `#[serde(default)]`——任意子集 YAML 都能反序列化（roadmap §4.6 "顶层字段缺失用编译期默认值" 兑现）。`ConfigError` `#[non_exhaustive]` 4 变体（LoadFailed / ParseFailed / SaveFailed / SchemaVersionMismatch）。4 公开 fn：`load` / `load_from(&Path)` / `save(&cfg)` / `save_to(&cfg, &Path)`（atomic `.tmp` + rename）。**不读 env override**（各模块自管，与 `lark_cli/subprocess.rs::ENV_BIN` 等不耦合）；**不实现 schema migration**（v2 落地时由 cs-roadmap update 评估）|
| YAML lib | `serde_yml = "0.0.12"`——`serde_yaml` 2024 起 unmaintained，`serde_yml` 是主流 maintained fork（drop-in replacement）。仅 `config` 模块直接 import；其他模块不引 YAML 依赖 |
| `hooks_merge` 模块 | Phase 3 落地的 `crates/roostery/src/hooks_merge.rs`（feature `2026-05-18-hooks-merge`）。JSON 深合并把 Stop hook 片段注入 `~/.claude/settings.json` / `~/.codex/hooks.json`，按 event key + matcher + command tail 三层幂等去重；3 个模板用 `include_str!` 编译期嵌入：`CC_STOP_HOOK_JSON` + `CODEX_STOP_HOOK_JSON` + `STOP_HOOK_AGENT_NOTIFY_SH`（roadmap §4.7 兑现首例）；3 公开 fn `render_template` / `merge_event_hook` / `apply_template`；`HooksError` `#[non_exhaustive]` 4 变体；JSON 输出 `indent=2` + `\n` trailing（Python golden file byte-for-byte 除 env 前缀偏离）；atomic `.tmp` + rename |
| `ROOSTERY_AGENT` env | Stop hook command 拼前缀 `ROOSTERY_AGENT=cc` / `=codex` 让下游 stop bridge sh 识别 runtime；**不沿用** Python `FEISHU_HUB_AGENT`（roadmap items.yaml notes "除非文档另有规定" 明示偏离）。与 `ROOSTERY_HOME` / `ROOSTERY_LARK_CLI_BIN` / `ROOSTERY_REAL_LARK_CLI` / `ROOSTERY_NOJOURNAL` 同 prefix |
| `src/templates/` 资源文件子目录 | 项目首次引入"非 Rust 源码资源文件子目录"模式（feature `2026-05-18-hooks-merge` 落地）。纯文本资源（.json / .sh），用 `include_str!` 引用，不进 `pub mod` 声明；rust-module-organization decision 拟扩展第 5 档归档 |
| `Identity` | Phase 3 落地的 `crates/roostery/src/identity.rs`（feature `2026-05-18-roostery-init`）。Immutable snapshot of active lark-cli profile + auth state：`#[non_exhaustive]` struct，7 字段全 `private` + accessor 返 `Option<&str>`（typestate-lite，禁直暴 `Option<String>`）：profile_name / user_open_id / user_name / bot_app_id / brand / token_status / host。method `short_user` / `short_bot` / `is_ready` / `describe`。`pub async fn current(runner: &dyn LarkRunner) -> Result<Identity, IdentityError>`（经 LarkRunner trait 调 `auth status` + `profile list`）；`IdentityError` `#[non_exhaustive]` 2 变体（AuthStatusFailed / ProfileListFailed）。**Roostery 不发明 identity**——只 reflect lark-cli profile，与 `config::Identity { user_id, default_chat_id, default_task_app_token }` 不同维度（一个是运行时 reflect，一个是 schema-defined static）|
| `AgentSpec` | `crates/roostery/src/agent_detect.rs`（feature `2026-05-18-roostery-init`）。`#[non_exhaustive]` struct + `&'static str` 字段：`kind: AgentKind` / `cli: &'static str` / `hooks_target: &'static str`（未展开 `~/`，consumer 走 `expanded_hooks_target()`）。`pub const AGENTS: &[AgentSpec]` 3 项（Cc/Codex/Gemini）|
| `AgentKind::Gemini` | `hooks_merge.rs:43` `AgentKind` enum 第 3 变体（hooks-merge 起 cc/codex 二项 + roostery-init 顺手加 gemini）。对应模板 `GEMINI_STOP_HOOK_JSON` `include_str!` 编译期嵌入 `templates/gemini_stop_hook.json`；Gemini CLI 走 `~/.gemini/settings.json` SessionEnd event |
| `ShellKind` | `onboarding.rs:98`（feature `2026-05-18-roostery-init`）。`#[non_exhaustive]` enum { Zsh, Bash }——其他 shell（fish / nushell）目前不支持。`detect_from_env()` 走 `$SHELL` ends-with 检测；`rc_path()` 返 `~/.zshrc` / `~/.bashrc` |
| `RealLarkCliSource` | `onboarding.rs`（feature `2026-05-18-init-real-lark-cli-override`）。`#[non_exhaustive]` 3 变体 `Flag` / `Env` / `PathDetected` + `Display` 实现（lowercase "flag" / "env" / "path"）。`InitReport.real_lark_cli_source` 暴露真 lark-cli 解析路径来源；`format_report` 输出 "real: {path} (from {source})" 让用户知道走的哪条 fallback 链 |
| `OnboardingError 3 sub-variants` | `onboarding.rs:84-102`（feature `2026-05-18-init-real-lark-cli-override`，替换原 `RealLarkCliMissing` 单变体）。`LarkCliNotInPath`（PATH 上 0 候选）/ `LarkCliCollidesShimTarget { found_at, shim_target }`（PATH 唯一候选 == shim 安装目标，npm 全局 prefix 撞 `~/.local/bin/lark-cli` 经典场景）/ `OverrideInvalid { path, reason }`（flag / env 路径不存在或是目录）。每条 `Display` 含 fix hint（提示 `--real-lark-cli` flag + `ROOSTERY_LARK_CLI_BIN` env）|
| `roostery init` 子命令 | `crates/roostery/src/main.rs Command::Init(InitArgs)`（feature `2026-05-18-roostery-init` + UX fix `2026-05-18-init-real-lark-cli-override`）。`--dry-run` + `--skip-agent <AGENT>`（可重复）+ `--real-lark-cli <PATH>`（真 lark-cli 路径 override，绕过 PATH 搜索）；handler 起 tokio current_thread runtime block_on `onboarding::run`。装机流水线（**resolve early-gate 后**）：F1 smoke gate → **resolve_real_lark_cli（flag > `ROOSTERY_LARK_CLI_BIN` env > PATH 搜索三层链；失败零文件副作用）** → F3 identity reflect → F4 agent_detect → F2 mkdir state → F5 install_shim（current_exe sibling + sha2 hash 比对幂等）→ F6 write_sh_bridge → F7 merge_hooks per installed agent → F8 write `~/.roostery/env`（含 `export ROOSTERY_REAL_LARK_CLI=<resolved>`）+ patch_shell_rc（marker block `# >>> roostery >>>` / `# <<< roostery <<<` 幂等）→ format_report（输出 "real: ... (from {flag\|env\|path})"）|
| `TraceContext` | Phase 4 落地的 `crates/roostery/src/trace.rs`（feature `2026-05-18-dispatcher-trace-budget`）。`#[non_exhaustive]` 4 字段（`trace_id: TraceId` / `parent_event_id: Option<String>` / `depth: u32` / `max_depth: u32`，**depth 从 0 起步**——与 Python 1-based parity 偏离，按 roadmap §4.5 docs-authority）；`new_root` 起 fresh trace_id + depth=0，`child(parent_event_id)` 返新值 depth+1（不可变），`check_depth` `>= max_depth` 返 `TraceError::DepthExceeded`，`to_env_pairs` / `from_env` 用 `ROOSTERY_TRACE_ID` / `ROOSTERY_DEPTH` / `ROOSTERY_PARENT_EVENT_ID` env 跨 process 传递，`stamp_journal(&mut entry)` 写 trace 三字段到 JournalEntry 不动其他字段 |
| `TraceId` | `trace.rs:36` `(String)` `#[serde(transparent)]` newtype（16-byte hex via getrandom，32 hex chars 编码）。`PartialOrd / Ord / Hash` derive 让 `BTreeMap<TraceId, _>` 索引可行。与 `business-identifier-newtype` decision §6 一致——禁用直接 String 互转，编译期防 cross-wiring 与 event_id / parent_event_id 等其他 id 类字符串 |
| `BudgetState` / `Bucket` | `crates/roostery/src/budget.rs`（feature `2026-05-18-dispatcher-trace-budget`）。`BudgetState` `#[non_exhaustive]` 3 字段（`schema_version: u32` const 1 / `day: NaiveDate` / `default: Bucket`，仅 default 单 bucket——roadmap §4.6 当前形状；per-runner / per-rule 等粒度走未来 `cs-roadmap update`）；`Bucket { calls, cost_usd, max_calls, max_cost_usd }` 全 f64 USD（**不沿用 Python i64 cents**）；线性流水 `from_cfg → roll_over_if_needed → check_or_raise → consume → save`；`roll_over_if_needed` 跨日 reset 触发，每次 `check_or_raise` / `consume` 前内部调一次让 tail-running daemons 过午夜也正确；持久化 `~/.roostery/state/budget.json` atomic `.tmp` + rename + 缺父目录自建 + JSON pretty + `\n` trailing；`BudgetError` `#[non_exhaustive]` 5 变体（LoadFailed / ParseFailed / SaveFailed / Exceeded / SchemaVersionMismatch） |
| `BUDGET_SCHEMA_VERSION` | `budget.rs:25` 公开 const `= 1`。**公开承诺**：bump 需 `cs-roadmap update` + 旧版兼容反序列化（同 journal `JournalEntry.schema_version` 模型） |
| `RunawayTracker` | `crates/roostery/src/runaway.rs`（feature `2026-05-18-dispatcher-trace-budget`）。事后兜底防御层（trace `check_depth` 是事前防御，runaway 是事后阈值兜底）。`window: Duration` + `threshold: u32` + `fires: BTreeMap<TraceId, Vec<Instant>>` 内存 only；默认 window=300s threshold=10 const；`record` 懒清窗口外 + 返窗内 count；`check` `>= threshold` → `RunawayError::Detected`；`with_clock(...)` 注入伪 clock 测试；**进程内单实例**，跨进程 runaway 跟踪推后到真有需求时评估（roadmap §7 观察项） |
| `HookEvent` | Phase 4 落地的 `crates/roostery/src/hook_event.rs`（feature `2026-05-18-dispatcher-rules`）。dispatcher 入口数据形状（roadmap §4.4）：`#[non_exhaustive]` 6 字段 `schema_version: u32` const 1 / `hook_source: String` (e.g. `"claude-code-stop"` / `"codex-stop"` / `"gemini-stop"`) / `session_id: String` / `workspace: PathBuf` / `trigger_meta: serde_json::Value` opaque runtime payload / `trace: Option<TraceContext>`（外部 hook 必为 None；dispatcher fire 时分配新 trace_id）。`trigger_meta_path(&str) -> Option<&Value>` 提供点路径取值（非 object 中断返 None）|
| `HOOK_EVENT_SCHEMA_VERSION` | `hook_event.rs:18` 公开 const `= 1`。公开承诺：bump 需 `cs-roadmap update` + 旧版兼容反序列化（同 `JournalEntry.schema_version` 模型）|
| `RulesConfig` / `CompiledRule` / `Match` | `crates/roostery/src/rules.rs`（feature `2026-05-18-dispatcher-rules`）。YAML schema v1 + typestate-lite 两态分离：`RawRule`（反序列化态，仅字段值）→ `CompiledRule`（含 `globset::GlobMatcher` 实例态）。`Match<'a>` 零拷贝借用 rules / event 字段（`rule_name: &'a RuleName / runner: &'a str / args: &'a Value`）。Match 维度 3 项 AND（user 拍板 MVP）：`hook_source` 字符串相等 / `workspace_glob` fnmatch（`globset` 一次编译）/ `trigger_meta_eq` 点路径取值字面量相等。Action 是 opaque `{ runner: String, args: Value }`——rules 不解释 args 透传给 Runner impl（dispatcher-runners feature 落地后才有真正 Runner trait 消费）|
| `RULES_SCHEMA_VERSION` | `rules.rs:31` 公开 const `= 1`。公开承诺：bump 需 `cs-roadmap update` + 旧版兼容反序列化 |
| `SELF_EVENT_PREFIXES` | `rules.rs:33` 内部 const `&["dispatcher.", "roostery."]`——`matches` 第一步是 self-event 短路（防 dispatcher 自激）。`hook_source` 前缀任一命中即返 `None`，**剩余规则不评估** |
| `RuleName` | `rules.rs:37` `(String)` `#[serde(transparent)]` newtype（与 `business-identifier-newtype` decision 一致）。`Ord` derive 用于 `BTreeSet` 重名 grep。构造器 `RuleName::new(impl Into<String>)`（不是 `from_str`——避免与 `std::str::FromStr` 撞名 clippy 警告）|
| `Runner` trait | Phase 4 落地的 `crates/roostery/src/runners.rs`（feature `2026-05-18-dispatcher-runners`，roadmap §4.3）。`#[async_trait] pub trait Runner: Send + Sync { fn kind(&self) -> &'static str; async fn run(event: &HookEvent, ctx: &TraceContext, args: &Value) -> Result<RunOutcome, RunnerError>; }`。**与 §4.3 偏离**（user 拍板）：`run` 不收 `&BudgetGate` 参数（budget gate 留给 dispatcher-loop 编排）；`RunOutcome` 加 `cost_usd: Option<f64>` 字段让 caller 走 `budget.consume`。每个 runtime adapter（noop / cc_headless / 未来 codex_exec / gemini_headless）实装一个 `impl Runner` 挂入 `RunnerRegistry`，对 dispatcher-loop 编排零耦合（兑现 `runtime-neutral` req）|
| `RunOutcome` / `RunnerStatus` / `RunnerError` | `runners.rs`。`RunOutcome` 5 字段（`status: RunnerStatus / stdout: String / stderr: String / emitted_events: Vec<HookEvent> / cost_usd: Option<f64>`）。`RunnerStatus` 三态 `Success / Failed { reason } / Skipped { reason }`（`#[serde(tag = "kind", rename_all = "snake_case")]`）。`RunnerError` `#[non_exhaustive]` 4 变体（`BinaryNotFound / SpawnFailed / Timeout / OutputParseFailed`）—— **基础设施失败**（spawn / timeout / 解析）；与 `RunOutcome.status.Failed`（**runner 业务失败**，跑完了但 exit code 非 0）语义分层 |
| `RunnerRegistry` | `runners.rs`。`Vec<Box<dyn Runner>>` linear-scan registry（n=2-4，O(n) 可忽略）。公开 API：`new() / with_runner(Box<dyn Runner>) -> Self / with_defaults() -> Self / find(&str) -> Option<&dyn Runner> / len / is_empty`。`with_defaults` 自动注册 `NoopRunner` + `CcHeadlessRunner`；同 kind 二次注册 linear find 返第一（用户责任不报错）|
| `NoopRunner` / `CcHeadlessRunner` | `runners.rs`。Phase 4 dispatcher-runners 首发两实装。`NoopRunner::kind() == "noop"`，`run` 返 `RunOutcome { status: Success, stdout/stderr/emitted_events 空, cost_usd: None }`。`CcHeadlessRunner::kind() == "cc_headless"`，调 `claude -p <prompt> --output-format json [--model <m>] [--resume <id>]`；`bin_override: Option<PathBuf>` 测试可注入；走 `tokio::task::spawn_blocking` 包同步 `std::process::Command`（不引 `tokio::process` 避 ETXTBSY race）；stdout JSON parse 解 `cost_usd / result / text`，**解析失败仍返 Success cost None**；timeout 走 `args.timeout_ms` 覆盖 `DEFAULT_TIMEOUT_MS`。`emitted_events` 本期始终空 Vec（chain dispatch 推给 dispatcher-loop）|
| `DispatchOutcome` / `DispatchStep` / `StepStatus` | Phase 4 落地的 `crates/roostery/src/dispatcher.rs`（feature `2026-05-18-dispatcher-loop`）。`DispatchOutcome` 3 字段（`trace_id: TraceId` / `root_event_id: String` / `dispatched: Vec<DispatchStep>`）= 单次 `fire` 编排总览；`DispatchStep` 7 字段（`event_id` / `hook_source` / `depth` / `matched_rule: Option<String>` / `runner_kind: Option<String>` / `status: StepStatus` / `fanout: usize`）= 链式分发中每个 event 一条 step。`StepStatus` 5 态（`Success` / `Skipped { reason }` / `GateRejected { reason }` / `Failed { reason }` / `NoMatch`）覆盖 fire 主链路全部分支可观察结果；`reason` 字符串承载 gate / runner 原始错误描述给 journal 落档 |
| `DispatchError` | `dispatcher.rs`。dispatcher 编排层错误（与 `RunnerError` / `RulesError` / `BudgetError` 分层不混）。`#[non_exhaustive]` 6 变体（`ConfigLoadFailed` / `RulesLoadFailed` / `JournalDirNotFound` / `ReplayNotFound` / `EventReconstructFailed` / `BadCliInput`）。**`fire` 内部所有 gate / runner 失败不冒泡**——全部走 `journal.append` 落档 + `StepStatus` 反映；`replay` / `test_rule` 因为用户主动调用对错误敏感，DispatchError 直接返给 caller 让 main.rs exit 1。`fire` 主入口加载阶段（config / rules load 失败）目前在 `main.rs::run_fire` 内做 fallback 写 eprintln，未来可改走 DispatchError 路径 |
| `DEFAULT_MAX_FANOUT` | `dispatcher.rs:28` 公开 `usize = 16` const。`fire` 链式分发的 single-step width 上限（`trace.max_depth` 守深度，本 const 守 width）——单个 runner 返 `emitted_events` 超 16 条时截断 + journal 标 `fanout_truncated`。防 runner bug / 链式风暴把队列撑爆 |
| `TaskRef` / `TaskGuid` / `TaskWriterError` | Phase 5 落地的 `crates/roostery/src/bot_task_writer.rs`（feature `2026-05-18-bot-task-writer`）。`TaskRef { guid: TaskGuid, url: String }` = 飞书 task 引用（guid 用 newtype 隔离防与 url / event_id / trace_id 等其他 id-like 串混；与 `business-identifier-newtype` decision 一致）。`TaskGuid(String) #[serde(transparent)]`。`TaskWriterError` `#[non_exhaustive]` 5 变体（LarkCallFailed / ResponseShapeUnexpected / CacheLoadFailed / CacheSaveFailed / IdentityResolveFailed）——与 `LarkError` / `IdentityError` 分层不混 |
| `CreateTaskOptions` / `AppendStepsOptions` | `bot_task_writer.rs`。可选参数集合 `#[non_exhaustive]` + `Default` + lifetime borrow + `new() / with_*` builder API（attention.md E0639 规约要求 builder，不允许 struct literal）。`CreateTaskOptions` 5 字段（`description / assignee_open_id / idempotency_key / host / profile`）；`AppendStepsOptions` 2 字段（`idempotency_key / profile`）|
| `SESSION_CACHE_SCHEMA_VERSION` | `bot_task_writer.rs:22` 公开 const `= 1`。`~/.roostery/state/session_tasks/{safe}.json` schema 字段名 / 类型 / 序列化形态变更需 bump version + `cs-roadmap update` 评估 + 旧版兼容反序列化。schema_version 缺失走 serde default（0）= 兼容旧版 cache 读 |
| `DEFAULT_HOST_FALLBACK` | `bot_task_writer.rs:23` 公开 const `= "unknown"`。host suffix 三 fallback 链终态（`ROOSTERY_HOST` env > hostname 首段 > 本兜底）|
| `PushRequest` / `PushOutcome` / `PushStatus` / `PushOptions` | Phase 5 落地的 `crates/roostery/src/bot_stop_hook.rs`（feature `2026-05-18-bot-stop-hook`）。`PushRequest` builder API（`new(agent, session, cwd) + with_summary / with_description / with_assignee`）是双 CLI surface（`bot stop-hook` + `bot push`）共享的类型化边界。`PushOutcome` `#[derive(Serialize)]` 是 `--json` 模式下 caller 可 jq 消费的**稳定契约**（v1 字段不破坏性变更；新字段走 backwards-compatible append）。`PushStatus` 4 变体 `#[serde(rename_all = "snake_case")]`：`Success` / `FallbackUsed` / `Failed` / `Skipped`。`PushOptions { strict, json_output, no_im_fallback }` 3 bool，Default = hook 路径推荐（不 strict / 不 json / 走 IM 兜底）|
| `StopHookInput` | `bot_stop_hook.rs` `#[serde(default)]` 全字段 Option 的 CC/Codex/Gemini SessionEnd stdin JSON schema。空 stdin / 缺字段 / 非法 JSON 都不报错（fallback 到 default + 走 Skipped）|
| `DEFAULT_SUMMARY` / `SUMMARY_MAX_BYTES` | `bot_stop_hook.rs:21-24` 公开 const `"Agent stopped (no summary)"` / `200`。append_steps 文本默认值 + UTF-8 边界安全截断（`floor_char_boundary` polyfill）|
| `paths::TEST_ENV_LOCK` | `crates/roostery/src/paths.rs:67` `pub static Mutex<()>`。**跨模块共享**的测试 env 串行化锁。所有 `#[test]` / `#[tokio::test]` 改 `ROOSTERY_*` / `HOSTNAME` / `FEISHU_*` 等进程级 env 必须先 lock 这把。修订原因：之前每 mod 在 `mod tests` 各自声明 ENV_LOCK，多 mod 并行跑触碰同 env var 时 race（一 mod lock 不能阻挡另一 mod set_var），任 test 因 race 失败 panic 还 poison 该 mod lock 连锁拖挂。**Corollary**：`fn` 内消费 `paths::roostery_home()` / `paths::journal_dir()` 等 env-dependent helper 的测试（如 config roundtrip）也要锁——`Config::default()` 里 `journal.dir = paths::journal_dir()` 会读 env 当前值。落定于 bot-stop-hook feature S10.5 |
| `SAFE_ENV_FORWARD` | `runners.rs:36` 公开 `&[&'static str]` const allowlist。子进程 env 经此过滤——父 hook 状态（如 `ROOSTERY_AGENT`）**不串到子 agent**避 trace 链断裂。覆盖：POSIX baseline（USER/LOGNAME/SHELL/TMPDIR）+ XDG_* + 代理（HTTP_PROXY etc.）+ TLS CA + API keys（ANTHROPIC/OPENAI/GEMINI/GOOGLE）+ Custom base URLs + 各 vendor config dirs。私有 helper `prep_env(ctx, kind)` 合并 allowlist + POSIX 兜底（PATH/HOME/LANG/TERM）+ trace 三 env（`to_env_pairs()`）|
| `DEFAULT_TIMEOUT_MS` / `STDOUT_HEAD_CAP` | `runners.rs:30-31` 公开 const，分别 `600_000` (10 min) / `4096` (4 KiB)。CcHeadless 默认 timeout + stdout/stderr 截断阈值 |
| `BotRole` / `BotsConfig` / `BOTS_SCHEMA_VERSION` | Phase 5 落地的 `crates/roostery/src/bot_bridge/role.rs`（feature `2026-05-19-bot-bridge-cluster`）。`BotRole` `#[non_exhaustive]` 9 字段（app_id 双关 lark-cli profile / role 显示名 / mention_alias `@<alias>` 匹配键 / runner `Runner::kind()` 值 / default_cwd / prompt_template + reply_template / chat_whitelist 空=不限 / next_bot_mention 接力链下一棒）。`BotsConfig` `#[non_exhaustive] { schema_version, bots }`，schema_version 缺失走 `serde(default)` = 1（向后兼容）。`BotRoleError` `#[non_exhaustive]` 4 变体（LoadFailed / ParseFailed / SchemaVersionMismatch / MissingField{ index, field }）。`pub fn load_bots(&Path) -> Result<BotsConfig, BotRoleError>` 校验链：read → parse → schema_version 校验 → 必填字段存在性校验（先于 serde 报更友好错）→ 反序列化 |
| `roostery bot bridge` 子命令 | `crates/roostery/src/bot_stop_hook/cli.rs:27 BotSub::Bridge(BridgeCliArgs)`（feature `2026-05-19-bot-bridge-cluster`）。第 3 个 `bot` 子命令（与 `stop-hook` / `push` 并列）；clap 5 flags：`--bots <PATH>` (default `~/.roostery/bots.yaml`) / `--profile <ID>`（可重复）/ `--max-concurrency <N>` (default 8) / `--max-events <N>` (0 = unlimited) / `--timeout <SEC>`。语义对称：push / stop-hook 是 single-shot，bridge 是 long-running daemon |
| `HitlDecision` / `HitlSignal` / `ABORT_KEYWORDS` / `ADJUST_PREFIXES` | `crates/roostery/src/bot_bridge/{hitl,active_registry}.rs`（feature `2026-05-19-bot-bridge-cluster`）。`HitlDecision` `#[non_exhaustive]` 三态 `Abort{reason} / Adjust{body} / Pass`——`hitl::classify(&str) -> HitlDecision` 把 IM 消息正文判定为三态之一。`HitlSignal` `#[non_exhaustive]` 二态 `Abort{reason} / Adjust{body}`——只有需要 runner 立即响应的状态发到 oneshot channel，Pass 不发信号。`ABORT_KEYWORDS: &[&str]` = `&["/stop", "/abort", "停", "中止"]` 整段精确匹配；`ADJUST_PREFIXES: &[&str]` = `&["/adjust ", "/adjust\n", "调整 ", "调整\n"]` 前缀匹配，body 空 → Pass 退化。**const 写死不开放配置**（与 Python 一致） |
| `ActiveRunnerRegistry` / `RunnerHandle` | `crates/roostery/src/bot_bridge/active_registry.rs`（feature `2026-05-19-bot-bridge-cluster`）。**进程内活跃 runner 表**——`Mutex<BTreeMap<TaskGuid, RunnerHandle>>` 记录"当前哪条 task 由哪个子进程跑"。`RunnerHandle { kill_tx: tokio::sync::oneshot::Sender<HitlSignal>, task_guid, task_url, chat_id, started_at }`。**与 `dispatcher::runners::RunnerRegistry` 同名不同概念**——后者是"哪些 runner kind 可用"（trait 注册表，全局静态），前者是"哪些活跃 task 在跑"（实例表，daemon 重启清零）。命名前缀 `Active` 避让；长期重构待 cs-refactor 把 dispatcher 那个改 `RunnerKindRegistry`（design D2）。`send_signal(guid, sig)` 实装是 remove-on-send pattern（oneshot::Sender 消费 self）|
| `BOT_CHAT_CACHE_SCHEMA_VERSION` / `EndOutcome` / `RelayTaskError` | `crates/roostery/src/bot_bridge/relay_task.rs`（feature `2026-05-19-bot-bridge-cluster`）。const `= 1` 公开承诺；`~/.roostery/state/bot_chats/{app_id}/{safe_chat}.json` schema 字段变更需 bump + 兼容旧版反序列化。`EndOutcome` `#[non_exhaustive]` 四态 `Success{adjust_attempts} / Failed{exit_code} / Aborted{reason} / Timeout`，对应 step 文案 ✅ / ❌ / ⚠️ / ⏱️ emoji 前缀。`RelayTaskError` `#[non_exhaustive]` 3 变体（TaskWriter / CacheLoad / CacheSave）。3 pub async fn `record_start / record_adjust / record_end`，按 chat_id 索引 cache：cache hit 复用 TaskRef + append step，cache miss 调 `bot_task_writer::create_task` + 写 cache。`record_adjust` 入参含 `chat_id` 让 cache.adjust_count 真实递增（design 实装阶段 commit `e5d9366` 修） |
| `BridgeOptions` / `BridgeReport` / `ShutdownReason` / `CancelToken` | `crates/roostery/src/bot_bridge/daemon.rs`（feature `2026-05-19-bot-bridge-cluster`）。`BridgeOptions` 12 字段（max_concurrency / max_events / timeout / profile_filter / event_channel_buffer / shutdown_deadline / lark_binary / journal_dir / runner_registry / lark_runner / cancel 注入点）——daemon 启动 + 测试可观察性扩展；多数注入点可走 `None = default` 让生产路径走默认实装。`BridgeReport` 11 字段聚合（events_received / events_skipped_unmatched_chat / events_skipped_no_match / hitl_abort_signaled / hitl_adjust_signaled / hitl_signal_misses / handle_event_spawned / handle_event_results map / event_source_errors / bots_subscribed / shutdown_reason）。`ShutdownReason` `#[non_exhaustive]` 5 态（CtrlC / MaxEvents / MaxDuration / EventSourceClosed / NoBots）。`CancelToken { flag: AtomicBool, notify: Notify }` 进程内取消令牌——daemon 接受外部注入实现 ctrl_c 之外的程序化 cancel（仓库无 tokio-util，手撸 Arc<AtomicBool>+Notify 即足）|
| `bot_bridge` 子目录模块图 | `crates/roostery/src/bot_bridge/`（feature `2026-05-19-bot-bridge-cluster`）9 个子模块：`role` / `hitl` / `active_registry` / `relay_task` / `event` / `runner` / `daemon` / `cli` / `mod.rs`。**编排链**：daemon spawn per-bot consume_im → 中央 mpsc → main loop classify HITL → 命中 lookup+send_signal / 未命中 event_matches_bot → spawn handle_event → record_start → register active + select! { runner_future, kill_signal } → Adjust 重启循环 / Abort / 自然终态 → unregister + record_end → reply（通过 LarkRunner）。**journal source 命名空间** `bot_bridge:daemon` / `bot_bridge:handle_event`——daemon 主循环编排副作用 vs handle_event 协程内副作用分两个 source；下游 query 可按 source 过滤拿到 daemon 流水 |

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
纯数据操作。`schema` 常量、`redact`（敏感字段脱敏）、`remoterefs`（JSON walk + match-dispatch 抽 9 个 newtype token）。

**redact 模块**（已落地，commit `1e392e5`，Phase 1）：

- 公开 API：`scrub_value(&Value) -> (Value, Vec<String>)`、`scrub_argv(&[String]) -> (Vec<String>, Vec<String>)`、`scrub_text(&str) -> String`，全部纯函数返回 owned 新值不修改入参
- 公开常量：`MASK = "***"`、`SENSITIVE_KEYS: &[&str]` 11 entries（见术语表）
- audit path 格式：argv 用 `argv[N]`，结构化用 RFC 6901 JSON Pointer
- **下游使用约束**：`journal-core`（Phase 1）/ `lark-cli-shim`（Phase 2）/ `bot-task-writer`（Phase 5）等所有写 journal 的模块**必经此模块脱敏**（见 roadmap §4.2 JournalEntry schema 契约）
- 定位：logging-boundary scrubber，**不替代** in-memory secret wrapper（`Secret<T>` 类）；未来 Module D Config 持有 secret 字段时单独引 `redact` crate

**remoterefs 模块**（已落地，commit `4714683`，Phase 1）：

- 公开类型：9 个 `#[serde(transparent)]` newtype token —— `MessageId` / `DocToken` / `FolderToken` / `RecordId` / `ChatId` / `AppToken` / `WikiToken` / `TaskId` / `ThreadId`（每个都额外 impl `AsRef<str>` + `Display`，caller 拼 URL/log 不用 `.0`）；JSON 形态是裸字符串（与 Python 版兼容），Rust 类型互不兼容
- 公开容器：`RemoteRefs` struct + `#[non_exhaustive]` + 每字段 `Option<Token> + skip_serializing_if`——加字段不破坏外部 caller；全 None serialize 为 `{}`
- 公开 API：`extract(argv: &[String], stdout: &str) -> RemoteRefs`——单一入口，best-effort，**永不 panic / 永不返 Result**；所有失败路径返 `RemoteRefs::default()`
- 实现策略：单趟 match-walk 直接 in-place 填字段（不引中间 HashMap 聚合）；同 key 多匹配靠 `is_none()` guard 显式首匹配赢；walk 深度上限 64（防御深嵌套栈溢出）
- Sibling-key 顺序契约：由 serde_json `Map` 默认 BTreeMap 字典序决定，**不承诺 stdout 物理顺序**——依赖物理顺序的下游 caller 应自己 parse 不依赖 RemoteRefs
- **类型隔离编译期保证**：下游函数签名 `fn send(msg: &MessageId)` 物理上无法接 `&DocToken` 等其他 8 种 token；cross-wiring bug 编译期拦截（compile_fail,E0308 doctest 守护）
- **下游使用约定**：Phase 2 `lark-cli-shim` / `LarkCli` wrapper 等写 journal 前自己调 `remoterefs::extract` 把结果塞 `entry.params.remote_refs` 子字段；`Journal::append` 不内建集成（关注点分离，与 redact 同口径）

- 子 feature：`rust-scaffold` / **`core-redact`（done）** / **`core-remoterefs`（done）**

### Module B · 本地审计 / Journal（Phase 1）
本地 jsonl audit / replay。`JournalEntry` schema 是 `portable-by-default` req 的契约载体（公开、稳定、可移植）。

**journal 模块**（已落地，commit `b9ac5be`，Phase 1）：

- 公开类型：`JournalEntry`（11 字段，roadmap §4.2 钦定）+ `JournalResult`（`#[serde(tag="outcome")]` 的 `Ok{value}` / `Err{kind,message}` 两态）
- 公开 API：`Journal::open(dir)` / `Journal::default()` / `journal.append(&entry) -> io::Result<PathBuf>`；`JournalEntry::new(source, action)` 关联函数填默认；`new_event_id()` 生成 ULID（Crockford base32，26 字符）
- 写入语义：同步、`OpenOptions::append+create` + 单 `write_all`，POSIX <PIPE_BUF 原子；文件名按 `entry.ts`（UTC 日）= `YYYY-MM-DD.jsonl`，跨午夜 backfill 落到正确日；mkdir -p 自动建目录
- **schema_version=1 公开承诺**：本模块落地起，字段名 / 类型 / 序列化形态变更需 bump + 兼容旧版 deserialize + `cs-roadmap update` 评估 portable-by-default 影响
- **下游使用约束**：`params` 字段在 caller 侧用 `redact::scrub_value` 脱敏后填入；`Journal::append` 不内建脱敏（关注点分离）
- **remoterefs 集成约定**：同理，caller 自己调 `remoterefs::extract(argv, stdout)` 把结果塞 `entry.params.remote_refs` 子字段；`Journal::append` 不感知 RemoteRefs 类型——9 个 newtype token（MessageId / DocToken / 等）的 JSON 形态因 `#[serde(transparent)]` 保持裸字符串与 Python 版兼容
- 路径解析单独成 `paths` 模块（`roostery_home()` / `journal_dir()`）；env 覆盖走 `ROOSTERY_HOME`，**不读** legacy `FEISHU_HUB_HOME`
- 不在范围（Phase 1）：read / replay API、size/never rotation 策略、跨进程 flock、Python jsonl 迁移工具——后续 phase 起独立 feature

- 子 feature：**`journal-core`（done）**

### Module C · 飞书 Syscall（Phase 2）
飞书通信的唯一 sanctioned 通道。`LarkRunner` trait + 默认 subprocess 实现 + `roostery smoke` + `bin/shim` 二进制。

**lark_cli 模块**（已落地，commit `cc44dfa`，Phase 2）：

- 子目录 `crates/roostery/src/lark_cli/`（首个走 compound convention 档 2 子目录组织的模块）：`mod.rs` + `runner.rs` + `error.rs` + `subprocess.rs` + `mock.rs` + `journaled.rs`
- 公开 trait：`LarkRunner: Send + Sync`（async；`run(args)` 默认 method 委托 `run_with_options(args, opts)`）；roadmap §4.1 兑现层
- 公开类型：`RunOptions`（`#[non_exhaustive]` + **builder API** `new/with_timeout/with_stdin/with_profile`）；`LarkError`（`#[non_exhaustive]` rich enum + thiserror，4 变体 `Spawn { path, program_args, source: io::Error }` / `NonZeroExit { exit_code, body_code, message, stdout, stderr }` / `OutputParse { source: serde_json::Error, stdout }` / `Timeout { timeout_ms }`，`MAX_FIELD_LEN_IN_ERR = 4 KiB` 字段截断，`retriable()` method 由 `matches!` 实现）
- 三个 LarkRunner 实现：
  - `LarkCli`（默认 subprocess；`ROOSTERY_LARK_CLI_BIN` env > 默认 `"lark-cli"` 走 PATH；30s 默认 timeout；`kill_on_drop(true)`）
  - `MockLarkRunner`（test utility；FIFO `VecDeque` 队列 + 调用记录；`enqueue_*(&Self)` 链式；空队列 panic；Drop 未消费 `tracing::warn!`）
  - `Journaled<R: LarkRunner>`（装饰器；写 journal 前用 `redact::scrub_argv` 脱敏 argv；写失败 `tracing::warn!` 不破坏原 result）
- **下游约定**：模块 D/E/F/G/H 依赖飞书操作必须 take `Arc<dyn LarkRunner>` / `impl LarkRunner` 注入，禁止直接 `Command::new("lark-cli")`（双向引用 §6 第 1 条架构红线）
- **不在范围**（Phase 2）：业务包裹函数（`im_send_text` 等归 Phase 5+）；retry（归 Phase 4 dispatcher）；jq 选择器；Config 驱动构造

**shim 二进制**（已落地，feature `2026-05-17-lark-cli-shim`，Phase 2）：

- 新增 bin target：`Cargo.toml [[bin]] name = "shim" path = "src/bin/shim.rs"`；同 crate 复用 `journal` / `redact` / `remoterefs` 模块
- 装机点：`~/.local/bin/lark-cli`（PATH-prefix shim 拦截 agent runtime 直接调 `lark-cli` 的所有调用；装机由 Phase 3 `roostery init` 完成）
- 核心行为：`resolve_real_cli`（env `ROOSTERY_REAL_LARK_CLI` + canonicalize anti-recursion）→ `is_interactive` 三段式（TTY / verb `["auth"]` / flag `--interactive`/`-i`/`--repl`） → 命中走 `std::os::unix::process::CommandExt::exec()` 直通；未命中走 `run_non_interactive`（`std::process::Command` + 2 个 `std::thread::spawn` pump 流式 tee + head buffer 64 KiB stdout / 16 KiB stderr） → `build_entry` 11 字段映射（source="shim" / action="lark-cli:{verb}" / params 含 argv+cwd+stdin_present+stdout_head+stderr_head+remote_refs / result Ok refs \| Err NonZeroExit / duration_ms） → `journal.append`
- 与 `Journaled<LarkCli>` 装饰器的区别：两者都写 journal，但 I/O 模型不同——shim 是 streaming bytes（透明 tee 4 KiB chunks），`Journaled<LarkCli>` 是 buffered Value（一次性 `wait_with_output` + JSON parse）；caller 路径也不同——shim 截获 agent runtime 直接调用，`Journaled<LarkCli>` 由 dispatcher 内部调度。两条路径独立写 journal，下游 read/replay 通过 `source` 字段区分（"shim" vs "dispatcher"）
- 设计约束：不引 tokio（启动开销 / 二进制大小敏感）；不调 LarkRunner trait（I/O 语义不同）；不读 Config（Phase 3 起来后由 init 写 env 注入）；不实现 retry / 不 parse stdout JSON / 不修改 lark_cli wrapper 模块
- 不变量：透明性（用户 stdout/stderr 字节 = real lark-cli 输出）；exit code 透传（setup 失败固定 127）；anti-recursion 强制；journal 写失败 `tracing::warn!` 不影响 exit code；pump 写用户 stream 失败 silent（broken pipe tolerant）；head buffer 超 cap 后继续 tee 但停扩
- NOJOURNAL=1 路径：仍跑 real lark-cli + tee，但写一条 `action="lark-cli:{verb}:skipped"` + `params.reason="nojournal"` 的标记 entry（"知道发生了但故意没记完整"）

**smoke 模块**（已落地，feature `2026-05-17-roostery-smoke`，Phase 2）：

- 新文件 `crates/roostery/src/smoke.rs`（档 1 单文件，产品 ~349 行 + 内联单测 ~300 行）；同 crate 复用 `paths` 模块（新增 `state_dir` / `smoke_state_path`）
- 引入 `clap = "4"` derive 作为项目首个 CLI 解析器；主 bin `roostery` 重写为 clap subcommand 模式（保留 `--version` 输出严格 `roostery 0.0.0 (rust)`）；`Command::Smoke` 是首个子命令
- `PROBE_MATRIX` 6 条命令：im_messages_send / docs_create_v2 / docs_update_overwrite / drive_files_list / drive_create_folder / drive_move（与 Python 版 `legacy/python/src/roostery/smoke.py::PROBES` 1:1 复刻，本机 2026-05-17 实测 lark-cli 1.0.29 全过）
- `probe_one` 实现：spawn + `try_wait` 50ms 轮询 + 10s timeout（超时 `kill` + `wait`）；分类 "Dry Run" marker + rc==0 → ok / "unknown flag" / "unknown command" → mismatch / 其他 → unexpected
- 状态文件 `SmokeReport`（`#[non_exhaustive]` 6 字段：schema_version / binary / lark_cli_version / started_at / all_ok / probes `BTreeMap<String, ProbeResult>` 保证字典序 diff 友好）；`schema_version=1` 公开承诺；`save_report` 写 `.tmp` + `rename` atomic
- 公开 API：`run() -> SmokeReport` 跑完整矩阵 + 持久化；`ensure_ready() -> Result<(), SmokeError>` 给后续 `roostery init` / `daily_report` 当升级 gate（`SmokeError` `#[non_exhaustive]` 4 变体：NeverRun / LastFailed / StateLoadFailed / BinaryNotFound）
- 设计约束：**不调 LarkRunner trait**（raw bytes vs buffered Value 同 shim 决定）；**不引 tokio**（同步顺序 6 probe，不需要 runtime）；**不写 journal**（state 快照不是事件流）；**不读 Config**（Phase 3 起来再扩）；binary 解析仅 `ROOSTERY_LARK_CLI_BIN` env > default `"lark-cli"`（与 `lark_cli/subprocess.rs::ENV_BIN` 同字符串）
- 不变量：smoke run 是 idempotent（覆盖 state）；atomic write；binary 未找到不 panic（6 条全标 ok=false 仍写 state file）；`ensure_ready` 区分 NeverRun / LastFailed / StateLoadFailed 三个 specific 错误变体

- 子 feature：**`lark-cli-wrapper`（done）** / **`roostery-smoke`（done）** / **`lark-cli-shim`（done）**——Module C 完成

### Module D · 本地配置与安装（Phase 3）
bootstrap `~/.roostery/`（自 journal-core 起；env 覆盖走 `ROOSTERY_HOME`）、merge Stop hooks 进 `~/.claude/settings.json` / `~/.codex/hooks.json`、装 shim、识别 agent runtime、嵌入模板。

**config 模块**（已落地，feature `2026-05-17-config-yaml`，Phase 3）：

- 新文件 `crates/roostery/src/config.rs`（档 1 单文件，产品 ~200 行 + 内联测试 ~218 行）；同 crate 复用 `paths` 模块（新增 `config_path`）
- 引入 `serde_yml = "0.0.12"` YAML 库（`serde_yaml` maintained fork，2024 起原作者弃坑）；**仅 config 模块直接 import**
- 顶层 schema：`Config` `#[non_exhaustive]` 6 字段（`schema_version` / `identity` / `runners` / `budgets` / `trace` / `journal`），全 `#[serde(default)]` per-field 满足 roadmap §4.6 "顶层字段缺失用编译期默认值"
- `runners: BTreeMap<String, serde_yml::Value>` 开放结构——Phase 4 dispatcher-runners 落地时各 Runner impl 自己 deserialize 子节，新加 runner kind 不动顶层 schema
- 公开 4 fn：`load()` / `load_from(&Path)` / `save(&cfg)` / `save_to(&cfg, &Path)`；缺失文件 → `Ok(Config::default())`（first-run 装机友好）；atomic write `.tmp` + `fs::rename`
- `ConfigError` `#[non_exhaustive]` 4 变体（LoadFailed / ParseFailed / SaveFailed / SchemaVersionMismatch）
- schema_version 处理：缺失字段隐式 = 1（`#[serde(default = "default_schema_version")]`）；==1 OK；!=1 报 `SchemaVersionMismatch { found, expected }` 让 caller 决定 migration
- `SCHEMA_VERSION_CURRENT: u32 = 1` 模块私有 const，与 `lib.rs::SCHEMA_VERSION`（管 JournalEntry schema）独立 bump
- 设计约束：**不读 env override**（各模块自管，如 `lark_cli/subprocess.rs::ENV_BIN`、`smoke::ENV_BIN` 自管 `ROOSTERY_LARK_CLI_BIN`；config 仅管文件层）；**不实现 schema migration**（v2 落地时走 cs-roadmap update）；**不消费 runners 子结构**（占位给 Phase 4）；**不修改 main.rs**（纯 lib 扩展，无 CLI 子命令变更）
- 不变量：`load` 文件不存在 → default；atomic save；schema_version 缺失隐式=1；config 不调 redact（不含敏感数据）；`Config::default()` 可 save+load round-trip 等价

- 子 feature：**`config-yaml`（done）** / **`hooks-merge`（done）** / **`roostery-init`（done）**

**identity / agent_detect / onboarding 模块**（已落地，feature `2026-05-18-roostery-init` + UX 修复 `2026-05-18-init-real-lark-cli-override`，Phase 3 收尾）：

> **2026-05-18 init UX 修复**（feature `2026-05-18-init-real-lark-cli-override`，commit `aa06807`，CI run `26036982700` 全绿）：根治 onboarding init 在 npm 全局 prefix == shim target 时 `RealLarkCliMissing` fail，且 live 模式失败留 install_shim 破损态的硬 bug。三件套：(1) `--real-lark-cli <PATH>` flag + 复用 `ROOSTERY_LARK_CLI_BIN` env 作 init-time override（与 runtime LarkCli subprocess 同源 env，**不**引入新 env 名）；(2) `resolve_real_lark_cli` 调用上移到 F1 smoke gate 后第一时间——失败零文件副作用退出（修原 L205 fail-late 漏洞）；(3) `OnboardingError::RealLarkCliMissing` 拆 3 sub-variant（`LarkCliNotInPath` / `LarkCliCollidesShimTarget` / `OverrideInvalid`），每条 Display 含 fix hint（`--real-lark-cli` flag + env 提示）。**真机 dogfood 完整跑通**（mv npm symlink + `roostery init --real-lark-cli <real-path>` → shim 装 + `~/.roostery/env` + ~/.zshrc patch + ~/.claude/settings.json hook 合并 → shim transparent forward + journal 自动落档 → CC SessionEnd 被动路径模拟真飞书出 task `d4e8c06f-...`）。closes issue `2026-05-18-init-shim-conflicts-npm-prefix`。


- 三新文件 `crates/roostery/src/{identity,agent_detect,onboarding}.rs`；同模块 `paths.rs` 加 `scripts_dir()` + `env_file()` 两公开 fn；`main.rs` 加 `Command::Init(InitArgs)` 子命令；`hooks_merge.rs` 加 `AgentKind::Gemini` 变体 + `GEMINI_STOP_HOOK_JSON` const + `templates/gemini_stop_hook.json` 第 3 模板
- 新增依赖 `which = "7"`（PATH walk 探 agent CLI）/ `gethostname = "0.5"`（identity host 字段）/ `sha2 = "0.10"`（shim install 幂等 hash 比对）；无 `reqwest` / HTTP client / 外部 LLM client（架构红线守住）
- 公开 API：
  - `identity::current(runner) -> Result<Identity, IdentityError>` async；`Identity::{short_user, short_bot, is_ready, describe, 7 accessor}`
  - `agent_detect::{AGENTS, detect_all(skip), AgentSpec::expanded_hooks_target, DetectResult::installed}`
  - `onboarding::{run(runner, opts), format_report, InitOptions, InitReport, ShellKind, OnboardingError 9 变体, SkipReason 3 变体}`
- 装机流水线 9 阶段（编排见术语表 `roostery init 子命令` 词条）；线性 + 单 agent 失败不阻塞（`SkipReason::MergeFailed(reason)` 汇总）+ identity 失败不阻塞（`(Option<Identity>, Option<IdentityError>)` 主流程二元组）
- 不变量：smoke 失败 → 文件系统零改动；shim 装机幂等（sha2 hash 比对，相同跳，不同覆盖，非 shim 报错）；shell rc patch 幂等（marker block 检测）；dry-run 模式零副作用；sh bridge chmod 0755；`patch_shell_rc` marker block conda/pyenv 风格（用户可自己 unpatch）
- 设计约束：**不创建 welcome task / 不调 task_writer**（推 Phase 5 `bot-stop-hook` + `bot-task-writer`）；**不实现 `--force`**（认为"我知道在干什么"语义本期不暴露）；**不读 `FEISHU_HUB_*` legacy env**；**不写 config.yaml**（仅读，找不到用编译期默认）；**不支持 fish / nushell**（`UnsupportedShell` 错误明示）；**不实现 uninstall**（marker block 让用户能手动 unpatch）；**onboarding 模块名沿用 Python 但职责改为纯 installer**（Phase 5 才扩 welcome task；模块顶部 doc-comment 显式说明范围演化避免 git blame 跨期混淆）
- 测试覆盖：lib 200（含本 feature +26）+ onboarding integ 5（dry-run / smoke NeverRun / smoke LastFailed / full install + idempotent / identity error 不阻塞）+ hooks_merge integ 12（含 Gemini 模板 byte-for-byte）；ENV_LOCK 串行化 HOME/ROOSTERY_HOME/SHELL/PATH 隔离

**hooks_merge 模块**（已落地，feature `2026-05-18-hooks-merge`，Phase 3）：

- 新文件 `crates/roostery/src/hooks_merge.rs`（档 1 单文件，产品 ~249 行 + 内联测试 ~296 行）+ `src/templates/` 子目录存 3 个资源文件（`cc_stop_hook.json` / `codex_stop_hook.json` / `agent_stop_notify.sh`）
- 3 个 `pub const` via `include_str!` 编译期嵌入（roadmap §4.7 兑现首例）：`CC_STOP_HOOK_JSON` / `CODEX_STOP_HOOK_JSON` / `STOP_HOOK_AGENT_NOTIFY_SH`
- 公开 3 fn：`render_template(src, hook_script)`（`{{HOOK_SCRIPT}}` `str::replace` + `serde_json::from_str`）/ `merge_event_hook(target_path, fragment)`（按 event key + matcher + command tail 三层幂等去重）/ `apply_template(src, target_path, hook_script)`（一站式 render+merge+atomic write）
- `HooksError` `#[non_exhaustive]` 4 变体：ReadFailed / ParseFailed / FragmentInvalid / SaveFailed
- 模板内容：CC + Codex 模板都用 `SessionEnd` event + matcher `"*"` + command `ROOSTERY_AGENT={cc,codex} {{HOOK_SCRIPT}}` + timeout 10；sh 模板从 stdin 抽 session_id / transcript_path / cwd 后调 `roostery dispatcher fire`（Phase 4 dispatcher 起来后正常工作；Phase 3 期间 hook 触发会 clap "unknown subcommand" 但末尾 `\|\| true` 吞掉不阻塞 agent runtime）
- 设计约束：**不引模板引擎**（只 1 个占位符用 `str::replace`）；**env 前缀切到 `ROOSTERY_AGENT`**（不沿用 Python `FEISHU_HUB_AGENT`，roadmap items.yaml "除非文档另有规定" 明示偏离）；**不消费 config**（roadmap depends_on 是规划顺序而非代码 import）；**不实现 unmerge / schema 校验**；**不内置 target_path 默认**（caller 显式传）
- 不变量：merge idempotent（同 fragment 跑 N 次 = 跑 1 次，按 command tail 去重）；atomic `.tmp` + rename；target 不存在 → fragment 直接当结果不报错；parse 失败 → Err 不破坏原文件；command 去重用 tail 匹配（剥 `KEY=VAL` env 前缀，让"用户改 env value 但脚本路径不变"识别为同 hook）；JSON 输出 `indent=2` + `\n` trailing newline

### Module E · Dispatcher（Phase 4）
本地执行桥。event → 规则匹配 → trace/budget gate → runner → emit。`runtime-neutral` req 的执行机制（通过 `Runner` trait 调度，不感知具体 runtime）。

**trace / budget / runaway 模块**（已落地，feature `2026-05-18-dispatcher-trace-budget`，Phase 4 起步）：

- 三独立 gate 模块，互不引用——上层 dispatcher-loop（Phase 4 收尾 feature）作为 caller 串场景把它们编排成 `trace.check_depth → runaway.record + check → budget.check_or_raise → 派发到 runner → budget.consume + save` 链路
- 新文件 `crates/roostery/src/trace.rs`（产品 ~202 行 + 14 内联测）/ `src/budget.rs`（产品 ~330 行 + 14 内联测）/ `src/runaway.rs`（产品 ~160 行 + 8 内联测）；`paths.rs` 加 `budget_state_path()`；`lib.rs` 加 3 pub mod
- 公开 API：
  - `trace::{TraceContext, TraceId, TraceError, ENV_TRACE_ID, ENV_DEPTH, ENV_PARENT_EVENT_ID}`
  - `budget::{BudgetState, Bucket, BudgetError, BUDGET_SCHEMA_VERSION, load, load_from, save, save_to}`
  - `runaway::{RunawayTracker, RunawayError, DEFAULT_WINDOW_SECS, DEFAULT_THRESHOLD}`
- 设计约束：**caller 注入 max_depth**（trace 模块不读 Config，由 dispatcher-loop 把 `Config.trace.max_depth` 传进 `TraceContext::new_root`）；**budget 默认 bucket only**（per-runner / per-rule 推后）；**runaway 内存 only**（跨进程跟踪推后）；**三模块不消费 LarkRunner**（无飞书 IO 责任，纯本地 gate）
- 不变量：TraceContext 不可变（new_root/child 返新值；stamp_journal 仅借用 mut entry 字段）；depth 单调递增（child 总 +1，无 decrement API）；budget save atomic（.tmp + rename + 父目录自建）；budget rollover 幂等（同日多次调无副作用）；BUDGET_SCHEMA_VERSION=1 公开承诺；runaway tracker drop 即丢；runaway 窗口清理懒计算（record 时 retain）
- Cargo.toml 0 新增依赖（trace 用 getrandom 既有；budget 用 serde_json + chrono 既有；runaway std-only）；测试用 atomic clock 不触碰 env，三模块设计上独立于 env 状态

- 子 feature：**`dispatcher-trace-budget`（done）** / **`dispatcher-rules`（done）** / **`dispatcher-runners`（done）** / **`dispatcher-loop`（done）** —— **Module E 整体完成（Phase 4 收尾）**

**dispatcher 主循环模块**（已落地，feature `2026-05-18-dispatcher-loop`，Phase 4 第 4 / 收尾子 feature）：

- 新文件 `crates/roostery/src/dispatcher.rs`（产品 ~470 行 + 内联测 ~530 行）；`journal.rs` 加 `load_by_trace_id` read API；`main.rs` 加 `Command::Dispatcher` 嵌套 `fire / replay / test-rule` clap subcommand；`lib.rs` 加 1 pub mod；新测试文件 `tests/dispatcher_integration.rs` 7 集成测试；**0 新增 Cargo 依赖**
- 公开 API：
  - `dispatcher::{fire, replay, test_rule, DispatchOutcome, DispatchStep, StepStatus, DispatchError, DEFAULT_MAX_FANOUT}`
  - `journal::load_by_trace_id(dir, trace_id) -> std::io::Result<Vec<JournalEntry>>`（journal 首次有 read API）
  - CLI: `roostery dispatcher fire [--agent --session --cwd --summary] [--stdin-event] [--verbose]` / `replay --trace <id>` / `test-rule [flags]`
- 设计约束 / 关键决策（user 拍板）：
  - **`fire` 始终 exit 0 + journal 落档失败原因**（hook 调用方对错误不敏感）；`replay` / `test-rule` DispatchError exit 1（用户主动调）
  - **emitted_events 本期消费走自触发链式分发**（BFS 队列 + `ctx.child()` depth+1 + `trace.check_depth` gate）
  - **`replay` live 真跑 runner**（不 dry）+ **分配新 trace_id**（不沿用，避审计混淆）+ journal `trigger_meta.replay_of` 字段关联源
  - **unknown runner kind → `StepStatus::Skipped`**（与 runtime-neutral req 一致：runtime 未接入前用户感知 not supported；budget 不消费）
- 不变量（fire 主链路）：5 gate / 1 engine 顺序 `trace.check_depth → rules.matches → budget.check_or_raise(0.0) → runaway.record + check → registry.find → runner.run → budget.consume(cost) + save`；每分支 journal.append；`trace.max_depth` 守深度 + `DEFAULT_MAX_FANOUT` 守 width 双守门；`dispatcher.rs` 不消费 LarkRunner / 不直接 spawn / 不引 reqwest（红线 grep N1-N3 守护）；rules / config 在 fire 入口加载一次，链式分发期间不重读
- caller 编排终点：`bot-stop-hook`（Phase 5）会在 stop hook sh 喂 HookEvent 给 `roostery dispatcher fire`；目前 `templates/agent_stop_notify.sh` 已经走这条路径，本 feature 落地后该模板从"clap unknown subcommand 被 `|| true` 吞"切换到"真跑 dispatcher 主循环"

**runners 模块**（已落地，feature `2026-05-18-dispatcher-runners`，Phase 4 第 3 子 feature）：

- 新文件 `crates/roostery/src/runners.rs`（产品 ~465 行 + 24 内联测）；`lib.rs` 加 1 pub mod；新依赖 `async-trait`（trait async method）/ `which`（PATH 查找 `claude` binary）/ `tempfile`（dev-dep，test fixture）；无 reqwest / 外部 LLM client；不引 `tokio::process` / `tokio::time::timeout`
- 公开 API：
  - `runners::{Runner, RunnerStatus, RunOutcome, RunnerError, RunnerRegistry, NoopRunner, CcHeadlessRunner, SAFE_ENV_FORWARD, DEFAULT_TIMEOUT_MS, STDOUT_HEAD_CAP}`
- 设计约束（user 拍板，acceptance 阶段已建议走 `cs-roadmap update` 同步 §4.3 契约）：(a) `Runner::run` **不收** `&BudgetGate` 参数（与 roadmap §4.3 偏离）——budget 编排留给 dispatcher-loop；(b) `RunOutcome` 加 `cost_usd: Option<f64>` 字段——让 caller 走 `budget.consume(cost_usd)`；(c) 首发 = `noop` + `cc_headless` 真实现（codex_exec / gemini_headless 完全不出现，items.yaml notes 明示可推后）；(d) 内部走 `tokio::task::spawn_blocking` 包同步 `std::process::Command`（async trait 兼容 + 不踩 ETXTBSY race）；(e) env sanitize 走 `SAFE_ENV_FORWARD` const allowlist 而非父进程整盘 copy；(f) CC JSON 解析容错——失败仍返 Success cost None
- 不变量：`RunnerError` vs `RunOutcome.status.Failed` 语义分层（基础设施失败 vs runner 业务失败）；`emitted_events` 本期 cc_headless 始终空 Vec（chain dispatch 推给 dispatcher-loop）；registry find 未命中返 None 不报错；同 kind 二次注册 linear find 返第一；trace env 注入经 `trace::to_env_pairs()` 三 env（`ROOSTERY_TRACE_ID` / `ROOSTERY_DEPTH` / `ROOSTERY_PARENT_EVENT_ID`），覆盖父 env collide
- caller 编排预期（dispatcher-loop 后续 feature 拼）：`registry.find(m.runner) → runner.run(event, ctx, m.args) → match outcome.status { Success → budget.consume(cost_usd if Some) → journal; Failed/Skipped → log + skip consume → journal }`

**hook_event / rules 模块**（已落地，feature `2026-05-18-dispatcher-rules`，Phase 4）：

- 两新文件 `crates/roostery/src/hook_event.rs`（产品 ~50 行 + 7 内联测）+ `src/rules.rs`（产品 ~240 行 + 21 内联测）；`paths.rs` 加 `rules_path()`；`lib.rs` 加 2 pub mod；新依赖 `globset = "0.4"`（ripgrep team，well-tested 仅 fnmatch glob 编译用）；无 reqwest / HTTP / 外部 LLM client
- 公开 API：
  - `hook_event::{HookEvent, HOOK_EVENT_SCHEMA_VERSION}`
  - `rules::{RulesConfig, RawRule, RuleWhen, RuleAction, CompiledRule, Match, RuleName, RulesError, RULES_SCHEMA_VERSION, load, load_from, matches}`
- 设计约束（user 拍板 MVP）：(a) Match 维度仅 3 项 AND（hook_source eq + workspace_glob fnmatch + trigger_meta 点路径 eq）；扩 OR / regex / contains 等走未来 cs-roadmap update；(b) Action opaque args 透传（rules 不解释 args，runner impl 自决怎么 parse）；(c) 无模板引擎（runner 自决怎么拼 prompt）；(d) first-match-wins 返 `Option<Match>`（continue 链推后）
- 不变量：load 缺文件 → Ok(vec![]) first-run 友好；compile 一次性（matches 阶段不走字符串）；self-event 短路（`hook_source` 前缀 `dispatcher.` / `roostery.` 直接返 None，**防 dispatcher 自激**）；CompiledRule 不可变（matches 只读引用）；`HOOK_EVENT_SCHEMA_VERSION=1` + `RULES_SCHEMA_VERSION=1` 双公开承诺
- caller 编排预期（dispatcher-loop 后续 feature 拼）：`HookEvent in → rules.matches → trace.check_depth → runaway.check → budget.check_or_raise → runner.run(args) → budget.consume + save`。本 feature 提供 `matches` 入口；剩余 chain 由 dispatcher-loop 接

### Module F · Bot Bridge（Phase 5）
agent run → Feishu task card + step stream + IM thread。**`agent-work-in-feishu` req 的直接兑现层**。`bot-stop-hook` feature 完成 = "Rust 可用" milestone = **0.1.0 触发判据达成**（2026-05-18，feature `2026-05-18-bot-stop-hook` 合入 commit `220c7b0`，CI run `26030808131` 全绿）。
- 子 feature：**`bot-task-writer`（done）** / **`bot-stop-hook`（done）** / **`bot-bridge-cluster`（done，2026-05-19）**

**bot_task_writer 模块**（已落地，feature `2026-05-18-bot-task-writer`，Phase 5 第 1 子 feature）：

- **首次让 Rust 业务模块真消费 `LarkRunner` trait 做生产飞书 IO**（dispatcher 不走飞书；smoke 走 raw bytes；shim 走透传 + journal——本模块是首条 buffered Value 业务路径）
- 新文件 `crates/roostery/src/bot_task_writer.rs`（产品 ~440 行 + 内联测 ~597 行）；`lib.rs` 加 1 pub mod；新测试文件 `tests/bot_task_writer_integration.rs` 3 集成测试；**0 新增 Cargo 依赖**
- 公开 API（纯库 3 pub async fn，**不**挂 dispatcher registry）：
  - `bot_task_writer::create_task(runner, agent, cwd, summary, opts) -> Result<TaskRef, TaskWriterError>`
  - `bot_task_writer::append_steps(runner, &task_guid, steps, opts) -> Result<(), TaskWriterError>`
  - `bot_task_writer::get_or_create_for_session(runner, agent, session, cwd, summary, opts) -> Result<TaskRef, TaskWriterError>`
- 关键行为（user 拍板）：
  - **host suffix** 自动后缀 `· {host}`（`ROOSTERY_HOST` env > hostname 首段 > `DEFAULT_HOST_FALLBACK="unknown"` 三 fallback；幂等不重复加）
  - **assignee None 走 `identity::current` 注入**；identity 失败返 `Err(IdentityResolveFailed)`，**不** silently 不带 assignee（没 assignee 的 task 不进用户"我的待办"，与 req 核心 UX 冲突）
  - **session_cache JSON v1** 持久化在 `~/.roostery/state/session_tasks/{safe}.json`；atomic `.tmp` + rename；schema_version 缺失向后兼容 read；safe_filename 防路径跳出（连续 `..` → `__`）
  - **部分失败语义**：`create_task` OK + `append_steps` Err → 本模块 fn 不耦合 caller 编排；caller 自决；下次 `get_or_create_for_session` 自然走 cache 重试 append
- caller 编排预期（Phase 5 第 2 子 feature `bot-stop-hook` 拼）：stop hook sh 喂 stdin JSON → `bot_stop_hook` 读 → 调 `get_or_create_for_session` + `append_steps` 把 agent 工作过程串成飞书 task

**bot_stop_hook 模块**（已落地，feature `2026-05-18-bot-stop-hook`，Phase 5 第 2 子 feature = **minimal-loop closing / 0.1.0 触发判据**）：

- **双 CLI surface 共享单一核心 lib fn `push`**：
  - `roostery bot stop-hook` — **被动 hook 入口**：从 stdin 读 CC/Codex/Gemini SessionEnd JSON（schema = `StopHookInput`），Rust 端原生 tail transcript jsonl 抽最后一条 assistant text 作 summary（替代 Python 期 sh+jq 抽字段链）
  - `roostery bot push` — **反向调用入口**：让任意 agent / 脚本 / cron / CI 通过 flag-based CLI 主动推送进度到飞书。flag = `--agent --session --cwd --summary | --summary-stdin --description --assignee-open-id --strict --json --no-im-fallback`。这是 `agent-work-in-feishu` req 的**第二维兑现**——不只 stop hook 被动触发，任何 agent 都能脚本化把工作推进度推到飞书
- 新文件 `crates/roostery/src/bot_stop_hook.rs`（产品 ~600 行 + 内联测 ~450 行）；`lib.rs` 加 1 pub mod；新测试文件 `tests/bot_cli_integration.rs` 4 binary-level e2e；**1 新增 Cargo 依赖 `blake3 = "1"`**（跨进程稳态 idempotency key 短哈希，修 `std::hash::DefaultHasher` SipHash 启动种子随机化在 lark-cli `--idempotency-key` 链路里的幂等失效 bug）
- 公开 API（核心 lib + cli 适配）：
  - `bot_stop_hook::push(req: PushRequest, runner, opts) -> PushOutcome` — 共享业务编排
  - `bot_stop_hook::run_stop_hook(runner, opts) -> PushOutcome` — stop-hook CLI 适配（stdin 解析 + transcript tail）
  - `bot_stop_hook::cli::{BotArgs, BotSub::{StopHook, Push}, PushCliArgs, StopHookCliArgs}` + `cli::run(args) -> ExitCode` — clap 入口；main.rs 仅一行 dispatch（**设计 2.5 convention 提议**：未来子命令的 args / run 都放对应模块的 `pub mod cli`，main.rs 只做顶层 enum + match）
- 关键行为（user 拍板）：
  - **receive_id 三层链** = `env ROOSTERY_NOTIFY_TO > identity::current(runner).user_open_id > config.identity.user_id`；三层全空 → `PushStatus::Skipped` 静默 exit 0（不调 lark-cli）。**不引入 config.identity.notify_receive_id 新字段**——复用 `user_id`
  - **task_writer 失败 → IM 兜底** (`lark-cli im +messages-send`)：除 `--no-im-fallback` opt-out 外，所有非 Ok 都走 IM；IM 也失败 → `PushStatus::Failed` + 累积 `errors`
  - **默认 exit 0**（hook 路径不阻塞 agent runtime）；`--strict` opt-in 真实 exit code（仅 `Failed` 时 exit 1，`Skipped` 不算错）
  - **`--json` 结构化 stdout**：`PushOutcome` 序列化（v1 稳定契约，新字段 backwards-compatible append）让 CI / cron / 脚本 caller 可 jq 取 task_url / fallback_used
  - **summary 截断 UTF-8 边界安全**：`truncate_utf8` 用 `is_char_boundary` 不切坏多字节字符（Python `head -c 200` 在中文 / emoji 上会切坏）
- **极简 sh wrapper 切换**（`templates/agent_stop_notify.sh` 47 行 → 10 行 + 0 jq/tac）：从"sh 用 jq 抽 cwd / session / transcript / tail + 调 `roostery dispatcher fire`"退化为"stdin 直透 `roostery bot stop-hook`"，Rust 端原生处理。旧用户重跑 `roostery init` 自然升级（include_str! 编译期嵌入 + hooks_merge 幂等覆盖）
- **与 dispatcher 的关系**：bot_stop_hook 与 dispatcher 是**两个独立顶层 CLI 入口**，不互调（架构红线 D14）。dispatcher 是通用 rule 引擎（HookEvent / budget / trace），bot push/stop-hook 是飞书 task 快通道（PushRequest / IM 兜底）。未来若需统一可新开 feature 加 `BotPushRunner` 适配器，不阻塞 0.1.0

**bot_bridge 模块簇**（已落地，feature `2026-05-19-bot-bridge-cluster`，Phase 5 第 3 子 feature）：

- **长跑 daemon `roostery bot bridge`**：订阅 IM event → 路由匹配 @mention 的消息 → 调 Runner 实例跑 → 写飞书 task step 流 → IM thread 回复；群里 `/stop` `/abort` `停` `中止` 中止正在跑的 runner，`/adjust <body>` 带追加 prompt 重启 runner（上限 `ADJUST_MAX = 1` Python parity）
- **5 Python 模块（bot_role / bot_runner / bot_bridge / bot_relay_task / hitl_router）→ 1 Rust 子目录** `crates/roostery/src/bot_bridge/`，9 文件按职责切分（D1）：
  - `role.rs`            BotRole + BotsConfig + load_bots + event_matches_bot + extract_message_body
  - `hitl.rs`            HitlDecision 三态分类 + ABORT_KEYWORDS / ADJUST_PREFIXES const 写死
  - `active_registry.rs` ActiveRunnerRegistry 进程内活跃 runner 表 + oneshot HitlSignal 通道
  - `relay_task.rs`      chat_id → TaskRef 缓存 + EndOutcome 四态 step 文案 + record_start/adjust/end
  - `event.rs`           ImEvent + consume_im（lark-cli `im_messages_subscribe` 子进程 NDJSON tail + 指数退避重连 cap 60s）
  - `runner.rs`          handle_event 编排（select! runner_future vs kill_signal + Adjust 重启循环）
  - `daemon.rs`          run_bridge 主循环（per-bot consume_im → 中央 mpsc → HITL 串行分流 → spawn handle_event → graceful shutdown）
  - `cli.rs`             BridgeCliArgs 5 flags（bots / profile / max_concurrency / max_events / timeout）
- **关键架构选择**：
  - **HITL 信号通道走进程内 `tokio::sync::oneshot::Sender<HitlSignal>` 不落盘 sentinel**（design D3）——Python 期 `~/.feishu_hub/state/runner_registry/{task_guid}/abort.txt` 文件通信是"runner 跨 process"的副产品；Rust 期 runner 与 bridge 同 tokio runtime，oneshot 是 idiom，消除 race window
  - **runner 调用必经 `dispatcher::runners::Runner` trait + Registry**（design D4）——`BotRole.runner` 字段值 = `Runner::kind()`；Phase 4 已落 `NoopRunner` / `CcHeadlessRunner`，未来加 codex_exec / gemini_headless 无需改 bot_bridge
  - **task 写入复用 `bot_task_writer` 公开 API**（design D5）——不直接调 LarkRunner 创 task；继承 `append_steps --yes` 架构红线显式破例（§6 #18）
  - **per BotRole 独立 cache 目录** `~/.roostery/state/bot_chats/<safe(bot_app_id)>/`（design D10）——与 `session_tasks/` 平级兄弟目录，由 `paths::bot_chat_cache_dir()` 解析；chat_id 文件名层 safe_filename 防路径跳出
  - **HITL 判定必须串行先于 spawn handle_event**（design §2.2 流程级约束）——daemon 中央 dispatcher loop 收 mpsc 时先 `classify` 命中 abort/adjust 直接走 `active_registry.send_signal` 不 spawn；否则 `/stop` 可能错过新启动的 runner
  - **`ActiveRunnerRegistry` 命名避让 `dispatcher::runners::RunnerRegistry`**（design D2）——后者是 "runner kind 注册表"，前者是 "活跃 task 实例表"；长期重构待 `cs-refactor` 把 dispatcher 那个改 `RunnerKindRegistry`
- **明确不做**（design §3 反向核对项）：
  - 不引用 Base / base_intent_router（Phase 7 base-indexer 落地后再起独立 feature 评估）
  - 不沿用 Python `--parallel` flag（Rust 默认 tokio spawn per event；`--max-concurrency N` 控并发）
  - 不实现 `cleanup_orphans`（ActiveRunnerRegistry 是进程内内存表，daemon 重启天然清零）
  - 不引 user-customizable abort / adjust 关键词（const 写死）
  - 不沿用 `relay_writer_app_id` 跨 bot 共享 task（推后；本期每 bot 独立 chat→task 缓存）
  - 不沿用 POSIX `os::kill` / SIGTERM / SIGKILL（走 tokio oneshot channel）
- **journal source / action 命名空间** `bot_bridge:*`：
  - `bot_bridge:daemon` —— daemon main loop + dispatch_hitl_{abort,adjust} 副作用
  - `bot_bridge:handle_event` —— event:received / event:skipped / event:hitl_adjust / event:handle_complete
- **跨模块边界**：
  - 飞书 IO 必经 `LarkRunner` trait（红线 #1）；唯一例外是 `event.rs` 的 IM streaming subscribe 走 `tokio::process::Command::new(&opts.binary)` 注入二进制路径（NDJSON tail 是长跑流式模型，与 buffered Value LarkRunner 不兼容；变量名而非字面量 `"lark-cli"`）
  - `~/.roostery/state/bot_chats/{app_id}/{safe_chat}.json` 仅是缓存（红线 #2），丢失重建即可——任务状态查询永远走飞书
  - 0 LLM client import（红线 #3）；runner 调用走 dispatcher::runners 已有实装
- 引用相关 decisions：`.codestable/compound/2026-05-19-decision-runtime-launch-strategy.md`（tmux default over ACP / direct spawn）、`.codestable/compound/2026-05-18-decision-cli-subcommand-module-layout.md`（cli.rs per-module convention）、`.codestable/compound/2026-05-16-decision-rust-module-organization.md`（500+ 行升档 2 子目录约定，本 feature 9 文件落实）

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
| 4.3 | `Runner` trait | E → 具体 runner | Phase 4 — **已落地**（feature `2026-05-18-dispatcher-runners`）；**与 §4.3 原契约偏离两项**：(a) `run` 不收 `&BudgetGate` 参数（budget gate 编排留给 dispatcher-loop）；(b) `RunOutcome` 加 `cost_usd: Option<f64>` 字段。建议 `cs-roadmap update` 把 §4.3 原文改齐 |
| 4.4 | `HookEvent` schema | D/E → E | Phase 3-4 — **已落地**（feature `2026-05-18-dispatcher-rules`） |
| 4.5 | `TraceContext` | E → F → C | Phase 4 — **已落地**（feature `2026-05-18-dispatcher-trace-budget`） |
| 4.6 | Config schema | D 写 → 所有读 | Phase 3 — **已落地**（feature `2026-05-17-config-yaml`） |
| 4.7 | 模板嵌入约定 | D → 用户文件系统 | Phase 3 — **已落地**（feature `2026-05-18-hooks-merge` 立首例 cc+codex 二模板；feature `2026-05-18-roostery-init` 顺手补 gemini 第 3 模板，3 个 `pub const` `include_str!` 编译期嵌入） |

## 5. 关键架构决定

1. **vendor-neutral 桥而非 SDK**。Roostery 不替代 agent runtime，也不替代 Feishu，它只做转换 + 审计
2. **Feishu = default view，不是 lock-in**。本地是 cache / audit，journal 是 portable 数据形态——飞书出问题 / 想换前端，能基于 journal 重建（兑现 `portable-by-default` req）
3. **`lark-cli` 是唯一飞书入口**。不允许新增 HTTP client 直连 `open.feishu.cn`
4. **dispatcher hook-agnostic**。新 hook 源（Codex / Gemini / Cursor）通过 `hooks_merge` + 模板嵌入扩展，loop 不感知 provider
5. **`llm_summary` 模块是 LLM provider 集成的唯一白名单**。其他模块保持 vendor-neutral
6. **业务标识符 newtype 隔离**（自 core-remoterefs 起）：对**从飞书侧拿到的、有明确业务语义角色的标识符**（token / id / cursor）一律用 newtype + `#[serde(transparent)]` 隔离类型；不实现互转 `From` impl。Phase 4 dispatcher 的 `TraceId` / `EventId` / `ParentEventId` 是合格候选；**不**适用于"还没成为业务 token 的字符串"（subcommand 名 / 原始 argv / 普通 String 参数）——后者 newtype 化是 noise。详见 `.codestable/compound/` 待归档 convention
7. **Rust 模块组织五档约定**（自 core-redact / journal-core 起，0.1.0 后由 bot_stop_hook / onboarding 大规模实战验证）：单文件 < 500 行 / 500+ 升档 2 子目录 + `mod.rs` / 独立 crate / Cargo bin target / 资源文件子目录。**主动路径**走 feature `design §2.5` 评估；**回溯路径**走 `cs-audit → cs-refactor`（rustc E0761 禁止 `foo.rs` 与 `foo/mod.rs` 并存，搬运必须原子动作）。详见 `.codestable/compound/2026-05-16-decision-rust-module-organization.md`
8. **shim / smoke 与 LarkRunner 走三条独立 I/O 路径**（自 lark-cli-shim 起，roostery-smoke 进一步验证）：
   - **shim**：streaming bytes 模型 + `std::thread::spawn` + `std::process`（透明 tee 4 KiB chunks 给用户，head buffer 副本写 journal）
   - **smoke**：raw bytes 模型 + 同步 `std::process` + `try_wait` 50ms 轮询（用 stdout 文本检 "Dry Run" marker；不解析 JSON；写 state file 不写 journal）
   - **`LarkRunner`**：buffered Value 模型 + tokio（`wait_with_output` 一次性 collect + `serde_json::Value` parse；调用结果返给 caller）

   三条路径 I/O 语义根本不同（streaming/raw bytes 检文本 vs buffered Value parse JSON），所以**不强行抽公共 trait**；只共享下层 `journal` / `redact` / `remoterefs` / `paths` 模块。下游 read/replay 通过 `JournalEntry.source` 字段（"shim" / "dispatcher" / ...）+ `~/.roostery/state/smoke.json` 状态文件分流
9. **多 bot daemon + IM HITL 反向控制走进程内 tokio oneshot channel**（自 feature `2026-05-19-bot-bridge-cluster` 落地起）：`bot_bridge::active_registry::ActiveRunnerRegistry` 用 `BTreeMap<TaskGuid, RunnerHandle>` 内存表 + `tokio::sync::oneshot::Sender<HitlSignal>` 给运行中 runner 发 abort / adjust 信号；**不落盘 sentinel 文件**——与 Python 期 `~/.feishu_hub/state/runner_registry/{task_guid}/abort.txt` 跨 process 文件通信对比，Rust 期 runner 与 bridge 在同 tokio runtime 下，oneshot 是 idiom，消除文件 race window 与 ~80 行落盘清理代码。**与 Python 1:1 翻译的偏离代表案例**——印证 "代码-文档优先级"（attention.md）：Python 是 prior baseline 不是应有形态，Rust port 不机械翻译。daemon 重启 ActiveRunnerRegistry 天然清零（不存在 cleanup_orphans 需求）。多 bot daemon 用中央 mpsc 把 per-bot consume_im 流合并到单一 dispatcher loop，**HITL classify 串行先于 spawn handle_event** 是保证 `/stop` 不被并发新 runner 抢先排队的核心顺序约束

## 6. 已知约束 / 硬边界

> 完整 9 条硬约束见 `.codestable/attention.md`——每次 CodeStable 子技能启动自动加载。

1. **禁止重实现 lark-cli**。飞书 API 必经 `lark_cli` wrapper；不准 `reqwest` / `requests` 打 `open.feishu.cn`，也不引 Feishu SDK。**兑现层**：`crates/roostery/src/lark_cli/`（feature `2026-05-16-lark-cli-wrapper`，commit `cc44dfa`）暴露 `LarkRunner` trait + 三个实现；新模块依赖飞书操作必须 take `Arc<dyn LarkRunner>` / `impl LarkRunner` 注入，禁止直接拼 `Command::new("lark-cli")`（双向引用 `lark_cli/mod.rs` 顶部 docstring）。**装机端兑现链**：feature `2026-05-17-lark-cli-shim` 把 `bin/shim` 装到 `~/.local/bin/lark-cli`（PATH 前段拦截），agent runtime 直接调 `lark-cli` 也被透明截获写 journal 后透传到 real lark-cli——这是同一红线的"客户端绕过路径"封堵，与 wrapper 是同一硬约束的两个兑现层（`crates/roostery/src/bin/shim.rs` 顶部 docstring 反向引用本节）
2. **本地 state 是 cache 不是真相**。`~/.roostery/`（Rust 期；Python 期 `~/.feishu_hub/`）下任何东西都只是可重放的审计，不回答"任务 X 现在状态如何"
3. **`llm_summary` 是外部 LLM client import 的唯一允许位置**
4. **lark-cli 版本最低 pin 在 1.0.28**（`task append_task_steps` timestamp schema 兼容）。升级需先跑 smoke。**1.0.29 已实测兼容**（feature `2026-05-17-roostery-smoke` 2026-05-17 跑通 6 条 PROBE_MATRIX）
5. **smoke 是升级后的 gate**。任意 probe 失败 `roostery init` 和 `daily_report` 拒绝运行。**兑现层**：`crates/roostery/src/smoke.rs::ensure_ready()`（feature `2026-05-17-roostery-smoke`），caller 走 `Result<(), SmokeError>` match 三个具体错误变体（NeverRun / LastFailed / StateLoadFailed）；状态文件 `~/.roostery/state/smoke.json` 含 `lark_cli_version` 字段助升级漂移诊断
6. **代码-文档优先级**：Python baseline 与最新文档冲突时**以文档为准**（见 attention.md）。Rust port 不机械 1:1 翻译，失配点记观察项
7. **redact 模块函数纯且幂等**：`redact::scrub_value` / `scrub_argv` / `scrub_text` 不修改入参（接 `&` 借用返回 owned 新值）；对已含 `MASK` 的输入再跑结果等价；audit path 顺序 = 遍历顺序（Phase 1 落地，commit `1e392e5`）
8. **journal schema_version=1 公开承诺**：自 journal-core 落地起（commit `b9ac5be`），`JournalEntry` 字段名 / 类型 / 序列化形态变更需 bump version + 兼容旧版 deserialize + `cs-roadmap update` 评估 portable-by-default 影响。`Journal::append` 不内建脱敏，caller 自行用 `redact::scrub_value` 过 `params` 后填入
9. **`ROOSTERY_AGENT` env 约定**：agent runtime 识别用 `ROOSTERY_AGENT=cc` / `=codex` / `=gemini`（Stop hook command 拼前缀），由 stop bridge sh 在 hook fire 时读取传给 `roostery dispatcher fire`；**不沿用** Python `FEISHU_HUB_AGENT`（feature `2026-05-18-hooks-merge` 一次切口径 cc/codex 立项，feature `2026-05-18-roostery-init` 加 gemini）
10. **`ROOSTERY_REAL_LARK_CLI` env 持久化路径** = `~/.roostery/env` + shell rc marker block 幂等 append（`# >>> roostery >>>` / `# <<< roostery <<<`，conda/pyenv 风格）。由 feature `2026-05-18-roostery-init` 在 `roostery init` 装机末段写入；用户后续升级 / 切 lark-cli 路径时编辑 `~/.roostery/env` 即可，不必重跑 `roostery init`；marker block 让用户能定位 / unpatch（Roostery 不实装 uninstall）。仅支持 zsh / bash（fish / nushell `UnsupportedShell` 拒绝）
11. **`BUDGET_SCHEMA_VERSION = 1` 公开承诺**：自 feature `2026-05-18-dispatcher-trace-budget` 落地起，`~/.roostery/state/budget.json` schema 字段名 / 类型 / 序列化形态变更需 bump version + `cs-roadmap update` 评估 + 旧版兼容反序列化。同 `JournalEntry.schema_version` 模型；目前仅 default 单 bucket，per-runner / per-rule / by-rule 等粒度扩展会触发 schema bump
12. **`TraceContext` max_depth caller 注入**：trace 模块不读 `Config`，`Config.trace.max_depth` 由 caller（Phase 4 dispatcher-loop）在 `TraceContext::new_root(parent_event_id, max_depth)` 调用点显式传入。**理由**：trace 模块是无状态 gate，不承担读 config 的副作用；caller 自决何时刷新 max_depth（restart vs hot reload）
13. **Runner 子进程 env 必经 `SAFE_ENV_FORWARD` allowlist**（自 feature `2026-05-18-dispatcher-runners` 落地起）：父 hook 状态（如 `ROOSTERY_AGENT` / `ROOSTERY_REAL_LARK_CLI` / 任意 `ROOSTERY_*` 调用方 state）**不串到子 agent**——避 trace 链断裂、避用户 env 噪声透传到 LLM 服务、避隐式依赖。新增允许的 env 必须改 `runners::SAFE_ENV_FORWARD` const（grep-able 单点定义）+ 改 design doc 说明理由。trace ctx 三 env（`ROOSTERY_TRACE_ID` / `_DEPTH` / `_PARENT_EVENT_ID`）单独经 `trace::to_env_pairs()` 注入，**优先级覆盖**任何 collide 的父 env
13. **`HOOK_EVENT_SCHEMA_VERSION = 1` + `RULES_SCHEMA_VERSION = 1` 双公开承诺**：自 feature `2026-05-18-dispatcher-rules` 落地起，`HookEvent` schema 字段名 / 类型 + `~/.roostery/rules.yaml` schema 变更需 bump version + `cs-roadmap update` + 旧版兼容反序列化。同 `JournalEntry` / `Config` / `BudgetState` 模型
14. **dispatcher self-event 防自激约定**：`HookEvent.hook_source` 以 `dispatcher.` / `roostery.` 开头的事件被 `rules::matches` 短路返 `None`——这是 dispatcher 自己产生的事件不应再触发新规则评估的硬约束（防自激死循环）。`SELF_EVENT_PREFIXES` const 在 `rules.rs:33`；dispatcher-loop 上层 caller 命名自身 emit event 时**必须**用 `dispatcher.` / `roostery.` 前缀
15. **`dispatcher::fire` 始终 exit 0**（自 feature `2026-05-18-dispatcher-loop` 落地起）：`roostery dispatcher fire` 子命令无论 gate 拒 / runner 失败 / DispatchError 都 `ExitCode::SUCCESS`，失败原因走 journal 落档。**理由**：hook 调用方（CC / Codex SessionEnd sh）对错误不敏感（hook 已结束），分级 exit code 只会污染 hook 链上下游。`replay` / `test-rule` 不在此约束内——这俩用户主动调，DispatchError exit 1 让脚本能感知失败
16. **emitted_events 链式分发 fanout cap**（自 feature `2026-05-18-dispatcher-loop` 落地起）：`fire` 内部 BFS 队列消费 `RunOutcome.emitted_events` 时，单 step 单批 emitted_events 个数 ≤ `dispatcher::DEFAULT_MAX_FANOUT`（= 16），超出截断 + journal 标 `fanout_truncated`。**理由**：`trace.max_depth` 守深度，但单层 width 也需守门，防 runner bug / 链式风暴把队列撑爆。改 cap 必须改 const + 改 design doc 说明
17. **`dispatcher.rs` 不直接走飞书 IO + 不直接 spawn**（自 feature `2026-05-18-dispatcher-loop` 落地起）：dispatcher 只做编排——飞书 IO 责任在具体 Runner impl 内部（如 CcHeadless 调 `claude` binary）或后续 Phase 5 `bot-task-writer` feature；子进程 spawn 责任在 Runner impl。`dispatcher.rs` grep `LarkRunner|lark_cli::|reqwest|Command::new|std::process::Command|tokio::process` 必须 0 命中（doc 注释中的 disclaimer 不算）
18. **`bot_task_writer::append_steps` `--yes` 是 lark-shared 红线显式破例**（自 feature `2026-05-18-bot-task-writer` 落地起）：lark-shared SKILL 红线规定"未经用户同意不加 `--yes`"，本处是 sanctioned 例外——bot 写自己创建的 task 等价 agent 内部行为（append-only step stream，对用户资源无破坏性影响）。**理由链**：(a) `task.agent_task_step_info.append_task_steps` 在 lark-cli 标为 high-risk-write，缺 `--yes` 会 exit 10 `confirmation_required`；(b) 写入对象是 bot 自己 create 的 task（user-created task 写 step 会 10403），所以 bot 写自己的 step ≠ 写用户资源；(c) Python 版 POC 已验证；(d) Rust 版模块顶部 doc + design §1.2 D4 明示。**未来加新破例必须先 update 本节** + 模块顶部 doc 双签
19. **`ROOSTERY_LARK_CLI_BIN` env 双语义复用**（自 feature `2026-05-18-init-real-lark-cli-override` 落地起）：同一 env 在 **runtime** 与 **init time** 双场景下被读取——runtime 决定 `LarkCli` subprocess 调什么二进制（`lark_cli/subprocess.rs:14`），init time 决定写到 `~/.roostery/env` 的 `ROOSTERY_REAL_LARK_CLI` 是什么（`onboarding.rs::resolve_real_lark_cli` 三层链第 2 层）。两者在用户视角一致（"我的 lark-cli 在这里"），但**红线**：env 永远不该被设成 shim 自身路径（`~/.local/bin/lark-cli`），否则 shim 自递归 forward 死循环。本 feature 不防御 self-loop（design §1.3 明确不做，记观察项 O2）；后续若有 user 撞到走 cs-issue 开 self-check 改进 feature。**与现有红线 #1（`lark-cli` 唯一飞书入口）+ #10（`ROOSTERY_REAL_LARK_CLI` 持久化路径）配套**——一个 env 串起 runtime / init / shim forward 三个层面，确保 single source of truth
20. **`BOTS_SCHEMA_VERSION = 1` 公开承诺**（自 feature `2026-05-19-bot-bridge-cluster` 落地起）：`~/.roostery/bots.yaml` 顶层 `schema_version: 1` 是用户编辑的配置 schema 承诺；字段名 / 类型 / 序列化形态变更需 bump version + `cs-roadmap update` 评估 + 旧版兼容反序列化。schema_version 缺失走 `serde(default)` = 1（向后兼容）；显式 != 1 → `BotRoleError::SchemaVersionMismatch`。const 定义 `crates/roostery/src/bot_bridge/role.rs:17`。同 `JournalEntry` / `Config` / `BudgetState` / `HookEvent` / `SESSION_CACHE_SCHEMA_VERSION` 模型
21. **`BOT_CHAT_CACHE_SCHEMA_VERSION = 1` 公开承诺**（自 feature `2026-05-19-bot-bridge-cluster` 落地起）：`~/.roostery/state/bot_chats/{app_id}/{safe_chat}.json` schema 字段名 / 类型 / 序列化形态变更需 bump version + `cs-roadmap update` 评估 + 旧版兼容反序列化（缺失走 serde default 0 → 视为 1）。const 定义 `crates/roostery/src/bot_bridge/relay_task.rs:31`
22. **`bot bridge` daemon 不感知 Base / base_intent**（自 feature `2026-05-19-bot-bridge-cluster` 落地起）：`bot_bridge::daemon::run_bridge` 主循环只做 IM event → @mention 路由 → runner → 回复，**不解释 base_intent / `/run <base_ref>` 路由**（Python 期 `bot_bridge._try_base_intent` 是 M4.D 与 Base 模块的耦合）。Rust 期 Base 在 Phase 7 落地后是否在 bridge 加 base intent 钩子由独立 feature 评估，不在本 feature 范畴。**与 dispatcher / bot push 三条独立顶层 CLI 入口语义并列**——三者不互调（dispatcher = 通用 rule 引擎；bot push/stop-hook = 飞书 task 快通道；bot bridge = IM 长跑 daemon），未来若需统一可起 cs-refactor
