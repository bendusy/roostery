# 🪺 Roostery

> A roost for your agent flock — vendor-neutral agent broker, Feishu-native.
>
> 飞书原生的多 Agent 中立接入器。让任意 agent runtime（OpenClaw / Claude Code / Codex / Cursor / Gemini ...）在飞书生态里栖息、协作、被监督。

---

## 状态

🚧 **筹划期 / Planning Phase**（2026-05-15）

本仓库正在筹备开源。Spec 与实现尚未公开。欢迎围观。

## 核心定位

- **vendor-neutral**：任意 agent runtime / 任意 LLM provider，皆可接入
- **Feishu-native**：飞书 Base / IM / 画板 / 任务 作为天然呈现层 + 数据底
- **local-first**：本地 daemon 主导调度，数据留在用户飞书租户
- **developer-priority**：code-defined workflow（yaml/Python），非 GUI 拖拽
- **MIT licensed**：开源，自托管

## 跟谁错位

| 类别 | 已有产品 | Roostery |
|---|---|---|
| 通用 agent dashboard | builderz-labs/mission-control | 飞书原生 UI 复用 |
| Canvas 多线程 agent | NoteLoom / Flowith / jaaz | 数据不假于人，飞书租户内闭环 |
| Agent runtime | OpenClaw / Claude Code / Codex / Cursor | 不造 runtime，做中立接入 |
| 商业 SaaS | AgentCenter / Manus | 自托管 + 数据主权 |

## 引擎

- 多 Agent runtime adapter（首批：OpenClaw / Claude Code / Codex / Gemini / Python custom）
- 多 LLM provider router
- 多形态入口并行（IM chat / Base 字段改写 / 画板 / 任务卡片 / CLI / HTTP）
- launchd / systemd 本地守护
- agent 行为级审计

## License

MIT
