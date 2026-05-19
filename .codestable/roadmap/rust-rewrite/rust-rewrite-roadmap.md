---
doc_type: roadmap
slug: rust-rewrite
status: active
created: 2026-05-15
last_reviewed: 2026-05-19
tags: [rust, rewrite, porting, milestone]
related_requirements: [agent-work-in-feishu, runtime-neutral, portable-by-default]
related_architecture: [ARCHITECTURE]
---

# Roostery Rust 重写 Roadmap

## 1. 背景

Roostery 现有代码是从 prior `feishu_hub` baseline 整体 import 的 Python 实现（M3.C → M5.A，~7339 LOC，40+ 测试）。该 baseline 从未公开发布过，brainstorm `v0.x-direction` 已敲定：

- **Rust 重写**是 0.x 阶段唯一推进方向，Python 是 baseline 参考、不维护、Phase 7 删
- **不发任何 release** 直到 Rust 达到"可用"形态（暂定 Phase 5 完成 = bot bridge 通 + 至少 CC runtime 出 task）
- 学 Rust 同时是作者本人的学习路径，**节奏自由 / 不急 / 占位好好做**

本 roadmap 把原 `planning/2026-05-15-rust-rewrite.md`（gitignored 本地笔记）正式化为可被外部 contributor 看到的 phase 计划，并把每个 phase 跟 req 验收挂钩。原 planning 文档仍然作为 Phase 划分 + Rust 学习目标 + 技术选型的依据档案，本 roadmap 不重复其内容，只接管 phase → feature 拆解 + 跨模块接口契约。

## 2. 范围与明确不做

### 本 roadmap 覆盖
- Rust workspace 脚手架 + Python 归档（Phase 0）
- 所有 Python 模块的 Rust port（Phase 1-7，按模块功能聚类）
- 跨模块接口契约：LarkRunner trait、JournalEntry schema、Runner trait、HookEvent schema、TraceContext、Config schema、模板嵌入约定
- 终态清理（删 legacy/python/、重写 README + CLAUDE.md、crates.io 准备）

### 明确不做
- **不做 0.1.0 之后的 release 节奏规划**——本 roadmap 推到 0.1.0 验收为止，后续版本演进等首发后再起新 roadmap
- **不在 Rust port 期间引入新功能**——port 期间发现 Python 版有缺陷 / 想加的新能力，记观察项推后处理，不让 scope 漂移
- **不做 Feishu SDK / 直连 HTTP / WebSocket 等替代 `lark-cli` 的实现**——架构红线
- **不引外部 LLM SDK / 不用 HTTP client 直连 LLM endpoint**——Roostery 二进制对任意 LLM provider 0 binding；需要 LLM 能力（daily-recap 等）通过 §4.8 `Summarizer` trait 委托给用户已装的 agent runtime CLI。架构红线，与上一条并列
- **不做 GUI / TUI**——CLI-only
- **不做 PyO3 互操作**——Python 与 Rust 不共存，到 Phase 7 整体切换
- **不做"自建非飞书前端"**——portable-by-default req 承诺数据形态可移植，但 Roostery 自身不附带其他 view（社区扩展点）
- **README 改写**单独走（brainstorm v0.x-direction 已列为显式任务），由 `legacy-removal` feature 收尾

### 代码-文档对齐原则

本 roadmap 列的"port from Python X.py"**不是 1:1 机械翻译**。Python baseline 从未严格对齐 vendor-neutral / portable-by-default 等愿景，是"上次的实现"不是"应该的实现"。每条子 feature 实现时：

- **优先读 req / 本 roadmap §4 接口契约 / 相关 arch doc** 确认 WHAT 该做
- Python 源码作 reference 理解 current behavior，不作 ground truth
- 发现"Python 这么做但文档说应该那样" → 按文档做，差异记观察项
- 发现"文档过时反而代码对" → 回 `cs-req update` / `cs-roadmap update` 改文档再继续，不在 feature 里偷偷按代码做

## 3. 模块拆分（概设）

按职责聚类成 8 个模块（A-H）。每个模块对应一组 Rust source 文件，跟 planning 文档 §3 目录结构对齐但更粗粒度（planning 是文件级，这里是模块级）。

```
Roostery Rust
├── 模块 A · 基础工具 (foundations)
├── 模块 B · 本地审计 (journal)
├── 模块 C · 飞书 syscall (lark_cli + shim binary)
├── 模块 D · 本地配置与安装 (config / hooks / onboarding)
├── 模块 E · Dispatcher (hook event → runner)
├── 模块 F · Bot Bridge (agent run → 飞书 task / IM)
├── 模块 G · Reporting (daily report + LLM summary)
└── 模块 H · Base Index (Feishu Base 索引层)
```

### 模块 A · 基础工具
- **职责**：纯数据操作，无 I/O，无 async。包含 schema 常量（`SCHEMA_VERSION`）、`redact`（敏感字段脱敏）、`remoterefs`（从 stdout 抽 `doc_token` 等）。所有上层模块的基础
- **承载的子 feature**：`rust-scaffold`、`core-redact`、`core-remoterefs`
- **触碰的现有代码**：全新（Python 版 `src/roostery/redact.py` + `remoterefs.py` 作行为 reference）

### 模块 B · 本地审计 (Journal)
- **职责**：本地 jsonl audit / replay 基础设施。`JournalEntry` schema 在此定型，**这是 portable-by-default req 的具体兑现载体**——schema 公开、稳定、可移植
- **承载的子 feature**：`journal-core`
- **触碰的现有代码**：全新（Python 版 `src/roostery/journal.py` 作 reference）；schema 重新设计

### 模块 C · 飞书 Syscall
- **职责**：与飞书通信的唯一 sanctioned 通道。包含 `LarkRunner` trait + 默认 subprocess 实现 + `roostery smoke` 子命令 + `lark-cli` shim 二进制（PATH-prefix shim 透传 + 写 journal）
- **承载的子 feature**：`lark-cli-wrapper`、`roostery-smoke`、`lark-cli-shim`
- **触碰的现有代码**：全新（Python 版 `lark_cli.py` + `smoke.py` + `shim.py` 作 reference）

### 模块 D · 本地配置与安装
- **职责**：bootstrap `~/.roostery/`（自 journal-core 起；env 覆盖 `ROOSTERY_HOME`）、merge Stop hooks 进 `~/.claude/settings.json` / `~/.codex/hooks.json`、装 shim、识别 agent runtime、嵌入 Stop hook 脚本模板。用户面对的 `roostery init` 入口
- **承载的子 feature**：`config-yaml`、`hooks-merge`、`roostery-init`
- **触碰的现有代码**：全新（Python 版 `config.py` + `hooks_merge.py` + `identity.py` + `agent_detect.py` + `onboarding.py` + `templates/` 作 reference）

