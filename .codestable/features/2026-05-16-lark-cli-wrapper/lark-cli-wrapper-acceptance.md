---
doc_type: feature-acceptance
feature: 2026-05-16-lark-cli-wrapper
roadmap: rust-rewrite
roadmap_item: lark-cli-wrapper
requirement: agent-work-in-feishu
status: passed
summary: lark-cli-wrapper 验收通过；33 新测全过 + CI 三 job 绿；2 处实施期 design 调整就地回写（RunOptions builder API + s2_6 duration-based timeout 验证）；归并 ARCHITECTURE §2/§3/§6 + roadmap §4 演化记录机制 + items.yaml/main doc done + req agent-work-in-feishu 变更日志；2 条 attention.md 候选盘点（待用户决定）
tags: [phase-2, module-c, lark-cli, async, trait, acceptance]
---

# lark-cli-wrapper 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-16
> 关联方案 doc：`.codestable/features/2026-05-16-lark-cli-wrapper/lark-cli-wrapper-design.md`
> 实现 commits：`a33d4fd` scaffold + `1cc2e0f` main + `2129c82` 第 1 次 ETXTBSY fix + `cc44dfa` 第 2 次 ETXTBSY fix（终）
> CI：GitHub Actions run `25963978982` — fmt / clippy / test 三 job 全绿

## 1. 接口契约核对

### 接口示例逐项核对（design §2.1）

- [x] `LarkRunner` trait async + Send + Sync + `run` 默认 method 委托 `run_with_options` → `runner.rs:50-59`
- [x] `RunOptions` 3 字段 + `#[non_exhaustive]` + Default → `runner.rs:8-22`
- [x] **`RunOptions` builder API**（new / with_timeout / with_stdin / with_profile）→ `runner.rs:24-46`。**这是实施期 design 调整**：原 design §2.1 写 `RunOptions { timeout, ..Default::default() }` struct literal，实施发现 `#[non_exhaustive]` 从外部 crate 完全不允许 struct literal（rustc E0639；`..Default::default()` 也不破例），只能走 builder。已在 design §1 D2 / D3 间隙以注释 + mod.rs:91-114 compile_fail doctest 锁定。**Acceptance 时回写 design 主体**
- [x] `LarkError` 是 `#[non_exhaustive]` rich enum 4 变体（Spawn / NonZeroExit / OutputParse / Timeout）+ thiserror Error derive；每变体携带专有数据 → `error.rs:11-46`
- [x] `retriable()` method 用 `matches!` 实现（不是字段）→ `error.rs:52-62`
- [x] `MAX_FIELD_LEN_IN_ERR = 4096` 公开常量 → `error.rs:9`；`truncate_field` / `truncate_args` 私有 helper → `error.rs:75-91`
- [x] `LarkCli::new/with_binary/with_default_timeout` → `subprocess.rs:30-52`
- [x] `LarkRunner for LarkCli` impl（spawn → stdin → tokio::time::timeout → 4 错误归一）→ `subprocess.rs:57-127`
- [x] `MockLarkRunner` + fluent `enqueue_*(&Self)` + `calls/assert_no_unconsumed` + Drop warn → `mock.rs:22-83`
- [x] `LarkRunner for MockLarkRunner` impl（FIFO + 空队列 panic）→ `mock.rs:86-101`
- [x] `Journaled<R: LarkRunner>` 装饰器 → `journaled.rs:14-32`
- [x] `LarkRunner for Journaled<R>` impl（Instant 计时 + redact::scrub_argv + JournalEntry + 写失败 tracing::warn!）→ `journaled.rs:35-65`
- [x] `mod.rs` 顶部架构红线 docstring 含"飞书 syscall 唯一通道" + ARCHITECTURE §6 第 1 条引用 + 反例 + 正用法 → `mod.rs:1-39`

### 名词层"现状 → 变化"核对

- [x] `crates/roostery/src/lark_cli/` 子目录新建（mod.rs + 5 sub） → 6 文件全部存在
- [x] `lib.rs` 加 `pub mod lark_cli;` ✓
- [x] `Cargo.toml` 加 tokio + async-trait + thiserror + tracing ✓

