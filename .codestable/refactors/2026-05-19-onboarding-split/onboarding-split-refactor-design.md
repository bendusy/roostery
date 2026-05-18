---
doc_type: refactor-design
refactor: 2026-05-19-onboarding-split
status: approved
scope: crates/roostery/src/onboarding.rs（产品 632 行 + 测试 435 行 = 1067 行）
summary: 拆 onboarding.rs 为 onboarding/ 子目录（mod / types / shim / hooks / lark_cli_override / env_rc），同时 inline memmem helper 删除（finding-05）
related_audit: 2026-05-18-post-release-rust-idiom
related_findings: [finding-04, finding-05]
---

# onboarding split refactor design

## 1. 范围

### 做什么

**finding-04**：按已有 7 块业务边界拆 `crates/roostery/src/onboarding.rs` 为 `crates/roostery/src/onboarding/` 子目录：

```
crates/roostery/src/onboarding/
├── mod.rs             # OnboardingError + run() + format_report + 小 helpers
│                      # (create_dir_all / home_join / set_executable) + pub use re-export
├── types.rs           # ShellKind / SkipReason / InitOptions / RealLarkCliSource / InitReport
├── shim.rs            # install_shim + file_sha256 + looks_like_roostery_shim
├── hooks.rs           # write_sh_bridge + merge_hooks_for
├── lark_cli_override.rs  # resolve_real_lark_cli + validate_override
└── env_rc.rs          # write_env_file + shell_quote + patch_shell_rc
```

**finding-05**：在 `shim.rs` 落点同时 inline `memmem` helper 到 `looks_like_roostery_shim` 唯一调用点（不再单独定义函数）。

**与 audit finding-04 偏差**：audit 提议有 `shell.rs`，但 `ShellKind` 与 `InitOptions` 关系比与 `shell_quote` 紧（前者是装机 API 类型，后者只服务 `patch_shell_rc`），所以：
- `ShellKind` 归 `types.rs`（与其他装机 API 类型）
- `shell_quote` 与 `patch_shell_rc` 同住 `env_rc.rs`（co-locate 单一使用者）

### 不做

- ❌ 不改公开 API：`onboarding::run` / `InitOptions` / `SkipReason` / `format_report` / `OnboardingError` 全部通过 `mod.rs` `pub use` 保持原路径
- ❌ 不改函数体（除 finding-05 inline memmem 一处）
- ❌ 不顺手做其他优化
- ❌ 不改测试断言；测试随业务下沉到对应子文件 `#[cfg(test)] mod tests`

### 工作量 / 风险

- **工作量**：写 6 新文件 + 删 1 旧文件，~1-2 小时
- **风险**：低。`pub(super)` / `pub(crate)` 边界需正确（onboarding 模块内部 helper 跨子模块互调）
- **行为等价证据**：`cargo test --all`（含集成测 `tests/onboarding_integration.rs`）全绿

## 2. 前置依赖

无。最近 commit `bdfe86d` 后 460+ tests 全绿。

## 3. 执行顺序

**S1+S2 合并执行**（同 bot-stop-hook split 经验：rustc E0761 禁止 `onboarding.rs` 与 `onboarding/mod.rs` 并存）：

### 步骤 1：原子搬运 + finding-05 inline memmem

- 删除 `crates/roostery/src/onboarding.rs`
- 创建 `crates/roostery/src/onboarding/` 下 6 文件
- 各文件函数体 byte-for-byte 搬运；`memmem` 函数体 inline 到 `looks_like_roostery_shim` 的 `.windows().any()` 调用
- 跨子模块 helper 用 `pub(super)` / `pub(crate)`
- 测试随业务下沉

**退出信号**：cargo build / clippy / test --all 三绿

**验证**：AI 自证

### 步骤 2：fmt + grep 反向核对

- `cargo fmt --all`
- grep 验证：
  - `main.rs:10,342,345` + `tests/onboarding_integration.rs` 引用未改
  - 旧 `onboarding.rs` 不存在
  - `grep -rn 'fn memmem' crates/roostery/src/` 返 0
  - 各子文件 < 350 行

**退出信号**：fmt-check + clippy + test --all + test --doc 四绿；grep 反向核对全 pass

**验证**：AI 自证

## 4. 风险与看点

- **同 bot-stop-hook split**：rustc E0761 强制 S1+S2 合并，已知约束
- **`OnboardingError` 跨子模块**：作为模块中心错误类型，需 `pub` 露在 `mod.rs`，各子文件用 `use super::OnboardingError`
- **`onboarding_integration.rs` 集成测试**：4+ 测试覆盖 `run()` E2E，是行为等价的最强证据
- **`hooks_merge` 模块依赖**：onboarding 调 `hooks_merge::*`；拆分后路径不变（`crate::hooks_merge::...`）
- **finding-05 改动验证**：原 `memmem(&buf, b"roostery shim")` → `buf.windows(b"roostery shim".len()).any(|w| w == b"roostery shim")`，行为等价由 `looks_like_roostery_shim` 相关测试覆盖（如有；否则由 install_shim 集成测试覆盖）

## 5. 完成判据

- [ ] 步骤 1-2 全 done
- [ ] cargo fmt-check / clippy -D warnings / test --all / test --doc 四绿
- [ ] main.rs + tests/onboarding_integration.rs 公开 API 引用未改
- [ ] 原 onboarding.rs 已删
- [ ] memmem 函数已删（inline 到唯一调用点）
- [ ] 各子文件 < 350 行
- [ ] apply-notes 记录每步验证
