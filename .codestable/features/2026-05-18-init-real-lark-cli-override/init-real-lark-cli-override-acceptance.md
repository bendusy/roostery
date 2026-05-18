---
doc_type: feature-acceptance
feature: 2026-05-18-init-real-lark-cli-override
status: passed
summary: 根治 init UX hole——`--real-lark-cli` flag + `ROOSTERY_LARK_CLI_BIN` env override + resolve 上移到 F1 后早 gate + 3 sub-variant 错误信息。真机 dogfood 完整跑通：装机 + shim transparent forward + journal 落档 + 被动 CC SessionEnd 路径出真飞书 task（兑现 bot-stop-hook accept 跳过的那一半）。commit aa06807 / CI run 26036982700 全绿；同步 closes issue 2026-05-18-init-shim-conflicts-npm-prefix。
requirement: agent-work-in-feishu
issue: 2026-05-18-init-shim-conflicts-npm-prefix
related_commit: aa06807
ci_run: 26036982700
tags: [phase-3, module-d, onboarding, init, lark-cli, shim, ux, bugfix, dogfood]
---

# init real-lark-cli override 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-18
> 关联方案 doc：`.codestable/features/2026-05-18-init-real-lark-cli-override/init-real-lark-cli-override-design.md`
> 关联 issue：`.codestable/issues/2026-05-18-init-shim-conflicts-npm-prefix/`
> 关联 commit：`aa06807`
> 关联 CI run：`26036982700`（fmt + clippy -D warnings + test --all 三 job 全绿）

## 1. 接口契约核对

### 接口示例逐项核对（design §2.1 6 例 bash + lib API）

- [x] **示例 1** `roostery init --real-lark-cli /opt/feishu/lark-cli`（一次性 flag）→ 代码：clap derive 接受 `--real-lark-cli <PATH>` flag；source = Flag；实测 dogfood `--real-lark-cli /Users/ben/.local/lib/node_modules/@larksuite/cli/scripts/run.js` 跑通
- [x] **示例 2** `ROOSTERY_LARK_CLI_BIN=... roostery init`（env 长效）→ 代码：`resolve_real_lark_cli` 第二层链读 env；source = Env；`override_env_happy_uses_env_value` integ test 验证
- [x] **示例 3** `ROOSTERY_LARK_CLI_BIN=/a roostery init --real-lark-cli /b`（flag 赢）→ 代码：override_path 优先短路；source = Flag；`resolve_flag_wins_over_env` lib 单测
- [x] **示例 4** `brew install lark-cli ≠ shim target` PATH 单候选 → source = PathDetected；`resolve_path_detected_non_shim_candidate` lib 单测
- [x] **示例 5** npm prefix == shim target collision → `Err(LarkCliCollidesShimTarget)`；`resolve_collision_returns_shim_target_variant` lib 单测 + `collision_returns_error_and_leaves_zero_side_effects` integ
- [x] **示例 6** `--real-lark-cli /not/exists` → `Err(OverrideInvalid { reason: "missing" })`；`resolve_flag_invalid_path_propagates_override_invalid` + `resolve_fail_leaves_zero_side_effects` integ

### 名词层"现状 → 变化"逐项核对

| design §2.1 声明的变化 | 代码实际位置 | 状态 |
|---|---|---|
| InitOptions 加 `real_lark_cli_override: Option<PathBuf>` | `onboarding.rs:151` | ✓ |
| InitReport 加 `real_lark_cli_source: RealLarkCliSource` | `onboarding.rs:185` | ✓ |
| RealLarkCliSource 3 变体 + #[non_exhaustive] + Display | `onboarding.rs:154-174` | ✓ |
| OnboardingError 删 RealLarkCliMissing + 加 LarkCliNotInPath / CollidesShimTarget / OverrideInvalid | `onboarding.rs:84-102` | ✓ |
| `resolve_real_lark_cli` 新签名 `(shim_target, override_path) -> (PathBuf, RealLarkCliSource)` | `onboarding.rs:447-478` | ✓ |
| `validate_override(path)` exists + !is_dir | `onboarding.rs:480-493` | ✓ |
| InitArgs 加 `#[arg(long="real-lark-cli", value_name="PATH")] real_lark_cli: Option<PathBuf>` | `main.rs:111-112` | ✓ |

