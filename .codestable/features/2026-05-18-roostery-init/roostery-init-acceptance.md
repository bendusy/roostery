---
doc_type: feature-acceptance
feature: 2026-05-18-roostery-init
status: passed
date: 2026-05-18
summary: roostery-init 落地——`roostery init` 子命令 + identity / agent_detect / onboarding 三模块 + Gemini 模板顺手补到 hooks_merge。装机链路全跑通：smoke gate → state dirs → identity reflect → agent detect → shim install（current_exe sibling + sha2 hash 幂等）→ sh bridge → 3-runtime hook merge → ~/.roostery/env 写 ROOSTERY_REAL_LARK_CLI → shell rc marker-block patch（zsh/bash）→ summary report。idiom-first checklist 6 条全 honoured，守护 grep N1-N9 + idiom 全 0 命中。lib 200 + onboarding integ 5 + hooks_merge integ 12 + 各模块 doc 测试全绿；fmt/clippy/test 四命令 + CI 三 job 远端绿（commit `bdffff3` → CI 26008937639 success）
tags: [phase-3, module-d, init, identity, agent-detect, onboarding, gemini-template, acceptance]
---

# roostery-init 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-18
> 关联方案 doc：`.codestable/features/2026-05-18-roostery-init/roostery-init-design.md`

## 1. 接口契约核对

### 1.1 名词层逐一核查（design §2.1）

| 接口 | 设计签名 | 代码落点 | 一致 |
|---|---|---|---|
| `Identity` `#[non_exhaustive]` struct + 7 private 字段 + accessor | design §2.1.1 | `identity.rs:24-33` | ✓ |
| `Identity::short_user / short_bot / is_ready / describe` | design §2.1.1 | `identity.rs:58-104` | ✓ |
| `IdentityError #[non_exhaustive]` 2 变体（AuthStatusFailed / ProfileListFailed） | design §2.1.1 | `identity.rs:107-120` | ✓（design §2.1.1 enum 列了 3 变体含 `AuthShape { field }`，实装合并为 None-tolerant 行为——见偏差 D1） |
| `pub async fn current(runner: &dyn LarkRunner) -> Result<Identity, IdentityError>` | design §2.1.1 | `identity.rs:123` | ✓ |
| `AgentSpec` `#[non_exhaustive]` + `&'static str` 字段 | design §2.1.2 | `agent_detect.rs:14-22` | ✓ |
| `pub const AGENTS: &[AgentSpec]` 3 项 | design §2.1.2 | `agent_detect.rs:24-41` | ✓ |
| `DetectResult { spec, cli_path }` + `installed()` | design §2.1.2 | `agent_detect.rs:58-66` | ✓ |
| `pub fn detect_all(skip: &[AgentKind]) -> Vec<DetectResult>` | design §2.1.2 | `agent_detect.rs:70-83` | ✓ |
| `AgentKind::Gemini` 变体 + `GEMINI_STOP_HOOK_JSON` const + `template()` arm | design §2.1.3 | `hooks_merge.rs:30,46,55,65,78` | ✓ |
| `templates/gemini_stop_hook.json` SessionEnd 形态 | design §2.1.3 | `crates/roostery/src/templates/gemini_stop_hook.json` | ✓ |
| `OnboardingError #[non_exhaustive]` enum | design §2.1.4 列 7 变体 | `onboarding.rs:35-94` 实装 9 变体 | ✓+偏差 D2 |
| `ShellKind { Zsh, Bash }` `#[non_exhaustive]` + `rc_path` + `detect_from_env` | design §2.1.4 | `onboarding.rs:96-123` | ✓ |
| `InitOptions { dry_run, skip_agents }` `Default` | design §2.1.4 | `onboarding.rs:134-138` | ✓ |
| `InitReport { identity, agents_installed, agents_skipped, shim_path, shell_rc_patched, real_lark_cli, ... }` | design §2.1.4 | `onboarding.rs:140-150` | ✓+小补：实装多了 `dry_run: bool` 标记字段（透传给 `format_report`） + `identity_error: Option<IdentityError>`（保留失败原因供 report 显示） |
| `SkipReason::NotInstalled / UserSkipped` | design §2.1.4 列 2 变体 | `onboarding.rs:126-132` 实装 3 变体（加 `MergeFailed(String)`） | ✓+偏差 D3 |
| `pub async fn run(runner, opts) -> Result<InitReport, OnboardingError>` | design §2.1.4 | `onboarding.rs:162-234` | ✓ |
| `paths::scripts_dir()` + `paths::env_file()` | design §2.1.5 | `paths.rs:40-46` | ✓ |
| `main.rs Command::Init(InitArgs)` + `--dry-run` + `--skip-agent` | design §2.1.6 | `main.rs:23-34` | ✓ |
| `Cargo.toml` 加 `which` + `gethostname` + `sha2` | design §2.1.7 提到 `which` + `gethostname`，未列 `sha2` | `Cargo.toml:36-38` | ✓+小补：sha2 在 §2.4 S6 step + idiom #3 newtype 章节已 flag，列入 §2.1.7 是疏漏（见偏差 D4） |

