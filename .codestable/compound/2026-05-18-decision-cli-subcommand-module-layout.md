---
doc_type: decision
category: convention
slug: cli-subcommand-module-layout
status: active
created: 2026-05-18
tags: [rust, clap, module-organization, cli, convention]
related_features: [2026-05-18-bot-stop-hook]
related_commits: [220c7b0]
---

# CLI 子命令的模块分层约定

## 背景

`crates/roostery/src/main.rs` 在 Phase 0-4 期间持续往里塞 subcommand 实现：
- `Command::Smoke` — 几行内联
- `Command::Init(InitArgs)` + `run_init` ~40 行
- `Command::Dispatcher(DispatcherArgs)` + 3 sub variants + `run_dispatcher` / `run_fire` / `run_replay` / `run_test_rule` / `synth_hook_event` / `print_outcome` ~180 行

到 Phase 5 第 2 子 feature `bot-stop-hook` 起步前，main.rs 已 352 行，且**职责开始混杂**——既是 CLI 顶层 router 又是各 subcommand 的胶水。再继续往里塞 `Bot { stop-hook, push }` 子命令的 args struct + run 实现，main.rs 会越过 ~400 行临界 + 多个不相关 subcommand 的代码相互交叉污染（grep 一个 subcommand 的实现要跳过其他几个 subcommand 的 args struct 定义）。

bot-stop-hook design §2.5 评估时反射检查触发，提议把"子命令的 args + run"移回对应模块的 `pub mod cli`，main.rs 只做一行 dispatch。实施跑通后 (commit `220c7b0`) 验证模式可行：
- main.rs 增量 = 3 行（`Command::Bot(BotArgs)` 变体 + 一行 `=> bot_stop_hook::cli::run(args)` + 一行 import）
- `bot_stop_hook::cli` 模块容纳 ~150 行子命令逻辑（`BotArgs / BotSub / PushCliArgs / StopHookCliArgs / build_request_from_push_args / outcome_to_exit_code / run`）

本约定把这条已落地的实践上升为跨 feature 稳定原则。

## 决定

**子命令的 args struct 和 run 函数放在该 feature 的对应模块的 `pub mod cli` 子模块；main.rs 只做"顶层 `Command` enum 定义 + 一行 dispatch"**。

### 具体规范

#### 1. main.rs 的职责（仅这些）

```rust
// 1. 顶层 Cli / Command enum 定义
#[derive(Subcommand)]
enum Command {
    Smoke,
    Init(InitArgs),
    Dispatcher(DispatcherArgs),
    Bot(bot_stop_hook::cli::BotArgs),  // 子命令 args 类型 reference 模块
    // ...
}

// 2. 一行 dispatch
fn main() -> ExitCode {
    match Cli::parse().command {
        Some(Command::Bot(args)) => bot_stop_hook::cli::run(args),
        // 每条都是一行
    }
}
```

#### 2. 每个 feature 模块自带 cli 子模块

```rust
// crates/roostery/src/bot_stop_hook.rs
pub mod cli {
    use super::*;
    use clap::{Args, Subcommand};
    use std::process::ExitCode;

    #[derive(Args)]
    pub struct BotArgs {
        #[command(subcommand)]
        pub subcmd: BotSub,
    }

    #[derive(Subcommand)]
    pub enum BotSub {
        StopHook(StopHookCliArgs),
        Push(PushCliArgs),
    }

    #[derive(Args)]
    pub struct PushCliArgs { /* flag fields */ }

    pub fn run(args: BotArgs) -> ExitCode {
        // tokio rt 构建 + runner 注入 + dispatch 到 super:: lib fn
    }
}
```

#### 3. 适用范围

- ✅ **所有未来新增 subcommand** —— 至少 2 个以上 flag 字段或 ≥ 1 个 `run_*` 辅助函数的子命令都遵守
- ✅ **新 feature 引入新 subcommand 时**，把 args + run 直接放进 feature 的 `pub mod cli`
- ⚠️ **现存 subcommand 不要求强制迁移**——`Command::Smoke` / `Command::Init` / `Command::Dispatcher` 现有实现保留，下次该模块做较大改动时顺手迁；不为迁移而专门开 cs-refactor
- ❌ **不适用于内部 helper 子命令**（如 dispatcher 的 fire / replay / test-rule 这种已经在 dispatcher::sub 内组织的）——它们已自成体系

