---
doc_type: requirement
slug: portable-by-default
pitch: 你的 agent 工作痕迹在本地——飞书出问题 / 想换平台 / 自建前端都能继续用。Roostery 是中立中间件，不是飞书附属。
status: draft
last_reviewed: 2026-05-16
implemented_by: [2026-05-15-core-redact, 2026-05-15-journal-core]
tags: [portability, vendor-neutral, local-first, escape-hatch]
---

# 你的数据在本地——可读、可迁、可换前端

## 用户故事

- 作为遇到飞书侧异常的开发者（任务没生成 / 群消息丢了 / Docs 写错位置），我希望查到 Roostery 当时发了什么命令、飞书返了什么，而不是凭终端 buffer 猜
- 作为想给小团队 / 公司留底的人，我希望"agent 通过 Roostery 操作飞书"的全过程留在本地、可审，而不是依赖飞书云端的审计能力（在哪、看不看得到、留多久都未知）
- 作为对飞书未来风险担心的人（涨价 / 政策变化 / 合规要求 / 服务中断），我希望我累积的 agent 工作痕迹不会因此被锁死，能迁去别的协作平台或自建前端继续用，而不是从零开始
- 作为想试试"不用飞书看 agent 产出、自己拼个网页 dashboard"的开发者，我希望基于 Roostery 的本地数据就能搭，而不是去飞书 API 倒爬
- 作为想在另一台设备上接续 / 复现 agent 历史行为的开发者，我希望本地 journal 是可携带格式，能拷到另一台机器或同步盘上重跑，而不是被锁在某台机器的私有日志里
- 作为想做自动化测试 / 复现 bug 的 Roostery 用户，我希望拿历史 journal 在本地重跑一次 Roostery 验证修复，而不是每次都得真去飞书走一遍

## 为什么需要

Roostery 的名字本身在讲一件事：它是个 **roost（栖息地）**，agent 来这里栖息、离开时能带着自己的痕迹走。它不是某家协作平台的附属。

飞书是 0.x 阶段的默认呈现面，但**承诺停留在"默认"，不会演化成"绑死"**。这层承诺为什么重要：

1. **协作平台本身有迁移风险**——飞书可能涨价、合规要求变、某次更新让团队不得不切。如果所有 agent 工作痕迹被绑死在飞书里，这些风险全嫁接到 Roostery 用户头上
2. **agent dashboard 类产品的暗痛**——数据在厂家服务器，想导出 / 自建 / 切平台时基本无路可退。Roostery 不能复制这种暗痛
3. **中间件的本分**——管道不该让数据形态由两头中任一端定义。"管道里流过的东西" 留在本地、保持中立，是中间件该做的事

具体到日常：排错时本地有上下文不用瞎猜；想审计时所有"对飞书做了什么"在本地能查；想离开飞书时不必从零开始。

## 怎么解决

Roostery 把每一次"管道行为"——agent runtime 触发 hook、Roostery 调用 `lark-cli`、飞书返回结果——在本地写一行 journal（jsonl）：参数、结果、时间戳、关联的 agent run。文件是纯文本，用户能直接 `cat` / `grep` / `jq` 翻，不需要 Roostery 自己提供查询工具。

journal 的 schema 公开、稳定（每行带 version 标记）。这意味着：

- **排错**：失败那一刻发了什么、收了什么，本地有完整上下文
- **审计 / 留底**：所有 Roostery 替你做的事都在本地，长期归档 / 自查 / 给老板汇报都直接基于这份数据
- **可迁移**：飞书侧呈现是基于 journal 的一种 view。哪天你不用飞书了，journal 还在——别的人可以基于它写 Slack view / Teams view / 自建网页 dashboard，不需要从飞书反向爬数据
- **可重放**：把 journal 喂回 Roostery 能重跑一次执行，调试 / 回归测试用

这一切的前提是 **journal 是 first-class 数据，不是"为了 debug 顺手存的日志"**。这是 Roostery 跟飞书的关系底线：飞书是 default view，journal 才是 source of truth。

## 边界

- **不替代飞书云端的真实状态**——journal 记的是"我这台机器发了什么、收了什么"，不是"飞书云端最终保留了什么"。两边可能因飞书侧后续变更而不一致，以飞书为准
- **不附带其他平台的 view**——0.x 阶段 Roostery 只提供飞书 view 一种；Slack / Teams / 自建 frontend / dashboard 不是 Roostery 自带能力，是社区 / 用户可以基于公开 schema 自建的扩展点
- **不承诺一键迁移工具**——只承诺数据形态可移植（schema 公开、纯文本），不承诺"按个按钮就把飞书历史迁到 Slack"那种丝滑工具
- **不保证跨平台视觉等效**——飞书任务卡迁到别处时，UI 会因目标平台特性变化（Slack 没"任务卡"，可能映射为 thread）。Roostery 不替你定义这种映射
- **不做敏感内容的强脱敏保证**——journal 默认包含调用参数（可能含 token、文档片段、消息正文）。Roostery 对明显的密码 / token 形态做基础脱敏，最终 journal 里有什么、敏不敏感由用户自己评估
- **不是云端审计平台**——所有 journal 留本地，不上传、不跨机器汇总
- **不做长期归档管理**——文件在本地积累，rotation / 压缩 / 备份归用户管
- **不做 journal 的跨设备同步**——journal 是可移植的本地数据形态，但 Roostery 自己不替你把不同设备的 journal 合并 / 同步。多设备 agent 状态的实时跨设备整合走飞书侧（见 `agent-work-in-feishu`），不走 journal
- **replay 不替代真测**——replay 只重现 Roostery 自己这一侧，不重现飞书云端真实状态、不重现 agent runtime 的副作用

## 变更日志

- **2026-05-16** · `journal-core` 落地（commit `b9ac5be`）：`JournalEntry` schema_version=1 对外公开承诺正式生效，jsonl 写入侧基础设施 + ULID `event_id` + UTC 日切 rotation + redact 集成完成。**写入侧 req 已兑现**；read/replay API + 跨设备 / 自建 view 的具体落地仍待后续 phase——req 保持 `draft` 直至这些场景出现具体可消费的工具
- **2026-05-15** · `core-redact` 落地（commit `1e392e5`）：`scrub_value` / `scrub_argv` / `scrub_text` + `MASK` + 11 个 `SENSITIVE_KEYS`。兑现"基础脱敏"边界；为 journal 写入提供 logging-boundary 脱敏前置
