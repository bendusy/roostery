# Roostery 能力清单

> 项目愿景层索引。每条 req 描述一个用户可感能力——为什么要有、怎么解决、边界在哪。技术怎么搭看 `.codestable/architecture/`。

## current

_（暂无 current 能力——项目处于 planning 阶段，Rust 重写中，无对外 release）_

## draft

- [Agent 工作过程长在飞书里](agent-work-in-feishu.md) — 让 agent 工具的产出长在你飞书里：不切工具、不交给第三方 dashboard、不重学新界面
- [不绑 agent runtime 的中立接入](runtime-neutral.md) — 不被某家 agent runtime 绑死：CC / Codex / Gemini / 自己写的都能在同一套飞书面里出活
- [你的数据在本地——可读、可迁、可换前端](portable-by-default.md) — Roostery 是中立中间件，不是飞书附属：飞书出问题 / 想换平台 / 自建前端都能继续用

## outdated

_（无）_

---

## 整理后的合并 / 落档说明

- 原 brainstorm 的 "本地自托管 + 数据主权" 已合并进 `agent-work-in-feishu`（pitch 的"不交给第三方 dashboard" + 第 4 条用户故事），不单独立 req
- 原 brainstorm 的 "agent 行为级审计 / replay" 已合并进 `portable-by-default`——audit / replay 是 portable journal 的副作用，不是独立能力
