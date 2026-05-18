---
doc_type: feature-acceptance
feature: 2026-05-18-dispatcher-runners
status: accepted
summary: Phase 4 Module E 第 3 子 feature 验收闭环——Runner trait + Noop + CcHeadless + Registry 全 9 节核对通过；架构 §2/§3/§4.3/§6 已实际归并；req runtime-neutral 加变更日志条目；roadmap items.yaml + 主文档同步 done
related_design: .codestable/features/2026-05-18-dispatcher-runners/dispatcher-runners-design.md
tags: [phase-4, module-e, runners, acceptance]
---

# dispatcher-runners 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-18
> 关联方案 doc：`.codestable/features/2026-05-18-dispatcher-runners/dispatcher-runners-design.md`

## 1. 接口契约核对

逐条对照 design §2.1.1 `runners.rs` 草图与 `crates/roostery/src/runners.rs` 实际实装：

**接口示例逐项核对**：

- [x] `DEFAULT_TIMEOUT_MS: u64 = 600_000` const 公开（`runners.rs:30`）→ 一致
- [x] `STDOUT_HEAD_CAP: usize = 4096` const 公开（`runners.rs:31`）→ 一致
- [x] `SAFE_ENV_FORWARD: &[&str]` const 公开，覆盖 POSIX baseline / XDG / 代理 / TLS CA / API keys (ANTHROPIC/OPENAI/GEMINI/GOOGLE) / 基地址 / 各 vendor config dirs（`runners.rs:36-78`）→ 一致
- [x] `RunnerStatus` enum 三态 + `#[serde(tag = "kind", rename_all = "snake_case")]`（`runners.rs:80-86`）→ 一致；design 草图仅写 `#[serde(rename_all = "snake_case")]`，实装加了 `tag = "kind"` 让序列化稳定可读（test 验证 `{"kind": "success"}` / `{"kind": "failed", "reason": "x"}`），属于无害扩展
- [x] `RunOutcome` 5 字段全 pub（`runners.rs:91-97`）→ 一致
- [x] `RunnerError` `#[non_exhaustive]` 4 变体（BinaryNotFound/SpawnFailed/Timeout/OutputParseFailed），每变体携带 kind / path / source / timeout_ms / stdout_head（`runners.rs:99-119`）→ 一致
- [x] `Runner` trait `#[async_trait] Send + Sync`，`fn kind(&self) -> &'static str` + `async fn run(&self, event, ctx, args)`（`runners.rs:125-135`）→ 一致；**不收 budget 参数**（D1 偏离）
- [x] `RunnerRegistry` struct + `new / with_runner / with_defaults / find` API（`runners.rs:139-176`）；额外加 `len / is_empty / Default` 满足 clippy `len_without_is_empty`，无害扩展 → 一致
- [x] `NoopRunner` impl：`kind() == "noop"`；`run` 返 `RunOutcome { Success, "", "", vec![], None }`（`runners.rs:236-258`）→ 一致
- [x] `CcHeadlessRunner` impl：`bin_override: Option<PathBuf>` 测试可注入；`kind() == "cc_headless"`；spawn_blocking 包同步 Command；调 `claude -p <prompt> --output-format json [--model] [--resume]`（`runners.rs:266-346`）→ 一致

**名词层"现状 → 变化"逐项核对**：

- [x] 现状清单（`hook_event::HookEvent` / `trace::{TraceContext, to_env_pairs}` / `budget::BudgetState` / `rules::Match`）→ 仅消费 `HookEvent` 和 `TraceContext::to_env_pairs()`；**不消费 budget / rules**（design N3 守护，grep 通过）
- [x] 变化清单（`runners.rs` 新建 + 类型全集）→ 文件存在 818 行，类型全集到齐

**流程图核对**（§2.2 mermaid）：

