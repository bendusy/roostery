---
doc_type: feature-acceptance
feature: 2026-05-18-dispatcher-rules
status: passed
date: 2026-05-18
summary: Phase 4 Module E 第 2 子 feature 落地——HookEvent §4.4 schema + Rule MVP（3 维 AND match + opaque action args 透传 + first-match-wins + self-event 防自激）。hook_event.rs ~120 LOC + rules.rs ~530 LOC + integ 5 测试。Cargo 加 globset 0.4（ripgrep team）。267 lib + 5 integ + 既有 testsuite 全绿；fmt/clippy/test --all/--doc 四命令 + CI 三 job 远端绿（commit f9b7ce9，CI run 26012250328 success）。守护 grep N1-N10 + idiom 全 0 命中（N2 仅 doc-comment 描述 Runner 下游消费者）
tags: [phase-4, module-e, rules, hook-event, yaml, dispatcher, acceptance]
---

# dispatcher-rules 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-18
> 关联方案 doc：`.codestable/features/2026-05-18-dispatcher-rules/dispatcher-rules-design.md`

## 1. 接口契约核对

### 1.1 名词层逐一核查（design §2.1）

| 接口 | 设计签名 | 代码落点 | 一致 |
|---|---|---|---|
| `HookEvent` `#[non_exhaustive]` 6 字段 | design §2.1.1 | `hook_event.rs:20-27` | ✓ |
| `HOOK_EVENT_SCHEMA_VERSION` pub const = 1 | design §2.1.1 | `hook_event.rs:18` | ✓ |
| `HookEvent::trigger_meta_path(&str) -> Option<&Value>` | design §2.1.1 | `hook_event.rs:33-45` | ✓ |
| `RULES_SCHEMA_VERSION` const + `SELF_EVENT_PREFIXES` const | design §2.1.2 | `rules.rs:31-33` | ✓ |
| `RuleName(String)` `#[serde(transparent)]` newtype | design §2.1.2 | `rules.rs:35-52` | ✓ + 偏差 D1（`from_str` → `new` 避免 clippy `manual_from_str` 与 std trait 同名警告） |
| `RulesConfig / RawRule / RuleWhen / RuleAction / CompiledRule / Match<'a>` | design §2.1.2 | `rules.rs:54-103` | ✓ |
| `RulesError #[non_exhaustive]` 5 变体 | design §2.1.2 | `rules.rs:105-131` | ✓ |
| `load / load_from / matches` 公开 fn | design §2.1.2 | `rules.rs:134, 139, 196` | ✓ |
| `paths::rules_path()` | design §2.1.3 | `paths.rs:51-53` | ✓ |
| `lib.rs` `pub mod hook_event; pub mod rules;` | design §2.1.4 | `lib.rs:6, 14` | ✓ |
| `Cargo.toml` `globset = "0.4"` | design §2.1.5 | `Cargo.toml:39` | ✓ |

### 1.2 调用示例核对

- design §2.1.1 `ev.trigger_meta_path("user.role")` → `hook_event::tests::trigger_meta_path_nested_hit` 单测 + integ `trigger_meta_eq_with_nested_path_real_yaml` ✓
- design §2.1.2 `let rules = rules::load()?;` + `matches(&rules, &ev)` → integ `load_and_match_real_yaml_cc_branch` 全验证 ✓
- design §2.1.2 rules.yaml 示例（含 hook_source / workspace_glob / trigger_meta_eq / runner / args）→ integ test 真磁盘加载 + 字段级断言 ✓

### 1.3 流程图核对（design §2.2）

主流程图（rules 内部 load + matches）：

| 节点 | 代码落点 | ✓ |
|---|---|---|
| A rules.yaml file | path 参数 | ✓ |
| B load_from | `rules.rs:139` | ✓ |
| C serde_yml::from_slice → RulesConfig | `rules.rs:150-154` | ✓ |
| D schema_version 校验 | `rules.rs:155-160` | ✓ |
| E 重名 grep + compile_rule | `rules.rs:162-170` | ✓ |
| F Glob::new → GlobMatcher | `rules.rs:177-183` | ✓ |
| G Vec<CompiledRule> 返回 | `rules.rs:170` | ✓ |
| H HookEvent in | matches 参数 | ✓ |
| I matches | `rules.rs:196` | ✓ |
| J self-event 短路 | `rules.rs:197-199, 212-216` | ✓ |
| L 遍历 rules | `rules.rs:200-208` | ✓ |
| M hook_source 判定 | `rules.rs:219-223` | ✓ |
| N workspace glob.is_match | `rules.rs:224-228` | ✓ |
| O trigger_meta_eq 全 key 判定 | `rules.rs:229-234` | ✓ |
| P Return Some Match | `rules.rs:202-206` | ✓ |
| K Return None | `rules.rs:198, 209` | ✓ |

