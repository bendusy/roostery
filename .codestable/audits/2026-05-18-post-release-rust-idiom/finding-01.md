---
doc_type: audit-finding
slug: dispatcher-process-one-gate-repetition
audit: 2026-05-18-post-release-rust-idiom
dimension: maintainability
severity: P1
confidence: high
suggested_action: cs-refactor
tags: [dispatcher, rust-idiom, function-decomposition, error-handling]
---

# Finding 01：`dispatcher::process_one` 5 处 gate 模式重复，201 行单函数

## 位置

`crates/roostery/src/dispatcher/mod.rs:157-358`（`async fn process_one`，201 行）

## 证据

5 个 gate（trace.check_depth / rules.matches / budget.check / runaway.record / registry.find）+ runner.run + budget.consume，每个 gate 失败路径用同一模板写：

```rust
// Gate 1: trace.check_depth (line 179)
if let Err(e) = ctx.check_depth() {
    return finalize_step(
        journal,
        entry,
        DispatchStep {
            event_id,
            hook_source,
            depth,
            matched_rule: None,
            runner_kind: None,
            status: StepStatus::GateRejected {
                reason: e.to_string(),
            },
            fanout: 0,
        },
    );
}

// rules.matches NoMatch 分支 (line 199-218) — 同模板 + 不同 reason
// Gate 2: budget.check_or_raise (line 220-239) — 同模板
// Gate 3: runaway.record (line 241-267) — 同模板
// registry.find None 分支 (line 269-290) — 同模板
// runner.run Err 分支 (line 297-...) — 同模板
```

`finalize_step(...)` 调用站点出现 **6 次**，每次构造 `DispatchStep { event_id, hook_source, depth, matched_rule, runner_kind, status, fanout }` 几乎一致，差别只在 `status` 内的 `reason` 字符串。

## 为什么构成问题

1. **改动放大**：未来加新 gate / 新维度（如 design §6 提的 `BotPushRunner` 适配器）要重复同样的 5 字段 + reason 模板；改 `DispatchStep` 字段时 6 处都得改。
2. **可读性低**：主流程 5 个业务步骤被 5 段错误处理 boilerplate 撑到 201 行，reader 要扫 60% 的代码才能找到"真正在做什么"。
3. **Rust 惯用法落差**：5 处都是 `if let Err(e) = ...` 的命令式写法，Rust 的 `?` operator + `Result::map_err` 组合可以让每个 gate 缩成一行。

## 建议改法（不在本审计动手，留给 cs-refactor）

**Step 1** — 抽取 `reject_step` helper：

```rust
fn reject_step(
    journal: &Journal,
    entry: JournalEntry,
    base: DispatchStepBase,  // event_id / hook_source / depth 三字段打包
    reason: impl Into<String>,
) -> DispatchStep {
    finalize_step(journal, entry, DispatchStep::gate_rejected(base, reason.into()))
}
```

**Step 2** — 把 `process_one` 主体改成 `?` 链：

```rust
async fn process_one(...) -> DispatchStep {
    let base = DispatchStepBase::new(event_id.clone(), hook_source.clone(), depth);
    let entry = build_entry(&event, &ctx, &event_id);

    ctx.check_depth()
        .map_err(|e| reject_step(journal, entry.clone(), base.clone(), e.to_string()))?;
    let m = rules::matches(rules, &event)
        .ok_or_else(|| reject_step(journal, entry.clone(), base.clone(), "NoMatch"))?;
    // ... 其余 gate 同样模式
}
```

返回类型从 `DispatchStep` 改为 `Result<DispatchStep, DispatchStep>`（Ok = 走通主路径，Err = gate 拒绝），最后 `.unwrap_or_else(|e| e)` 收口；或者用 `try_blocks` nightly 特性等价表达（不要现在用，等稳定）。

**预期效果**：201 行 → 80-100 行；新加 gate 1 行 `?` 调用即可，不再有 6 字段 struct 构造模板。

## 影响范围

- 单文件改动（`dispatcher/mod.rs`）
- 公开 API 不变（`fire` / `replay` / `test_rule` 三入口签名零变化）
- 测试零改动（行为不变；现有 `mod tests` 14 测试应继续过）
- 这是典型 design §2.5 "只搬不改行为" 的微重构边界

## 关联

- 决策 `2026-05-16-decision-rust-module-organization.md`（模块组织 convention）
- ARCHITECTURE.md §3 Module E 描述了 `process_one` 的 5 gate 不变量——重构不能改这个不变量，只能改表达方式
