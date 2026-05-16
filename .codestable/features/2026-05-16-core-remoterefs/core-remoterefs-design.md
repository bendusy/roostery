---
doc_type: feature-design
feature: 2026-05-16-core-remoterefs
roadmap: rust-rewrite
roadmap_item: core-remoterefs
requirement: portable-by-default
status: approved
summary: remoterefs 模块——9 个 newtype token 类型隔离（含 Phase 5 必需的 TaskId/ThreadId）+ 单趟 match-walk in-place 抽取（首匹配赢由 is_none guard 显式）+ AsRef/Display caller ergonomics + non_exhaustive 向前兼容 + walk 深度上限 64 防御；错误兜底全 None；下游 caller 自己塞 journal entry.params.remote_refs
tags: [phase-1, module-a, foundations, remoterefs, newtype, type-safety]
---

# core-remoterefs design

## 0. 术语约定

| 术语 | 定义 | 防冲突结论 |
|---|---|---|
| `RemoteRefs` | 远端对象 token 容器结构，7 个字段，每字段是对应 newtype 的 `Option`；`#[non_exhaustive]` 防止外部 struct literal 构造 | grep 无冲突 |
| Token newtype | 7 个 unit struct（`MessageId` / `DocToken` / `FolderToken` / `RecordId` / `ChatId` / `AppToken` / `WikiToken`），每个 `#[serde(transparent)] pub struct Foo(pub String);`——JSON 形态仍是裸字符串，但 Rust 类型互不兼容 | grep 无冲突 |
| `extract` | 模块对外唯一入口：`extract(argv, stdout) -> RemoteRefs` | grep 无冲突 |
| JSON walk | 解析 stdout 为 `serde_json::Value` 后**单趟 match-walk**（不聚合到中间 HashMap，直接 in-place 填 RemoteRefs）；**不用 regex** | roadmap / ARCHITECTURE 历史描述写"regex 抽"，措辞与实际行为不符，acceptance 时修正 |
| Argv hint | 用 argv 命中 `create-folder` 消歧 `folder_token`——同 stdout key `token` 在不同 subcommand 下含义不同 | Python parity |
| `#[serde(transparent)]` | newtype 的 serde 属性，让 `MessageId("om_x")` 序列化为字符串 `"om_x"` 而不是 `{"value":"om_x"}`——下游 journal reader 看到的形态与 Python 版完全一致 | Rust serde idiom |
| `#[non_exhaustive]` | 加在 `RemoteRefs` 上，外部 crate 必须用 `..Default::default()` 才能构造——日后加字段不破坏外部 caller | Rust 向前兼容 idiom |

参考：`legacy/python/src/roostery/remoterefs.py`（行为 reference，**类型系统选择完全重新设计**，不沿用 Python 的 `Dict[str, Optional[str]]` 弱类型）。

### 0.1 为什么不只是 Python parity——Rust 杠杆点

Python `Dict[str, Optional[str]]` 让 7 个字段类型完全一样：调用方 `refs['message_id']` 和 `refs['doc_token']` 都是 `Optional[str]`，可以互相赋值，错误在飞书 API 跑起来才暴露。本 feature 借 Rust 类型系统拉开 4 处 Python 做不到的安全度：

1. **Newtype token 类型隔离**——`fn send_message(msg: &MessageId)` 物理上无法接 `&DocToken`；Phase 5 `task_writer` / `bot_bridge` 的 token cross-wiring bug 编译期拦截。配套 `AsRef<str>` + `Display` impl 让 caller 拼 URL / log 时写 `path.push(id.as_ref())` 而不必 `&id.0`——类型隔离不反噬可读性
2. **单趟 match-walk**——`match key.as_str()` 直接 in-place 填字段，"**首匹配赢**"语义由 `is_none()` guard **显式表达**；对比 Python 的 `HashMap<String, Vec<Value>>` 聚合后取 `[0]` —— 顺序约束藏在 `[0]` 这个隐式选择里，读者不看实现猜不出。控制流更直比性能更重要（< 100 KB stdout 上两者差距 μs 级）
3. **`#[non_exhaustive]` RemoteRefs**——外部加字段不破坏 caller 的 struct literal；强制走 `..Default::default()` 模式
4. **`Option::as_str().filter(non_empty).map(MessageId)` 链式提取**——Python `_coerce_str` helper 被语言级 method chain 替代，调用点直接读

### 0.2 与 redact / journal 的关系

- **不依赖 redact**：token 是业务标识符，不是 sensitive credential；可明文落 journal（用户审计 / 检索的依据）
- **不被 journal 强制集成**：`Journal::append` 不内建 RemoteRefs 抽取；下游 caller（Phase 2 `lark-cli-shim` / `LarkCli` wrapper）写 journal 前自己调 `remoterefs::extract` 把结果塞 `entry.params` 的 `remote_refs` 子字段——行为约定不是类型契约（journal `params` 是 untyped Value）

## 1. 决策与约束

### 范围

