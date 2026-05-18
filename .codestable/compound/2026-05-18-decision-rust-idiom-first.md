---
doc_type: decision
category: convention
slug: rust-idiom-first
status: active
created: 2026-05-18
tags: [rust, idiom, code-quality, python-port, backlog]
---

# Rust idiom first — 新 Rust 代码不做 Python 1:1 翻译

## 背景

Roostery 自 2026-05-15 起 Rust 重写。前 7 个 feature（rust-scaffold / core-redact / journal-core / core-remoterefs / lark-cli-wrapper / lark-cli-shim / roostery-smoke）在引入 Rust idiom 上做得相对好——`thiserror` enum + `#[non_exhaustive]` + newtype + `Result` 分类、`std::process` + `std::thread` 同步替代 Python `threading + Popen`、`include_str!` + serde derive。

但 Phase 3 进入 Module D 后**走样了**：

- `2026-05-17-config-yaml`：runners 字段 `BTreeMap<String, serde_yml::Value>`完全无类型——design 阶段选择"开放结构"，但已知字段（enabled）也没用 `#[serde(flatten)]` 给类型守护
- `2026-05-18-hooks-merge`：基本是 Python `hooks_merge.py` 148 行的 1:1 翻译——`serde_json::Value` 字符串 indexing 满天飞，`.as_object_mut().unwrap()` 已 `.is_object()` 检过又 unwrap 多处，`HooksError::FragmentInvalid { reason: String }` 是 Python `raise ValueError(str)` 思维，agent runtime 标识"cc"/"codex"是字符串字面量散落，shell bridge `agent_stop_notify.sh` 是 Python "调 sh 调 python" 桥接的纯翻版

用户反馈："Python 的坏处你全学进去了，但是 Rust 的好却没凸显"。这话准。

## 决定

**新 Rust 代码强制 Rust idiom**——design 阶段把"用了哪些 Rust 杠杆"明确列入决策项（D1-Dn 中），不让 implement 自己默默选 Python 1:1 翻译。**已落地代码维护一份 backlog**，在 release cycle 间或 Phase 5 收尾时一次性 `cs-refactor`，不阻塞当前功能开发。

### 新代码硬约束

design 阶段必须显式回应以下 6 条（即使结论是"本 feature 不适用"也要写出来，不能默认略过）：

1. **强类型 schema vs 无类型 `Value`**——能 serde derive 成 struct 的就别留 `Value`。`#[serde(flatten)]` 处理"已知字段 + 任意扩展"的混合需求
2. **error 变体颗粒度**——具体错误情境一种一个 enum 变体，不混 `String reason`。caller `match` 时编译器穷尽守护
3. **newtype 隔离**——业务标识符（agent runtime kind / matcher 字符串 / env key 等）该 newtype 就 newtype，别让字符串字面量满代码飞
4. **类型态（typestate）**——"validate 过的" vs "raw" 用类型参数区分，避免运行时再 panic
5. **零拷贝 + 借用优先**——`&str` / `&Path` / `Cow<'static, str>` 优先，能不 `to_string()` / `clone()` 就别做
6. **能用编译期就别运行时**——`include_str!` / `const fn` / `static` parsed-once 优先于 disk read

design 阶段任一条选了 "本 feature 不适用"要给具体理由（如"runners 字段 roadmap §4.6 钉死开放结构，不适用第 1 条"）。

### 已落地 backlog（cs-refactor 跟踪）

按"返修代价 / 收益比"从高到低排序：

| # | 模块 | 问题 | Rust idiom 化方向 | 难度 | 状态 |
|---|---|---|---|---|---|
| B1 | `hooks_merge::merge_event_hook` | `serde_json::Value` 字符串 indexing + `.unwrap()` 满天飞 | parse fragment 为强类型 `HookFragment { event: EventKey, matchers: Vec<MatcherEntry> }` + serde derive；merge 在强类型层做完再 serialize | 中 | **done** @ `42d8b98`（fragment 侧全程强类型；target 文件侧保留 `Value` 以保留用户未知键原状） |
| B2 | `hooks_merge::HooksError::FragmentInvalid { reason: String }` | 单变体打包多种 invalid，caller 只能 string match | 拆 `NoEventKey` / `MultipleEventKeys { found }` / `EmptyMatcherArray { event }` / `MissingCommand { matcher }` 等独立变体 | 低 | **done** @ `42d8b98`（拆 `FragmentError` 9 变体 `#[non_exhaustive]`，`HooksError::Fragment(#[from])` 透传） |
| B3 | `hooks_merge::command_tail` | `&str` 字符串外科手术剥 env 前缀 | 引入 `struct HookCommand { env_prefix, script, args }` + parser；script 是 `PathBuf` 不是 `&str` | 中 | **dropped** @ `42d8b98`（7 行小函数已足够 idiomatic，promote 到 struct 是 ceremony 无安全收益；保留观察，若 dispatcher 阶段需 parse 完整 command 再重启） |
| B4 | agent runtime 标识 "cc" / "codex" | 字符串字面量散落 templates + 测试 + 未来 dispatcher | `enum AgentKind { Cc, Codex }` + `Display` + `FromStr` + `serde` derive；模板用 `concat!` 或运行时拼接而非硬编码字符串 | 低 | **done** @ `42d8b98`（`pub enum AgentKind { Cc, Codex }` `#[non_exhaustive]` + `Display`/`FromStr`/`serde lowercase` + `template()` 选模板；模板文件保留字符串字面量作 byte-for-byte JSON 资源） |
| B5 | `agent_stop_notify.sh` 桥接 | Python "sh 调 python" 桥接的纯翻版；引 jq / bash 依赖 | Phase 5 `bot-stop-hook` 起来时 dispatcher 直接 parse hook stdin JSON 替代 sh（design 已 flag）。**不在 hooks-merge 范围**，由 Phase 5 feature 自然吞掉 | 高（实际归 Phase 5） | **deferred** → Phase 5 `bot-stop-hook` feature 自然替换 |
| B6 | `config::Config.runners: BTreeMap<String, serde_yml::Value>` | 已知字段（enabled / cli_path / extra_args）无类型守护 | 改 `BTreeMap<String, RunnerConfig>` + `RunnerConfig { enabled: bool, #[serde(flatten)] extra: BTreeMap<String, Value> }`——已知字段强类型，开放结构走 flatten | 低-中 | **done** @ `42d8b98`（`RunnerConfig { enabled, #[serde(flatten)] extra }`；提前于 Phase 4 完成，因 hooks_merge 同期返修 cost 边际为零） |
| B7 | `smoke::ProbeResult` / `SmokeReport` 字段 `head / reason / lark_cli_version` 都用 `Option<String>` | 信息丰富但类型粒度粗 | `head` 改 `Option<HeadBuffer>`（newtype 含 capped 行为）；`reason` 拆 enum；`lark_cli_version` 解析成 `semver::Version` | 低（但收益也低）| **open**（优先级最低，下次有 smoke 相关 feature 触碰时顺手做） |

