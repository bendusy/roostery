---
doc_type: feature-acceptance
feature: 2026-05-15-journal-core
roadmap: rust-rewrite
roadmap_item: journal-core
requirement: portable-by-default
status: passed
summary: journal-core 验收通过；schema_version=1 公开承诺生效；ROOSTERY_HOME / ~/.roostery/ 路径迁移已同步 4 份架构 doc；roadmap items.yaml + 主文档第 4 条标记 done；req 追加 implemented_by 与变更日志（保持 draft 等 read/replay 工具）
tags: [phase-1, module-b, journal, acceptance]
---

# journal-core 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-16
> 关联方案 doc：`.codestable/features/2026-05-15-journal-core/journal-core-design.md`
> 实现 commit：`b9ac5be`

## 1. 接口契约核对

### 接口示例逐项核对（design §2.1）

- [x] `paths::roostery_home() -> PathBuf`：env > home_dir().join('.roostery') > CWD/.roostery 三段回退 → `crates/roostery/src/paths.rs:11-22` 实现一致
- [x] `paths::journal_dir() -> PathBuf`：`roostery_home().join("journal")` → `paths.rs:24` 实现一致
- [x] `JournalEntry` 结构 11 字段、derive (Serialize/Deserialize/Debug/Clone/PartialEq) → `journal.rs:13-26` 一致
- [x] `JournalResult` enum `#[serde(tag = "outcome", rename_all = "lowercase")]` 双变体 → `journal.rs:28-33` 一致
- [x] `JournalEntry::new(source, action) -> Self` 关联函数 → `journal.rs:35-53` 一致；默认值：event_id=新生成、ts=Utc::now、depth=0、其他 Option=None、params=Null、result=Ok(Null)、duration_ms=0、schema_version=1
- [x] `new_event_id() -> String` ULID 26 字符 Crockford → `journal.rs:68-81` 一致
- [x] `Journal::open(impl Into<PathBuf>) -> Self` → `journal.rs:88-90` 一致
- [x] `Journal::default() -> Self` 走 `paths::journal_dir()` → `journal.rs:104-108`（实现为 `impl Default for Journal`，调用形态 `Journal::default()` 不变；clippy 提示从 inherent fn 改为 trait 实现）
- [x] `Journal::append(&self, &JournalEntry) -> std::io::Result<PathBuf>` → `journal.rs:92-101` 一致

### 名词层"现状 → 变化"核对

- [x] `crates/roostery/src/journal.rs` 新建 → 存在（382 行）
- [x] `crates/roostery/src/paths.rs` 新建 → 存在（80 行）
- [x] `lib.rs` 加 `pub mod journal; pub mod paths;` → 已加
- [x] `Cargo.toml` 加 serde + chrono → 已加；额外加了 `dirs = "5"`（path 解析必需）+ `getrandom = "0.2"`（design §1 未列；implement 阶段 AskUserQuestion 与用户对齐后追加，本节第 4 条记录）+ `tempfile`（dev-deps）

### 流程图核对（design §2.2 mermaid）

- [x] `caller 构造 → append → 算 filename → mkdir → OpenOptions → to_vec + push '\n' → write_all → return PathBuf` 在 `Journal::append` 实现里逐节点对应（`journal.rs:92-101`）

**结论**：接口契约 100% 对齐 design。`Journal::default()` 改为 `impl Default` trait 是 clippy 反射触发的等价兑现，design checks 第 3 条已涵盖。

## 2. 行为与决策核对

### 需求摘要逐项验证（design §0+§1）

- [x] schema_version=1 对外公开承诺：`new_uses_schema_version_1` 测试 + `lib.rs` SCHEMA_VERSION=1 双重锁定
- [x] jsonl 原子 append：`OpenOptions::append+create+open` + 单 `write_all` syscall（`journal.rs:99-100`），POSIX <PIPE_BUF 原子
- [x] 文件名按 entry.ts UTC 日切（不是 now）：`journal.rs:94` `entry.ts.format("%Y-%m-%d")`；`cross_day_backfill_lands_on_entry_ts_day` 测试反向锁定
- [x] 路径迁 ROOSTERY_HOME / ~/.roostery/：`paths.rs:9-10` 常量；`ignores_legacy_feishu_hub_home` 测试反向锁定

