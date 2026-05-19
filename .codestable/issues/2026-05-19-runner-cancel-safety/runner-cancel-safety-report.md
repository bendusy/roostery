---
doc_type: issue-report
issue: 2026-05-19-runner-cancel-safety
status: open
severity: P1
summary: bot_bridge::runner::handle_event 收到 HITL Abort/Adjust 时只是 drop runner future，没有显式 kill 底层子进程。bridge 侧报告已 aborted/restarted，但原 runner 进程仍在消耗 budget / 写文件 / 占用资源。runner-tmux-launch decision 落地前的中间态问题。
tags: [bot_bridge, runner, hitl, cancel-safety, runner-trait]
discovered_during: codex audit 2026-05-19 round-7 P1-1
related_features: [2026-05-19-bot-bridge-cluster]
related_decisions: [2026-05-19-decision-runtime-launch-strategy]
---

# Runner cancel-safety — handle_event drop ≠ kill child process

## 1. 问题现象

`bot_bridge::runner::handle_event` 在 `tokio::select!` 内 await runner_future
和 kill_rx；HITL Abort 命中时 select 退出循环并 break HandleOutcome::Aborted，
**未显式调用** runner 的 kill / cancel 方法。runner_future 的 Drop 实现是
默认（即把 future 状态扔掉）——但底层若已 spawn 子进程（如 `CcHeadlessRunner`
用 `tokio::process::Command`），子进程不会随 future drop 自动 kill。

后果：用户 `/stop` 命中后，飞书侧报告 ⚠️ aborted，但本机 `cc` / `codex` 子
进程仍在跑，继续：
- 消耗 budget（cost_usd 累计已 commit）
- 写工作目录文件（用户可能后悔但已晚）
- 占 cpu / 内存 / 文件句柄

## 2. 触发路径

```
1. user @bot do task                → handle_event spawn runner_future
2. user /stop                        → daemon dispatch_hitl_abort
                                     → active.send_signal(run_id, Abort)
                                     → kill_tx.send(Abort) 命中
                                     → handle_event select biased → kill_rx 分支
                                     → break HandleOutcome::Aborted (drop runner_future)
3. dispatcher::runners::Runner trait 内 spawn 的子进程没 kill
4. runner 子进程持续 cpu，最终自然完成或永跑
```

## 3. 根因

`dispatcher::runners::Runner` trait 当前签名：
```rust
async fn run(&self, event: &HookEvent, ctx: &TraceContext, args: &Value) -> Result<RunOutcome, RunnerError>
```

返一个 Future，但没有 "cancel/abort current run" 机制。caller 只能 drop future
寄望底层资源释放——对 `Command::new` spawn 的子进程是无效的（除非 builder
设了 `kill_on_drop(true)`）。

查 `CcHeadlessRunner` 实现：未设 `kill_on_drop`。

## 4. 与 runtime-launch-strategy decision 的关系

已 approved decision `2026-05-19-decision-runtime-launch-strategy`：runner
长期走 tmux session 启动。tmux session 自带 attach/detach/kill 控制——
HITL 信号可直接 `tmux kill-session` 命中。

**但**：tmux 改造在独立 feature `runner-tmux-launch`，本 issue 是中间态修复。
两条路径：
- **路径 A**：在 `runner-tmux-launch` 内一起修——把 runner trait 加 cancel
  方法，tmux runner impl 调 `tmux kill-session`，old direct-spawn runner 调
  `child.kill()`。本 issue 等 `runner-tmux-launch` 落地自动解决。
- **路径 B**：先在 `dispatcher::runners::CcHeadlessRunner` 内加 `kill_on_drop`
  + 改 Command builder（最小改动），让 future drop 时子进程随被 SIGKILL。
  作为 stop-gap 修。等 tmux 改造再正式重设计。

## 5. 实际影响估算

中等：用户 `/stop` 是低频操作；即便 stop 后 runner 还跑一段也是有限资源
浪费（agent runtime 平均跑时分钟级，最坏多跑几分钟）。但用户感受很差
（明明 /stop 了为什么还在烧 token）。

## 6. 路径建议

**短期**（≤ 1 个 commit）：路径 B——`CcHeadlessRunner::run` 内 Command
builder 加 `.kill_on_drop(true)`，runner_future 被 drop 时 tokio 自动 SIGKILL
子进程。trade-off：drop 时机受 tokio runtime 调度影响，有几十 ms 窗口子进程
仍跑——可接受。

**长期**：`runner-tmux-launch` feature 内重新设计 Runner trait 加 cancel
方法，所有 impl 显式提供 kill 能力。

## 7. 关联

- 模块：`crates/roostery/src/dispatcher/runners.rs::CcHeadlessRunner`
- 调用点：`crates/roostery/src/bot_bridge/runner.rs::handle_event` 内
  `tokio::select! { sig = kill_rx => ... }`
- 上游：runtime-launch-strategy decision
- 不阻塞：bot-bridge-cluster 0.1.x 发布；这是 cleanup 性质修复

## 8. 路径

next step: `cs-issue-analyze` 选 A or B → `cs-issue-fix`。