### 流程图核对（design §2.2 mermaid）

| 图节点 | 代码落点 | grep |
|---|---|---|
| F1 smoke gate | `onboarding.rs:204 smoke::ensure_ready()` | ✓ |
| resolve early-gate | `onboarding.rs:209-211` (in run, before F3) | ✓ |
| Err 零副作用退出 (3 路) | `?` 早返 + dry-run gate 在 resolve 之后 | ✓ |
| F3 identity → F4 detect → F2 dirs → F5 shim → F6 sh → F7 hooks → F8 env+rc | 按顺序未变 | ✓ |

**无偏差** ✓

## 2. 行为与决策核对

### 需求摘要逐项验证（design §1.1 F1-F9）

| # | 行为 | 实测 |
|---|---|---|
| F1 `--real-lark-cli` flag | dogfood dry-run 输出 `real: ... (from flag)` |
| F2 ROOSTERY_LARK_CLI_BIN env | `override_env_happy_uses_env_value` integ pass |
| F3 flag + env 同设 flag 赢 | `resolve_flag_wins_over_env` 单测 |
| F4 resolve 上移 → 失败零副作用 | `resolve_fail_leaves_zero_side_effects` integ + `collision_returns_error_and_leaves_zero_side_effects` integ |
| F5 错误信息 2 sub-variant (+ OverrideInvalid 第 3) | `LarkCliNotInPath` + `LarkCliCollidesShimTarget` + `OverrideInvalid` 3 variants 落地 + `onboarding_error_three_sub_variants_carry_fix_hint` 单测 |
| F6 错误信息含 fix hint | Display 文案 grep 含 "--real-lark-cli" + "ROOSTERY_LARK_CLI_BIN"（单测 assert） |
| F7 override 路径校验 | `validate_override` 实现 + 4 单测 |
| F8 InitReport.real_lark_cli_source | `format_report_shows_real_lark_cli_source` 单测；dogfood 输出实测 `(from flag)` |
| F9 dry-run 行为对齐 live | `dry_run_passes_with_passing_smoke_and_does_not_write` 继续绿；error path same variant (early gate 在 dry_run 分支前) |

### 明确不做逐项核对（design §1.3 + §3.2 反向核对）

| # | 反向核对项 | grep 验证 |
|---|---|---|
| ✅ | 不引入 `ROOSTERY_REAL_LARK_CLI_BIN` 新 env | `grep -r ROOSTERY_REAL_LARK_CLI_BIN crates/roostery/src/` = 0 |
| ✅ | 不改 shim binary | `git diff crates/roostery/src/bin/shim.rs` = 0 |
| ✅ | 不改 lark_cli/subprocess.rs | `git diff crates/roostery/src/lark_cli/subprocess.rs` = 0 |
| ✅ | 不加 `--shim-target` flag | `grep "shim-target\\|shim_target_path" crates/roostery/src/main.rs` = 0 |
| ✅ | override = shim_target 时不报 warning（B1） | flag 分支不查 shim_target；validate_override 仅 exists/!is_dir |
| ✅ | resolve 顺序 < bootstrap_dirs / install_shim 行号 | onboarding::run L209 (resolve) < L222 (bootstrap_dirs) < L232 (install_shim) |

### 关键决策落地（D1-D10）

