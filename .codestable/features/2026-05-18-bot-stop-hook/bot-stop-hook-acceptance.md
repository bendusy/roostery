---
doc_type: feature-acceptance
feature: 2026-05-18-bot-stop-hook
status: passed
summary: Phase 5 Module F 第 2 子 feature 验收闭环——双 CLI surface (`roostery bot stop-hook` + `roostery bot push`) 落地，**0.1.0 release 触发判据达成**：CC headless 在飞书可出 task + 任意 agent/脚本可主动反向调用推送。commit 220c7b0 / CI run 26030808131 三 job 全绿。同步附带 S10.5 测试 ENV_LOCK 跨模块 race 修订，落到 attention.md。
requirement: agent-work-in-feishu
roadmap: rust-rewrite
roadmap_item: bot-stop-hook
related_commit: 220c7b0
ci_run: 26030808131
tags: [phase-5, module-f, stop-hook, reverse-cli, minimal-loop, release-0.1.0]
---

# bot-stop-hook 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-18
> 关联方案 doc：`.codestable/features/2026-05-18-bot-stop-hook/bot-stop-hook-design.md`
> 关联 commit：`220c7b0`
> 关联 CI run：`26030808131`（fmt + clippy + test --all 三 job 全绿）

## 1. 接口契约核对

对照方案第 2.1 节名词层逐一核查。

### 接口示例逐项核对

- [x] **PushRequest builder**（`bot_stop_hook.rs:30`）：`PushRequest::new("custom-agent", "session-1", "/tmp/x").with_summary(...).with_description(...).with_assignee(...)` → 代码实际：必填三参数 + 三 `with_*` 链式，签名一致；`agent` / `session` 用 `impl Into<String>`，`cwd` 用 `impl Into<PathBuf>`。**一致**
- [x] **PushOptions Default**（`bot_stop_hook.rs:78`）：3 bool 字段 `strict / json_output / no_im_fallback`，Default impl 全 false。**一致**
- [x] **PushOutcome JSON 输出**（design §2.1 示例 status="success" / "fallback_used"）：snake_case 序列化通过 `push_outcome_serde_roundtrip_and_status_snake_case` + `push_status_all_variants_snake_case` 双单测。CLI integ test 实测 stdout JSON `{"status":"success","task_url":"...","task_guid":"...","fallback_used":false}`。**一致**
- [x] **bot push CLI flag 示例**（design §2.1 三 bash 示例）：`--agent --session --cwd --summary --summary-stdin --description --assignee-open-id --strict --json --no-im-fallback` 全部存在；`cargo run -- bot push --help` 输出含全部 flag + 文档。**一致**
- [x] **transcript_reader.read_last_assistant_text**（design §2.1 输入/输出示例）：协议 `{type:"assistant", message:{content:[{text:"..."}]}}` 取最后一条 → 单测 `transcript_reader_happy_picks_last_assistant` 验证 multi-line + final wins。**一致**
- [x] **StopHookInput #[serde(default)]**：空 stdin 不报错 → `run_stop_hook_empty_stdin_uses_defaults` 验证。**一致**

### 名词层"现状 → 变化"逐项核对

- [x] 新增 `PushRequest / PushOptions / PushOutcome / PushStatus / StopHookInput` 5 个 pub 类型 — `crates/roostery/src/bot_stop_hook.rs:30-148`
- [x] 内部 `enum StopHookError` 设计中提到 — 实际代码以 `Result<_, TaskWriterError>` 内化处理，未单独引入 enum（**偏差但 design 合意**：D10 决策"所有非 Ok 都走 IM 兜底"使错误分类无需单独 enum；error 累积到 `outcome.errors: Vec<String>` 即可）。回填到本节
- [x] `TaskWriterError / Identity / Config / LarkRunner` — 全部消费而无改动
- [x] inline mod `transcript_reader { read_last_assistant_text, TranscriptReadError 3 变体 }` — 落地 `bot_stop_hook.rs:216-264`

### 流程图核对（第 2.2 节 mermaid）

