---
doc_type: feature-acceptance
feature: 2026-05-17-config-yaml
status: passed
date: 2026-05-17
summary: config 模块落地——Config 顶层 6 字段（schema_version / identity / runners / budgets / trace / journal）#[non_exhaustive]，runners 走开放 BTreeMap<String, serde_yml::Value>；ConfigError 4 变体；4 公开 fn load / load_from / save / save_to + atomic .tmp + rename；serde_yml 0.0.12（serde_yaml maintained fork）；schema_version=1 公开承诺；config 不读 env override（各模块自管）；缺失 schema_version 默认为 1，不等于 1 → SchemaVersionMismatch。17 lib 单测 + 2 集成测试全过；fmt/clippy/test --all/--doc 四命令绿；同步更新 ARCHITECTURE §2 术语 + §3 Module D 落地 config 子节 + §4.6 标 Phase 3 已落地；roadmap items + 主文档 status → done；agent-work-in-feishu req 加变更日志 + implemented_by。**已知偏离**：impl 阶段 step 1-4 一起写（紧耦合单文件 lib feature），用户在 impl 完成汇报确认放过
tags: [phase-3, module-d, config, acceptance]
---

# config-yaml 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-17
> 关联方案 doc：`.codestable/features/2026-05-17-config-yaml/config-yaml-design.md`

## 1. 接口契约核对

对照方案第 2.1 节名词层：

**公开 API 接口契约**（`crates/roostery/src/config.rs`）逐项核对：

| 接口 | 设计签名 | 代码落点 | 一致 |
|---|---|---|---|
| `Config` #[non_exhaustive] 6 字段全 #[serde(default)] | schema_version / identity / runners / budgets / trace / journal | `config.rs:25-54` | ✓ |
| `Identity` #[non_exhaustive] 3 String 字段 | user_id / default_chat_id / default_task_app_token | `config.rs:56-65` | ✓ |
| `Budgets` #[non_exhaustive] { default: BudgetCfg } | 单 default 子节 | `config.rs:67-72` | ✓ |
| `BudgetCfg` #[non_exhaustive] | max_calls=100, max_cost_usd=1.0 默认 | `config.rs:74-97` | ✓（D 默认值锁定）|
| `TraceConfig` #[non_exhaustive] | max_depth: u32 = 8（roadmap §4.6 钉死）| `config.rs:99-116` | ✓ |
| `JournalConfig` #[non_exhaustive] | dir: PathBuf 调用时刻快照, rotation: String = "daily" | `config.rs:118-141` | ✓ |
| `ConfigError` #[non_exhaustive] thiserror 4 变体 | LoadFailed / ParseFailed / SaveFailed / SchemaVersionMismatch | `config.rs:144-158` | ✓ |
| `pub fn load() -> Result<Config, ConfigError>` | 默认路径 | `config.rs:162-164` | ✓ |
| `pub fn load_from(&Path) -> Result<Config, ConfigError>` | 指定路径 | `config.rs:167-182` | ✓ |
| `pub fn save(&Config) -> Result<(), ConfigError>` | 默认路径 atomic | `config.rs:185-187` | ✓ |
| `pub fn save_to(&Config, &Path) -> Result<(), ConfigError>` | 指定路径 atomic | `config.rs:190-199` | ✓ |
| `SCHEMA_VERSION_CURRENT: u32 = 1` 模块私有 | const | `config.rs:19` | ✓ |
| `paths::config_path()` | 返 `~/.roostery/config.yaml` | `paths.rs:36-38` | ✓ |

**调用示例**核对（design §2.1）：

- caller 调用 `config::save(&cfg)` + `config::load()` 模式：集成测试 `roostery_home_override_drives_default_paths` 覆盖完整 round-trip ✓
- config.yaml 形态：与 design 示例 7 顶层节（schema_version / identity / runners / budgets / trace / journal）match；集成测试 `full_yaml_with_runners_round_trips` 加载完整 schema ✓

**名词层"现状 → 变化"**：

- ✓ 新增 `crates/roostery/src/config.rs`
- ✓ `paths.rs:36-38` 加 `config_path()`
- ✓ `lib.rs:4` 加 `pub mod config;`
- ✓ `Cargo.toml:34` 加 `serde_yml = "0.0.12"`

**流程图核对**（§2.2 两张 mermaid）：

- `load` 流程：A read → file NotFound → default / ok bytes → serde_yml::from_slice → 校 schema_version → Ok/Err 5 个节点在 `config.rs:167-182` 1:1 落地 ✓
- `save` 流程：A → ensure parent dir → to_string → write tmp → rename → Ok 在 `config.rs:190-199` 1:1 落地 ✓

**无接口偏离**。

