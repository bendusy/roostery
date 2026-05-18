---
doc_type: feature-design
feature: 2026-05-18-hooks-merge
roadmap: rust-rewrite
roadmap_item: hooks-merge
requirement: agent-work-in-feishu
status: approved
summary: hooks_merge 模块——JSON 深合并把 Stop hook 片段注入 `~/.claude/settings.json` / `~/.codex/hooks.json`，幂等去重（按 event key + matcher + command 尾匹配）；模板 const 用 `include_str!` 嵌入 3 个文件（cc_stop_hook.json / codex_stop_hook.json / agent_stop_notify.sh）；env 前缀切到 `ROOSTERY_AGENT=cc|codex`（不沿用 Python `FEISHU_HUB_AGENT`，文档另有规定）；render_template 用 `{{HOOK_SCRIPT}}` 字符串替换；JSON 输出 indent=2 + trailing newline；atomic .tmp + rename；本 feature 纯 lib 扩展无 CLI 变更（roostery-init 下个 feature 才调）
tags: [phase-3, module-d, hooks, json-merge, templates, include_str]
---

# hooks-merge design

## 0. 术语约定

| 术语 | 定义 | 防冲突结论 |
|---|---|---|
| Hook fragment | 单条 hook 配置 JSON 片段（含 `hooks.<event>[0]` 数组）；template 渲染产物 | 新概念；与 `JournalEntry` 不冲突 |
| Event key | hook fragment 顶层 `hooks` 对象下的事件名（如 `SessionEnd` / `Stop`）；本 feature 自动探测 fragment 唯一 event key 不写死 | 与 lark_cli 的 `Event` 命名空间不冲突（后者是飞书侧概念） |
| Matcher | hook 数组每项的 `matcher` 字符串（如 `"*"` / `"after-tool-use"`）；hooks-merge 按 matcher 找同 bucket | 新概念 |
| Command tail | hook command 字符串去掉 `KEY=VAL` env 前缀后剩余部分；用作去重 key（Python parity） | 新概念 |
| Template | `crates/roostery/src/templates/` 下的 `.json` / `.sh` 文件，用 `include_str!` 编译期嵌入；含 `{{HOOK_SCRIPT}}` 占位符 | roadmap §4.7 钉死；与 `legacy/python/.../templates/` 是 reference 关系 |
| `HOOK_SCRIPT` 占位符 | template 里被 `render_template` 替换为实际脚本绝对路径的标记 | 与 attention.md 既有 env 词条不冲突 |
| `ROOSTERY_AGENT` env | Stop hook command 拼前缀 `ROOSTERY_AGENT=cc` / `=codex` 让下游 stop hook handler 识别 runtime；**不沿用** Python `FEISHU_HUB_AGENT`（一次切口径） | 新概念；与 `ROOSTERY_HOME` / `ROOSTERY_LARK_CLI_BIN` 同 prefix 风格 |
| `HooksError` | thiserror enum `#[non_exhaustive]` 4 变体：ReadFailed / ParseFailed / FragmentInvalid / SaveFailed | 与 `ConfigError` / `SmokeError` / `LarkError` / `ShimError` 平行 |

参考：`legacy/python/src/roostery/hooks_merge.py`（148 行）+ `templates/{claude_code_settings,codex_hooks,agent-stop-notify}.{json.tmpl,sh}`——行为 reference；模板字符串拷贝时 env 前缀切到 `ROOSTERY_AGENT`，sh 中 `python3 -m roostery.stop_hook` 替换为 `roostery dispatcher fire` 形态（Phase 4 dispatcher 起来后正常工作；本 feature 期间 hook 触发会拿到 clap "unknown subcommand" 错误，`|| true` 吞掉不影响 agent runtime）。

### 0.1 Rust idiom 杠杆

1. **`include_str!` 编译期嵌入模板**——roadmap §4.7 钉死；模板放 `src/templates/` 子目录 + 顶层 `pub const` 暴露
2. **`serde_json::Value` 深合并**——比 Python 手写 `_deep_merge` 自然；类型由 serde 管
3. **`#[derive(thiserror::Error)] #[non_exhaustive]` `HooksError`** 4 变体——同 ConfigError 风格
4. **atomic write `.tmp` + `std::fs::rename`**——同 config / smoke / shim 项目惯例
5. **`replace("{{HOOK_SCRIPT}}", ...)`**——`{{...}}` 字符串替换；不引模板引擎（YAGNI）

