---
doc_type: audit-finding
slug: onboarding-single-file-overload
audit: 2026-05-18-post-release-rust-idiom
dimension: maintainability
severity: P2
confidence: high
suggested_action: cs-refactor
tags: [onboarding, module-layout, rust-idiom]
---

# Finding 04：`onboarding.rs` 1067 行单文件 7 块职责未拆

## 位置

`crates/roostery/src/onboarding.rs`（产品 632 行 + 测试 435 行 = 1067 行）

## 证据

文件内 7 块职责清晰可分：

| 行号 | 块 | 内容 |
|---|---|---|
| 37-108 | A. error | `OnboardingError` 枚举 + From impls |
| 109-145 | B. shell | `ShellKind` 枚举 + path detection |
| 146-198 | C. types | `InitOptions` / `InitReport` / `SkipReason` / `RealLarkCliSource` |
| 199-277 | D. run | `pub async fn run` 主编排（78 行）|
| 281-396 | E. shim | `install_shim` / `file_sha256` / `looks_like_roostery_shim` / `memmem` / `set_executable` |
| 399-525 | F. hooks | `write_sh_bridge` / `merge_hooks_for` / `resolve_real_lark_cli` / `validate_override` |
| 526-630 | G. env_rc | `write_env_file` / `shell_quote` / `patch_shell_rc` / `format_report` |

## 为什么构成问题

与 finding-02 同性质——单文件多概念聚集，未来加新 agent runtime hook / 新 shell rc 处理（如 fish）时落点不明。

但优先级低于 bot_stop_hook：
- `run()` 78 行是合理 orchestration 长度（不是 process_one 那种 200+ 行 boilerplate）
- 各 helper 之间内聚度高（都在装机这一个场景下协作）
- `bot-stop-hook` accept 时已写决策 `cli-subcommand-module-layout` 不覆盖 onboarding；本模块没受新决策推动

## 建议改法（不在本审计动手，留给 cs-refactor）

候选拆法（与 lark_cli / dispatcher 对齐）：

```
crates/roostery/src/onboarding/
├── mod.rs              # OnboardingError / InitOptions / InitReport / run() + format_report
├── shell.rs            # ShellKind + 检测 + shell_quote
├── shim.rs             # install_shim + file_sha256 + looks_like_roostery_shim
├── hooks.rs            # write_sh_bridge + merge_hooks_for
├── lark_cli_override.rs  # resolve_real_lark_cli + validate_override
└── env_rc.rs           # write_env_file + patch_shell_rc
```

`memmem` 见 finding-05，单独处理。

## 影响范围

- 改动量：纯 split，无函数体改动
- 公开 API 零变化（`onboarding::run` / `onboarding::InitOptions` / `onboarding::format_report` 仍 re-export）
- 集成测试（`tests/onboarding_integration.rs` 等）零改动
- 与 finding-02 同性质，可一起做或独立做

## 关联

- 决策 `2026-05-16-decision-rust-module-organization.md`
- 决策 `2026-05-18-decision-cli-subcommand-module-layout.md`
- finding-02（同模式）
- finding-05（memmem 删除）