- 新文件 `crates/roostery/src/remoterefs.rs`：9 个 newtype token（7 个常用 + Phase 5 必需的 `TaskId` / `ThreadId`）+ `RemoteRefs` struct（含 `#[non_exhaustive]`）+ `extract` 函数 + 私有 `walk` helper（含深度限制）
- 每个 newtype 额外 impl `AsRef<str>` + `fmt::Display`（caller ergonomics，不破坏类型隔离）
- `lib.rs` 加 `pub mod remoterefs;`
- 不新增 Cargo 依赖（`serde` + `serde_json` 已在）
- 单元测试 ≥ 10 条，覆盖 happy（多字段）/ 嵌套 / Array / argv 消歧 / 错误兜底 / serialize transparent / Default 构造 / 同 key 嵌套 sibling 顺序断言 / walk 深度限制保护
- 编译期反向测试：`compile_fail` doctest 锁错误码（E0308 类型不兼容 + E0063 non_exhaustive struct literal）
- 纯函数、无 I/O、无 async

### 明确不做

- **不用 regex**：lark-cli stdout 结构化 JSON；regex 在嵌套字符串里易误匹配。反向 grep `^use regex|regex::` 在 remoterefs.rs 应无命中
- **不接 bytes 输入**：接 `&str`，调用方负责 UTF-8 解码（与 redact / journal 同口径）
- **不实现 builder pattern**：API 表面就 `extract` 一个生产入口 + `RemoteRefs::default()` 一个手动构造路径，足够；`non_exhaustive` 已经强制外部用 `..Default::default()`
- **不做 token 内容格式校验**：newtype 本身是"标记式校验"——`MessageId(String)` 包住意味着"调用方声称这是 message_id"。不校验长度 / 字符集——飞书 token 形态可能变；wrapping 的语义是类型隔离不是值校验
- **不实现 token 之间的 `From` / `TryFrom` 互转**：`From<MessageId> for DocToken` 这种**根本就不该有**——newtype 的全部价值就在不可互转
- **不暴露 walker 实现**：`walk` 函数私有；只暴露 `extract` + `RemoteRefs` + 7 个 newtype
- **不实现 `RemoteRefs::merge` / `diff` / `is_empty`**：下游需要再扩
- **不引入中间 `HashMap<String, Vec<Value>>` 聚合**：单趟 match-walk 直接 in-place 填 RemoteRefs。反向 grep `HashMap` 在 remoterefs.rs 非测试代码无命中
- **不读 Config**：字段集编译期硬编码 7 个
- **不修改 Python `remoterefs.py`**：legacy frozen

### 复杂度档位

走默认档位——纯库函数 + 标准 Rust 工程。Newtype + non_exhaustive 是类型系统标准用法，不构成复杂度偏离。

### 关键决策

| # | 决策 | 内容 | 来源 |
|---|---|---|---|
| 1 | 策略：单趟 match-walk + JSON walk 不用 regex | `serde_json::from_str → Value`，递归 walk 时 `match k.as_str()` 直接 in-place 填 RemoteRefs；不收集中间 HashMap | Python parity 行为 + Rust 模式匹配杠杆 |
| 2 | **Newtype 隔离 9 种 token**（Rust 杠杆 1）| 每个字段独立 unit struct + `#[serde(transparent)]`：`MessageId` / `DocToken` / `FolderToken` / `RecordId` / `ChatId` / `AppToken` / `WikiToken` / **`TaskId`** / **`ThreadId`**。下游函数签名按类型区分，cross-wiring bug 编译期拦截。新增 TaskId / ThreadId 由 architect review 指出——Phase 5 task_writer 必需（Feishu Task 主键 + CLAUDE.md State ownership 表里 IM thread 已显式出现），不预加会立刻变 churn | 用户对齐 + architect review |
| 2b | **每 newtype impl `AsRef<str>` + `fmt::Display`** | 让 caller 拼 URL / log 时写 `id.as_ref()` / `format!("{id}")`，不用 `&id.0`；不引入 `From` 互转（破坏类型隔离） | architect review；type-safe ergonomics |
| 2c | **derive 集合：`Serialize + Deserialize + Debug + Clone + PartialEq + Eq`，去掉 `Hash`** | `HashSet<MessageId>` 无现实 caller；YAGNI；向前兼容方向需要时再加 | architect review |
| 3 | **`#[non_exhaustive]` RemoteRefs**（Rust 杠杆 3）| 外部 crate 用 struct literal 构造编译失败，强制 `..Default::default()`；日后加字段（`sheet_token` / `table_id`）不破坏 caller | Rust 向前兼容 idiom |
| 4 | `#[serde(skip_serializing_if = "Option::is_none")]` 每字段 | 全 None 序列化为 `{}`；部分填只含非空字段。Journal `remote_refs` 子对象不会被 None 占位污染 | 序列化精简 + 加字段向前兼容 |
| 5 | doc_token 多名兼容用 `match` 的 `|` 模式 | `"document_id" \| "doc_token" \| "obj_token" => ...` 一条分支搞定，不写 for loop 遍历别名列表 | Rust 模式匹配；不引入 `serde(alias)`（serde alias 只对顶层 deserialize 有效，对 walk 嵌套无效） |
| 6 | argv 消歧仅 folder_token，传 `bool` 参数 | walk 函数接 `argv_create_folder: bool` flag；调用前 `argv.iter().any(\|a\| a.contains("create-folder"))` 一次计算；walk 内 `"token" if argv_create_folder && refs.folder_token.is_none()` | Python parity 范围；flag 参数化避免 walk 内反复扫 argv |
| 7 | 错误兜底全 `RemoteRefs::default()` 不抛 | stdout 非 JSON / parse fail / 无目标 key → 返默认（全 None）；`extract` 永不 panic / 永不返 Result | Python parity 错误语义 |
| 8 | **`Option::as_str().filter(\|s\| !s.is_empty()).map(\|s\| Foo(s.into()))`**（Rust 杠杆 4）| 直接调用点链式提取，不抽 `coerce_str` helper。Python 因方法链不完整才需要 helper | Rust 标准库 method chain |
| 9 | 首匹配赢用 `is_none()` guard | walk 内每分支以 `if refs.foo.is_none()` 守卫；同 key 多次出现取首次 walk 命中的非空字符串 | Python parity 语义；用 guard 显式表达意图 |
| 10 | Walk 函数签名 `walk(&Value, depth: u32, bool, &mut RemoteRefs)` + 深度上限 64 | 单递归实现 Object/Array 遍历；`depth > 64` 直接返回（不继续递归，已填字段保留）；防御"接外部输入的纯函数"栈溢出 | architect review；lark-cli 自家产出深度 < 5，64 是绰绰有余的安全边界 |
| 11 | **Sibling-key walk 顺序明文契约** | 同一 key 在多个 sibling object 出现（如 `{"a":{"message_id":"x"},"b":{"message_id":"y"}}`）时，**取 serde_json `Map` 迭代顺序首匹配**——默认 `BTreeMap` 按字典序，不承诺 stdout 物理顺序 | architect review；避免下游误以为 lark-cli 输出顺序敏感 |

