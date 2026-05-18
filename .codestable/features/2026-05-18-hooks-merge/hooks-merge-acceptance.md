---
doc_type: feature-acceptance
feature: 2026-05-18-hooks-merge
status: passed
date: 2026-05-18
summary: hooks_merge 模块落地——3 个 include_str! 模板（cc_stop_hook.json / codex_stop_hook.json / agent_stop_notify.sh）+ 3 公开 fn（render_template / merge_event_hook / apply_template）；JSON 深合并按 event key + matcher + command tail 幂等去重；env 切到 ROOSTERY_AGENT=cc|codex；JSON 输出 indent=2 + \n；atomic .tmp + rename；HooksError #[non_exhaustive] 4 变体。25 lib 单测 + 4 集成测试全过；fmt/clippy/test --all/--doc 四命令绿；同步更新 ARCHITECTURE §2 术语 + §3 Module D hooks_merge 子节 + §4.7 标 Phase 3 已落地；agent-work-in-feishu req 加变更日志；roadmap items + 主文档 status → done。design §2.5 建议沉淀的 "Rust 资源文件子目录约定" 已就绪等 cs-decide 归档评估
tags: [phase-3, module-d, hooks, json-merge, templates, acceptance]
---

# hooks-merge 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-18
> 关联方案 doc：`.codestable/features/2026-05-18-hooks-merge/hooks-merge-design.md`

## 1. 接口契约核对

对照方案第 2.1 节名词层：

| 接口 | 设计签名 | 代码落点 | 一致 |
|---|---|---|---|
| `CC_STOP_HOOK_JSON` const | `include_str!("templates/cc_stop_hook.json")` | `hooks_merge.rs:17` | ✓ |
| `CODEX_STOP_HOOK_JSON` const | 同上 codex | `hooks_merge.rs:18` | ✓ |
| `STOP_HOOK_AGENT_NOTIFY_SH` const | include_str! sh | `hooks_merge.rs:19` | ✓ |
| `HooksError` `#[non_exhaustive]` 4 变体 | ReadFailed / ParseFailed / FragmentInvalid / SaveFailed | `hooks_merge.rs:24-38` | ✓ |
| `pub fn render_template(&str, &str) -> Result<Value, HooksError>` | `{{HOOK_SCRIPT}}` replace + parse | `hooks_merge.rs:41-44` | ✓ |
| `pub fn merge_event_hook(&Path, &Value) -> Result<Value, HooksError>` | event + matcher + command tail 幂等去重 | `hooks_merge.rs:149-219` | ✓ |
| `pub fn apply_template(&str, &Path, &str) -> Result<PathBuf, HooksError>` | render + merge + atomic write | `hooks_merge.rs:240-249` | ✓ |

**调用示例**核对（design §2.1）：

- caller `apply_template(CC_STOP_HOOK_JSON, &cc_settings, sh_path.to_str().unwrap())` → 集成测试 `cc_and_codex_coexist_in_one_settings_file` 覆盖 ✓
- 模板形态：`templates/cc_stop_hook.json` 含 `ROOSTERY_AGENT=cc {{HOOK_SCRIPT}}` 与 design §2.1 一致 ✓
- sh 形态：`agent_stop_notify.sh` 调 `roostery dispatcher fire` 与 design §2.1 一致 ✓

**名词层"现状 → 变化"**：

- ✓ 新增 `crates/roostery/src/hooks_merge.rs`
- ✓ `templates/cc_stop_hook.json`、`templates/codex_stop_hook.json`、`templates/agent_stop_notify.sh` 三新文件
- ✓ `lib.rs:5` 加 `pub mod hooks_merge;`
- ✓ Cargo.toml 不动（serde_json 已有）
- ✓ paths.rs 不动（caller 自管 target path）

**流程图核对**（§2.2 mermaid）：

- `apply_template` 主流程 A→B→C→...→T→U 在 `hooks_merge.rs:240-249` + 内部子 fn 完整落地 ✓
- `merge_event_hook` 内部"detect event key / extract matcher entry / load existing / find bucket by matcher / dedupe by command tail" 逐节点对应 `hooks_merge.rs:149-219` 主体 + `detect_event_key` / `extract_first_matcher_entry` / `load_existing` / `command_tail` 私有 fn ✓

**无接口偏离**。

## 2. 行为与决策核对

**需求摘要逐项验证**：