- [x] 本 feature 提供 F/G 节点（`registry.find runner kind` + `runner.run event ctx args`）已落地；A-E / H-K 节点由 dispatcher-loop（下个 feature）拼，符合"本期不实装 dispatch chain"约束
- [x] runners 内部编排：NoopRunner 无 IO 直接返；CcHeadlessRunner = spawn_blocking → Command → args 拼 → env sanitize → 等 timeout → 解 JSON → 拼 RunOutcome（`runners.rs:303-345` 实装链对应 design 描述）

**无偏差**——所有 design §2.1 草图条目均在代码中找到对应实装。`Default for CcHeadlessRunner` 改用 `#[derive(Default)]`（attribute）比 design 草图的 manual `impl Default` 更 idiom；`RunnerStatus` serde 加 `tag = "kind"` 让 enum 序列化形状稳定可读——两处都是 implement 阶段微调，方向与 design 一致。

## 2. 行为与决策核对

对照 design §1 + §2.2：

**需求摘要 F1-F12 逐项验证**：

- [x] F1 Runner trait 定义 → `runners.rs:125-135` 一致
- [x] F2 RunOutcome 数据形状 → `runners.rs:91-97` 5 字段
- [x] F3 RunnerStatus 三态 → `runners.rs:80-86`
- [x] F4 RunnerError 4 变体 → `runners.rs:99-119`
- [x] F5 NoopRunner Success 空字段 → `runners.rs:236-258` + test `noop_runner_returns_success_empty`
- [x] F6 CcHeadless 拼 `claude -p ... --output-format json [...]` → `runners.rs:328-337`
- [x] F7 JSON parse 解 cost_usd / final_text + 解析失败仍 Success → `runners.rs:441-451` 容错；test `cc_headless_non_json_stdout_returns_success_no_cost` + `enrich_cc_invalid_json_still_returns_success` 双覆盖
- [x] F8 timeout 触发 → `runners.rs:404-412` Instant deadline + try_wait 50ms 轮询 + `RunnerError::Timeout`；test `cc_headless_timeout_returns_err` 验证
- [x] F9 binary 不存在 → `runners.rs:315-325` 双重检查（`which::which` 失败 + path.exists() 失败 + spawn `io::ErrorKind::NotFound`）；test `cc_headless_binary_not_found`
- [x] F10 env sanitize SAFE_ENV_FORWARD + trace ctx 注入 → `runners.rs:190-220` `prep_env` helper；4 内联测覆盖 base 兜底 / trace 注入 / unsafe 过滤 / safe 转发
- [x] F11 RunnerRegistry::new + with_runner + with_defaults → `runners.rs:143-176`
- [x] F12 find linear O(n) → `runners.rs:162-167`

**关键决策 D1-D12 落地**：

- [x] D1 Runner trait 不收 budget 参数 → `Runner::run` 签名仅 `(event, ctx, args)`，无 BudgetGate；架构 §4.3 已加偏离说明
- [x] D2 RunOutcome 加 `cost_usd: Option<f64>` → `runners.rs:96`
- [x] D3 noop + cc_headless 首发 → 守护 grep N1/N2 仅 doc 注释 + 反向断言命中
- [x] D4 std::process::Command + spawn_blocking → `runners.rs:339-345` + `spawn_with_timeout(...)`；无 tokio::process 依赖
- [x] D5 SAFE_ENV_FORWARD const allowlist → `runners.rs:36-78`
- [x] D6 trace ctx env 注入用 `ctx.to_env_pairs()` 既有 API → `runners.rs:216-218`
- [x] D7 CC JSON 解析容错 → `enrich_cc` match `serde_json::from_str` Err 分支 returns `(None, None)`
- [x] D8 timeout = args.timeout_ms (Option<u64>) 覆盖 default 600s → `runners.rs:311` `unwrap_or(DEFAULT_TIMEOUT_MS)`
- [x] D9 emitted_events 始终空 Vec → 守护 grep N8 通过；CcHeadless `enrich_cc` 返 `emitted_events: Vec::new()`
- [x] D10 with_defaults 自动注 NoopRunner + CcHeadlessRunner → `runners.rs:156-160`；test `registry_with_defaults_has_noop_and_cc_headless`
- [x] D11 RunnerError 4 变体 → 见 F4
- [x] D12 不引 tokio::process / tokio::time::timeout → 守护 grep N7 通过