caller 编排预期图（dispatcher-loop 后续 feature 拼）：本 feature 提供 A→B 节点；C-G（trace / runaway / budget / runner / consume）由 dispatcher-loop 接 ✓

### 1.4 偏差与处理

- **D1** `RuleName::from_str` → `RuleName::new`。Clippy `manual_from_str` 警告："`from_str` can be confused for the standard trait method `std::str::FromStr::from_str`"。**理由**：避免与 `FromStr` trait 撞名造成歧义；`new(impl Into<String>)` 更符合 Rust idiom（同 `String::new` / `PathBuf::new`）。**已回填 design §2.1.2**：见编辑。

无其他接口偏差。

## 2. 行为与决策核对

### 2.1 需求摘要 11 个 F 行为验证（design §1.1）

| # | 行为 | 实测 |
|---|---|---|
| F1 | HookEvent schema 6 字段 | ✓ `hook_event.rs:20-27` + `serde_round_trip_preserves_fields` 单测 |
| F2 | RulesConfig YAML 反序列化 + 校验 | ✓ `load_happy_two_rules` + duplicate / version 错误 单测 |
| F3 | hook_source eq | ✓ `matches_hook_source_eq_hit` + `_miss_skips` |
| F4 | workspace fnmatch | ✓ `matches_workspace_glob_double_star_hit` |
| F5 | trigger_meta 点路径 eq | ✓ `matches_trigger_meta_eq_hit` + `_path_missing_skips_rule` |
| F6 | AND 多维度 | ✓ `matches_and_dimensions_all_pass` + `_partial_fail` |
| F7 | first-match-wins | ✓ `matches_first_match_wins_skips_later_rules` |
| F8 | Self-event 防自激 | ✓ `matches_self_event_dispatcher_short_circuits` + `_roostery_short_circuits` + integ `self_event_does_not_match_any_rule` |
| F9 | Action 透传 | ✓ `Match::args: &'a Value` 借用 + integ 断言 `m.args` 含 yaml 内 `prompt / model` 字段 |
| F10 | 文件不存在 → 空 | ✓ `load_missing_file_returns_empty` + integ `missing_rules_file_returns_empty_set` |
| F11 | YAML 非法 / schema_version != 1 | ✓ `load_invalid_yaml_returns_parse_failed` + `_wrong_schema_version_errors` |

### 2.2 明确不做（design §1.3 N1-N10）反向核查

```bash
$ grep -rE 'render|template_render|\{\{|handlebars|tinytemplate' src/{rules,hook_event}.rs
0 hits
$ grep -rE 'Runner|runner_registry' src/{rules,hook_event}.rs
仅 doc-comment："不解释 args 透传给 Runner impl" / "dispatcher-runners feature 落地后才有真正的 Runner trait 消费"
$ grep -rE 'BudgetState|RunawayTracker' src/{rules,hook_event}.rs
0 hits
$ grep -rE '"continue"|\.cont\b' src/rules.rs
0 hits
$ grep -rE 'switch_by_field|branches' src/rules.rs
0 hits
$ grep -rE 'budget_override|result_writeback' src/rules.rs
0 hits
$ grep 'FEISHU_HUB_' src/{rules,hook_event}.rs
0 hits
$ grep 'Command::Rules' src/main.rs
0 hits
$ grep -E 'LarkRunner|lark_cli::' src/{rules,hook_event}.rs
0 hits
$ grep -E 'disabled|priority' src/rules.rs
0 hits
$ grep -rE 'as_object_mut\(\)\.unwrap\(\)|as_array_mut\(\)\.unwrap\(\)' src/{rules,hook_event}.rs
0 hits
```

N2 命中均为 doc-comment 描述（"Runner trait 消费"是设计意图叙述非代码逻辑）。**意图守住**。

### 2.3 关键决策 D1-D10 落地（design §1.2）

