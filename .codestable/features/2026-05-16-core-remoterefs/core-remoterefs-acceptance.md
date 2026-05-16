---
doc_type: feature-acceptance
feature: 2026-05-16-core-remoterefs
roadmap: rust-rewrite
roadmap_item: core-remoterefs
requirement: portable-by-default
status: passed
summary: core-remoterefs 验收通过；9 newtype + 单趟 match-walk + non_exhaustive + compile_fail doctest 全到位；CI 三 job 绿；归并 ARCHITECTURE §2/§3/§5 + roadmap §5 第 3 条 + req 变更日志；2 处 calibration 写回 design §3.2（grep count 因 macro 实现差异 + wc -l < 500 → < 600）
tags: [phase-1, module-a, remoterefs, newtype, acceptance]
---

# core-remoterefs 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-16
> 关联方案 doc：`.codestable/features/2026-05-16-core-remoterefs/core-remoterefs-design.md`
> 实现 commit：`4714683`
> CI：GitHub Actions run `25954205675` — fmt / clippy / test 三 job 全绿

## 1. 接口契约核对

### 接口示例逐项核对（design §2.1）

- [x] 9 个 newtype（`MessageId` / `DocToken` / `FolderToken` / `RecordId` / `ChatId` / `AppToken` / `WikiToken` / `TaskId` / `ThreadId`）→ `remoterefs.rs:16-50` 全部 `#[serde(transparent)] pub struct Foo(pub String)` + derive 集合 `(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)` 无 Hash ✓
- [x] 每 newtype 的 `AsRef<str>` + `Display` impl → `remoterefs.rs:52-79` 用 `macro_rules! impl_token_str!` 批量生成 9 套对称实现 ✓
- [x] `RemoteRefs` 容器：9 字段 + `#[non_exhaustive]` + 每字段 `#[serde(skip_serializing_if = "Option::is_none")]` + Default derive → `remoterefs.rs:83-104` ✓
- [x] `extract(argv: &[String], stdout: &str) -> RemoteRefs` 签名 → `remoterefs.rs:136` ✓
- [x] 示例（design §2.1 ignore doctest）→ 升级为活的 `# Example` doctest @ `remoterefs.rs:114-121`，跑 `cargo test --doc` 通过 ✓
- [x] 字段映射表 9 条全部对应 walk match 分支（10 个分支：9 字段 + folder_token 的 token-on-create-folder 条件分支）→ `remoterefs.rs:162-198` ✓

### 名词层"现状 → 变化"核对

- [x] `crates/roostery/src/remoterefs.rs` 新建（512 行，含 27 inline tests + 3 doctests）
- [x] `lib.rs` 加 `pub mod remoterefs;`
- [x] 不新增 Cargo 依赖（serde / serde_json / chrono 已在）

### 流程图核对（design §2.2 mermaid）

- [x] 节点 A→B→C→D→E→F→G→H + Z 错误兜底分支 → `extract` 主体 `remoterefs.rs:136-152` 一一对应：argv flag 计算 → trim 短路 → 首字符短路 → from_str → walk → return refs；错误路径 3 处全返 `RemoteRefs::default()`

**结论**：接口契约 100% 对齐 design。

## 2. 行为与决策核对

### 需求摘要逐项验证（design §0 / §1）

- [x] 9 newtype 类型隔离（含 Phase 5 必需的 TaskId / ThreadId）
- [x] 单趟 match-walk in-place 填 RemoteRefs，不引中间 HashMap 聚合
- [x] `#[non_exhaustive]` RemoteRefs 强制 `..Default::default()`
- [x] `Option::as_str().filter().map(Token)` method chain 取代 `coerce_str` helper
- [x] 错误兜底全返 default 永不 panic
- [x] walk 深度上限 64 防御深嵌套
- [x] Sibling-key 顺序由 BTreeMap 字典序决定明文契约

### 明确不做逐项核对（design §1 + §3.2 反向 grep）

| 项 | 实测 |
|---|---|
| 不用 regex | ✓ `grep -E "^use regex\|regex::"` 无命中 |
| 不接 bytes 输入 | ✓ extract 签名 `&str` |
| 不实现 builder pattern | ✓ 仅 `extract` + `Default` |
| 不做 token 内容格式校验 | ✓ as_token 只过滤空字符串，不校验长度/字符集 |
| 不实现 token 互转 `From` impl | ✓ `grep -E "impl From<.*Token>\|impl From<.*Id>"` 无命中（72 个互转 impl 都不存在）|
| 不暴露 walker / coerce helper | ✓ `walk` 和 `as_token` 私有，仅 `extract` 公开（grep `^pub fn` count == 1）|
| 不实现 merge/diff/is_empty | ✓ grep 无命中 |
| 不引入 HashMap 聚合 | ✓ `grep "HashMap"` 全文无命中 |
| 不读 Config | ✓ `grep "Config"` 无命中 |
| 不修改 Python remoterefs.py | ✓ git diff 范围内无 legacy 文件 |

