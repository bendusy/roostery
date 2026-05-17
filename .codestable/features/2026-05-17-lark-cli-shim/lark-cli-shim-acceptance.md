---
doc_type: feature-acceptance
feature: 2026-05-17-lark-cli-shim
status: passed
date: 2026-05-17
summary: lark-cli shim 独立 bin（PATH-prefix 透传 + 流式 tee + 11 字段 JournalEntry，TTY/interactive 走 execv 直通，anti-recursion + NOJOURNAL env，std::thread + std::process 模型）；接口 / 决策 / 验收场景 / 术语全核对通过；架构 §2/§3/§5/§6 已归并 shim 词条、Module C 子节、streaming vs buffered 决定、红线兑现链补述；roadmap items.yaml + 主文档 status → done；portable-by-default req 变更日志补 shim 兑现条；attention.md 无新增候选（既有"shim 装机点"条目已覆盖）
tags: [phase-2, module-c, shim, acceptance]
---

# lark-cli-shim 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-17
> 关联方案 doc：`.codestable/features/2026-05-17-lark-cli-shim/lark-cli-shim-design.md`

## 1. 接口契约核对

对照方案第 2.1 节名词层：

**行为契约（CLI 调用形态）逐项核对**：

- [x] **透明性**：`pump` 内 `dst.write_all(chunk)` 透传 4 KiB chunk，head buffer 是副本不替代 tee → 代码：`shim.rs:110-131`；集成测试 `non_interactive_writes_full_entry` 断言 `out.stdout` 含 `om_int_abc` ✓
- [x] **exit code 透传**：`run_non_interactive` 返 `status.code().unwrap_or(1)`；main 用 `rc_to_exitcode(rc)` 转 `ExitCode`，setup 失败固定 127 → 代码：`shim.rs:159, 295-301`；集成测试 `exit_code_passthrough`（rc=7）+ `missing_env_returns_127` ✓
- [x] **journal 副作用**：每次调用 main 末段 `journal.append(&entry)`，写失败 `tracing::warn!` 不影响 exit → `shim.rs:289-292` ✓
- [x] **interactive 直通**：`is_interactive(&sub_argv)` 命中 → `CommandExt::exec()` 替换当前进程 → `shim.rs:248-265` ✓

**内部辅助类型逐项核对**：

| 类型/函数 | 设计签名 | 代码落点 | 一致性 |
|---|---|---|---|
| `ENV_REAL_CLI: &str` | 常量 | `shim.rs:17` | ✓ |
| `ENV_NOJOURNAL: &str` | 常量 | `shim.rs:18` | ✓ |
| `INTERACTIVE_VERBS: &[&str]` | `&["auth"]` | `shim.rs:19` | ✓ |
| `STDOUT_HEAD_CAP: usize` | 64 KiB | `shim.rs:20` | ✓ |
| `STDERR_HEAD_CAP: usize` | 16 KiB | `shim.rs:21` | ✓ |
| `enum ShimError` | 4 变体 + thiserror | `shim.rs:23-36` | ✓（含 `JournalFailed` 但实际改为 tracing::warn 不向 caller 传播，保留变体作为 future-proof） |
| `resolve_real_cli() -> Result<PathBuf, ShimError>` | env + canonicalize + anti-recursion | `shim.rs:40-59` | ✓ |
| `is_interactive(&[String]) -> bool` | 三段式 | `shim.rs:64-79` | ✓ |
| `run_non_interactive(&Path, &[String]) -> io::Result<(i32, Vec<u8>, Vec<u8>, u64)>` | 2 pump thread + std::process | `shim.rs:135-162` | ✓ |
| `build_entry(&[String], Outcome) -> JournalEntry` | 11 字段 + extras 进 params | `shim.rs:166-227` | ✓ |
| `enum Outcome { Full, Skipped }` | 两变体 | `shim.rs:94-105` | ✓ |
| `fn main() -> ExitCode` | 整合 | `shim.rs:229-293` | ✓ |

**流程图核对**（第 2.2 节 mermaid）：