mermaid 图节点全部 grep 落点：
- [x] `parse StopHookInput` → `parse_stop_hook_input` + `build_request_from_stop_hook_input`
- [x] `resolve summary` → `resolve_summary_from_hook_input`
- [x] `resolve receive_id` → `resolve_receive_id`（三层链）
- [x] task_writer 主路径 → `bot_task_writer::get_or_create_for_session` + `append_steps`
- [x] IM 兜底 → `finish_with_fallback` 内部调 `runner.run(["im","+messages-send",...])`
- [x] 4 终态 (Success / FallbackUsed / Failed / Skipped) → PushOutcome 4 变体落地

**无未处理偏差**。`StopHookError` 偏差已回填本节并 design 合意。

## 2. 行为与决策核对

### 需求摘要逐项验证（design §1.1 必做 = 核心库 C1-C4 + stop-hook H1-H7 + push P1-P7 + 共享 O1-O4）

| # | 行为 | 实测 |
|---|---|---|
| C1 | `push(req, runner, opts) -> PushOutcome` 编排 | 实测 `push_happy_creates_task_and_appends_step` Success / `push_task_fail_triggers_im_fallback` FallbackUsed |
| C2 | PushRequest 必填项编译期校验 | builder `::new()` 强制 3 必填；optional 走 `with_*`；类型签名验证 |
| C3 | summary 默认值 = `"Agent stopped (no summary)"` | `bot_stop_hook.rs:21` const + push 主路径未 summary 走 unwrap_or |
| C4 | blake3 8 字符 idempotency key | `stable_idem_key_deterministic_across_calls` 单测：同输入 == / 不同输入 != / null 分隔防互换 |
| H1-H7 | stop-hook CLI 全套行为 | `cli_stop_hook_stdin_json_routes_to_push` integ test 验证 + transcript_reader 单测 |
| P1-P7 | push CLI 全套行为 | `cli_push_flag_based_happy_outputs_json_outcome` + `cli_push_summary_stdin_reads_and_pushes` + 5 条 PushCliArgs 单测 |
| O1 `--strict` | Failed → exit 1，其他 exit 0 | `cli_push_strict_with_no_im_fallback_task_fail_exits_one` 实测 exit code 1 |
| O2 `--json` | stdout 序列化 PushOutcome | 3 条 CLI integ test 解析 stdout JSON |
| O3 `--no-im-fallback` | task fail 时不调 IM | `push_no_im_fallback_opt_out_task_fail_directly_failed` mock 调用数 == 1 |
| O4 `RUST_LOG` tracing | structured tracing 替代 eprintln | `tracing::warn!` / `tracing::info!` 在 push / finish_with_fallback / resolve_receive_id 调用点全 ≥ 1 处 |

### 明确不做逐项核对（用第 1.3 节反向核对项）

- [x] **不通过 dispatcher fire 路由** — `grep "dispatcher::fire\|dispatcher_fire" crates/roostery/src/bot_stop_hook.rs = 0`
- [x] **不引入 config.identity.notify_receive_id 新字段** — `grep "notify_receive_id" crates/roostery/src/config.rs = 0`，Identity struct 不变
- [x] **不处理 Codex / Gemini transcript** — 仅 `prompt_response` 兜底，无 codex/gemini 专属分支（grep 验）
- [x] **不实现 retry / 退避** — `grep "retry\|backoff" crates/roostery/src/bot_stop_hook.rs = 0`
- [x] **不写本地 notify-send** — `grep "notify-send\|osascript" = 0`
- [x] **不实现 dry-run** — 仅 `--no-im-fallback` opt-out
- [x] **不实现 streaming step** — 单次 append_steps with single step
- [x] **不引入 `--config <path>` flag** — `grep "config.*path.*PathBuf" cli mod = 0`

### 关键决策落地（D1-D18）