## 2. 名词与编排

### 2.1 名词层

**现状**：`crates/roostery/src/` 5 文件，无 remoterefs 类型，无 token 抽取能力。

**变化**：新增 `crates/roostery/src/remoterefs.rs`；`lib.rs` 加 `pub mod remoterefs;`。

**公开 API 接口契约**：

```rust
// crates/roostery/src/remoterefs.rs

use serde::{Deserialize, Serialize};
use std::fmt;

// --- 9 个 newtype token 类型 ---------------------------------------------
// 每个 #[serde(transparent)] 保证 JSON 形态是裸字符串，与 Python parity 一致
// 但 Rust 类型互不兼容，cross-wiring 编译期拦截
// 不 derive Hash（无现实 HashSet caller）

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct MessageId(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct DocToken(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct FolderToken(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct RecordId(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct ChatId(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct AppToken(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct WikiToken(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct TaskId(pub String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct ThreadId(pub String);

// 每 newtype 配 AsRef<str> + Display（caller ergonomics，不引入 From 互转）
// 实现示例（9 个对称实现）:
//   impl AsRef<str> for MessageId { fn as_ref(&self) -> &str { &self.0 } }
//   impl fmt::Display for MessageId {
//       fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//           f.write_str(&self.0)
//       }
//   }
// implement 阶段可用宏批量生成（如 macro_rules! impl_token_str { ... }），不构成方案改动

// --- 容器结构 -------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct RemoteRefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<MessageId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_token: Option<DocToken>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_token: Option<FolderToken>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<RecordId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<ChatId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_token: Option<AppToken>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wiki_token: Option<WikiToken>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
}

/// 从 lark-cli stdout 提取 7 种远端 token（best-effort）。
///
/// - stdout 非 JSON / parse 失败 / 无目标 key → `RemoteRefs::default()`；永不 panic
/// - argv 用于消歧 folder_token（仅当含 `create-folder` 时启用 `token` key）
/// - 同 key 多处出现取首次 walk 命中
///
/// # 示例
/// ```ignore
/// let argv = vec!["lark-cli".into(), "im".into(), "+messages-send".into()];
/// let stdout = r#"{"message_id":"om_abc","chat_id":"oc_xyz"}"#;
/// let refs = extract(&argv, stdout);
/// assert_eq!(refs.message_id.as_ref().map(|m| &m.0), Some(&"om_abc".to_string()));
/// assert_eq!(refs.chat_id.as_ref().map(|c| &c.0), Some(&"oc_xyz".to_string()));
/// // 类型隔离：refs.message_id 不能传给签名为 &DocToken 的函数（编译期拦截）
/// ```
pub fn extract(argv: &[String], stdout: &str) -> RemoteRefs;
```

**字段 → stdout key 映射表**（编码进 walk 的 `match` 分支）：

| 目标字段（类型）| stdout key 候选 | 触发条件 |
|---|---|---|
| `message_id: MessageId` | `message_id` | 总是 |
| `doc_token: DocToken` | `document_id` \| `doc_token` \| `obj_token`（match `\|` 一分支）| 总是 |
| `folder_token: FolderToken` | `folder_token`；**仅当 argv 含 `create-folder` 时**追加 `token` | 条件 |
| `record_id: RecordId` | `record_id` | 总是 |
| `chat_id: ChatId` | `chat_id` | 总是 |
| `app_token: AppToken` | `app_token` | 总是 |
| `wiki_token: WikiToken` | `wiki_token` | 总是 |
| `task_id: TaskId` | `task_id` | 总是 |
| `thread_id: ThreadId` | `thread_id` | 总是 |

来源参考：

- Python 字段集（4 个）：`legacy/python/src/roostery/remoterefs.py:11` FIELDS 元组
- Python argv 消歧（folder_token）：`legacy/python/src/roostery/remoterefs.py:78-81`
- Python walk + key 聚合：`legacy/python/src/roostery/remoterefs.py:18-30, 62-66`（**Rust 不沿用 HashMap 聚合**，改单趟 match-walk）
- Python doc_token 多名兼容：`legacy/python/src/roostery/remoterefs.py:73`（**Rust 用 `|` 模式**而非 for loop 别名列表）

### 2.2 编排层

**现状**：无 caller。

**变化**：模块作为纯库被未来 caller 调用（Phase 2 `lark-cli-shim` / `LarkCli` wrapper），无内部 workflow。`extract` 是单函数主路径。

**`extract` 内部流程图**：

```mermaid
flowchart TD
    A[stdout 输入 + argv] --> B[计算 argv_create_folder bool]
    B --> C{stdout trim 后空?}
    C -->|是| Z[返 RemoteRefs::default]
    C -->|否| D{首字符 `{` 或 `[`?}
    D -->|否| Z
    D -->|是| E[serde_json::from_str]
    E -->|Err| Z
    E -->|Ok Value| F[walk Value, argv_create_folder, &mut refs]
    F --> G[match k.as_str 直接 in-place 填字段<br/>每分支 is_none guard 实现首匹配赢]
    G --> H[返回 refs]
