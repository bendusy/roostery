---
doc_type: roadmap
slug: rust-rewrite
status: active
created: 2026-05-15
last_reviewed: 2026-05-15
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
- **职责**：日报功能 —— git log 多仓聚合 + LLM 摘要 + 写飞书 docx + Base 记录。`llm_summary.rs` 是唯一允许 import 外部 LLM client 的模块（架构红线）。Cargo feature flag 控制，默认开
- **承载的子 feature**：`report-git-llm`、`report-daily`
- **触碰的现有代码**：全新（Python 版 `git_log.py` + `llm_summary.py` + `daily_report.py` + `record_writer.py` 作 reference）

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
   - 状态：planned
   - 主要支持的 req：—（验证基础设施）
   - 备注：Phase 2；6 个 probe 跟 Python 版命令矩阵对齐（im / docs / drive），probe 内容以 Python 版 reference 调整为符合最新 lark-cli 1.0.28 schema

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
   - 状态：planned
   - 主要支持的 req：`agent-work-in-feishu`（用户身份 / 默认群配置）
   - 备注：Phase 3；schema 重新设计；落 `~/.roostery/config.yaml`；用户已有 `~/.feishu_hub/config.yaml`（Python 版）的迁移不在本 feature 范围（Python 不维护）

9. **`hooks-merge`** — JSON 深合并，CC / Codex Stop hook 注入 `~/.claude/settings.json` / `~/.codex/hooks.json`
   - 所属模块：D
   - 依赖：`config-yaml`
   - 状态：planned
   - 主要支持的 req：`runtime-neutral`（接入多 runtime 的入口）
   - 备注：Phase 3；Python 版输出做 golden file，Rust 版必须 byte-for-byte 一致（除非文档另有规定）

10. **`roostery-init`** — `roostery init` 子命令 + identity / agent_detect / 模板嵌入（§4.7）
    - 所属模块：D
    - 依赖：`hooks-merge`、`lark-cli-shim`
    - 状态：planned
    - 主要支持的 req：**`agent-work-in-feishu`**（B 用户首次装机入口）
    - 备注：Phase 3；完成后陌生开发者第一次能跑通"装机 + 配 hook + 装 shim"链路（但还不能 E2E 出 task，那要等 Phase 5）

### Module E · Dispatcher

11. **`dispatcher-trace-budget`** — Trace（§4.5）+ Budget gate 模块；持久化 `~/.roostery/state/budget.json`
    - 所属模块：E
    - 依赖：`rust-scaffold`、`journal-core`、`config-yaml`
    - 状态：planned
    - 主要支持的 req：**`runtime-neutral`**（loop 保护是中立 dispatcher 的前提）
    - 备注：Phase 4

12. **`dispatcher-rules`** — Rules 模块 + YAML 规则反序列化 + 匹配逻辑
    - 所属模块：E
    - 依赖：`dispatcher-trace-budget`
    - 状态：planned
    - 主要支持的 req：`runtime-neutral`
    - 备注：Phase 4；rule schema 重新设计

13. **`dispatcher-runners`** — `Runner` trait（§4.3）+ `cc_headless` / `codex_exec` / `gemini_headless` / `noop` 默认实现 + `runner_registry`
    - 所属模块：E
    - 依赖：`dispatcher-trace-budget`、`lark-cli-wrapper`
    - 状态：planned
    - 主要支持的 req：**`runtime-neutral`**（这是中立接入的执行点）
    - 备注：Phase 4；首发实现 `cc_headless` 即可工作；其他 runner 实现可为 stub，跟 runtime-neutral req 边界"首发不保证所有 runtime 同等支持"一致

14. **`dispatcher-loop`** — Loop + Event bridge + `roostery dispatcher` 子命令（fire / replay / test-rule）
    - 所属模块：E
    - 依赖：`dispatcher-rules`、`dispatcher-runners`
    - 状态：planned
    - 主要支持的 req：`runtime-neutral`
    - 备注：Phase 4；Python 版 7 个 `test_dispatcher_*.py` 的核心 case 作 reference 翻译为 Rust 集成测试，case 行为以文档为准（特别是 trace / budget / 错误处理）

### Module F · Bot Bridge

15. **`bot-task-writer`** — Task writer：创建 Feishu task + append step stream + session cache
    - 所属模块：F
    - 依赖：`lark-cli-wrapper`、`journal-core`、`config-yaml`
    - 状态：planned
    - 主要支持的 req：**`agent-work-in-feishu`**（任务卡片这条主路径）
    - 备注：Phase 5；session cache 本地 schema 重新设计，不继承 Python 版