### 0.2 与已落地模块的关系

- **`config`**：roadmap depends_on 标 config-yaml，但 hooks-merge **代码不 import config**——roadmap 依赖是规划顺序而非代码耦合。hooks-merge 提供 lib API；caller（下个 feature `roostery-init`）可能用 `cfg.identity.user_id` 拼脚本路径
- **`paths`**：不直接消费；caller 决定 target_path 走 `~/.claude/settings.json` / `~/.codex/hooks.json`（外部路径，不在 `~/.roostery/` 下）
- **`journal`**：hooks-merge 不写 journal（装机操作不是 agent runtime 调用）
- **`redact`**：不消费（hook command 含 env 前缀但都是公开标识不脱敏）
- **`lark_cli` / `smoke`**：完全无关
- **`main.rs`**：本 feature 不动 main.rs（CLI 子命令归 roostery-init feature）

## 1. 决策与约束

### 范围

- 新文件 `crates/roostery/src/hooks_merge.rs`（档 1 单文件，预估 ~350 行含 inline tests）
- 新目录 `crates/roostery/src/templates/` 存 3 个模板文件：
  - `templates/cc_stop_hook.json`（CC SessionEnd hook fragment）
  - `templates/codex_stop_hook.json`（Codex SessionEnd hook fragment）
  - `templates/agent_stop_notify.sh`（共用 stop bridge shell；Phase 5 `bot-stop-hook` 替换为原生 Rust handler 时删除此文件）
- 修改 `crates/roostery/src/lib.rs`——加 `pub mod hooks_merge;`
- 不修改 `Cargo.toml`（`serde_json` 已有）
- 不修改 `paths.rs`（caller 自管 target path）
- 单元测试 ≥ 8 条：render template 占位替换 / merge 空 target / merge 已有不同 event / merge 同 event 不同 matcher / merge 同 matcher 不同 command（追加）/ merge 同 command（去重）/ atomic save / fragment 校验
- 集成测试 ≥ 1 条：用 fixture 走 apply_template 完整路径（template → render → merge → atomic save → load 验证）

### 明确不做

- **不引模板引擎**（tinytemplate / handlebars-rust）——只一个占位符 `{{HOOK_SCRIPT}}` 用 `replace` 即可。grep 反向核对：`grep -E "use tinytemplate|use handlebars|use askama" hooks_merge.rs` → 无
- **不读 Python legacy env**：`FEISHU_HUB_AGENT` / `FEISHU_NOTIFY_TO` 不沿用；模板 env 前缀切到 `ROOSTERY_AGENT`。grep 反向核对：`grep "FEISHU_HUB_" hooks_merge.rs` → 无；`grep "FEISHU_HUB_" crates/roostery/src/templates/` → 无
- **不实现 roostery init 子命令**：本 feature 仅提供 lib API + 模板 const；`roostery init` 子命令 + identity 解析 + agent_detect 归 `roostery-init` feature
- **不消费 config**：caller 决定是否读 config（roostery-init 会）
- **不内置 target_path 默认值**：`~/.claude/settings.json` / `~/.codex/hooks.json` 路径由 caller 传入；本 feature 只暴露模板和 merge 算法
- **不写 journal**：装机时机操作；非 agent runtime 调用
- **不实现 Gemini 模板**：roadmap §4.7 示例仅列 CC / Codex 两个 const；Gemini 推后续 feature。`gemini_settings.json.tmpl` 在 Python 期存在但不本期搬运
- **不做 schema 校验**（如检查 CC settings.json 顶层是否符合 CC schema）：本 feature 只管"合并 hook 片段"；CC / Codex 端 schema 由它们各自负责
- **不实现 unmerge / 卸载 hook**：装机协议是 idempotent append；卸载（拔掉 hook）走人工编辑文件或后续 `roostery uninstall` feature
- **不修改 `legacy/python/`**：frozen

### 复杂度档位

走默认档位——单 lib 模块 + 同步 IO + JSON 操作。无对外 SDK / 高并发 / size-sensitive 信号。

### 关键决策