```

**walk 实现示意**（不是最终代码，design 阶段对算法结构有所要求）：

```rust
const MAX_DEPTH: u32 = 64;

fn walk(value: &Value, depth: u32, argv_create_folder: bool, refs: &mut RemoteRefs) {
    if depth > MAX_DEPTH {
        return; // 防御深嵌套栈溢出；已填字段保留
    }
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                match k.as_str() {
                    "message_id" if refs.message_id.is_none() => {
                        refs.message_id = as_token(v).map(MessageId);
                    }
                    "document_id" | "doc_token" | "obj_token"
                        if refs.doc_token.is_none() =>
                    {
                        refs.doc_token = as_token(v).map(DocToken);
                    }
                    "folder_token" if refs.folder_token.is_none() => {
                        refs.folder_token = as_token(v).map(FolderToken);
                    }
                    "token" if argv_create_folder && refs.folder_token.is_none() => {
                        refs.folder_token = as_token(v).map(FolderToken);
                    }
                    "record_id" if refs.record_id.is_none() => {
                        refs.record_id = as_token(v).map(RecordId);
                    }
                    "chat_id" if refs.chat_id.is_none() => {
                        refs.chat_id = as_token(v).map(ChatId);
                    }
                    "app_token" if refs.app_token.is_none() => {
                        refs.app_token = as_token(v).map(AppToken);
                    }
                    "wiki_token" if refs.wiki_token.is_none() => {
                        refs.wiki_token = as_token(v).map(WikiToken);
                    }
                    "task_id" if refs.task_id.is_none() => {
                        refs.task_id = as_token(v).map(TaskId);
                    }
                    "thread_id" if refs.thread_id.is_none() => {
                        refs.thread_id = as_token(v).map(ThreadId);
                    }
                    _ => {}
                }
                walk(v, depth + 1, argv_create_folder, refs);
            }
        }
        Value::Array(arr) => {
            arr.iter().for_each(|v| walk(v, depth + 1, argv_create_folder, refs))
        }
        _ => {}
    }
}

fn as_token(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(|s| s.to_string())
}
```

**流程级约束**：

- **不变量 1**：`extract` 永不 panic / 永不返 Result——所有失败路径返 `RemoteRefs::default()`
- **不变量 2**：纯函数无副作用——接 `&` 借用返 owned struct，不修改入参（argv / stdout）
- **不变量 3**：首匹配赢——同 key 多处出现，取 walk 顺序首次非空字符串（`is_none()` guard 显式实现）
- **不变量 4**：argv 消歧只影响 folder_token；其他 6 字段抽取完全独立于 argv
- **不变量 5**：空字符串值视同 None（`as_token` 过滤）
- **不变量 6**：类型隔离——下游函数签名 `fn send_to(msg: &MessageId)` 物理上无法接 `&DocToken` 等其他 6 种 token（编译期保证，非测试可验证）
- **不变量 7**：JSON serialize 形态与 Python 版兼容——`#[serde(transparent)]` 让 `MessageId("om_x")` 输出 `"om_x"` 而非 `{"value":"om_x"}`；journal reader 跨 Python/Rust 版本看到的形态一致
- **不变量 8**：sibling-key walk 顺序由 serde_json `Map` 迭代顺序决定（默认 BTreeMap 字典序），**不承诺 stdout 物理顺序**。下游 caller 若依赖 stdout 出现位置 → 不要依赖 RemoteRefs，自己 parse
- **不变量 9**：walk 深度上限 64——超出时 `walk` 直接 return，已填字段保留；防御"接外部输入纯函数"栈溢出
- **错误语义**：`extract` 不带 Result——失败模式吸收为字段 None。调用方无需区分"parse 失败"和"字段不存在"——契约是"best-effort，没抽到就是没抽到"