### 明确不做逐项核对（design §1 + §3.2 反向核对）

- [x] 不实现 read / replay API：`grep -E "fn read_day|fn replay|fn read_entries"` → 无
- [x] 不暴露 async / 不引 tokio：`grep -E "async fn|use tokio"` → 无
- [x] 不实现 size / never rotation：`grep` 非测试代码无 rotation 字面值
- [x] 不读 Config 文件：`Journal::open` 接 `PathBuf`；无 Config import
- [x] 不做 flock：代码无 `flock` / `fs2` / `file_lock` 调用
- [x] 不做自动 cleanup / 跨设备同步：模块仅 append，无 unlink/rotate 逻辑
- [x] 不读 / 不迁 Python `~/.feishu_hub/journal/`：`paths.rs` 不读 `FEISHU_HUB_HOME`，单测反向锁定
- [x] 不实现 builder pattern：仅 `JournalEntry::new` 关联函数
- [x] 不约束 source / action 字符串：均为 `String`，无 enum
- [x] 不内置 redact 调用：`append` 不调 `redact::*`；`scrubbed_params_persist_through_journal` 集成测试证明 caller-side 集成
- [x] 不动 `legacy/python/`：git diff 范围内无 legacy 文件
- [x] 不动 SCHEMA_VERSION 值：`lib.rs:2` 仍为 `pub const SCHEMA_VERSION: u32 = 1`

### 关键决策落地（design §1 表）

- [x] D1 字段照搬 §4.2：11 字段一一对应（`field_names_match_roadmap` 测试断言 `obj.len() == 11`）
- [x] D2 ULID 自实现：`encode_b32` + `getrandom` 共 ~25 行；不引 `ulid` / `uuid` crate
- [x] D3 ts 用 chrono::DateTime<Utc>：`ts_serializes_as_rfc3339_with_z` 测试锁定
- [x] D4 JournalResult tag="outcome", rename_all=lowercase：`result_ok_serializes_with_outcome_tag` / `_err_` 两测试锁定
- [x] D5 路径一次性切到 ROOSTERY_HOME / ~/.roostery/：`paths.rs:9-10` 常量；不读 FEISHU_HUB_HOME；架构 doc / attention.md / roadmap §4.6 / CLAUDE.md 同步更新（见第 5 节）
- [x] D6 daily rotation 按 entry.ts UTC：`journal.rs:94`
- [x] D7 Journal::open + append handle struct：`journal.rs:84-108`
- [x] D8 单 syscall 原子 append：`OpenOptions().append(true).create(true).open` + `write_all(&buf)` 一次
- [x] D9 std::io::Result 错误：append 返 `std::io::Result<PathBuf>`，无自定义 `JournalError`
- [x] D10 JournalEntry::new 关联函数 + 结构体 update syntax：`journal.rs:35-53`

### 编排层"现状 → 变化"核对

- [x] 模块作为纯库被未来 caller 调用，无内部 workflow（per design §2.2）→ 实现内无 workflow，仅函数调用

### 流程级约束核对

- [x] 不变量 1（append 失败不写半行）：单 `write_all` syscall，POSIX 保证
- [x] 不变量 2（多进程并发不互相截断）：`OpenOptions::append(true)` → `O_APPEND` semantics，best-effort 超 PIPE_BUF（per design 接受）
- [x] 不变量 3（ts UTC + 文件名 UTC 日切）：`cross_day_backfill_lands_on_entry_ts_day` 测试覆盖
- [x] 不变量 4（schema_version=1 公开承诺）：架构 doc §6 已加硬约束第 8 条
- [x] 不变量 5（ULID 时间序）：`time_prefix_is_monotonic_across_ms` 测试覆盖
- [x] 错误语义（append 仅返 io::Result + serialize 失败 panic）：实现一致；`expect("JournalEntry serializes")` 在 `journal.rs:98`

### 挂载点反向核对（design §2.3）