- ✓ JSON 深合并 hook 片段 → settings.json
- ✓ 幂等去重（event + matcher + command tail 三层）
- ✓ 3 个 include_str! 模板
- ✓ env 前缀切到 `ROOSTERY_AGENT`
- ✓ `{{HOOK_SCRIPT}}` 字符串替换
- ✓ JSON 输出 indent=2 + `\n`
- ✓ atomic write
- ✓ 纯 lib 扩展无 CLI 变更

**明确不做（§1 + §3.2 反向核对）grep**：

- [x] `grep -E "use tinytemplate|use handlebars|use askama" hooks_merge.rs` → 无
- [x] `grep "FEISHU_HUB_" hooks_merge.rs` → 仅文档注释 + 测试断言 / migration fixture（合规）
- [x] `grep -rn "FEISHU_HUB_" crates/roostery/src/templates/` → 无（模板内容 ROOSTERY_AGENT）
- [x] `grep -E "use tokio|async fn" hooks_merge.rs` → 无
- [x] `grep "Config\|config::" hooks_merge.rs` → 无
- [x] `grep "Journal\|journal::" hooks_merge.rs` → 无
- [x] `grep -E "fn parse_cc_schema|fn validate_cc|fn unmerge|fn remove_hook" hooks_merge.rs` → 无
- [x] `wc -l hooks_merge.rs` = 545（产品 249 < 500，超出皆 25 条内联单测，同前 3 feature 同性质偏离）
- [x] `Cargo.toml` 无新依赖
- [x] `grep "fn main\|Subcommand" hooks_merge.rs` → 无

**关键决策 D1-D12 落地**：

| # | 决策 | 代码体现 |
|---|---|---|
| D1 | 3 模板嵌入 | `hooks_merge.rs:17-19` 3 const + `src/templates/` 3 文件 |
| D2 | env 切 ROOSTERY_AGENT | 模板内容 + 测试断言 |
| D3 | indent=2 + `\n` | `write_json_atomic` 用 `to_vec_pretty` + `push(b'\n')` |
| D4 | `{{HOOK_SCRIPT}}` 替换 | `render_template` 用 `str::replace` |
| D5 | atomic `.tmp` + rename | `write_json_atomic` |
| D6 | 3 公开 fn | render/merge/apply 全部公开 |
| D7 | HooksError 4 变体 #[non_exhaustive] | `hooks_merge.rs:24-38` |
| D8 | event key 自动探测 | `detect_event_key` |
| D9 | command tail 去重 | `command_tail` fn + `merge_event_hook` 内 dedup |
| D10 | sh 调 roostery dispatcher fire | `templates/agent_stop_notify.sh` |
| D11 | 不内置 target_path 默认 | 公开 fn 都要 caller 传 path |
| D12 | 纯 lib 扩展 | main.rs 不动；无 CLI 子命令 |

**编排层"现状 → 变化"**：API 已对外公开；caller（未来 roostery-init）调用模式与 design §2.1 调用示例一致；本 feature 不真正消费 hooks-merge ✓

**流程级约束（§2.2 不变量 1-7）**：

| 不变量 | 守护方式 |
|---|---|
| 1 merge idempotent | `apply_template_idempotent` + 集成 `double_apply_is_byte_for_byte_idempotent` 双重守护 |
| 2 atomic `.tmp` + rename | `apply_template_to_missing_target` 断言 `.tmp` 不残留 |
| 3 target 不存在 → fragment 当结果 | `merge_into_missing_target` 单测 |
| 4 parse 失败不破坏原文件 | `target_invalid_json_returns_parse_failed` + 实现层只在 rename 成功才覆盖原文件 |
| 5 fragment 5 形态校验 | S3.1-S3.5 单测全覆盖 |
| 6 command tail 去重 | `command_tail_strips_env_prefix` + `merge_legacy_env_treated_as_same_command_by_tail` |
| 7 indent=2 + `\n` | 集成 `output_is_indent_two_with_trailing_newline` |

**挂载点反向核对（§2.3）+ 沙盘推演**：

| # | 挂载点 | grep 验证 |
|---|---|---|
| 1 | `crates/roostery/src/hooks_merge.rs` 存在 | `ls` ✓ |
| 2 | `pub mod hooks_merge;` in lib.rs | `lib.rs:5` ✓ |
| 3 | `templates/` 3 文件全在 | `ls templates/` ✓ |
| 4 | 3 个 pub const 暴露 | `hooks_merge.rs:17-19` ✓ |
| 5 | `apply_template` 公开 fn | `hooks_merge.rs:240` ✓ |

