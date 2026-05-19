---
doc_type: issue-report
issue: 2026-05-19-runaway-tracker-empty-bucket-leak
status: resolved
resolved_at: 2026-05-19
severity: P3
summary: `RunawayTracker.fires: BTreeMap<TraceId, Vec<Instant>>` 不清理过期空 bucket。dispatcher daemon 长跑下每个独立 TraceId 至少残留一个 key + 旧 Instant，内存累计有界但慢增长（每条 trace ~32 字节起步），未来 Phase 5 bot-bridge daemon 拉起后会更明显。
tags: [dispatcher, runaway, memory-leak, daemon, observation]
discovered_during: 2026-05-19 自审第二轮（commit 5ea6cee 之后）
related_features: [2026-05-18-dispatcher-trace-budget, 2026-05-19-bot-bridge-cluster]
---

# RunawayTracker 不清理过期空 bucket — Issue Report

## 1. 问题现象

`dispatcher::runaway::RunawayTracker.fires: BTreeMap<TraceId, Vec<Instant>>`
对每个调用 `record(&trace_id)` 的 TraceId 创建（若不存在）一个 bucket entry。
`record` 内部 `bucket.retain(|ts| *ts >= cutoff)` 只清理**当前 bucket 内**
过期的 Instant。从未删除 bucket 本身（也无周期性清理任务）。

后果：
- 任何 record 过至少一次但之后再不触发的 TraceId，对应 key 永久驻留
- 配合 record 后 bucket 至少留 1 个 Instant（新 push），bucket 不会变空
- 未来 trace_id 池足够大（长跑 daemon、跨多 agent / 多 session）→ 内存
  线性增长，无上界

## 2. 复现思路

```rust
let mut t = RunawayTracker::with_window_and_threshold(Duration::from_secs(60), 100);
for i in 0..10_000_000 {
    let tid = TraceId::from_existing(format!("trace-{i}"));
    t.record(&tid);
}
// t.fires.len() == 10_000_000；每个 bucket 含 1 个旧 Instant，再不会被清
```

10M 独立 trace_id × (32B TraceId + Vec 开销 ~24B + 16B Instant + BTreeMap node)
≈ 800MB-1GB+。

## 3. 实际影响估算

当前阶段（Phase 4 dispatcher 已上）TraceId 产生频率较低：
- 每次 hook event 一个 root TraceId
- 大多数 trace 链路 depth 1-3 ≤ 几个 TraceId/event
- 单机日均触发数十到数百级别

→ 短期影响可忽略（数十 MB 量级慢增长）。

Phase 5 bot-bridge-cluster 起来后会上升：
- 多 BotRole × 每条 IM event 各自一个 TraceId
- daemon 长跑 7×24
- 高峰场景 (高频 @mention) 每秒数 TraceId

→ 中期需要解决，不至 P1。

## 4. 备选修复方向（不在本 report 拍板）

**方案 A**：record 内顺手扫一定数量 buckets 做老化清理。优点：无后台
线程；缺点：record 路径变 O(N) amortized。

**方案 B**：单独 `prune()` 公开方法，由 dispatcher 主循环定期调用（如每
N 次 dispatch 一次）。优点：record 保 O(1)；缺点：需要 caller 配合。

**方案 C**：换数据结构（如 time-ordered ring buffer），整体重写。优点：
彻底解决；缺点：改动量大，影响公开 API。

倾向 B（最小改动 + 不引入 record 路径成本）。

## 5. 关联

- 模块：`crates/roostery/src/dispatcher/runaway.rs` line 25-95
- 发现于：post-0.1.0 自审循环（CodeStable 2026-05-19）
- 不阻塞当前任何 feature
- 上游 design：`.codestable/features/2026-05-18-dispatcher-trace-budget/`

## 6. 路径

next step: `cs-issue-analyze` → 选方案 → `cs-issue-fix`。可在
bot-bridge-cluster 之后或之间穿插处理。