| # | 决策 | 内容 | 来源 |
|---|---|---|---|
| D1 | 嵌入 3 个模板（sh + 2 JSON） | `templates/cc_stop_hook.json` + `templates/codex_stop_hook.json` + `templates/agent_stop_notify.sh`；每个 `pub const` 暴露 via `include_str!` | 用户对齐 |
| D2 | env 前缀切到 `ROOSTERY_AGENT=cc/codex` | 一次切口径与 vendor-neutral 主基调一致；与 attention.md 既有"Python 期 env 一次切口径"原则一致 | 用户对齐 |
| D3 | JSON 输出 `serde_json::to_writer_pretty` + `\n` 结尾 | Python golden file byte-for-byte（除 env 前缀已明示偏离）；indent=2 默认；trailing newline 方便 git diff | 用户对齐 |
| D4 | `{{HOOK_SCRIPT}}` 字符串替换 | 单占位无需模板引擎；预测性高 | 用户对齐 |
| D5 | atomic write `.tmp` + `std::fs::rename` | 同 config / smoke / shim 项目惯例；caller 不会留半文件 | 项目惯例 |
| D6 | 公开 API：`pub const X3 个` + `render_template` + `merge_event_hook` + `apply_template` | 三层：const（最底层）/ render+merge（中层组合）/ apply_template（一站式入口） | Python parity |
| D7 | `HooksError` thiserror 4 变体 `#[non_exhaustive]` | ReadFailed / ParseFailed / FragmentInvalid / SaveFailed；同 ConfigError 风格 | 项目惯例 |
| D8 | event key 自动探测 fragment | fragment 必须含**恰好 1 个** event key，否则 `FragmentInvalid`；同 Python parity | Python parity |
| D9 | command 去重用"尾匹配" | 去掉前缀 `KEY=VAL` 比较剩余命令；不严格比较 env 部分 | Python parity（M3.E.A 设计）|
| D10 | sh 模板 `python3 -m roostery.stop_hook` 替换为 `roostery dispatcher fire` 形态 | Rust port 自然替换；Phase 4 dispatcher 起来即可工作；Phase 3 期间触发会 clap error 但 sh 末尾 `\|\| true` 吞掉 | Rust port 自然延伸 |
| D11 | 不内置 target_path 默认 | caller 显式传入；本 feature 不假设用户家目录布局 | 范围最小化 |
| D12 | 本 feature 纯 lib 扩展无 CLI 子命令变更 | main.rs 完全不动 | 范围最小化 |

### 前置依赖

- `config-yaml`（done）—— roadmap 依赖关系（规划顺序），代码不 import
- 隐式：`serde_json` 已在 Cargo.toml

## 2. 名词与编排

### 2.1 名词层

**现状**：

- `crates/roostery/src/` 无 hooks_merge 模块；无 `templates/` 子目录
- `crates/roostery/src/lib.rs` 导出 7 pub mod（config / journal / lark_cli / paths / redact / remoterefs / smoke）
- `legacy/python/src/roostery/hooks_merge.py` 148 行参考 + Python `templates/` 4 个 .tmpl 参考
- `Cargo.toml` 含 `serde_json = "1"` 已就绪

**变化**：

- 新增 `crates/roostery/src/hooks_merge.rs`：声明 `pub const` 模板 3 个 + 3 个公开 fn + `HooksError`
- 新增 `crates/roostery/src/templates/cc_stop_hook.json`（CC SessionEnd 模板，含 `ROOSTERY_AGENT=cc` 前缀 + `{{HOOK_SCRIPT}}` 占位）
- 新增 `crates/roostery/src/templates/codex_stop_hook.json`（同上 codex 前缀）
- 新增 `crates/roostery/src/templates/agent_stop_notify.sh`（Rust port 的 stop bridge sh；call `roostery dispatcher fire` 形态）
- `lib.rs` 加 `pub mod hooks_merge;`

**公开 API 接口契约**：

