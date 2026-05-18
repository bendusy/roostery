---
doc_type: requirement
slug: agent-work-in-feishu
pitch: agent 跑在桌面 / 远程开发机上，飞书里看进度、群里接续讨论——多设备无缝跟进 vibecoding。
status: draft
last_reviewed: 2026-05-18
implemented_by: [2026-05-16-lark-cli-wrapper, 2026-05-17-config-yaml, 2026-05-18-hooks-merge, 2026-05-18-roostery-init, 2026-05-18-bot-task-writer]
tags: [feishu, agent, cross-device, vibecoding, observability, dogfood]
---

# Agent 工作过程长在飞书里

## 用户故事

- 作为习惯 SSH 到远程开发机干活的人，我希望桌面机 / 远程机上的 agent 在跑时，手机也能看进度、点开飞书任务卡接续讨论，而不是只能 Termius SSH 重连（更别说有的开发环境根本没内网穿透）
- 作为在多设备 / 多窗口间切换的 vibecoder（早上桌面、出门换笔记本、地铁掏手机），我希望 agent 的状态跟着我走，而不是被钉在某台机器的某个终端窗口里
- 作为单人开发者，我希望本地 agent（Claude Code / Codex / Gemini 之类）跑任务时，过程和产出自动落到飞书任务卡和群里，而不是自己 tail 终端 / 复制日志
- 作为协作中的团队成员，我希望队友的 agent 跑了什么、卡在哪、产出在哪，我打开飞书就能围观、点评、接续，而不是又开一个 dashboard 工具同步进度
- 作为事后 review 的人，我希望翻飞书的任务卡和群消息就能拼出"这次 agent run 做了什么、引用了哪些文档、最后是谁拍的板"
- 作为对数据主权敏感的开发者，我希望 agent 的工作痕迹留在我自己的飞书租户里，而不是某家 SaaS dashboard 厂商的服务器上

## 为什么需要

现代开发越来越多在 **vibecoding** —— 跟 agent 对话出活，agent 跑在某台特定机器上（桌面、远程开发机、云 GPU 实例），但**开发者本人在多设备间切换**：早上在工位、午休换笔记本、通勤掏手机、晚上回家用平板。

这中间有个**结构性断层**：agent 跑在那台机器上，状态和上下文也在那台机器，可一旦人不在桌前，就没有简单办法跟进。终端 SSH 在手机上极其难用，Termius / blink shell 这种方案要求公网可达 / 内网穿透——对自己有专业网络基础的人能凑合，对**绝大多数普通开发者**直接劝退。

飞书（或任何云同步的 IM / 协作面）天然解决了"同一套数据多设备可见"——你给同事发条飞书消息，对方桌面、手机、网页都能秒收。把 agent 的工作状态推到飞书，等于**借云协作面已经做好的同步层**完成多设备整合，不必自己搭。这是 Roostery 选择 Feishu-native 路径的**最强 "why"**。

剩下两层补充动机：

1. **协作语境不撕裂**——日常沟通在飞书（或类似 IM），agent 产出却在别处；想讨论一次 agent run，要么把对方拉去 dashboard，要么把 dashboard 里的东西手动搬回飞书。每次都要做一次
2. **数据主权**——agent dashboard 类产品要把数据交给厂商，对企业 / 团队 / 隐私敏感者不可接受。Roostery 让数据全程留在用户自己的飞书租户

如果 agent 跑完的痕迹本身就出现在团队已用的飞书里、并且天生跨设备同步，上述三层痛点同时消失。

## 怎么解决

装好后，本地的 agent runtime 跑任务，过程会自动在飞书生成一张可点开的任务卡片：任务名、状态、步骤流、引用的文档、产出链接、谁触发的、跑了多久。同一次任务在群里也有同步消息可以接续讨论；下次 agent 再跑时能读到群里的反馈，不必从零开始。

体验上，用户在任何装了飞书的设备（桌面客户端、手机 app、网页）都能看完一次 agent run 的全貌——上下文从触发、过程、产出到讨论都在同一个界面里。装机过程是本地的（一个命令、一个 daemon），跑出来的数据也都写进用户自己的飞书租户，不经过第三方 dashboard。

## 边界

- **不替代 agent runtime 本身**——不跑模型、不做编排逻辑，只把已有 runtime 的产出搬进飞书呈现
- **不是企业级 agent 平台**——0.x 阶段服务单人和小团队（共享一个飞书租户的几个人），不做多租户隔离、权限矩阵、SSO
- **0.x 阶段呈现层只覆盖飞书**——Roostery 不主动支持 Slack / Teams / Discord 等其他 IM 的官方 view。但底层数据保持 portable（见 `portable-by-default`），用户 / 社区可基于公开 journal schema 自建其他呈现，不被 Roostery 锁死
- **依赖飞书原生面的产品形态**——任务卡、群消息、Docs 评论是核心载体；如果用户不愿意用飞书自己的任务和群，这能力对 TA 没意义
- **跨设备同步走飞书侧、不走 Roostery 自己**——多设备一致性靠飞书云的同步能力，Roostery 本身不做设备间状态同步；这意味着只要飞书 app 装在设备上、能登录，就能跟进；反过来，飞书没装 / 网络断了等问题超出 Roostery 控制范围
- **用户需要先准备好**：飞书租户（可创建任务和群）+ 配好的 `lark-cli` + 至少一个本地 agent runtime（CC / Codex / Gemini / 自定义 Python 之一）
- **不承诺所有 agent runtime 同等支持**——首发只保证一两个 runtime 跑通，其他 runtime 的接入质量随 roadmap 演进