## 2. 行为与决策核对

**需求摘要逐项验证**：

- ✓ Config schema strongly typed identity/budgets/trace/journal + runners 开放 BTreeMap
- ✓ load/save 4 公开 fn
- ✓ schema_version=1 公开承诺
- ✓ 缺字段编译期默认值（`#[serde(default)]` per-field）
- ✓ missing schema_version 当 1 + mismatch 报 SchemaVersionMismatch
- ✓ serde_yml YAML lib
- ✓ config 加载不动 env（各模块自管）
- ✓ 纯 lib 扩展无 CLI 变更（main.rs 不动）

**明确不做（§1 + §3.2 反向核对）grep**：

- [x] `grep -E "use tokio|tokio::|async fn" config.rs` → 无
- [x] `grep "FEISHU_HUB_\|FEISHU_NOTIFY_TO" config.rs` → 无
- [x] `grep -E "notify_receive_id|daily_report|bitable" config.rs` → 无（不沿用 Python schema）
- [x] `grep -E "env::var|env_os" config.rs` → 无
- [x] `grep "LarkRunner\|Journaled\|smoke::" config.rs` → 无
- [x] `grep -E "fn migrate|fn upgrade" config.rs` → 无
- [x] `wc -l config.rs` = 418；产品 LOC = 200 < 500 ✓

**关键决策 D1-D12 落地**：

| # | 决策 | 代码体现 |
|---|---|---|
| D1 | serde_yml 0.0.12 | `Cargo.toml:34` + use 内化于 schema struct serde derive |
| D2 | runners 开放 BTreeMap<String, serde_yml::Value> | `config.rs:34` |
| D3 | config 不动 env override | `grep env::var` → 无 |
| D4 | schema_version：缺=1 / =1 OK / ≠1 mismatch | `default_schema_version` + `load_from` 校验逻辑 + 4 单测全过 |
| D5 | atomic write .tmp + rename | `save_to` `config.rs:194-197` |
| D6 | 4 公开 fn 双路径形态 | load / load_from / save / save_to 4 fn |
| D7 | ConfigError 4 变体 #[non_exhaustive] | `config.rs:144-158` |
| D8 | 顶层 6 字段全 #[serde(default)] | 类型签名守护 |
| D9 | 5 子 struct 全 #[non_exhaustive] | 集成测试 `cfg.budgets.default.max_calls = 42` 用字段赋值绕过 struct literal 限制验证 |
| D10 | SCHEMA_VERSION_CURRENT 模块私有 | `const` 非 `pub const` |
| D11 | paths::config_path() | `paths.rs:36-38` |
| D12 | 纯 lib 扩展无 CLI 变更 | `git diff main.rs` 无修改 |

**编排层"现状 → 变化"**：本 feature 落地后 caller（未来 init / dispatcher）可 `config::save(&cfg)` 写 + `config::load()` 读。本 feature 不真正消费 config，API 公开 ✓

**流程级约束（§2.2 不变量 1-6）**：

| 不变量 | 守护方式 |
|---|---|
| 1 load 文件不存在 → Ok(default) | `load_from` NotFound 分支 + 单测 `load_from_missing_returns_default` |
| 2 atomic save | `.tmp` + `fs::rename` + 单测 `save_to_creates_parent_dir` 断言 `.tmp` 不残留 |
| 3 schema_version 缺失隐式=1 | `default_schema_version` fn + 单测 `load_from_missing_schema_version_treated_as_one` |
| 4 load 不调 env / save 不调 redact | grep 验证 |
| 5 default 可 save + load round-trip 等价 | 单测 `save_default_round_trip` |
| 6 runners 子键开放任意 | 单测 `runners_open_structure`（3 键混合）+ 集成测试 |

**挂载点反向核对（§2.3）+ 沙盘推演**：

| # | 挂载点 | grep 验证 |
|---|---|---|
| 1 | `crates/roostery/src/config.rs` 存在 | `ls` ✓ |
| 2 | `pub mod config;` in lib.rs | `lib.rs:4` ✓ |
| 3 | `paths::config_path()` 返 `~/.roostery/config.yaml` | `paths.rs:36-38` + 扩 `env_override_wins` 单测 ✓ |
| 4 | Cargo.toml 含 `serde_yml` 依赖 | `Cargo.toml:34` ✓ |
| 5 | `SCHEMA_VERSION_CURRENT = 1` | `config.rs:19` ✓ |

**反向核查**：`grep -rn "config" crates/roostery/src/` → `lib.rs:4` + `config.rs` + `paths.rs:36-38` + 测试文件；无清单外挂入点 ✓