```rust
// crates/roostery/src/hooks_merge.rs

/// CC SessionEnd hook fragment 模板（含 `{{HOOK_SCRIPT}}` 占位）
pub const CC_STOP_HOOK_JSON: &str = include_str!("templates/cc_stop_hook.json");

/// Codex SessionEnd hook fragment 模板
pub const CODEX_STOP_HOOK_JSON: &str = include_str!("templates/codex_stop_hook.json");

/// 通用 stop hook bridge shell 脚本（CC + Codex 共享）
pub const STOP_HOOK_AGENT_NOTIFY_SH: &str = include_str!("templates/agent_stop_notify.sh");

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HooksError {
    #[error("read existing hook file failed: {source}")]
    ReadFailed { #[from] source: std::io::Error },
    #[error("parse existing hook file failed: {source}")]
    ParseFailed { source: serde_json::Error },
    #[error("fragment invalid: {reason}")]
    FragmentInvalid { reason: String },
    #[error("save hook file failed: {source}")]
    SaveFailed { source: std::io::Error },
}

/// 替换模板里的 `{{HOOK_SCRIPT}}` 占位为实际脚本路径，返 parsed JSON Value。
pub fn render_template(
    template_src: &str,
    hook_script: &str,
) -> Result<serde_json::Value, HooksError>;

/// 把 fragment 合并进 target_path 现有 JSON；按 event key + matcher + command tail 幂等去重；
/// 返合并后完整 JSON Value（caller 负责落盘）。
pub fn merge_event_hook(
    target_path: &std::path::Path,
    fragment: &serde_json::Value,
) -> Result<serde_json::Value, HooksError>;

/// 一站式：render template → merge → atomic write；返实际写入路径。
pub fn apply_template(
    template_src: &str,
    target_path: &std::path::Path,
    hook_script: &str,
) -> Result<std::path::PathBuf, HooksError>;
```

**调用示例**（caller 视角，未来 `roostery-init` 使用）：

```rust
use roostery::hooks_merge::{self, CC_STOP_HOOK_JSON, STOP_HOOK_AGENT_NOTIFY_SH};
use std::path::PathBuf;

// init 写 stop bridge 脚本到 ~/.roostery/scripts/agent_stop_notify.sh
let scripts_dir = roostery::paths::roostery_home().join("scripts");
let sh_path = scripts_dir.join("agent_stop_notify.sh");
std::fs::create_dir_all(&scripts_dir)?;
std::fs::write(&sh_path, STOP_HOOK_AGENT_NOTIFY_SH)?;
std::os::unix::fs::PermissionsExt::set_mode(
    &mut std::fs::metadata(&sh_path)?.permissions(), 0o755,
);

// 把 CC stop hook 注入 ~/.claude/settings.json
let cc_settings = PathBuf::from(std::env::var_os("HOME").unwrap())
    .join(".claude/settings.json");
hooks_merge::apply_template(
    CC_STOP_HOOK_JSON,
    &cc_settings,
    sh_path.to_str().unwrap(),
)?;
```

**模板文件形态**（`templates/cc_stop_hook.json` 内容；端口自 Python）：

```json
{
  "hooks": {
    "SessionEnd": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "ROOSTERY_AGENT=cc {{HOOK_SCRIPT}}",
            "timeout": 10
          }
        ]
      }
    ]
  }
}
```

`codex_stop_hook.json` 同形态，`ROOSTERY_AGENT=codex`。

**stop bridge sh 形态**（`templates/agent_stop_notify.sh`，Rust port）：

- 同 Python sh 的 stdin 解析逻辑（jq tail transcript 拿 summary）
- 末尾 `python3 -m roostery.stop_hook ...` → `roostery dispatcher fire --agent "$AGENT" --session ... --cwd ... --summary ...`（Phase 4 dispatcher 起来后正常工作）
- 保留 `|| true` 不阻塞 agent runtime

**来源参考**：

- 整体算法：`legacy/python/src/roostery/hooks_merge.py:60-110`（merge_event_hook） + 文件结构
- 模板 baseline：`legacy/python/src/roostery/templates/{claude_code_settings,codex_hooks,agent-stop-notify}.{json.tmpl,sh}`——env 前缀 + sh 调用入口 Rust port 化

### 2.2 编排层

**现状**：无 hooks-merge 模块；无 caller 调用模板嵌入 / hook 合并；agent runtime 装机靠用户手动编辑 `~/.claude/settings.json`。

**变化**：本 feature 落地后形成两条调用路径（本 feature 仅暴露 API，不真正消费）——

1. **写 hook**：未来 `roostery init` → `hooks_merge::apply_template(CC_STOP_HOOK_JSON, &cc_settings, &sh_path)` → atomic 写
2. **写 bridge sh**：未来 `roostery init` → 把 `STOP_HOOK_AGENT_NOTIFY_SH` 写到 `~/.roostery/scripts/agent_stop_notify.sh` + chmod 755

**主流程图（`apply_template`）**：

