# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-05-18

🎯 **首个 Rust release**——"Rust 可用 + B 类自托管验收形态" 达成。Phase 5 minimal-loop closing：CC headless / 任意 agent 都能在飞书出 task 卡 + step stream，多设备同步可见。

按 Rust 重写路线图 Phase 0 → 5 顺序列出 17 个落地 feature。

### Added

**Phase 0 — Scaffold**

- **rust-scaffold** (`2026-05-15-rust-scaffold`)：建 Cargo workspace + 归档 Python baseline 到 `legacy/python/` + 配 GitHub Actions CI（fmt / clippy / test on ubuntu-latest）

**Phase 1 — Module A/B Foundations**

- **journal-core** (`2026-05-15-journal-core`)：`JournalEntry` schema（roadmap §4.2，`schema_version=1` 对外承诺）+ jsonl 原子 append + 目录迁 `~/.roostery/`（`ROOSTERY_HOME` env 覆盖）
- **core-redact** (`2026-05-15-core-redact`)：redact 模块敏感字段脱敏 + 审计 path；纯函数无 I/O；journal 写入前置 scrub_argv
- **core-remoterefs** (`2026-05-16-core-remoterefs`)：9 个 newtype token 类型隔离（含 Phase 5 必需的 `TaskId` / `ThreadId`）+ 单趟 match-walk in-place 抽取 + `AsRef` / `Display` ergonomics + `non_exhaustive` 向前兼容

**Phase 2 — Module C Feishu Syscall**

- **lark-cli-wrapper** (`2026-05-16-lark-cli-wrapper`)：`LarkRunner` trait（roadmap §4.1 rich enum + thiserror）+ LarkCli subprocess 实现（async/tokio）+ `MockLarkRunner` fluent enqueue API + `Journaled<R>` 装饰器（写 journal 前后过 `redact::scrub_argv`）
- **lark-cli-shim** (`2026-05-17-lark-cli-shim`)：`bin/shim` 独立二进制——PATH-prefix 透传 lark-cli + 流式 tee + 写 `JournalEntry`；TTY/interactive 走 `execv` 直通；anti-recursion + `NOJOURNAL` env 旁路
- **roostery-smoke** (`2026-05-17-roostery-smoke`)：smoke 模块 + `roostery smoke` 子命令——6 条 `lark-cli --dry-run` probe 矩阵（im / docs / drive），结果写 `~/.roostery/state/smoke.json`；`smoke::ensure_ready()` 给后续 init / daily_report 做守门

**Phase 3 — Module D Local Config & Install**

- **config-yaml** (`2026-05-17-config-yaml`)：`Config` schema 强类型化（identity / budgets / trace / journal 强类型 + runners 开放 `BTreeMap<String, Value>`）；`pub fn load / save / load_from / save_to`；`schema_version=1`
- **hooks-merge** (`2026-05-18-hooks-merge`)：JSON 深合并把 Stop hook 片段注入 `~/.claude/settings.json` / `~/.codex/hooks.json`，幂等去重（event key + matcher + command 尾匹配）；3 个模板（cc / codex / sh bridge）`include_str!` 编译期嵌入
- **roostery-init** (`2026-05-18-roostery-init`)：`roostery init` 单命令装机编排——smoke gate / state dir / identity reflect / agent detect / shim install / sh bridge / 3-runtime hook merge / `~/.roostery/env` 写 `ROOSTERY_REAL_LARK_CLI` / shell rc marker block 幂等 patch
- **init-real-lark-cli-override** (`2026-05-18-init-real-lark-cli-override`)：`roostery init` 加 `--real-lark-cli <path>` flag + 复用 `ROOSTERY_LARK_CLI_BIN` env override；resolve 上移到 F1 早 gate；错误信息拆 3 sub-variant 含 fix hint

**Phase 4 — Module E Dispatcher**

- **dispatcher-trace-budget** (`2026-05-18-dispatcher-trace-budget`)：`TraceContext`（trace_id / parent_event_id / depth / max_depth）+ Budget gate（default bucket f64 USD，原子持久化 `~/.roostery/state/budget.json` + 跨日 rollover）+ `RunawayTracker`
- **dispatcher-rules** (`2026-05-18-dispatcher-rules`)：Rule 模块 YAML schema v1 + 编译 + 匹配。Match 3 维 MVP（hook_source eq / workspace_glob fnmatch / trigger_meta 点路径 eq）；Action = opaque 透传
- **dispatcher-runners** (`2026-05-18-dispatcher-runners`)：`Runner` trait（roadmap §4.3 微偏 budget 移出）+ `noop` / `cc_headless` 两实现 + `runner_registry` 线性查找；`cc_headless` 调 `claude -p ... --output-format json`
- **dispatcher-loop** (`2026-05-18-dispatcher-loop`)：把 trace / budget / runaway / rules / runners 5 个 gate / engine 串成 `HookEvent in → RunOutcome out + journal` 主链路；`roostery dispatcher fire / replay / test-rule` 三子命令

**Phase 5 — Module F Bot Bridge（minimal-loop closing ⭐）**

- **bot-task-writer** (`2026-05-18-bot-task-writer`)：3 pub async fn 纯库 API（`create_task` / `append_steps` / `get_or_create_for_session`）+ session_cache JSON v1 持久化 + host suffix 多机区分 + `safe_filename` 路径跳出防御；首次让 Rust 业务模块真消费 `LarkRunner` trait 做生产飞书 IO
- **bot-stop-hook** (`2026-05-18-bot-stop-hook`) ⭐：双 CLI surface 共享 `bot::push` 核心。`roostery bot stop-hook` 接 CC/Codex/Gemini SessionEnd stdin JSON 做被动 hook 入口；`roostery bot push --agent X --session Y --summary "..." --json --strict` 是面向任意 agent / 脚本 / cron / CI 的反向 push CLI——vendor-neutral broker 定位真兑现。**0.1.0 触发判据达成**

### Fixed

- **init UX**：npm 全局 prefix == shim target (`~/.local/bin/lark-cli`) 时 `RealLarkCliMissing` fail + live 模式失败留破损态——issue `2026-05-18-init-shim-conflicts-npm-prefix` 已由 `init-real-lark-cli-override` 根治

### Documentation

- README 重写为 user-why leading 五段结构（多设备 vibecoding 痛点 + 数据主权前置；技术属性下移）；Quickstart 含 `roostery init --real-lark-cli` + `roostery bot push --json`（feature `2026-05-18-release-0.1.0-prep`）
- CHANGELOG.md 起步——Keep a Changelog 格式 `[0.1.0]` 章节按 Added/Fixed/Documentation 分类

### Metadata

- workspace `Cargo.toml` 补 `description` / `keywords` / `categories` / `readme` / `homepage` / `documentation` / `rust-version` 七字段——为 0.2.0 crates.io publish 预热（0.1.x 本期**不** publish；决议见 `.codestable/brainstorms/v0.x-direction/`）
- crate `Cargo.toml` `description` 改 `description.workspace = true` 单点维护

[Unreleased]: https://github.com/bendusy/roostery/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/bendusy/roostery/releases/tag/v0.1.0