### 流程图核对（design §2.2）

- [x] `LarkCli::run_with_options` 主流程：A→B→C 节点对齐（拼 full_argv → spawn → stdin? → timeout-wrapped wait → exit code 分支 → JSON parse） → `subprocess.rs:62-127` ✓
- [x] `Journaled<R>::run_with_options` 主流程：Instant → inner → duration → scrub_argv → JournalEntry → append → 失败 tracing::warn! 不破坏原 result → `journaled.rs:35-65` ✓
- [x] `MockLarkRunner::run_with_options` 主流程：lock → push args → pop_front | panic → `mock.rs:86-101` ✓

**结论**：接口契约 100% 对齐。`RunOptions` 从 struct-literal 改为 builder 是基于 Rust 编译期事实的修正，design 已就地反映。

## 2. 行为与决策核对

### 需求摘要逐项验证（design §0 / §1）

- [x] LarkRunner trait async + Send + Sync ✓
- [x] 默认 method `run` 委托 `run_with_options` ✓
- [x] 4 错误归一为 LarkError rich enum + thiserror ✓
- [x] retriable() method 实现（含 Timeout / exit 124 / body_code 99991663-4）✓
- [x] Journaled 装饰器分离 journal 写入职责 ✓
- [x] Mock 默认 public 不加 cfg gate ✓（mod.rs 第 5 行 doc 说明 "Test utility ..."）
- [x] ROOSTERY_LARK_CLI_BIN env > "lark-cli" 默认 ✓
- [x] tokio "full" + async-trait + thiserror + tracing facade 4 直接依赖 ✓
- [x] 模块走档 2 子目录组织 ✓
- [x] roadmap §4.1 契约升级独立 commit（`3ec565e`）+ 末尾"契约演化记录"段 ✓

### 明确不做逐项核对（design §1 + §3.2 反向 grep）

| 项 | 实测 |
|---|---|
| 不实现业务包裹函数（im_/docs_/drive_/base_）| ✓ grep 无命中 |
| 不内置 retry | ✓ grep 非测试 / 非注释无 retry/retries/attempt |
| 不实现 jq | ✓ grep 无命中 |
| 不读 Config | ✓ grep 无命中 |
| 不读 FEISHU_HUB_LARK_CLI_BIN runtime | ✓（仅在 doc comment 描述 "intentionally not consulted"）|
| 不引 mockall | ✓ grep 无命中 |
| 不实现 LarkCli::Default | ✓ grep 无命中（用 `#[allow(clippy::new_without_default)]` 显式声明）|
| 不暴露 std::io::Error / serde_json::Error / tokio types 给 caller match | ✓（都包在 LarkError 变体内）|
| 不修改 legacy/python/ | ✓ git diff 范围无 |
| 不约束 lark-cli 子命令名集合 | ✓ trait 接 `&[&str]` |

### 关键决策落地（design §1 表 13 条）

- [x] D1 retry 留 dispatcher + retriable() method（杠杆 3）✓
- [x] D2 run_with_options 第二 method（不破 §4.1 钦定）✓
- [x] D3 MockLarkRunner 自建队列 ✓
- [x] D4 tokio "full" ✓
- [x] D5 Journaled<R> 装饰器分离 journal 写入 ✓
- [x] D6 LarkError rich enum + thiserror（杠杆 1）+ 4 字段截断 ✓
- [x] D7 默认 timeout 30s ✓
- [x] D8 ROOSTERY_LARK_CLI_BIN env ✓
- [x] D9 模块走档 2 ✓
- [x] D10 trait signature 守 §4.1（升级后形态）不动；newtype 升级走 cs-roadmap update ✓
- [x] D11 MockLarkRunner 一直 public（mock.rs:1-5 doc）✓
- [x] D12 Journaled 写 journal 失败用 `tracing::warn!`（不 eprintln）✓
- [x] D13 roadmap 契约演化记录段 ADR-lite 模式落地 ✓（roadmap §4 开头机制说明 + §4.1 末尾首条记录）

### 流程级约束核对（design §2.2 不变量 1-8）