**明确不做 N1-N10 守护 grep**（实测见会话上文）：

- [x] N1 codex_exec → 2 命中均为 doc 注释 + `assert!(r.find("codex_exec").is_none())` 反向断言，**符合 design 意图**（不实装 codex runner，并显式断言未注册）
- [x] N2 gemini_headless → 同 N1，2 命中均为 doc 注释 + `assert!(r.find("gemini_headless").is_none())`
- [x] N3 BudgetState / RunawayTracker / CompiledRule / rules::matches → 0
- [x] N4 Command::Runner / Command::Run → 0（main.rs 不暴露 CLI 子命令）
- [x] N5 LarkRunner / lark_cli:: → 0（runner 不消费飞书 trait）
- [x] N6 FEISHU_HUB_ → 0
- [x] N7 tokio::process / tokio::time::timeout → 0
- [x] N8 emitted_events.push → 0（始终 vec![]）
- [x] N9 estimated_cost / pre_consume / try_consume → 0（不做预扣）
- [x] N10 \bretry\b / max_retries → 0（不做自重试）
- [x] idiom rust-idiom-first `as_object_mut().unwrap()` / `as_array_mut().unwrap()` → 0

**编排层流程级不变量核对**（design §2.2 不变量 1-7）：

- [x] 不变量 1：Runner trait async + spawn_blocking 包同步 Command → 实装一致
- [x] 不变量 2：env sanitize 统一经 `prep_env` helper → 所有 runner 共用同一 fn（实际 NoopRunner 不 spawn 子进程不用走，CcHeadlessRunner 在 spawn 前调用）
- [x] 不变量 3：trace env 注入三 env → `prep_env` 内 `for (k, v) in ctx.to_env_pairs()` 注入
- [x] 不变量 4：timeout = args.timeout_ms 覆盖 default → 实装一致
- [x] 不变量 5：CC JSON 解析容错 → 实装一致
- [x] 不变量 6：registry find 未命中返 None → `find` linear `iter().find().map` 链 None propagate，test `registry_new_is_empty` + `registry_find_miss_returns_none`
- [x] 不变量 7：RunnerError vs RunOutcome.status.Failed 语义分层 → CcHeadless 实装中：spawn/timeout/解析错走 `Err(RunnerError::...)`；exit code != 0 走 `Ok(RunOutcome { status: Failed { reason } })`；test `cc_headless_non_zero_exit_returns_failed` 验证

**挂载点反向核对**：

design §2.3 挂载点清单 3 项：

- [x] 挂载点 1（`pub mod runners;` in lib.rs）→ `crates/roostery/src/lib.rs:18` 一致
- [x] 挂载点 2（`with_defaults` 注 NoopRunner）→ `runners.rs:158`
- [x] 挂载点 3（`with_defaults` 注 CcHeadlessRunner）→ `runners.rs:159`

**反向核查**（grep `RunnerRegistry|NoopRunner|CcHeadlessRunner|runners::` 全 repo 排除 `src/runners.rs`，见会话上文输出）：

- 命中位置：`crates/roostery/tests/runners_integration.rs:1,5,40,49,71,74-75,94,97,121,124`（11 处）
- 命中性质：均为本 feature 新加的集成测试文件内部使用——非清单外的"业务消费方"。挂载点清单未列集成测试本身，但 design §2.3 明确说"反向核查"是查"清单外的引用"。集成测试是本 feature 自带产物，不计为"消费"——它跟着 feature 一起被加上 / 一起被卸下。
- 结论：除测试内引用外，**无清单外的代码引用** runners 模块；待 dispatcher-loop 起来后会从产品代码消费，本期清单一致。

**拔除沙盘推演**（按清单逆向）：