```mermaid
flowchart TD
    A[apply_template src/path/script] --> B[render_template src + script]
    B -->|含 {{HOOK_SCRIPT}}| C[String::replace + serde_json::from_str]
    C -->|parse 失败| D[Err FragmentInvalid]
    C -->|ok Value| E[merge_event_hook path fragment]
    E --> F[load existing target / fs::read]
    F -->|file 不存在| G[fragment 直接当结果]
    F -->|IO 错误| H[Err ReadFailed]
    F -->|parse 失败| I[Err ParseFailed]
    F -->|ok target Value| J[detect event key from fragment]
    J -->|0 或 >1 keys| K[Err FragmentInvalid]
    J -->|exactly 1 event| L[找/建 hooks.event 数组]
    L --> M[按 matcher 找同 bucket]
    M -->|no bucket| N[append fragment item]
    M -->|有 bucket| O[bucket.hooks 按 command tail 去重]
    O -->|新 command| P[append to bucket.hooks]
    O -->|同 command| Q[更新 timeout 不追加]
    G --> R[serde_json::to_writer_pretty + \n]
    N --> R
    P --> R
    Q --> R
    R --> S[fs::write .tmp]
    S --> T[fs::rename .tmp target]
    T --> U[Ok target_path]
```

**`merge_event_hook` 内部行为**（按 Python parity 复刻）：

```rust
// 伪代码
let target = load_existing(target_path)?;  // 不存在 → {}
let event_key = detect_event_key(fragment)?;  // 唯一 event；多个/无 → FragmentInvalid
let new_matcher_entry = fragment["hooks"][event_key][0];
let matcher = new_matcher_entry["matcher"].as_str().unwrap_or("*");
let new_hook = new_matcher_entry["hooks"][0];
let new_cmd = new_hook["command"].as_str()?;

let arr = target.entry("hooks").or_insert(json!({}))
                .entry(event_key).or_insert(json!([]));

// 找同 matcher
let bucket = arr.iter_mut().find(|item| item["matcher"] == matcher);
match bucket {
    None => arr.push(new_matcher_entry.clone()),
    Some(b) => {
        let bucket_hooks = b["hooks"].as_array_mut()?;
        // 按 command tail 去重
        if let Some(existing) = bucket_hooks.iter_mut().find(|h| {
            tail_eq(h["command"].as_str().unwrap_or(""), new_cmd)
        }) {
            // 更新 timeout 不追加
            existing["timeout"] = new_hook["timeout"].clone();
        } else {
            bucket_hooks.push(new_hook.clone());
        }
    }
}
target
```

**流程级约束**：

- **不变量 1**：merge 是 idempotent —— 同 fragment 跑 N 次结果与跑 1 次相同（按 command tail 去重保证）
- **不变量 2**：atomic write —— `.tmp` + `fs::rename`；不留半文件
- **不变量 3**：target 文件不存在 → 用 fragment 直接当结果；不报错（first-run 装机友好）
- **不变量 4**：parse 失败 → Err 不破坏原文件（不在原地 truncate）
- **不变量 5**：fragment 必须含恰好 1 个 event key + matcher entry 数组至少 1 元素 + 第一个 entry 的 `hooks` 数组至少 1 元素含 `command` 字符串；任一缺失 → FragmentInvalid
- **不变量 6**：command 去重用尾匹配（剥 `KEY=VAL` env 前缀比较剩余），让"用户改了 env value 但脚本路径不变"被识别为同一 hook
- **不变量 7**：JSON 输出 `indent=2 + \n trailing newline`（Python parity，golden file 友好）
- **错误语义**：4 类 `HooksError` 都实现 Display + thiserror；caller match 决定怎么处理

### 2.3 挂载点清单

判据"删了它 feature 是否消失"：

1. **`crates/roostery/src/hooks_merge.rs` 存在** — 删 → 模块不存在 → feature 消失
2. **`pub mod hooks_merge;` in lib.rs** — 删 → API 不可达
3. **`templates/cc_stop_hook.json` + `codex_stop_hook.json` + `agent_stop_notify.sh` 三个文件存在** — 删任一 → `include_str!` 编译失败
4. **三个 `pub const`（CC_STOP_HOOK_JSON / CODEX_STOP_HOOK_JSON / STOP_HOOK_AGENT_NOTIFY_SH）暴露** — 删 → caller 拿不到模板
5. **`apply_template` 公开 fn** — 删 → 一站式入口消失，caller 必须手动串 render + merge + write