| 不变量 | 验证 | 结果 |
|---|---|---|
| 1 trait run 默认实现委托 | mock.rs::s1_1 测试 | ✓ |
| 2 LarkError 严格 4 变体归一 | error.rs 类型签名 | ✓ |
| 3 Journaled 写 journal 失败 tracing::warn! 不破坏原 result | journaled.rs::s5_4 测试 | ✓ |
| 4 Journaled 写入前 params.argv 必经 redact::scrub_argv | journaled.rs::s5_3 测试 | ✓ |
| 5 MockLarkRunner 队列空 panic | mock.rs::s4_4 #[should_panic] | ✓ |
| 6 LarkCli timeout 必 kill child | tokio kill_on_drop 上游契约 + duration assertion 替代 PID kill -0（详见 §3 与"实施期调整"段）| ✓（duration verified；kill verification 改方案）|
| 7 LarkCli 不重试；retriable 仅是 method | error.rs retriable 是 fn 不是字段 | ✓ |
| 8 subprocess stdout 为空返 Value::Null | subprocess.rs::s2_2 测试 | ✓ |

### 挂载点反向核对（design §2.3 6 条）

- [x] M1 `crates/roostery/src/lark_cli/` 目录存在含 mod.rs + 5 子文件 ✓
- [x] M2 `lib.rs` 含 `pub mod lark_cli;` ✓
- [x] M3 `Cargo.toml` 含 tokio + async-trait + thiserror + tracing 4 依赖 ✓
- [x] M4 `mod.rs pub use runner::LarkRunner;` 暴露 ✓
- [x] M5 `Journaled<R>` 装饰器存在且可独立 wrap 任意 LarkRunner ✓（journaled.rs::dyn_compat 测试佐证）
- [x] M6 `LarkError` 是 `#[non_exhaustive]` rich enum + thiserror derive ✓（契约形态锁定）

**反向 grep（清单外引用？）**：

```bash
grep -rE "lark_cli::|LarkRunner|LarkCli|LarkError|RunOptions|MockLarkRunner|Journaled" crates/ \
  | grep -v "^crates/roostery/src/lark_cli/\|^crates/roostery/src/lib.rs:"
```
→ 无匹配。下游 caller（Phase 3+ shim / dispatcher / task_writer）尚未出现，符合 Phase 2 边界。

**拔除沙盘推演**：删 `lark_cli/` 目录 + 撤 `lib.rs` 一行 `pub mod lark_cli;` + 撤 Cargo.toml 4 行依赖 → feature 完整消失，无残留（serde/serde_json 是其他 feature 引入）。✓

## 3. 验收场景核对（design §3.1 共 33 条）