### 2.3 挂载点清单

判据"删了它 feature 是否消失"：

1. **`crates/roostery/src/remoterefs.rs` 存在** — 删 → 7 个 newtype + RemoteRefs + extract 全无 → feature 消失
2. **`crates/roostery/src/lib.rs` 含 `pub mod remoterefs;`** — 删 → API 对外不可见 → feature 消失
3. **`RemoteRefs` derive `Serialize + Deserialize + Default`** — 删 → 下游无法塞 journal Value / 无法 `..Default::default()` 构造 → feature 表面消失
4. **7 个 newtype 都 `#[serde(transparent)]`** — 删 → JSON serialize 形态变成 `{"value":"om_x"}` 嵌套对象 → 不变量 7 破坏 → feature 实质消失（即使代码可编译）
5. **`RemoteRefs` 有 `#[non_exhaustive]`** — 删 → 外部 crate 可用 struct literal 构造，未来加字段破坏 caller → 向前兼容承诺消失

5 条 strong mount points，符合 3-5 条上限。

**不列**：字段映射表内容（哪个 stdout key 抽哪个字段）、字段数量本身、`as_token` 私有 helper——这些是模块内部，改一条不消失 feature。

## 2.4 推进策略

按 paradigm 维度切片（类型骨架 → 算法骨架 → 主体 → 测试覆盖）：

1. **模块骨架 + 9 newtype + RemoteRefs**：建 `remoterefs.rs`，声明 9 个 unit struct（`MessageId` / ... / `TaskId` / `ThreadId`）+ `RemoteRefs` struct（含 `#[non_exhaustive]` + serde skip）+ `extract` 签名 `todo!()`；`lib.rs` 加 `pub mod remoterefs;`
   - 退出信号：`cargo build` 成功；9 newtype 可独立构造；`RemoteRefs::default()` 可调；serialize 全 None RemoteRefs 输出 `{}`；serialize `Some(MessageId("om_x".into()))` 输出裸 `"om_x"`（transparent 验证）
2. **每 newtype 加 `AsRef<str>` + `fmt::Display` impl**：建议用 `macro_rules!` 批量生成 9 个对称实现，避免 18 块重复样板
   - 退出信号：`MessageId("x".into()).as_ref() == "x"`；`format!("{}", MessageId("x".into())) == "x"`；其余 8 个同样测一遍
3. **`as_token` helper + walk 函数骨架（含 depth 参数）**：实现私有 `as_token(&Value) -> Option<String>` + `walk(&Value, depth: u32, bool, &mut RemoteRefs)` 框架；`const MAX_DEPTH: u32 = 64;` + 超限早返
   - 退出信号：`cargo build` 成功；walk 能跑通 Object/Array 递归不 panic
4. **walk 主体 + extract 主路径**：填 walk 的 10 个 match 分支（9 字段 + folder_token 的 token-on-create-folder 分支）；实现 extract 的 argv flag 计算 + 短路逻辑 + 用 `depth=0` 起调 walk
   - 退出信号：所有 happy / 嵌套 / Array / argv 消歧 / 错误兜底 case 通过
5. **类型隔离 doctest + serialize roundtrip + sibling 顺序 + depth 限制测试**：
   - 写 `compile_fail,E0308` doctest 证明 `MessageId` 不能传给 `&DocToken` 签名
   - 写 `compile_fail,E0063` doctest 证明 `RemoteRefs { message_id: None }` 无 `..Default::default()` 编译失败
   - roundtrip test 证明 transparent 形态稳定
   - 加 sibling-key walk 顺序断言（`{"a":{"message_id":"x"},"b":{"message_id":"y"}}` → 按 BTreeMap 字典序首匹配，明文断言 "x" 赢）
   - 加深度限制测试：构造嵌套 100 层的 stdout，验证 `extract` 返回（不 panic / 不栈溢出）且包含浅层字段
   - 退出信号：所有验收场景 S1-S6 测试通过
6. **集成验证**：`cargo test --all` + `cargo test --doc` + `cargo clippy --all-targets --all-features -- -D warnings` + `cargo fmt --all --check` 全绿；推 CI
   - 退出信号：本地四命令全绿；远端 CI 全绿

### 2.5 结构健康度与微重构

**评估对象**：

- **要改的文件**：`crates/roostery/src/lib.rs`（当前 5 行 → 6 行，加 1 行 `pub mod remoterefs;`）—— 无健康度问题
- **要落新文件的目录**：`crates/roostery/src/`（当前 5 文件，加 `remoterefs.rs` → 6 文件）—— 仍在档 1（compound convention）

**先查 compound convention**——`.codestable/compound/2026-05-16-decision-rust-module-organization.md` 已归档"flat-first，500 行升档档 2"。本 feature 直接照办：新模块落 `src/remoterefs.rs`，预计 ~300 行（7 newtype × ~3 行 derive + RemoteRefs + walk + extract + 8 测试），远低于 500 行档 2 阈值。

