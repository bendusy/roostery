---
doc_type: feature-acceptance
feature: 2026-05-18-bot-task-writer
status: accepted
summary: Phase 5 Module F 第 1 子 feature 验收闭环——3 pub async fn 纯库 API + session cache + host suffix 全 9 节核对通过；架构 §2/§3/§6 已实际归并（含 §6 第 18 条 --yes 红线破例显式归档）；req agent-work-in-feishu 加变更日志（保持 draft 等 bot-stop-hook 升级 current）；roadmap items.yaml + 主文档 §5 第 15 项同步 done。顺手 fix onboarding shell_kind_detect_* race
related_design: .codestable/features/2026-05-18-bot-task-writer/bot-task-writer-design.md
tags: [phase-5, module-f, task-writer, acceptance, milestone]
---

# bot-task-writer 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-18
> 关联方案 doc：`.codestable/features/2026-05-18-bot-task-writer/bot-task-writer-design.md`
> 里程碑：**Phase 5 第一砖**——首次让 Rust 业务模块真消费 LarkRunner trait 做生产飞书 IO；下一 feature `bot-stop-hook`（minimal-loop=true）调本模块完成 0.1.0 E2E 闭环

## 1. 接口契约核对

逐条对照 design §2.1.1 草图与 `crates/roostery/src/bot_task_writer.rs` 实际实装：

- [x] `pub const SESSION_CACHE_SCHEMA_VERSION: u32 = 1` / `DEFAULT_HOST_FALLBACK: &str = "unknown"` (`bot_task_writer.rs:22-23`) ✓
- [x] `pub struct TaskRef { guid: TaskGuid, url: String }` 全 pub (`bot_task_writer.rs:27-31`) ✓
- [x] `pub struct TaskGuid(String) #[serde(transparent)]` + `as_str / from_existing / Display` (`bot_task_writer.rs:36-54`) ✓
- [x] `pub enum TaskWriterError #[non_exhaustive]` 5 变体（LarkCallFailed / ResponseShapeUnexpected / CacheLoadFailed / CacheSaveFailed / IdentityResolveFailed）(`bot_task_writer.rs:56-83`) ✓
- [x] `pub struct CreateTaskOptions<'a> #[non_exhaustive] + Default` 5 字段 + 5 `with_*` builder (`bot_task_writer.rs:85-125`) ✓
- [x] `pub struct AppendStepsOptions<'a> #[non_exhaustive] + Default` 2 字段 + 2 `with_*` builder (`bot_task_writer.rs:127-146`) ✓
- [x] 3 pub async fn 签名与 design §2.1.1 一致 ✓
- [x] lib.rs 加 `pub mod bot_task_writer;` ✓

**与 design 唯一增强**：因 `#[non_exhaustive]` 跨 crate 不允许 `..Default::default()` literal（attention.md E0639 注），加了 builder API `new() + with_*`——与 `RunOptions::new() / with_*` 同模式，方向与 design §1.2 D5 一致。这是合理增强不是偏离（impl 汇报已说明）。

**流程图核对**（design §2.2.1 mermaid）：
- [x] cache hit → 直接返；cache miss → assignee 解析 → host suffix → argv → runner.run → parse JSON → save_cache 全链路在 `get_or_create_for_session + create_task + save_cache` 三 fn 内有实际落点 ✓

无未处理偏差。

## 2. 行为与决策核对

**需求摘要 F1-F12 逐项验证**：

- [x] F1 create_task 调 `lark-cli task +create --as bot --summary --description --assignee --idempotency-key` → 单测 `create_task_happy_with_explicit_assignee` 验证 argv
- [x] F2 host suffix 幂等自动加 → 单测 `apply_host_suffix_appends_marker / apply_host_suffix_is_idempotent`
- [x] F3 assignee 默认走 identity::current → 单测 `create_task_falls_back_to_identity_for_assignee`（验 3 lark 调用：auth/profile/task）
- [x] F4 append_steps argv + 空 steps 短路 → `append_steps_empty_does_not_call_lark`（mock 0 调） + `append_steps_includes_yes_flag_and_data`
- [x] F5 `--yes` 始终带 → 实装内嵌 + 单测验证 + 模块顶部 doc 显式标
- [x] F6-F8 get_or_create + session cache schema v1 + safe_filename → `get_or_create_first_call_creates_and_caches` + `safe_filename_neutralizes_path_traversal` + integ `cache_file_lives_under_state_session_tasks`
- [x] F9 TaskGuid newtype → `task_guid_serde_transparent`
- [x] F10 TaskWriterError 5 变体 → `task_writer_error_display_includes_context` + 实装类型定义
- [x] F11 三 pub fn take `&dyn LarkRunner` → 编译期保证（函数签名）
- [x] F12 tracing 私有 instrument → 部分失败 caller 自决（design §2.2.3 不变量 8）；本期不强制 tracing::warn 入口（保持纯库 API 简洁，caller 拿到 Err 自行 log）