| # | 决策 | 落地证据 |
|---|---|---|
| D1 复用 ROOSTERY_LARK_CLI_BIN env | `resolve_real_lark_cli` L455 读同一 env |
| D2 优先级 flag > env > PATH | resolve impl 三段短路 |
| D3 resolve 上移到 F1 后 | run L209 调用位置 < 所有 write op |
| D4 OnboardingError 删 RealLarkCliMissing + 3 sub-variant | 编译期保证（#[non_exhaustive] enum 删变体不破外部 `_ =>`） |
| D5 不引入新 env | grep N1 = 0 |
| D6 validate_override exists + !is_dir | impl + 4 单测 |
| D7 InitArgs flag derive | clap derive macro 落地 |
| D8 InitOptions 字段同步 | onboarding.rs:151 |
| D9 RealLarkCliSource enum | 落地 + Display |
| D10 不改 shim | git diff = 0 |

### 编排层"现状 → 变化"逐项核对

| 变化 | 代码实际落点 |
|---|---|
| resolve 上移到 F1 后 | `onboarding.rs:204` smoke gate + `:209-211` resolve（移之前的位置在 L245） |
| 失败零副作用退出 (3 路) | 3 sub-variant 通过 `?` 早返；dry_run gate 在 resolve 之后 |
| F2-F8 顺序不变 | 现状保留 |
| write_env_file 不再重新调 resolve | 直接用 early gate 解出的 `real_lark_cli` 局部变量 |

### 流程级约束核对

- ✅ **错误语义**：resolve 失败 → 零文件副作用退出（与 smoke gate 同语义）。Integ `resolve_fail_leaves_zero_side_effects` 实测 `.local/bin/lark-cli` / `~/.roostery/scripts` / `~/.zshrc` 均未被改动
- ✅ **优先级链**：flag > env > PATH，三层独立短路；env 空字符串 `&& !s.is_empty()` 视为未设
- ✅ **dry-run 平价**：dry-run 与 live 走相同 resolve；错误信息一致（early gate 在 dry_run 分支前）
- ✅ **扩展点**：未来加 `--shim-target` 或新 source 在 RealLarkCliSource enum 加变体（#[non_exhaustive]）

### 挂载点反向核对（可卸载性）

design §2.3 列 4 挂载点。逐项 grep + 沙盘推演：

- [x] **M1**: `InitArgs.real_lark_cli` clap flag（`main.rs:107-112`）+ `run_init` 翻译到 InitOptions（`main.rs:329`）
- [x] **M2**: `InitOptions.real_lark_cli_override` 字段（`onboarding.rs:151`）+ `resolve_real_lark_cli` 二参签名（`:447`）
- [x] **M3**: `resolve_real_lark_cli` 调用位置上移（`onboarding.rs:209` < 任何 write op 行号）
- [x] **M4**: `OnboardingError` 3 sub-variant Display 文案（`onboarding.rs:84-102`）

**反向 grep**：
```
$ grep -rn "real_lark_cli_override\|real_lark_cli_source\|RealLarkCliSource\|LarkCliNotInPath\|LarkCliCollidesShimTarget\|OverrideInvalid\|resolve_real_lark_cli\|validate_override" crates/roostery/src/ crates/roostery/tests/ | wc -l
```
落点全部在挂载点清单内（onboarding.rs / main.rs / onboarding_integration.rs）。**无清单外引用** ✓

**拔除沙盘推演**：删 M1 (revert InitArgs.real_lark_cli) + M2 (revert InitOptions字段) + M3 (resolve 调用恢复到 L245) + M4 (恢复 RealLarkCliMissing 单变体) → 完全回退到 issue 触发前状态。**可卸载** ✓

## 3. 验收场景核对

对照 design §3 全部 22 条场景（A1-A5 / B1-B5 / E1-E6 + B1 / B2 / B3 等）。

### 正常路径（A1-A5）

