---
doc_type: audit-finding
slug: dispatcher-ghost-code-stubs
audit: 2026-05-18-post-release-rust-idiom
dimension: arch-drift
severity: P1
confidence: high
suggested_action: cs-issue
tags: [dispatcher, ghost-code, defensive-rules, cleanup]
---

# Finding 03：`dispatcher/mod.rs:415-423` ghost-code stub 违反 defensive-rules

## 位置

`crates/roostery/src/dispatcher/mod.rs:413-423`

## 证据

```rust
// suppress unused-warnings until S5 wires journal dir / paths into replay
#[allow(dead_code)]
fn _unused_pathbuf_hint() -> PathBuf {
    PathBuf::new()
}

// keep TraceError / RunnerError imports referenced for clarity (errors are
// matched-on inline via to_string()); silence "unused import" if any.
#[allow(dead_code)]
fn _unused_error_imports(_t: TraceError, _r: RunnerError) {}
```

两个函数：
- `_unused_pathbuf_hint`：返回空 `PathBuf`，被 `#[allow(dead_code)]` 标注，**目的是抑制 import unused warning**
- `_unused_error_imports`：吃两个错误类型参数返回 `()`，**目的是让 `TraceError` 和 `RunnerError` import 不被警告**

## 为什么构成问题

1. **违反 CLAUDE 用户规则 `claude-code-defensive.md` §5 幽灵代码**：
   > ❌ 用"注释掉旧实现"代替删除
   > ❌ 提交调试残留：`print`/`console.log`/`debugger`

   这两个 stub 是同种问题的不同形态——**用占位函数代替删除未用 import**。注释自承"suppress unused-warnings"——把抑制警告当成"代替删除"的手段。

2. **注释承诺已过期**：第一段注释说 "until S5 wires journal dir / paths into replay"——但 dispatcher-loop feature 已 S5 accept，`replay` 函数现存于 line 428（已用 `PathBuf` 真消费 journal dir）。stub 应在 S5 accept 时删除，未删 = forgotten cleanup。

3. **第二段注释逻辑不成立**：注释说 `TraceError` / `RunnerError` "via to_string() inline"——`grep TraceError\|RunnerError crates/roostery/src/dispatcher/mod.rs` 显示这两类型在 `process_one` 内多处 `match` / 通过 `e.to_string()` 真实使用。stub 是冗余的——直接删除 import + stub 应该不影响编译。

4. **架构红线对齐**：`.codestable/architecture/ARCHITECTURE.md` §5 关键架构决定 + §6 已知约束都强调"代码必须真实表达意图"。ghost-code stub 是相反信号。

## 验证步骤（开 issue 时跑）

```bash
# 1. 删两个 stub fn 和 #[allow(dead_code)] 行
# 2. 看 import 是否真的需要：
cargo check 2>&1 | grep -E 'unused|TraceError|RunnerError'
# 3. 如果 import 真未用 → 一并删；如果有警告 → 修正使用点直接 reference 类型
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
```

预期：单 commit 删 11 行，全绿。

## 建议改法（cs-issue 流程）

按 issue 工作流：`cs-issue-report` 一句话写问题，然后 fast-path `cs-issue-fix`（不需要 analyze 阶段，root cause 已在本审计写明）：

1. 删除 `_unused_pathbuf_hint` / `_unused_error_imports` 两个 stub 函数（line 413-423）
2. 跑 `cargo check` 看 import 是否真不需要：
   - 真不需要 → 一并删 import
   - 真需要 → 类型在 process_one 里有真实使用，import 应保留（删 stub 即可）
3. `cargo clippy -D warnings` + `cargo test --all` 验证

## 影响范围

- 改动量：≤ 15 行删除
- 公开 API 零变化
- 行为零变化（dead code 本来就没执行）

## 关联

- 用户私人规则 `~/.claude/rules/claude-code-defensive.md` §5
- ARCHITECTURE.md §5 关键架构决定 / §6 已知约束 / 硬边界
- feature `2026-05-18-dispatcher-loop` 的 S5 应该清掉这些 stub 但漏了
