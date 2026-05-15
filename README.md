# 🪺 Roostery

> 🚧 **Rust 重写中 / Rust Rewrite In Progress** — 仓库未发版，仅作围观。
>
> A roost for your agent flock — vendor-neutral agent broker, Feishu-native. 飞书原生的多 Agent 中立接入器，让任意 agent runtime（OpenClaw / Claude Code / Codex / Cursor / Gemini ...）在飞书生态里栖息、协作、被监督。

---

## 状态

🚧 **Rust 重写中 / Rust Rewrite In Progress**（自 2026-05-15）

仓库未发布版本。Python baseline 归档在 `legacy/python/` 仅作 reference，活跃实现在 `crates/roostery/`。完整 Rust 重写路线图（21 个 feature / 7 阶段）见 `.codestable/roadmap/rust-rewrite/`，CodeStable 规范体系见 `.codestable/`。

仍处于不可装机状态——0.1.0 release 等到 Rust 达到"可用"形态（Phase 5 完成，CC headless 能在飞书出 task）。版本策略决议见 `.codestable/brainstorms/v0.x-direction/`。

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
