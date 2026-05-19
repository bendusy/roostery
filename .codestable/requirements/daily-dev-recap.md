---
doc_type: requirement
slug: daily-dev-recap
pitch: 每天自动把你今天写了啥、和 agent 一起跑了啥总结成人话日报，落进飞书 docx + Base，多设备翻、按周月查都顺手
status: draft
last_reviewed: 2026-05-19
implemented_by: []
tags: [feishu, daily-recap, vibecoding, git, llm, draft]
---

# 自动开发日报

## 用户故事

- 作为多设备切换的 vibecoder（早上桌面 / 中午笔记本 / 晚上手机），我希望每天结束有人替我把今天动了哪些代码总结成一段能读的日报，而不是自己翻 `git log` 拼
- 作为同时维护几个仓的独立开发者（主项目 + side project + 工具脚本），我希望日报跨仓一次出全貌，而不是每个仓单独 `cd` 进去看一眼
- 作为偶尔回顾"上周到底干了什么"的人，我希望日报历史留在能搜的地方（飞书 docx + Base 索引），而不是从终端 history / commit message 凭记忆拼
- 作为和 agent 高频协作的人（CC / Codex 一天提一堆细碎 commit），我希望 LLM 把零散提交翻译成叙事段落，而不是直接看 50 条 `feat: tweak X` 之类的噪音

## 为什么需要

现代独立开发常常这样：早上写主项目、下午改 side project、晚上和 agent 折腾点工具脚本——一天可能动 3-5 个仓、提 30 条 commit，到了晚上完全记不清今天具体干了什么。周末想回顾"上周做了啥"更糟。

agent 协作让这个问题加剧：vibecoding 风格下 agent 帮你拆得细、commit 也细，`git log` 读起来像噪音不像故事。`git log` 本身是"机器友好不人友好"的——但你想要的是一段类似"今天主要在做 X，顺手碰了 Y，跟 agent 一起卡在 Z"的人话叙述。

写日报本身又是没人愿意每天主动做的事——做的人坚持几周就忘，不做的人月底回顾时只能凭模糊印象拼。这就是这能力的位置：**把每天的 git 流水自动翻译成人话存到一个能搜的地方**，等于把"未来某天会想到的回顾需求"提前兑现，不依赖你今晚有没有心情写。

## 怎么解决

装好后，每天约定时间（晚上下班 / 第二天早上 / 手动触发都行）自动跑一次：扫你今天动过的几个仓，抓出今天的 commit，让一个 LLM 把它们翻译成人话摘要，推到飞书——docx 一份是给人读的、Base 一行是结构化可检索索引的。

体验上：在飞书 app（任意设备）打开就能看到"今日开发日报"；按周或按月翻 Base 索引就能拉历史。日报内容反映今天涉及的仓库、主要改动主题、关键 commit、以及和 agent 协作中的拐点（在 commit message 可读出来时）。

## 边界

- **不是项目管理 / 待办系统**——不分配任务、不排优先级、不跟踪 deadline
- **不替代深度 retrospective**——只覆盖"今天客观干了什么"，反思 / 复盘 / 决策记录归别处
- **信号来源只是 git commit**——没 commit 的活（思考、阅读、讨论、未提交 WIP）记不到；想被算进日报就得 commit
- **只信任本地 git**——不调 GitHub / GitLab API；要进日报的仓必须本机有 clone
- **LLM 这一段是 [[portable-by-default]] / [[agent-work-in-feishu]] "数据全自家飞书"承诺的显式破例**——commit 摘要会发外部 LLM provider，用户自担 LLM token 成本和这一段数据离开本地的隐私选择；不接受这种取舍的用户可以禁用本能力或自接本地模型，Roostery 主链路不强制依赖
- **0.x 阶段是单用户日报**——不做团队聚合、不做跨人对比、不做权限管理
- **不主动后台 daemon**——主流程是用户配好 cron / launchd / 手动触发；Roostery 不替用户在系统里常驻一个调度进程
- **用户需要先准备好**：本机有想统计的 git 仓 + 可用的 LLM provider credentials + 飞书侧可写的 docx + Base 容器 + 已跑过 `roostery init` 完成装机

## 变更日志

- **2026-05-19**：drafted 初版落档（feature `2026-05-19-report-recap-engine` design 阶段触发起草）
- **2026-05-19**：引擎层落地 — feature `2026-05-19-report-recap-engine` accept（533 lib tests + 8 integration tests 全过）。本 feature 兑现 req 的"git 多仓聚合 + 自动委托 agent CLI 出人话摘要"维度——`roostery daily-recap` 子命令 + 库 API `daily_recap::run / prepare` 已就绪，可手动 / cron 触发出 summary。**req status 保持 `draft`**——req 用户视角的"日报写到飞书 docx + Base"完整能力还差 `report-daily` feature 把 `RecapOutcome` 落到飞书；升级 `current` 待 `report-daily` accept。**边界第 5 条"LLM 显式破例"措辞软化候选**（roadmap §7 留观察项 + acceptance §6 标记）：Roostery 自身 0 LLM SDK binding（落地 ARCHITECTURE §5.5 / §6.3 / §5.10 三处归并），LLM 调用是用户已装 agent CLI 子进程的副作用，不再是"Roostery 自身"破例；用户与自己 agent 厂商的既有关系决定 prompt 是否出域——边界文字层下次 `cs-req update` 时一并修，本次保留原文不动以避免 req 漂移