### 模块 E · Dispatcher
- **职责**：本地执行桥。event → 规则匹配 → trace/budget gate → runner → emit。是 runtime-neutral req 的具体执行机制（不感知具体哪家 runtime，只通过 `Runner` trait 调度）
- **承载的子 feature**：`dispatcher-trace-budget`、`dispatcher-rules`、`dispatcher-runners`、`dispatcher-loop`
- **触碰的现有代码**：全新（Python 版 `src/roostery/dispatcher/*` + `runner_registry.py` + `event_bridge.py` 作 reference）

### 模块 F · Bot Bridge
- **职责**：把 agent run 映射成飞书 task card + step stream + IM thread。**agent-work-in-feishu req 的最直接兑现层**。包含 task_writer 主路径 + IM 兜底 + 角色 / 中转 / HITL 路由
- **承载的子 feature**：`bot-task-writer`、`bot-stop-hook`、`bot-bridge-cluster`
- **触碰的现有代码**：全新（Python 版 `task_writer.py` + `stop_hook.py` + `bot_*.py` + `hitl_router.py` 作 reference）

### 模块 G · Reporting
- **职责**：日报功能 —— git log 多仓聚合 + 摘要生成（**复用 §4.3 `Runner` trait + `RunnerRegistry`，不走 `dispatcher::fire` 事件流**：daily-recap 直接 `registry.find(kind).run(synthetic_event, trace, args)` + 自管 `BudgetGuard` 跨进程锁 + 自写 `JournalEntry source="daily_recap"`；理由：dispatcher.fire 是 hook-event 分发 API 返 trace 摘要不返业务输出，daily-recap 是 one-shot string-return call 语义不同，强行复用 fire 要把它改成 RPC API 不值——见 `2026-05-19-report-recap-engine` design §0 D5 + codex review 留痕）+ 写飞书 docx + Base 记录。**Roostery 自身不引外部 LLM SDK / 不用 reqwest 直连 LLM endpoint**，0 LLM client import 在任何模块——LLM 调用是用户已装 agent CLI 子进程的"副作用"，不是 Roostery 自身的 capability。Cargo feature flag `daily-report` 控制（默认开）
- **承载的子 feature**：`report-recap-engine`、`report-daily`
- **触碰的现有代码**：全新（Python 版 `git_log.py` + `llm_summary.py` + `daily_report.py` + `record_writer.py` 作 reference；Python 版 `llm_summary.py` 直接调外部 SDK 的做法不照搬——Rust 版走 dispatcher 复用，0 LLM client 依赖）

### 模块 H · Base Index
- **职责**：Feishu Base 作为索引层（**非** source of truth）。包含 Base config / indexer / intent_router
- **承载的子 feature**：`base-indexer`
- **触碰的现有代码**：全新（Python 版 `base_config.py` + `base_indexer.py` + `base_intent_router.py` 作 reference）

## 4. 模块间接口契约 / 共享协议（架构层详设）

下面 7 个契约是 feature-design 的**硬约束输入**——单 feature 实现不允许擅自违反，要改先回 `cs-roadmap update`。

**契约演化记录机制（ADR-lite）**：任何对 §4.x 的修订必须在该 § 末尾追加一条记录，固定格式：

> **{YYYY-MM-DD}**（trigger feature `{feature-slug}`，commit `{hash}`）：{change summary 一句}. **理由**：{rationale 一句}. **受影响 caller**：{count} ({list or "0 (首个实现者)"})。

这是首个走 lark-cli-wrapper 落地的轻量机制——避免 0 下游受影响的小型契约修订也要起 cs-roadmap full workflow，又对未来 maintainer 透明可审计。结论性 / 重大契约改动（多 caller 受影响 / 语义变更）仍走 cs-roadmap update。

### 4.1 `LarkRunner` trait

**方向**：模块 E / F / G / H → 模块 C
**形式**：Rust trait

```rust
#[async_trait::async_trait]
pub trait LarkRunner: Send + Sync {
    /// 最简调用形态。args[0] 是 lark-cli 子命令（如 "im"）。
    async fn run(&self, args: &[&str]) -> Result<serde_json::Value, LarkError> {
        self.run_with_options(args, RunOptions::default()).await
    }

    /// 高级场景：自定义 timeout / stdin / profile。
    async fn run_with_options(
        &self,
        args: &[&str],
        opts: RunOptions,
    ) -> Result<serde_json::Value, LarkError>;
}

#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub timeout: Option<std::time::Duration>,  // None = 用实现的默认值
    pub stdin: Option<String>,
    pub profile: Option<String>,                // lark-cli --profile global flag
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LarkError {
    #[error("failed to spawn lark-cli at {path:?}: {source}")]
    Spawn {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("lark-cli exited {exit_code} (body code {body_code:?}): {message}")]
    NonZeroExit {
        exit_code: i32,
        body_code: Option<i64>,    // 解出来的飞书业务码（含 transient codes）
        message: String,
        stdout: String,
        stderr: String,
    },

    #[error("lark-cli stdout is not valid JSON: {source}")]
    OutputParse {
        #[source]
        source: serde_json::Error,
        stdout: String,
    },

    #[error("lark-cli timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
}

impl LarkError {
    /// 提示给 caller 的"是否值得重试"——本契约自身不重试，retry 策略归 dispatcher。
    pub fn retriable(&self) -> bool {
        matches!(self,
            Self::Timeout { .. }
            | Self::NonZeroExit { exit_code: 124, .. }
            | Self::NonZeroExit { body_code: Some(99991663 | 99991664), .. }
        )
    }
}
```

**约束**：
- 所有走向飞书的调用必须通过这个 trait——不允许直接 `tokio::process::Command::spawn` 调 `lark-cli`
- `args` 第一个元素是 lark-cli 子命令（如 `"im"`），后续是参数；调用方不传 `lark-cli` 本身的路径
- 返回的 `serde_json::Value` 是 lark-cli stdout 解析后的 JSON Value，调用方负责按 schema 抽字段
- 实现写 `JournalEntry`（见 §4.2）通过 **`Journaled<R: LarkRunner>` 装饰器** 完成（解耦 subprocess 实现与 journal 写入）；mock / 直接 LarkCli 不带 journal 行为，需要时显式 wrap
- `LarkError` 是 `#[non_exhaustive]` rich enum——caller 必须用 `match` 显式处理或 `_ =>`；新增变体不破坏二进制兼容
- `LarkError::retriable()` 是函数（match 表达式），不是字段——避免构造时 retriable 与 variant 数据不一致