5 条 strong mount points，符合 3-5 条上限。

**不列**：`HooksError` 变体数、command tail 去重算法、indent=2 数值——内部参数。

### 2.4 推进策略

按 paradigm 维度切片（基础设施 → 模板 → 计算节点 → 持久化 → 集成测试）：

1. **lib.rs + 模板文件 + 类型骨架**：lib.rs 加 `pub mod hooks_merge;`；新建 `src/hooks_merge.rs` 含 3 个 `pub const` (`include_str!`) + `HooksError` enum + 3 个 fn 签名 `todo!()`；新建 `src/templates/{cc_stop_hook.json,codex_stop_hook.json,agent_stop_notify.sh}` 3 个文件
   - 退出信号：`cargo build` 成功；3 个 const 字符串非空；既有 testsuite 无回归
2. **render_template + 字符串替换**：实现 `render_template`；占位符替换 + serde_json parse
   - 退出信号：render 4 单测（CC / Codex 各 1 happy + 占位符未替换 case + 模板含非法 JSON case）
3. **merge_event_hook 核心算法**：实现 `merge_event_hook`；event key 探测 / matcher 找 bucket / command tail 去重；本步骤**不**做文件 IO 直接测纯函数行为
   - 退出信号：merge 6 单测（空 target / 已有不同 event / 同 event 不同 matcher / 同 matcher 不同 command 追加 / 同 command 去重 / fragment invalid 各错误变体）
4. **apply_template + atomic save**：实现 apply_template（render + merge + write）；fs::write `.tmp` + rename；ensure parent dir
   - 退出信号：apply_template 端到端单测（fixture 路径 + 验 .tmp 不残留 + 写入 JSON 与预期 byte-for-byte 一致）
5. **集成测试 + JSON 输出格式**：1 集成测试覆盖完整链路（CC + Codex 都注入；二次 apply 幂等）；golden file 断言（与 Python 期 byte-for-byte 一致除 env 前缀）
   - 退出信号：集成测试通过；输出 JSON indent=2 + `\n` 结尾守护；`cargo test --all` 全绿
6. **完整验收 + CI**：四命令全绿
   - 退出信号：本地 fmt/clippy/test --all/--doc 四命令全绿；远端 CI 全绿

### 2.5 结构健康度与微重构

**评估对象**：

- **要改的文件**：
  - `lib.rs`（+1 行 pub mod）—— 健康
- **要落新文件的目录**：
  - `crates/roostery/src/`（已有 redact / journal / remoterefs / paths / smoke / config / lark_cli/ / bin/）；新增 `hooks_merge.rs` —— 符合档 1
  - `crates/roostery/src/templates/`（**新建子目录**，存模板资源文件而非 Rust 源码）

**先查 compound convention**——`.codestable/compound/2026-05-16-decision-rust-module-organization.md`：

- 档 1 / 2 / 3 / 4（bin target）—— 都是 Rust 源码组织约定，**不适用于"模板资源文件子目录"**
- 项目首次出现"非 Rust 源码的资源文件子目录"

**结论**：**本次不做微重构**，但**记录新模式**——`crates/roostery/src/templates/` 是 Rust 资源文件惯例：

- 内部不是 Rust 源码，是 `include_str!` 引用的纯文本资源
- 不进 `mod.rs` 也不进 `pub mod` 声明
- 命名走文件本身扩展名（.json / .sh / .yaml 等）
- 路径相对 `include_str!` 调用点（即 `src/hooks_merge.rs` 同级的 `src/templates/` 子目录）

**建议沉淀的 convention**（implement 跑通后 acceptance 评估是否走 cs-decide）：

> **Rust 资源文件子目录约定（暂定）**：
> - `crates/{crate}/src/templates/`（或同级其他名）：放纯文本资源（.json / .sh / .yaml / .md 等），用 `include_str!` 编译期嵌入
> - 不写 `pub mod`、不进 mod 树
> - 每个资源文件对应 lib 顶层 `pub const NAME: &str = include_str!("templates/file.ext");` 暴露
> - 替换占位符用最朴素的 `String::replace`（除非真有多变量替换才上模板引擎）
> - 修改资源文件必须有 golden file 对比测试

acceptance 阶段评估扩 rust-module-organization decision 第 5 档（资源文件子目录）。

