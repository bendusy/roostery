---
doc_type: decision
category: convention
slug: business-identifier-newtype
status: active
created: 2026-05-16
tags: [rust, type-safety, newtype, serde, identifier, feishu]
---

# 业务标识符 newtype 隔离约定

## 背景

Python `feishu_hub` baseline 的 `remoterefs.py` 用 `Dict[str, Optional[str]]` 装 4 个 token 字段（`message_id` / `doc_token` / `folder_token` / `record_id`）。调用方拿到的 `refs['message_id']` 和 `refs['doc_token']` 类型完全一样，可以互相赋值——bug 只在飞书 API 跑起来后才暴露（且飞书的错误信息常常是"invalid xxx_id format"，区分不清是哪种 token 传错位）。

core-remoterefs feature 在 Rust port 时彻底重做这一层：9 个独立 newtype + `#[serde(transparent)]`。架构 review（architect agent）确认这条原则适用范围远超单 feature——Phase 4 dispatcher 的 trace/event/parent 标识符、Phase 5 task_writer 的回调参数、Phase 7 base 索引的 record 引用都会再遇到同样的"业务标识符相互不能错位"诉求。

本约定把这条已落地的实践上升为跨 feature 稳定原则。

## 决定

**对从飞书侧拿到的、有明确业务语义角色的标识符（token / id / cursor），一律用 newtype + `#[serde(transparent)]` 隔离类型。**

具体规范：

### 1. newtype 定义模板

```rust
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(transparent)]
pub struct MessageId(pub String);
```

- 单字段元组 struct 包 `String`
- `#[serde(transparent)]` 保证 JSON 形态是裸字符串（与 Python 版兼容、下游 view 透明）
- derive 集合默认 `Serialize, Deserialize, Debug, Clone, PartialEq, Eq`
- **不**默认 derive `Hash`（YAGNI；真有 HashSet/HashMap key 需求时再加）
- **不**实现任何 `From<TokenA> for TokenB` 互转——newtype 的全部价值在不可互转

### 2. ergonomics 辅助 trait

每个 newtype 配 `AsRef<str>` + `fmt::Display` impl，让调用方写 `path.push(id.as_ref())` / `format!("{id}")` 而不是 `&id.0`：

```rust
macro_rules! impl_token_str {
    ($($t:ident),+ $(,)?) => {
        $(
            impl AsRef<str> for $t { fn as_ref(&self) -> &str { &self.0 } }
            impl fmt::Display for $t {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str(&self.0)
                }
            }
        )+
    };
}
```

模块内 newtype 多时用 `macro_rules!` 批量生成，避免 2N 块重复样板。

### 3. 容器结构

聚合多个 newtype 的容器 struct 加 `#[non_exhaustive]`：

```rust
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct RemoteRefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<MessageId>,
    // ...
}
```

`#[non_exhaustive]` 强制外部 caller 用 `..Default::default()`——日后加字段不破坏既有调用方。

### 4. 编译期类型隔离的测试守护

`compile_fail` doctest 锁错误码，防 future refactor 让类型隔离悄悄失效：

```rust
/// ```compile_fail,E0308
/// use roostery::remoterefs::{MessageId, DocToken};
/// fn takes_msg(_: &MessageId) {}
/// let dt = DocToken("x".into());
/// takes_msg(&dt); // mismatched types
/// ```
```

`E0308`（mismatched types）/ `E0063`（missing field for non_exhaustive struct literal）等错误码必须显式锁定——否则 doctest 因别的原因 fail 时会 silently pass。

### 5. 适用范围

**适用**——有飞书侧业务语义角色的标识符：

- `MessageId`（IM 消息）
- `DocToken`（Docs/Sheets/Bitable 文档对象）
- `FolderToken` / `RecordId` / `ChatId` / `AppToken` / `WikiToken` / `TaskId` / `ThreadId`（已落地）
- 未来：`SheetToken` / `FileToken` / `TableId` / `SpaceId` / `CursorToken`（按需扩）
- **dispatcher 层的内部标识符同样适用**：`TraceId` / `EventId` / `ParentEventId`（Phase 4）

**不适用**——还没成为业务 token 的字符串：

- subcommand 名（`"im"` / `"+messages-send"`）—— 是协议字符串不是标识符
- 原始 argv 元素 / 临时字符串变量
- 用户自由文本输入（消息正文 / 文档内容）
- 通用错误信息字符串
- 配置项的字符串值（除非配置项本身是 token，如 `default_chat_id: ChatId`）

判据：**这个字符串如果传错位置，下游会出业务错误（飞书 API 报错 / 找错对象 / 串数据）→ newtype；如果只是字符串处理 → 普通 `String`/`&str`**。

## 为什么这样选

1. **Python 弱类型踩过坑**：`Dict[str, Optional[str]]` 让 cross-wiring bug 只能 runtime 暴露；飞书的错误信息精度差到难以反推。Rust 类型系统直接消除这类 bug
2. **`#[serde(transparent)]` 让类型升级零成本**：JSON 形态不变意味着 journal / 飞书 view / 第三方 reader 都看不出差异——是纯 Rust 侧的安全增强，不破坏 portable-by-default req 的公开契约
3. **`#[non_exhaustive]` 保护演化**：飞书每隔一段时间会新增 API 返回新字段（`sheet_token` / `wiki_token` 等都是后期加的），容器加字段时不要让用户代码因 struct literal 不全而编译失败
4. **`AsRef<str>` + `Display` 化解 ergonomics 反噬**：没有这两个 impl 时，调用方拼 URL 要写 `&id.0`，类型隔离的设计感被"丑写法"反噬。加上后类型安全和调用便利两不耽误
5. **`compile_fail` doctest 加错误码锁定意图**：rustdoc 默认 `compile_fail` 不区分失败原因；加 `,E0308` 后 future refactor 一旦让类型隔离失效（比如有人手贱加了 `From` impl），doctest 会因错误码不匹配而真的 fail

