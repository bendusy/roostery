---
doc_type: audit-index
slug: codex-second-pass
status: active
audit_date: 2026-05-19
auditor: codex (second-opinion)
scope: crates/roostery/ post-refactor 状态（commit 3de735f）
dimensions: [bug, security, performance, maintainability, arch-drift]
tags: [audit, second-opinion, codex, post-0.1.0]
related_audits: [2026-05-18-post-release-rust-idiom]
---

# Codex 第二轮独立审计

## 范围

post-refactor 状态（commit `3de735f`）—— Claude 第一轮 audit 之后做的独立深度审计，重点找 Claude 视角下的盲区。

## 总评

**A → B+**。Claude 第一轮主要看 maintainability / arch-drift 维度，找到 6 条；Codex 二轮在 **correctness / concurrency** 维度找到 10 条 Claude 全部漏掉的问题，**其中 1 条 P1 是架构红线违规**（SAFE_ENV_FORWARD 未真兑现）。

Claude 漏检模式总结：
1. 偏向"代码可读性"维度，忽略运行时正确性（pipe deadlock / panic 路径 / fall-through）
2. 信任注释/字段名（`prep_env`、`Config.journal.dir` 字段存在不等于真生效）
3. 测试同步原语跨文件复制粘贴未抓到（4 处独立 `static ENV_LOCK`）

## 发现清单（10 条）

| # | 维度 | 严重 | 置信 | 标题 | 建议 |
|---|---|---|---|---|---|
| 01 | arch / correctness | **P1** | high | `cc_headless` 缺 `env_clear()`，SAFE_ENV_FORWARD 红线未兑现 | cs-issue |
| 02 | correctness / concurrency | P2 | high | `cc_headless` 子进程大输出 pipe deadlock 触发 timeout | cs-issue |
| 03 | correctness | P2 | high | `cc_headless` args 解析错误静默吞，空 prompt 仍执行消耗预算 | cs-issue |
| 04 | correctness | P2 | high | `timeout_ms` 无上限可让 `Instant::now() + Duration` 溢出 panic | cs-issue |
| 05 | correctness | P2 | high | dispatcher budget load/save 错误吞，预算门可被绕过 | cs-issue |
| 06 | correctness | P2 | high | `total_cost_usd` 反序列化后被丢弃，cost 永远按 None 处理 | cs-issue |
| 07 | correctness | P2 | high | Stop hook 模板 `{{HOOK_SCRIPT}}` 未 shell quote，HOME 含空格破坏 | cs-issue |
| 08 | correctness / arch | P2 | high | `Config.journal.dir` 配置存在但 dispatcher 实际用 `paths::journal_dir()` 写死 | cs-issue |
| 09 | concurrency / test | P2 | high | 多处 integration test 各声明独立 `static ENV_LOCK`，与 attention.md 规约冲突 | cs-issue |
| 10 | correctness | P3 | medium | transcript `content[0].text` 假设第一项必是 text，tool block 在前会误判 | cs-issue |

## 修复状态（2026-05-19）

**全 10 findings 修复完成**：

| Finding | Commit | 修复状态 |
|---|---|---|
| 01 env_clear | `18f0e32` | ✅ resolved |
| 02 pipe deadlock + reader join | `18f0e32` + `1e36d9e` (timeout path) | ✅ resolved |
| 03 args BadArgs | `18f0e32` | ✅ resolved |
| 04 timeout overflow | `18f0e32` | ✅ resolved |
| 05 budget error log | `e290bfa` | ✅ resolved（保留 fallback 设计意图）|
| 06 total_cost_usd | `18f0e32` | ✅ resolved |
| 07 hook shell quote | `f084840` | ✅ resolved |
| 08 cfg.journal.dir | `e290bfa` | ✅ resolved |
| 09 ENV_LOCK 统一 | `57d62cd` | ✅ resolved |
| 10 transcript multi-block | `d4d70ba` | ✅ resolved |

**Round 3 验证（codex 二次确认）**：8/10 ✅ resolved，2 partial 已由 `1e36d9e` + `e290bfa` 补足。

**Round 3 新发现（6 个 Medium，全已处理）**：
| 新发现 | Commit | 状态 |
|---|---|---|
| reader thread join (timeout path) | `1e36d9e` | ✅ |
| LarkError::StdinWriteFailed 传播 | `1e36d9e` | ✅ |
| DispatchStep.fanout_truncated 契约 | `1e36d9e` | ✅ |
| bot_task_writer tmp 唯一名 | `fe0f726` | ✅ |
| MockLarkRunner 记录 RunOptions | `63c2997` | ✅ |
| journal multi-proc append race | — | ⏸ 评估后接受（POSIX 单 syscall 原子）|
| bot_cli_integration fake 矩阵过宽 | — | ⏸ 留后续，不阻塞 |

**Round 4 codex 审计结论**：发现 2 P（1 P1 + 1 P2），全已修复
| 新发现 | Commit | 状态 |
|---|---|---|
| P1 budget 跨进程 RMW race → BudgetGuard flock | `b0185cb` | ✅ |
| P2 reader memory blow-up → drain_with_head_cap | `79f036e` | ✅ |

**Round 5 codex 审计结论**：发现 2 P2（都是 round-4 修复 latent bug），全已修复
| 新发现 | Commit | 状态 |
|---|---|---|
| P2 drain U+FFFD 扩展溢出 cap → push_str 后二次 truncate | `de03eb5` | ✅ |
| P2 BudgetGuard fail-open 退化到 unlocked fresh state → fail-closed GateRejected | `de03eb5` | ✅ |

**Round 6 codex 审计结论**：**STOP CONDITION REACHED**
- round-5 修复无新 latent bug
- 全局扫描无新 P1/P2
- 可进入 feature 推进阶段

## 累计统计

| 轮 | 新 finding | 修复 commit | 累计 |
|---|---|---|---|
| Round 2 | 10 | 18f0e32 / e290bfa / f084840 / 57d62cd / d4d70ba | 10 ✅ |
| Round 3 | 6 + 2 partial | 1e36d9e / fe0f726 / 63c2997 | 6+2 ✅ |
| Round 4 | 2 | b0185cb / 79f036e | 2 ✅ |
| Round 5 | 2 | de03eb5 | 2 ✅ |
| Round 6 | 0 | — | stop |
| **合计** | **22 P findings** | **10 fix commits** | **全 ✅** |

测试覆盖：~390 lib + 集成测试 → **418 lib + 集成测试**（+28 新覆盖 finding 行为）。
CI：每次 push 全绿，最终 commit `de03eb5` 仍绿。

## 修复优先级建议

**第一波（P1 + 安全 / 红线类，立刻修）**：
- finding-01 env allowlist — 红线违规
- finding-07 shell quote — 安全
- finding-09 ENV_LOCK — 直接影响 attention.md 已记录的 flake 规约

**第二波（correctness 类，分批修）**：
- finding-02 pipe deadlock
- finding-04 panic 路径
- finding-05 budget 静默吞错
- finding-03 args 解析静默
- finding-06 cost 丢失
- finding-08 config.journal.dir 失效

**第三波（P3）**：
- finding-10 transcript content 多 block 适配

## 下一步

按"第一波 → 第二波 → 第三波"顺序开 issue 修复。每条走 cs-issue 快速通道（根因明确 + 改动小 + 单文件）或标准路径（涉及测试架构改动）。