- [x] `main → resolve_real_cli → is_interactive → execv 分支 / NOJOURNAL 分支 / run_non_interactive 主路径` 5 个节点在 `main` 中 1:1 落地（`shim.rs:229-293`）
- [x] `run_non_interactive` 内部：`Command → spawn → 2 thread pump → wait → join → 返 tuple` 6 节点在 `shim.rs:142-161` 落地
- [x] `pump` 伪代码：`loop { read; if empty break; write; if head<cap extend }` 在 `shim.rs:117-129` 落地

**无偏离**。

## 2. 行为与决策核对

**需求摘要（第 1 节 summary）逐项验证**：

- [x] 独立 bin `bin/shim`：`Cargo.toml [[bin]] name = "shim"` + `src/bin/shim.rs`
- [x] PATH-prefix 透传 + 流式 tee：`run_non_interactive` 用 `Stdio::piped()` + `std::thread` pump
- [x] 写 JournalEntry：main 末段 `journal.append`
- [x] TTY/interactive 走 execv：`is_interactive` true → `CommandExt::exec()`
- [x] anti-recursion：`resolve_real_cli` canonicalize 比对 current_exe
- [x] NOJOURNAL env：main 检 `ROOSTERY_NOJOURNAL=1` → `Outcome::Skipped { reason: "nojournal" }`
- [x] std::thread + std::process（不上 tokio）：grep `tokio` in shim.rs → 无
- [x] 仅填 §4.2 11 字段：`build_entry` 用 `JournalEntry::new` + 仅赋值 `params` / `duration_ms` / `result`，不新增字段，extras 全在 params

**明确不做（第 1 节 + 第 3 节反向核对）grep 验证**：

- [x] `grep -E "use tokio\|tokio::\|#\[tokio::main\]" shim.rs` → 无
- [x] `grep "LarkRunner\|LarkCli\|Journaled" shim.rs` → 无
- [x] `grep "FEISHU_HUB_" shim.rs` → 无
- [x] `grep -E "use nix\|use libc\|^nix \|^libc " Cargo.toml` → 无新增
- [x] `grep "Config\|cfgmod\|toml::" shim.rs` → 无
- [x] `grep -E "fn retry\|retries\|backoff" shim.rs` → 无
- [x] `grep -E "serde_json::from_str\|::from_slice" shim.rs` → 无
- [x] `grep "INTERACTIVE_VERBS" shim.rs` → 3 处（1 常量 + 1 prod 使用 + 1 test-only 镜像）。design 写"== 1（常量定义）"指代"单一定义来源"精神（无重复常量定义）；消费处属于使用，未违反精神 ✓
- [x] `wc -l shim.rs` = 521；产品代码 307 < 400 档 1 单文件预算 ✓（用户在 implement 阶段确认放过总 LOC，因为超出部分皆为内联单测）

**关键决策（第 1 节 D1-D13）落地**：

| # | 决策 | 代码体现 |
|---|---|---|
| D1 | std::thread + std::process 同步 | `shim.rs:153-154`（两个 `std::thread::spawn`），`shim.rs:142-148`（`std::process::Command`） |
| D2 | 不调 LarkRunner trait | grep 验证无引用；shim 直接用 `Command::new` |
| D3 | bin 同 crate `src/bin/shim.rs` | `Cargo.toml:18-20` |
| D4 | 仅读 env `ROOSTERY_REAL_LARK_CLI` | `shim.rs:41-42`；不设报错退 127 验证 in `missing_env_returns_127` |
| D5 | Interactive 三段式 | `shim.rs:64-79`（TTY + verb + 3 flag） |
| D6 | execv 替换进程 | `shim.rs:260-263` `CommandExt::exec()` |
| D7 | Anti-recursion canonicalize | `shim.rs:47-57` |
| D8 | NOJOURNAL=1 写 skipped 标记 | `shim.rs:267-285`（"1" 严格匹配；其他值视为未启用） |
| D9 | Head buffer caps 64/16 KiB 常量 | `shim.rs:20-21`，测试 `run_head_caps` 验证 |
| D10 | JournalEntry 字段映射（source=shim / action=lark-cli:{verb} / params 6 子字段 / result Ok\|Err / duration_ms） | `build_entry` `shim.rs:166-227` |
| D11 | Skipped 形态（action 后缀 :skipped / reason / Ok Null / duration_ms=0） | `shim.rs:215-225` |
| D12 | ShimError thiserror 4 变体 | `shim.rs:23-36` |
| D13 | setup 失败 127 / real cli 退码透传 | main `shim.rs:235, 280, 293` + `rc_to_exitcode` |