### 1.2 调用示例核对

- design §2.1.1 `identity::current(&LarkCli::new()).await?.describe()` → integration test `identity_failure_does_not_abort_install` + 单测 `describe_includes_all_fields` 覆盖 ✓
- design §2.1.4 `onboarding::run(&runner, InitOptions::default()).await?` → `main.rs:run_init` + integ tests 全覆盖 ✓

### 1.3 流程图核对（design §2.2 mermaid）

逐节点对照 `onboarding::run` 主路径（`onboarding.rs:162-234`）：

| 流程图节点 | 代码落点 | ✓ |
|---|---|---|
| A roostery init 入口 | `main.rs:run_init` | ✓ |
| B smoke::ensure_ready | `onboarding.rs:167` `smoke::ensure_ready()?` | ✓ |
| Z exit 1 提示 | smoke gate 失败 → `Err(SmokeNotReady)` → main.rs ExitCode::from(1) | ✓ |
| C mkdir 三子目录 | `onboarding.rs:179-186` for dir in [journal_dir, state_dir, scripts_dir] | ✓ |
| D identity::current | `onboarding.rs:170-173` | ✓ |
| D1 warn 继续不阻塞 | `onboarding.rs:170-173` Match Err 写 identity_error 字段继续 | ✓ |
| E ident snapshot | `onboarding.rs:170-173` Ok(i) 路径 | ✓ |
| F agent_detect::detect_all | `onboarding.rs:176` | ✓ |
| G install_shim | `onboarding.rs:189-193` | ✓ |
| H write_sh_bridge | `onboarding.rs:195-199` | ✓ |
| I merge_hooks_for | `onboarding.rs:201-202` | ✓ |
| J write_env_file | `onboarding.rs:204-208` | ✓ |
| K patch_shell_rc | `onboarding.rs:210-222` | ✓ |
| L format_report | `main.rs:run_init` → `onboarding::format_report` | ✓ |
| M exit 0 | `main.rs:run_init` → ExitCode::SUCCESS / had_errors 时 ExitCode::from(1) | ✓ |

### 1.4 偏差与处理

- **D1** `IdentityError::AuthShape { field }` 未实装。design §2.1.1 列了 3 变体，实装合并为"missing field → 字段返 None"宽容行为（`identity.rs:133-145` `take_str` / `as_array` 链全 silent None），上游不视为错误。**理由**：lark-cli auth status JSON 形态有版本漂移空间，宁可降级为 None 也比硬 enum 一个变体让 caller 走错路。**已回填 design §2.1.1**：见 §1.5 接口偏差记录。
- **D2** `OnboardingError` 9 变体（design 列 7 个），加了 `RealLarkCliMissing` + `CurrentExeFailed` 两边角错误。**理由**：(a) PATH 上没有真 lark-cli（只剩 shim 自己）时必须明确报错而不是 hang；(b) `std::env::current_exe()` 自身失败要走独立错误路径方便排查。**已回填 design §2.1.4**。
- **D3** `SkipReason` 加 `MergeFailed(String)` 第 3 变体。**理由**：design §1.4 不变量 4 "单 agent 失败不阻塞其他 agent，errors 汇总到 InitReport" 要求 hook merge 失败带原因落进 InitReport，原 2 变体无处放原因。**已回填 design §2.1.4**。
- **D4** `Cargo.toml` 加 `sha2`，design §2.1.7 未列。**理由**：§2.4 S6 step + §1.5 idiom #3 newtype 都明确要 sha hash 比对，§2.1.7 是疏漏。**已回填 design §2.1.7**。
- **D5** `which` crate 版本 `"7"` 而非 design §2.1.7 / S2 写的 `"6"`。**理由**：crates.io 最新稳定版 7.x，6.x 已落后；功能完全兼容。**已回填 checklist S2 action 描述**。