16. **`bot-stop-hook`** — Stop hook 入口：替代 Python shell→python bridge，原生处理 stdin JSON event，task_writer 主路径 + IM 兜底
    - 所属模块：F
    - 依赖：`bot-task-writer`、`dispatcher-loop`、`roostery-init`
    - 状态：planned
    - 主要支持的 req：**`agent-work-in-feishu`**（E2E 闭环点）
    - 备注：Phase 5；**🎯 minimal_loop = true**——完成后真跑一次 CC headless 会话能在飞书 app 看到新任务 + step。0.1.0 release 触发判据

17. **`bot-bridge-cluster`** — bot_role / bot_runner / bot_bridge / bot_relay_task / hitl_router 合并实现
    - 所属模块：F
    - 依赖：`bot-stop-hook`
    - 状态：planned
    - 主要支持的 req：`agent-work-in-feishu`（协作 / HITL 路由扩展）
    - 备注：Phase 5；这一组在 Python 版相互耦合较紧，Rust 版可借机重新拆分但**行为以文档为准**——目前文档对这一组的具体行为描述薄弱，本 feature 启动前可能需要 `cs-arch new` 把现状梳清（见 §7 观察项）

### Module G · Reporting

18. **`report-git-llm`** — `git_log` 多仓聚合 + `llm_summary`（**唯一**允许 `reqwest` 直连外部 LLM 的模块，架构红线）
    - 所属模块：G
    - 依赖：`rust-scaffold`
    - 状态：planned
    - 主要支持的 req：—（产品扩展能力，未在三份 draft req 明确覆盖）
    - 备注：Phase 6；Cargo feature flag `daily-report` 默认开；`cargo build --no-default-features` 必须能剥掉 `reqwest` 验证边界

19. **`report-daily`** — Daily report 主流程 + `record_writer`（写飞书 docx + Base 记录）
    - 所属模块：G
    - 依赖：`report-git-llm`、`lark-cli-wrapper`、`journal-core`、`config-yaml`
    - 状态：planned
    - 主要支持的 req：—（产品扩展能力）
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

**最小闭环**：第 16 条 `bot-stop-hook` 做完后 CC headless 会话能在飞书 app 看到新任务 + 1 条 step。这一刻 = "Rust 可用"判据成立 = 0.1.0 release 触发点（参 brainstorm v0.x-direction "首个 release"决议）。后续条目（17 / 18 / 19 / 20 / 21）是 0.1.0 之后到 1.0 之间的扩展。

## 6. 排期思路

**主轴 = 模块依赖 + Rust 学习曲线**：

- **Phase 0-1（A + B）**：scaffold + 三个无 I/O 底层模块（journal / redact / remoterefs）。学 cargo / 模块系统 / 所有权 / 错误处理 / 序列化。这一段是 Rust 入门基础，跑通后心智模型搭起来
- **Phase 2（C）**：lark-cli wrapper + smoke + shim。引入 async / subprocess / trait 抽象。**第一个能跑通"飞书端到端往返"的 milestone**（虽然不是 user-facing 价值）
- **Phase 3（D）**：onboarding 入口。引入路径处理 / YAML / JSON merge / 嵌入资源。**第一个 user-facing 装机 milestone**——陌生开发者能跑 `roostery init` 装好 hook 和 shim（虽然还不能出 task）
- **Phase 4（E）**：dispatcher。引入复杂 trait + enum + 状态机 + 超时。**最难一段**：Python 版 dispatcher 测试覆盖最厚，行为 corner case 多
- **Phase 5（F）**：bot bridge。**关键 milestone**——`bot-stop-hook` 完成 = 最小闭环 = "Rust 可用" = 0.1.0 触发
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
- **Phase 6 推迟项**——LLM provider 默认家、crates.io 发布时机：等 Phase 6 临近再决，不在本 roadmap 范围（brainstorm v0.x-direction 已记）
- **`report-git-llm` / `report-daily` / `base-indexer` 缺 req 覆盖**——这三条目前所支持的 req 列"—"。它们承载的能力（日报生成 / Base 索引）值不值得起 draft req？建议在 Phase 6 / Phase 7 启动前补 req；也可能届时决定砍掉作为 Roostery 范围
- **原 `planning/2026-05-15-rust-rewrite.md` 的去向**——本 roadmap 起草时已在该文件头部加注"phase 拆解已迁至本 roadmap"，原文件保留作 Rust 学习目标 + 技术选型档案。本 roadmap 与原 planning 文档维持分工关系：本 roadmap 主管 phase → feature → 接口契约，planning 文档主管学习目标 + 技术选型决策

## 8. 变更日志

_（new 模式，本节为空；update 时记录改动）_