**编排层"现状 → 变化"核对**：装机后 `~/.local/bin/lark-cli` → shim → execv/run_non_interactive → real lark-cli。装机由 Phase 3 `roostery init` 提供（本 feature 范围外，仅保证 shim 自身行为）。

**流程级约束（第 2.2 节"不变量"）逐项核对**：

| 不变量 | 守护方式 |
|---|---|
| 1 透明性 | pump 透传 4 KiB chunk；`run_happy_and_exit_passthrough` 断言 byte-equal `b"hello\n"` |
| 2 exit code 透传 | `run_non_interactive` 返 `status.code()`；setup 失败固定 127；`exit_code_passthrough` 测试 |
| 3 interactive 用 exec() | `shim.rs:260-263`；exec 调用返回即视为失败 + 退 127 |
| 4 anti-recursion 强制 | `resolve_real_cli` 内嵌；`resolve_recursion_detected` 测试 |
| 5 NOJOURNAL 仍跑 real + tee，只跳完整 entry | main 在 nojournal 分支仍走 `run_non_interactive` 再 build skipped entry；`nojournal_writes_skipped_entry` 测试断言 stdout 内容 + entry 形态 |
| 6 journal 写失败 warn 不影响 rc | `shim.rs:289-292` + `shim.rs:255-258`；`tracing::warn!` 不传播 |
| 7 pump 写 dst 失败 silent | `shim.rs:122-123` `let _ = dst.write_all(...)` |
| 8 head 超 cap 后继续 tee 不扩 head | `shim.rs:125-129` 显式 `if head.len() < cap` 守护 |

**挂载点反向核对（第 2.3 节）+ 沙盘推演**：

| # | 挂载点 | grep 验证 |
|---|---|---|
| 1 | `crates/roostery/src/bin/shim.rs` 存在 | `ls crates/roostery/src/bin/shim.rs` → 存在 ✓ |
| 2 | `Cargo.toml` 含 `[[bin]] name = "shim"` 段 | `grep -A1 'name = "shim"' Cargo.toml` → 存在 ✓ |
| 3 | main 调用 `journal::Journal` + `redact::scrub_argv` + `remoterefs::extract` | `grep "Journal::default\|redact::scrub_argv\|remoterefs::extract" shim.rs` → 3 处全命中 ✓ |
| 4 | `ENV_REAL_CLI = "ROOSTERY_REAL_LARK_CLI"` 字符串常量 | `shim.rs:17` ✓ |
| 5 | `CommandExt::exec()` 在 interactive 路径 | `grep "CommandExt\|\.exec()" shim.rs` → `shim.rs:259-262` ✓ |

**反向核查（grep 本 feature 在代码里的所有引用）**：

- `grep -rn "ROOSTERY_REAL_LARK_CLI\|ROOSTERY_NOJOURNAL" crates/ tests/` → 仅 `shim.rs` + `tests/shim_integration.rs` 命中，无第三方文件引用 ✓
- `grep -rn 'bin/shim\|name = "shim"' crates/` → `Cargo.toml` + 代码注释，无第三方文件依赖 ✓

**拔除沙盘推演**：

- 删 `src/bin/shim.rs` + `Cargo.toml [[bin]]` 段 → `cargo build` 仅剩 `roostery` bin；`lib` 模块不变（journal/redact/remoterefs/lark_cli 不依赖 shim）；agent runtime 调 `~/.local/bin/lark-cli` 会找不到（但那是 Phase 3 装机点，不本 feature 拔除范围）→ 拔除后无残留 ✓

**遗留**：无。

## 3. 验收场景核对

对照方案第 3 节关键场景清单，逐条可观察证据验证：

**Setup 失败路径**