**拔除沙盘推演**：删 `src/config.rs` + `lib.rs:4` + `paths.rs:36-38` + `Cargo.toml serde_yml` → `cargo build` 仍能编译（其他模块未消费 config）；无残留 ✓

**遗留**：无清单外挂入点漏记。

## 3. 验收场景核对

#### Schema 反序列化 S1.1-S1.5

- [x] **S1.1 空 YAML**：`empty_yaml_is_default_config`（`"{}"` → `Config::default()`） ✓
- [x] **S1.2 完整 YAML**：集成测试 `full_yaml_with_runners_round_trips`（7 顶层节全反序列化） ✓
- [x] **S1.3 部分字段**：`partial_yaml_fills_defaults`（仅 identity.user_id 其余 default） ✓
- [x] **S1.4 未知字段忽略**：`unknown_fields_ignored`（mystery_field: 42） ✓
- [x] **S1.5 runners 任意子键**：`runners_open_structure`（cc_headless + codex_exec + custom_runner_v3） ✓

#### schema_version 校验 S2.1-S2.4

- [x] **S2.1 缺失=1**：`load_from_missing_schema_version_treated_as_one` ✓
- [x] **S2.2 =1 OK**：`load_from_valid_yaml` ✓
- [x] **S2.3 =2 mismatch**：`load_from_schema_version_mismatch`（found=2 expected=1） ✓
- [x] **S2.4 =0 mismatch**：`load_from_schema_version_zero_mismatch`（不容忍降级） ✓

#### load API S3.1-S3.5

- [x] **S3.1 missing file**：`load_from_missing_returns_default` ✓
- [x] **S3.2 valid**：`load_from_valid_yaml` ✓
- [x] **S3.3 parse fail**：`load_from_bad_yaml_returns_parse_failed`（损坏 YAML 串） ✓
- [x] **S3.4 IO error**：类型签名保证（`std::fs::read` Err 非 NotFound 走 `LoadFailed`）—— 编译期由 match arm 守护，未独立 case 测（permission denied 需要 sudo 难自动化）
- [x] **S3.5 ROOSTERY_HOME 联动**：集成测试 `roostery_home_override_drives_default_paths`（save 到 home/config.yaml + load 拿回 cfg） ✓

#### save API S4.1-S4.5

- [x] **S4.1 parent dir 自动**：`save_to_creates_parent_dir`（嵌套 a/b/c 路径） ✓
- [x] **S4.2 round-trip**：`save_then_load_round_trip`（identity.user_id 自定义） ✓
- [x] **S4.3 default round-trip**：`save_default_round_trip` ✓
- [x] **S4.4 atomic .tmp**：`save_to_creates_parent_dir` 断言 `.tmp` 不残留 ✓
- [x] **S4.5 YAML 可读**：`saved_yaml_is_human_readable`（断言 `schema_version: 1` / `trace:` / `max_depth: 8` 关键字） ✓

#### 模块级 S5.1-S5.4

- [x] `cargo test --all`：142 lib + 12 shim unit + 4 shim integration + 4 smoke integration + **2 config integration**（新增）+ 3+4 doc 全绿
- [x] `cargo test --doc`：全绿
- [x] `cargo clippy --all-targets --all-features -- -D warnings`：通过
- [x] `cargo fmt --all --check`：通过

前端改动：无。

## 4. 术语一致性

对照方案第 0 节 + §2.1 grep：

| 术语 | 代码命中 | 一致 |
|---|---|---|
| Config | `config.rs:25` 顶层 struct | ✓ |
| Identity | `config.rs:56` | ✓ |
| Budgets / BudgetCfg | `config.rs:67-97` | ✓ |
| TraceConfig | `config.rs:99-116` | ✓ |
| JournalConfig | `config.rs:118-141` | ✓ |
| `RunnerCfgRaw` | design §0 提及作为概念名；代码实际用 `BTreeMap<String, serde_yml::Value>` 不引入 `RunnerCfgRaw` 类型 | **轻偏差**：design §0 用 `RunnerCfgRaw` 形容词描述含义，未要求落 Rust 类型名；BTreeMap value type 直接是 `serde_yml::Value` 更直白。**不修代码**（design §0 措辞已足够说明） |
| ConfigError | `config.rs:144-158` | ✓ |
| `SCHEMA_VERSION_CURRENT` | `config.rs:19` | ✓ |
| `paths::config_path` | `paths.rs:36-38` | ✓ |

**防冲突**：`grep "Config" crates/roostery/src/ --include="*.rs" -l` → 仅 `config.rs` / 模块导出 / 测试引用；无另一处 `Config` 类型冲突 ✓

`RunnerCfgRaw` 轻偏差不影响读者——design §0 是概念描述（"runner 子节占位类型"），代码用 `BTreeMap<String, serde_yml::Value>` 直接对应该概念。