**反向核查**：`grep -rn "hooks_merge" crates/roostery/src/` → `lib.rs:5` + `hooks_merge.rs` + 测试；无清单外挂入点 ✓

**拔除沙盘推演**：删 `hooks_merge.rs` + `lib.rs:5` + `templates/` 3 文件 → `cargo build` 仍能编译（其他模块未消费 hooks_merge）；无残留 ✓

**遗留**：无清单外挂入点漏记。

## 3. 验收场景核对

#### Render S1.1-S1.4

- [x] **S1.1 CC render**：`render_cc_template_happy` ✓
- [x] **S1.2 Codex render**：`render_codex_template_happy` ✓
- [x] **S1.3 占位符替换**：`render_no_placeholder_left` ✓
- [x] **S1.4 模板非法 JSON**：`render_invalid_json_returns_parse_failed` ✓

#### Merge S2.1-S2.7

- [x] **S2.1** target 不存在 → `merge_into_missing_target` ✓
- [x] **S2.2** 已有不同 event → `merge_into_target_with_different_event` ✓
- [x] **S2.3** 同 event 不同 matcher → `merge_into_same_event_different_matcher` ✓
- [x] **S2.4** 同 matcher 不同 command 追加 → `merge_into_same_matcher_different_command_appends` ✓
- [x] **S2.5** 同 command 去重 + timeout 更新 → `merge_dedup_same_command_updates_timeout` ✓
- [x] **S2.6** ROOSTERY env 同 command 去重 → 同 S2.5（fragment 自带 ROOSTERY env）✓
- [x] **S2.7** legacy FEISHU env 同 tail 去重保留 existing → `merge_legacy_env_treated_as_same_command_by_tail` ✓

#### Fragment 校验 S3.1-S3.5

- [x] **S3.1** 0 event key → `fragment_with_zero_event_keys_invalid` ✓
- [x] **S3.2** 2+ event keys → `fragment_with_two_event_keys_invalid` ✓
- [x] **S3.3** matcher array 空 → `fragment_with_empty_matcher_array_invalid` ✓
- [x] **S3.4** hooks 数组空 → 由 `extract_first_matcher_entry` 守护 + S3.5 间接覆盖 ✓
- [x] **S3.5** 缺 command → `fragment_without_command_invalid` ✓

#### apply_template S4.1-S4.5

- [x] **S4.1** target 不存在 → `apply_template_to_missing_target` ✓
- [x] **S4.2** 已有不同 event 共存 → `apply_template_preserves_other_event_hooks` + 集成 ✓
- [x] **S4.3** 二次幂等 → `apply_template_idempotent` + 集成 `double_apply_is_byte_for_byte_idempotent` ✓
- [x] **S4.4** parent dir 自动 → `apply_template_creates_parent_dir` ✓
- [x] **S4.5** atomic `.tmp` → `apply_template_to_missing_target` 断言 ✓

#### JSON 输出 S5.1-S5.2

- [x] **S5.1** indent=2 + `\n` → 集成 `output_is_indent_two_with_trailing_newline` ✓
- [x] **S5.2** Byte-for-byte 与 Python parity（除 env 偏离）→ 默认序列化 + 测试覆盖 ✓

#### 模块级 S6.1-S6.5

- [x] **S6.1** `cargo test --all` 167 lib + 4 hooks_merge integration + 2 config + 4 smoke + 4 shim + 12 shim unit + 3+4 doc 全绿（新增 ≥12 条 = 25 lib 新增 + 4 集成新增）
- [x] **S6.2** `cargo test --doc` 全绿
- [x] **S6.3** clippy `-D warnings` 通过
- [x] **S6.4** `cargo fmt --all --check` 通过
- [x] **S6.5** 3 const 非空：`embedded_consts_nonempty` 单测 ✓

前端改动：无。

## 4. 术语一致性

| 术语 | 代码命中 | 一致 |
|---|---|---|
| Hook fragment | 用作变量名 fragment / `extract_first_matcher_entry` 参数 | ✓ |
| Event key | `detect_event_key` fn + 字符串字段名 "hooks" | ✓ |
| Matcher | `new_matcher` 变量 / fragment 中 `matcher` 字段 | ✓ |
| Command tail | `command_tail` fn `hooks_merge.rs:83-93` | ✓ |
| `HOOK_SCRIPT_PLACEHOLDER` | const `hooks_merge.rs:21` | ✓ |
| `ROOSTERY_AGENT` env | 模板内容 + 测试断言 | ✓ |
| HooksError 4 变体 | `hooks_merge.rs:24-38` | ✓ |
| 3 个 pub const | `hooks_merge.rs:17-19` | ✓ |
| `apply_template` / `render_template` / `merge_event_hook` | 全公开 | ✓ |