- [x] **S1.1** env 未设 → 退 127 + stderr 含 "ROOSTERY_REAL_LARK_CLI not set"
  - 证据：单测 `resolve_missing_env`（`shim.rs` tests）+ 集成 `missing_env_returns_127` 断言 `out.status.code() == Some(127)` 且 stderr 含 env 名 ✓
- [x] **S1.2** real cli 不存在 → 退 127 + stderr 含 "not found"
  - 证据：单测 `resolve_nonexistent_path`（`/definitely/not/here/lark-cli`） ✓
- [x] **S1.3** anti-recursion → 退 127 + stderr 含 "resolves to shim itself"
  - 证据：单测 `resolve_recursion_detected`（指 current_exe，canonicalize 同路径） ✓

**Interactive 直通路径**

- [x] **S2.1** TTY 检测：`is_interactive` 在 `IsTerminal` 命中时返 true
  - 证据：`is_interactive` 函数实现 `shim.rs:65-67`；集成测试用 `Stdio::null()` 走 false 分支间接覆盖
- [x] **S2.2-S2.4** verb / flag truth table
  - 证据：单测 `is_interactive_truth_table`（5 case：auth-verb / --interactive / -i / --repl / 无命中） ✓
- [x] **S2.5** Interactive 写 "skipped: interactive" entry
  - 证据：单测 `build_entry_skipped_schema` + main `shim.rs:248-258`（写 entry 后 exec） ✓

**非交互流式 pump**

- [x] **S3.1** Happy：fixture echo hello + err >&2 + exit 0 → 用户收到 "hello\n" / "err\n" / 退 0
  - 证据：单测 `run_happy_and_exit_passthrough` 断言 byte-equal `(0, b"hello\n", b"err\n")` ✓
- [x] **S3.2** Exit code 透传：fixture exit 42 → shim 退 42
  - 证据：同上单测 + 集成 `exit_code_passthrough`（rc=7） ✓
- [x] **S3.3** Head buffer cap：fixture 200 KiB → 用户全收，journal 仅 64 KiB
  - 证据：单测 `run_head_caps` 断言 `out.len() == STDOUT_HEAD_CAP` ✓
- [x] **S3.4** Broken pipe tolerance：pump 不 panic / 不阻塞
  - 证据：code-level，`shim.rs:122-123` `let _ = dst.write_all(...)`；间接守护（未触发 panic 验证） ✓

**NOJOURNAL 路径**

- [x] **S4.1** `NOJOURNAL=1` + 非交互 → 仍跑 + tee + entry 形态 ":skipped"+"reason: nojournal"
  - 证据：集成 `nojournal_writes_skipped_entry` 断言 action="lark-cli:docs:skipped" + params.reason="nojournal" + stdout="hello" ✓
- [x] **S4.2** `NOJOURNAL=0` / 未设 → 写完整 entry
  - 证据：集成 `non_interactive_writes_full_entry`（`env_remove("ROOSTERY_NOJOURNAL")`）+ 严格 `matches!(Ok("1"))` 排除 "0" / "true" / "yes" ✓

**Journal entry schema 锁定**

- [x] **S5.1** 完整 entry 11 字段：source / action / params(argv+cwd+stdin_present+stdout_head+stderr_head+remote_refs) / result(Ok refs \| Err NonZeroExit) / duration_ms / schema_version=1
  - 证据：单测 `build_entry_full_schema_locked` 逐字段断言 + 集成测试断言 jsonl 反序列化后的 6 个 params 子字段 + result.outcome ✓
- [x] **S5.2** Skipped entry：action 后缀 :skipped + params.reason + result Ok Null + duration_ms=0
  - 证据：单测 `build_entry_skipped_schema` + `build_entry_edge_cases` ✓
- [x] **S5.3** Empty argv：sub_argv=[] → action="lark-cli:<empty>" 不 panic
  - 证据：单测 `build_entry_edge_cases` ✓

**Redact / remoterefs 集成**

- [x] **S6.1** argv 含 sensitive → params.argv 对应位置 "***"
  - 证据：单测 `build_entry_edge_cases`（`["im","--access-token","xyz","send"]` → `scrubbed[2]=="***"`） ✓
