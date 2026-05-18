---
doc_type: audit-finding
slug: bot-stop-hook-single-file-overload
audit: 2026-05-18-post-release-rust-idiom
dimension: maintainability
severity: P1
confidence: high
suggested_action: cs-refactor
tags: [bot-stop-hook, module-layout, rust-idiom, file-decomposition]
---

# Finding 02：`bot_stop_hook.rs` 1463 行单文件 4 大块职责未拆

## 位置

`crates/roostery/src/bot_stop_hook.rs`（产品 725 行 + 测试 738 行 = 1463 行）

## 证据

文件内 `pub struct / pub fn / pub mod` 边界划分清晰，已分 4 个内聚块：

| 行号范围 | 块 | 长度 | 内容 |
|---|---|---|---|
| 31-140 | A. types | ~110 行 | `PushRequest` / `PushOptions` / `PushOutcome` / `PushStatus` |
| 142-345 | B. stop_input | ~204 行 | `StopHookInput` struct + transcript jsonl tail 解析 |
| 346-512 | C. push | ~166 行 | `push` / `finish_with_fallback` / `run_stop_hook` / `parse_stop_hook_input` + 工具函数 `truncate_utf8` / `cwd_basename` / `stable_idem_key` / `resolve_receive_id` |
| 530-700 | D. cli | ~170 行 | `pub mod cli` — clap args + `run` dispatch |
| 725-1463 | 测试 | 738 行 | 22+ 个测试函数 |

## 为什么构成问题

1. **认知负担**：1463 行单文件 IDE 折叠状态下都难总览；新加 IM thread / hitl_router（feature `bot-bridge-cluster` 已 planned）势必继续往这个文件加，3000+ 行可预期。
2. **与项目其他多概念模块对齐失败**：`lark_cli/` 已拆 5 文件（error / journaled / mock / runner / subprocess + mod）；`dispatcher/` 已拆 7 文件。`bot_stop_hook` 是同等概念体量却塞单文件，破坏一致性。
3. **测试边界混淆**：728 行测试覆盖 4 块业务，文件内分 5 个 `mod tests` 子模块，与"按业务块分文件"的物理隔离相比可发现性差。
4. **决策 `cli-subcommand-module-layout` 落地不彻底**：该决策（commit `220c7b0` 时签下）规定 "CLI args + run 放对应模块的 `pub mod cli`"。当前 `pub mod cli` 是 inline nested module，下一步推到独立 `cli.rs` 子文件是该 convention 的完整形态。

## 建议改法（不在本审计动手，留给 cs-refactor）

按已划分的 4 块拆成 `bot_stop_hook/` 子目录：

```
crates/roostery/src/bot_stop_hook/
├── mod.rs          # pub use re-export，~50 行
├── types.rs        # PushRequest / PushOptions / PushOutcome / PushStatus + 内部工具
├── stop_input.rs   # StopHookInput + transcript jsonl tail
├── push.rs         # push / finish_with_fallback / run_stop_hook
└── cli.rs          # BotArgs / BotSub / PushCliArgs / StopHookCliArgs / run
```

测试随业务块下沉到对应文件的 `#[cfg(test)] mod tests`。集成测试（`tests/bot_cli_integration.rs`）零改动。

**注意 pub(crate) 边界**：
- `truncate_utf8` / `cwd_basename` / `stable_idem_key` 是跨子模块工具 → 放 `mod.rs` 或新 `util.rs`
- `resolve_receive_id` 同上
- 子模块之间 `use super::types::*` / `use super::util::*` 即可

## 影响范围

- 改动量：纯文件 split + path-only diff（**无函数体改动**）
- 公开 API 零变化（`bot_stop_hook::push` / `run_stop_hook` / `cli::run` 等签名不变）
- `lib.rs` 一行 `pub mod bot_stop_hook;` 不变（`mod.rs` 接管）
- 这是 design §2.5 "只搬不改行为" 的标准微重构边界
- 完成后 `bot-bridge-cluster` feature（已 planned）加 hitl_router / bot_relay_task 时有清晰落点

## 关联

- 决策 `2026-05-16-decision-rust-module-organization.md`（模块组织 convention）
- 决策 `2026-05-18-decision-cli-subcommand-module-layout.md`（CLI 子模块布局）
- ARCHITECTURE.md §3 Module F bot_stop_hook 段
