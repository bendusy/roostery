---
doc_type: feature-acceptance
feature: 2026-05-17-roostery-smoke
status: passed
date: 2026-05-17
summary: smoke 模块 + roostery smoke 子命令落地——6 条 lark-cli --dry-run probe 矩阵（im/docs/drive）+ ~/.roostery/state/smoke.json 状态快照 + ensure_ready() gate API；clap 4 derive 作为项目首个 CLI 解析器；smoke 直接 std::process 不走 LarkRunner（同 shim raw bytes vs buffered Value 决定）；--version 严格保持 'roostery 0.0.0 (rust)'。19 lib 单测 + 4 集成测试全过；fmt/clippy/test --all/--doc 四命令绿；同步更新 ARCHITECTURE §2 术语 + §3 Module C 子节 + §5 第 7 条扩展 + §6 第 4/5 条兑现 commit；roadmap items + 主文档 status → done。attention.md 候选两条留 acceptance 阶段决议
tags: [phase-2, module-c, smoke, gate, clap, acceptance]
---

# roostery-smoke 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-17
> 关联方案 doc：`.codestable/features/2026-05-17-roostery-smoke/roostery-smoke-design.md`

## 1. 接口契约核对

对照方案第 2.1 节名词层：

**公开 API 接口契约**（`crates/roostery/src/smoke.rs`）逐项核对：

| 接口 | 设计签名 | 代码落点 | 一致 |
|---|---|---|---|
| `PROBE_MATRIX: &[Probe]` 6 条 | im_messages_send / docs_create_v2 / docs_update_overwrite / drive_files_list / drive_create_folder / drive_move | `smoke.rs:28-116` | ✓（name + argv 与 Python `PROBES` 1:1） |
| `struct ProbeResult` 4 字段 | ok + rc + head + reason（后三个 Option） | `smoke.rs:119-128`，`#[serde(skip_serializing_if)]` 完整 | ✓ |
| `struct SmokeReport` 6 字段 `#[non_exhaustive]` | schema_version + binary + lark_cli_version + started_at + all_ok + probes BTreeMap | `smoke.rs:131-140` | ✓ |
| `enum SmokeError` 4 变体 `#[non_exhaustive]` | NeverRun / LastFailed { failed_probes } / StateLoadFailed { source } / BinaryNotFound { path } | `smoke.rs:143-159` | ✓ |
| `pub fn run() -> SmokeReport` | resolve binary → fetch version → 顺序跑 6 probe → save | `smoke.rs:306-333` | ✓ |
| `pub fn ensure_ready() -> Result<(), SmokeError>` | load_last → match all_ok | `smoke.rs:336-348` | ✓ |

**调用示例**核对（design §2.1）：

- caller 调用 `smoke::ensure_ready()` 模式：`smoke.rs:336-348` 签名一致；`Err(e)` 模式经单测 `ensure_ready_never_run` / `ensure_ready_last_failed` / `ensure_ready_state_load_failed` 验证 ✓
- `roostery smoke` 子命令打印 pretty JSON + exit code：`main.rs:33-44`；集成测试 `smoke_all_ok_exits_zero` 断言 stdout JSON parse 通过 ✓
- state file 形态示例 6 字段（schema_version / binary / lark_cli_version / started_at / all_ok / probes）+ probes 子对象按 name 字典序：集成测试持久化 + 单测 `smoke_report_round_trip` 验证 ✓

**main.rs CLI 形态**（clap derive）：

- `#[derive(Parser)]` + `#[command(name="roostery", version=concat!(env!("CARGO_PKG_VERSION"), " (rust)"))]`：`main.rs:5-15` ✓
- `#[derive(Subcommand)]` + `Command::Smoke`：`main.rs:18-22` ✓
- `disable_help_subcommand = true` 避免 `roostery help` 多余子命令：`main.rs:11` ✓
- 实测 `./target/debug/roostery --version` → `roostery 0.0.0 (rust)`（集成 `version_string_locked` 断言）

**名词层"现状 → 变化"**：

- ✓ 新增 `crates/roostery/src/smoke.rs`
- ✓ `paths.rs` 加 `state_dir` + `smoke_state_path`（`paths.rs:28-34`）
- ✓ `lib.rs` 加 `pub mod smoke;`（`lib.rs:9`）
- ✓ `main.rs` 重写为 clap subcommand 模式
- ✓ `Cargo.toml` 加 `clap = { version = "4", features = ["derive"] }`（`Cargo.toml:33`）

**流程图核对**（§2.2 mermaid 主流程图节点 A-Z2）：