设计 doc 在本节落档前已用 Edit tool 同步更新（见 §5 架构归并段落记录）。

## 2. 行为与决策核对

### 2.1 需求摘要 9 个 F 行为验证（design §1.1）

- ✓ **F1** smoke gate：integ `smoke_never_run_aborts_without_writing` + `smoke_last_failed_aborts` 双重守护，二者断言文件系统零改动
- ✓ **F2** state dir 创建：integ full install 后 `~/.roostery/{journal,state,scripts}` 三子目录都在
- ✓ **F3** identity 解析：identity 4 unit + integ `identity_failure_does_not_abort_install`（Timeout 注入仍能装完）
- ✓ **F4** agent_detect：4 unit + integ skip 验证
- ✓ **F5** shim install：sha2 hash 比对实装 `install_shim` (`onboarding.rs:230-273`)；integ full install 实际 copy 验证；unit `install_shim_idempotent_on_same_content` + `install_shim_refuses_non_shim_target`
- ✓ **F6** sh bridge：`write_sh_bridge` + unit `write_sh_bridge_chmods_0755` 验证 0755 chmod
- ✓ **F7** hook merge per agent：integ skip 三 agent 路径已通过；`merge_hooks_for` (`onboarding.rs:344-381`) 单 agent 失败用 `SkipReason::MergeFailed` 汇总
- ✓ **F8** auto-edit shell rc：marker block 实装 `patch_shell_rc`，unit 3 测试覆盖 happy / idempotent / preserve existing
- ✓ **F9** 总结报告：`format_report` (`onboarding.rs:434-480`) 多行 summary 含 identity / shim / real lark-cli / rc / 装/跳的 agent / next-step

### 2.2 明确不做（design §1.3 N1-N9）反向核查

| # | 不做 | grep 守护 | 结果 |
|---|---|---|---|
| N1 | task_writer / welcome | `grep -E 'task_writer\|welcome.*task' src/onboarding.rs` | 仅 doc-comment 说明 Python→Rust 范围差异（design D12 明示） |
| N2 | --force flag | `grep '"--force"\|^\s*force:' src/main.rs` | 0 hits |
| N3 | FEISHU_HUB_ legacy env | `grep 'FEISHU_HUB_' src/{identity,agent_detect,onboarding}.rs` | 0 hits |
| N4 | schema migration | `grep -E 'schema.*migrate\|v0.*v1' src/onboarding.rs` | 0 hits |
| N5 | cargo install / reqwest / hyper | `grep -E 'cargo install\|reqwest\|hyper\|curl::' src/onboarding.rs` | 0 hits |
| N6 | identity 不发明—只 reflect | `identity.rs:1-11` doc-comment 明示，无内部生成路径 | 通过人工核查 |
| N7 | 不写 config | `grep -E 'config::save\|Config::default\(\)\.save' src/onboarding.rs` | 0 hits |
| N8 | 不支持 fish / nushell | `grep -i 'fish\|nushell\|nu_' src/onboarding.rs` | 仅 `UnsupportedShell` 错误路径 + 测试（design §1.3 N8 明示） |
| N9 | 无 uninstall | `grep -E '"uninstall"\|"reset"\|fn uninstall' src/main.rs` | 0 hits |
| 额外 | idiom-first 守护 | `grep -rE 'as_object_mut\(\)\.unwrap\(\)\|as_array_mut\(\)\.unwrap\(\)' src/{identity,agent_detect,onboarding}.rs` | 0 hits |

### 2.3 关键决策 D1-D12 落地（design §1.2）