## 考虑过的替代方案

| 方案 | 为什么没选 |
|---|---|
| **`type MessageId = String;` 类型别名** | 别名只是改名，类型系统完全不区分——和 `String` 互相赋值合法，编译期无任何保护。Python 弱类型问题不解决 |
| **`enum TokenKind { Message(String), Doc(String), ... }` 单 enum 多变体** | 编译期能区分但调用方拿到的是 enum，要 match 才能拿出字符串——比 newtype 啰嗦；且 enum 强制"一个值只能是其中一种"，无法表达"同时持有 message_id 和 chat_id"这种自然情况 |
| **`Secret<T>` 包装（参考 `secrecy` crate）** | `Secret` 关注的是脱敏 / Debug 安全，不是类型隔离；语义不匹配。本约定关注"防 cross-wire"而非"防意外泄露" |
| **typestate / phantom 类型参数（如 `Token<MessageMarker>`）** | 单 newtype 已经够用；引入 phantom type 让 ergonomics（特别是 derive 派生）显著复杂，杀鸡用牛刀 |
| **不归档，靠 review 兜底** | AI 没有跨 feature 上下文时，会回到"什么字段都用 String"的默认；Python `Dict[str, str]` 翻车证明 review 不可靠 |

## 影响 / 后续约束

- **新 feature design §2.1 强制检查**：定义涉及飞书侧标识符的类型时必须按本约定走 newtype；走默认档位无需在 design §1 重复列出
- **跨模块 trait 设计约束**：
  - `LarkRunner` trait（Phase 2）的方法返回值如果是标识符，应返 newtype 而非 `String`
  - dispatcher `TraceContext`（roadmap §4.5）的 `trace_id` / `parent_event_id` 应定义为 `TraceId` / `EventId` newtype，而非现 roadmap 写的 `String`——roadmap 文档需相应更新
  - `Runner` trait（roadmap §4.3）的 `event: &HookEvent` 内部标识符同上
- **测试守护标准化**：涉及新增 newtype 的 feature 必须包含 `compile_fail,E0308` doctest 锁类型隔离；涉及新增 `#[non_exhaustive]` 容器的 feature 必须包含 `compile_fail,E0063` doctest 锁 struct literal 约束
- **macro 复用**：4+ 个 newtype 集中在同一模块时，统一用 `macro_rules!` 批量生成 `AsRef<str>` + `Display`（参考 `remoterefs.rs:52-79` `impl_token_str!`）。不需要抽到共用 macro crate——重复 6 行声明优于跨模块依赖
- **不溯及既往**：现有 `String` 类型的标识符（如 lib.rs 当前的 `SCHEMA_VERSION: u32` —— 注意这是数字不是标识符不在范围）不强制重写；下次该字段所属模块走 feature 时按本约定升级
- **审视周期**：Phase 4 dispatcher（`TraceContext` / `HookEvent` 实际落地）完成后回看一次——届时验证本约定在 trait 设计中的运行情况；不合理走本 decision 的 `update` 流程

## 相关文档

- `.codestable/features/2026-05-16-core-remoterefs/core-remoterefs-design.md` §0.1 / §1 D2 / §4.1：本约定的来源 feature 与首次系统使用
- `.codestable/architecture/ARCHITECTURE.md` §5 关键架构决定第 6 条：本约定的简明摘要 + 跨模块影响指引
- `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §4.1 / §4.3 / §4.5：`LarkRunner` / `Runner` / `TraceContext` 三个 trait 现写 `String` / `&str`——本约定生效后这些接口设计时需相应升级到 newtype；本 decision 不直接改 roadmap，由对应 feature design 阶段执行
- `.codestable/compound/2026-05-16-decision-rust-module-organization.md`：互补约定（模块组织 vs 类型组织）