- A 取当前时间：`run` 内 `chrono::Utc::now()` ✓
- B resolve_binary：`smoke.rs:257-264` ✓
- C binary exists 隐式在 spawn 失败 NotFound 分支处理：`probe_one` 内 `Err(e) if e.kind() == NotFound`（`smoke.rs:175-182`）✓
- D binary not found → 全 6 条标 ok=false：实现上每条 probe 独立失败，等价 ✓（单测 `run_with_missing_binary_marks_all_failed`）
- E fetch_lark_cli_version：`smoke.rs:266-279` ✓
- F-L for probe / probe_one / classify / probes.insert：`run` 主循环 + `probe_one` ✓
- M all_ok：`probes.values().all(|p| p.ok)` ✓
- N save_report：`smoke.rs:281-291`（.tmp + rename）✓

**ensure_ready 流程图**（§2.2 第二张）：

- B load_last：`smoke.rs:293-303`，区分 NotFound→NeverRun / IO→StateLoadFailed / parse→StateLoadFailed ✓
- C/D/E/F/G 分支：`ensure_ready` 内三分支（Ok / NeverRun / LastFailed）+ load 失败传递 ✓

**无偏离**。

## 2. 行为与决策核对

**需求摘要逐项验证**：

- ✓ smoke 模块 + `roostery smoke` 子命令：落地
- ✓ 6 条 probe 矩阵（im/docs/drive）：`PROBE_MATRIX` 6 条
- ✓ 结果写 `~/.roostery/state/smoke.json`：集成 `smoke_all_ok_exits_zero` 断言 state 文件存在
- ✓ 公开 `ensure_ready() -> Result<(), SmokeError>`：lib export
- ✓ 引入 clap 作为项目首个 CLI 解析器：`Cargo.toml` + `main.rs`
- ✓ smoke 直接 `std::process::Command` 不走 LarkRunner：grep 验证（见下）
- ✓ paths 模块扩 `state_dir()`：`paths.rs:28-30`

**明确不做（§1 + §3.2 反向核对）**：

- [x] `grep -E "use tokio|tokio::|#\[tokio::main\]" smoke.rs` → 无
- [x] `grep "LarkRunner\|LarkCli\|Journaled" smoke.rs` → 无
- [x] `grep "FEISHU_HUB_" smoke.rs` → 无
- [x] `grep "Config\|cfgmod\|toml::" smoke.rs` → 无
- [x] `grep -E "fn retry|retries|backoff" smoke.rs` → 无
- [x] `grep -E "rayon|tokio::spawn|std::thread::spawn" smoke.rs` → 无（仅 `std::thread::sleep` 在 probe_one 50ms 轮询）
- [x] `grep -E "Journal::|journal::" smoke.rs` → 无（smoke 不写 journal，与 design §0.2 一致）
- [x] `grep PROBE_MATRIX smoke.rs` → 5 处（1 常量定义 + 1 run() 内 use + 3 tests 引用名字字符串）
- [x] `grep "smoke" crates/roostery/src/lark_cli/` → 无（单向依赖，lark_cli 不引 smoke）

**关键决策 D1-D12 落地核对**：

| # | 决策 | 代码体现 |
|---|---|---|
| D1 | clap 4 derive | `Cargo.toml:33` + `main.rs:1-22` |
| D2 | 不调 LarkRunner trait | grep 验证 0 命中 |
| D3 | gate 用 lib fn `ensure_ready() -> Result<(), SmokeError>` | `smoke.rs:336-348` |
| D4 | 6 条 probe 直接搬 Python 版 | `PROBE_MATRIX` 与 `legacy/python/src/roostery/smoke.py:24-67` 字符串 1:1 |
| D5 | state file 加 `lark_cli_version` 字段 | `SmokeReport.lark_cli_version: Option<String>` + `fetch_lark_cli_version` 实现 |
| D6 | binary 解析：`ROOSTERY_LARK_CLI_BIN` env > `"lark-cli"` PATH | `resolve_binary` `smoke.rs:257-264`；与 `lark_cli/subprocess.rs:14` 同字符串 |
| D7 | probe timeout 10s/条 + head 截 500 字节 | `PROBE_TIMEOUT_SECS = 10` + `HEAD_BYTES = 500` |
| D8 | "Dry Run" marker + rc==0 才视 ok；unknown flag/command 模式探测 | `probe_one` 分类逻辑 `smoke.rs:234-254` |
| D9 | `SmokeError` thiserror 4 变体 `#[non_exhaustive]` | `smoke.rs:143-159` |
| D10 | paths 模块加 `state_dir` + `smoke_state_path` | `paths.rs:28-34` |
| D11 | atomic write `.tmp` + rename | `save_report` `smoke.rs:281-291`；单测 `save_and_load_round_trip` 断言 `.tmp` 不残留 |
| D12 | smoke 失败不 panic；binary not found 仍写 state file | spawn NotFound 分支返 ProbeResult 而非 panic；`run` 跑完仍 save_report |