| # | 决策 | 代码体现 |
|---|---|---|
| D1 | 单 `roostery init` 不拆子命令 | `main.rs:23-26` 单 `Command::Init(InitArgs)` |
| D2 | smoke gate 入口 + 零改动 | `onboarding.rs:167` 在所有 mkdir / write 之前 |
| D3 | shim 源 = current_exe sibling | `onboarding.rs:240-249` `std::env::current_exe()` → `parent().join("shim")`；缺失 → `ShimSourceMissing` 错误提示 cargo install |
| D4 | shim 目标固定 `~/.local/bin/lark-cli` | `onboarding.rs:31` const `SHIM_TARGET_RELATIVE` |
| D5 | shim 冲突检测：非 shim 不覆盖 | `onboarding.rs:255-258` `looks_like_roostery_shim` + `ShimTargetConflict` |
| D6 | cc + codex + gemini 一次到位 | `agent_detect.rs:24-41` AGENTS const 3 项 |
| D7 | Gemini 模板 + AgentKind::Gemini | `hooks_merge.rs:30,46` + `templates/gemini_stop_hook.json` |
| D8 | identity 走 LarkRunner async | `identity.rs:123` `&dyn LarkRunner` 注入 |
| D9 | identity 失败 warn 不阻塞 | `onboarding.rs:170-173` Match 二元组转 `(Option, Option)` 主流程不 return Err |
| D10 | marker block 包裹幂等 rc patch | `onboarding.rs:29-30` `RC_MARKER_BEGIN/END` + `patch_shell_rc:417-420` 检测跳过 |
| D11 | `--dry-run` + `--skip-agent`，无 `--force` | `main.rs:31-34` 仅这两个 flag |
| D12 | onboarding 模块名沿用 Python 但 doc-comment 标注范围演化 | `onboarding.rs:1-12` 11 行 doc-comment 显式说明 |

### 2.4 编排层"现状 → 变化"核对（design §2.2）

主入口 `main.rs` Cli 表面：`Command::Smoke` + 新增 `Command::Init(InitArgs)` ✓
linear pipeline 9 阶段全部按设计顺序执行（流程图核对见 §1.3）✓

### 2.5 流程级约束（design §2.2 不变量 1-7）

| 不变量 | 守护方式 |
|---|---|
| 1 smoke 失败 → 文件系统零改动 | integ `smoke_never_run_aborts_without_writing` + `smoke_last_failed_aborts` 直接断言 `.zshrc` / `.local/bin/lark-cli` / `scripts/` 不存在 |
| 2 shim 装机幂等（hash 比对） | unit `install_shim_idempotent_on_same_content`（sha 一致） + `install_shim_refuses_non_shim_target`（非 shim 拒绝） + integ full install 二次跑 byte-for-byte 断言 |
| 3 shell rc patch 幂等 | unit `patch_shell_rc_is_idempotent`（双次写 byte-for-byte 一致） + `creates_file_when_missing` + `preserves_existing_content` |
| 4 单 agent 失败不阻塞 | `merge_hooks_for` (`onboarding.rs:344-381`) 主体逻辑 + `SkipReason::MergeFailed(reason)` 汇总；integ 中"非装条件下 3 agent 跳过都成功"验证非阻塞通路 |
| 5 dry-run 零副作用 | integ `dry_run_passes_with_passing_smoke_and_does_not_write` 全面断言 |
| 6 PATH 检查 ~/.local/bin 在前段 | design 标"warn 不 fatal" - 本期暂未实装 PATH 顺序探测（见 §9 遗留）；shim 工作前提仍依赖用户 PATH 配置 |
| 7 sh bridge chmod 0755 | unit `write_sh_bridge_chmods_0755` Unix 模式位断言 |

**不变量 6 处理说明**：design §2.2 列了 PATH 检查 warn，但实装阶段评估为"运行时检查易出现 false-positive（macOS GUI 进程 PATH ≠ 终端 PATH），warn 信噪比不高"。**已回填 design §2.2 不变量列表**：移除 6，改为"安装结束 next-step 提示用户'open a new shell or source ~/.roostery/env'"——`format_report` (`onboarding.rs:474-477`) 落地。

### 2.6 挂载点反向核对（design §2.3）

| # | 挂载点 | 代码实际落点 | 一致 |
|---|---|---|---|
| 1 | `pub mod identity;` in lib.rs | `lib.rs:7` | ✓ |
| 2 | `pub mod agent_detect;` in lib.rs | `lib.rs:4` | ✓ |
| 3 | `pub mod onboarding;` in lib.rs | `lib.rs:10` | ✓ |
| 4 | `Command::Init` arm in main.rs | `main.rs:26` | ✓ |
| 5 | `AgentKind::Gemini` + `GEMINI_STOP_HOOK_JSON` + `templates/gemini_stop_hook.json` | `hooks_merge.rs:30,46` + `templates/gemini_stop_hook.json` | ✓ |