| # | 决策 | 代码体现 |
|---|---|---|
| D1 | Match 3 维 MVP（user 拍板） | `RuleWhen` 仅 `hook_source / workspace_glob / trigger_meta_eq` 3 字段 |
| D2 | Action opaque args 透传（user 拍板） | `RuleAction { runner: String, args: Value }` 无 prompt / model 字段；`Match` 透传 `args: &Value` |
| D3 | 无模板引擎（user 拍板） | 0 模板 lib import；守护 grep N1 通过 |
| D4 | first-match-wins（user 拍板） | `matches` 返 `Option<Match>` + `first_match_wins_skips_later_rules` 单测 |
| D5 | globset crate | `Cargo.toml:39`；`Glob::new(pattern).compile_matcher()` 一次性 |
| D6 | serde_yml（既有） | 不引第二 YAML 库 |
| D7 | rules path 固定 ~/.roostery/rules.yaml | `paths::rules_path()` + 同目录 |
| D8 | SELF_EVENT_PREFIXES 内置 const | `rules.rs:33` + 2 单测 |
| D9 | RulesError 5 变体 #[non_exhaustive] | `rules.rs:105-131` |
| D10 | HookEvent 独立 src/hook_event.rs | 单独文件，不进 rules.rs |

### 2.4 编排层"现状 → 变化"核对（design §2.2）

dispatcher 模块树扩展：trace / budget / runaway 三 gate（上 feature）+ hook_event / rules 两件套（本 feature）。本 feature 不引入跨模块编排——rules 是纯函数库 ✓

### 2.5 流程级约束（design §2.2 不变量 1-7）

| 不变量 | 守护方式 |
|---|---|
| 1 load 缺文件 → Ok(vec![]) | `load_missing_file_returns_empty` + `rules.rs:140-143` 显式 NotFound 分支 |
| 2 compile 一次性 | `compile_rule` 把 `String pattern` → `GlobMatcher` 落进 `CompiledRule.workspace`；`matches` 阶段直接 `is_match` |
| 3 first-match-wins | `matches` 返 `Option<Match>` 不是 `Vec`；测试断言后续规则不评估 |
| 4 self-event 短路 | `matches.rs:197-199` 在遍历前 |
| 5 AND 多维度 | `matches_rule` 三段都 return false 即跳；测试 `_partial_fail` 覆盖 |
| 6 CompiledRule 不可变 | `&'a [CompiledRule]` + `Match<'a>` 借用 |
| 7 RULES_SCHEMA_VERSION=1 公开承诺 | `pub const RULES_SCHEMA_VERSION: u32 = 1;` + load 中显式校验 |

### 2.6 挂载点反向核对（design §2.3）

| # | 挂载点 | 代码实际落点 | 一致 |
|---|---|---|---|
| 1 | `pub mod rules;` in lib.rs | `lib.rs:14` | ✓ |
| 2 | `pub mod hook_event;` in lib.rs | `lib.rs:6` | ✓ |
| 3 | `paths::rules_path()` | `paths.rs:51-53` | ✓ |
| 4 | `Cargo.toml` `globset = "0.4"` | `Cargo.toml:39` | ✓ |

**反向 grep**：

```bash
$ grep -rn 'use roostery::\(rules\|hook_event\)' crates/roostery/src/ crates/roostery/tests/
crates/roostery/tests/rules_integration.rs:3: use roostery::hook_event::HookEvent;
crates/roostery/tests/rules_integration.rs:4: use roostery::rules::{self, RuleName};
```

外部消费者仅 integ test。`rules.rs` 内部 `use crate::hook_event::HookEvent` 是同 crate 引用非挂载点外。后续 Phase 4 dispatcher-runners + dispatcher-loop 起来后才有外部消费。**无清单外挂入点**。

**拔除沙盘推演**：删 4 处挂载点（2 pub mod + 1 paths fn + 1 globset dep）+ 2 模块文件 + integ test → `cargo build` 通过；trace / budget / runaway 不受影响；journal / config 不受影响。**完整可卸载**。

## 3. 验收场景核对（design §3）

### 3.1 HookEvent C1.1-C1.4

| # | 场景 | 证据 |
|---|---|---|
| C1.1 | JSON round-trip + trace default None | ✓ `serde_round_trip_preserves_fields` + `trace_field_defaults_to_none_on_missing` |
| C1.2 | schema_version const = 1 | ✓ `schema_version_const_is_one` |
| C1.3 | trigger_meta_path 命中 | ✓ `trigger_meta_path_single_segment_hit` + `_nested_hit` |
| C1.4 | trigger_meta_path 缺失 | ✓ `_missing_segment_returns_none` + `_through_non_object_returns_none` |

### 3.2 RulesConfig load C2.1-C2.6

| # | 场景 | 证据 |
|---|---|---|
| C2.1 | 文件不存在 → Ok(vec![]) | ✓ `load_missing_file_returns_empty` |
| C2.2 | YAML 非法 → ParseFailed | ✓ `load_invalid_yaml_returns_parse_failed` |
| C2.3 | schema_version=2 → SchemaVersionMismatch | ✓ `load_wrong_schema_version_errors` |
| C2.4 | 重名 → DuplicateRuleName | ✓ `load_duplicate_rule_name_errors` |
| C2.5 | invalid glob → InvalidGlob | ✓ `load_invalid_glob_errors` |
| C2.6 | happy 2 规则 | ✓ `load_happy_two_rules` |