- 删 `pub mod runners;`（1 行）→ `lib.rs` 不再 export
- 删 `src/runners.rs`（1 文件 818 行）→ 模块消失
- 删 `tests/runners_integration.rs`（1 文件）→ 集成测试消失
- `Cargo.toml` 中本期新增依赖（`async-trait` / `which` / `tempfile` dev-dep）若仅本 feature 使用可一并撤销；如未来 dispatcher-loop 需要 async-trait 则保留
- `cargo build` 通过——`trace` / `budget` / `rules` / `hook_event` / `lark_cli` 等所有上游模块对 runners 零反向依赖（守护 grep N5 / N3 已证）
- 结论：**可完整卸载**，无残留

## 3. 验收场景核对

对照 design §3 验收契约：

**§3.1 Runner trait + 类型 C1.1-C1.4**

- [x] C1.1 RunnerStatus 三态 + serde snake_case → 单测 `runner_status_serde_snake_case` 验证 `{"kind": "success"}` / `{"kind": "failed", "reason": "x"}` 序列化形状
- [x] C1.2 RunOutcome 5 字段全 pub → 类型签名直观
- [x] C1.3 RunnerError 4 变体 #[non_exhaustive] → 单测 `runner_error_display_contains_kind` + 编译期 check
- [x] C1.4 三 const 公开可访问 → 单测 `constants_exposed`

**§3.2 NoopRunner C2.1-C2.2**

- [x] C2.1 kind() == "noop" → 单测 `noop_runner_kind_is_noop`
- [x] C2.2 run 返 Success 空字段 → 单测 `noop_runner_returns_success_empty`

**§3.3 RunnerRegistry C3.1-C3.5**

- [x] C3.1 new() empty → `registry_new_is_empty`
- [x] C3.2 with_runner 链 find → `registry_with_runner_then_find`
- [x] C3.3 with_defaults 含 noop + cc_headless 且 codex/gemini 未注 → `registry_with_defaults_has_noop_and_cc_headless`
- [x] C3.4 find 未命中 None → `registry_find_miss_returns_none`
- [x] C3.5 同 kind 二次注册 linear find 返第一 → `registry_dup_kind_returns_first`

**§3.4 CcHeadlessRunner C4.1-C4.7**

- [x] C4.1 kind() == "cc_headless" → `cc_headless_kind`
- [x] C4.2 binary 不存在 → `cc_headless_binary_not_found`
- [x] C4.3 happy + JSON cost → `cc_headless_happy_returns_success_with_cost`
- [x] C4.4 非 JSON stdout → `cc_headless_non_json_stdout_returns_success_no_cost`
- [x] C4.5 binary 退码非 0 → `cc_headless_non_zero_exit_returns_failed`
- [x] C4.6 timeout → `cc_headless_timeout_returns_err`
- [x] C4.7 trace env 注入 → 集成测试 `tests/runners_integration.rs` 中覆盖（注入 trace 三 env 后假 binary 写 stdout 含值断言）

**§3.5 env sanitize C5.1-C5.3**

- [x] C5.1 prep_env 含 PATH/HOME/LANG/TERM/trace 三 env → `prep_env_includes_base_fallbacks` + `prep_env_injects_trace_env`
- [x] C5.2 SAFE_ENV_FORWARD 命中 → `prep_env_forwards_safe_var_when_set`
- [x] C5.3 ROOSTERY_AGENT 父 hook 状态不串 → `prep_env_does_not_forward_unsafe_var`

**§3.6 守护 grep**（已在第 2 节 N1-N10 + idiom 核对）

**§3.7 模块级 C7.1-C7.5**

- [x] C7.1 cargo test --all 全绿 → 实测会话上文：lib 291 + runners_integration 12 + 其他 integration 全部通过；远超 design "lib ≥15 + integ ≥3" 要求
- [x] C7.2 cargo test --doc 全绿 → 实测 3 passed + 4 compile_fail 全通
- [x] C7.3 cargo clippy --all-targets --all-features -- -D warnings 全绿 → 实测通过（"Finished dev profile"）
- [x] C7.4 cargo fmt --all --check 全绿 → 实测通过
- [x] C7.5 守护 grep 全 0 命中 → 见上