**契约演化记录**：
- **2026-05-16**（lark-cli-wrapper feature design 阶段）：原 struct + C-style discriminator + flat fields 形态升级为 `#[non_exhaustive]` rich enum + thiserror，每变体携带各自数据；retry 策略明确归 dispatcher（caller 通过 `LarkError::retriable()` 拿提示）；新增 `run_with_options` 第二 method + `RunOptions` 应对 timeout / stdin / profile；新增 `Journaled<R>` 装饰器约定（解耦 journal 写入与 subprocess 实现）。理由：lark-cli-wrapper 是首个实现者 0 下游 caller 受影响，趁 cheap 把 Rust 错误类型 idiom 拉满

### 4.2 `JournalEntry` schema（**portable-by-default 的契约载体**）

**方向**：写者 = 模块 C / E / F；读者 = replay 工具 / 用户 / 社区第三方 view
**设计原则**：本 schema 是 portable-by-default req 的公开契约，**重新设计而非继承 Python 版 jsonl**——Python 版 schema 作 reference 帮助识别有用字段，不约束 Rust 版结构选择。一旦 Phase 1 落地，schema_version=1 成为对外承诺

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JournalEntry {
    pub schema_version: u32,         // 当前 = 1；任何不兼容改动必须 bump 并保留旧版兼容反序列化
    pub event_id: String,            // ULID / UUID v4
    pub trace_id: Option<String>,
    pub parent_event_id: Option<String>,
    pub depth: u32,                  // loop 保护用
    pub ts: chrono::DateTime<chrono::Utc>,
    pub source: String,              // "shim" | "dispatcher" | "task_writer" | "stop_hook" | ...
    pub action: String,              // "lark-cli:im_messages_send" | "runner:cc_headless" | "task:append_steps" | ...
    pub params: serde_json::Value,   // 调用参数，经 redact 模块脱敏后写入
    pub result: JournalResult,
    pub duration_ms: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "outcome")]
pub enum JournalResult {
    Ok { value: serde_json::Value },
    Err { kind: String, message: String },
}
```

**约束**：
- **schema_version 是公开契约**——破坏性改动需 bump version + 兼容旧版反序列化 + roadmap update 评估 portable-by-default 影响
- 每行独立合法 JSON（jsonl 而非 JSON array），用户能用 `head -1 | jq` 抽样
- `params` 写入前必须过 `redact` 模块脱敏；用户对最终内容敏感性自负（见 portable-by-default 边界）
- 时间戳必须 UTC（避免 timezone 混乱）

### 4.3 `Runner` trait

**方向**：模块 E (dispatcher) → 具体 runner 实现
**形式**：Rust trait

```rust
#[async_trait::async_trait]
pub trait Runner: Send + Sync {
    fn kind(&self) -> &'static str;  // "cc_headless" / "codex_exec" / "gemini_headless" / "noop"
    async fn run(&self, event: &HookEvent, ctx: &TraceContext, budget: &BudgetGate)
        -> Result<RunOutcome, RunnerError>;
}

pub struct RunOutcome {
    pub status: RunnerStatus,  // Success | Failed | Skipped (budget exhausted)
    pub stdout: String,
    pub stderr: String,
    pub emitted_events: Vec<HookEvent>,  // 该 runner 触发的下游事件（可为空）
}
```

**约束**：
- runner 的 `kind()` 是 Config 中 `runners: { <kind>: {...} }` 的 key，必须唯一
- runner 必须在执行前调 `budget.try_consume()`，超额返回 `Skipped`
- runner 必须把所有跟飞书的交互走 `LarkRunner` trait（不允许绕过）
- runner 必须为子进程 / IO 设置 timeout

### 4.4 `HookEvent` schema

**方向**：写者 = 模块 D（hook 脚本）、模块 E（dispatcher 自触发）；读者 = 模块 E
**形式**：JSON over stdin / 文件

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HookEvent {
    pub schema_version: u32,         // 当前 = 1
    pub hook_source: String,         // "claude-code-stop" | "codex-stop" | "gemini-stop" | "cron" | ...
    pub session_id: String,
    pub workspace: PathBuf,
    pub trigger_meta: serde_json::Value,  // runtime-specific payload，dispatcher 透传
    pub trace: Option<TraceContext>,      // 内部 dispatcher → dispatcher 跨层传递时填
}
```

**约束**：
- 外部 hook（CC / Codex 等 runtime 触发的）`trace` 必为 `None`，由 dispatcher 在 fire 时分配新 trace_id
- 模块 D 的 stop hook 脚本（embedded template）负责把 runtime-specific 输出拼成这个 schema 喂给 `roostery dispatcher fire`

### 4.5 `TraceContext`

**方向**：贯穿 E（dispatcher loop）→ F（bot bridge）→ C（lark_cli wrapper）
**形式**：Rust struct

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TraceContext {
    pub trace_id: String,
    pub parent_event_id: Option<String>,
    pub depth: u32,                  // 起始为 0，每层 +1
    pub max_depth: u32,              // 配置上限，超出 dispatcher 拒绝执行
}
```

**约束**：
- 每条 `JournalEntry` 必须带能追溯到 `TraceContext` 的字段（`trace_id` + `parent_event_id` + `depth`）
- dispatcher loop 在分发到 runner 前必须把 depth +1，超出 max_depth 直接拒绝
- bot bridge / lark_cli wrapper 接收到 trace 必须传递不能丢——这是 loop 保护和 replay 重建依赖链的依据

### 4.6 Config schema（`~/.roostery/config.yaml`）

**方向**：写者 = 模块 D（roostery init）；读者 = 所有模块
**形式**：YAML

```yaml
schema_version: 1
identity:
  user_id: string                  # 飞书 open_id 或自定义
  default_chat_id: string          # 默认任务挂载的群 chat_id
  default_task_app_token: string   # 默认 Base app_token（任务列表）
runners:
  cc_headless:
    enabled: bool
    cli_path: string               # claude-code 可执行路径
    extra_args: [string]
  codex_exec: { ... }
  gemini_headless: { ... }
budgets:
  default:
    max_calls: u32                 # 单 trace 链最多调用次数
    max_cost_usd: f64
trace:
  max_depth: u32                   # 默认 8
journal:
  dir: string                      # 默认 ~/.roostery/journal/（journal-core 已落地）
  rotation: "daily" | "size:{MB}" | "never"