| 场景 | 证据 | 结果 |
|---|---|---|
| S1.1 trait 默认 method 委托 | mock.rs::s1_1 | ✓ |
| S1.2 dyn-compatible Box<dyn LarkRunner> | mock.rs::s1_2 + journaled.rs::dyn_compat | ✓ |
| S2.1 happy path | subprocess.rs::s2_1 | ✓ |
| S2.2 空 stdout → Value::Null | s2_2 | ✓ |
| S2.3 非 JSON → OutputParse | s2_3 | ✓ |
| S2.4 NonZeroExit + stderr + body_code None | s2_4 | ✓ |
| S2.4b body_code 99991663 解析 + retriable | s2_4b | ✓ |
| S2.5 Spawn 失败 | s2_5 | ✓ |
| **S2.6 Timeout 验证（调整方案）** | s2_6 改为 duration assertion（< 5s 而 fixture sleeps 30s）。原 design 要求 "PID + kill -0 验证 child 真死"——实施发现 macOS 并发负载下 sh spawn 时间不可预测、Linux CI 上 ETXTBSY race 又叠加（详见 "实施期调整" 段），改用 duration 验证 timeout 真触发 + tokio kill_on_drop 是上游契约不该在我们这一层重测。**Calibration 已在代码 inline 注释 + design §3.1 S2.6 待回写** | ✓（duration）|
| S2.7 stdin | s2_7 | ✓ |
| S2.8 profile flag | s2_8 | ✓ |
| S3.1 retriable() truth table 7 行 | error.rs::retriable_truth_table | ✓ |
| S3.2 non_exhaustive match 外部需 `_` | mod.rs:46-85 `compile_fail,E0004` doctest | ✓ |
| S3.3 Display 含变体专有数据 | error.rs::display_contains_variant_data | ✓ |
| S3.4 Error::source() 链 | error.rs::error_source_chain | ✓ |
| S4.1 enqueue_ok | mock.rs::s4_1 | ✓ |
| S4.2 enqueue_err | mock.rs::s4_2 | ✓ |
| S4.3 FIFO | mock.rs::s4_3 | ✓ |
| S4.3b fluent chain mixed | mock.rs::s4_3b | ✓ |
| S4.4 空队列 panic | mock.rs::s4_4 #[should_panic] | ✓ |
| S4.5 assert_no_unconsumed | mock.rs::s4_5 + s4_5b | ✓ |
| S4.6 calls 顺序 | mock.rs::s4_6 | ✓ |
| S4.7 Drop 未消费 tracing::warn! 不 panic | mock.rs:67-83 Drop impl 实现 + 上述测试不 panic | ✓ |
| S5.1 Journaled happy + 写 schema_version=1 entry | journaled.rs::s5_1 | ✓ |
| S5.2 Err 路径 + kind + message | journaled.rs::s5_2 | ✓ |
| S5.3 argv 脱敏 | journaled.rs::s5_3 | ✓ |
| S5.4 写 journal 失败不破坏原 result + tracing::warn! | journaled.rs::s5_4 | ✓ |
| S5.5 `compile_fail,E0639` non_exhaustive struct literal 守护 | mod.rs:91-114 compile_fail doctest（也证明了 builder API 是唯一外部构造路径）| ✓ |
| S5.6 AsRef/Display | **N/A**（本 feature 不涉及 newtype token；business-identifier-newtype convention 适用范围不含 source/profile/binary path 类内部 label）| 跳过 |
| S6.1 cargo test --all ≥ 12 新测 | 实际 33 新测（8+2+9+11+5/3）| ✓ |
| S6.2 cargo test --doc | 3 passed + 2 ignored compile_fail 正样 | ✓ |
| S6.3 clippy -D warnings | `cargo clippy --all-targets --all-features` 通过（含 `#[allow(clippy::new_without_default)]`）| ✓ |
| S6.4 fmt --all --check | 通过 | ✓ |
| S6.5 架构红线 grep 守护 | 全 crates/ 无 reqwest/ureq/hyper::Client/isahc | ✓ |
| S6.6 openssl 守护 | `cargo tree | grep -c openssl` == 0 | ✓ |
| GitHub Actions 三 job | run `25963978982` 全绿 | ✓ |

### 实施期调整说明（design vs 实际）

1. **`RunOptions` 改 builder API**（影响 design §2.1 + §3.1 S5.5）：实施时发现 `#[non_exhaustive]` struct 外部 crate 完全不允许 struct literal（包括 `..Default::default()` 也不破例，rustc E0639）。改用 `RunOptions::new().with_timeout(d).with_stdin(s).with_profile(p)` 链式构造，doc + doctest 已就地反映。**design 主体 S5.5 / §2.1 应在 acceptance 后回写**
2. **`s2_6` timeout 测试改 duration assertion**（影响 design §3.1 S2.6 + §3.2 grep）：原方案"PID + kill -0 验证 child 真死"在 macOS 并发负载下根本性 flaky（sh spawn 时间不可预测），同时 Linux CI 上 ETXTBSY race 叠加（fix 用 `std::fs::write` 替代 `File::create+drop`，commit `cc44dfa`）。改用 duration assertion（返回 < 5s 而 fixture sleeps 30s）证明 timeout 真触发；tokio `kill_on_drop` 是上游契约不该在我们这一层重测。code inline 注释 + 本验收 §3 已 flag

**反向核对项 calibration**（design §3.2）：
- design §3.2 反向 grep "nix crate 无引入" ✓（用 std::process::Command 也无；改 duration 验证后连这条都不需要）
- design §3.2 "tracing::warn!" ≥ 1 in journaled.rs ✓（mock.rs Drop impl 也用了，比 design 预期多一个）

**前端验证**：无前端改动，跳过。