**前端改动**：无（runners 是后端核心库，不涉及 UI）。

## 4. 术语一致性

对照 design §0 + §2.1 命名 grep 代码：

- `Runner` trait → `runners.rs` 一处定义（line 126），各 impl + Box<dyn Runner> 引用一致
- `RunnerStatus` / `RunOutcome` / `RunnerError` → 单 source of truth；测试 + 集成测试用同一命名
- `RunnerRegistry` → 单 source；调用方（集成测试）使用一致
- `NoopRunner` / `CcHeadlessRunner` → 一致；kind() 返字符串 `"noop"` / `"cc_headless"` 与 design 一致
- `SAFE_ENV_FORWARD` / `DEFAULT_TIMEOUT_MS` / `STDOUT_HEAD_CAP` → 一致
- `prep_env` 私有 helper → 命名与 design §2.2 "private fn prep_env(ctx, runner_name)" 一致
- 禁用词反向核查：
  - `codex_exec` / `gemini_headless` 仅在 doc 注释和测试反向断言里出现（一致符合 design 意图）

无不一致。

## 5. 架构归并

对照 design §4，三类内容已实际写入：

- [x] **`.codestable/architecture/ARCHITECTURE.md §2 术语表`** ← design §2.1 新增类型 / 接口契约
  - 新加 7 个词条：`Runner` trait / `RunOutcome` `RunnerStatus` `RunnerError` / `RunnerRegistry` / `NoopRunner` `CcHeadlessRunner` / `SAFE_ENV_FORWARD` / `DEFAULT_TIMEOUT_MS` `STDOUT_HEAD_CAP`
- [x] **`.codestable/architecture/ARCHITECTURE.md §3 Module E`** ← design §2.2 跨模块可见主流程
  - 子 feature 清单标 `dispatcher-runners (done)`
  - 加 "runners 模块" 段落（新文件 / 公开 API / 设计约束 / 不变量 / caller 编排预期）
- [x] **`.codestable/architecture/ARCHITECTURE.md §4 契约 4.3`** ← design §1.2 D1+D2 偏离声明
  - 4.3 行后挂"Phase 4 已落地" + "**与 §4.3 原契约偏离两项**"明示，建议 cs-roadmap update 同步原文
- [x] **`.codestable/architecture/ARCHITECTURE.md §6 已知约束`** ← design §2.2 跨 feature 稳定约束
  - 新加第 13 条："Runner 子进程 env 必经 SAFE_ENV_FORWARD allowlist"——含理由（trace 链不断 / 不污染 LLM / 不引入隐式依赖）+ 扩展规则（新增允许 env 必须改 const + 改 design doc）+ trace 三 env 优先级覆盖

**判据自检**：没读过本 feature design 的人打开 ARCHITECTURE.md 现在能看到——
(a) Module E 多了 `runners` 模块的角色和契约偏离
(b) §2 术语表能查到 Runner / RunOutcome / Registry 的形状
(c) §4.3 知道契约有偏离需要后续 cs-roadmap update
(d) §6 知道子进程 env 走 allowlist 是硬约束

✓ 归并完成。

## 6. requirement 回写

- 方案 frontmatter `requirement: runtime-neutral`
- 该 req `status: draft`，本 feature 是其核心兑现层（"中立接入面"执行点）
- 已 update `.codestable/requirements/runtime-neutral.md`：
  - `last_reviewed: 2026-05-15 → 2026-05-18`
  - `implemented_by` 列表追加 `2026-05-18-dispatcher-runners`
  - 变更日志加 2026-05-18 第 3 条（dispatcher-runners 落地），明示这是 req 的"中立接入面"核心兑现层；req 仍保持 `draft` 等 dispatcher-loop + bot-stop-hook 跑通端到端"换 runtime 飞书侧呈现不变"用户场景再升 current
