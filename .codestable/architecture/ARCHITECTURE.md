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
| `LarkRunner` trait | Rust 期 lark-cli wrapper 的抽象接口（async + Send + Sync），下游所有模块依赖 trait 而非具体 struct；已落地 `crates/roostery/src/lark_cli/`（feature `2026-05-16-lark-cli-wrapper`，commit `cc44dfa`）。三实现：`LarkCli`（subprocess）/ `MockLarkRunner`（测试替身）/ `Journaled<R>`（journal 装饰器）。`run` 和 `run_with_options` 双 method 见 roadmap §4.1 |
| `LarkError` | `#[non_exhaustive]` rich enum + thiserror，4 变体 `Spawn` / `NonZeroExit` / `OutputParse` / `Timeout`，每变体携带专有数据；`retriable()` method（非字段）。caller 必经 `match` + `_ =>` 处理（外部 crate E0004 守护）。见 roadmap §4.1 |
| Dispatcher | 本地事件 → 规则匹配 → runner 执行的桥接层（Module E，Phase 4） |
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

- 子 feature：**`config-yaml`（done）** / **`hooks-merge`（done）** / `roostery-init`

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
| 4.6 | Config schema | D 写 → 所有读 | Phase 3 — **已落地**（feature `2026-05-17-config-yaml`） |
| 4.7 | 模板嵌入约定 | D → 用户文件系统 | Phase 3 — **已落地**（feature `2026-05-18-hooks-merge`） |

## 5. 关键架构决定

1. **vendor-neutral 桥而非 SDK**。Roostery 不替代 agent runtime，也不替代 Feishu，它只做转换 + 审计
2. **Feishu = default view，不是 lock-in**。本地是 cache / audit，journal 是 portable 数据形态——飞书出问题 / 想换前端，能基于 journal 重建（兑现 `portable-by-default` req）
3. **`lark-cli` 是唯一飞书入口**。不允许新增 HTTP client 直连 `open.feishu.cn`
4. **dispatcher hook-agnostic**。新 hook 源（Codex / Gemini / Cursor）通过 `hooks_merge` + 模板嵌入扩展，loop 不感知 provider
5. **`llm_summary` 模块是 LLM provider 集成的唯一白名单**。其他模块保持 vendor-neutral
6. **业务标识符 newtype 隔离**（自 core-remoterefs 起）：对**从飞书侧拿到的、有明确业务语义角色的标识符**（token / id / cursor）一律用 newtype + `#[serde(transparent)]` 隔离类型；不实现互转 `From` impl。Phase 4 dispatcher 的 `TraceId` / `EventId` / `ParentEventId` 是合格候选；**不**适用于"还没成为业务 token 的字符串"（subcommand 名 / 原始 argv / 普通 String 参数）——后者 newtype 化是 noise。详见 `.codestable/compound/` 待归档 convention
7. **shim / smoke 与 LarkRunner 走三条独立 I/O 路径**（自 lark-cli-shim 起，roostery-smoke 进一步验证）：
   - **shim**：streaming bytes 模型 + `std::thread::spawn` + `std::process`（透明 tee 4 KiB chunks 给用户，head buffer 副本写 journal）
   - **smoke**：raw bytes 模型 + 同步 `std::process` + `try_wait` 50ms 轮询（用 stdout 文本检 "Dry Run" marker；不解析 JSON；写 state file 不写 journal）
   - **`LarkRunner`**：buffered Value 模型 + tokio（`wait_with_output` 一次性 collect + `serde_json::Value` parse；调用结果返给 caller）

   三条路径 I/O 语义根本不同（streaming/raw bytes 检文本 vs buffered Value parse JSON），所以**不强行抽公共 trait**；只共享下层 `journal` / `redact` / `remoterefs` / `paths` 模块。下游 read/replay 通过 `JournalEntry.source` 字段（"shim" / "dispatcher" / ...）+ `~/.roostery/state/smoke.json` 状态文件分流

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
9. **`ROOSTERY_AGENT` env 约定**：agent runtime 识别用 `ROOSTERY_AGENT=cc` / `=codex`（Stop hook command 拼前缀），由 stop bridge sh 在 hook fire 时读取传给 `roostery dispatcher fire`；**不沿用** Python `FEISHU_HUB_AGENT`（feature `2026-05-18-hooks-merge` 一次切口径，roadmap items.yaml "除非文档另有规定" 明示偏离）
