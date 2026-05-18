# Attention

本文件是 CodeStable 技能启动必读的项目注意事项入口。所有 CodeStable 子技能开始工作前必须读取它。

## 项目碎片知识

<!-- cs-note managed: 用 cs-note 维护，新条目按下面分节追加 -->

### 编译与构建

- npm 上的 `roostery` 包名只是 **namespace reservation**（防被他人占用），仓库无实际 JS 代码入口；Roostery 是 Rust 项目（Python baseline 重写中）

### 运行与本地起服务

- `lark-cli` 版本**最低 pin 在 1.0.28**（特别是 `task append_task_steps` timestamp schema 兼容性）；**1.0.29 已实测兼容**（feature `2026-05-17-roostery-smoke` 2026-05-17 跑通 6 条 probe）；升级前必须本地跑 `roostery smoke` 验证矩阵，任意 probe 失败 `roostery init` 和 `daily_report` 拒绝运行
- `lark-cli` shim 安装到 `~/.local/bin/lark-cli`（PATH-prefix shim 透传 + 写 journal）；要求 `~/.local/bin` 在 PATH 前段才能拦截到真 `lark-cli`，`roostery init` 会校验

### 测试

- 飞书相关功能测试一律用 `LarkRunner` trait 的 mock 实现（Rust Phase 2 起）；不要写跑真飞书的测试除非显式标 `#[ignore]` e2e 并由人手跑
- Rust 2024 edition `std::env::set_var` / `remove_var` 是 `unsafe`，写 env 的生产代码需 `unsafe {}` 块。测试中并发触碰 env **必须用 crate-wide 共享 Mutex 串行化**——用 `crate::paths::TEST_ENV_LOCK`（而非各模块自己声明 `static Mutex<()>`）。**修订原因**（bot-stop-hook feature S10.5 2026-05-18）：之前每个 mod 在 `mod tests` 里各自声明 ENV_LOCK，多 mod 同时跑触碰同 env var（如 `ROOSTERY_HOME`）时 race，一旦因 race 失败 panic 还会 poison 该 mod 的 lock 连锁拖挂同 mod 后续 env 测试。**Corollary**：任何在 `fn` 内消费 `paths::roostery_home()` / `paths::journal_dir()` 等 env-dependent helper 的测试也要锁——典型如 config roundtrip 测试，`Config::default()` 里 `journal.dir = paths::journal_dir()` 会读 env 当前值，race 会让 before/after snapshot 不等
- 测试中创建可执行 fixture 文件再立即 `execve` 时用 `std::fs::write(path, content)` **不要** `File::create + write_all + drop` —— Linux 后者有 ETXTBSY race（fd close 与 execve 之间窗口；macOS 不报这个错），CI 偶发 `ExecutableFileBusy`。参考 `crates/roostery/src/lark_cli/subprocess.rs::fixture_script`

### 命令与脚本陷阱

- 飞书 API **必经 `lark-cli`**（`lark_cli.py` / `lark_cli.rs` subprocess wrapper）——不允许直接 `requests` / `reqwest` / 任何 HTTP client 打 `open.feishu.cn`，也不允许引 Feishu SDK；架构红线，code review 拒收
- agent runtime 的 Stop hook 安装走 `roostery init`（Rust 期 hooks_merge feature），**不要手动编辑** `~/.claude/settings.json` / `~/.codex/hooks.json` 注入——会跟下次 init 的深合并冲突
- 仓库里看到 `*(*的冲突副本*_YYYY-MM-DD HH-MM-SS).*`（macOS iCloud / Dropbox / Syncthing 多机同步生成的 stale 快照）直接 `rm`——cargo 不会编译它（文件名带括号 + 不在 mod tree），但 git status untracked 会让人困惑；原始内容已在正常文件里
- Rust `#[non_exhaustive]` struct 从外部 crate **完全不允许** struct literal——包括 `..Default::default()` 也会触发 rustc E0639。必须配 builder API（参考 `RunOptions::new().with_timeout(d)`）；新引入 non_exhaustive 容器 struct 时同时加 `new() + with_*` 链不要假设 `..Default::default()` 旁路。**测试 fixture 侧 corollary**（跨 crate integ test 想预置 non_exhaustive 类型作为 seed 时）：用 `serde_json::from_str` 反序列化 JSON 字面量绕过（如 `tests/onboarding_integration.rs::seed_passing_smoke` 写 raw JSON 到 `~/.roostery/state/smoke.json`），或挑非 non_exhaustive 的 enum variant 作为代表（`LarkError::Timeout { timeout_ms }` 比 `NonZeroExit` 更易构造）

### 路径与目录约定

- 本地 state 根目录：**Rust 期 `~/.roostery/`**（自 journal-core 起；env 覆盖 `ROOSTERY_HOME`）；Python 期 `~/.feishu_hub/` / `FEISHU_HUB_HOME`（legacy）。所有 state（journal / session cache / budget）是**可重放审计 cache，不是 source of truth**——回答"任务 X 现在状态如何"必须查飞书侧（Feishu Task / IM / Base），不查本地 state

### 环境变量与凭证

- LLM provider 客户端**只允许在 `llm_summary.py` / `llm_summary.rs` import**；其他模块出现 OpenAI / Anthropic / GA-style llmcore client / `reqwest` 直连 LLM 都会被 review 拒——架构红线

### 其他

- 项目命名是 **Roostery 而非 feishu-xxx / lark-agent-xxx**——刻意强调"中立中间件主体性"，不要在新建文档 / 代码 / 命名里把 Roostery 写成飞书附属（"feishu hub"、"lark agent broker"等暗示是飞书周边工具的措辞）。飞书是 default view，不是 lock-in
- **代码-文档优先级**：当 Python baseline 代码与最新文档（`.codestable/requirements/` / `.codestable/architecture/` / `.codestable/roadmap/` / `CLAUDE.md` 等）不一致时**以文档为准**，Python 仅作 reference。原因：Python 是 prior `feishu_hub` baseline import，未严格对齐 vendor-neutral / portable-by-default 等愿景，是"上次的实现"不是"应该的实现"。Rust port 不机械 1:1 翻译；失配点记入 `.codestable/roadmap/rust-rewrite/` §7 观察项或独立 compound learning 笔记，必要时走 `cs-req update` / `cs-roadmap update` 改文档再继续