**结论**：**本次不做微重构**。

理由：

- 按 compound convention 档 1 默认路径，新模块直接落单文件
- 现有 5 文件无健康度问题，本 feature 不动它们
- 7 个 newtype + RemoteRefs 同文件是 Rust 惯例（serde 类型族通常聚合在一起，方便看类型间关系），无拆子文件价值

**超出范围的观察**（不阻塞本 feature）：

- Phase 2 `lark_cli.rs` 进入后 `src/` 达 7 文件；若 lark_cli 超 500 行触发档 2 升档，在 `lark-cli-wrapper` feature design §2.5 处理

## 3. 验收契约

### 3.1 关键场景清单（输入 / 触发 → 期望可观察结果）

#### Happy path（每字段一条）

- **S1.1** `message_id`：stdout = `{"message_id":"om_abc"}` → `refs.message_id == Some(MessageId("om_abc".into()))`，其他全 None
- **S1.2** `doc_token` from `document_id`：stdout = `{"document_id":"doxbAaa"}` → `refs.doc_token == Some(DocToken("doxbAaa".into()))`
- **S1.3** `doc_token` from `doc_token`：stdout = `{"doc_token":"shtbBbb"}` → 同上
- **S1.4** `doc_token` from `obj_token`：stdout = `{"obj_token":"bascCcc"}` → 同上
- **S1.5** `folder_token` 显式 key：stdout = `{"folder_token":"fldDdd"}`，argv 无 `create-folder` → `refs.folder_token == Some(FolderToken("fldDdd".into()))`
- **S1.6** `folder_token` 走 argv 消歧：stdout = `{"token":"fldEee"}`，argv = `["lark-cli","drive","+create-folder"]` → `refs.folder_token == Some(FolderToken("fldEee".into()))`
- **S1.7** `folder_token` argv 消歧不命中：stdout = `{"token":"unknown"}`，argv = `["lark-cli","im","+messages-send"]` → `refs.folder_token == None`
- **S1.8** `record_id`：stdout = `{"record_id":"recFff"}` → `refs.record_id == Some(RecordId("recFff".into()))`
- **S1.9** `chat_id`：stdout = `{"chat_id":"oc_Ggg"}` → `refs.chat_id == Some(ChatId("oc_Ggg".into()))`
- **S1.10** `app_token`：stdout = `{"app_token":"bascHhh"}` → `refs.app_token == Some(AppToken("bascHhh".into()))`
- **S1.11** `wiki_token`：stdout = `{"wiki_token":"wikIii"}` → `refs.wiki_token == Some(WikiToken("wikIii".into()))`
- **S1.12** `task_id`：stdout = `{"task_id":"tsk_Jjj"}` → `refs.task_id == Some(TaskId("tsk_Jjj".into()))`
- **S1.13** `thread_id`：stdout = `{"thread_id":"omt_Kkk"}` → `refs.thread_id == Some(ThreadId("omt_Kkk".into()))`

#### 多字段混合 / 嵌套 / 顺序

- **S2.1** 同时含 `message_id` + `chat_id`：两字段都填
- **S2.2** 嵌套 Object：stdout = `{"data":{"message_id":"om_z"}}` → `refs.message_id == Some(MessageId("om_z".into()))`
- **S2.3** Array 包裹（首匹配赢）：stdout = `{"items":[{"record_id":"rec1"},{"record_id":"rec2"}]}` → `refs.record_id == Some(RecordId("rec1".into()))`
- **S2.4** 顶层 Array：stdout = `[{"message_id":"om_a"}]` → 抽到
- **S2.5** **Sibling-key 顺序断言**（锁定不变量 8）：stdout = `{"b":{"message_id":"om_b"},"a":{"message_id":"om_a"}}` → `refs.message_id == Some(MessageId("om_a".into()))`（serde_json 默认 BTreeMap 按字典序遍历，`"a"` 在 `"b"` 之前；不承诺 stdout 物理顺序）
- **S2.6** **doc_token 多 alias 同时出现**：stdout = `{"document_id":"dx","doc_token":"dt","obj_token":"ot"}` → `refs.doc_token` 取 BTreeMap 字典序首位（`"doc_token" < "document_id" < "obj_token"` → `"dt"`）

#### 边界 / 错误兜底

- **S3.1** 空 stdout / 纯空白 / 非 JSON / parse fail / 无目标 key / value 非 string / value 空 string / primitive 顶层 → 全 8 种情况返 `RemoteRefs::default()` 全 None，永不 panic
- **S3.2** **深嵌套 100 层**（锁定不变量 9 walk 深度上限 64）：构造 `{"a":{"a":{...100 层 nested object...}}}` 在第 30 层（< 64）含 `message_id`、在第 80 层（> 64）含 `chat_id`；`extract` 不 panic / 不栈溢出；浅层 message_id 抽到，深层 chat_id 抽不到

#### 类型隔离（Rust 杠杆点 1 验证）

