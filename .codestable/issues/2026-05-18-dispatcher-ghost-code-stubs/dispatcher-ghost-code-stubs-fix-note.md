---
doc_type: issue-fix
issue: 2026-05-18-dispatcher-ghost-code-stubs
status: fixed
severity: P1
path: quick-track
summary: 删除 dispatcher/mod.rs 的 _unused_pathbuf_hint / _unused_error_imports 两个 ghost-code stub 并同步清掉只被它们喂养的 dead import（PathBuf / TraceError / RunnerError）
tags: [dispatcher, ghost-code, defensive-rules, cleanup, audit-finding-03]
related_audit: 2026-05-18-post-release-rust-idiom
related_finding: finding-03
---

# Dispatcher Ghost-Code Stub Cleanup

## 来源

`cs-audit` 2026-05-18-post-release-rust-idiom 的 finding-03。审计阶段已读代码定位根因 + 给出修复方案，本 issue 走快速通道：跳过 `report.md` / `analysis.md`，只产 `fix-note.md`。

## 根因

`crates/roostery/src/dispatcher/mod.rs:413-423` 存在两个 `#[allow(dead_code)]` 标注的占位函数：

```rust
fn _unused_pathbuf_hint() -> PathBuf {
    PathBuf::new()
}

fn _unused_error_imports(_t: TraceError, _r: RunnerError) {}
```

注释自承"suppress unused-warnings until S5 wires journal dir / paths into replay"。但 `dispatcher-loop` feature S5 已 accept（replay 函数现于 line 428 真消费 journal dir），stub 应在 S5 accept 时一并清掉，**漏了**。

这违反 `~/.claude/rules/claude-code-defensive.md` §5 "幽灵代码"——"用占位函数代替删除未用 import" 是同种问题的不同形态。

## 修复

单文件改动 `crates/roostery/src/dispatcher/mod.rs`，14 行删除 / 2 行替换：

1. 删除 `_unused_pathbuf_hint` 函数及其前注释（5 行）
2. 删除 `_unused_error_imports` 函数及其前注释（4 行）
3. 同步清掉只被这两个 stub 引用的 dead import：
   - `use self::runners::{..., RunnerError, ...}` → 去掉 `RunnerError`
   - `use self::trace::{..., TraceError, ...}` → 去掉 `TraceError`
   - `use std::path::PathBuf;` → 整行删除（line 87 的真实使用走 `std::path::PathBuf` 全限定路径）

**做了什么 / 没做什么**：
- ✅ 删 stub + 删 dead import 全做完
- ❌ 没顺手做 finding-01（process_one 重构）—— 那是独立 cs-refactor 工作
- ❌ 没顺手做 finding-02（bot_stop_hook 拆分）—— 独立 cs-refactor

## 验证

```
$ cargo build           → Finished `dev` profile in 2.72s
$ cargo clippy --all-targets --all-features -- -D warnings → 0 warnings
$ cargo test --all      → 436+ tests passed, 0 failed
$ grep '_unused_' crates/roostery/src/dispatcher/mod.rs → 0 lines
$ git diff --stat       → 1 file changed, 2 insertions(+), 14 deletions(-)
```

行为零变化：dispatcher 主链路（fire / process_one / replay / test_rule）签名与逻辑完全未触碰；dead code 本就不执行。

## 学习

**捕获机制建议**：feature accept 时如果在某 feature 留了"未来 S5 接通后清理"类临时 stub，应该在 acceptance 报告的"遗留"段显式记一笔提醒下个 feature accept 时清理。这次 stub 是 dispatcher-trace-budget / dispatcher-rules / dispatcher-runners 等早期子 feature 累积下来的占位（每加一个就在某处放占位防 unused-import 告警），dispatcher-loop S5 accept 时本应一并清理但当时焦点在 process_one 串联，漏看了文件底部。

不到沉淀 learning / decide 的级别（具体局部清理事件），但可以是 cs-feat-accept 工作流文档里一条"反向核对" hint——"是否还有该 feature 引入的占位 stub 未清理？grep `_unused_` / `#[allow(dead_code)]` 看一下"。这条 hint 是否值得加，留 user 拍板。

## 顺手发现

无。改动严格控制在 dispatcher/mod.rs。

## 影响范围

- 文件：`crates/roostery/src/dispatcher/mod.rs`（1 文件 16 行 diff）
- 公开 API：零变化
- 行为：零变化
- 测试：零改动（436+ 测试继续过）
