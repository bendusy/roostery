---
doc_type: issue-report
issue: 2026-05-19-relay-task-per-chat-race
status: resolved
resolved_at: 2026-05-19
severity: P1
summary: bot_bridge::relay_task::record_start 在 cache miss 时 load-then-create 没有 per-(bot_app_id, chat_id) 级别锁或 recheck-after-create，同 chat 并发两条事件首次到达会创建两条飞书 Task，最后一次 cache 写覆盖前一条 → 较早创建的 TaskGuid 变成孤儿。同时损坏的 cache JSON 因 load_cache 返 Ok(None) 也会触发同一路径（P2-1）。
tags: [bot_bridge, relay_task, race, cache, idempotency]
discovered_during: codex audit 2026-05-19 round-7 P1-6 + P2-1
related_features: [2026-05-19-bot-bridge-cluster]
---

# relay_task per-chat race — 并发 record_start 创建重复飞书 Task

## 1. 问题现象

`bot_bridge::relay_task::record_start(lark, bot, event, summary)` 路径：
```
1. cache_path_for(bot.app_id, event.chat_id)
2. load_cache(path) → Some(entry) → 返回 reuse TaskGuid（cache hit）
                   → None → create_task + save_cache + 返回新 TaskGuid (cache miss)
```

**race 窗口**：同 (bot_app_id, chat_id) 首条事件到达时 cache 不存在。若几乎
同时第二条事件到达（用户连发两条 @bot），两个 handle_event 任务并发跑
record_start，**都**走 cache miss 分支：
- T1 load_cache → None
- T2 load_cache → None
- T1 create_task → 飞书侧建 task_A
- T2 create_task → 飞书侧建 task_B（**重复 task**）
- T1 save_cache(task_A)
- T2 save_cache(task_B) → 覆盖 → cache 最终指向 task_B → task_A 变孤儿

## 2. P2-1 同源问题

`load_cache` 对**损坏的 JSON** 也返 `Ok(None)`（同 `bot_task_writer` 策略，
"create 自然修复"）。但同一文件先前已 record_start 写过一个 TaskGuid，损坏
后重启 daemon 看不到 → 又走 create_task → 飞书侧重复 task。

P1-6 和 P2-1 是同一问题的并发态 vs 持久态两面，应统一修。

## 3. 实际影响估算

中等-高：
- 用户连续两条 @ 在 IM 客户端是常见操作（"@bot do A" 紧接 "@bot ah also B"）
- 重复飞书 task 让用户困惑，且 task_A 的 step 流没人 append（孤儿）
- 长跑 daemon 内 cache 损坏概率随时间累积（磁盘写中断、并发 atomic rename 失败等）

## 4. 备选修复方向（不在本 report 拍板）

**方案 A**：tokio::Mutex 池——`Arc<Mutex<HashMap<(String,String), tokio::sync::Mutex<()>>>>`，
load_cache + create_task + save_cache 三步整个串行化。优点：彻底解决。缺点：
新增内存数据结构，daemon 内 mutex 持有跨 await 边界（注意 cancel-safety）。

**方案 B**：稳定的 idempotency_key 给 create_task——以 `(bot_app_id, chat_id)`
做 key，create_task 服务端去重。前提：飞书 task API 是否支持 idempotency_key
（lark-cli 包装层是否暴露）需调研。优点：服务端去重最干净。缺点：依赖飞书侧
支持。

**方案 C**：load_cache 加文件锁（advisory flock 同 budget.json 模式）。优点：
跨进程也防（虽然单机 daemon 通常单实例）。缺点：fs 锁需要小心 deadlock。

**方案 D**：create_task 成功后 reload cache（"check 后再 check"），发现已被
别人写过 → 自己 create 的 task 显式 abandon（API 调用删 task）+ 用别人写的。
优点：无需引入锁。缺点：飞书侧已建 task 的"撤回" API 可能不存在，至少要 step
"orphan task abandoned" 文案。

倾向 A（tokio::Mutex 池），结合 P2-1 路径再加 "corrupt JSON → warn + 重建路径
但同 lock 内"。

## 5. 关联

- 模块：`crates/roostery/src/bot_bridge/relay_task.rs::record_start` line 245+
- load_cache: line 148 (Ok(None) on corrupt)
- 单测覆盖：当前 `record_start_cache_hit_reuses_task_guid` 验证 hit 路径
  ；race 路径无测试
- 上游 design：`.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-design.md`
  §2.2 / §3 N2（"同 chat 连续 @ → 同一 TaskGuid"——current impl 在 race 下不
  满足这一不变量）

## 6. 不阻塞

bot-bridge-cluster 已 accepted (2026-05-19 cc14ca5)。本 issue 属于发布后修复
（与 0.2.x 一起考虑）。生产环境出现频率取决于用户连发节奏。

## 7. 路径

next step: `cs-issue-analyze` 选方案 → `cs-issue-fix`。建议与 runaway-tracker-
empty-bucket-leak issue 一起 batch，都是 daemon 长跑可靠性补强。