- [x] **S6.2** stdout 含 message_id → params.remote_refs.message_id 填充
  - 证据：单测 `build_entry_full_schema_locked`（stdout 含 `om_abc` → `value["message_id"]=="om_abc"`）+ 集成 `om_int_abc` ✓
- [x] **S6.3** stdout 含 sensitive → params.stdout_head 经 scrub_text
  - 证据：`build_entry` 在 `shim.rs:182-183` 调 `redact::scrub_text`；redact 模块自带 11 个 SENSITIVE_KEYS（间接守护，未单测覆盖具体 token 字符串）

**模块级**

- [x] **S7.1** `cargo test --all` 全绿：106 lib + 12 shim unit + 4 shim integration + 3 + 4 doc → 全 ok
- [x] **S7.2** `cargo test --doc` 全绿：本 feature 不引入新 doctest，已有 4 doc 维持通过
- [x] **S7.3** `cargo clippy --all-targets --all-features -- -D warnings` 通过
- [x] **S7.4** `cargo fmt --all --check` 通过
- [x] **S7.5** 架构红线 grep：`grep "LarkRunner\|LarkCli\|Journaled" crates/roostery/src/bin/shim.rs` → 无 ✓

前端改动：本 feature 无 UI 改动。

## 4. 术语一致性

对照方案第 0 节 + §2.1 命名 grep 代码：

| 术语 | 期望命中 | 实际 | 一致 |
|---|---|---|---|
| `shim` 二进制 | 文件名 + `[[bin]] name = "shim"` | 命中 | ✓ |
| `ROOSTERY_REAL_LARK_CLI` | 常量 `ENV_REAL_CLI` + 字符串 | 常量定义 + 引用 | ✓ |
| `ROOSTERY_NOJOURNAL` | 常量 `ENV_NOJOURNAL` + 字符串 | 常量定义 + 引用 | ✓ |
| `Interactive 直通路径` | `is_interactive` + execv | `is_interactive` fn + `CommandExt::exec()` | ✓ |
| `Streaming pump` | `pump` fn + `std::thread::spawn` | `pump<R,W>` + 2 spawn | ✓ |
| `Head buffer` | `STDOUT_HEAD_CAP` / `STDERR_HEAD_CAP` + 局部 `head` | 命中 | ✓ |
| `Anti-recursion check` | `resolve_real_cli` 内 canonicalize 比较 | 命中 | ✓ |
| `ShimError` | enum 4 变体 | 命中 | ✓ |
| `Outcome::Full / Skipped` | enum 两变体 | 命中 | ✓ |

**防冲突**：

- `grep "FEISHU_HUB_" crates/roostery/src/bin/shim.rs` → 无 ✓
- `grep "LarkRunner" crates/roostery/src/bin/shim.rs` → 无 ✓
- `grep "shim" crates/roostery/src/lark_cli/ crates/roostery/src/journal.rs` → 仅文档注释提及无运行时依赖 ✓

无不一致。

## 5. 架构归并

对照方案第 4 节：