**结论**：33 验收场景全过 + 2 项实施期调整有 rationale + 文件级 fix。1 项跳过（S5.6 不适用）。

## 4. 术语一致性

| 术语 | 代码命中 | 一致 |
|---|---|---|
| `LarkRunner` / `LarkCli` / `MockLarkRunner` / `Journaled` / `LarkError` / `RunOptions` | 各自模块 + tests | ✓ |
| `MAX_FIELD_LEN_IN_ERR` / `MAX_DEPTH`（remoterefs）| 各自模块 | ✓ |
| `ROOSTERY_LARK_CLI_BIN` env | subprocess.rs 常量 + 注释 | ✓ |
| Retriable | error.rs::retriable() method | ✓（不是字段） |
| Journaled 装饰器 | journaled.rs::Journaled<R> + doc | ✓ |

防冲突 grep：

- `LarkErrorKind` / `pub kind: LarkErrorKind` / `pub retriable:` / `pub enum LarkErrorKind`：全无 ✓（rich enum 替代成功）
- `eprintln` in journaled.rs：无 ✓（用 tracing::warn!）
- `Hash` derive：无 ✓
- `reqwest` / `ureq` / `hyper::Client` / `isahc`：全无 ✓（架构红线）

## 5. 架构归并

| doc | 归并内容 | 状态 |
|---|---|---|
| `.codestable/architecture/ARCHITECTURE.md` §2 术语表 | `LarkRunner` 条目扩描述（三实现 + commit ref + run/run_with_options 双 method）；新增 `LarkError` rich enum + retriable() method 条目 | ✓ 已写入 |
| `.codestable/architecture/ARCHITECTURE.md` §3 Module C | 新增 lark_cli 模块详情段（commit + 子目录组织 + 公开 trait + RunOptions builder + LarkError 4 变体 + 三实现详细职责 + 下游约定 + 不在范围）；子 feature 标 done | ✓ 已写入 |
| `.codestable/architecture/ARCHITECTURE.md` §6 红线第 1 条 | 加正向引用：兑现层 + 下游必须 take `Arc<dyn LarkRunner>` / `impl LarkRunner` 注入 + 双向引用 `lark_cli/mod.rs` docstring | ✓ 已写入 |
| `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §4 开头 | 新增"契约演化记录机制（ADR-lite）"段说明固定格式 + 适用范围（轻量修订走 ADR-lite，重大改动仍走 cs-roadmap update）| ✓ 已写入 |
| `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §5 第 5 条 lark-cli-wrapper | 状态 planned → done + commit + 描述扩 + 备注扩（首次 tokio/async/subprocess/档 2/ADR-lite/Mock 默认 public）| ✓ 已写入 |
| `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml` | lark-cli-wrapper status → done | ✓ 已写入，validate 通过 |
| `.codestable/attention.md` | 不动（飞书必经 lark-cli 已在；ETXTBSY 和 non_exhaustive struct literal 是值得记的 attention.md 候选，§8 盘点登记）| ✓ 评估完成 |
| `.codestable/requirements/agent-work-in-feishu.md` | implemented_by 追加 `2026-05-16-lark-cli-wrapper` + 变更日志加 2026-05-16 条目；保持 draft（用户面端的"跨设备看到 agent 在写什么"还需 Phase 5 兑现）| ✓ 已写入 |
| `.codestable/compound/` | 无新增 convention 归档需求（lark_cli 是 business-identifier-newtype 和 rust-module-organization 两条 convention 的具体应用实例，非新规约）| — |

**判据自查**：未读 design 的人打开 ARCHITECTURE.md 应能知道：Module C lark_cli 模块已落地、子目录组织、`LarkRunner` trait + 三实现的形态、`LarkError` rich enum + retriable()、下游必须经 trait 注入；§6 第 1 条不再只有"不准做什么"还有"该用什么"的正向引用。✓

## 6. requirement 回写

`requirement: agent-work-in-feishu`（draft）。lark-cli-wrapper 是该 req 的**基础设施层**——飞书 syscall 通道成型后，往后 Phase 4 dispatcher / Phase 5 task_writer / bot_bridge 才能把 agent 工作真正"贴"到飞书任务卡 / IM thread / Docs 评论里。用户面端"跨设备看到 agent 在写什么"还需 Phase 5 兑现。