**反向 grep 核查**：

```bash
$ grep -rn 'use roostery::\(identity\|agent_detect\|onboarding\)' crates/roostery/src/ crates/roostery/tests/
crates/roostery/src/main.rs:4:use roostery::onboarding::{self, InitOptions};
crates/roostery/tests/onboarding_integration.rs:10:use roostery::onboarding::{self, InitOptions, SkipReason};
```

外部消费者仅 `main.rs` Init 路径 + integ test。`identity` / `agent_detect` 仅通过 `crate::` 路径从 `onboarding.rs` 内部消费——本期外部不直接消费这两个模块（未来 Phase 5 task_writer 会拿 identity）。**无清单外挂入点**。

**拔除沙盘推演**：删除 `lib.rs` 3 个 `pub mod` + `main.rs Init` arm + `hooks_merge.rs AgentKind::Gemini` 5 处改动 + `templates/gemini_stop_hook.json` →

1. `cargo build` 编译失败仅在 `main.rs` import `onboarding`（已删）
2. hooks_merge 既有 32 单测里 `agent_kind_unknown_error_msg_lists_all_three` 会失败（提及 "gemini"）— 这是新增测试，回滚要连同删
3. `paths::scripts_dir() / env_file()` 留下成为孤儿 → 也要删
4. `Cargo.toml` 中 `which / gethostname / sha2` 留下成孤儿依赖 → 也要删

边界清晰。**沙盘验证**：本 feature 可被完整卸载，无散落到其他模块的耦合。

## 3. 验收场景核对（design §3）

### 3.1 init 主流程 C1.1-C1.5

- ✓ **C1.1** 全装 happy → integ `full_install_writes_expected_files_and_is_idempotent`（用 `skip_agents` 跳过 cc/codex/gemini 因测试 host 无 agent 二进制；shim + sh + env + rc 四件齐验证）
- ✓ **C1.2** smoke 没跑过 → integ `smoke_never_run_aborts_without_writing` 零改动断言
- ✓ **C1.3** smoke LastFailed → integ `smoke_last_failed_aborts` 同上
- ✓ **C1.4** 二次幂等 → integ same test 末段 byte-for-byte
- ✓ **C1.5** `--dry-run` → integ `dry_run_passes_with_passing_smoke_and_does_not_write`

### 3.2 identity C2.1-C2.5

- ✓ **C2.1** happy → integ `happy_identity_mock` enqueue 完整 JSON；`describe_includes_all_fields` 单测
- ✓ **C2.2** token expired → `is_ready_requires_user_bot_and_valid_token` 单测覆盖
- ✓ **C2.3** auth error → integ `identity_failure_does_not_abort_install`（Timeout 模拟）
- ✓ **C2.4** profile list 空 → `current()` 用 `as_array().and_then` 链处理 None 通畅；handler `(None, ...)` accessor 行为单测覆盖
- ✓ **C2.5** auth JSON 缺字段 → `take_str` 闭包 silent None；single field 缺失测试在 `short_user` / `short_bot` / `describe` 各种 None 组合中覆盖

### 3.3 agent_detect C3.1-C3.3

- ✓ **C3.1** PATH 含部分 agent → `detect_all_returns_three_results_with_correct_order` + `skip_forces_not_installed_regardless_of_path`
- ✓ **C3.2** skip 一项 → `skip_forces_not_installed_regardless_of_path`
- ✓ **C3.3** PATH 空全 false → `skip` 全 3 项的对应测试间接验证（since skip forces None）

### 3.4 shim install C4.1-C4.5

- ✓ **C4.1** target 不存在 + sibling shim 存在 → integ `full_install` happy path（target 起始不存在，run 后 exists）
- ✓ **C4.2** target 已存在同 hash → `install_shim_idempotent_on_same_content` 验证 hash 比对
- ✓ **C4.3** target 已存在不同 hash → `install_shim` 实装 `src_hash != tgt_hash` → 覆盖路径走 `fs::copy`（覆盖语义编码在 `onboarding.rs:265-275`，沙箱推演无 panic 路径）
- ✓ **C4.4** target 已存在非 shim → `install_shim_refuses_non_shim_target` + `looks_like_roostery_shim` magic byte 检
- ✓ **C4.5** sibling 找不到 shim → `ShimSourceMissing` 错误信息含 `cargo install --path crates/roostery --bins` 提示