**编排层"现状 → 变化"**：装机后 caller（Phase 3 init / Phase 6 daily_report）调 `ensure_ready` → 检 state file → Ok/Err。本 feature 不真正消费 `ensure_ready`（caller 还没起），但 API 已对外公开 ✓

**流程级约束（§2.2 不变量 1-6）**：

| 不变量 | 守护方式 |
|---|---|
| 1 idempotent，每次跑覆盖 state | `save_report` 用 rename 替换；每次 `run()` 重新构造 `SmokeReport` |
| 2 atomic write `.tmp` + rename | `smoke.rs:281-291`；`save_and_load_round_trip` 测试断言 `.tmp` 不残留 |
| 3 顺序固定（BTreeMap 字典序） | `BTreeMap<String, ProbeResult>` 类型保证；`probe_matrix_names_match_python_parity` 测试 |
| 4 binary 未找到不 panic | spawn NotFound 走 `ProbeResult` 而非 `?` 传播；`run_with_missing_binary_marks_all_failed` 测试 |
| 5 单条 probe timeout 10s | `PROBE_TIMEOUT_SECS = 10`；`probe_one_timeout` 测试（200ms timeout vs sleep 30） |
| 6 ensure_ready 区分 NeverRun/LastFailed/StateLoadFailed | `load_last` NotFound 分流；`ensure_ready_*` 4 单测覆盖 |

**挂载点反向核对（§2.3）+ 沙盘推演**：

| # | 挂载点 | grep 验证 |
|---|---|---|
| 1 | `crates/roostery/src/smoke.rs` 存在 | `ls` ✓ |
| 2 | `pub mod smoke;` in lib.rs | `lib.rs:9` ✓ |
| 3 | `PROBE_MATRIX` 含 6 条 probe | 单测 `probe_matrix_has_six_entries` + `probe_matrix_names_match_python_parity` 守护 ✓ |
| 4 | `paths::smoke_state_path()` 返 `~/.roostery/state/smoke.json` | `paths.rs:32-34`；扩展 `env_override_wins` 测试覆盖 ✓ |
| 5 | Cargo.toml 含 clap + main.rs Smoke 子命令 | `Cargo.toml:33` + `main.rs:18-22` ✓ |

**反向核查**：

- `grep -rn "smoke" crates/roostery/src/` → 仅 `lib.rs:9` + `smoke.rs` + `main.rs:32-44` + 测试；无清单外挂入点 ✓
- `grep -rn "ROOSTERY_LARK_CLI_BIN" crates/` → smoke.rs + lark_cli/subprocess.rs（两处使用同字符串，§4.1 已 flag 为后续 refactor 候选）

**拔除沙盘推演**：

- 删 `src/smoke.rs` + `lib.rs` 那行 + `main.rs` Smoke 子命令 + `Cargo.toml` clap 依赖 + `paths.rs` state_dir/smoke_state_path → `cargo build` 仅剩 roostery / shim bin，无残留；lark_cli 模块独立不受影响 ✓

**遗留**：无清单外挂入点漏记。

## 3. 验收场景核对

#### Probe matrix 行为

- [x] **S1.1 Happy**：集成 `smoke_all_ok_exits_zero` 断言 `all_ok=true` + 6 probe entries ✓
- [x] **S1.2 Binary 不存在**：单测 `run_with_missing_binary_marks_all_failed` ✓
- [x] **S1.3 Unknown flag**：单测 `probe_one_unknown_flag`（断言 reason 含 "flag/command mismatch"） ✓
- [x] **S1.4 Timeout**：单测 `probe_one_timeout`（200ms timeout vs sleep 30） ✓
- [x] **S1.5 rc!=0 非已知错误**：单测 `probe_one_unexpected_exit`（exit 5） ✓
- [x] **S1.6 顺序固定**：`BTreeMap<String, ProbeResult>` 类型保证；序列化时字典序；类型签名编译期保证

#### State file 形态

