---
doc_type: refactor-apply-notes
refactor: 2026-05-18-module-e-subdir
---

# module-e-subdir apply notes

## 总览

8 步全部按计划完成，单 session 内闭合。零行为变更，11 文件 +52 / -45 行（仅 use / pub mod / 文档头注释 + 7 个 git mv + 1 新 mod.rs 入口）。322+ lib + integration + doc test 全过；fmt / clippy -D warnings / test --all / test --doc 四命令本地全绿；反向 grep 双 0 命中。

## 步骤 1-2：git mv 7 文件 + mod.rs 头部加 pub mod

- 完成时间：2026-05-18
- 改动文件：
  - `git mv crates/roostery/src/dispatcher.rs → src/dispatcher/mod.rs`（git rename 保留 history）
  - `git mv crates/roostery/src/{trace,budget,runaway,hook_event,rules,runners}.rs → src/dispatcher/*.rs`
  - `crates/roostery/src/dispatcher/mod.rs` 头部加 6 行 `pub mod {budget,hook_event,rules,runaway,runners,trace};`（注意只有 6 个——`dispatcher` 自己不是子模块）
- 验证结果：`git status` 显示 7 个 R/RM；`find dispatcher -name '*.rs'` = 8 个（mod.rs + 7 子模块）✓
- 偏离：无

## 步骤 3：lib.rs 删 6 个顶层 pub mod

- 改动文件：`crates/roostery/src/lib.rs`
- 删除：`pub mod budget;` / `hook_event;` / `rules;` / `runaway;` / `runners;` / `trace;` 6 行
- 保留：`pub mod dispatcher;`
- 验证：grep `pub mod` lib.rs → 业务模块 18 → 12 ✓
- 偏离：无

## 步骤 4：dispatcher 内部 sibling 互引改 super::

- 改动文件：`src/dispatcher/{hook_event,rules,runaway,runners,mod}.rs`
- 改动：
  - `hook_event.rs`：`use crate::trace::TraceContext` → `use super::trace::TraceContext`
  - `rules.rs`：`use crate::hook_event::HookEvent` → `use super::hook_event::HookEvent`
  - `runners.rs`：`use crate::{hook_event,trace}` → `use super::{hook_event,trace}`
  - `runaway.rs`：`use crate::trace::TraceId` → `use super::trace::TraceId`
  - `mod.rs`：
    - `use crate::budget::{...}` → `use self::budget::{...}`
    - `use crate::hook_event::HookEvent` → `use self::hook_event::HookEvent`
    - `use crate::runaway::{...}` → `use self::runaway::{...}`
    - `use crate::runners::{...}` → `use self::runners::{...}`
    - `use crate::trace::{...}` → `use self::trace::{...}`
    - `use crate::{config, rules}` → `use crate::config;`（删 `rules`——已通过 `pub mod rules` 在作用域内）
    - test fn 内 `crate::runners::*` → `super::runners::*`
    - test fn 内 `crate::hook_event::*` → `self::hook_event::*`
- 偏离 1：第一次尝试加 `use self::{budget, rules};` 显式 import 与 `pub mod` 的 namespace 冲突（E0255 redefined）。修正：删该行——`pub mod` 已让 `budget` / `rules` 在作用域内可直接 `budget::save(...)` / `rules::matches(...)` 调用
- 验证：`cargo build` 通过 ✓

## 步骤 5：main.rs + 5 个 integration test 改 use 路径

- 改动文件：
  - `crates/roostery/src/main.rs`：3 行 use（hook_event / runners / rules）改 `roostery::dispatcher::{name}::`；分离 `roostery::{config, rules}` 为独立 `use roostery::config;` + `use roostery::dispatcher::rules;`
  - `tests/runners_integration.rs`：3 行
  - `tests/dispatcher_integration.rs`：5 行 use + 2 处函数体内 `roostery::runners::NoopRunner` → `roostery::dispatcher::runners::NoopRunner`
  - `tests/trace_budget_integration.rs`：4 行
  - `tests/rules_integration.rs`：2 行