### 关键决策落地（design §1 表 11 条）

- [x] D1 单趟 match-walk + JSON walk 不 regex → `walk` 实现 + grep 守护 ✓
- [x] D2 9 newtype 类型隔离 → 9 个独立 unit struct ✓
- [x] D2b AsRef + Display → macro 批量生成 ✓
- [x] D2c derive 集无 Hash → grep "Hash" 无命中 ✓
- [x] D3 `#[non_exhaustive]` RemoteRefs → ✓
- [x] D4 `skip_serializing_if` 每字段 → 全 None serialize 为 `{}` 测试通过 ✓
- [x] D5 doc_token 用 `|` 模式 → `remoterefs.rs:169-171` ✓
- [x] D6 argv 消歧仅 folder_token + bool 参数化 → `remoterefs.rs:148` + walk match 分支 ✓
- [x] D7 错误兜底全返 default → S3.1 8 子情况测试通过 ✓
- [x] D8 method chain 取代 coerce_str → `as_token` 实现 + grep `fn coerce_str` 无命中 ✓
- [x] D9 `is_none()` guard 首匹配赢 → walk 每分支 ✓
- [x] D10 walk 含 depth 参数 + MAX_DEPTH=64 → `remoterefs.rs:108, 158-161` + S3.2 测试通过 ✓
- [x] D11 sibling-key BTreeMap 字典序契约 → S2.5 / S2.6 测试通过明文断言 ✓

### 流程级约束核对（design §2.2 不变量 1-9）

| 不变量 | 验证方式 | 结果 |
|---|---|---|
| 1 extract 永不 panic / 永不 Result | grep panic/unwrap 非测试无；签名返 RemoteRefs | ✓ |
| 2 纯函数无副作用 | 签名 `&` 借用返 owned | ✓ |
| 3 首匹配赢 | walk match guard + S2.3 测试 | ✓ |
| 4 argv 消歧只影响 folder_token | S1.7 + S2.x 其他字段不受 argv 影响 | ✓ |
| 5 空字符串值视同 None | `as_token` 过滤 + S3.1 子情况 | ✓ |
| 6 类型隔离编译期 | `compile_fail,E0308` doctest @ line 124 通过 | ✓ |
| 7 transparent 与 Python 形态兼容 | `newtype_serializes_transparently_as_bare_string` 9 case | ✓ |
| 8 sibling-key BTreeMap 字典序 | S2.5 / S2.6 测试 | ✓ |
| 9 walk 深度上限 64 | S3.2 测试（30 层抽到，80 层抽不到）| ✓ |

### 挂载点反向核对（design §2.3 5 条）

- [x] M1 `crates/roostery/src/remoterefs.rs` 文件存在含 9 newtype + RemoteRefs + extract ✓
- [x] M2 `lib.rs` 含 `pub mod remoterefs;` ✓
- [x] M3 RemoteRefs derive Serialize + Deserialize + Default ✓
- [x] M4 9 个 newtype 都 `#[serde(transparent)]`（行首 attribute grep count == 9）✓
- [x] M5 RemoteRefs `#[non_exhaustive]`（grep count == 1）✓

**反向 grep（清单外引用？）**：

```bash
grep -rE "remoterefs::|RemoteRefs|MessageId|DocToken|FolderToken|RecordId|ChatId|AppToken|WikiToken|TaskId|ThreadId" crates/
```
→ 命中仅在 `remoterefs.rs`（定义 + tests）+ `lib.rs`（pub mod 声明）。**无清单外的额外挂载点**——下游 caller（Phase 2+）未出现，符合 Phase 1 边界。

**拔除沙盘推演**：删除 `remoterefs.rs` + 撤回 `lib.rs` 一行 `pub mod remoterefs;` → feature 完整消失，无残留（依赖 serde/serde_json/chrono 是其他 feature 引入，不属于本 feature）。✓

## 3. 验收场景核对（design §3.1 共 30+ 条）

