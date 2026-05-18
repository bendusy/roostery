---
doc_type: audit-index
slug: post-release-rust-idiom
status: active
audit_date: 2026-05-18
scope: crates/roostery/src/ + crates/roostery/tests/ (~13k 行 / 39 文件)
dimensions: [bug, security, performance, maintainability, arch-drift]
tags: [audit, post-0.1.0, rust-idiom, refactor-candidates]
---

# 0.1.0 后 Rust 惯用法重构机会审计

## 范围

- **目标**：`crates/roostery/src/` 活跃 Rust 代码 + `crates/roostery/tests/` 集成测试
- **维度**：bug / security / performance / maintainability / arch-drift 五维全扫
- **重点**：用户明确要求"在 Rust 特长下重构"——优先 maintainability + arch-drift 维度
- **背景**：0.1.0 已 tag（commit `49a0f37`，2026-05-18），21 feature 累积约 13k 行；首次系统性回看代码质量与 Rust 惯用法落差

## 总评

**整体健康度：A-**。架构红线全部守住 / 测试覆盖 436+ test 全绿 / 无 unsafe 误用 / 无 TODO 残留 / 模块映射与 ARCHITECTURE.md 全对齐。

但快速增量交付节奏下，**单文件 size 与函数复杂度积累的债已显**：3 个文件超 1000 行（最大 1463），dispatcher 主流程 `process_one` 单函数 201 行 5 处重复 gate 模式，1 处明确 `_unused_*` ghost code stub。这些不影响功能但显著拉低未来 feature 加新 gate / 新 hook input 时的可维护性。

**没发现的事**（同等重要）：
- 没有 P0 安全 / bug / 数据丢失
- 没有架构红线违规（`grep reqwest|Command::new("lark-cli")` 干净）
- 没有 production 代码 `.unwrap()` 滥用（最多文件 9 处，9 处全是"validated by earlier check"的 invariant assertion，合理）
- 没有 N+1 / 死循环 / 资源泄漏的明显嫌疑

**结论**：是健康项目的"成长债"，不是"危机债"。0.2.0 推进前花 1-2 天清这批 P1 投资回报很高，会让 0.2.0+ 加 feature 时 dispatcher / bot 模块明显更舒服。

## 发现清单

| # | 维度 | 严重 | 置信 | 标题 | 建议 |
|---|---|---|---|---|---|
| 01 | maintainability | P1 | high | `dispatcher::process_one` 5 处 gate 模式重复，201 行单函数 | cs-refactor |
| 02 | maintainability | P1 | high | `bot_stop_hook.rs` 1463 行单文件 4 大块职责未拆 | cs-refactor |
| 03 | arch-drift / maintainability | P1 | high | `dispatcher/mod.rs:415-423` 明确 ghost-code stub `_unused_*` 违反 defensive 规则 | cs-issue |
| 04 | maintainability | P2 | high | `onboarding.rs` 1067 行单文件 7 块职责未拆 | cs-refactor |
| 05 | maintainability | P2 | high | `onboarding::memmem` 重新包裹 `<[u8]>::windows().any()` 的无效抽象 | cs-refactor |
| 06 | maintainability | P2 | medium | `bot_stop_hook::push` 7-arg `finish_with_fallback` 调用位重复 | cs-refactor |

**维度统计**：bug 0 / security 0 / performance 0 / maintainability 5 / arch-drift 1（finding-03 重叠）。

每条详细见 `finding-01.md` 起。

## 下一步建议（按性价比排序）

1. **优先：finding-03（ghost-code stub 清理）** — 5 行删除 + 调试器跑测试，半小时搞定，立刻消除 defensive-rules 违规。直接 `cs-issue` 开 issue 修。
2. **高 ROI：finding-01（process_one 重构）** — 提取 `reject_step()` helper，201 行 → ~80 行，5 个 gate 都用 `?` operator + closure。`cs-refactor`，预计半天，完成后未来加新 gate 改动量减半。
3. **高 ROI：finding-02（bot_stop_hook.rs 拆分）** — 按已分块（types / stop_input / push / cli）拆成 `bot_stop_hook/` 子目录，与 `lark_cli/` / `dispatcher/` 同模式。`cs-refactor`，预计 1-2 小时（机械拆分 + `mod.rs` re-export），完成后下次加 `bot-bridge-cluster` feature 不在巨型文件里挤。
4. **批量 P2**：finding-04/05/06 一起做一轮 `cs-refactor`，预计半天。完成后 onboarding 模块结构与 dispatcher / lark_cli 等其他多概念模块对齐。

如果只做一件事 → 做 finding-01（process_one 重构）。Phase 4 dispatcher 是项目逻辑最复杂模块，未来加 budget 维度 / 新 Action 类型时这里会反复改，重构后改动量预计减 60%。
