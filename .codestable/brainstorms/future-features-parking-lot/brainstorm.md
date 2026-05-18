---
doc_type: brainstorm
slug: future-features-parking-lot
created: 2026-05-19
status: parking-lot
summary: 后续要加入但当前不动手的 feature 候选清单——记下"在雷达上"避免遗忘，留待用户决定何时升级成正式 brainstorm/roadmap 条目
tags: [parking-lot, future, codeburn, mirage, agent-fs, cost-dashboard]
---

# Roostery 后续 Feature 候选清单

> 创意空间 | 2026-05-19 | 状态：仅占位、不动手

本文档存放"用户已经表态后续要加但当前不立即推进"的 feature 候选。每条
是**轻量级 placeholder**，记下来源 + 一句话描述 + 与 Roostery 主线的潜在
关系，**不展开设计**——等用户拍板要做时升级成正式 brainstorm 或直接走
`cs-feat-design`。

---

## 候选 1: CodeBurn — AI 编码工具花费看板

**来源**：用户 2026-05-19 在推 `bot-bridge-cluster` step 2 期间提出
（参考 <https://www.daemonology.net/hn-daily/>）

**一句话**：把开发者使用 Claude Code / Codex / Gemini 等 AI 编码工具的
花费（token / 计费 / API 调用）聚合成可视化看板。

**与 Roostery 主线的潜在关系**：

- Roostery 已经在 `dispatcher::runners::CcHeadlessRunner` 抽 `cost_usd`
  字段（commit 188871f）；这部分数据已经经过 sanitize 后流向 budget。
- 把同一数据流接入"看板视角"是自然延伸——`journal/` 里有完整 cost 历史，
  只缺聚合 + 渲染层。
- 兑现形式可能是：
  - 选项 A：飞书 Base（Roostery 已选 Base 当 index 层）多维表 + 现成
    chart view
  - 选项 B：本地 `roostery cost --since 7d` CLI 出 stdout 表
  - 选项 C：独立 web 看板（拉远 Roostery 的"中立中间件"定位）
- 倾向：A 或 B 优先（不离开 Roostery 现有红线）

**前置依赖**：Phase 6 `report-git-llm` + `report-daily` 落地后跨度变小。

**未开始设计**——等 Phase 5 / Phase 6 收尾再讨论。

---

## 候选 2: Mirage — Agent 统一虚拟文件系统

**来源**：同上

**一句话**：给 agent runtime 提供一层统一的虚拟文件系统抽象，让不同
agent（CC / Codex / Gemini / 自定义）看到一致的工作空间视图，可能含
overlay / snapshot / sandbox 能力。

**与 Roostery 主线的潜在关系**：

- 与 `runtime-launch-strategy` decision（2026-05-19，tmux default）正交但
  互补：tmux 解决"进程隔离 + 可见性"，VFS 解决"文件视图隔离 + 一致性"。
- 与 Roostery 的 vendor-neutral 定位一致——抽象层让 agent 不依赖具体
  host filesystem 布局。
- 可能用途：
  - 多 agent 并发改同一仓库不互相踩（overlay snapshot）
  - 实验性改动可一键 rollback（基于 snapshot diff）
  - 跨设备 agent 工作空间镜像（同 Roostery 多设备同步主线契合）
- 实现路径未定：FUSE / overlayfs / 自研 watcher + journal 都可能。

**前置依赖**：bot-bridge-cluster 落定 + 多 BotRole 真实运行后才能看到
"VFS 隔离"的实际需求。

**未开始设计**——等 Phase 5 跑起来产生真实反馈后再讨论。

---

## 操作约定

- **新候选加进来**：在本文档末尾追加节，按上面格式写
- **某条升级成正式 brainstorm**：另开 `.codestable/brainstorms/{slug}/brainstorm.md`
  做详细脑暴，本文档对应节标注 `→ 见 brainstorms/{slug}/`
- **某条降级（决定不做）**：本文档对应节标注 "DROPPED" + 一句理由，不删
  历史记录

## 相关

- `.codestable/brainstorms/v0.x-direction/brainstorm.md` — 主项目方向脑暴
- `.codestable/compound/2026-05-19-decision-runtime-launch-strategy.md` — tmux
  默认决策（与候选 2 互补）
- `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` — 当前主 roadmap
  （Phase 5 进行中），本文档候选**不**在该 roadmap 内
