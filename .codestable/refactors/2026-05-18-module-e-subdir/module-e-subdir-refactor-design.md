---
doc_type: refactor-design
refactor: 2026-05-18-module-e-subdir
status: approved
scope: 把 trace / budget / runaway / hook_event / rules / runners / dispatcher 7 个 Phase 4 Module E 模块从 crates/roostery/src/ 顶层聚到 src/dispatcher/ 子目录；纯文件移动 + import 路径更新，0 行为变更
summary: dispatcher 子目录化——design 阶段反复 flag 的稳定动作，本期 Phase 4 收尾正好触发。编译器全程校验 + 322 测试全跑兜底
---

# module-e-subdir refactor design

## 1. 本次范围

**从 scan 勾选**：R-1（唯一条目，预定义 scope）

**明确不做**：
- 不改任何函数 / 类型签名（编译器 + 测试双自检）
- 不改测试逻辑（仅改 use 路径）
- 不改 cargo.toml 依赖
- 不改 ~/.roostery/* 用户路径 / 文件格式
- 不重命名模块（trace.rs 仍叫 trace.rs，仅迁入子目录）
- 不引入新模块 / 不删旧模块
- 不动 lib.rs 以外的非 Module E 模块（agent_detect / config / hooks_merge / identity / journal / lark_cli / onboarding / paths / redact / remoterefs / smoke 全部留在顶层）

**预估总工作量**：单 session 内可完成。改动约 ~30 处 use 行 + 7 个 git mv + 1 新文件（mod.rs）。

**总风险档位**：低

## 2. 前置依赖

- [x] **测试覆盖**：lib 322 + integration（runners 12 / dispatcher 7 / trace_budget 6 / rules 5 / 其他 ~20）+ doctest 全部已存在；本 refactor 无需补刻画测试
- [x] **call site grep 已完成**（scan §R-1 list site 段）
- [x] **本次 refactor 前 main 干净**：`git status` 仅有 refactor 目录新文件，无其他 in-flight 改动
- [x] **CI baseline 绿**：commit `a7d23e5` CI run #26018196490 全绿；refactor 后再次推 CI 验证

## 3. 执行顺序

**总体策略**：按 dependency 倒序搬——先搬 leaf 模块（无内部依赖），最后搬 dispatcher.rs（依赖最多）。每步独立 commit + cargo build 全绿。

依赖图（who imports whom）：
```
trace ← {hook_event, runaway, runners, dispatcher}
budget ← dispatcher
runaway ← dispatcher
hook_event ← {rules, runners, dispatcher}
rules ← dispatcher
runners ← dispatcher
dispatcher ← {main.rs, dispatcher_integration test}
```

Leaf 顺序：**budget → trace → hook_event → runaway → rules → runners → dispatcher**（dispatcher 最后搬，因为它依赖前 6 个）。

但**单步一搬一步 commit 成本太高**——每搬一个文件都要同步改所有 caller 的 use 路径，7 步会让中间态长时间挂着混合路径。

**采用方案 B：一次性原子搬迁**——单步内 git mv 全部 7 文件 + 新建 `dispatcher/mod.rs` + 同步改 lib.rs / main.rs / 5 个 integration test / 7 模块内部互引 use 行 + cargo build/test 全绿验证。理由：
- 编译器一次性校验所有路径；中间任何错就一次性暴露
- 单 commit 完整事务，git revert 干净
- 路径改动机械且范围明确（scan 已 grep 全清单）

如果一次性搬迁 cargo build 失败 → 排查根因；中途不强行拆分步骤（保持事务原子性）。

### 步骤 1：建 dispatcher 子目录骨架

- **动作**：建 `crates/roostery/src/dispatcher/mod.rs`，内容仅 7 行 `pub mod {name};` 子模块声明 + 原 `dispatcher.rs` 文件内容
- **引用方法**：L3-M2 重组目录变种"先建新结构"
- **退出信号**：文件创建成功（暂不 cargo build——下一步同步搬迁后才编译）
- **验证责任**：AI（文件存在）
- **回滚**：rm 新文件

### 步骤 2：git mv 7 文件 + 删除原 dispatcher.rs

- **动作**：
  - `git mv crates/roostery/src/trace.rs crates/roostery/src/dispatcher/trace.rs`
  - 同样搬 budget / runaway / hook_event / rules / runners
  - `git mv crates/roostery/src/dispatcher.rs crates/roostery/src/dispatcher/_old.rs`（临时，下一步删掉——这一步只是搬开占位让 dispatcher/ 目录干净）
  - 然后把 `_old.rs` 内容覆盖到上一步建的 `mod.rs` 然后删 `_old.rs`，或直接调整顺序：步骤 1 不建 mod.rs，本步先 git mv dispatcher.rs → dispatcher/mod.rs（git mv 保留 history）
- **方案调整**：步骤 1 不预建 mod.rs；步骤 2 改为 `git mv src/dispatcher.rs src/dispatcher/mod.rs`，然后在 mod.rs 顶部追加 7 行 `pub mod {trace,budget,...};`，再 git mv 6 个 leaf
- **退出信号**：`git status` 显示 7 个 rename + mod.rs 加了 7 行 pub mod
- **验证责任**：AI（git status + cat 检查 mod.rs 头部）
- **回滚**：`git reset HEAD .` + restore

### 步骤 3：改 lib.rs

- **动作**：删 lib.rs 第 5/8/14/17/18/19/21 行 7 个 `pub mod` 声明（budget / hook_event / rules / runaway / runners / trace / dispatcher），仅留 `pub mod dispatcher;`
- **退出信号**：lib.rs 业务模块条目从 18 减到 12 个
- **验证**：AI（grep 确认）
- **回滚**：git checkout

### 步骤 4：改 7 模块内部互引 use 路径

- **动作**：把模块内 `use crate::{hook_event,trace,budget,...}::X` 改成 `use crate::dispatcher::{hook_event,trace,...}::X`；或更简洁，用 `use super::*` / `use super::trace::*`（同 dispatcher/ 目录内 sibling 引用）
- **决策**：优先 `use super::{trace,budget,...}::X` 风格——sibling 模块互引最 idiomatic
- **影响范围**：scan §R-1 列举的 7 处：rules.rs / hook_event.rs / runaway.rs / runners.rs / dispatcher/mod.rs（4 处） / 加上 dispatcher/mod.rs 自身的 `use crate::{config, rules}` 中的 rules 改为 super
- **退出信号**：cargo build 不报"unresolved import"
- **验证**：AI（cargo build）
- **回滚**：git checkout 7 文件

### 步骤 5：改 main.rs + 5 个 integration test 的 use 路径

- **动作**：
  - `main.rs`：`roostery::hook_event::*` → `roostery::dispatcher::hook_event::*`；`roostery::runners::*` → `roostery::dispatcher::runners::*`（dispatcher / DispatchError 已在 dispatcher 路径下不动）
  - `tests/runners_integration.rs`：3 处 use 行
  - `tests/dispatcher_integration.rs`：4 处 use 行（dispatcher 不变；其余加 dispatcher::）
  - `tests/trace_budget_integration.rs`：3 处
  - `tests/rules_integration.rs`：2 处
  - 总计约 ~13 处 use 行
- **退出信号**：cargo build + cargo build --tests 全过
- **验证**：AI（cargo build / build --tests）
- **回滚**：git checkout 各文件

### 步骤 6：cargo build / clippy / fmt / test 全跑

- **动作**：
  - `cargo fmt --all --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --all`
  - `cargo test --doc`
- **退出信号**：四命令全绿；测试 count 不少于 baseline（322+ 通过；零回归）
- **验证**：AI（贴命令输出）
- **回滚**：整 refactor 起点 git revert

### 步骤 7：HUMAN 整体目视确认

- **动作**：用户检查
  - `tree crates/roostery/src/dispatcher/` 结构合理
  - `cat crates/roostery/src/lib.rs` 干净
  - 抽一个 use 路径（如 `crates/roostery/tests/dispatcher_integration.rs` 头部）确认是 `roostery::dispatcher::...` 风格
  - 跑一次 `cargo run --bin roostery -- dispatcher --help` 输出正常
- **退出信号**：用户明确说"通过"
- **验证**：HUMAN
- **回滚**：git revert refactor commit

### 步骤 8：commit + 推 CI

- **动作**：单 commit message "refactor(module-e-subdir): 7 modules → src/dispatcher/"；推 main
- **退出信号**：CI 三 job 全绿
- **验证**：AI 等 CI，HUMAN 最终签字

## 4. 风险与看点

**高风险步骤**：步骤 4（7 模块内部互引）和步骤 5（main.rs + 5 测试）合在一起 ~25 处 use 行改动——单个错字会被 cargo build 立刻打回，无运行时风险。

**容易出错的点**：
1. `dispatcher/mod.rs` 内部 `use crate::dispatcher::{trace,...}` vs `use super::{trace,...}`——后者更 idiomatic，需要 implement 阶段确认 7 模块内部统一一种写法
2. doctest 中如果有引用顶层路径（如 `lark_cli/mod.rs` 内 _doctest_anchor）需检查——grep 验证后无 Module E 模块在 doctest 中被引用
3. `dispatcher::DispatchError` / `dispatcher::fire` 等顶层符号是从 `dispatcher/mod.rs` 直接 pub，已经在新 mod.rs 内不动语义；caller `roostery::dispatcher::fire` 路径完全不变
4. main.rs 已经用 `roostery::dispatcher::*`，**这部分不需要改**

**反向核查**：
- 步骤 6 后 grep `use roostery::(trace|budget|runaway|hook_event|rules|runners)::` 全 repo → 0 命中
- grep `use crate::(trace|budget|runaway|hook_event|rules|runners)::` 全 src/ → 0 命中（dispatcher 内部应用 super::）
- git diff stat 应只显示 use 行变化 + 7 文件 rename + 1 新文件，无函数体改动

**外部影响**：
- npm 包 / PyPI namespace 是空 reservation，零影响
- crates.io 未 publish 零影响
- Rust users（无），单 binary 内部重构

## 5. 用户 review 要点

请整体过一遍，重点：

1. **§3 一次性原子搬迁方案**——单步事务而非 7 步逐模块搬。理由：路径改动机械范围明确，编译器一次性校验，git revert 干净；分步会留长时间混合路径中间态
2. **§3 步骤 4 `use super::` 选型**——dispatcher/ 内部 sibling 互引优先 `use super::{name}` 而非 `use crate::dispatcher::{name}`，更 idiomatic + 路径短
3. **§4 反向核查**——commit 前必跑 grep 确认顶层路径 0 命中
4. **dispatcher 子目录顶层符号不变**——`roostery::dispatcher::fire / replay / test_rule / DispatchError / DispatchOutcome / ...` 全部不动；只是顶层 `roostery::trace::TraceContext` → `roostery::dispatcher::trace::TraceContext` 等迁徙

放行后落 `status: approved`，抽 checklist 进 apply。