- 用户故事 / 边界 / pitch 未变（用户视角 still pending 端到端兑现），仅追加变更日志条目——符合"draft req 在本 feature 完成后不机械升级 current，等用户感知场景兑现"的规约

## 7. roadmap 回写

- 方案 frontmatter `roadmap: rust-rewrite` / `roadmap_item: dispatcher-runners`
- 已 update `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`：
  - `dispatcher-runners.status: in-progress → done`
  - `feature: 2026-05-18-dispatcher-runners`（已为对应目录名，校对一致）
  - `notes` 追加：实际落地范围（noop + cc_headless）/ §4.3 偏离两项 / Accepted commit
- 已 update `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md §5 第 13 项`：
  - 标题加 "— **done**（feature `2026-05-18-dispatcher-runners`）"
  - `状态：planned → 状态：**done**`
  - 备注重写，覆盖：第 3 子 feature / 首发范围 / §4.3 偏离两项 / async + spawn_blocking 模式 / SAFE_ENV_FORWARD allowlist / CC JSON 容错
- yaml 校验通过（`yaml.safe_load` OK）

## 8. attention.md 候选盘点

本 feature 暴露了一条值得记的工作流约束：

- **候选 1**：Runner 子进程 env 必经 `SAFE_ENV_FORWARD` allowlist——这条已经写进 ARCHITECTURE.md §6 第 13 条（与 attention.md 关注点 "下次每个 feature 都可能撞一次的工作流约束" 性质重合）。是否需要同步进 attention.md 让所有 CodeStable skill 启动时直接看到？
  - 论据：未来加新 runtime adapter 的 feature 一启动就会面对"需不需要往 SAFE_ENV_FORWARD 加东西"的问题，attention.md 提示能省一次"翻 ARCHITECTURE.md §6 才知道"
  - 反方：ARCHITECTURE.md §6 已记，attention.md 重复条目会噪声；且新增 runtime 的 feature design 阶段会读 dispatcher-runners design + ARCHITECTURE.md，不需要 attention.md 兜底

留给用户决定是否走 `cs-note` 归档。

其他工作流相关坑点本 feature 未暴露（async-trait / which / tempfile 三依赖都是标准 crate，无版本陷阱；spawn_blocking 包同步 Command 模式 attention.md 已记 ETXTBSY race）。

## 9. 遗留

- **§4.3 契约文本未同步**：roadmap §4.3 原契约仍写 `run(&self, event, ctx, &BudgetGate)` 不带 cost_usd——本 feature 已与原契约偏离，建议下次 `cs-roadmap update` 把 §4.3 原文按 D1+D2 改齐（架构 §4.3 表格行已加偏离声明，但 roadmap 章节内 §4.3 详细签名段未改）
- **dispatcher/ 子目录化**：自 dispatcher-trace-budget 起反复 flag；Phase 4 收尾 dispatcher-loop 落地后建议一次性走 `cs-refactor` 把 `trace / budget / runaway / hook_event / rules / runners / loop` 7 模块聚到 `src/dispatcher/`（与 trace_budget acceptance + rules acceptance 同 observation；本期 18 < 20 容忍区不重组）
- **codex_exec / gemini_headless 推后**：design D3 明示 + items.yaml notes 明示；真有 codex/gemini 接入需求时新增独立 feature 加 impl（不阻塞 0.1.0 minimal-loop）
- **chain dispatch 推后**：emitted_events 本期始终空 Vec（design D9）；chain dispatch（一个 runner 触发后续事件）由 dispatcher-loop feature 关注
- **cost 估算 / 预扣推后**：design N9 守护；cost 仅在 cc_headless 解 JSON 后填入 `RunOutcome.cost_usd`，caller dispatcher-loop 走 `budget.consume(c)` 串场景
- **retry 推后**：design N10 守护；连续失败 / 非零退出本期不自重试