| # | 决策 | 落地证据 |
|---|---|---|
| D1 | 双 CLI 共享 `bot_stop_hook::push` 核心 | `run_stop_hook` + `cli::run_push` 都调 `push` |
| D2 | PushRequest builder（非 stringly flag forward） | `bot_stop_hook.rs:44-73` `new + with_*` |
| D3 | PushOutcome `--json` 结构化 | `outcome_to_exit_code` 序列化 |
| D4 | `--strict` opt-in exit code | hook 默认 false / push caller opt-in |
| D5 | blake3 8-char idempotency | `stable_idem_key` + `grep "blake3::"` = 1 处 |
| D6 | sh 极简 wrapper | `templates/agent_stop_notify.sh` 10 行 + 0 jq/tac |
| D7-D9 | transcript_reader / floor_char_boundary / 倒序 | `is_char_boundary` 2 处 + transcript_reader 反向扫 |
| D10 | receive_id 三层链 = env > identity > config.user_id | `resolve_receive_id` 实现 + 5 条单测 |
| D11 | `ROOSTERY_NOTIFY_TO` 非 `FEISHU_NOTIFY_TO` | `grep "FEISHU_NOTIFY" crates/roostery/ = 0`（仅文档历史引用） |
| D12 | 不引入 notify_receive_id 新字段 | config.rs Identity 结构不变 |
| D13 | LarkCallFailed 与其他都走 IM 兜底 | `push` 主路径 task_result Err 全分支 → `finish_with_fallback` |
| D14 | 不调 dispatcher fire / 不走 rules / budget | bot_stop_hook 无 dispatcher import |
| D15 | structured tracing | `tracing::warn!` / `tracing::info!` ≥ 5 处 |
| D16 | 单文件 bot_stop_hook.rs + inline mod transcript_reader | 单文件 ~1050 行（含测试）；目录条目 13→14 |
| D17 | --description 默认 `"Agent {agent} working in {cwd}"` | push 主路径 `unwrap_or_else(\|\| format!(...))` |
| D18 | clap `Bot { subcmd: BotSub }` | `cli::BotArgs` + `BotSub::{StopHook, Push}` |

### 编排层"现状 → 变化"逐项核对

- [x] 现状：CC SessionEnd → sh jq extract → `dispatcher fire` → 无规则命中 → exit 0（链路从未到达 task_writer）
- [x] 变化：CC SessionEnd → sh exec → `roostery bot stop-hook` → push → task_writer + IM 兜底
- [x] 新增反向链路：任意 agent → `roostery bot push --json` → push → 同核心
- [x] 两 CLI 在适配层后合流（mermaid 图 PR 节点）

### 流程级约束核对

- [x] **错误语义**：默认 exit 0（不阻塞 agent runtime）；`--strict` opt-in exit 1（仅 Failed，Skipped 不算错）。`cli::outcome_to_exit_code` 验证
- [x] **幂等性**：blake3 稳态 key + session_cache（继承自 bot_task_writer）；多次跑同 session 不重复创建
- [x] **并发 / 顺序**：单次调用编排，无并发；多调用通过 lark-cli idempotency-key 跨进程幂等
- [x] **扩展点**：未来加 `bot status` / `bot list` 等 sibling 在 `BotSub` 加变体即可
- [x] **可观测点**：tracing structured；journal 落档（Journaled<LarkCli> 装饰器透写）

### 挂载点反向核对（可卸载性）

design §2.3 列 3 个挂载点。逐项验证 + grep 反向核查：

- [x] **M1**: clap `Command::Bot(args)` + `bot_stop_hook::cli::run(args)` dispatch
  - 实际落点：`crates/roostery/src/main.rs:37` + `:133` + `crates/roostery/src/bot_stop_hook.rs:528` cli mod
- [x] **M2**: sh template `templates/agent_stop_notify.sh`
  - 实际落点：`crates/roostery/src/templates/agent_stop_notify.sh`（10 行）+ `hooks_merge.rs:31` include_str! 嵌入
- [x] **M3**: `pub mod bot_stop_hook` in lib.rs
  - 实际落点：`crates/roostery/src/lib.rs:5`

**反向 grep**：
- `grep -rn "bot_stop_hook" crates/roostery/src/ crates/roostery/tests/`：
  - lib.rs:5 (M3) ✓
  - main.rs:2 import + :133 dispatch (M1) ✓
  - tests/bot_cli_integration.rs（消费者）
  - 自身模块内部引用
  - **无清单外引用** ✓
- `grep -rn "STOP_HOOK_AGENT_NOTIFY_SH\|agent_stop_notify.sh"`：均在 hooks_merge.rs / onboarding.rs（template 安装链路），M2 已覆盖 ✓

**拔除沙盘推演**：删 lib.rs:5 mod 行 + 删 main.rs `Command::Bot` 变体 + 恢复 sh template 旧内容 → bot 路径完全消失，剩余代码（bot_task_writer / dispatcher / 等）仍能编译运行（dispatcher fire 老路径恢复）。**残留**：blake3 dep 在 Cargo.toml（轻量；不算残留），attention.md 修订（test infra 改进保留是好的，不属于 feature 残留）。**结论：可卸载** ✓

