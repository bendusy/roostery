---
doc_type: decision
category: architecture
date: 2026-05-19
slug: runtime-launch-strategy
status: active
area: dispatcher / runners / multi-agent-host
tags: [tmux, runner, agent-runtime, multi-agent, vibecoding, acp, daemon]
related_features: [2026-05-19-bot-bridge-cluster]
related_requirements: [agent-work-in-feishu]
---

## 背景

Roostery 作为 vendor-neutral agent broker，需要本机长跑 daemon 启动各家 agent
runtime（Claude Code / Codex / Gemini / 未来 custom）并消费它们的输出。当前
（Phase 4 dispatcher 落地后）`dispatcher::runners::CcHeadlessRunner` 直接用
`tokio::process::Command::new` spawn 子进程，捕获 stdout/stderr 解析 JSON。

随着 Phase 5 bot-bridge-cluster（IM 群里反向操控 agent / 多 bot daemon）
设计中，runner 启动方式需要拍板长期方向。两个备选：

1. **ACP（Agent Client Protocol）抽象层**：用标准 host-agent 协议（JSON-RPC over
   stdio）让 roostery 以"host"身份与各家 agent runtime 解耦交互。
2. **tmux session default**：runner 在 tmux session/window 中跑，roostery 通过
   tmux 控制启停、通过 pipe-pane 抓输出。

## 决定

Roostery **默认走 tmux session** 启动 agent runtime。短期 `CcHeadlessRunner` 现状
（direct `Command::new`）保留；中期单独开 feature `runner-tmux-launch` 改造所有
runner impl 走 tmux；长期 tmux 启动成为新 runner adapter 的默认模板。

**不**采用 ACP 作为默认方案。

## 理由

1. **直接可见性（核心）**：用户随时 `tmux attach <session>` 看 agent 实时
   输出 / 接管 / 干预——vibecoding 场景下用户想随时插话指挥，这是最关键的
   UX 维度（对齐 `agent-work-in-feishu` req 的"用户主体性"）。direct spawn
   没有 attach 能力；ACP 也是后台协议，没有"看见"维度。
2. **roostery 是本机 daemon，host 由用户控制**：ACP 设计目标是"不同主机能跑
   同一 agent / agent 能换 host"——典型应用场景是 IDE 插件 / 云端编排
   平台。Roostery 是用户自己机器上的中间件，主机即用户，不需要这层 vendor
   抽象。
3. **抽象成本低**：tmux 只是 wrapper，runner 输出仍走 pipe（tmux pipe-pane
   或 tmux 内 shell redir），roostery 抽 JSON 的逻辑不变；现有
   `dispatcher::runners::Runner trait` 不需要重设计——只是 default impl
   从"直接 spawn"换"tmux spawn"。
4. **多 bot daemon 天然兼容**：bot-bridge-cluster Phase 5 一台机器一个
   daemon × 多 BotRole × 多并发 trace，tmux 的 session/window/pane 三级
   命名空间自然映射（一个 session = 一个 daemon、一个 window = 一条 trace、
   一个 pane = 一个 runner step）。
5. **降级与诊断**：tmux session 在用户机器持久存活，agent 崩了用户能直接
   `tmux attach` 看现场 stderr/stdout；ACP 协议层一旦掉链，用户没有"现场"
   可看。

## 考虑过的替代方案

### A. 保持 direct spawn（不引 tmux）

**优点**：无新外部依赖（tmux 虽是 macOS/Linux 常见但仍需明示要求）；进程模型
最简单。

**否决理由**：用户失去 attach 能力——vibecoding 体验下"看不见 agent 在干啥"
是反模式。

### B. ACP 协议层

**优点**：标准化、面向未来；与 Zed / Cursor / 外部编排平台兼容。

**否决理由**：（1）抽象代价 vs roostery 实际场景（本机 daemon）不匹配；
（2）失去 attach 维度；（3）ACP 生态尚未稳定，押注早期协议风险高；
（4）需要适配每家 agent runtime 对 ACP 的支持度——不少 runtime 当前还没
ACP 实现。

### C. 自研 IPC（Unix socket / named pipe）

**优点**：可控、可观测。

**否决理由**：重新发明 tmux 已经提供的功能（多 session 管理 / attach /
detach / scrollback / 命名空间），且失去 tmux 用户既有工具链
（tmux-resurrect / tmux-continuum 等）。

## 后果

### 正面

- bot-bridge-cluster daemon 用户可 `tmux attach roostery-bot-{app_id}` 实时
  观察任何 BotRole 的 agent 工作流
- 单台机器多 BotRole / 多 trace 并发不互相干扰（pane 隔离）
- 用户接管：要直接给 agent 输入 / 暂停 / 杀，tmux 标准操作即可
- 故障现场：agent crash 后 pane scrollback 仍在，方便 post-mortem

### 负面

- 引入 tmux 作为运行时依赖：`roostery init` 后续要校验 tmux 在 PATH（写
  `roostery smoke` 新增 probe）
- 平台门槛：Windows 原生不支持 tmux——目前 Roostery 只针对 macOS/Linux
  开发，与现状一致；未来支持 Windows 时需要替代方案（PowerShell / WSL2）
- `Runner trait` impl 需统一改造（中期 feature `runner-tmux-launch`）
- 测试复杂度上升：MockTmux 或在 CI 里跑 tmux

### 兑现路径

- **短期**：bot-bridge-cluster（feature `2026-05-19-bot-bridge-cluster`）
  按现 design 继续推，runner 启动仍 direct spawn。本决策**不阻塞**当前
  feature。
- **中期**：cluster 落定后开新 feature `runner-tmux-launch`：
  - 把 `dispatcher::runners::CcHeadlessRunner` 改成 `CcTmuxRunner`
  - 新增 `paths::tmux_session_name(bot_app_id, trace_id)` helper
  - `roostery smoke` 加 `tmux -V` probe
  - `roostery init` 校验 tmux 存在
- **长期**：新增 Codex / Gemini / 自定义 runner adapter 时默认按 tmux
  launch 模板写

## 相关文档

- `.codestable/architecture/ARCHITECTURE.md` — runtime 启动一节应引用本决策
  （Phase 5 推进时由用户决定是否加索引；本决策不主动改 ARCHITECTURE.md）
- `.codestable/requirements/agent-work-in-feishu.md` — 本决策兑现"用户主体性
  + 随时插话"的体验维度
- `.codestable/features/2026-05-19-bot-bridge-cluster/bot-bridge-cluster-design.md` —
  消费此决策（短期 runner 启动方式不变，长期由 `runner-tmux-launch` feature
  改造）
- `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` — Phase 5 之后
  应增 `runner-tmux-launch` planned item（由用户决定加 roadmap 时机）