**当前状态**（2026-05-18 sweep 后）：

- B1 / B2 / B4 / B6 已 done（commit `42d8b98`）
- B3 dropped（结论：函数太小，typestate 化得不偿失）
- B5 deferred → Phase 5
- B7 open，优先级最低

**剩余 open 项**：仅 B7。新 feature 若触碰 smoke 模块即合并做掉；否则等 0.1.0 release 节点统一评估。

### 触发返修的时机

不开"返修专项 release cycle"——按以下规则触发：

1. **新 feature 触碰到 backlog 项**：例如 roostery-init 要用 `AgentKind` 就一并做 B4
2. **下游 caller feature 落地时**：例如 Phase 4 dispatcher-runners 起来时一并做 B6
3. **release 节奏点**（0.1.0 触发判据 = `bot-stop-hook` 完成时）：盘点剩余 backlog 评估是否一次清掉

## 为什么这样选

1. **不阻塞当前进度**——roostery-init 是 Phase 3 milestone 收尾 feature；停下来反向修 4 个已落地 feature 会推迟 user-facing 装机能力。Backlog 显式记录避免"以后改"变"永不改"
2. **新代码立规矩成本最低**——design 阶段加 6 条 idiom checklist 是单次决策成本；implement 阶段返修是反复改 同一文件 + 测试连锁的复合成本
3. **后人 git blame 友好**——`.unwrap()` 满天飞如果不显式记录"这是已知技术债"，下个 feature 的 AI 会以为这是项目风格 → 复制扩散
4. **可发现性**——本 decision 通过 `search-yaml.py --filter doc_type=decision` 能查到；新 feature 的 cs-feat-design 启动时扫 compound 会读到这条 → 自动启用 6 条 idiom checklist

## 考虑过的替代方案

| 方案 | 为什么没选 |
|---|---|
| **立即停下当前进度，开"返修 sprint"清完 4 feature 的 Python 翻译痕迹** | Phase 3 milestone（init 落地后陌生开发者能装机）拖延代价远高于代码风格统一；返修期间没有功能产出 |
| **从下一个 feature 起新代码强制 idiom，已落地代码"不动"，无 backlog 文档** | 已落地代码会成为"惯性模板"——下个 feature 的 AI 读 `hooks_merge.rs` 的 `.unwrap()` 满天飞会以为是项目风格而复制；没 backlog 记录"以后改"等于"永不改" |
| **每个 feature acceptance 阶段强制返修一处 backlog 项** | 把功能 feature PR 稀释成"功能 + 返修"综合改动，违反 cs-feat-impl 的"只做 design 范围内事" 原则 |
| **在 architecture/ARCHITECTURE.md §6 加一条硬约束 "新代码强制 Rust idiom"** | architecture 是稳定文档，本规约更像 evolving convention；放 compound 更准（且 cs-feat-design 自动扫 compound） |

## 影响 / 后续约束

- **feature design §1 决策表强制项**：新 feature design 第 1 节关键决策表必须显式列上面 6 条 idiom 中每条的应用情况（或写"本 feature 不适用，原因：……"）。这是**新增的 design 阶段 gate**
- **新代码 grep 守护**：feature acceptance 阶段反向核对项加 `grep -E "as_object_mut\(\)\.unwrap\(\)|as_array_mut\(\)\.unwrap\(\)" {新文件}` → 期望无（除非 design 明示放过）
- **backlog 管理**：本 decision 的 backlog 表是 single source of truth；新 feature 触碰到某项后更新表（划掉 / 加 commit ref）
- **审视周期**：每个 release cycle 节点（0.1.0 / 0.2.0 / ...）回看 backlog 评估剩余项

## 相关文档

- `.codestable/features/2026-05-18-hooks-merge/hooks-merge-design.md`：触发本 decision 的 feature（Python 1:1 翻译最严重的一例）
- `.codestable/features/2026-05-17-config-yaml/config-yaml-design.md`：B6（runners 无类型）来源
- `.codestable/compound/2026-05-16-decision-rust-module-organization.md`：本 decision 关注**代码内部**风格（idiom），rust-module-organization 关注**代码组织**（文件 / 目录 / crate）；两者维度互补
- `.codestable/compound/2026-05-16-decision-business-identifier-newtype.md`：newtype 隔离已有专项 decision；本 decision 的 6 条 idiom 中第 3 条 newtype 隔离与其呼应