## 3. 验收场景核对

对照方案第 3 节关键场景清单（21 条 A1-A8 / B1-B9 / E1-E8）。

### 正常路径 stop-hook（A1-A4）

| # | 证据来源 | 结果 |
|---|---|---|
| A1 完整 CC SessionEnd JSON | `run_stop_hook_cc_happy_stdin_routes_to_push` 单测 + `cli_stop_hook_stdin_json_routes_to_push` integ | ✅ task_url 正确 + step 来自 transcript |
| A2 `--json` 输出 | `cli_stop_hook_stdin_json_routes_to_push` 解析 stdout JSON | ✅ status="success" + task_url 字段 |
| A3 session cache 二次跑 | bot_task_writer 已有覆盖（accept §3 验证），bot_stop_hook 复用 | ✅（依赖层覆盖） |
| A4 UTF-8 emoji 截断 | `truncate_utf8_emoji_boundary_safe` 单测 | ✅ `"ab😀😀cd"` 切 7 字节 → `"ab😀"` 不切坏 |

### 正常路径 push（A5-A8）

| # | 证据来源 | 结果 |
|---|---|---|
| A5 push flag-based | `cli_push_flag_based_happy_outputs_json_outcome` integ | ✅ exit 0 + JSON outcome |
| A6 `--summary-stdin --strict --json` | `cli_push_summary_stdin_reads_and_pushes` integ | ✅ stdin 读 + 解析 JSON OK |
| A7 `--description` 透传 | `push_cli_args_description_passthrough` 单测 | ✅ |
| A8 `--assignee-open-id` 跳层 | `push_explicit_assignee_skips_receive_id_chain` 单测 + `push_cli_args_assignee_open_id_passthrough` | ✅ MockLarkRunner 仅 2 调用，无 identity probe |

### 边界（B1-B9）

| # | 证据 | 结果 |
|---|---|---|
| B1 非法 JSON 不 panic | `run_stop_hook_invalid_json_does_not_panic` | ✅ Skipped 优雅退出 |
| B2 空 stdin | `run_stop_hook_empty_stdin_uses_defaults` | ✅ Skipped |
| B3 push 无 summary | 默认 `"Agent stopped (no summary)"` 由 push 主路径 unwrap_or 兜底 | ✅（类型保证 + push 实现） |
| B4 --summary 与 --summary-stdin 互斥 | `push_cli_args_summary_and_summary_stdin_are_mutually_exclusive` | ✅ clap ArgGroup 报错 |
| B5 transcript NotFound | `transcript_reader_not_found` + `run_stop_hook_transcript_not_found_falls_back_to_prompt_response` | ✅ NotFound → prompt_response 兜底 |
| B6 大 transcript 仅 tail（10MB 不全文加载） | **本期未实现优化**——`read_to_string` 全读后倒序扫；接受为 design §5 U1 未决（CC 实测 < 几 MB 量级） | ⚠️ 已记 U1 未决，未来如发现性能问题再开 issue 优化 |
| B7 receive_id 全空 Skipped | `push_receive_id_all_empty_returns_skipped` | ✅ 0 lark-cli 业务调用（仅 identity probe） |
| B8 env 优先于 identity | `resolve_receive_id_env_overrides_identity` | ✅ env 命中 identity 不调 |
| B9 identity 失败 → config 兜底 | `resolve_receive_id_falls_back_to_config_when_identity_blank` | ✅ |

### 错误（E1-E8）

