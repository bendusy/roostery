---
doc_type: refactor-design
refactor: 2026-05-19-bot-stop-hook-split
status: approved
scope: crates/roostery/src/bot_stop_hook.rs（产品 725 行 + 测试 738 行 = 1463 行单文件）
summary: 拆 bot_stop_hook.rs 为 bot_stop_hook/ 子目录（mod / types / util / stop_input / push / cli 六文件），与 lark_cli/ + dispatcher/ 同模式；纯文件 split + 路径更新，行为零变化
related_audit: 2026-05-18-post-release-rust-idiom
related_finding: finding-02
---

# bot_stop_hook split refactor design

## 1. 本次范围

### 做什么

把 `crates/roostery/src/bot_stop_hook.rs`（1463 行）按已有的 4 大块边界拆成 `crates/roostery/src/bot_stop_hook/` 子目录：

```
crates/roostery/src/bot_stop_hook/
├── mod.rs          # pub re-export + 模块声明
├── types.rs        # PushRequest / PushOptions / PushOutcome / PushStatus
├── util.rs         # truncate_utf8 / cwd_basename / stable_idem_key /
│                   #   resolve_receive_id / 常量 / transcript_reader 子模块
├── stop_input.rs   # StopHookInput + resolve_summary_from_hook_input
├── push.rs         # push / finish_with_fallback / run_stop_hook /
│                   #   parse_stop_hook_input / build_request_from_stop_hook_input
└── cli.rs          # BotArgs / BotSub / PushCliArgs / StopHookCliArgs / run
```

### 不做

- ❌ 不改公开 API（`bot_stop_hook::cli::BotArgs` / `bot_stop_hook::cli::run` / `bot_stop_hook::push` / `bot_stop_hook::run_stop_hook` 等签名零变化）
- ❌ 不改函数体（每个 fn 体 byte-for-byte 搬运）
- ❌ 不顺手优化（finding-06 的 7-arg 重复留独立 refactor 做；本次只 split）
- ❌ 不改测试断言（测试函数随业务下沉到对应文件的 `#[cfg(test)] mod tests`）

### 工作量 / 风险

- **工作量**：3-4 个 Edit/Write 操作（每个子文件一次），1 次 Bash 删原文件
- **风险**：低。纯机械搬运，行为等价由 `cargo test --all` 自证；`pub(crate)` 边界需要正确暴露（util 函数被 push / stop_input 共用 → 必须 pub(crate)；不能 pub 也不能 pub(self)）
- **复杂度档位**：标准（不是 fastforward — 跨多文件；不需 Parallel Change — 公开 API 不变）

## 2. 前置依赖

无。`cargo test --all` 全绿（最近 commit `1231ee3` 后已验证 436+ tests pass），覆盖范围足够。

## 3. 执行顺序

### 步骤 1：创建 `bot_stop_hook/mod.rs` 骨架 + 6 个子文件占位

**方法**：M-L3-Split（File Split）

**动作**：
- 在 `bot_stop_hook/` 新建 6 个 .rs 文件
- `mod.rs` 暂时只含 `mod types; mod util; mod stop_input; mod push; mod cli;` + `pub use` re-export
- 各子文件初始空内容 + 文件顶部模块 doc
- **暂不删** `bot_stop_hook.rs`——避免编译同名冲突，下一步分块搬完再删

**退出信号**：6 个文件存在；`mod.rs` 不被 `lib.rs` 引用（旧 `bot_stop_hook.rs` 仍工作）

**验证**：AI 自证 `cargo build` 仍绿（新文件未参与编译）

**回滚**：`rm -r crates/roostery/src/bot_stop_hook/`

### 步骤 2：搬运到子文件 + 删原文件 + 切 lib.rs

**方法**：M-L3-Split（File Split）