```

**约束**：
- `schema_version` bump 需 roadmap update 评估存量 config 兼容性
- 顶层字段缺失时使用编译期默认值（不让 `roostery init` 写过的 config 因新增字段而失效）
- runner 配置为开放结构——加新 runner kind 只需在 `runners` 加一个子键，不动 schema 顶层

### 4.7 模板嵌入约定

**方向**：模块 D（onboarding）→ 用户文件系统
**形式**：`include_str!` 编译期嵌入 + 写到 `~/.claude/settings.json` / `~/.codex/hooks.json` / `~/.roostery/scripts/`

```rust
pub const STOP_HOOK_AGENT_NOTIFY_SH: &str = include_str!("templates/agent_stop_notify.sh");
pub const CC_STOP_HOOK_JSON: &str = include_str!("templates/cc_stop_hook.json");
pub const CODEX_STOP_HOOK_JSON: &str = include_str!("templates/codex_stop_hook.json");
```

**约束**：
- 所有 template 必须编译期嵌入（不在运行时去 disk 找）—— 这是"单二进制 self-contained"的承诺
- 模板修改后必须有 golden file 对比测试（Python 版输出做 baseline，Rust 版必须 byte-for-byte 一致，除非文档另有规定）
- 加新 runtime 的模板（Gemini stop hook 等）走 hooks-merge 现有机制扩展，不引入新嵌入路径

## 5. 子 feature 清单

21 条，按依赖序排列。每条对应 items.yaml 一个条目。"Phase" 标签便于跟原 planning 文档对照（保留学习目标 + 技术决策依据）。每条还标了**主要支持的 req**（不是 frontmatter 关联，是给 reader 一眼看出"这条 feature 为兑现哪份愿景而做"）。

### Module A · 基础工具

1. **`rust-scaffold`** — Cargo workspace + Python 整体归档进 `legacy/python/` + CI 骨架（`cargo fmt --check` / `clippy -D warnings` / `cargo test`）
   - 所属模块：A（含 Python 归档作业，跨模块）
   - 依赖：无
   - 状态：**done**（2026-05-15）
   - 主要支持的 req：—（基础设施）
   - 对应 feature：`2026-05-15-rust-scaffold`（commit `511dce3` / CI run #25912520438 全绿）
   - 备注：Phase 0；完成后 `cargo run -- --version` 输出 `roostery 0.0.0 (rust)`；删 `pyproject.toml` / `src/roostery.egg-info` / `index.js` / `package.json`（占位放弃，npm + PyPI namespace 已 reserved）

2. **`core-redact`** — `redact` 模块，敏感字段脱敏
   - 所属模块：A
   - 依赖：`rust-scaffold`
   - 状态：**done**（2026-05-15）
   - 主要支持的 req：`portable-by-default`（脱敏是 journal 敏感处理基础）
   - 对应 feature：`2026-05-15-core-redact`（commit `1e392e5` / CI run #25914996799 全绿）
   - 备注：Phase 1；Python 版作 reference，SENSITIVE_KEYS 11 个（Python 7 + 扩展 4：password/secret/cookie/private_key）

3. **`core-remoterefs`** — `remoterefs` 模块，JSON walk + match-dispatch 从 lark-cli stdout 抽 9 个 newtype token（MessageId / DocToken / FolderToken / RecordId / ChatId / AppToken / WikiToken / TaskId / ThreadId）
   - 所属模块：A
   - 依赖：`rust-scaffold`
   - 状态：**done**（feature `2026-05-16-core-remoterefs`，commit `4714683`）
   - 主要支持的 req：**`portable-by-default`**（让 journal entry 携带远端 token 便于审计/检索）
   - 备注：Phase 1；newtype + `#[serde(transparent)]` 隔离 9 种 token 类型（Python parity 4 + 业界 3 + Phase 5 必需 2 = 9）；JSON walk + match-dispatch 取代 Python `Dict[str, Optional[str]]` 弱类型；walk 深度上限 64；首匹配赢按 BTreeMap 字典序

### Module B · 本地审计

4. **`journal-core`** — Journal 模块 + `JournalEntry` schema（§4.2）首次落地
   - 所属模块：B
   - 依赖：`rust-scaffold`、`core-redact`
   - 状态：**done**（feature `2026-05-15-journal-core`，commit `b9ac5be`）
   - 主要支持的 req：**`portable-by-default`**（核心契约载体）
   - 备注：Phase 1；schema 重新设计不继承 Python 版 jsonl 格式；schema_version=1 已落地为对外承诺；同步迁移 `~/.feishu_hub/` → `~/.roostery/`、`FEISHU_HUB_HOME` → `ROOSTERY_HOME`（一次性切断，无双读期）；Phase 1 仅 daily rotation + 同步 API + 写入侧（read/replay 留后续 phase）

### Module C · 飞书 Syscall

5. **`lark-cli-wrapper`** — `LarkRunner` trait（§4.1 已升级 rich enum + thiserror）+ LarkCli subprocess + MockLarkRunner + Journaled<R> 装饰器
   - 所属模块：C
   - 依赖：`rust-scaffold`、`journal-core`
   - 状态：**done**（feature `2026-05-16-lark-cli-wrapper`，commit `cc44dfa`）
   - 主要支持的 req：`agent-work-in-feishu`（飞书通信基础）、`portable-by-default`（每次调用经 Journaled 写 journal）
   - 备注：Phase 2；项目首次引入 tokio + async + subprocess；首次走档 2 子目录组织；首次走"契约演化记录段"ADR-lite 路径修订 roadmap §4.1（rich enum + retriable() method + RunOptions builder + Journaled 装饰器分离）；Mock 默认 public 供下游 feature 测试用

6. **`roostery-smoke`** — `roostery smoke` 子命令，跑验证矩阵
   - 所属模块：C
   - 依赖：`lark-cli-wrapper`
   - 状态：**done**（feature `2026-05-17-roostery-smoke`）
   - 主要支持的 req：—（验证基础设施）
   - 备注：Phase 2；6 个 probe 跟 Python 版命令矩阵 1:1 复刻（im / docs / drive）；本机 2026-05-17 实测 lark-cli 1.0.29 全过；引 clap 4 derive 作为项目首个 CLI 解析器（main.rs 重写为 subcommand 模式，`--version` 锁定 `roostery 0.0.0 (rust)`）；smoke 不走 LarkRunner trait（raw bytes 检 "Dry Run" marker vs buffered Value parse JSON 同 shim 决定）；公开 `ensure_ready() -> Result<(), SmokeError>` 给 init / daily_report 当升级 gate；状态文件 `~/.roostery/state/smoke.json` schema_version=1 含 `lark_cli_version` 字段助升级漂移诊断；atomic write `.tmp` + rename