**超出范围的观察**（不阻塞本 feature）：

- 未来 Phase 5 `bot-stop-hook` 替代 sh bridge 为原生 Rust handler 时，会删 `templates/agent_stop_notify.sh` 文件 + 删 `STOP_HOOK_AGENT_NOTIFY_SH` const。届时本 feature 的"挂载点 3 + 4 部分子集"被拔除，但 hooks-merge 算法（render_template / merge_event_hook / apply_template）继续保留供其他 hook 用
- Gemini hook 模板归后续 feature

## 3. 验收契约

### 3.1 关键场景清单

#### Render 模板

- **S1.1** CC 模板 happy：`render_template(CC_STOP_HOOK_JSON, "/path/to/sh")` → 返 Value，`value["hooks"]["SessionEnd"][0]["hooks"][0]["command"]` == `"ROOSTERY_AGENT=cc /path/to/sh"`
- **S1.2** Codex 模板 happy：同上但 env 前缀 `ROOSTERY_AGENT=codex`
- **S1.3** 模板含 `{{HOOK_SCRIPT}}` 未替换：调 render 后 Value 不再含字符串 `{{HOOK_SCRIPT}}`
- **S1.4** 模板内容损坏（手工构造）：返 `HooksError::FragmentInvalid`

#### Merge 算法

- **S2.1** target 不存在：merge_event_hook fragment 直接当结果，不报错
- **S2.2** target 已有不同 event（如已有 `Stop`，加 `SessionEnd`）：两个 event 共存
- **S2.3** target 已有同 event 不同 matcher：在该 event 数组追加新 matcher entry
- **S2.4** target 已有同 matcher 不同 command（如已有第三方 hook）：bucket.hooks 数组追加新 command
- **S2.5** target 已有同 command（按 tail 比较）：不重复追加；timeout 字段 update 到 fragment 值
- **S2.6** target 已有 `ROOSTERY_AGENT=cc /path/script` + fragment 是 `ROOSTERY_AGENT=cc /path/script`：识别为同 command（尾匹配）→ 去重
- **S2.7** target 已有 `FEISHU_HUB_AGENT=cc /path/script` + fragment 是 `ROOSTERY_AGENT=cc /path/script`：command tail（`/path/script`）相同 → 识别为同 hook → 更新 env 前缀实际**不**更新（保留 existing 整个 command）。**注**：这种 migration case 由 Phase 3 `roostery init` 检测旧 env 主动重写而非靠 hooks-merge 自动迁移

#### Fragment 校验

- **S3.1** Fragment 含 0 个 event key：返 `FragmentInvalid { reason: "..." }`
- **S3.2** Fragment 含 2+ event key：返 `FragmentInvalid`
- **S3.3** Fragment.hooks[event] 数组空：返 `FragmentInvalid`
- **S3.4** Fragment 中第一 entry 的 hooks 数组空：返 `FragmentInvalid`
- **S3.5** Fragment hooks[0].command 缺失：返 `FragmentInvalid`

#### apply_template 端到端

- **S4.1** target 不存在：apply_template 成功；target 文件含 fragment 完整内容；`.tmp` 不残留
- **S4.2** target 存在并已有不同 event hooks：apply_template 后两 event 共存
- **S4.3** apply_template 二次调用：idempotent，文件内容与第一次相同（含 byte-for-byte）
- **S4.4** apply_template 自动 ensure parent dir：嵌套路径不存在时自动创建
- **S4.5** atomic：模拟 `.tmp` 写完之前崩溃（手工 rm `.tmp`）→ 原文件保持不变

#### JSON 输出格式

- **S5.1** 输出 JSON `indent=2` + `\n` 结尾：手验文件内容
- **S5.2** Byte-for-byte 与 Python golden file 一致（除 env 前缀已明示偏离 `FEISHU_HUB_AGENT` → `ROOSTERY_AGENT`）

#### 模块级

- **S6.1** `cargo test --all` 全绿，本 feature 新增测试 ≥ 12 条（unit + integration）
- **S6.2** `cargo test --doc` 全绿
- **S6.3** `cargo clippy --all-targets --all-features -- -D warnings` 通过
- **S6.4** `cargo fmt --all --check` 通过
- **S6.5** 三个 `pub const` 实测非空字符串：`assert!(!hooks_merge::CC_STOP_HOOK_JSON.is_empty())` 等