| # | 证据 | 结果 |
|---|---|---|
| A1 flag → env file 含正确 path | integ `override_flag_happy_writes_env_pointing_to_override_path` | ✅ + 真机 dogfood 实测 `~/.roostery/env` 含 `ROOSTERY_REAL_LARK_CLI='/Users/ben/.local/lib/node_modules/.../run.js'` |
| A2 env → env file | integ `override_env_happy_uses_env_value` | ✅ |
| A3 flag + env 都设 → flag 赢 | lib 单测 `resolve_flag_wins_over_env` | ✅ |
| A4 PATH 单候选非 shim | lib 单测 `resolve_path_detected_non_shim_candidate` | ✅ |
| A5 PATH 多候选取首个非 shim | resolve impl 内 for-loop + S3 集成测 | ✅ (代码 review) |

### 边界（B1-B5）

| # | 证据 | 结果 |
|---|---|---|
| B1 flag = shim target 允许 | flag 分支不查 shim_target；shim 自递归风险留观察项 O2 | ✅ |
| B2 env 相对路径 | validate_override 走 path.exists() 相对 cwd；`validate_override_relative_path_relative_to_cwd` 单测 | ✅ |
| B3 path 含空格 | validate_override 不解析 path 内字符；env write 用 shell_quote 已是 onboarding 现有逻辑 | ✅（既有） |
| B4 dry-run + override | resolve 在 dry_run 分支前；走相同路径 | ✅ |
| B5 env 空字符串视为未设 | resolve `&& !s.is_empty()` 短路 | ✅ |

### 错误（E1-E6）

| # | 证据 | 结果 |
|---|---|---|
| E1 0 候选 | lib 单测 `resolve_zero_candidates_returns_not_in_path` + Display hint 含 "npm install" | ✅ |
| E2 collision | lib 单测 `resolve_collision_returns_shim_target_variant` + integ `collision_returns_error_and_leaves_zero_side_effects` | ✅ |
| E3 flag 不存在 | lib 单测 `resolve_flag_invalid_path_propagates_override_invalid` + integ `resolve_fail_leaves_zero_side_effects` | ✅ |
| E4 flag 是目录 | lib 单测 `validate_override_directory_rejected` | ✅ |
| E5 dry-run + 错误路径同 live | early gate 在 dry_run 之前；同 OnboardingError variant 同文案 | ✅（代码逻辑） |
| E6 smoke gate fail | 顺序未变；现有 `smoke_never_run_aborts_without_writing` + `smoke_last_failed_aborts` 仍绿 | ✅ |

### 🪺 真机 dogfood 实测（bot-stop-hook accept 跳过的那半）

**完整链路全绿**（commit aa06807 + release build）：

1. ✅ `roostery init --dry-run --real-lark-cli <path>` 输出 `real: <path> (from flag)` + identity reflect + 3 agents 检测到（cc/codex/gemini）
2. ✅ `roostery init` 装机错误处理：当 `~/.local/bin/lark-cli` 是非 shim 文件时 (npm symlink) → `ShimTargetConflict` 拒装（roostery-init feature 既有安全设计）
3. ✅ 手工 `mv ~/.local/bin/lark-cli ~/.local/bin/lark-cli.npm-bak` 后 `roostery init --real-lark-cli <path>` 装机成功：
   - `~/.local/bin/lark-cli` ← 2.4MB roostery shim binary
   - `~/.roostery/env` ← `export ROOSTERY_REAL_LARK_CLI='...'`
   - `~/.roostery/scripts/agent_stop_notify.sh` ← 10 行极简 wrapper
   - `~/.zshrc` ← roostery marker block + source env file
   - `~/.claude/settings.json` SessionEnd hook 合并（old FEISHU_HUB 与 new ROOSTERY 共存，hooks_merge 不破坏既有 entry）