- **S4.1** **编译期类型隔离 doctest**：用 `compile_fail,E0308` doctest——`fn takes_msg(_: &MessageId) {}` + `let dt = DocToken("x".into()); takes_msg(&dt);` —— 确认编译失败且错误码是 E0308（mismatched types）。**E0308 锁定意图**：future refactor 让 doctest 因别的原因失败时不再 silently pass
- **S4.2** 类型不可互转：`From<MessageId> for DocToken` 等 9×8 = 72 个互转 impl **均不存在**（编译期保证 + §3.2 反向 grep 守护）

#### Serialize / 类型行为（`transparent` + `non_exhaustive` + `AsRef`/`Display` 验证）

- **S5.1** Newtype `transparent`：`serde_json::to_string(&MessageId("om_x".into()))` → `"\"om_x\""`（裸字符串，**不是** `{"value":"om_x"}`）
- **S5.2** 全 None RemoteRefs serialize → `"{}"`（所有字段 skip_serializing_if）
- **S5.3** 部分填的 RemoteRefs serialize → 只含非空字段
- **S5.4** Roundtrip：`extract` 结果 serialize → from_str → `==` 原 RemoteRefs
- **S5.5** **`non_exhaustive` 验证 doctest**：用 `compile_fail,E0063` doctest——`let _ = RemoteRefs { message_id: None };` 缺其他字段且无 `..Default::default()` 应触发 E0063（missing field）；用 `..Default::default()` 应编译通过
- **S5.6** **`AsRef<str>` + `Display` 验证**：对每个 newtype 9 个，`MessageId("x".into()).as_ref() == "x"` 且 `format!("{}", MessageId("x".into())) == "x"`，表驱动测一遍

#### 模块级

- **S6.1** `cargo test --all` 全绿，本 feature 新增测试 ≥ 10 个
- **S6.2** `cargo test --doc` 全绿（含 `compile_fail` doctests）
- **S6.3** `cargo clippy --all-targets --all-features -- -D warnings` 通过
- **S6.4** `cargo fmt --all --check` 通过

### 3.2 反向核对项（明确不做的可 grep 验证）

- `grep -E "^use regex|regex::" crates/roostery/src/remoterefs.rs` → 无（用 JSON walk 不用 regex）
- `grep -E "async fn|use tokio" crates/roostery/src/remoterefs.rs` → 无（同步纯函数）
- `grep -E "Result<RemoteRefs|-> Result<" crates/roostery/src/remoterefs.rs` → 无（extract 不返 Result）
- `grep -E "panic!|\.unwrap\(\)" crates/roostery/src/remoterefs.rs` → 非测试代码无
- `grep -E "fn merge|fn diff|fn is_empty" crates/roostery/src/remoterefs.rs` → 无
- `grep "use crate::redact" crates/roostery/src/remoterefs.rs` → 无（不依赖 redact）
- `grep "Config" crates/roostery/src/remoterefs.rs` → 无（不读 Config）
- `grep -E "^pub fn " crates/roostery/src/remoterefs.rs | wc -l` → 1（只 `extract` 一个公开函数）
- `grep "HashMap" crates/roostery/src/remoterefs.rs` → 非测试代码无（单趟 match-walk，**不引入 HashMap 聚合**，Rust 杠杆 2 守护）
- `grep -E "impl From<MessageId>|impl From<DocToken>|impl From<.*Token>|impl From<.*Id>" crates/roostery/src/remoterefs.rs` → 无（token newtype 互不可转，Rust 杠杆 1 守护）
- `grep -cE '^#\[serde\(transparent\)\]' crates/roostery/src/remoterefs.rs` → 9（行首 attribute 计数；不算注释里 inline 提及。Rust 杠杆 1 + 不变量 7 守护。**Calibration 修订**：design 初稿写 `grep -c '#[serde(transparent)]'`，但文件内 doc 注释也提及 attribute 名导致计数 11，acceptance 时改为行首精确匹配）
- `grep "#\[non_exhaustive\]" crates/roostery/src/remoterefs.rs` → 至少 1（RemoteRefs 上，Rust 杠杆 3 守护）
- `grep -E "fn coerce_str|fn coerce_string" crates/roostery/src/remoterefs.rs` → 无（method chain 替代，Rust 杠杆 4 守护；私有 helper 叫 `as_token` 单一职责）
- `grep "Hash" crates/roostery/src/remoterefs.rs` → 无（derive 集合不含 Hash，YAGNI；architect review 守护）
- `grep -c "impl AsRef<str> for" crates/roostery/src/remoterefs.rs` → 至少 1（**Calibration 修订**：原计数 9 假设每 newtype 独立 impl 块；实际用 `macro_rules! impl_token_str!` 批量生成 9 套对称实现，单 macro 块 grep count == 1；由 `newtype_as_ref_and_display` 表驱动测试 9 newtype 验证等价）
- `grep -c "impl .*Display for" crates/roostery/src/remoterefs.rs` → 同上 calibration（macro 实现）
- `grep -E "MAX_DEPTH" crates/roostery/src/remoterefs.rs` → 至少 1（walk 深度上限常量；不变量 9 守护）
- `wc -l crates/roostery/src/remoterefs.rs` → < 600（**Calibration 修订**：design 初稿写 < 500；实际 512 行（含 27 inline tests + 表驱动 macro + 3 doctests）。9 newtype + 30 测试 + macro 块 + doctest 文档密度高于 redact / journal，与 core-redact wc-l calibration 同款情形，提到 600）