7. **`lark-cli-shim`** — `bin/shim` 独立二进制，PATH-prefix shim 透传真 `lark-cli` 并写 journal
   - 所属模块：C
   - 依赖：`lark-cli-wrapper`、`journal-core`
   - 状态：**done**（feature `2026-05-17-lark-cli-shim`）
   - 主要支持的 req：`portable-by-default`（所有飞书操作必经 journal）
   - 备注：Phase 2；shim 是 `~/.local/bin/lark-cli` 装机点；agent runtime 调 lark-cli 时被 shim 透明拦截；streaming bytes 模型（std::thread + std::process + 2 pump thread + head buffer 64 KiB/16 KiB），不引 tokio 不调 LarkRunner trait（I/O 语义不同）；interactive 三段式（TTY/verb `["auth"]`/flag）走 `CommandExt::exec()` 直通；anti-recursion 用 canonicalize；`ROOSTERY_REAL_LARK_CLI` env 必填，`ROOSTERY_NOJOURNAL=1` 写 skipped 标记 entry

### Module D · 本地配置与安装

8. **`config-yaml`** — Config schema（§4.6）读写 + 默认值 + 升级路径
   - 所属模块：D
   - 依赖：`rust-scaffold`
   - 状态：**done**（feature `2026-05-17-config-yaml`）
   - 主要支持的 req：`agent-work-in-feishu`（用户身份 / 默认群配置）
   - 备注：Phase 3；schema 1:1 落 roadmap §4.6（6 顶层节，全 `#[serde(default)]`）；YAML 库 `serde_yml = "0.0.12"`（`serde_yaml` maintained fork）；`runners` 走开放 `BTreeMap<String, serde_yml::Value>`，runner 强类型化推到 Phase 4 dispatcher-runners；config 不读 env override，各模块自管；4 公开 fn `load` / `load_from` / `save` / `save_to`，atomic `.tmp` + rename；`SchemaVersionMismatch` 错误变体留 v2 升级钩子；纯 lib 扩展，main.rs 无变更

9. **`hooks-merge`** — JSON 深合并，CC / Codex Stop hook 注入 `~/.claude/settings.json` / `~/.codex/hooks.json` — **done**（feature `2026-05-18-hooks-merge`）
   - 所属模块：D
   - 依赖：`config-yaml`
   - 状态：planned
   - 主要支持的 req：`runtime-neutral`（接入多 runtime 的入口）
   - 备注：Phase 3；Python 版输出做 golden file，Rust 版必须 byte-for-byte 一致（除非文档另有规定）

10. **`roostery-init`** — `roostery init` 子命令 + identity / agent_detect / 模板嵌入（§4.7） — **done**（feature `2026-05-18-roostery-init`）
    - 所属模块：D
    - 依赖：`hooks-merge`、`lark-cli-shim`
    - 状态：**done**（feature `2026-05-18-roostery-init`）
    - 主要支持的 req：**`agent-work-in-feishu`**（B 用户首次装机入口）
    - 备注：Phase 3 收尾 feature；3 个新模块 `identity` / `agent_detect` / `onboarding` + 顺手扩 `AgentKind::Gemini` + Gemini stop hook 模板；shim 装 `~/.local/bin/lark-cli`（sha2 hash 比对幂等）；shell rc marker block（`# >>> roostery >>>` / `# <<< roostery <<<`）幂等 patch + `~/.roostery/env` 写 `ROOSTERY_REAL_LARK_CLI`；smoke gate 守门失败零文件副作用；identity 走 `LarkRunner` trait 异步反映 lark-cli profile，失败不阻塞装机；onboarding 模块本期只做 installer，**不**创建 welcome task（推 Phase 5 `bot-stop-hook`）；本 feature 完成 = 陌生开发者跑通"装机 + 配 hook + 装 shim"链路，但 E2E 出 task 仍要等 Phase 5

### Module E · Dispatcher

11. **`dispatcher-trace-budget`** — Trace（§4.5）+ Budget gate 模块；持久化 `~/.roostery/state/budget.json` — **done**（feature `2026-05-18-dispatcher-trace-budget`）
    - 所属模块：E
    - 依赖：`rust-scaffold`、`journal-core`、`config-yaml`
    - 状态：**done**（feature `2026-05-18-dispatcher-trace-budget`）
    - 主要支持的 req：**`runtime-neutral`**（loop 保护是中立 dispatcher 的前提）
    - 备注：Phase 4 起步 feature；3 独立 gate 模块 `trace` / `budget` / `runaway`（互不引用，dispatcher-loop 上层 caller 串场景）；`TraceContext`（depth+max_depth 守门，env 跨 process 传播）+ `BudgetState`（roadmap §4.6 default 单 bucket + f64 USD + 跨日 rollover + atomic 持久化）+ `RunawayTracker`（事后兜底，window/threshold 内存滑动窗口）；Cargo.toml 0 新增依赖；`TraceId` newtype 与 `business-identifier-newtype` decision 一致；本 feature 完成 = caller 装弹就绪，dispatcher 还不会跑（等 dispatcher-loop 收尾 feature）

12. **`dispatcher-rules`** — Rules 模块 + YAML 规则反序列化 + 匹配逻辑 — **done**（feature `2026-05-18-dispatcher-rules`）
    - 所属模块：E
    - 依赖：`dispatcher-trace-budget`
    - 状态：**done**（feature `2026-05-18-dispatcher-rules`）
    - 主要支持的 req：`runtime-neutral`
    - 备注：Phase 4 第 2 子 feature；**rule schema 全新设计**（拒绝 Python parity）；HookEvent §4.4 schema 同步落地；3 维 AND MVP（hook_source eq + workspace_glob fnmatch 经 globset + trigger_meta 点路径 eq）；Action opaque `{runner, args: Value}` 透传；无模板引擎；first-match-wins；self-event 防自激（`dispatcher.` / `roostery.` 前缀短路）；本期不交付 dispatch 编排（dispatcher-loop feature）

13. **`dispatcher-runners`** — `Runner` trait（§4.3）+ `cc_headless` / `codex_exec` / `gemini_headless` / `noop` 默认实现 + `runner_registry` — **done**（feature `2026-05-18-dispatcher-runners`）
    - 所属模块：E
    - 依赖：`dispatcher-trace-budget`、`lark-cli-wrapper`
    - 状态：**done**（feature `2026-05-18-dispatcher-runners`）
    - 主要支持的 req：**`runtime-neutral`**（这是中立接入的执行点）
    - 备注：Phase 4 第 3 子 feature；首发实际落地 = `noop` + `cc_headless`（`codex_exec` / `gemini_headless` 完全不出现，推后到真有需求时新增 feature 加 impl，与 runtime-neutral req 边界"首发不保证所有 runtime 同等支持"一致）。**与 §4.3 偏离两项**（user 拍板）：(a) `Runner::run` 不收 `&BudgetGate` 参数（budget gate 编排留给 dispatcher-loop）；(b) `RunOutcome` 加 `cost_usd: Option<f64>` 字段。建议后续 `cs-roadmap update` 把 §4.3 原契约改齐。Runner trait async + 内部 `tokio::task::spawn_blocking` 包同步 `std::process::Command`（不引 `tokio::process` 避 ETXTBSY race）；env sanitize 经 `SAFE_ENV_FORWARD` const allowlist；CC JSON 解析容错——失败仍返 Success cost None