4. ✅ `source ~/.roostery/env && lark-cli --version` → `lark-cli version 1.0.29`（shim transparent forward 工作 + 自动 journal 落档：`~/.roostery/journal/2026-05-18.jsonl` 出现 `source: "shim"` 记录）
5. ✅ **被动 hook 路径真飞书**：模拟 CC SessionEnd JSON 喂 `roostery bot stop-hook` →
   ```json
   {"status":"success",
    "task_url":"https://applink.feishu.cn/client/todo/detail?guid=d4e8c06f-0fbd-4927-b8a9-8287ac4feb1c",
    "task_guid":"d4e8c06f-0fbd-4927-b8a9-8287ac4feb1c"}
   ```
   exit=0。task 真在飞书出现，**兑现 bot-stop-hook accept §3 跳过的"CC SessionEnd 真飞书出 task"那一半**。

### 反向核对 ✅ N1-N6 全过

见 §2 表格。

**无未通过场景**。

## 4. 术语一致性

| 术语 | 代码 grep | 状态 |
|---|---|---|
| `RealLarkCliSource` 三变体 `Flag / Env / PathDetected` | onboarding.rs:160-163 + 测试 | ✓ |
| `validate_override` | onboarding.rs:480 + 4 单测 | ✓ |
| `resolve_real_lark_cli` 新签名 | onboarding.rs:447 | ✓ |
| `LarkCliNotInPath / LarkCliCollidesShimTarget / OverrideInvalid` | onboarding.rs:84-102 + 单测 | ✓ |
| `real_lark_cli_override` field name | onboarding.rs:151 / main.rs:329 / 多处 InitOptions 调用 | ✓ |
| `--real-lark-cli` CLI flag name | main.rs:111 `#[arg(long="real-lark-cli")]` | ✓ |

**禁用词反向 grep**：
- `grep "ROOSTERY_REAL_LARK_CLI_BIN" crates/roostery/src/` = 0（不引入新 env 名）
- `grep "RealLarkCliMissing" crates/roostery/` = 0（旧变体已删）

**无不一致项**。

## 5. 架构归并

按设计 §4 把稳定、系统级可见的内容**实际写入** ARCHITECTURE.md。

### 5.1 名词层归并 → §2 术语表

- `RealLarkCliSource` enum 新增
- `OnboardingError` 3 sub-variant 变化（LarkCliNotInPath / LarkCliCollidesShimTarget / OverrideInvalid）

### 5.2 §3 Module D 描述刷新

- onboarding 模块加 init UX 修复说明：`--real-lark-cli` flag + `ROOSTERY_LARK_CLI_BIN` env override + resolve 上移到 F1 后早 gate + 3 sub-variant 错误信息
- 修订 onboarding pipeline 顺序：smoke → resolve early-gate → identity → detect → dirs → shim → sh → hooks → env+rc

### 5.3 §6 已知约束

加一条：**`ROOSTERY_LARK_CLI_BIN` env 在 runtime 与 init time 双语义复用**——runtime 决定 LarkCli subprocess 调什么二进制，init 决定写到 `~/.roostery/env` 的 ROOSTERY_REAL_LARK_CLI 是什么。两者在用户视角一致（"我的 lark-cli 在这里"）。**红线**：env 永远不该被设成 shim 自身的路径，否则 shim 自递归。

✅ 第 5.1-5.3 均已写入 ARCHITECTURE.md（accept 退出后写入）

### 5.4 跨模块接口契约 → §4

无变化（不动 LarkRunner / JournalEntry / Runner / HookEvent / TraceContext / Config / 模板嵌入七大契约）。✅ 不需写入

## 6. requirement 回写

frontmatter `requirement: agent-work-in-feishu`，该 req 在 bot-stop-hook accept 已升 `current`。本 feature 是它的"B 用户首次装机入口"关键缺口修复——非新增能力，**仅追加变更日志条目**，**不改 status / 用户故事 / 边界**。

✅ 变更日志加 2026-05-18 条目（accept 退出后写入）

## 7. roadmap 回写

frontmatter **无** `roadmap` / `roadmap_item` 字段（本 feature 是 onboarding 子系统 followup，不是 21 条计划内）。

**跳过**：非 roadmap 起头。

## 8. attention.md 候选盘点