**关键决策 D1-D14 落地**：全部实装一致（D1 纯库 API ✓ / D2 host suffix Python parity ✓ / D3 partial fail 设计 ✓ / D4 `--yes` 破例 + 显式归档到 ARCHITECTURE §6 第 18 条 ✓ / D5 三 fn `&dyn` 注入 ✓ / D6 TaskGuid serde transparent ✓ / D7 5 错误变体 ✓ / D8 路径不沿用 `~/.feishu_hub` ✓ / D9 schema_version=1 ✓ / D10 safe_filename ✓ / D11 atomic 写 ✓ / D12 默认 idempotency_key `{agent}-session-{session}` ✓ / D13 host suffix 幂等 ✓ / D14 顶层文件不入 src/bot/ 子目录 ✓）

**明确不做 N1-N12 守护 grep** 全部 0 命中（见 impl 阶段 + accept 复查；N3/N4 因 doc 注释 disclaimer 也无命中，比 dispatcher-loop / dispatcher-runners 更严格）

**流程级约束 不变量 1-10 核对**：全部实装一致，关键点 ✓
- 不变量 1 (`&dyn LarkRunner` 注入) ✓
- 不变量 2 (不绕过 LarkRunner trait) ✓ grep N3/N4 守
- 不变量 6 (`--yes` 架构红线破例) ✓ 模块顶部 doc + ARCHITECTURE §6 第 18 条
- 不变量 10 (assignee 失败返 Err 不 silent) ✓ 单测 `create_task_identity_error_surfaces`

**挂载点反向核对**（design §2.3 3 项）：

- [x] M1 `pub mod bot_task_writer;` in lib.rs → `lib.rs:5` ✓
- [x] M2 三 pub fn → `bot_task_writer.rs` ✓
- [x] M3 `~/.roostery/state/session_tasks/` 使用 → `session_cache_dir()` + integ `cache_file_lives_under_state_session_tasks` ✓

**反向核查**（grep `bot_task_writer::` 全 repo 排除自身）：
- 命中：`lib.rs:5` 模块导出 + `tests/bot_task_writer_integration.rs` 集成测试 use
- 0 清单外引用 ✓

**拔除沙盘推演**：删 `lib.rs:5 pub mod` + `src/bot_task_writer.rs` + `tests/bot_task_writer_integration.rs` → cargo build 通过；其他模块零反向依赖（task_writer 是 leaf）；用户目录 `~/.roostery/state/session_tasks/` 无 GC 但用户可手清。**可完整卸载**。

## 3. 验收场景核对

对照 design §3 验收契约（C1-C9 ~30 场景）：

**§3.1 类型 C1.1-C1.5** ✓ 3 lib + 编译期保证
**§3.2 host suffix C2.1-C2.4** ✓ 5 lib（host_default 3 fallback + apply_host_suffix 2）
**§3.3 safe_filename C3.1-C3.3** ✓ 3 lib（normal / special / path-traversal）
**§3.4 session cache C4.1-C4.4** ✓ 5 lib（empty / round-trip / no .tmp artifact / schema_version 缺失兼容 + corrupt 返 None）
**§3.5 create_task C5.1-C5.5** ✓ 5 lib（happy / lark err / shape err / explicit assignee / identity err）
**§3.6 append_steps C6.1-C6.3** ✓ 3 lib（empty / yes+data / lark err）
**§3.7 get_or_create C7.1-C7.3** ✓ 3 lib（first call create+save / second hit cache / corrupt fallthrough）
**§3.8 守护 grep** ✓ N1-N12 全 0 命中
**§3.9 模块级 C9.1-C9.5** ✓ 340 lib + 3 bot_task_writer integ + 全部其他 integ + 4 doctest 全过；fmt / clippy -D warnings / test --all / test --doc 四命令全绿；CI run #26021247942 三 job 全绿

**前端**：无 UI 改动。

## 4. 术语一致性

对照 design §0 + §2.1 命名 grep：

- `TaskRef / TaskGuid / TaskWriterError / CreateTaskOptions / AppendStepsOptions / create_task / append_steps / get_or_create_for_session / SESSION_CACHE_SCHEMA_VERSION / DEFAULT_HOST_FALLBACK` → 单 source of truth (`bot_task_writer.rs`)；main.rs 未引用；集成测试 use 一致
- 私有 helper（`host_default / apply_host_suffix / safe_filename / session_cache_dir / load_cache / save_cache / parse_task_response / truncate_for_error / SessionCacheEntry / hostname_first_segment / first_segment`）命名风格与 design §2.4 实装结构一致
- 禁用词反向核查：N1-N12 守护 grep 全 0 命中

无不一致。

## 5. 架构归并

已实际写入：

- [x] **`ARCHITECTURE.md §2 术语表`**：加 4 词条（TaskRef/TaskGuid/TaskWriterError / CreateTaskOptions+AppendStepsOptions / SESSION_CACHE_SCHEMA_VERSION / DEFAULT_HOST_FALLBACK）
- [x] **`ARCHITECTURE.md §3 Module F`**：子 feature `bot-task-writer` 标 done + 加 "bot_task_writer 模块" 段落（首条 buffered Value 业务路径声明 + 公开 API + 关键行为 + caller 编排预期指向 bot-stop-hook）
- [x] **`ARCHITECTURE.md §6 已知约束`**：加第 18 条——`bot_task_writer::append_steps --yes` 是 lark-shared 红线显式破例；含完整理由链（high-risk-write / bot 写自己 task / Python 验证 / Rust 模块顶部双签）+ 扩展规则（未来加新破例先 update 本节 + 模块顶部 doc 双签）

