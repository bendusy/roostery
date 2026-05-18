---
doc_type: requirement
slug: runtime-neutral
pitch: 不被某家 agent runtime 绑死——CC / Codex / Gemini / 自己写的，都能在同一套飞书面里出活。
status: draft
last_reviewed: 2026-05-15
implemented_by: [2026-05-18-dispatcher-trace-budget, 2026-05-18-dispatcher-rules]
tags: [vendor-neutral, agent-runtime, interoperability]
---

# 不绑 agent runtime 的中立接入

## 用户故事

- 作为已经用顺手 Claude Code 但偶尔想试 Codex / Gemini 的开发者，我希望切到别的 runtime 时飞书侧的呈现和团队约定不变，而不是每换一家就重学一套接入流程、刷新一遍队友的认知
- 作为团队领头，我希望成员各自用自己顺手的 runtime 也能让产出汇到同一个飞书空间，而不是为了"统一观察面"强迫全员用同一家工具
- 作为自己拼了 agent 的开发者（拿 Python / 自定义 SDK 写了点东西），我希望我的 agent 也能挂进来在飞书里被看到、被讨论，而不是因为"不是主流厂家"就得自己另搞一套 UI
- 作为对厂商绑定警惕的人，我希望今天用 CC 出活，明天 Anthropic 涨价 / 出问题 / 改条款，我能切别家继续干，而不是被困在某家 runtime 的生态里——光是想到迁移成本就不敢离开

## 为什么需要

agent runtime 生态正在裂变。Claude Code、Codex、Gemini、OpenClaw、Cursor 自带 agent、各种用 SDK 拼的自定义 agent，每隔几个月又冒出新的。**没有"哪家最好"的统一答案**——开发者实际是混用的：写代码用一家、跑批处理用另一家、做调研又换一家。

但主流的 agent dashboard 类产品要么绑死某家（"我们只支持 Claude"），要么把多 runtime 做得很轻——支持是支持，配起来怪、用起来割裂。结果是开发者要么忍着单一 runtime 的短板，要么自己手工维护几套呈现 / 通知 / 沟通约定，再加一层心智负担。

**锁定单一 runtime 的代价不只是"换不了工具"**——团队围绕这个 runtime 长出的教程、命令习惯、协作约定、screenshot 模板，也一起被锁定。一旦上游厂商出事或换路线，迁移成本远超想象。

如果飞书侧的呈现和协作约定独立于具体哪家 runtime，换 runtime 就是"换发动机不换车厢"——工作流不动，团队认知不动，只是后台的执行引擎换了。

## 怎么解决

每家 agent runtime 通过同一套接入约定挂进 Roostery：装一次就能用，hook 触发后产出会以**相同的形态**出现在飞书（任务卡、步骤流、群消息），读者看不出来这次是哪家 runtime 跑的——也不必关心。

切换 runtime 等于换执行后端，不动飞书侧的任何东西。混用也可以：今天 CC、明天 Codex、后天自己写的 Python agent，飞书里看到的是同一类卡片、同一种结构。如果某家 runtime 升级 / 出问题 / 改条款，用户可以把工作流逐步迁到别家，飞书侧的记录和讨论不需要重做。

自己写的 agent 也是一等公民——只要按同一套接入约定挂进来，呈现质量和"主流厂家"一致。

## 边界

- **不替任何 runtime 做事**——不跑模型、不做 agent 编排、不调度任务，只把 runtime 已经产出的东西搬到飞书呈现
- **首发不保证所有 runtime 同等支持**——0.1.0 至少跑通一家（首选 CC），其他 runtime 的接入质量随 roadmap 演进，每家完整支持的标准要单独验收
- **不做 runtime 之间的 cross-runtime 通信**——A runtime 的产出可以被 B runtime 读到（通过飞书群 / 文档），但不替你做"A 的输出自动喂给 B"这种编排
- **不替用户选 runtime**——不做 "auto-select best runtime for this task" 这种判断，选哪家由用户自己决定
- **不接管 runtime 的配置和密钥**——每家 runtime 各自的 API key / 模型选择 / 参数仍归用户管，Roostery 不做集中身份层
- **加新 runtime 不是零开发自动支持**——接入新 runtime 需要写一份 adapter，这是开发者侧的工作。用户感知层是"这个 runtime 已经支持 / 还不支持"，而不是"任何 runtime 都自动支持"

## 变更日志

- 2026-05-15：drafted（初稿落档）
- **2026-05-18**：`dispatcher-trace-budget` 落地（feature `2026-05-18-dispatcher-trace-budget`），Phase 4 Module E 起步。`TraceContext`（深度守门）+ `BudgetState`（配额守门）+ `RunawayTracker`（事后阈值兜底）三独立 gate 模块就位。**这是 req 的"loop 保护是中立 dispatcher 的前提"基础设施层**：三 gate 不感知具体哪家 runtime，是后续 Phase 4 dispatcher-loop 接入任意 runtime 时必经的守门基底。req 仍保持 `draft`——用户视角"换 runtime 飞书侧呈现不变"要 Phase 4 收尾 dispatcher-loop + Phase 5 bot-stop-hook 全套兑现
- **2026-05-18**：`dispatcher-rules` 落地（feature `2026-05-18-dispatcher-rules`），Phase 4 Module E 第 2 子 feature。`HookEvent` §4.4 schema 落地（外部 hook 触发的事件标准形状）+ Rule engine MVP（3 维 AND match：hook_source / workspace_glob / trigger_meta_eq；first-match-wins；self-event 防自激）。**这是 req 的"接入新 runtime 写一份 adapter"用户配置接入面**：用户通过 `~/.roostery/rules.yaml` 把 HookEvent 路由到具体 runner，规则维度独立于 runtime——CC / Codex / Gemini / 自定义 runtime 共享同一套规则 schema。Action 是 opaque `{runner, args: Value}` 透传给 Runner trait impl（dispatcher-runners feature 落地后才有真消费）。req 仍保持 `draft`——dispatcher-loop 收尾后才有完整链路
