# Attention

本文件是 CodeStable 技能启动必读的项目注意事项入口。所有 CodeStable 子技能开始工作前必须读取它。

## 项目碎片知识

<!-- cs-note managed: 用 cs-note 维护，新条目按下面分节追加 -->

### 编译与构建

- npm 上的 `roostery` 包名只是 **namespace reservation**（防被他人占用），仓库无实际 JS 代码入口；Roostery 是 Rust 项目（Python baseline 重写中）

### 运行与本地起服务

- `lark-cli` 版本 **pin 在 1.0.28**（特别是 `task append_task_steps` timestamp schema 兼容性）；升级前必须本地跑 smoke 验证矩阵，任意 probe 失败 `roostery init` 和 `daily_report` 拒绝运行
- `lark-cli` shim 安装到 `~/.local/bin/lark-cli`（PATH-prefix shim 透传 + 写 journal）；要求 `~/.local/bin` 在 PATH 前段才能拦截到真 `lark-cli`，`roostery init` 会校验

### 测试

- 飞书相关功能测试一律用 `LarkRunner` trait 的 mock 实现（Rust Phase 2 起）；不要写跑真飞书的测试除非显式标 `#[ignore]` e2e 并由人手跑

### 命令与脚本陷阱

- 飞书 API **必经 `lark-cli`**（`lark_cli.py` / `lark_cli.rs` subprocess wrapper）——不允许直接 `requests` / `reqwest` / 任何 HTTP client 打 `open.feishu.cn`，也不允许引 Feishu SDK；架构红线，code review 拒收
- agent runtime 的 Stop hook 安装走 `roostery init`（Rust 期 hooks_merge feature），**不要手动编辑** `~/.claude/settings.json` / `~/.codex/hooks.json` 注入——会跟下次 init 的深合并冲突

### 路径与目录约定

- `~/.feishu_hub/` 下所有 state（journal / session cache / budget）是**可重放审计 cache，不是 source of truth**——回答"任务 X 现在状态如何"必须查飞书侧（Feishu Task / IM / Base），不查本地 state

### 环境变量与凭证

- LLM provider 客户端**只允许在 `llm_summary.py` / `llm_summary.rs` import**；其他模块出现 OpenAI / Anthropic / GA-style llmcore client / `reqwest` 直连 LLM 都会被 review 拒——架构红线

### 其他

- 项目命名是 **Roostery 而非 feishu-xxx / lark-agent-xxx**——刻意强调"中立中间件主体性"，不要在新建文档 / 代码 / 命名里把 Roostery 写成飞书附属（"feishu hub"、"lark agent broker"等暗示是飞书周边工具的措辞）。飞书是 default view，不是 lock-in
- **代码-文档优先级**：当 Python baseline 代码与最新文档（`.codestable/requirements/` / `.codestable/architecture/` / `.codestable/roadmap/` / `CLAUDE.md` 等）不一致时**以文档为准**，Python 仅作 reference。原因：Python 是 prior `feishu_hub` baseline import，未严格对齐 vendor-neutral / portable-by-default 等愿景，是"上次的实现"不是"应该的实现"。Rust port 不机械 1:1 翻译；失配点记入 `.codestable/roadmap/rust-rewrite/` §7 观察项或独立 compound learning 笔记，必要时走 `cs-req update` / `cs-roadmap update` 改文档再继续