- [x] **S2.1 Schema 6 顶层字段**：单测 `smoke_report_round_trip` + 集成 `smoke_all_ok_exits_zero` 断言 5 关键字段；`ProbeResult` 4 字段 `#[serde(skip_serializing_if)]` 验证（单测 `probe_result_optional_fields_skipped_when_none`） ✓
- [x] **S2.2 Atomic write**：单测 `save_and_load_round_trip` 断言 `.tmp` 不残留 + state file 内容正确 ✓
- [x] **S2.3 schema_version=1**：集成断言 `report["schema_version"] == 1` ✓
- [x] **S2.4 Pretty JSON**：`serde_json::to_vec_pretty` 落地（`save_report:285`）；`main.rs:36` 也用 pretty 打印 stdout
- [x] **S2.5 lark_cli_version 抓取**：`fetch_lark_cli_version` 实现；fixture 不打印版本字段为 None 时仍合规（类型层保证 `Option<String>`）

#### Gate API

- [x] **S3.1 NeverRun**：单测 `ensure_ready_never_run` ✓
- [x] **S3.2 LastFailed**：单测 `ensure_ready_last_failed`（断言 `failed_probes == ["docs_create_v2"]`） ✓
- [x] **S3.3 StateLoadFailed**：单测 `ensure_ready_state_load_failed`（bad JSON） ✓
- [x] **S3.4 Ok**：单测 `ensure_ready_happy` ✓

#### CLI 集成

- [x] **S4.1 全过退 0**：集成 `smoke_all_ok_exits_zero` ✓
- [x] **S4.2 部分失败退 1**：集成 `smoke_partial_failure_exits_one`（im_messages_send 失败）✓
- [x] **S4.3 `--version` 严格匹配**：集成 `version_string_locked` 断言 `== "roostery 0.0.0 (rust)"` ✓
- [x] **S4.4 无参欢迎**：集成 `no_args_prints_welcome` ✓
- [x] **S4.5 `smoke --help`**：clap derive 内置；`./target/debug/roostery smoke --help` 手验通过（clap 类型保证） ✓

#### 模块级

- [x] **S5.1 `cargo test --all`**：125 lib + 4 smoke integration + 12 shim unit + 4 shim integration + 3+4 doc 全绿（新增 ≥10 条 = 14 smoke unit + 4 smoke integration + 1 paths 扩 = 19 新）
- [x] **S5.2 `cargo test --doc`**：全绿
- [x] **S5.3 `cargo clippy --all-targets --all-features -- -D warnings`**：通过
- [x] **S5.4 `cargo fmt --all --check`**：通过
- [x] **S5.5 架构红线**：grep 验证无 LarkRunner/LarkCli/Journaled 命中
- [x] **S5.6 env name 共享**：smoke.rs + lark_cli/subprocess.rs 两处出现 `ROOSTERY_LARK_CLI_BIN`

前端改动：无。

## 4. 术语一致性

对照方案第 0 节 + §2.1 grep：

| 术语 | 代码命中 | 一致 |
|---|---|---|
| Probe | `struct Probe` `smoke.rs:23` | ✓ |
| Probe matrix | `const PROBE_MATRIX` `smoke.rs:28` | ✓ |
| Smoke state | `paths::smoke_state_path` + `SmokeReport` 形态 | ✓ |
| Gate API | `pub fn ensure_ready` `smoke.rs:336` | ✓ |
| `SmokeError` | enum 4 变体 `smoke.rs:145-159` | ✓ |
| `ROOSTERY_LARK_CLI_BIN` | const `ENV_BIN` `smoke.rs:17` + `lark_cli/subprocess.rs:14` 同字符串 | ✓ |
| `roostery smoke` 子命令 | `Command::Smoke` `main.rs:21` | ✓ |

**防冲突**：

- `grep "FEISHU_HUB_" crates/roostery/src/smoke.rs` → 无 ✓
- `grep "LarkRunner" crates/roostery/src/smoke.rs` → 无 ✓
- `grep "smoke" crates/roostery/src/lark_cli/` → 无 ✓

无不一致。

## 5. 架构归并

对照方案第 4 节，实际写入：