14. **`dispatcher-loop`** — Loop + Event bridge + `roostery dispatcher` 子命令（fire / replay / test-rule）— **done**（feature `2026-05-18-dispatcher-loop`）
    - 所属模块：E（**Phase 4 / Module E 整体完成**）
    - 依赖：`dispatcher-rules`、`dispatcher-runners`
    - 状态：**done**（feature `2026-05-18-dispatcher-loop`）
    - 主要支持的 req：`runtime-neutral`（dispatcher 编排层最终兑现层）
    - 备注：Phase 4 收尾子 feature。串 trace / budget / runaway / rules / runners / journal 6 上游模块为 `HookEvent in → DispatchOutcome out + journal` 主链路，暴露 `roostery dispatcher fire / replay / test-rule` 三 CLI 子命令；fire 主链路 5 gate 顺序 (trace.check_depth → rules.matches → budget.check_or_raise(0.0) → runaway.record + check → registry.find → runner.run → budget.consume + save → journal.append)；emitted_events 链式自触发 BFS 队列 + `trace.max_depth` 守深度 + `DEFAULT_MAX_FANOUT=16` 守 width 双守门；replay live 真跑 runner + 分配新 trace_id (不沿用) + journal trigger_meta.replay_of 关联源；unknown runner kind → `StepStatus::Skipped`；fire 始终 exit 0 + journal 落档失败原因 (hook 调用方对错误不敏感)；replay / test-rule 走 DispatchError exit 1。dispatcher.rs 不消费 LarkRunner / 不直接 spawn / 不引 reqwest（红线 N1-N3）。0 新增 Cargo 依赖。journal 模块加 `load_by_trace_id` read API（首次有 read path 启动）。Accepted 2026-05-18（commit `7fd07fc`，CI run #26018196490 全绿）

### Module F · Bot Bridge

15. **`bot-task-writer`** — Task writer：创建 Feishu task + append step stream + session cache — **done**（feature `2026-05-18-bot-task-writer`）
    - 所属模块：F
    - 依赖：`lark-cli-wrapper`、`journal-core`、`config-yaml`
    - 状态：**done**（feature `2026-05-18-bot-task-writer`）
    - 主要支持的 req：**`agent-work-in-feishu`**（任务卡片这条主路径）
    - 备注：Phase 5 Module F 第 1 子 feature。3 pub async fn 纯库 API（`create_task` / `append_steps` / `get_or_create_for_session`）；session_cache JSON schema_version=1 持久化 `~/.roostery/state/session_tasks/`；host suffix 多机区分；safe_filename 路径跳出防御；`append_steps --yes` 架构红线显式破例已归档 ARCHITECTURE.md §6 第 18 条。**首次让 Rust 业务模块真消费 LarkRunner trait 做生产飞书 IO**（dispatcher 不走飞书；smoke / shim 走独立 I/O 路径）。0 新增 Cargo 依赖。顺手 fix：onboarding shell_kind_detect_* 4 测试加 ENV_LOCK 串行化（attention.md 规约）。Accepted 2026-05-18（commit `083b8ba`，CI run #26021247942 全绿）

16. **`bot-stop-hook`** — Stop hook 入口 + 反向调用 CLI；双 CLI surface 共享 bot::push 核心 lib fn
    - 所属模块：F
    - 依赖：`bot-task-writer`、`dispatcher-loop`、`roostery-init`
    - 状态：**done**（feature `2026-05-18-bot-stop-hook`）
    - 主要支持的 req：**`agent-work-in-feishu`** ⭐（升级 draft → current 触发点；E2E 闭环 + 反向调用双维兑现）
    - 备注：Phase 5 Module F 第 2 子 feature = **minimal-loop closing = 🎯 0.1.0 release 触发判据达成**。**双 CLI surface**：(1) `roostery bot stop-hook`（被动 hook，Rust 端原生 stdin JSON + transcript jsonl tail，替代 Python shell→python bridge）；(2) `roostery bot push`（反向调用，让任意 agent / 脚本 / cron / CI 主动推飞书，`--agent / --session / --summary | --summary-stdin / --description / --assignee-open-id / --strict / --json / --no-im-fallback`）。共享 `bot_stop_hook::push` 核心 lib fn。**Rust 红利显式发挥**：类型化 `PushRequest` builder + `PushOutcome` 结构化 `--json` 输出（v1 稳定契约）+ `--strict` opt-in 真实 exit code + blake3 稳态 idempotency key（修 SipHash 启动种子随机化 bug）+ structured tracing。receive_id 三层链 `env ROOSTERY_NOTIFY_TO > identity::current > config.identity.user_id`（不引入新字段）；task_writer 失败走 IM 兜底（`--no-im-fallback` opt-out）；不调 dispatcher fire（与 dispatcher 双独立顶层入口）；`templates/agent_stop_notify.sh` 47 行 → 10 行极简 wrapper。**S10.5 顺手修**：4 mod 各自 ENV_LOCK 跨模块 race（多 mod 改 ROOSTERY_HOME 时 race）→ 统一切 `crate::paths::TEST_ENV_LOCK` 共享锁，attention.md 修订规约。新增 dep：`blake3 = "1"`。Accepted 2026-05-18（commit `220c7b0`，CI run #26030808131 三 job 全绿，420 tests 全过）