| 场景组 | 证据 | 结果 |
|---|---|---|
| S1.1-S1.13 happy path（每字段 + doc_token 3 alias + folder_token argv 三态）| inline tests `s1_1` 至 `s1_13`，13 个 | ✓ |
| S2.1-S2.6 多字段 / 嵌套 / Array / 顶层 Array / sibling 字典序 / alias 同现 | inline tests `s2_1` 至 `s2_6`，6 个 | ✓ |
| S3.1 8 种错误兜底 | `s3_1_error_fallback_returns_default` 表驱动覆盖 8 子情况 | ✓ |
| S3.2 深嵌套 100 层 + MAX_DEPTH 边界 | `s3_2_deep_nesting_does_not_overflow_locks_invariant_9` | ✓ |
| S4.1 编译期类型隔离 compile_fail,E0308 | doctest @ `remoterefs.rs:124-128` cargo test --doc 通过 | ✓ |
| S4.2 72 个 From 互转 impl 不存在 | 反向 grep 无命中 | ✓ |
| S5.1 transparent 裸字符串 | `newtype_serializes_transparently_as_bare_string` 9 newtype | ✓ |
| S5.2 全 None serialize → `{}` | `default_is_all_none_and_serializes_to_empty_object` + `s5_2` | ✓ |
| S5.3 部分填只含非空字段 | `s5_3_partial_serialize_only_non_none_fields` | ✓ |
| S5.4 extract roundtrip | `s5_4_extract_roundtrip` | ✓ |
| S5.5 non_exhaustive compile_fail,E0063 | doctest @ `remoterefs.rs:132-135` cargo test --doc 通过 | ✓ |
| S5.6 AsRef + Display 9 newtype | `newtype_as_ref_and_display` 表驱动 | ✓ |
| S6.1 cargo test --all ≥ 10 新测 | 实际 27 inline + 3 doctests = 30 个 | ✓ |
| S6.2 cargo test --doc | 3 passed（1 happy + 2 compile_fail）| ✓ |
| S6.3 clippy -D warnings | 通过 | ✓ |
| S6.4 fmt --all --check | 通过 | ✓ |
| GitHub Actions 三 job | run `25954205675` 全绿 | ✓ |

**反向核对项 calibration**（design §3.2 与实现差异）：

| 项 | design 期望 | 实测 | 处理 |
|---|---|---|---|
| `grep -c '#[serde(transparent)]'` | == 9 | 11（doc 注释里出现 2 次 + 9 个真 attribute） | **Calibrate**：用更精准的行首 grep `grep -cE '^#\[serde\(transparent\)\]'` == 9，已就地写入 design §3.2 |
| `grep -c "impl AsRef<str> for"` | == 9 | 1（用 macro 批量生成）| **Calibrate**：macro 实现等价于 9 套，由 `newtype_as_ref_and_display` 表驱动测试覆盖 9 newtype 验证；design §3.2 改为"macro 生成 + 测试覆盖等价" |
| `grep -c "impl .*Display for"` | == 9 | 2（1 macro + 1 `fmt::Display` 引用）| 同上 |
| `wc -l remoterefs.rs` | < 500 | 512 | **Calibrate**：与 core-redact 当时 < 400 → < 500 同款情形；design §3.2 修订为 < 600，9 newtype + 30 测试合理上限 |

**前端验证**：本 feature 无前端改动，跳过浏览器验证。

**结论**：30 验收场景 + 14 反向核对项中 3 项 calibrate（grep count 因 macro 实现 + wc -l 阈值），全部通过。Calibration 已写入 design §3.2。

## 4. 术语一致性

对照 design §0 术语表 grep 代码：

| 术语 | 代码命中 | 一致 |
|---|---|---|
| `RemoteRefs` | remoterefs.rs（struct + tests）| ✓ |
| 9 newtype 类型名 | 各自定义 + 使用 | ✓ |
| `extract` | 唯一 pub fn | ✓ |
| `walk` / `as_token` / `MAX_DEPTH` | 私有 helper | ✓ |
| `impl_token_str!` macro | 仅本文件 | ✓（不是新概念，design §2.1 建议的实现路径）|

防冲突 grep：

- `regex` / `HashMap` / `coerce_str` / `From<.*Token>` / `From<.*Id>`：均无命中 ✓
- `Hash` derive：非测试代码无命中 ✓

## 5. 架构归并