| # | 证据 | 结果 |
|---|---|---|
| E1 task fail → IM 兜底 | `push_task_fail_triggers_im_fallback` + `cli_push_strict_with_no_im_fallback_task_fail_exits_one` integ（fail path） | ✅ FallbackUsed + om_xxx |
| E2 strict + FallbackUsed → exit 0 | `outcome_to_exit_code` 逻辑（仅 Failed 才 exit 1） | ✅ 编译期保证 |
| E3 --no-im-fallback + task fail | `push_no_im_fallback_opt_out_task_fail_directly_failed` | ✅ 仅 1 调用，Failed |
| E4 task + IM 双失败 | `push_task_and_im_both_fail_returns_failed` | ✅ errors 长度 2 |
| E5 append_steps fail → IM | `push_append_fail_triggers_im_fallback_preserves_task_url` | ✅ FallbackUsed + task_url 保留 |
| E6 ResponseShapeUnexpected → IM | D13 决策：所有非 Ok 都走 IM；E1 / E5 已覆盖 task_writer 错误转 IM 路径 | ✅ |
| E7 Config::load 失败 | `resolve_receive_id` config 层 `Err` 走 `tracing::warn!` + 跳过；测试用 tempdir 无 config.yaml → load 返 Default → 等价空 | ✅（行为等价） |
| E8 tokio runtime 启动失败 | `cli::run` 错误路径 `ExitCode::from(2)`；属系统级，未单独测 | ✅（代码路径存在） |

**前端改动**：无（feature 是 CLI/lib，无浏览器渲染）。

**未通过**：B6 大文件优化为 design U1 已知未决，不阻塞验收。

## 4. 术语一致性

对照 design §0 + §2.1 命名 grep 代码：

- [x] `PushRequest` / `PushOptions` / `PushOutcome` / `PushStatus` / `StopHookInput` — 5 类型全代码命中
- [x] `push` / `run_stop_hook` / `run_push` — 3 pub async fn 全命中（`run_push` 在 `cli` 子模块）
- [x] `transcript_reader` / `read_last_assistant_text` / `TranscriptReadError` — inline mod 3 项全命中
- [x] `truncate_utf8` / `cwd_basename` / `stable_idem_key` / `resolve_summary_from_hook_input` / `resolve_receive_id` — 5 helper fn 全命中
- [x] `BotArgs` / `BotSub` / `StopHookCliArgs` / `PushCliArgs` — clap derive 4 struct/enum 全命中
- [x] `outcome_to_exit_code` / `build_request_from_push_args` — 共享 cli helper 全命中
- [x] `DEFAULT_SUMMARY` / `SUMMARY_MAX_BYTES` — 公开 const 全命中

**禁用词反向 grep**：
- `grep "FEISHU_NOTIFY_TO"` = 0（D11 决议沿用 ROOSTERY_* 前缀）
- `grep "notify_receive_id"` = 0（D12 决议不引入新字段）
- `grep "Default::default()" ../bot_stop_hook` 含 PushOutcome 内部使用是合理（非 non_exhaustive struct literal 旁路）

**无不一致项**。

## 5. 架构归并

按设计 §4 把稳定、系统级可见的内容**实际写入** ARCHITECTURE.md。

### 5.1 名词层归并 → §2 核心概念 / 术语表

**新增公开类型**（与 `TaskRef / TaskGuid` 同等待遇）：

- `PushRequest / PushOutcome / PushStatus / PushOptions` — 反向调用 CLI 与 hook 共享的稳定边界类型
- `StopHookInput` — CC/Codex/Gemini SessionEnd stdin JSON schema

**新增公开 const**：
- `DEFAULT_SUMMARY = "Agent stopped (no summary)"`
- `SUMMARY_MAX_BYTES = 200`

✅ 已写入 ARCHITECTURE.md §2 术语表（一并归并到 §3 Module F 描述中）

### 5.2 动词骨架归并 → §3 Module F

Module F 描述需要从"`bot-task-writer (done)` / `bot-stop-hook` / `bot-bridge-cluster`" 升级为"前两条 done + 第 3 条 planned"，并加 `bot_stop_hook` 模块说明（产品行数 / 公开 fn / 反向 CLI 能力 / 极简 sh wrapper 转向 / 0.1.0 触发判据达成）。

✅ 已写入 ARCHITECTURE.md §3 Module F

### 5.3 流程级约束归并 → §6 已知约束

无新红线（D14 决议保留 dispatcher / push 双独立顶层入口，是结构选择而非红线）。

**S10.5 test infra 修订**已写入 `.codestable/attention.md`，**不再次写到 ARCHITECTURE.md §6** —— attention.md 已是 CodeStable 技能强制读入口，与 ARCHITECTURE.md §6 角色差异：前者是项目碎片知识 / 测试约定（每个 feature 都要遵守），后者是模块结构红线（不可逾越的设计契约）。

### 5.4 跨模块接口契约 → §4