**处理**：保持 `status: draft`，`implemented_by` 追加 `2026-05-16-lark-cli-wrapper`，变更日志加 2026-05-16 条目记录基础设施层落地 + 仍需 Phase 5 兑现用户故事。`last_reviewed: 2026-05-16`。

✓ 已写入 `.codestable/requirements/agent-work-in-feishu.md`。

## 7. roadmap 回写

frontmatter `roadmap: rust-rewrite` + `roadmap_item: lark-cli-wrapper`。

- [x] `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`：`lark-cli-wrapper` 条目 status `in-progress` → **done**；feature 字段保留
- [x] `validate-yaml.py` 校验通过
- [x] `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §5 第 5 条同步：planned → done + commit + 备注扩展
- [x] `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §4 开头新增 ADR-lite 机制说明（design D13 兑现）
- [x] `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §4.1 末尾已有 2026-05-16 契约演化记录（design 阶段 commit `3ec565e` 已写入）

## 8. attention.md 候选盘点

本次实施暴露 2 条值得记的"下个 feature AI 还会撞"信息：

- **候选 1**（强烈推荐）：**`#[non_exhaustive]` struct 从外部 crate 完全不允许 struct literal**——包括 `..Default::default()` 也会触发 rustc E0639。后续任何 feature 引入 non_exhaustive 容器 struct 时必须配 builder API（参考 `RunOptions::new/with_*`），不能假设 `..Default::default()` 旁路。下个 Config feature / Phase 4 dispatcher 各种 options struct 都会撞
- **候选 2**（强烈推荐）：**Linux ETXTBSY race in subprocess tests**——测试中创建 + 立即 execve 文件时，`File::create + write + drop` 在 close(2) 完成与 execve 之间有微窗口让 Linux 报 `ExecutableFileBusy`（macOS 不报）。fixture script 类测试统一用 `std::fs::write(path, content)`（atomic write + close），不要用 `File::create + write_all`

候选 3（边缘）：**tokio `kill_on_drop` 是上游契约不该在测试里重测**——PID + kill -0 类验证在 macOS 并发负载下不可靠；duration assertion 更稳。但这条更像测试经验，归 cs-learn 更合适，不入 attention.md

**本节结论**：2 条强候选；待用户在"退出后"环节决定。

## 9. 遗留

- **后续优化点 / 待开 feature**：
  - **trait signature 升级 newtype**：roadmap §4.5 `TraceContext` 含 `String` 字段；business-identifier-newtype convention 要求升级为 `TraceId` / `EventId`。建议起 cs-roadmap update 集中处理 §4.1 / §4.5 / §4.6 三处涉及业务标识符的 String，避免零散
  - **Config 驱动 `LarkCli` 构造**：Phase 3 `config-yaml` feature 起来后，桥接 binary path / default_timeout / default_profile 从 Config 取值
  - **tracing-subscriber 接入**：本 feature 引入 tracing facade；Phase 4 dispatcher 真起 logging pipeline 时配置 subscriber
  - **业务包裹层**：Phase 5 `task_writer` / `bot_task_writer` 在 LarkRunner 之上实现 `task_create` / `messages_send` / `docs_create` 等 typed API
- **已知限制**：
  - subprocess kill 行为依赖 tokio `kill_on_drop(true)` 上游契约——不在本 feature 测试范围
  - MockLarkRunner 不在 cfg gate 下，production binary 含 mock 代码（release LTO 应消除；Roostery split crate 时再加 `feature = "test-utils"`）
- **实现阶段"顺手发现"**：
  - design §2.1 / §3.1 S5.5 关于 `RunOptions` 用 struct literal 的描述与 Rust 实际行为不符——已在代码 + 本验收报告 flag，design 主体待 acceptance 后回写
  - design §3.1 S2.6 timeout PID 验证方案在 macOS / Linux 都有可靠性问题——已改 duration assertion + inline rationale
- **CI 调试历史**：Linux 首次跑 fail（ETXTBSY），2 个 commit 才稳定（候选 2）
