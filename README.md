# 🪺 Roostery

**手机也能跟进电脑上的 agent。** 把多设备 vibecoding 的 agent 工作过程长在飞书里——任务卡 + 步骤流 + 群消息 thread 跨设备同步，不必 SSH 回桌面机看终端。数据全程留在你自己的飞书租户，不经第三方 dashboard。

> A roost for your agent flock — vendor-neutral, Feishu-native agent broker.

---

## 状态

🎯 **v0.1.0 已发**（2026-05-18）—— Rust 可用形态达成，Phase 5 minimal-loop 闭环：CC headless / 任意 agent 都能在飞书出 task 卡和 step stream。

- 活跃实现：`crates/roostery/`（Rust workspace）
- Python baseline：`legacy/python/` 仅作 reference，不再维护，Phase 7 删除
- 完整 Rust 重写路线图（21 feature / 7 阶段）：`.codestable/roadmap/rust-rewrite/`
- 版本策略 / B 类自托管定位：`.codestable/brainstorms/v0.x-direction/`

**前提**：飞书租户（能创建任务和群）+ 配好的 [`lark-cli`](https://github.com/larksuite/lark-cli)（≥ 1.0.28）+ 至少一个本地 agent runtime（Claude Code / Codex / Gemini / 自定义皆可）。

## Quickstart

```bash
# 1. 装 roostery（0.1.x 暂不上 crates.io，从 git 装）
cargo install --git https://github.com/bendusy/roostery --tag v0.1.0 --bin roostery --bin shim

# 2. 装机：smoke gate / shim 安装 / agent hook 注入一条龙
roostery init --real-lark-cli "$(realpath "$(which lark-cli)")"

# 3. 任意 agent / 脚本 / CI 主动推一条进度到飞书
echo "build green, tests passing" | \
  roostery bot push --agent ci --session run-1 --summary-stdin --json
```

跑完手机打开飞书 app，应能在默认 task 挂载点看到这条记录的 task 卡。配置 agent runtime（CC / Codex / Gemini）的 SessionEnd hook 后，agent 跑完任务会自动 fire `roostery bot stop-hook`，task 卡 + IM thread 自动出现。

## 核心定位

- **vendor-neutral**：任意 agent runtime（CC / Codex / Gemini / 自定义 Python）/ 任意 LLM provider 皆可接入；不造 runtime，只做中立接入
- **Feishu-native**：飞书 Task / IM / Docs / Base 作为天然呈现层 + 数据底，复用云协作面已做好的多设备同步层
- **local-first**：本地 daemon 主导调度，所有 state 在 `~/.roostery/`（可重放审计 cache，不是 source of truth）
- **developer-priority**：code-defined workflow（yaml / Rust），非 GUI 拖拽

## 跟谁错位

| 类别 | 已有产品 | Roostery |
|---|---|---|
| 通用 agent dashboard | builderz-labs/mission-control | 飞书原生 UI 复用 |
| Canvas 多线程 agent | NoteLoom / Flowith / jaaz | 数据不假于人，飞书租户内闭环 |
| Agent runtime | OpenClaw / Claude Code / Codex / Cursor | 不造 runtime，做中立接入 |
| 商业 SaaS | AgentCenter / Manus | 自托管 + 数据主权 |

## License

MIT