- [x] M1 `crates/roostery/src/journal.rs` 存在 ✓
- [x] M2 `crates/roostery/src/paths.rs` 存在 ✓
- [x] M3 `lib.rs` 含 `pub mod journal; pub mod paths;` ✓
- [x] M4 `Cargo.toml` [dependencies] 含 serde + chrono ✓（额外有 dirs + getrandom，design §1 第 4 条已说明加 getrandom 经用户确认）
- [x] M5 `lib.rs` SCHEMA_VERSION == 1 未改 ✓

**反向 grep（清单外引用？）**：

```bash
grep -rE "JournalEntry|JournalResult|::journal::|::paths::|new_event_id|roostery_home|journal_dir" crates/
```

→ 命中仅在 `journal.rs` / `paths.rs` / `lib.rs`（pub mod 声明）+ inline tests + Cargo.toml（无）。**无清单外的额外挂载点**。

**拔除沙盘推演**：删除 `journal.rs` + `paths.rs` + 撤回 `lib.rs` 两行 `pub mod` + `Cargo.toml` 4 行依赖（serde / chrono / dirs / getrandom）+ dev-deps tempfile → feature 完整消失，无残留。SCHEMA_VERSION=1 常量 lib.rs 仍保留（rust-scaffold feature 已挂载该常量），不属于本 feature 引入。✓

## 3. 验收场景核对（design §3.1）

| 场景 | 证据 | 结果 |
|---|---|---|
| S1.1 全字段 roundtrip（含 Err 变体）| `journal::tests::schema::full_entry_roundtrip` | ✓ |
| S1.2 字段集合（11 个）| `journal::tests::schema::field_names_match_roadmap` | ✓ |
| S1.3 Ok tag 形态 | `journal::tests::schema::result_ok_serializes_with_outcome_tag` | ✓ |
| S1.4 Err tag 形态 | `journal::tests::schema::result_err_serializes_with_outcome_tag` | ✓ |
| S1.5 ts RFC 3339 + Z | `journal::tests::schema::ts_serializes_as_rfc3339_with_z` | ✓ |
| S2.1 ULID 长度 26 | `journal::tests::ulid::length_is_26` | ✓ |
| S2.2 字符 Crockford 字母表 | `journal::tests::ulid::alphabet_is_crockford_base32` | ✓ |
| S2.3 同毫秒 unique（1000 次）| `journal::tests::ulid::many_calls_are_unique` | ✓ |
| S2.4 时间分量按毫秒递增 | `journal::tests::ulid::time_prefix_is_monotonic_across_ms` | ✓ |
| S3.1 ROOSTERY_HOME 覆盖 | `paths::tests::env_override_wins` | ✓ |
| S3.2 默认 ~/.roostery | `paths::tests::defaults_to_home_dot_roostery` | ✓ |
| S3.3 不读 FEISHU_HUB_HOME | `paths::tests::ignores_legacy_feishu_hub_home` | ✓ |
| S4.1 基本写入 + 回读 | `journal::tests::append::basic_write_and_readback` | ✓ |
| S4.2 跨日 backfill | `journal::tests::append::cross_day_backfill_lands_on_entry_ts_day` | ✓ |
| S4.3 mkdir -p | `journal::tests::append::mkdir_p_creates_nested_dir` | ✓ |
| S4.4 多行 jsonl | `journal::tests::append::multiple_appends_produce_multi_line_jsonl` | ✓ |
| S4.5 返回路径正确 | `journal::tests::append::returned_path_matches_actual_write` | ✓ |
| S5.1 redact 集成 | `journal::tests::redact_integration::scrubbed_params_persist_through_journal` | ✓ |
| S6.1 cargo test --all 全绿 + ≥6 新测 | 实测 46 passed；本 feature 新增 20 个（journal 16 + paths 4）| ✓ |
| S6.2 clippy -D warnings | `cargo clippy --all-targets --all-features -- -D warnings` 无 warning | ✓ |
| S6.3 fmt --check | `cargo fmt --all --check` 通过 | ✓ |
| S6.4 SCHEMA_VERSION + entry.schema_version | `new_uses_schema_version_1` 测试 + `lib.rs` 静态常量 | ✓ |