**防冲突**：

- `grep "FEISHU_HUB_" crates/roostery/src/templates/` → 无
- `grep "tinytemplate\|handlebars" Cargo.toml` → 无
- 三个 const 名字 grep 全仓库无重名冲突 ✓

无不一致。

## 5. 架构归并

对照方案第 4 节，实际写入：

- [x] **`ARCHITECTURE.md §2 术语表`** — 加 hooks_merge 相关词条
- [x] **`ARCHITECTURE.md §3 Module D`** — 加 hooks_merge 子节描述（模板嵌入 + 合并算法 + ROOSTERY_AGENT env 切口径 + Phase 5 替换 sh bridge 路径）+ 子 feature 列表标 done
- [x] **`ARCHITECTURE.md §4 契约表 §4.7`** — 标 "Phase 3 已落地（feature `2026-05-18-hooks-merge`）"
- [x] **`ARCHITECTURE.md §6`** — 加 ROOSTERY_AGENT env 约定条目
- [x] **`.codestable/requirements/agent-work-in-feishu.md`** — 变更日志加 hooks-merge 落地条目；`implemented_by` 加本 feature；status 保持 `draft`
- [x] **`.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`** — hooks-merge status `in-progress` → `done`
- [x] **`.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §3 第 9 项** — status `planned` → `done` + feature 引用
- [ ] **`.codestable/attention.md`** — 无新增候选（见 §8）
- [ ] **`.codestable/compound/`** — design §2.5 建议沉淀的 "Rust 资源文件子目录约定" 已就绪；扩 rust-module-organization decision 加第 5 档；**退出后提示用户走 cs-decide**

## 6. requirement 回写

- 方案 frontmatter `requirement: agent-work-in-feishu`（current status: `draft`）
- 本 feature 兑现 req 的"装机后 agent runtime 触发 hook 进入 Roostery 处理路径"基础设施
- 处理方式：**update** — frontmatter `implemented_by` 加本 feature；变更日志追加条目；status 保持 `draft`（用户视角 task 卡呈现要 Phase 5 真正写飞书）

## 7. roadmap 回写

- 方案 frontmatter `roadmap: rust-rewrite` / `roadmap_item: hooks-merge`，两字段都有值
- `rust-rewrite-items.yaml` 第 69-76 行 `slug: hooks-merge` 当前 `status: in-progress` + `feature: 2026-05-18-hooks-merge`（design 阶段已写入）
- 改 `status: done`，`validate-yaml.py --file` 校验通过
- `rust-rewrite-roadmap.md` §3 第 9 项当前 `状态: planned` → 改 `状态: **done**（feature ...）` + 补充备注

## 8. attention.md 候选盘点

**潜在候选**：

1. **`src/templates/` 资源文件子目录约定** — 本 feature 项目首次引入；归 cs-decide 扩 rust-module-organization 决策档位（acceptance §5 已记），**不进 attention.md**（attention 是"碎片硬约束"，不是"结构约定 / 选型"——后者归 decision）
2. **command tail 去重算法** — 业务逻辑细节归 cs-trick 而非 attention.md

**结论**：**本 feature 未暴露需要补入 attention.md 的内容**。

## 9. 遗留

- **后续观察项**（design §4.1 已记）：
  - Phase 5 `bot-stop-hook` 替代 sh bridge 时删 `templates/agent_stop_notify.sh` + 删 `STOP_HOOK_AGENT_NOTIFY_SH` const + 改 JSON 模板 command 指向原生 Rust handler
  - Gemini 模板归后续 feature
  - agent runtime 自动检测归 `roostery-init`
  - legacy `FEISHU_HUB_AGENT=cc` env migration 归 `roostery-init` first-run 检测
- **流程偏差**：与 config feature 同——单文件 lib 紧耦合，step 拆开会反复改同一文件；测试覆盖各 step exit signal；接受
- **总 LOC 偏离**：545 > design 反向核对 `< 500`；产品 249 < 500，超出皆内联测试（25 条）——同前 3 feature 同性质
- **顺手发现**：无
- **design 2.5 建议沉淀的 convention 已就绪**：等 acceptance 阶段确认是否走 cs-decide 归档（**退出后第 2 项特检提示**）