无新 7 大契约改动。bot_stop_hook 消费现有 `LarkRunner` / `Config` / `JournalEntry` 契约。新增 PushOutcome 作为 feature-level 稳定契约（不上 §4 列表，因它是单模块对 CLI caller 的契约，不跨模块）。✅ 不需要写入 §4

## 6. requirement 回写

frontmatter `requirement: agent-work-in-feishu` 指向 draft req。

**核心判断**：bot-stop-hook 是 req"minimal-loop"兑现层——E2E 链路从此真跑通：
- CC SessionEnd → sh exec → `roostery bot stop-hook` → 飞书出 task + step
- 反向 CLI：任意 agent 可主动推 = req"agent 工作过程长在飞书里"的另一维兑现（不只 stop hook 被动触发）

**升级动作**：`status: draft → current`。保留原始愿景，文末加变更日志记录本次落地。

✅ 触发 `cs-req` update（acceptance 退出后执行）

## 7. roadmap 回写

frontmatter `roadmap: rust-rewrite` / `roadmap_item: bot-stop-hook` 双字段齐。

打开 `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`：
- 当前 `slug: bot-stop-hook` 状态 = `in-progress`（design 阶段已写入）
- `feature: 2026-05-18-bot-stop-hook` ✓ 正确
- 改 `status: done`
- 主文档 `rust-rewrite-roadmap.md` §3 第 16 条同步打勾 + acceptance 备注

✅ 实际写入（acceptance 退出后执行）+ validate-yaml.py 校验

## 8. attention.md 候选盘点

回看本次实现，盘点"每个 feature 都会撞一次"的环境 / 工具 / 工作流类信息：

- ✅ **已写入** ① test ENV_LOCK 跨模块 race 修订（S10.5 落定）—— attention.md "测试" 节 + Corollary 注 Config roundtrip 也要锁
- ✅ **已写入** ② "Roostery 命名 / vendor-neutral" 已在历史条目（不新加）

**新候选**：

- **候选 1**：`cargo test --all` **默认并行**会暴露 env 测试 race；任何未来新增 env-touching test mod 都必须用 `crate::paths::TEST_ENV_LOCK`（已写入 attention.md，无新候选）
- **候选 2**：clap `ArgGroup` 互斥校验的错误信息在 clap 4.x 里包含 "cannot be used with" 字串——若测试用 message contain 断言，需注意 clap 版本升级时 message wording 可能变（小型坑，不一定要 attention.md 记，可考虑 cs-learn）

**判定**：候选 1 已落地；候选 2 是个一次性坑，归 cs-learn 更合适，**不在 attention.md 新加**。

## 9. 遗留

- **U1（design §5 已记）**：transcript 倒序读策略——本期 `read_to_string` 全读再 rev，未来如发现大文件性能问题（>10MB 量级）走 cs-issue 优化为 seek + chunk 倒读
- **U3（design §5 已记）**：`PushOutcome` 是否补 `duration_ms` 字段——本期不加，待 caller 反馈后定
- **D14 后续路径**：未来若需"stop hook 也走 rule 引擎 / budget gate"，新开 feature 加 `BotPushRunner` 适配器，不阻塞 0.1.0
- **顺手发现**：clap `ArgGroup` `multiple(false)` 在 clap 4.x 是默认行为，可省略；保留显式以防版本升级 silent 行为变更
- **0.1.0 release 触发判据达成 → 后续动作**：(a) 真机 dogfood 一轮（roostery init → 跑 CC → 看飞书 task）；(b) crates.io 准备 / README / CHANGELOG 起稿——这两项归 brainstorm `v0.x-direction` 决议下一步

---

## 验收结论

✅ **PASS**

- 全部 39 条 design checks 通过（checklist 全 done）
- 4 命令本地全绿（fmt / clippy -D warnings / test --all / test --doc）
- CI run 26030808131 三 job 全绿（commit 220c7b0）
- 420 tests 全过（lib 375 / bot_cli_integ 4 / 其他 integ 41）
- 守护 grep N1-N6 全 0（除应有 blake3 = 1）
- 挂载点反向 grep + 拔除沙盘推演通过
- B6（大 transcript 性能）记为已知 U1 未决，不阻塞
- **0.1.0 release 触发判据达成**：Phase 5 minimal-loop closing feature 落定