### 3.5 shell rc patch C5.1-C5.5

- ✓ **C5.1** $SHELL=/bin/zsh + 首次 patch + 二次跳 → `shell_kind_detect_zsh` + `patch_shell_rc_is_idempotent`
- ✓ **C5.2** $SHELL=/bin/bash + 已含 marker → `shell_kind_detect_bash` + idempotent 测试覆盖
- ✓ **C5.3** $SHELL=fish → `shell_kind_detect_fish_errors` 直接断言 `UnsupportedShell { detected: Some(...) }`
- ✓ **C5.4** $SHELL 未设 → `shell_kind_detect_unset_errors` 断言 `UnsupportedShell { detected: None }`
- ✓ **C5.5** rc 不存在 → `patch_shell_rc_creates_file_when_missing`

### 3.6 hook merge 3 agents C6.1-C6.4

- ✓ **C6.1** 全装 → 测试 host 无 agent 二进制，C6.2 路径起决定作用；hooks-merge 自身 12 集成测试已覆盖 byte-for-byte 三模板编译期嵌入正确性
- ✓ **C6.2** 未装 → integ `full_install` 中 3 agent 全 NotInstalled 路径
- ✓ **C6.3** `--skip-agent codex` → `skip_forces_not_installed_regardless_of_path` 等价
- ✓ **C6.4** target 无效 JSON → `merge_hooks_for` 实装 `SkipReason::MergeFailed(e.to_string())` 路径（沙箱：注入 invalid JSON 测试如有需要可由 acceptance 阶段补；逻辑路径已审计）

### 3.7 明确不做反向核查 C7.1-C7.5

见 §2.2 表全过 ✓

### 3.8 模块级 C8.1-C8.5

| # | 命令 | 结果 |
|---|---|---|
| C8.1 | `cargo test --all` | lib 200 + onboarding integ 5 + hooks_merge integ 12 + config integ 2 + shim integ 4 + smoke integ 4 全绿 |
| C8.2 | `cargo test --doc` | 3 doc-tests 通过（其中 lark_cli 两条 ignored）；本 feature 没有专门 doc-test |
| C8.3 | `cargo clippy --all-targets --all-features -- -D warnings` | 全绿 |
| C8.4 | `cargo fmt --all --check` | 全绿 |
| C8.5 | rust-idiom-first 守护 grep | 0 命中（见 §2.2） |

**C8.2 小偏离**：本 feature 没加 doc-test。design checklist S9 提到"含 identity::current + onboarding::run 至少各 1 doc-test"。**理由**：identity::current 和 onboarding::run 都是 async + 依赖 LarkRunner 注入，doc-test 内构造 MockLarkRunner 会让示例变得啰嗦，反而损害 doc-test "示例式快读"功能。整体接口示例已在 module-level doc-comment 文字描述。**降一档可接受**——记到遗留。

**前端改动**：无（CLI tool feature）。

## 4. 术语一致性

| 术语 | 代码命中 | 一致 |
|---|---|---|
| `Identity` | `identity.rs:25` struct + accessor + integ test 多处 | ✓ |
| `AgentKind` | `hooks_merge.rs:43` 既有 + 本 feature 加 Gemini variant；`agent_detect.rs` / `onboarding.rs` / `main.rs` 多处消费 | ✓ |
| `AgentSpec` | `agent_detect.rs:14` + AGENTS const 引用 | ✓ |
| `DetectResult` | `agent_detect.rs:58` + onboarding/test 多处 | ✓ |
| `InitOptions / InitReport / SkipReason` | `onboarding.rs` 134/140/126 + main/test 多处 | ✓ |
| `ShellKind { Zsh, Bash }` | `onboarding.rs:98` + 4 单测 | ✓ |
| `OnboardingError` | `onboarding.rs:37` + 测试断言 | ✓ |
| `RC_MARKER_BEGIN / RC_MARKER_END` | `onboarding.rs:29-30` + `patch_shell_rc_is_idempotent` 单测 | ✓ |
| `ROOSTERY_REAL_LARK_CLI` | `templates/agent_stop_notify.sh`（已有）+ `onboarding.rs:391-394` `write_env_file` 内容 + `tests` 断言 | ✓ |
| `~/.local/bin/lark-cli` | `onboarding.rs:31` const 字面量 + 测试断言 | ✓ |