### 3.3 matches C3.1-C3.11

| # | 场景 | 证据 |
|---|---|---|
| C3.1 | 空规则集 | ✓ `matches_empty_rules_returns_none` |
| C3.2 | self-event dispatcher | ✓ `matches_self_event_dispatcher_short_circuits` |
| C3.3 | self-event roostery | ✓ `matches_self_event_roostery_short_circuits` |
| C3.4 | hook_source eq 命中 | ✓ `matches_hook_source_eq_hit` |
| C3.5 | hook_source eq 不命中 | ✓ `matches_hook_source_eq_miss_skips` |
| C3.6 | workspace_glob `**` | ✓ `matches_workspace_glob_double_star_hit` |
| C3.7 | trigger_meta_eq 命中 | ✓ `matches_trigger_meta_eq_hit` |
| C3.8 | trigger_meta 路径不存在 | ✓ `matches_trigger_meta_path_missing_skips_rule` |
| C3.9 | AND 全命中 | ✓ `matches_and_dimensions_all_pass` |
| C3.10 | AND 部分命中 | ✓ `matches_and_dimensions_partial_fail` |
| C3.11 | first-match-wins | ✓ `matches_first_match_wins_skips_later_rules` |

### 3.4 明确不做反向核查 C4.1-C4.10

全 0 命中（见 §2.2）✓

### 3.5 模块级 C5.1-C5.5

| # | 命令 | 结果 |
|---|---|---|
| C5.1 | `cargo test --all` | 267 lib + 5 rules integ + 既有 6+4+12+5+4+4 integ + 3 doc 全绿 |
| C5.2 | `cargo test --doc` | 3 doc-tests 通过 |
| C5.3 | `cargo clippy --all-targets --all-features -- -D warnings` | 全绿（修过 manual_from_str + 2 处 manual_contains） |
| C5.4 | `cargo fmt --all --check` | 全绿 |
| C5.5 | 守护 grep 0 命中 | 通过（见 §2.2） |

**前端改动**：无。

## 4. 术语一致性

| 术语 | 代码命中 | 一致 |
|---|---|---|
| `HookEvent` | `hook_event.rs:20` + 测试 + integ + rules.rs 消费 | ✓ |
| `HOOK_EVENT_SCHEMA_VERSION` | `hook_event.rs:18` + 单测 | ✓ |
| `RulesConfig / RawRule / RuleWhen / RuleAction` | `rules.rs:54-83` + 测试 | ✓ |
| `CompiledRule` | `rules.rs:89` + 测试 / integ | ✓ |
| `Match<'a>` | `rules.rs:99` + 测试 / integ | ✓ |
| `RulesError` 5 变体 | `rules.rs:107-130` + 测试断言 | ✓ |
| `RuleName` newtype | `rules.rs:37` + integ + 测试 | ✓ |
| `RULES_SCHEMA_VERSION` | `rules.rs:31` + 单测 | ✓ |
| `SELF_EVENT_PREFIXES` | `rules.rs:33` + 单测 | ✓ |

**防冲突 grep**：

- `grep -rn 'RuleName' crates/roostery/src/` → 仅 rules.rs + integ；不冲突 hooks_merge `AgentKind` 等既有 enum
- `grep -rn 'HookEvent' crates/roostery/src/` → 仅 hook_event.rs / rules.rs + 测试；与 `hooks_merge::HookFragment` 名字相近但作用域明确（前者跨模块 dispatcher 入口，后者是 hooks_merge 内部 JSON 片段）；无误指

无术语冲突。

## 5. 架构归并

### 5.1 `ARCHITECTURE.md §2 术语表`

加 5 个新词条：`HookEvent` / `HOOK_EVENT_SCHEMA_VERSION` / `RulesConfig` + `CompiledRule` / `RULES_SCHEMA_VERSION` + `SELF_EVENT_PREFIXES` / `Match`。

### 5.2 `ARCHITECTURE.md §3 Module E`

加 hook_event + rules 子节描述；子 feature 列表 `dispatcher-rules` 标 done。

### 5.3 `ARCHITECTURE.md §4 契约表 §4.4`

`HookEvent` 行标 "Phase 4 已落地（feature `2026-05-18-dispatcher-rules`）"。

### 5.4 `ARCHITECTURE.md §6 已知约束`