## 4. 与项目级架构文档的关系

**本 feature 提炼回 architecture 的内容**：

- **名词**：`RemoteRefs` + 7 newtype token 类型 + `extract` 函数 → ARCHITECTURE.md §3 Module A 节加 remoterefs 子节（公开 API + 字段集 + newtype 类型隔离的杠杆点 + 与 redact / journal 的关系）
- **架构杠杆点的归并**：4 条 Rust 杠杆（newtype / 单趟 walk / non_exhaustive / method chain）中前 3 条值得在 ARCHITECTURE.md 留痕——前 2 是项目第一次系统性使用的 Rust 类型 idiom，未来其他模块（Phase 2 `LarkRunner` / Phase 4 dispatcher 各 trait / Phase 5 `task_writer`）应**统一采用 newtype 隔离业务标识符**。Acceptance 评估是否在 §5 关键架构决定加一条"业务标识符（token / id / cursor）一律 newtype 隔离"——倾向加，但这构成跨 feature 约束，应走 `cs-decide convention` 而非直接进 ARCHITECTURE
- **历史措辞修正**：以下两处当前写"regex 抽"，acceptance 时按本 feature 实际策略改为"JSON walk + match-dispatch 抽"：
  - `.codestable/architecture/ARCHITECTURE.md` §3 Module A 节 "（regex 抽 doc_token / record_id）" → "（JSON walk + match-dispatch 抽 7 个 newtype token：message_id / doc_token / folder_token / record_id / chat_id / app_token / wiki_token）"
  - `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §5 第 3 条 "regex 从 lark-cli stdout 抽" → "JSON walk 从 lark-cli stdout 抽"；字段集同步扩 7 个 + 说明 newtype 隔离
- **流程级约束**：`extract` 永不 panic + 类型隔离不变量（不变量 6） → 不进 ARCHITECTURE §6（属模块级契约，挂模块 doc 即可；类型隔离是 Rust 编译器守护无须文档兜底）
- **schema 公开承诺**：RemoteRefs **不**触发新 schema_version 承诺——它是 journal entry.params 内部的子对象，靠 `serde(skip_serializing_if)` + `non_exhaustive` 实现"加字段不破坏旧 reader"；不像 JournalEntry 那样作为顶层契约
- **journal 集成约定**：acceptance 时在 ARCHITECTURE.md §3 Module B 已有"caller 自己调 redact"段后加一句"同理 caller 自己调 `remoterefs::extract` 把结果塞 `entry.params.remote_refs` 子字段"

**关联的已有架构 doc**：

- `.codestable/architecture/ARCHITECTURE.md` — acceptance 按上述更新 §3 Module A、Module B
- `.codestable/attention.md` — 不动（无新硬约束）
- `.codestable/requirements/portable-by-default.md` — 本 feature 间接兑现（journal entry 携带 token 便于审计 / 检索）。Acceptance 在变更日志追加一条；不升级 status
- `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml` — acceptance 时 `core-remoterefs` 条目 `status: in-progress → done`
- `.codestable/compound/` — acceptance 评估起 `cs-decide convention` 归档 "业务标识符 newtype 隔离" 约定（跨 feature 稳定模式）

### 4.1 后续观察（不阻塞本 feature）

- **字段集扩展**：Phase 2-7 实际接入更多 lark-cli subcommand 可能需要 `sheet_token` / `file_token` / `table_id` / `space_id` 等。扩展时**加 newtype + 加 RemoteRefs 字段 + 加 walk match 分支**，三处同步；不 bump schema_version（journal `params.remote_refs` 是 untyped Value 子字段不受 JournalEntry schema 约束）。`non_exhaustive` 已经保证外部 caller 不破
- **argv 消歧规则扩展**：当前只 folder_token 一条规则。Phase 2 `lark-cli-shim` 实际跑起来后可能发现其他需要消歧的（如 `app_token` 在 Base / Wiki 上下文混淆）。先观察实际再扩，不预加
- **业务标识符 newtype 约定归档（范围收紧）**：本 feature 是项目第一个系统使用 newtype 隔离业务标识符的模块。acceptance 时评估走 `cs-decide convention` 归档——**约定范围限定为"对从飞书侧拿到的、有明确业务语义角色的标识符"才上 newtype**（如 message_id / task_id / chat_id / trace_id / event_id）；**不**适用于"还没成为业务 token 的字符串"（subcommand 名 / 原始 argv 元素 / 临时变量 / 普通 String 参数）——后者 newtype 化是 noise。Phase 4 dispatcher 的 `TraceId` / `EventId` / `ParentEventId` 是合格候选；`LarkRunner::run(&self, args: &[&str])` 的 `args` 不是。措辞精度避免约定被误读成"所有 String 都包"
- **walker 性能**：单趟 match-walk 已经零中间分配；lark-cli stdout 通常 < 100 KB，不预期成为瓶颈