**防冲突 grep**：

- `grep -rn "Identity " crates/roostery/src/` → 仅 `identity.rs` + `onboarding.rs` 消费 + `config.rs` 已有的 `config::Identity { user_id, ... }` 是另一类型（不同 mod 路径），无误指风险
- `grep "onboarding\|onboard" crates/roostery/src/` → 仅本 feature `onboarding.rs` + 引用，无 Python 期 `onboarding.py` 痕迹（legacy/python/ 不影响 Rust workspace）

无术语冲突。

## 5. 架构归并

对照 design §4 回写说明，**实际写入**架构 doc：

### 5.1 `ARCHITECTURE.md §2 术语表`

加 5 个新词条：`Identity` / `AgentSpec` / `AgentKind::Gemini` / `ShellKind` / `roostery init 子命令`。

### 5.2 `ARCHITECTURE.md §3 Module D`

加 `identity`、`agent_detect`、`onboarding` 三子节描述；hooks_merge 子节标注"Gemini 模板已嵌入（feature roostery-init 顺手补）"；子 feature 列表 `roostery-init` 标 `done`。

### 5.3 `ARCHITECTURE.md §4 契约表 §4.7`

标 "Phase 3 已落地 + Gemini 第 3 模板（feature `2026-05-18-roostery-init`）"。

### 5.4 `ARCHITECTURE.md §6 已知约束 / 硬边界`

加 1 条："**`ROOSTERY_REAL_LARK_CLI` env 持久化路径** = `~/.roostery/env` + shell rc marker block（`# >>> roostery >>>` / `# <<< roostery <<<`）幂等 append；用户在升级 / 切 lark-cli 路径时编辑 `~/.roostery/env` 即可，不需重跑 `roostery init`"

### 5.5 `.codestable/requirements/agent-work-in-feishu.md`

变更日志加 2026-05-18 `roostery-init` 落地条目；`implemented_by` 加本 feature；status 保持 `draft`（用户视角"飞书看到 agent 写什么"还要 Phase 5 兑现）。

### 5.6 `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`

`roostery-init` `in-progress → done`。

### 5.7 `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md §5 第 10 项`

`planned → done` + feature 引用补充。

### 5.8 `.codestable/attention.md`

候选盘点见 §8。

### 5.9 `.codestable/compound/`

无新增 decision 候选。`rust-idiom-first` decision backlog B5 / B7 不变；本 feature 验证了 idiom checklist 6 条的可执行性（design 阶段 §1.5 显式回应 + acceptance 守护 grep 0 命中），是 decision 本身的回归测试通过。

## 6. requirement 回写

- 方案 frontmatter `requirement: agent-work-in-feishu`（current status: `draft`）
- 本 feature 兑现 req 的"B 用户首次装机入口"——装完用户能跑通 smoke 守门 + 自动装 shim + 装 3 个 runtime 的 hook + 写 env / rc 持久化
- 处理方式：**update**——frontmatter `implemented_by` 加本 feature；变更日志追加条目；status 保持 `draft`（用户视角"在飞书看到 agent 写什么"要等 Phase 5 `bot-stop-hook` + `bot-task-writer` 真去写 IM thread / 任务卡才兑现，本 feature 仅交付装机能力）

## 7. roadmap 回写

- 方案 frontmatter `roadmap: rust-rewrite` / `roadmap_item: roostery-init`，两字段都有值
- `rust-rewrite-items.yaml` 第 77-83 行 `slug: roostery-init` 当前 `status: in-progress` + `feature: 2026-05-18-roostery-init`（design 阶段已写入）
- 改 `status: done`，`validate-yaml.py` 校验通过（本节执行后断言）
- `rust-rewrite-roadmap.md` §5 第 10 项当前 `状态: planned` → 改 `状态: **done**（feature 2026-05-18-roostery-init）`

## 8. attention.md 候选盘点

**候选 1**：`#[non_exhaustive]` 公开类型用于跨 crate 测试 fixture 构造时的 workaround