**反向核对项**（design §3.2）：

| 项 | 实测 |
|---|---|
| 无 read_day/replay/read_entries | ✓ |
| 无 async/tokio | ✓ |
| FEISHU_HUB_HOME 仅在注释 + 测试 | ✓（`paths.rs:5,38,67` 全是注释或测试代码）|
| 无 ulid/uuid crate | ✓ |
| 无 tracing-appender/slog | ✓ |
| 非测试无 size:/never rotation | ✓ |
| journal.rs < 400 / paths.rs < 100 | ✓（382 / 80）|
| **panic/unwrap 仅 1 处 → 实际 3 处** | **❗ Calibrate**：design §3.2 calibrate 为"≤3 处 expect 且皆为 programmer/env error 兜底"。三处分别是 `Crockford alphabet is ASCII`（programmer error，不可达）/ `OS RNG available`（env error 兜底，getrandom 失败仅极端嵌入式场景）/ `JournalEntry serializes`（design 钦定的那处，programmer error）|

**Calibration 已就地写入** design `journal-core-design.md` §3.2，与 core-redact 当时 wc-l calibration 同款处理。

**前端验证**：本 feature 无前端改动，跳过浏览器验证。

**结论**：22 个验收场景 + 8 个反向核对项全部通过；1 项 calibrate（panic 数量阈值）。

## 4. 术语一致性

对照 design §0 术语表 grep 代码：

| 术语 | 代码命中 | 一致 |
|---|---|---|
| `JournalEntry` | journal.rs (struct + tests) | ✓ |
| `JournalResult` | journal.rs (enum + tests) | ✓ |
| `Journal` | journal.rs (struct + tests + Default impl) | ✓ |
| ULID | journal.rs (注释 + 测试模块名 `ulid`) | ✓ |
| `ROOSTERY_HOME` | paths.rs (常量 `ENV_HOME = "ROOSTERY_HOME"`) | ✓ |
| Journal dir | `journal_dir()` 函数 | ✓ |
| Daily rotation | 文件名 `YYYY-MM-DD.jsonl` 体现 | ✓ |
| Atomic append | append 实现 `OpenOptions::append+create+open` + `write_all` | ✓ |

防冲突 grep：

- `JournalEnvelope` / `LogEntry` / `Record` 等可能冲突的旧名：无命中
- `FEISHU_HUB_HOME` 运行时读取：无（仅在 paths.rs 注释 + 测试 env 设置 + 测试断言信息字符串）

## 5. 架构归并

| doc | 归并内容 | 状态 |
|---|---|---|
| `.codestable/architecture/ARCHITECTURE.md` §2 术语表 | Journal 改路径默认；JournalEntry schema_version=1 公开承诺措辞强化；新增 `ROOSTERY_HOME` 词条 | ✓ 已写入 |
| `.codestable/architecture/ARCHITECTURE.md` §3 Module B | 新增 journal 模块详情段（公开类型 / API / 写入语义 / schema 承诺 / 下游约束 / 路径解析 / 不在范围）；标 `journal-core` done + commit `b9ac5be` | ✓ 已写入 |
| `.codestable/architecture/ARCHITECTURE.md` §3 Module D | 路径字面值 `~/.feishu_hub/` → `~/.roostery/`（自 journal-core 起 + ROOSTERY_HOME 覆盖） | ✓ 已写入 |
| `.codestable/architecture/ARCHITECTURE.md` §6 已知约束 | 加第 2 条括号注脚 Rust 期路径 + 第 8 条 schema_version=1 公开承诺 | ✓ 已写入 |
| `.codestable/attention.md` 路径与目录约定节 | 路径双期标注（Rust ~/.roostery/ + Python legacy ~/.feishu_hub/）+ ROOSTERY_HOME env 覆盖 | ✓ 已写入 |
| `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` Module D 职责 | bootstrap 路径迁移 | ✓ 已写入 |
| `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §4.6 Config schema header + journal.dir 默认值 + 模板嵌入路径 + config-yaml 备注 + dispatcher-trace-budget 路径 | 5 处 `~/.feishu_hub/` → `~/.roostery/` | ✓ 已写入 |
| `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §5 子 feature 第 4 条 journal-core | 状态 planned → **done** + commit + 路径迁移 + Phase 1 限定 | ✓ 已写入 |
| `CLAUDE.md` Architecture 红线第 2 条 + State ownership 表 Audit/replay 行 | 路径双期 + journal 模块 done | ✓ 已写入 |