**判据自检**：没读过本 feature design 的人打开 ARCHITECTURE.md 现在能看到——
(a) Module F 第 1 砖完成 + bot_task_writer 角色 + caller 编排预期
(b) §2 术语表能查到 TaskRef / TaskGuid / TaskWriterError / Options / 常量形状
(c) §6 第 18 条知道 `append_steps --yes` 是 sanctioned 破例（不是 bug）

✓ 归并完成。

## 6. requirement 回写

- 方案 frontmatter `requirement: agent-work-in-feishu`
- 该 req `status: draft`，本 feature 是 req 核心兑现层第一砖
- 已 update `.codestable/requirements/agent-work-in-feishu.md`：
  - `implemented_by` 列表追加 `2026-05-18-bot-task-writer`
  - 变更日志加 2026-05-18 第 1 条（bot-task-writer 落地）显式标 **Phase 5 Module F 第 1 子 feature**
  - **req 仍保持 `draft`**——升级 `current` 等 bot-stop-hook（minimal-loop=true）跑通端到端"用户在飞书 app 真看到 agent run 的 task 卡 + step stream"再升

## 7. roadmap 回写

- 方案 frontmatter `roadmap: rust-rewrite` / `roadmap_item: bot-task-writer`
- 已 update `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`：
  - `bot-task-writer.status: in-progress → done`
  - notes 重写：5 个 user 拍板决策摘要 + 0 新增 dep + 顺手 fix onboarding race + Accepted commit + CI run #
- 已 update `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md §5 第 15 项`：
  - 标题加 "— **done**（feature `2026-05-18-bot-task-writer`）"
  - 状态：planned → **done**
  - 备注重写：3 pub fn API / session_cache / `--yes` 破例归档 / 首条业务消费 LarkRunner / 0 新依赖 / 顺手 fix / commit + CI run
- yaml 校验通过（`yaml.safe_load` OK）

## 8. attention.md 候选盘点

回看本次实现暴露的项目通用约束 / 工具陷阱：

- **候选 1：CI rustfmt 与本地 rustfmt 偶有格式偏差**——实现中 `cargo fmt --check` 本地全过但 CI 报 fmt diff（具体在 `parse_task_response` 中 `.ok_or(struct literal)` 的多行布局）。本次解决方法是改成 CI 偏好的格式（`.ok_or(...)` 多行而非 method-chain 风格）。**判据评估**：未来 feature 的 AI 会不会再撞？可能——多行 method chain + struct literal 是常见模式。但**反方**：可能只是本仓库本次 CI cache 异常，不一定是稳定模式；且这条很难一行讲清（需要复现条件描述）。建议归 cs-learn 而非 attention.md
- **候选 2：跨模块测试 env 并发互相干扰**——本次发现 onboarding `shell_kind_detect_*` 4 测试无 ENV_LOCK 串行化，bot_task_writer 测试增加并发压力暴露 deterministic CI fail。attention.md 已有 "测试中并发触碰 env 必须用 static Mutex 串行化"规约——本次是该规约的又一次落点（onboarding 此前漏加，已顺手 fix）。**不需新加 attention 条目**，规约已覆盖；本次顺手 fix 是规约的正常落点
- **候选 3**：无其他需要补入 attention.md 的内容

留给用户决定候选 1 是否走 `cs-learn` 归档（不放 attention.md）。

## 9. 遗留

- **`bot_task_writer` 暂不实装 BotTaskWriterRunner**（design D1 + N1 守）：未来真有"用户 rules.yaml 路由到 bot_task_writer kind"需求时新开 feature 加 wrap
- **Module F 子目录化推到 Phase 5 收尾**（design §2.5 + ARCHITECTURE §3 Module F 段落）：3 个 Module F sub-feature（task_writer / stop-hook / bridge-cluster）全部进来后走 cs-refactor 一次性聚到 `src/bot/` 子目录（与 Module E module-e-subdir 同 convention）
- **Phase 5 路径继续铺平**：bot-task-writer 完成 = 下一个 feature `bot-stop-hook` 入口完整。bot-stop-hook 是 minimal_loop=true，跑通 = 0.1.0 release 触发判据 = req `agent-work-in-feishu` 升 current 时机
- **CI rustfmt 与本地偏差候选**：未决，留 cs-learn 候选
- **实现阶段顺手发现 / 顺手 fix**：onboarding `shell_kind_detect_*` 4 测试加 ENV_LOCK 串行化（attention.md 规约的又一次落点，5-10 行改动，与本 feature 主题独立但被本 feature 增加并发压力触发暴露——属"我的改动让 latent race 变 deterministic CI fail"范畴，必须 fix；不算方案外越界）
