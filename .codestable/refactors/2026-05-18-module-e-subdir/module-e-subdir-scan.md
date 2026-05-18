# module-e-subdir scan

> 扫描范围：`crates/roostery/src/` 顶层 7 个 Phase 4 / Module E 模块文件 + 所有 in-crate caller（lib.rs / main.rs / 7 模块互引 / 5 个 integration test）。
> 发现条数：1 条（结构性聚目录）。
> 风险档位：**低风险**——纯文件移动 + import 路径更新，编译器全程校验，0 行为变更。
> 前置依赖：本 feature acceptance 已完成（dispatcher-loop accept commit `244befc`，Phase 4 / Module E 整体 done），无新功能在飞。

## 背景（为什么这次做）

自 `2026-05-18-dispatcher-trace-budget` design §2.5 起，每个 Phase 4 子 feature 的设计 / 验收都 flag 同一句话：**"Phase 4 收尾 dispatcher-loop 起来时一次性聚 src/dispatcher/ 子目录"**。当前状态：

- `crates/roostery/src/` 顶层 .rs 文件 = 19（业务模块计数），逼近 compound decision `2026-05-16-rust-module-organization.md` 档 2 "< 20 不强制目录化"上限
- Phase 4 / Module E 已整体完成（trace / budget / runaway / hook_event / rules / runners / dispatcher 7 模块），它们是同一职责域（dispatcher 编排），各自独立但语义聚合
- 当前 cs-refactor 触发条件完全满足：业务收尾点 + scope 精确预定义 + 0 新功能在飞 + 编译器自检完整

## 清单条目

### R-1：把 Module E 7 模块聚到 `src/dispatcher/` 子目录

- **方法**：L3-M2 "结构拆分 / 重组目录"（只搬不改行为变种）
- **分类**：L3 结构调整
- **风险档位**：低（编译器全程校验 + 现有测试 322 条全跑）
- **当前状态**：
  - 7 顶层文件 `trace.rs / budget.rs / runaway.rs / hook_event.rs / rules.rs / runners.rs / dispatcher.rs`
  - `lib.rs` 直接 `pub mod {name};` 暴露
  - 模块间互引用：`trace ← {hook_event, runaway, runners, dispatcher}` / `hook_event ← {rules, runners, dispatcher}` / `rules ← dispatcher` / `runaway ← dispatcher` / `runners ← dispatcher` / `budget ← dispatcher`
  - 外部 caller：`main.rs`（dispatcher / hook_event / runners）/ 5 个 integration test
- **目标状态**：
  - 新建 `src/dispatcher/mod.rs`（替代原 `src/dispatcher.rs` 作为子目录入口）
  - 7 模块文件迁入 `src/dispatcher/{name}.rs`
  - `lib.rs` 只保留 `pub mod dispatcher;`，7 子模块从顶层移除
  - 用户从仓库视角看到 `roostery::dispatcher::{trace, budget, runaway, hook_event, rules, runners, ...}`——清晰反映模块归属
  - 顶层 .rs 文件数 19 → 13（业务模块 = 12 + lib.rs）；新增 1 个 dispatcher/ 目录含 8 个 .rs（mod.rs + 7 个子模块）
- **call site 更新**：
  - **lib.rs**：删 7 个 `pub mod`，留 `pub mod dispatcher`
  - **main.rs**：`roostery::dispatcher::{self, DispatchError}` 保留；`roostery::hook_event::*` → `roostery::dispatcher::hook_event::*`；`roostery::runners::*` → `roostery::dispatcher::runners::*`
  - **7 模块互引**：`crate::hook_event::*` → `crate::dispatcher::hook_event::*` 等（10 处左右）
  - **5 个 integration test**：`roostery::{trace,budget,runaway,hook_event,rules,runners}::*` → `roostery::dispatcher::{...}::*`（grep 列举 ~15 处）
  - **dispatcher.rs 内部** `use crate::{config, rules}`：`rules` 改 `crate::dispatcher::rules` 或在 mod.rs 内 `use super::rules` 风格——implement 时定
- **风险点**：
  - rust-analyzer / IDE 路径解析：实际靠 cargo 编译验证，IDE 缓存可后续刷新
  - 不影响外部 crates.io 用户——roostery 是 binary crate（lib + 2 bins），无公开 API 承诺；package 还未 publish
- **行为等价检查清单**：
  - 编译器全程绿灯（cargo build / clippy --all-targets --all-features -- -D warnings）
  - 现有测试 322 条全跑（cargo test --all）
  - cargo fmt --all --check
  - 集成测试 binary（shim / dispatcher 等）行为不变（cargo run 调用一遍）
  - **零函数体改动**：`git diff --stat` 应仅显示 lib.rs + main.rs + 5 个 integration test + 7 模块的 use 行 + dispatcher/mod.rs 新文件；无函数 / 类型 / 测试逻辑改动
- **回滚**：每步独立 commit；任一步失败 `git revert` 即回到当步前的干净中间态

### 用户勾选

- [✓] R-1（本 refactor 唯一目标，用户预设）

（注：本 scan 因 design 阶段已反复 flag 同一动作，scope 用户已预定义，等于隐式勾选——见下文 design 整体 review）