**判据自查**：未读 design 的人打开 ARCHITECTURE.md → 知道 Module B journal 模块已落地、API 形态、schema_version=1 公开承诺、路径默认 + env 覆盖、与 redact 的下游关系、不在范围的事项。✓

## 6. requirement 回写

`requirement: portable-by-default`（draft）。journal-core 是该 req 的核心兑现 feature 之一（schema 公开承诺 + 写入侧基础设施），但用户故事中"重跑 / 复现 / 自建 dashboard / 跨设备拷贝"等场景需要 read/replay 工具落地后才能完整兑现。

**处理**：保持 `status: draft`，追加 `implemented_by: [2026-05-15-core-redact, 2026-05-15-journal-core]` + 文末"变更日志"段记录两条 feature 的兑现范围与未兑现部分。`last_reviewed: 2026-05-16`。等 read/replay 工具落地（后续 phase 独立 feature）时再 `cs-req update` 升级为 current。

✓ 已写入 `.codestable/requirements/portable-by-default.md`。

## 7. roadmap 回写

frontmatter `roadmap: rust-rewrite` + `roadmap_item: journal-core`。

- [x] `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`：`journal-core` 条目 `status: in-progress` → **done**；`feature: 2026-05-15-journal-core` 保留
- [x] `validate-yaml.py` 校验通过
- [x] `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §5 第 4 条同步：`状态：planned` → `状态：done（feature 2026-05-15-journal-core，commit b9ac5be）` + 备注扩展

## 8. attention.md 候选盘点

回看本次实现暴露出的"下个 feature 的 AI 还会再撞一次"信息：

- **候选 1**（强烈建议加）：**Rust 2024 edition `std::env::set_var` / `remove_var` 是 unsafe**——env 操作改 unsafe block，且测试中并发触碰 env 必须用 `Mutex` 串行化。下个 feature 写涉及 env 的代码（config-yaml / hooks-merge / roostery-init 都会撞）会再问一次"为什么 set_var 报 unsafe error"。一句话能讲清。
- 候选 2（边缘）：`getrandom = "0.2"` 已加直接依赖；不要重复引 `rand` / `ulid` / `uuid`。但这条已经间接体现在 attention.md 现有 "redact 模块" / "lark-cli 唯一通道" 风格的"架构红线"段，且 ULID 自实现是局部决策，价值低于候选 1。

**本节不擅自写入**，下面收尾环节按 cs-feat-accept §6 步骤问用户是否走 `cs-note`。

## 9. 遗留

- **后续优化点 / 待开 feature**：
  - read / replay API（`read_day` / 流式 reader / 过滤）—— 真正消费方（debug 工具 / 自建 dashboard）出现时起独立 feature
  - size / never rotation 策略 + Config 驱动 dir —— 由 `config-yaml` feature（Phase 3）消费
  - 跨进程并发原子性升级（flock）—— Phase 4 dispatcher 多进程实际并发后再评估
  - Python `~/.feishu_hub/journal/` 历史数据迁移工具 —— 如有用户需求单独起 feature；目前 Python baseline 不维护
- **已知限制**：
  - jsonl entry 超 PIPE_BUF（4 KiB）时多进程并发可能撕裂，best-effort（per design 接受 + portable-by-default req "完整性用户自负" 兜底）
  - ULID 同毫秒内顺序不强保证（标准只承诺毫秒粒度排序）
- **实现阶段"顺手发现"**：无（本 feature 无旁逸改动）
- **CI 验证**：本地 fmt / clippy / test 三命令全绿；GitHub Actions 待 push 后验证（commit `b9ac5be` 已落本地，未推远端）