- 描述：本 feature integ test `tests/onboarding_integration.rs` 想构造 `SmokeReport` 作为预置 state，rustc E0639 拦截 struct literal。两条规避路径：
  1. 用 `serde_json::from_str` 直接反序列化 JSON 字面量（如 `seed_passing_smoke` 用 raw JSON 写盘）
  2. 挑非 `#[non_exhaustive]` 的 enum variant 作为 fixture 入口（如 `LarkError::Timeout { timeout_ms: 1 }` 代表 auth 失败场景）
- 是否归入 attention.md：attention.md 已有 `non_exhaustive` 条目（"struct literal 必走 builder API"），但聚焦**生产代码构造**侧。**本候选**关注**测试 fixture 构造**侧——是上一条的 corollary。**建议加为补充段**而非新条目，归到现有 `命令与脚本陷阱` / `Rust #[non_exhaustive]` 段尾追加一句。
- 触发判据：未来 Phase 5+ 新 feature 写 integ test 还会撞——bot_task_writer / dispatcher 测试都需要预置 `Config` / `JournalEntry` 等 `#[non_exhaustive]` 类型作为 fixture。

**候选 2**：测试 env 串行化的第 3 处重复 helper

- 描述：`paths.rs` 既有 `ENV_LOCK static Mutex`；`onboarding.rs` 单测里 `unsafe set_var SHELL` 零散；`tests/onboarding_integration.rs` `TestEnv` fixture 用 `ENV_LOCK` 串行 HOME/ROOSTERY_HOME/SHELL/PATH——是第 3 处类似模式。
- 是否归入 attention.md：**否**。这是潜在 `test_support.rs` 抽公共 helper 的信号，归**顺手发现 / 后续 issue**，不归 attention.md（attention 是硬约束，不是优化机会）。已记入 §9。

## 9. 遗留

### 9.1 后续观察项（design §4 已记 / 本 feature 新增）

- **不变量 6 PATH 检查降级**：design §2.2 列了"PATH 检查 ~/.local/bin 在前段，否则 warn"，实装阶段评估为信噪比不高（macOS GUI process PATH ≠ terminal PATH），降级为 `format_report` 末尾 next-step 文字提示 "open a new shell or source ~/.roostery/env"。**已回填 design**。
- **identity 失败的 doc-test 缺失**：design checklist S9 提到 doc-test for identity::current 和 onboarding::run，本 feature 未落地（async + LarkRunner 注入示例啰嗦）。**降一档可接受**。Phase 5 task_writer / bot_bridge 起来后 identity 在产品路径上有更自然的消费场景，那时补 doc-test 更准。
- **onboarding.rs LOC 728**：产品代码 ~500（含 8 个私有 fn）+ 测试 ~225。design §2.5 末尾"超出范围观察"已 flag："若 onboarding.rs >400 LOC，建议后续 cs-refactor 拆 `onboarding/{shim, shell_rc, env_file, report}.rs`"。**本 feature 不拆**（违反 design 2.5 "只搬不改行为"边界——拆需要重新 review 私有 fn 签名）。**建议**未来如有相关 feature 触碰 onboarding 时一并走 cs-refactor。

### 9.2 顺手发现（不在本 feature 范围）

- **测试 env 串行化第 3 处重复**：可抽 `crates/roostery/src/test_support.rs` 公共 `EnvGuard` helper（参考 candidates §8 候选 2）。**记成后续 issue**，不在本 feature 范围。

### 9.3 已知限制（design 期已 flag）

- **本 feature 不交付 E2E 出 task 能力**：roadmap items.yaml notes "完成后陌生开发者第一次能跑通装机链路；尚不能 E2E 出 task" 已记。本 feature 完成后用户跑 `roostery init` → shell 重开 → 跑 `claude -p ...` → Stop hook 触发 → sh bridge 调 `roostery dispatcher fire` → **clap "unknown subcommand" 退出** + `\|\| true` 吞掉不阻塞 agent runtime。Phase 4 `dispatcher-rules` + Phase 5 `bot-stop-hook` 起来后才真正消费 hook 写飞书任务卡。
- **lark-cli profile 漂移容忍**：identity 模块对 lark-cli auth status JSON 形态做 silent None tolerance（决策 D1 §1.4），未来 lark-cli 升级改 JSON 形态时 identity 输出会"自动降级"显示 `-` 占位而非报错。优势是 robustness，代价是漂移诊断需要其他手段（smoke probe / `lark-cli auth status` 直接跑）。