### 3.2 反向核对项（明确不做的可 grep 验证）

- `grep -E "use tinytemplate|use handlebars|use askama" crates/roostery/src/hooks_merge.rs` → 无
- `grep "FEISHU_HUB_" crates/roostery/src/hooks_merge.rs` → 无
- `grep -E "FEISHU_HUB_" crates/roostery/src/templates/` → 无（模板内容也不带 Python env）
- `grep -E "use tokio|async fn" crates/roostery/src/hooks_merge.rs` → 无
- `grep "Config\|config::" crates/roostery/src/hooks_merge.rs` → 无（不消费 config）
- `grep "Journal\|journal::" crates/roostery/src/hooks_merge.rs` → 无
- `grep -E "fn parse_cc_schema|fn validate_cc" crates/roostery/src/hooks_merge.rs` → 无（不做 CC schema 校验）
- `grep -E "fn unmerge|fn remove_hook" crates/roostery/src/hooks_merge.rs` → 无（不实现卸载）
- `wc -l crates/roostery/src/hooks_merge.rs` → < 500（档 1 阈值；预估 ~350）
- 反向核对：`Cargo.toml` 无新依赖
- 反向核对：`grep "fn main\|Subcommand" crates/roostery/src/hooks_merge.rs` → 无（纯 lib，main.rs 不动）

## 4. 与项目级架构文档的关系

**本 feature 提炼回 architecture 的内容**：

- **名词**：`Hook fragment` / `Event key` / `Matcher` / `Command tail` / `HOOK_SCRIPT 占位符` / `ROOSTERY_AGENT` env → ARCHITECTURE.md §2 术语表加 hook 装机相关词条
- **架构归并**：§3 Module D 加 hooks_merge 子节描述（模板 const + render + merge + apply_template + 与 roostery-init / bot-stop-hook 关系）+ 子 feature 列表标 done
- **§4.7 模板嵌入约定**：本 feature 落地 = §4.7 兑现首例；acceptance 标 Phase 3 已落地（feature `2026-05-18-hooks-merge`）
- **§5 关键架构决定**：可能加一条"模板资源文件子目录约定（src/templates/）"——但更适合走 cs-decide 归档（见 §2.5）
- **§6 已知约束**：加一条 "agent runtime env 前缀 `ROOSTERY_AGENT=<runtime>`"（与既有"ROOSTERY_HOME / ROOSTERY_LARK_CLI_BIN env prefix 切口径"一致）

**关联的已有架构 doc**：

- `.codestable/architecture/ARCHITECTURE.md` — acceptance 按上述更新 §2 / §3 Module D / §4.7 / §6
- `.codestable/requirements/agent-work-in-feishu.md` — 本 feature 兑现"装机后 agent runtime 触发 hook"的基础设施；acceptance 加变更日志 + implemented_by；status 保持 `draft`（用户视角 task 卡呈现要 Phase 5）
- `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml` — `hooks-merge` status `in-progress` → `done`
- `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §3 第 9 项 — status `planned` → `done`
- `.codestable/compound/2026-05-16-decision-rust-module-organization.md` — acceptance 评估扩展第 5 档（资源文件子目录）via cs-decide

### 4.1 后续观察（不阻塞本 feature）

- **Phase 5 `bot-stop-hook` 替代 sh bridge**：届时删 `templates/agent_stop_notify.sh` + 删 `STOP_HOOK_AGENT_NOTIFY_SH` const + 改 `CC_STOP_HOOK_JSON` / `CODEX_STOP_HOOK_JSON` 的 command 指向 native Rust handler（如 `roostery dispatcher fire ...`）
- **Gemini 模板**：roadmap §4.7 示例不含 Gemini；如未来要加，新增 `templates/gemini_stop_hook.json` + 新 const 即可
- **agent runtime 自动检测**：本 feature 不实现"识别本机装了哪些 runtime"；归 `roostery-init` 的 `agent_detect` 模块
- **legacy env 迁移**：`FEISHU_HUB_AGENT=cc /sh/path` 已存在的用户机器升级 Roostery → `ROOSTERY_AGENT=cc` 注入；本 feature **不自动迁移**，迁移路径归 `roostery init` 的 first-run 检测（"侦测到旧 Python env 前缀，问用户是否覆盖"）
- **资源文件子目录约定 cs-decide 归档**：见 §2.5 末尾建议