- [x] **`ARCHITECTURE.md §2 术语表`** — 加 `Smoke` 词条（PROBE_MATRIX / state file / gate API）
- [x] **`ARCHITECTURE.md §3 Module C`** — 加 smoke 子节描述（6 条 probe / state file schema_version=1 / atomic write / `ensure_ready` gate / 与 LarkRunner / Journaled 的关系）+ 子 feature 列表 smoke 标 done
- [x] **`ARCHITECTURE.md §5 第 7 条**` — 扩展为 "shim / smoke 与 LarkRunner 走两条独立 I/O 路径"（streaming/raw bytes vs buffered Value）
- [x] **`ARCHITECTURE.md §6 第 4/5 条`** — 第 4 条 lark-cli 1.0.28 pin + 第 5 条 smoke gate 加 feature 兑现引用（feature `2026-05-17-roostery-smoke`）
- [x] **`.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`** — roostery-smoke status `in-progress` → `done`
- [x] **`.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §3 第 6 项** — status `planned` → `done` + feature 引用
- [ ] **`.codestable/attention.md`** — 不写入（候选 2 条留 §8 决议）
- [ ] **`.codestable/compound/`** — 无新 decision 候选（clap 引入是 tech-stack 但被 rust-module-organization 决策间接覆盖；不独立归档）

判据自检：未读 design 的人打开 ARCHITECTURE.md §3 Module C smoke 子节 + §5 第 7 条，能知道"系统里现在有 `roostery smoke` 子命令 + state file gate，与 shim / LarkRunner 三条独立路径互不混淆"。

## 6. requirement 回写

- 方案 frontmatter `requirement: null`
- smoke 是验证基础设施（roadmap items.yaml 备注："主要支持的 req：—（验证基础设施）"）
- 不新增用户可感能力（用户视角：smoke 是开发者 / 升级 lark-cli 后才用，不直接服务"用 Roostery 做事"场景）
- 处理方式：**无 requirement 回写**

## 7. roadmap 回写

- 方案 frontmatter `roadmap: rust-rewrite` / `roadmap_item: roostery-smoke`，两字段都有值
- `rust-rewrite-items.yaml` 第 45-51 行 `slug: roostery-smoke` 当前 `status: in-progress` + `feature: 2026-05-17-roostery-smoke`（design 阶段已写入）
- 改 `status: done`，`validate-yaml.py --file` 校验通过
- `rust-rewrite-roadmap.md` §3 第 6 项当前 `状态: planned` → 改 `状态: **done**（feature ...）` + 加补充备注

## 8. attention.md 候选盘点

**候选 1**：lark-cli 版本 pin 1.0.28 → 1.0.29 实测兼容
- 当前 attention.md 写 "`lark-cli` 版本 pin 在 1.0.28"
- 本机 2026-05-17 实测 lark-cli 1.0.29 与 6 条 PROBE_MATRIX 全兼容
- **建议措辞**："`lark-cli` 版本最低 pin 在 1.0.28（`task append_task_steps` timestamp schema 兼容）；1.0.29 已实测兼容（feature `2026-05-17-roostery-smoke` 2026-05-17 跑通 6 条 probe）"

**候选 2**：`ROOSTERY_LARK_CLI_BIN` 是 lark-cli wrapper + smoke 共用的 binary 解析 env
- 两个模块同字符串硬编码（`lark_cli/subprocess.rs:14` + `smoke.rs:17`）
- 下个 feature 改 env name 时容易漏改一处；改 default value 同理
- **建议措辞**："`ROOSTERY_LARK_CLI_BIN` env 当前在两处硬编码（`lark_cli/subprocess.rs::ENV_BIN` + `smoke.rs::ENV_BIN`），改名需同步两处；将来抽公共 `pub const` 由后续 `cs-refactor` 处理"

两条都建议加入 attention.md，但**不擅自写入**——退出后逐条问。

## 9. 遗留

- **后续观察项**（design §4.1 已记）：
  - `ROOSTERY_LARK_CLI_BIN` 抽公共 const（cs-refactor 候选）
  - Phase 3 config-yaml 起来后 binary 解析加 config fallback
  - probe matrix 扩展由 config 驱动
  - lark-cli 1.0.29 升级 attention.md（见 §8 候选 1）
  - `SmokeReport.schema_version=1` 公开承诺（未在 roadmap §4 列接口契约层；read/replay 工具可视为 stable 形态）
- **已知偏差**：
  - design §3 反向核对 `wc -l smoke.rs < 500` 实际 650（产品 349 + 内联单测 ~300）。用户在 implement 阶段确认放过，因超出皆 19 条内联单测覆盖 ProbeResult/SmokeReport/probe_one 5 case/save_load 3 case/run 2 case/ensure_ready 4 case
  - design §3 反向核对 `grep PROBE_MATRIX smoke.rs == 至少 1（常量定义）` 实际 5 处（1 常量 + 1 run() 内 use + 3 tests 引用字符串）—— 设计意图"至少 1 处"已满足，未违反精神
- **顺手发现**：无