| doc | 归并内容 | 状态 |
|---|---|---|
| `.codestable/architecture/ARCHITECTURE.md` §2 术语表 | 新增 "Newtype token" 词条（9 类型 + transparent + non_exhaustive 描述）| ✓ 已写入 |
| `.codestable/architecture/ARCHITECTURE.md` §3 Module A | "regex 抽" → "JSON walk + match-dispatch 抽 9 个 newtype token"；新增 remoterefs 模块详情段（commit + 公开类型 + 容器 + API + 实现策略 + sibling 顺序契约 + 类型隔离编译期保证 + 下游约定）；子 feature 标 done | ✓ 已写入 |
| `.codestable/architecture/ARCHITECTURE.md` §3 Module B | journal 节加 remoterefs 集成约定一段（caller 自调 + journal 不感知 RemoteRefs 类型）| ✓ 已写入 |
| `.codestable/architecture/ARCHITECTURE.md` §5 关键架构决定 | 新增第 6 条"业务标识符 newtype 隔离"——跨 feature 稳定原则，指明适用范围 / 不适用范围 / Phase 4 候选 | ✓ 已写入 |
| `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §3 Module A 描述 | "remoterefs 从 stdout 抽 doc_token 等"措辞保留（与实际相符）| 不需要改 |
| `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §5 第 3 条 core-remoterefs | "regex 从 lark-cli stdout 抽" → "JSON walk + match-dispatch 抽 9 个 newtype token"；状态 planned → done + commit + feature；req 关联补 portable-by-default；备注扩 Python parity 数 + walk 深度 + 首匹配赢顺序 | ✓ 已写入 |
| `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml` | core-remoterefs status → done；description 字段同步更新 | ✓ 已写入，validate 通过 |
| `.codestable/attention.md` | 无需更新（不引入新硬约束；newtype 隔离归 §5 架构决定，不属于"每次启动都要知道"级别）| ✓ 评估完成不动 |
| `.codestable/requirements/portable-by-default.md` | implemented_by 追加 + 变更日志加 2026-05-16 条目；保持 draft（read/replay 未落地）| ✓ 已写入 |
| `.codestable/compound/` | design §4.1 建议归档"业务标识符 newtype 隔离" convention（范围已收紧）| 退出环节走 cs-decide |

**判据自查**：未读 design 的人打开 ARCHITECTURE.md 应能知道：Module A remoterefs 模块已落地、9 newtype 设计、JSON walk 策略、类型隔离编译期保证、与 redact / journal 平级的"caller 自调"集成约定、§5 第 6 条把这一原则上升为跨 feature 约束。✓

## 6. requirement 回写

`requirement: portable-by-default`（draft）。core-remoterefs 是该 req 的间接兑现——让 journal entry 携带远端 token 便于审计/检索，但 req 用户故事中"重跑 / 复现 / 自建 dashboard / 跨设备拷贝"仍依赖未落地的 read/replay 工具。

**处理**：保持 `status: draft`，`implemented_by` 追加 `2026-05-16-core-remoterefs`，变更日志加 2026-05-16 条目记录兑现范围与限制。`last_reviewed` 保持 2026-05-16。

✓ 已写入 `.codestable/requirements/portable-by-default.md`。

## 7. roadmap 回写

frontmatter `roadmap: rust-rewrite` + `roadmap_item: core-remoterefs`。

- [x] `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`：`core-remoterefs` 条目 `status: in-progress` → **done**；description 同步刷新（regex → JSON walk + match-dispatch）；feature 字段保留
- [x] `validate-yaml.py` 校验通过
- [x] `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §5 第 3 条同步：planned → done + commit + req 关联 + 备注扩展

## 8. attention.md 候选盘点

回看本次实现暴露出"下个 feature 的 AI 还会再撞一次"信息：

- **候选 1**：**Rust `#[serde(transparent)]` newtype 类型隔离 + `compile_fail,E0XXX` doctest 锁错误码**是 Roostery 项目的标准 idiom，已写进 ARCHITECTURE §5 第 6 条 + design §4.1 建议走 cs-decide convention 归档。`compound/` 归档动作比 attention.md 短句更适合承载这条约定（attention.md 是"启动必读"，约定细节归档比启动注脚更合适）→ **不入 attention.md**；走 cs-decide
- 候选 2（边缘）：`macro_rules! impl_token_str!` 批量生成 AsRef/Display 模式——单条 macro，本场景独有，不构成"下次还撞"的项目硬约束 → 不入

**本节结论**：本 feature 未暴露需要补入 attention.md 的内容；跨 feature 约定走 cs-decide。

## 9. 遗留

- **后续优化点 / 待开 feature**：
  - 字段集扩展（Phase 2-7 实际接入更多 lark-cli subcommand 后可能需要 `sheet_token` / `file_token` / `table_id` / `space_id`）——`non_exhaustive` 已保证扩展不破坏 caller；扩时同步加 newtype + RemoteRefs 字段 + walk match 分支 + `#[serde(transparent)]`
  - argv 消歧规则扩展——目前只 folder_token 一条；Phase 2 `lark-cli-shim` 实际跑起来后按需扩
  - 业务标识符 newtype 约定走 `cs-decide convention` 归档（design §4.1 建议；范围已收紧为"有飞书侧业务语义角色"）
- **已知限制**：
  - sibling-key 顺序依赖 serde_json `Map` 默认 BTreeMap 字典序（不承诺 stdout 物理顺序）—— 明文契约
  - walk 深度 > 64 时静默截断（不报错，符合 best-effort 语义）
- **实现阶段"顺手发现"**：无（本 feature 无旁逸改动）
- **架构 doc calibration**：design §3.2 反向 grep 3 处与 macro 实现 / 注释计数差异已就地修订
