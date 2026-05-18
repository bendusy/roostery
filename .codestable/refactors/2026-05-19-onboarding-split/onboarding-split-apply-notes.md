---
doc_type: refactor-apply-notes
refactor: 2026-05-19-onboarding-split
---

# onboarding split apply notes

## 步骤 1: 原子搬运 + finding-05 inline memmem

- 完成时间: 2026-05-19
- 改动文件:
  - 删除 `crates/roostery/src/onboarding.rs`（1067 行）
  - 新建 `crates/roostery/src/onboarding/mod.rs`（359 行）
  - 新建 `crates/roostery/src/onboarding/types.rs`（198 行）
  - 新建 `crates/roostery/src/onboarding/shim.rs`（138 行）
  - 新建 `crates/roostery/src/onboarding/hooks.rs`（91 行）
  - 新建 `crates/roostery/src/onboarding/lark_cli_override.rs`（243 行）
  - 新建 `crates/roostery/src/onboarding/env_rc.rs`（135 行）
- 总行数：1067 → 1164（+97 行，全部是各子文件顶部的 `use` import + 模块 docstring + `looks_like_roostery_shim` 加 2 个新测试覆盖增量）
- finding-05 落地：`fn memmem` 函数已删；`looks_like_roostery_shim` 内 inline 为 `bytes.windows(SHIM_MAGIC.len()).any(|w| w == SHIM_MAGIC)`；原 `memmem_finds_needle` 私有测试替换为 3 个 `looks_like_roostery_shim_*` 行为测试（覆盖 finds_magic / rejects_user_script / handles_missing_file），覆盖面增强
- 验证结果:
  - `cargo build` 绿（1.48s）
  - `cargo clippy --all-targets --all-features -- -D warnings` 0 警告
  - `cargo test --all` 全绿（392 lib + 12 onboarding_integration + 多个集成测试 0 failed）
  - 比重构前 +2 测试（looks_like_roostery_shim 行为测试新增 3 个，移除 1 个 memmem 直接测试）
- 偏离: 无

## 步骤 2: fmt + grep 反向核对

- 完成时间: 2026-05-19
- 改动文件: `cargo fmt --all` 已自动整理（无 diff，all clean）
- 验证结果:
  - `cargo fmt --all --check` pass
  - `cargo test --doc` 4 doctests 通过
  - grep 反向核对：
    - `main.rs:10,342,345` 公开 API 引用未改（`onboarding::{self, InitOptions}` + `onboarding::run` + `onboarding::format_report`）
    - `tests/onboarding_integration.rs:10` 引用 `roostery::onboarding::{self, InitOptions, SkipReason}` 未改
    - 旧 `onboarding.rs` 不复现（`ls` 报 not found）
    - `grep -rn 'fn memmem' crates/roostery/src/` 0 命中（finding-05 已落地）
    - 各子文件大小：mod 359 / lark_cli_override 243 / types 198 / shim 138 / env_rc 135 / hooks 91，主体小于 design 设定的 350 行；mod.rs 因 OnboardingError 大枚举 + run 编排 + format_report 共聚略超 9 行，可接受
- 偏离: 无

## 行为等价自检

- ✅ 公开 API 字面兼容：`onboarding::run` / `InitOptions` / `SkipReason` / `format_report` / `OnboardingError` / `InitReport` / `RealLarkCliSource` / `ShellKind` 通过 `mod.rs` `pub use` 全部保持原路径
- ✅ 函数体 byte-for-byte 搬运（除 finding-05 inline memmem 一处）
- ✅ 集成测试 `tests/onboarding_integration.rs` 多个 e2e 测试通过，证明 `run()` 外部可见行为不变
- ✅ `cargo test --all` 总通过数前后比对：390 → 392（+2 = inline memmem 时新加的 looks_like_roostery_shim 行为测试）

## 偏离 design 的小项

design §1 提议 "mod.rs 含 OnboardingError + run + format_report + 小 helpers + pub use re-export"。实际 mod.rs 359 行略超 design 设定的 < 350 budget——主要 `OnboardingError` enum 13 变体 + 详细错误消息撑了约 100 行。

考虑过拆 `error.rs` 单独存 `OnboardingError`，但 mod.rs 的 `run()` 主编排紧密用 `OnboardingError`，子模块也都 `use super::OnboardingError`——把 error 类型放 mod.rs 顶层是最自然的位置。9 行超 budget 不构成可读性问题，不做额外拆分。

## 顺手发现

无。严格 "只搬不改" + 单条 finding-05 inline 全程严守。

## 最终状态

- 改动文件清单（`git status --short`）：
  ```
  D  crates/roostery/src/onboarding.rs
  A  crates/roostery/src/onboarding/mod.rs
  A  crates/roostery/src/onboarding/types.rs
  A  crates/roostery/src/onboarding/shim.rs
  A  crates/roostery/src/onboarding/hooks.rs
  A  crates/roostery/src/onboarding/lark_cli_override.rs
  A  crates/roostery/src/onboarding/env_rc.rs
  ```
- 四绿状态：fmt-check ✓ / clippy ✓ / test --all ✓ / test --doc ✓
- 公开 API 零变化（grep 验证）
- finding-04 + finding-05 同 commit 落地
- 与 `lark_cli/` + `dispatcher/` + `bot_stop_hook/` 同模块组织模式对齐