加 1 条：`HOOK_EVENT_SCHEMA_VERSION = 1` + `RULES_SCHEMA_VERSION = 1` 双公开承诺；rules first-match-wins + self-event 短路是 dispatcher-loop 必依赖的合约。

### 5.5 `.codestable/requirements/runtime-neutral.md`

变更日志加 2026-05-18 `dispatcher-rules` 落地条目；`implemented_by` 加本 feature；status 保持 `draft`。

### 5.6 `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`

`dispatcher-rules` `in-progress → done`。

### 5.7 `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md §5 第 12 项`

`planned → done`。

### 5.8 `.codestable/attention.md`

候选盘点见 §8。

### 5.9 `.codestable/compound/`

无新 decision 候选。

## 6. requirement 回写

- 方案 frontmatter `requirement: runtime-neutral`（current status: `draft`）
- 本 feature 兑现 req 的"接入新 runtime 需要写一份 adapter"—— rule engine 是用户配置层的接入面：通过 YAML 规则把 HookEvent 路由到具体 runner，规则维度（hook_source / workspace_glob / trigger_meta）独立于哪家 runtime
- 处理方式：**update**——`implemented_by` 加本 feature；变更日志追加 2026-05-18 条目；status 保持 `draft`（等 dispatcher-loop 收尾才有完整链路）

## 7. roadmap 回写

- 方案 frontmatter `roadmap: rust-rewrite` / `roadmap_item: dispatcher-rules`，两字段都有值
- `rust-rewrite-items.yaml` 第 93-99 行 `dispatcher-rules` 当前 `status: in-progress` + `feature: 2026-05-18-dispatcher-rules`
- 改 `status: done`，`validate-yaml.py` 校验通过
- `rust-rewrite-roadmap.md` §5 第 12 项 `planned → done` + feature 引用

## 8. attention.md 候选盘点

**本 feature 暴露的"下个 feature 还会撞"硬约束**：

1. **clippy `manual_from_str`**：在 Rust 中给类型加 `pub fn from_str(s) -> Self` 不实现 `std::str::FromStr` trait 会撞 clippy 警告。**触发判据**：feature dev 想给新 newtype / value 类型加 `from_str(String) -> Self` 构造器。**是否归入 attention.md**：**否**——是 clippy lint 通识；下次撞了 5 分钟改个名就行；和上一 feature 撞的 `drop_non_drop` 同性质（lint 规则成千上百，逐一加 attention 会膨胀）。**结论：本 feature 无新 attention 候选**。

2. **`#[non_exhaustive]` 测试 fixture corollary 再次复用**：integ test 用 `serde_json::from_value(json!({...}))` 反序列化构造 `HookEvent`（`#[non_exhaustive]` struct）——上次 dispatcher-trace-budget integ 用同方式构造 `BudgetState`。**结论**：已有的 attention 条目持续起作用，**无新增**。

## 9. 遗留

### 9.1 后续观察项

- **Phase 4 收尾 dispatcher/ 子目录化**：dispatcher-trace-budget acceptance 已记，本 feature 又加 2 个 dispatcher 相关模块（hook_event + rules）。当前顶层 17 个 .rs 文件，dispatcher-runners + dispatcher-loop 起来后 ≥20；届时建议走 `cs-refactor` 一次性聚 `src/dispatcher/`。本期不做。
- **template / continue / switch_by_field 扩展**：design §1.3 N1/N4/N5 推后；dispatcher-loop / dispatcher-runners 真消费后若发现 MVP 不足，走 cs-roadmap update。
- **per-rule budget override**：dispatcher-trace-budget §9.1 已记。

### 9.2 顺手发现

- **clippy `manual_from_str` + `manual_contains`**：本期共撞 3 处（1 `from_str` 命名 + 2 处 `iter().any(==)`），全部当场修了。**经验**：新写 Rust 模块的 lint 阶段会比写功能花更多时间——下个 feature dev 写完功能先跑 `cargo clippy --all-targets -- -D warnings` 避免最后阶段堆错。

### 9.3 已知限制

- **rules 不支持 OR / 复合表达式**：当前 AND 多维度只支持 each 维度 1 个值；要"hook_source 是 A 或 B"必须写 2 条规则。design §1.3 N5 明示推后。
- **trigger_meta_eq 仅字面量相等**：不支持 regex / contains / range；要这些维度需要 cs-roadmap update 扩 schema。
- **本 feature 不交付 dispatch 编排**：rules 是纯查询函数库；事件流入 / runner 派发等 dispatcher-loop（Phase 4 收尾 feature）才有。