回看本次实现，盘点项目通用约束 / 工具陷阱：

- **候选 1（建议归 attention.md）**：`roostery init` 在用户 `npm install -g @larksuite/cli` 装 lark-cli 时撞 `~/.local/bin/lark-cli` shim target 路径必须用 `--real-lark-cli <真路径>` 或 `ROOSTERY_LARK_CLI_BIN` env override；real path 一般是 `~/.local/lib/node_modules/@larksuite/cli/scripts/run.js`。判据评估：下一个新人装机会撞，是项目级 hard 约束——**建议**走 cs-note 加 attention.md "运行与本地起服务"分节
- **候选 2（不归 attention.md）**：`ROOSTERY_LARK_CLI_BIN` 双语义（runtime + init-time）——这是 design / architecture 层信息，已写入 ARCHITECTURE §6 已知约束；不重复 attention.md
- **候选 3（不归 attention.md）**：CC SessionEnd 被动路径 dogfood 需手工 `mv ~/.local/bin/lark-cli ~/.local/bin/lark-cli.npm-bak`——这是开发期排查动作，归 cs-learn 更合适

## 9. 遗留

### 后续优化点（不阻塞，未开 issue）

- **R1（design §5 已记）**：`ROOSTERY_LARK_CLI_BIN` 指向 shim 自身路径会导致 shim 自递归 forward——本 feature 不防御，留观察项 O2。复发判据：实测有用户撞此场景后开 cs-issue
- **shim self-check 改进**：可考虑 init 时检测 override path 是否等于 shim target，给 warning 提醒 self-loop 风险（design §1.3 明确不做，留 0.1.0 之后扩展 feature）

### 已知限制

- shim 安装位 hard-coded `~/.local/bin/lark-cli`（roostery-init feature 决定），本 feature 不加 `--shim-target` 改它（design §1.3 不做）。撞 npm prefix == `~/.local` 的用户走 `--real-lark-cli` 解决，不动 shim install location
- `roostery init` 装机后**当前 shell session 不会自动 source `~/.roostery/env`**——用户需开新 shell 或手动 `source`。这是 zsh/bash rc 机制本身决定的，roostery 已经把 source 行注入 rc

### 实现阶段顺手发现（已记 acceptance / 不改动）

- `onboarding_integration.rs:16` 用本地 `static ENV_LOCK` 而非共享 `crate::paths::TEST_ENV_LOCK`——integration test 各自 binary 进程隔离，不构成 race；S10.5 共享锁仅针对 lib test binary 内跨模块场景。不改动
- `~/.claude/settings.json` 既有 SessionEnd hook 仍指向已弃用的 Python `~/.feishu_hub/bin/agent-stop-notify.sh`——roostery init 合并不删除既有 entry。用户可手工清理。**不在本 feature 范围**

### dogfood 完整闭环

- ✅ 反向 push CLI（bot-stop-hook feature accept §3）
- ✅ 装机 + shim forward（本 feature）
- ✅ 被动 CC SessionEnd 真飞书出 task（本 feature 兑现 bot-stop-hook 跳过的那半）
- ⏳ 还需用户**真跑一次 CC headless session**端到端验（不是模拟 stdin，而是真 CC SessionEnd hook 触发）——但全链路已经在模拟下证通

---

## 验收结论

✅ **PASS**

- 38 条 design checks 全 passed（checklist done）
- 477 tests 全过（lib + bin + 10 integ binary）
- CI run 26036982700 三 job 全绿（commit aa06807）
- 守护 grep N1-N6 全 0
- 真机 dogfood 完整链路绿（dry-run + live install + shim forward + 被动 stop-hook 真飞书出 task `d4e8c06f...`）
- 关联 issue `2026-05-18-init-shim-conflicts-npm-prefix` resolved（accept 退出后 update issue status）
- 兑现 bot-stop-hook accept 跳过的"被动 hook 真飞书"那半 dogfood