- [x] **`ARCHITECTURE.md §2 术语表`** — 加 `shim 二进制` / `ROOSTERY_REAL_LARK_CLI` / `ROOSTERY_NOJOURNAL` 三条词条，注明与 `LarkRunner` / `Journaled` 的 I/O 模型差异
- [x] **`ARCHITECTURE.md §3 Module C`** — Module C 节加 shim 子节描述（bin target + 流式 tee + interactive execv + anti-recursion + 与 LarkRunner / Journaled 的关系/区别 + commit 引用）；末尾 "子 feature" 列表更新 shim 状态 done
- [x] **`ARCHITECTURE.md §5 关键架构决定`** — 加一条"shim 走 streaming bytes 模型 + std::thread；LarkRunner 走 buffered Value 模型 + tokio。两条路径独立维护不强行抽公共 trait"
- [x] **`ARCHITECTURE.md §6 已知约束`** — §6 第 1 条"禁止重实现 lark-cli"加 shim 装机端兑现链说明（agent runtime → PATH-prefix shim → real lark-cli）
- [x] **`.codestable/requirements/portable-by-default.md`** — 变更日志加 shim 落地条目；status 保持 `draft`（read/replay 工具未落地）
- [x] **`.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`** — `lark-cli-shim` status `in-progress` → `done`
- [x] **`.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §3 第 7 项** — `lark-cli-shim` 状态 `planned` → `done`，加 feature 引用
- [ ] `.codestable/attention.md` — 已有"shim 装机到 `~/.local/bin/lark-cli`"条目覆盖装机点约定，本 feature 未暴露新的硬约束（决定不写入）

判据自检：未读 design 的人打开 ARCHITECTURE.md §3 Module C + §5 第 7 条，能知道"系统里现在有 shim 二进制承担装机端 lark-cli 透传 + journal 写入，与 LarkRunner 走两条独立 I/O 模型路径"。

## 6. requirement 回写

- 方案 frontmatter `requirement: portable-by-default`（draft status）
- 本 feature 兑现了"每次 lark-cli 调用都进本地 journal"装机端链路；但 req 的核心 acceptance 条件（read/replay 工具）仍未落地
- 处理方式：**update** — 仅在 portable-by-default.md 变更日志追加一条 shim 落地记录，status 保持 `draft`、用户故事 / 边界 / pitch 不改

## 7. roadmap 回写

- 方案 frontmatter `roadmap: rust-rewrite` / `roadmap_item: lark-cli-shim`，两者均有值
- `rust-rewrite-items.yaml` 第 53-59 行 `slug: lark-cli-shim` 当前 `status: in-progress` `feature: 2026-05-17-lark-cli-shim` → 改 `status: done`
- `rust-rewrite-roadmap.md` 第 407 行子 feature 清单 `lark-cli-shim` 状态 `planned`（design 阶段未同步） → 改 `done` + 加 feature 引用
- 已用 `python3 .codestable/tools/validate-yaml.py` 校验 items.yaml

## 8. attention.md 候选盘点

判据：每个 feature 都会撞一次的环境 / 工具 / 工作流类信息。

**回看本次实现可能触发的候选**：

- `ROOSTERY_REAL_LARK_CLI` 必须在 shim 启动时可见 → **不写入**：该 env 由 Phase 3 `roostery init` 负责注入，是 init feature 的输出契约不是开发期注意事项
- `~/.local/bin/lark-cli` PATH-prefix 装机点 → **不写入**：`.codestable/attention.md` "命令与脚本陷阱" 节已有"`lark-cli` shim 安装到 `~/.local/bin/lark-cli`（PATH-prefix shim 透传 + 写 journal）；要求 `~/.local/bin` 在 PATH 前段才能拦截到真 `lark-cli`，`roostery init` 会校验"条目，本 feature 实现了该条目早就 anticipated 的二进制
- `Cargo bin target` 单文件约定（design §2.5 暂定 convention）→ **不写入 attention.md**，归属 cs-decide convention 候选（见退出后第 2 点）
- 测试中 `std::fs::write(path)` 避免 ETXTBSY race → **已存在**：`.codestable/attention.md` "测试" 节已有该条目

**结论**：无新增 attention.md 候选。

## 9. 遗留

- **后续观察项**（不阻塞，已在 design §4.1 记录）：
  - Phase 3 `roostery init` 的装机协议（如何写 env、如何放 shim 到 `~/.local/bin/lark-cli`）
  - bin 二进制 size 优化（同 crate transitively 引入 tokio；release LTO 应能 strip；若 > 5 MB 走档 3 独立 crate）
  - `interactive_verbs` 扩展（Phase 3 config 起来后由配置驱动；本 feature 硬编码 `["auth"]` 最小集）
  - stdin 透传细节（如有 lark-cli 子命令对 stdin 有特殊期望届时由 verb 扩展处理）
- **顺手发现**：无
- **已知偏差**：
  - 设计 §3 反向核对 `wc -l shim.rs < 400` 实际 521（产品 307 + 测试 214）。用户在 implement 阶段确认放过，因超出部分皆为内联单测，产品代码守住档 1 单文件预算
  - 设计 §3 反向核对 `grep INTERACTIVE_VERBS shim.rs == 1` 实际 3 处（1 常量 + 1 prod + 1 test）。design 意图是"单一定义来源"，实际未违反精神