**动作**：
- `types.rs`：搬 line 1-140 的 `PushRequest / PushOptions / PushOutcome / PushStatus` + 相关 impl + use
- `util.rs`：搬 truncate_utf8 / cwd_basename / stable_idem_key / resolve_receive_id / SUMMARY_MAX_BYTES / DEFAULT_SUMMARY / transcript_reader 子模块 + use
- `stop_input.rs`：搬 `StopHookInput` struct + `resolve_summary_from_hook_input` + use
- `push.rs`：搬 `push` / `finish_with_fallback` / `run_stop_hook` / `parse_stop_hook_input` / `build_request_from_stop_hook_input` + use
- `cli.rs`：搬 `pub mod cli { ... }` 内全部内容（提到 module 顶层）+ use
- 各子文件的 `#[cfg(test)] mod tests` 跟着对应业务下沉
- `mod.rs` 完成 `pub use`：`pub use push::{push, run_stop_hook};` 等，保住公开 API 字面兼容
- 删 `crates/roostery/src/bot_stop_hook.rs` 原文件
- `lib.rs` 模块声明 `pub mod bot_stop_hook;` 不变（指向 `bot_stop_hook/mod.rs`）

**退出信号**：
- `cargo build` 绿
- `cargo clippy --all-targets --all-features -- -D warnings` 0 警告
- `cargo test --all` 全绿（436+ tests 不减不增）
- `git diff --stat` 显示 `bot_stop_hook.rs` 删除 + 6 新文件创建，行数总和 ≈ 1463 ± 30（搬运 + 必要的 use 调整）

**验证**：AI 自证三命令 + grep 公开 API 仍在（`grep 'bot_stop_hook::cli::\|bot_stop_hook::push' crates/roostery/src/`）

**回滚**：`git restore --source=HEAD --staged --worktree crates/roostery/src/bot_stop_hook.rs crates/roostery/src/bot_stop_hook/ && rm -r crates/roostery/src/bot_stop_hook/`

### 步骤 3：reformat + 终验

**方法**：M-L2-Mechanical（机械整理）

**动作**：
- `cargo fmt --all`
- 跑 `cargo fmt --all --check / clippy / test` 三连验
- grep 反向核对：
  - 公开 API 仍可见：`grep -rn 'bot_stop_hook::cli::BotArgs\|bot_stop_hook::cli::run' crates/roostery/src/main.rs` 应 = 2 命中
  - 旧文件不复现：`ls crates/roostery/src/bot_stop_hook.rs` 应报 not found
  - 各文件大小合理：每个 < 500 行

**退出信号**：四绿（fmt / clippy / test --all / test --doc）+ grep 反向核对全 pass

**验证**：AI 自证

**回滚**：步骤 2 的回滚 + `cargo fmt` 不可逆但无害

## 4. 风险与看点

### 高风险

- **`pub(crate)` 边界**：搬到子模块后，原 module-private 工具（如 `truncate_utf8`）需要 `pub(crate)` 才能跨子模块互调。漏改会编译失败但 rustc 报错明确，立刻能修
- **`use` 导入**：每个子文件的 `use` 需要重新整理（原文件顶部一堆 import 要分散到各子文件，按各自所需）。漏导致 unresolved import，rustc 报错明确
- **测试随业务下沉**：原文件多个 `#[cfg(test)] mod tests` 分布在多业务块旁。每个 `mod tests` 跟着搬到对应子文件，**不合并**。这维持了文件内"业务 + 测试相邻"的认知顺序

### 低风险

- 已有 436+ tests 包括 4 个集成测试（`tests/bot_cli_integration.rs`）覆盖公开 API；任何外部行为破坏会被立刻捕获
- 公开 API 字面不变，`main.rs` 路径不调整

### 不会发生的事

- 不会改函数签名（行为等价是 refactor 底线）
- 不会引入新依赖
- 不会改 templates/ 或 sh wrapper

## 5. 完成判据

- [ ] 步骤 1-3 全部 done
- [ ] `cargo fmt --all --check / cargo clippy -D warnings / cargo test --all / cargo test --doc` 四绿
- [ ] `main.rs:38,139` 公开 API 引用未改
- [ ] 原 `bot_stop_hook.rs` 已删
- [ ] 6 个子文件存在且各 < 500 行
- [ ] `apply-notes.md` 记录每步验证日志