17. **`bot-bridge-cluster`** — bot_role / bot_runner / bot_bridge / bot_relay_task / hitl_router 合并实现
    - 所属模块：F
    - 依赖：`bot-stop-hook`
    - 状态：**done**
    - 主要支持的 req：`agent-work-in-feishu`（协作 / HITL 路由扩展，兑现用户故事第 4 条 IM 群里围观 / 接续 / 反向操控）
    - 备注：**Phase 5 Module F 第 3 子 feature**。Rust 期 1 子目录 `crates/roostery/src/bot_bridge/`（9 文件 ~3 200 行 + 集成测 634 行）替代 Python 5 模块。**关键决策（design D1-D12，user approved 2026-05-19）**：(a) 5 Python 模块合并 1 Rust 子目录按职责切分；(b) HITL 信号通道走进程内 tokio oneshot channel **不落盘 sentinel**——Rust 期重新设计而非 Python 1:1 翻译的代表案例；(c) runner 调用走 `dispatcher::runners::Runner` trait + Registry 复用 Phase 4；(d) IM 事件源走 `lark-cli im_messages_subscribe` 子进程 NDJSON tail + 指数退避重连 cap 60s；(e) `ActiveRunnerRegistry` 命名避让 `dispatcher::runners::RunnerRegistry`；(f) `ADJUST_MAX = 1` const Python parity；(g) 每 BotRole 独立 cache 目录 `~/.roostery/state/bot_chats/{app_id}/`。**明确不做**：base_intent_router / `--parallel` flag / cleanup_orphans / 用户自定义 abort-adjust 关键词 / `relay_writer_app_id` 跨身份 profile 转向 / POSIX `os::kill` / SIGTERM。Accepted 2026-05-19（commits `3ccd2a3..dbd2470`，570 tests 全过，clippy/fmt 全绿）。新增公开承诺：`BOTS_SCHEMA_VERSION = 1` / `BOT_CHAT_CACHE_SCHEMA_VERSION = 1`。req `agent-work-in-feishu` status 保持 `current`（本 feature 兑现协作维度，加变更日志条目）

### Module G · Reporting

18. **`report-recap-engine`** — `git_log` 多仓聚合 + `roostery daily-recap` 子命令 + 直接调 `RunnerRegistry::find(kind).run` 委托用户已装 agent CLI（**复用 §4.3 Runner trait 但不走 dispatcher::fire**；自管 BudgetGuard + JournalEntry；Roostery 不直连 LLM）
    - 所属模块：G
    - 依赖：`rust-scaffold`、`dispatcher-runners`（Runner trait + Registry）、`dispatcher-trace-budget`（TraceContext + BudgetGuard）、`journal-core`、`core-redact`、`config-yaml`
    - 状态：**done**（2026-05-19）
    - 主要支持的 req：[`daily-dev-recap`](../../requirements/daily-dev-recap.md) + [`runtime-neutral`](../../requirements/runtime-neutral.md)（Runner trait 复用本身就是 runtime-neutral 的又一次兑现——daily-recap 不绑某家 LLM provider，跟着用户选的 Runner 走）
    - 对应 feature：`2026-05-19-report-recap-engine`（533 lib tests + 8 integration tests + clippy 双 build mode 全绿）
    - 备注：Phase 6；Cargo feature flag `daily-report` 默认开；`cargo build --no-default-features` 剥掉的是 daily-recap 子命令注册 + module 编译（**不涉及 `reqwest` 边界**——reqwest 整个不被 Roostery 引）。**不走 dispatcher::fire**——daily-recap 是 one-shot string-return call，跟 hook event 分发语义不同；详见 design §0 D5 + codex review 留痕。原 slug `report-git-llm` 于 2026-05-19 改名。Accepted 2026-05-19，4 轮 codex review + 5 个 design 版本（v1→v5）。ARCHITECTURE.md §3 Module G + §5.5 + §5.10（新增）+ §6.3 三处归并。req `daily-dev-recap` 仍保 draft——待 `report-daily` 把 `RecapOutcome` 真写到飞书 docx + Base 后一并升 current为 `report-recap-engine`

19. **`report-daily`** — Daily report 主流程 + `record_writer`（写飞书 docx + Base 记录）
    - 所属模块：G
    - 依赖：`report-recap-engine`、`lark-cli-wrapper`、`journal-core`、`config-yaml`
    - 状态：planned
    - 主要支持的 req：[`daily-dev-recap`](../../requirements/daily-dev-recap.md)
    - 备注：Phase 6

### Module H · Base Index

20. **`base-indexer`** — `base_config` + `base_indexer` + `base_intent_router`
    - 所属模块：H
    - 依赖：`lark-cli-wrapper`、`config-yaml`
    - 状态：planned
    - 主要支持的 req：—（索引层扩展，未在三份 draft req 明确覆盖）
    - 备注：Phase 7；Base 是索引层 **非** source of truth，实现以文档为准

### 收尾

21. **`legacy-removal`** — 删 `legacy/python/`、重写 README（兑现 brainstorm "leading with user-why" 任务）、更新 CLAUDE.md（去 Python-specific 内容）、`Cargo.toml` 准备 crates.io 元信息、GitHub Actions release workflow
    - 所属模块：跨模块（项目级清理）
    - 依赖：`bot-bridge-cluster`、`report-daily`、`base-indexer`
    - 状态：planned
    - 主要支持的 req：—（项目维护）
    - 备注：Phase 7；这一条对应 brainstorm 的"README 改写"显式任务 + 项目终态切换

**最小闭环 ✅ 达成**：第 16 条 `bot-stop-hook` **2026-05-18 落地**（commit `220c7b0`，CI run #26030808131 全绿）→ CC headless 会话能在飞书 app 看到新任务 + step，**且**任意 agent / 脚本可通过 `roostery bot push` 主动推送。这一刻 = "Rust 可用"判据成立 = **0.1.0 release 触发点达成**（参 brainstorm v0.x-direction "首个 release"决议）。

**0.1.0 ✅ 已 tag**（2026-05-18，feature `2026-05-18-release-0.1.0-prep` accept）：version bump `0.0.0 → 0.1.0` + `git tag v0.1.0`（本地，push 时机用户自决）+ README 五段重写 user-why leading + CHANGELOG.md 起步 + workspace Cargo.toml metadata 7 字段补齐（0.2.0 crates.io 预热）。**0.1.x 不上 crates.io** — `cargo publish --dry-run` 推到 0.2.0 前夜独立 feature。后续条目（17 / 18 / 19 / 20 / 21）是 0.1.0 之后到 1.0 之间的扩展。

## 6. 排期思路

**主轴 = 模块依赖 + Rust 学习曲线**：

- **Phase 0-1（A + B）**：scaffold + 三个无 I/O 底层模块（journal / redact / remoterefs）。学 cargo / 模块系统 / 所有权 / 错误处理 / 序列化。这一段是 Rust 入门基础，跑通后心智模型搭起来
- **Phase 2（C）**：lark-cli wrapper + smoke + shim。引入 async / subprocess / trait 抽象。**第一个能跑通"飞书端到端往返"的 milestone**（虽然不是 user-facing 价值）
- **Phase 3（D）**：onboarding 入口。引入路径处理 / YAML / JSON merge / 嵌入资源。**第一个 user-facing 装机 milestone**——陌生开发者能跑 `roostery init` 装好 hook 和 shim（虽然还不能出 task）
- **Phase 4（E）**：dispatcher。引入复杂 trait + enum + 状态机 + 超时。**最难一段**：Python 版 dispatcher 测试覆盖最厚，行为 corner case 多
- **Phase 5（F）**：bot bridge。**关键 milestone ✅**——`bot-stop-hook` 完成 = 最小闭环 = "Rust 可用" = **0.1.0 触发点达成（2026-05-18）**；第 3 子 feature `bot-bridge-cluster` 是 0.1.0 之后的扩展
- **Phase 6（G）**：reporting。0.1.0 后扩展，引入 Cargo feature flag + reqwest 边界控制
- **Phase 7（H + cleanup）**：Base + 终态切换