## 变更日志

- 2026-05-15：drafted（初稿落档）
- 2026-05-15：刷新 vision——把 "多设备 / 跨窗口 vibecoding" 提升为首要 "why"，重写 pitch、加 2 条 cross-device 用户故事、重写 "为什么需要" 段落顺序；A 边界第 3 条软化（"飞书是不可替换组件" → "0.x 只覆盖飞书但底层数据 portable"）以兑现 Roostery 中立中间件的命名意图，跟新立的 `portable-by-default` 互引；加边界第 5 条说明"跨设备同步走飞书侧"
- **2026-05-18**：`bot-task-writer` 落地（feature `2026-05-18-bot-task-writer`），**Phase 5 Module F 第 1 子 feature**。3 pub async fn 纯库 API（`create_task` / `append_steps` / `get_or_create_for_session`）+ session_cache JSON v1 持久化（`~/.roostery/state/session_tasks/`）+ host suffix 多机区分 + safe_filename 路径跳出防御 + `append_steps --yes` 架构红线显式破例。**这是 req 的"agent 跑完出现在飞书任务卡里"核心兑现层第一砖**：首次让 Rust 业务模块真消费 `LarkRunner` trait 做生产飞书 IO（dispatcher 不走飞书；smoke / shim 走独立 I/O 路径）。下一 feature `bot-stop-hook`（minimal-loop = true）调本模块完成 0.1.0 E2E 闭环——agent stop → sh bridge → stdin event → bot_stop_hook 调 `get_or_create_for_session` + `append_steps` 把 agent 工作过程串进飞书 task。**req 仍保持 `draft`**——升级 `current` 等 bot-stop-hook 跑通端到端"用户在飞书 app 真看到 agent run 的 task 卡 + step stream"再升
- **2026-05-18**：`roostery-init` 落地（feature `2026-05-18-roostery-init`），Phase 3 Module D 收尾。陌生开发者首次跑通装机链路——`roostery init` 单命令串起 smoke gate / state dir bootstrap / identity reflect / agent detect / shim install（PATH-prefix 拦截 lark-cli）/ sh bridge / 3-runtime hook merge（cc + codex + gemini）/ `~/.roostery/env` 写 `ROOSTERY_REAL_LARK_CLI` / shell rc marker block 幂等 patch。**这是 req 的"B 用户首次装机入口"**：装好后用户跑 `claude / codex / gemini` Stop hook 自动 fire 到 `roostery dispatcher fire`（Phase 4 dispatcher 起来后真消费，本期 hook 触发会 clap "unknown subcommand" 但 `\|\| true` 吞掉不阻塞 agent）。req 仍保持 `draft`——用户视角"在飞书看到 agent 写什么"要 Phase 5 bot bridge 写 IM thread / 任务卡才兑现。
- **2026-05-18**：`hooks-merge` 落地（feature `2026-05-18-hooks-merge`），Module D 装机桥接层兑现。3 个 Stop hook 模板（CC + Codex + sh bridge）`include_str!` 编译期嵌入；JSON 深合并按 event key + matcher + command tail 三层幂等去重把 hook 片段注入 `~/.claude/settings.json` / `~/.codex/hooks.json`；env 前缀切到 `ROOSTERY_AGENT=cc/codex`（一次切口径）。**这是 req 的"装机后 agent runtime 触发 hook 进入 Roostery 处理路径"基础设施**：agent 跑完调用 SessionEnd hook → sh bridge 从 stdin 抽 summary → 调 `roostery dispatcher fire`（Phase 4 dispatcher 起来后真正消费）。req 仍保持 `draft`——用户视角"飞书看到 agent 在写什么"还要 Phase 5 bot bridge 兑现
- **2026-05-17**：`config-yaml` 落地（feature `2026-05-17-config-yaml`），Module D 配置基础层兑现。`Config.identity { user_id, default_chat_id, default_task_app_token }` 三字段是 req 的"用户身份 + 默认任务挂载点"维度的 schema 承诺；`Config.runners` 是 Phase 4 dispatcher 路由不同 agent runtime 的开关位。**这是 req 的可配置性层**：以前 Phase 5 bot bridge 要写飞书任务卡时硬编码"挂哪个群 / 哪个 Base"是 blocker，现在有了配置入口可填。req 仍保持 `draft`——用户视角"跨设备看到 agent 在写什么"还要 Phase 5 bot bridge 真去写 IM thread / 任务卡才兑现
- **2026-05-16**：`lark-cli-wrapper` 落地（commit `cc44dfa`），Module C 飞书 syscall 通道成型——`LarkRunner` trait + LarkCli subprocess + Journaled 装饰器三件齐备。**这是 req 的基础设施层**：往后 Phase 4 dispatcher / Phase 5 task_writer / bot_bridge 才能开始往飞书任务卡 / IM thread / Docs 评论里真正"贴 agent 工作内容"。req 仍保持 `draft`——用户面端的"跨设备看到 agent 在写什么"还需要 Phase 5 兑现
