---
doc_type: refactor-apply-notes
refactor: 2026-05-19-bot-stop-hook-split
---

# bot_stop_hook split apply notes

## 偏离 design

**S1+S2 合并执行**：design 把"创建骨架"与"分块搬运"列为两个 step，但 rustc E0761 不允许 `bot_stop_hook.rs` 与 `bot_stop_hook/mod.rs` 并存——`pub mod bot_stop_hook;` 解析时两路径冲突。S1 一旦在 `bot_stop_hook/` 下放 `mod.rs`，旧文件就编译失败。

**实际执行**：合并为单原子动作"删旧文件 + 写 6 新文件"。这与 design 边界（"只搬不改"）一致，与 design §2.5 微重构 "拆文件" 验证条件一致：编译绿灯 + 现有测试通过 + 对外接口签名零 diff。

## 步骤 S1+S2: 删原文件 + 写 6 新文件

- 完成时间: 2026-05-19
- 改动文件:
  - 删除：`crates/roostery/src/bot_stop_hook.rs`（1463 行）
  - 新建：`crates/roostery/src/bot_stop_hook/mod.rs`（59 行）
  - 新建：`crates/roostery/src/bot_stop_hook/types.rs`（174 行）
  - 新建：`crates/roostery/src/bot_stop_hook/stop_input.rs`（246 行）
  - 新建：`crates/roostery/src/bot_stop_hook/util.rs`（216 行）
  - 新建：`crates/roostery/src/bot_stop_hook/push.rs`（480 行）
  - 新建：`crates/roostery/src/bot_stop_hook/cli.rs`（316 行）
- 总行数：1463 → 1491 (+28 行，绝大部分是各子文件顶部的 `use` import + 模块 docstring + cross-mod 测试 helper 模块的开销)
- 验证结果：
  - `cargo build` 绿（1.48s）
  - `cargo clippy --all-targets --all-features -- -D warnings` 0 警告
  - `cargo test --all` 全绿（390 lib + 12 bot_cli_integration + 多个集成测试，总计 460+ tests 0 failed）
- 偏离：S1+S2 合并执行（rustc 约束，见上）

## 步骤 S3: reformat + 终验

- 完成时间: 2026-05-19
- 改动文件: `cargo fmt --all` 自动整理 `bot_stop_hook/push.rs` 和 `dispatcher/mod.rs` 的 use 语句 / 长行换行（cosmetic only，零语义变化）
- 验证结果：
  - `cargo fmt --all --check` pass
  - `cargo clippy -D warnings` 0 警告
  - `cargo test --all` 全绿 460+ tests
  - `cargo test --doc` 4 doctests 通过
  - grep 反向核对：
    - `main.rs:38,139` 公开 API 引用未改（`bot_stop_hook::cli::BotArgs` / `bot_stop_hook::cli::run`）
    - 旧 `bot_stop_hook.rs` 不复现（`ls` 报 not found）
    - 各子文件大小：cli 316 / push 480 / stop_input 246 / util 216 / types 174 / mod 59，全 < 500 行
- 偏离：无

## 行为等价自检

- ✅ 公开 API 字面兼容：`bot_stop_hook::push` / `run_stop_hook` / `cli::run` / 所有 `pub struct` (`PushRequest` / `PushOptions` / `PushOutcome` / `PushStatus`) 通过 `mod.rs` `pub use` re-export 保持原路径可访问
- ✅ 函数体 byte-for-byte 搬运（除 `cargo fmt` 的 use 换行外）；无新增 / 删除业务逻辑
- ✅ 测试随业务下沉但**测试函数体不改**；测试覆盖矩阵不变；新增的 `test_helpers` 子模块只是把原文件内联的 4 个 helper（`install_tempdir_as_home` / `write_config_with_user_id` / `task_create_response` / `im_send_response`）抽出共享，原来这些 helper 在 `mod tests` 内联，多个 test fn 已经共用——纯组织调整无新行为
- ✅ `cargo test --all` 测试总数前后一致（390 lib tests，与重构前的 commit `1231ee3` 相同）
- ✅ 集成测试 `tests/bot_cli_integration.rs` 4 个 e2e 测试通过，证明 CLI 入口外部可见行为不变

## 顺手发现

无。本次严格 "只搬不改"，未顺手做 finding-06（`push` 7-arg `finish_with_fallback` 重复）— 留独立 refactor。

## 最终状态

- 改动文件清单（`git status --short`）：
  ```
  D  crates/roostery/src/bot_stop_hook.rs
  A  crates/roostery/src/bot_stop_hook/mod.rs
  A  crates/roostery/src/bot_stop_hook/types.rs
  A  crates/roostery/src/bot_stop_hook/stop_input.rs
  A  crates/roostery/src/bot_stop_hook/util.rs
  A  crates/roostery/src/bot_stop_hook/push.rs
  A  crates/roostery/src/bot_stop_hook/cli.rs
  ```
- 四绿状态：fmt-check ✓ / clippy ✓ / test --all ✓ / test --doc ✓
- 公开 API 零变化（已 grep 验证）
- 与 `lark_cli/` + `dispatcher/` 同模块组织模式对齐