**为什么这样拆**：每个 Phase 引入有限新 Rust 概念 + 单独可交付（cargo test 通过 / 有可演示功能）。Phase 之间不并行，循序推进——既是依赖约束，也是学习节奏约束。

**第一条 `rust-scaffold` 不是闭环点但是必要起点**：所有 feature 都依赖它。完成后仓库结构切到 Rust，旧 Python 归档，CI 跑起来。

**最小闭环选 `bot-stop-hook`**（第 16 条）而非其他：它是"用户看得见 agent 在飞书出 task" 的最早 milestone。在它之前虽然有 smoke（Phase 2）和 init（Phase 3）的中间 milestone，但都不构成 B 用户验收意义上的"能用"。

**节奏不绑外部 deadline**：brainstorm v0.x-direction 已敲定"不急 / 占位好好做"。每个 Phase 完成后允许停滞数周再开下一段。仓库始终保持 `cargo build` + `cargo test` 全绿。

## 7. 观察项

起草过程中发现的范围外问题，留给用户决定：

- **`bot-bridge-cluster`（第 17 条）行为文档薄弱**——Python 版 `bot_role.py` / `bot_bridge.py` / `bot_relay_task.py` / `hitl_router.py` 互相耦合较紧但 ARCHITECTURE.md 对这一组只有"参与 M3.B 主路径"级别的简述。Phase 5 临近时建议先走 `cs-arch new` 把这一组现状梳理出来（甚至可能要起 draft req 描述"HITL 路由 / agent 角色管理"这一独立能力），避免 feature-design 时无契约可依
- **代码-文档失配登记**（已写入 §2 代码-文档对齐原则）——建议在 `.codestable/compound/code-doc-misalignment-log.md`（learning 类型）累积失配发现，每条 feature-design / acceptance 顺手补，便于后期 review 一并刷新文档
- **CLAUDE.md / ARCHITECTURE.md 时间线问题**（已在 brainstorm v0.x-direction open questions）——它们现在描述 Python 是 going-forward，但本 roadmap Phase 0 第一步就是归档。`rust-scaffold` feature 落地时一并改这两个文档比较自然，但不强制写进 feature scope
- **Phase 6 推迟项 crates.io 发布时机**——等 Phase 6 临近再决，不在本 roadmap 范围（brainstorm v0.x-direction 已记）。**LLM provider 默认家**议题已于 2026-05-19 决定：不挑 provider，通过 §4.8 `Summarizer` trait 复用用户已装 agent runtime（见 §8 变更日志）
- **`base-indexer` 缺 req 覆盖**——这条目前所支持的 req 列"—"。承载的能力（Base 索引）值不值得起 draft req？建议在 Phase 7 启动前补；也可能届时决定砍掉作为 Roostery 范围。（`report-recap-engine` / `report-daily` 已于 2026-05-19 由 `daily-dev-recap` req 覆盖）
- **本次 §3 Module G 改写 / §2 加红线的下游同步**——以下三处现在和 roadmap 表述不一致，需要后续单独走对应 skill 刷新（**不阻塞** Phase 6 启动）：
    - `.codestable/attention.md` "LLM provider 客户端只允许在 `llm_summary.rs` import" → 走 `cs-note` 改为"不允许任何模块 import 外部 LLM client / 用 reqwest 打 LLM endpoint"
    - `.codestable/architecture/ARCHITECTURE.md` §5 第 3 / 第 5 条 LLM 红线表述同上 → 走 `cs-arch update`
    - `.codestable/requirements/daily-dev-recap.md` 边界第 5 条 "LLM 显式破例" 措辞可软化——Roostery 自身不破例，是用户跟自己 agent 厂商既有关系决定的 → 走 `cs-req update`（或留到 feature acceptance 一并处理）
- **原 `planning/2026-05-15-rust-rewrite.md` 的去向**——本 roadmap 起草时已在该文件头部加注"phase 拆解已迁至本 roadmap"，原文件保留作 Rust 学习目标 + 技术选型档案。本 roadmap 与原 planning 文档维持分工关系：本 roadmap 主管 phase → feature → 接口契约，planning 文档主管学习目标 + 技术选型决策

## 8. 变更日志

- **2026-05-19**：架构方向修订——LLM 摘要从"Roostery 直连外部 LLM（`llm_summary.rs` 是唯一 import 白名单）"改为"**复用已有 §4.3 `Runner` trait + Phase 4 dispatcher 路径**：daily-recap 构造 HookEvent → dispatcher.fire → rules.yaml 路由到用户已装 agent CLI → RunOutcome.stdout 即 summary"。**理由**：Roostery 定位为 vendor-neutral agent broker，自己持有 LLM key / 引 LLM SDK 与该定位冲突；而 `dispatcher-runners` 落地说明已经把"dispatcher 调度 runner"明确为 `runtime-neutral` req 的核心兑现层——daily-recap 走同一路径既复用已建好的全部基础设施（trace / budget / runaway / rules / journal），又把 daily-recap 变成 runtime-neutral 的又一次兑现。**初版误判记录**：起草时本想新增 §4.8 `Summarizer` trait 作为独立契约，user 反馈"通过 agent 调用 roostery 调度其他 agent" 是已有架构后，核对 `runtime-neutral.md` 边界第 1 条与 dispatcher-runners 变更日志确认 dispatcher 复用是正解，§4.8 不引入。**改动**：§2 加全局红线"不引外部 LLM SDK / 不用 HTTP client 直连 LLM endpoint"；§3 Module G 描述改写（去掉"唯一允许 import LLM client"红线，明确走 `dispatcher::fire` + Runner trait）；item 18 slug `report-git-llm → report-recap-engine` + 依赖加 `dispatcher-loop` / `config-yaml` + 备注重写（feature flag 不再涉 reqwest 边界 / 标 hook_source 避 SELF_EVENT_PREFIXES）；item 19 depends_on `report-git-llm → report-recap-engine`；§7 观察项删 "LLM provider 默认家"项 + 加"下游同步三处"项（attention / arch / req）。**受影响 caller**：0（item 18 仍 planned，无 in-progress / done feature 使用旧契约）