## 5. 架构归并

对照方案第 4 节，实际写入：

- [x] **`ARCHITECTURE.md §2 术语表`** — 加 `Config` 词条
- [x] **`ARCHITECTURE.md §3 Module D`** — Phase 3 第一个 feature 落地，加 config 子节描述（6 顶层字段 / serde_yml / atomic save / schema_version=1 公开承诺 / runners 开放结构 / 与 caller 关系）+ 子 feature 列表 config-yaml 标 done
- [x] **`ARCHITECTURE.md §4.6 Config schema`** — 第 4 节契约表更新 config 行 "Phase 3 已落地（feature `2026-05-17-config-yaml`）"
- [x] **`.codestable/requirements/agent-work-in-feishu.md`** — 变更日志加 config-yaml 落地条目；`implemented_by` 加本 feature；status 保持 `draft`（agent task 卡飞书呈现层 Phase 5 才落地）
- [x] **`.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`** — `config-yaml` status `in-progress` → `done`
- [x] **`.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §3 第 8 项** — status `planned` → `done` + feature 引用
- [ ] **`.codestable/attention.md`** — 无新增候选（见 §8）
- [ ] **`.codestable/compound/`** — 无新 decision 候选（serde_yml 是 tech-stack 但被 rust-module-organization 决策档 1 间接覆盖）

判据自检：未读 design 的人打开 ARCHITECTURE.md §3 Module D config 子节 + §4.6 Phase 3 状态，能知道"系统里现在有 `~/.roostery/config.yaml` 配置入口，schema 6 字段，caller 用 `roostery::config::{load, save}` 消费"。

## 6. requirement 回写

- 方案 frontmatter `requirement: agent-work-in-feishu`（当前 status: `draft`）
- 本 feature 兑现 req 的"用户身份 / 默认群配置"维度——Identity { user_id, default_chat_id, default_task_app_token } schema 落地
- 但 req 整体（飞书任务卡 / IM thread 跨设备呈现）远未实现——Phase 5 bot bridge 才会真正消费 identity
- 处理方式：**update** — frontmatter `implemented_by` 加本 feature；变更日志追加本 feature 条目；status 保持 `draft`（用户视角能"看见 agent 在飞书"还要等 Phase 5）

## 7. roadmap 回写

- 方案 frontmatter `roadmap: rust-rewrite` / `roadmap_item: config-yaml`，两字段都有值
- `rust-rewrite-items.yaml` 第 61-67 行 `slug: config-yaml` 当前 `status: in-progress` + `feature: 2026-05-17-config-yaml`（design 阶段已写入）
- 改 `status: done`，`validate-yaml.py --file` 校验通过
- `rust-rewrite-roadmap.md` §3 第 8 项当前 `状态: planned` → 改 `状态: **done**（feature ...）` + 补充备注

## 8. attention.md 候选盘点

回看实现过程：

**潜在候选**：

1. **`#[non_exhaustive]` 外部 crate struct literal 限制**——integration test 写 `Identity { ... }` 触发 `E0639`，要改 `cfg.identity.user_id = ...` 字段赋值。但 `.codestable/attention.md` "命令与脚本陷阱" 节已有该条目（"Rust `#[non_exhaustive]` struct 从外部 crate **完全不允许** struct literal..."），**不重复加** ✓

2. **`clippy::approx_constant` 守护浮点常量**——integration test 写 `3.14` 被 clippy 拒（误为 π 近似）。低频踩，归到 cs-learn 而非 attention.md。**不加** ✓

**结论**：**本 feature 未暴露需要补入 attention.md 的内容**。

## 9. 遗留

- **后续观察项**（design §4.1 已记）：
  - smoke 模块 binary 解析未来加 config fallback（cs-refactor 候选）
  - Phase 4 dispatcher-runners 起来时 runner 子节强类型化
  - schema_version v2 升级时 SchemaVersionMismatch 错误变体已留 future-proof 钩子
  - JournalConfig.rotation 字段反序列化但 journal 模块未消费（journal-core 硬编码 daily）
- **流程偏差（已知）**：impl 阶段 step 1-4 一起写完（schema + ConfigError + load + save 紧耦合单文件），用户在 impl 完成汇报阶段确认放过（不要求 git reset 重做）。技术上测试覆盖全 5 step 退出信号；功能正确
- **设计轻偏差**：design §0 提及 `RunnerCfgRaw` 作为概念占位词，代码实际用 `BTreeMap<String, serde_yml::Value>` 类型，未引入命名类型。判读不一致风险低（design §0 用法是描述含义而非要求类型名落地）；**不补代码**
- **顺手发现**：无