#### 4. 命名规约

- `pub mod cli` 是固定名称（不是 `pub mod commands` / `pub mod subcommand`）
- 内部 args struct 命名 `{Module}Args / {Sub}CliArgs`（如 `BotArgs` / `PushCliArgs`）
- 入口 fn 命名 `pub fn run(args: {Module}Args) -> ExitCode`
- 如需 async，在 `run` 内部建 tokio runtime 而非把 async 漏到 main.rs（与 dispatcher 现有模式一致）

#### 5. 编排惯例

`run` fn 负责的固定动作：
1. 构造 tokio runtime（current_thread 即可，hook / cli 都是短任务）
2. 构造 lark-cli runner（`LarkCli::new()`，按需 wrap `Journaled` 装饰器）
3. 解 args 走业务 fn
4. 把业务结果（如 `PushOutcome`）转 `ExitCode`

业务编排不放 `cli` 模块（业务 fn 走 `super::push` / `super::run_stop_hook`），`cli` 只做"clap input → 业务 fn input"+"业务 fn output → process output (stdout + exit code)"的两段适配。

## 为什么

### 拒绝的替代

- **方案 A：所有子命令都塞 main.rs**（Phase 0-4 现状）—— main.rs 失控膨胀；多个不相关 subcommand 代码交叉；新增 subcommand 要在 main.rs 加 args struct + run fn + match arm 三处，违反"一个变更一处生效"
- **方案 B：单独建 `crates/roostery/src/cli/` 子目录装所有子命令**——能解决 main.rs 膨胀，但子命令 args 与业务 fn 跨目录，每次 grep / 改 subcommand 要在两处跳；且这种"通用 cli 层"对 single-feature 子命令是过度抽象
- **方案 C：本约定**（采纳）—— args 与业务 fn 同模块同文件，main.rs 只做顶层 router

### 收益

- **就近原则**：grep 一个子命令的实现就到模块内，不跨文件
- **可卸载性**：删一个 feature 时连带删 `pub mod cli` 即可，main.rs 只少一行 match arm
- **测试就近**：clap args 的 `try_parse_from` 单测可与业务 fn 单测在同 `#[cfg(test)] mod tests` 里共享 fixture
- **main.rs 控制在 400 行内**：每个子命令在 main.rs 占用 = 1 个 enum 变体 + 1 行 dispatch ≈ 3 行

### 兑现条件

bot-stop-hook commit `220c7b0` 验证：
- main.rs 增量从历史平均 ~50 行/subcommand 降到 3 行/subcommand
- `bot_stop_hook::cli` 模块独立装下 `BotArgs / BotSub / PushCliArgs / StopHookCliArgs / build_request_from_push_args / outcome_to_exit_code / run` 共 ~150 行，模块边界清晰
- `bot --help` / `bot push --help` / `bot stop-hook --help` 输出齐全（clap derive 文档块工作正常）

## 反场景

- ❌ **简单到 1-2 行的 subcommand**（如 `Command::Smoke` 仅调一个 `smoke::run()` + 打印 JSON）—— 强迁回模块反而过度抽象，main.rs 内联即可
- ❌ **跨多个 feature 模块的通用子命令**（如未来"`roostery status` 跨 dispatcher / bot / journal 看综合状态"）—— 这种归属不清的，老老实实建 `cli/` 子目录或放 main.rs

## 何时复审本决策

- 当 main.rs 行数再次越过 400 行 → 检查是否有 subcommand 漏迁；批量迁回模块 cli
- 当出现"多 feature 共用一个 cli 模块"诉求 → 升级到独立 `cli/` 子目录的设计，本决策退役
- 当 clap 4.x 升级到 5.x 且 API 大改 → 复审 derive 模式是否仍可行