- 验证：`cargo build --tests` 通过 ✓
- 偏离：dispatcher_integration.rs 还有 2 处 function body 内的 `roostery::runners::NoopRunner` 全路径引用（除 use 行外），第一次 grep 时漏 catch，build error 暴露后补改。结论：refactor 时全 repo grep 不能只查 `use` 行，要查所有 `roostery::{ban_path}` 全路径

## 步骤 6：四命令 + 反向 grep

- `cargo fmt --all --check`：第一次失败（mod.rs 文档注释格式微调），跑 `cargo fmt --all` 修复后通过 ✓
- `cargo clippy --all-targets --all-features -- -D warnings`：通过 ✓
- `cargo test --all`：313 lib + 12 dispatcher inline + 7 dispatcher integ + 全部其他 integ 全过；test count 与 baseline 一致（dispatcher 模块从顶层挪进子目录，test 数不变）✓
- `cargo test --doc`：4 passed ✓
- 反向 grep：
  - `grep -rE 'use roostery::(trace|budget|runaway|hook_event|rules|runners)::' crates/roostery` → 0 命中 ✓
  - `grep -rE 'use crate::(trace|budget|runaway|hook_event|rules|runners)::' crates/roostery/src` → 0 命中 ✓

## 步骤 7：HUMAN 目视确认

- 用户确认：通过（2026-05-18）
- 验证项：
  - `find dispatcher` 结构 8 个 .rs ✓
  - `cat lib.rs` 12 个业务模块 + `dispatcher` 一行 ✓
  - 抽样 `tests/dispatcher_integration.rs` 头部 use 全 `roostery::dispatcher::*` 风格 ✓
  - `cargo run --bin roostery -- dispatcher --help` 输出三子命令树完整 ✓

## 步骤 8：commit + 推 CI

- commit: `d2d7d75` "refactor(module-e-subdir): Module E 7 modules → src/dispatcher/"
- CI run: #26019292413 三 job 全绿（2026-05-18T07:20Z 推送，~1min 完成）

## 行为等价自检

逐项检查：

- [x] 0 函数体改动：`git diff --stat` 显示 11 文件 +52 / -45 行；每个 diff 都是 use 行 / pub mod 行 / 文档注释 / 文件 rename，无任何 fn body / struct field / enum variant / test 函数体变动
- [x] 0 模块名 / 类型名改动：`TraceContext` / `BudgetState` / `HookEvent` / `RunnerRegistry` / `DispatchOutcome` 等公开符号定义不动；模块文件名 `trace.rs` / `budget.rs` 等也不变（仅 mv 进子目录）
- [x] 0 Cargo.toml 改动：未触碰
- [x] 0 用户路径 / 文件格式改动：`~/.roostery/journal/` / `budget.json` / `rules.yaml` / `config.yaml` schema 完全不动
- [x] CLI 行为不变：`roostery dispatcher --help` 输出与重构前一致

## 经验沉淀

- **L3 重组目录"只搬不改"实战**：编译器是最忠实的验证工具，单 session 内闭合是可行的（11 文件 / ~25 处 use 路径改动）
- **sibling 模块互引选 `use super::{name}::*`**：dispatcher/ 内部统一这一种风格，比 `use crate::dispatcher::{name}::*` 短且 idiomatic
- **`pub mod foo` 已让 foo 在作用域内**：不要画蛇添足加 `use self::{foo};`——会触发 E0255 redefined（同 namespace 重复声明）
- **全 repo grep 不能只查 `use` 行**：refactor 改路径时还要查所有 `roostery::{name}::*` 全路径引用（main.rs / 集成测试 / function body 内的全路径）——一次 grep 模式 `\b{old_root}::` 比 `^use {old_root}::` 更安全
