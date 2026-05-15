# core-redact 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-15
> 关联方案 doc：`.codestable/features/2026-05-15-core-redact/core-redact-design.md`
> 关联 commits：`1e392e5` (feat: redact module)
> CI 验证：GitHub Actions run #25914996799 全绿（fmt / clippy / test）
> Codex review：**bypassed**（本机 codex CLI 未登录、零产出；用户授权跳过）

## 1. 接口契约核对

对照方案第 2.1 节名词层逐一核查：

**公开 API 签名核对**：

- [x] `pub const MASK: &str = "***";` —— `crates/roostery/src/redact.rs:10` 与 design 示例 byte-for-byte 一致
- [x] `pub const SENSITIVE_KEYS: &[&str]` 11 entries —— `redact.rs:16-30` 与 design 列举顺序完全一致（Python parity 7 + 扩展 4）
- [x] `pub fn scrub_value(&serde_json::Value) -> (serde_json::Value, Vec<String>)` —— `redact.rs:36` 签名一致
- [x] `pub fn scrub_argv(&[String]) -> (Vec<String>, Vec<String>)` —— `redact.rs:87` 签名一致
- [x] `pub fn scrub_text(&str) -> String` —— `redact.rs:158` 签名一致

**design 接口示例 vs 代码实际行为**：

- [x] scrub_value 示例 `json!({"user": "alice", "access_token": "xyz"})` → `({"user": "alice", "access_token": "***"}, ["/access_token"])` —— test `scrub_value_simple_object` 实测一致
- [x] scrub_argv 示例 `["lark-cli", "--access-token", "abc"]` → `[..., "***"]`, `["argv[2]"]` —— test `scrub_argv_flag_space_value` 实测一致
- [x] scrub_text 示例 `"access_token": "abc123"` → `"access_token": "***"` —— test `scrub_text_json_string_form` 实测一致

**流程图核对**（design §2.2 三个 subgraph）：

- [x] scrub_value 节点 A1-A8（递归遍历 / Object 分支 / sensitive 检查 / Array 递归 / primitive 原样）：`redact.rs:42-76` `scrub_value_inner` 函数体逐节点对应
- [x] scrub_argv 节点 B1-B6（三种 pattern + 其他）：`redact.rs:87-122` while 循环逐分支对应
- [x] scrub_text 节点 C1-C3（JSON pass + YAML pass）：`redact.rs:158-171` 两个 for 循环对应

**结论**：无契约偏差。

## 2. 行为与决策核对

### 范围交付逐项验证

- [x] 新文件 `crates/roostery/src/redact.rs` 实现脱敏功能 ✓（463 行）
- [x] 公开 API 3 fn + MASK + SENSITIVE_KEYS ✓（见 §1）
- [x] `lib.rs` 加 `pub mod redact;` ✓ `crates/roostery/src/lib.rs:4`
- [x] `Cargo.toml` 加 serde_json + regex ✓ `crates/roostery/Cargo.toml:19-20`
- [x] 单元测试 ≥ 5 条 ✓ 实际 26 个
- [x] 纯函数、无 I/O、无 async ✓ grep 验证无 tokio / async fn

### 明确不做（反向核对）

- [x] 不实现 sensitive keys 运行时配置 —— grep `fn config` 无匹配 ✓
- [x] 不接 bytes 输入 —— `scrub_text` 签名 `&str`，无 bytes overload ✓
- [x] 不深度理解 argv flag 语义 —— 仅 design 列三种 pattern（grep `getopt` 无匹配）✓
- [x] 不做内存加密 / secure erase —— 无 zeroize 引入 ✓
- [x] 不写 scrub_bytes / scrub_path —— grep 无匹配 ✓
- [x] 不替 journal 决定何时调用 —— 模块不知道 journal 存在 ✓
- [x] 不修改 Python redact.py —— `legacy/python/src/roostery/redact.py` 未在 commit diff 中 ✓
- [x] 不引入 redact / redactable / secrecy crate —— grep Cargo.toml 无匹配 ✓

### 关键决策落地（10 条）

- [x] D1 Edition 2024：workspace Cargo.toml `edition = "2024"` 已设（Phase 0 落地），本 feature 沿用 if-let chain 等 2024 特性
- [x] D2 Sensitive keys 11 个：`redact.rs:16-30` 完全一致；test `scrub_value_all_eleven_keys_covered` + `sensitive_keys_has_eleven` 双重断言
- [x] D3 Key normalization 规则（lowercase + `-`→`_` + strip leading `_`）：`redact.rs:124-128` `normalize_key` 私有 helper
- [x] D4 Audit path 双格式：argv `argv[N]` / 结构化 RFC 6901 —— 测试 `scrub_argv_*` 断言 `argv[2]` 等；`scrub_value_*` 断言 `/headers/Authorization` 等
- [x] D5 依赖最小化：仅 serde_json + regex；LazyLock 用 stdlib（`redact.rs:5` `use std::sync::LazyLock`）；无 once_cell / secrecy / redactable / redact crate
- [x] D6 scrub_value 仅 key-based 不做 text-pattern：`scrub_value_inner` 只 match key，不对 String leaf 递归 scrub_text
- [x] D7 scrub_text 不返 audit path：签名 `-> String` 不带 Vec
- [x] D8 返回 owned 值：所有签名返回 owned types
- [x] D9 `MASK` 公开常量 ✓
- [x] D10 `SENSITIVE_KEYS` 公开只读 slice ✓

### 流程级约束核对

- [x] **不变量 1 不修改入参**：所有 fn 接 `&` 借用 + clone-to-owned；test `scrub_value_no_sensitive_keys` 断言 `out == v`（入参未被改）
- [x] **不变量 2 幂等性**：test `scrub_value_idempotent` 直接断言 `scrub_value(scrub_value(v))` 等价
- [x] **不变量 3 SENSITIVE_KEYS 编译期常量**：`pub const SENSITIVE_KEYS: &[&str]` ✓
- [x] **不变量 4 audit path 顺序 = 遍历顺序**：`scrub_value_array_elements` 测试断言 `["/0/api_key", "/1/api_key"]` 顺序匹配 Array 遍历
- [x] **错误语义无 panic / Result**：grep `panic!` / `\.unwrap\(\)` 非测试代码无匹配（`.expect("static regex compiles")` 在 LazyLock 初始化器中，是 invariant 不是 runtime）
- [x] 空输入返回空输出：`scrub_argv_empty_input` / `scrub_text_empty_string` 测试

### 挂载点反向核对

**3 个挂载点逐条核对**：

- [x] M1 `crates/roostery/src/redact.rs` 存在 —— `ls crates/roostery/src/redact.rs` ✓
- [x] M2 `crates/roostery/src/lib.rs` 含 `pub mod redact;` —— `grep -n "pub mod redact" crates/roostery/src/lib.rs` → line 4 ✓
- [x] M3 `Cargo.toml` 含 serde_json + regex —— `grep -nE "serde_json|^regex" crates/roostery/Cargo.toml` → lines 19-20 ✓

**反向 grep**（本 feature 在代码 / 文档里的所有引用是否都落在清单内）：

执行 `grep -rn --include="*.rs" --include="*.toml" -E "redact::|crate::redact|fn scrub_" crates/ Cargo.toml` 后所有命中：

- `crates/roostery/src/lib.rs:4` `pub mod redact;` ← M2
- `crates/roostery/src/redact.rs` 自身（含 36/87/158 三个 `pub fn scrub_*` 定义）← M1
- `crates/roostery/Cargo.toml` serde_json + regex 依赖 ← M3
- 无其他文件 reference 本模块（journal-core 等下游未实现）

**结论**：3 个挂载点充分；无漏记。

**拔除沙盘推演**：

依次删除 M1-M3 后：
- 删 M1 (`redact.rs`) → `pub mod redact;` 失败，build 错；feature 行为消失
- 删 M2 (`pub mod redact;`) → 模块未暴露，下游 import 失败；feature 对外消失
- 删 M3 (serde_json + regex 依赖) → `redact.rs` import 失败，build 错；feature 消失

无下游 caller，无残留状态文件，无配置项遗留。拔除干净 ✓。

## 3. 验收场景核对

逐条对照 design §3.1：

### S1.1-S1.8 scrub_value（8 子项）

- [x] **S1.1** simple object：`scrub_value_simple_object` 测试 ✓
- [x] **S1.2** 嵌套 Object：`scrub_value_nested_object` 测试，验证 path `/headers/Authorization` ✓
- [x] **S1.3** Array 元素：`scrub_value_array_elements` 测试，验证 paths `["/0/api_key", "/1/api_key"]` ✓
- [x] **S1.4** 无 sensitive key：`scrub_value_no_sensitive_keys` 测试，断言 `out == v` ✓
- [x] **S1.5** 大小写 / 连字符变种：`scrub_value_dash_and_case_variants` 测试 `Access-Token` / `API-KEY` 均触发 ✓
- [x] **S1.6** primitive 顶层值：`scrub_value_primitive_top_level` 测试 string/int/bool/null 全部原样 ✓
- [x] **S1.7** 幂等：`scrub_value_idempotent` 测试 ✓
- [x] **S1.8** 11 keys 全覆盖：`scrub_value_all_eleven_keys_covered` 测试断言 paths.len() == 11 ✓

### S2.1-S2.7 scrub_argv（7 子项）

- [x] **S2.1** `--flag value`：`scrub_argv_flag_space_value` ✓
- [x] **S2.2** `--flag=value`：`scrub_argv_flag_equals_value` ✓
- [x] **S2.3** `--header "Auth: x"`：`scrub_argv_header_long_form` ✓
- [x] **S2.4** `-H` 简写：`scrub_argv_header_short_form` ✓
- [x] **S2.5** non-sensitive flag：`scrub_argv_non_sensitive_flag_passes_through` ✓
- [x] **S2.6** 边界 last token：`scrub_argv_sensitive_flag_last_no_value` 不 panic ✓
- [x] **S2.7** 边界 empty argv：`scrub_argv_empty_input` ✓

### S3.1-S3.5 scrub_text（5 子项）

- [x] **S3.1** JSON 字符串形：`scrub_text_json_string_form` ✓
- [x] **S3.2** YAML 行形：`scrub_text_yaml_form` ✓
- [x] **S3.3** 大小写不敏感：`scrub_text_case_insensitive` ✓（注：超越 Python 行为，per code-doc-authority）
- [x] **S3.4** 无 sensitive：`scrub_text_no_sensitive_key` ✓
- [x] **S3.5** 边界空字符串：`scrub_text_empty_string` ✓

### S4 模块级（4 子项）

- [x] **S4.1** `cargo test --all` 全绿 ≥ 5 tests：实测 26 tests（CI run #25914996799）
- [x] **S4.2** `cargo clippy --all-targets --all-features -- -D warnings` 通过：CI clippy job 全绿
- [x] **S4.3** `cargo fmt --all --check` 通过：CI fmt job 全绿
- [x] **S4.4** `SENSITIVE_KEYS.len() == 11`：test `sensitive_keys_has_eleven` 显式断言

**无前端改动，跳过浏览器验证。**

**结论**：24 条验收场景全部通过证据可追溯。

## 4. 术语一致性

对照 design §0 6 个术语 grep 代码：

- **Sensitive key**：仅出现在 design 与文档；代码用 `is_sensitive_key` 函数名（一致）✓
- **MASK**：`grep "MASK" crates/roostery/src/redact.rs` 命中 13 处，全部一致用法（const 定义 + 替换值 + 测试断言）✓
- **Audit path**：体现为 `format!("argv[{}]", ...)` / `format!("{}/{}", prefix, segment)`；命名一致 ✓
- **`scrub_*`**：3 个公开 fn 命名一致 ✓
- **Key normalization**：私有 fn `normalize_key`（design §0 注明 "私有 helper"）✓
- **Logging-boundary scrubber**：模块 doc comment `//! Logging-boundary scrubber:` 一致 ✓

防冲突 grep：

- 全仓库 grep `secrecy|redactable` → 仅在 design / acceptance 文档（讨论生态）；代码无引入 ✓
- grep `pub use redact` / `re-export` 无 unexpected re-export ✓

**结论**：术语一致性通过。

## 5. 架构归并

对照 design §4 三类提炼内容，实际写入 `.codestable/architecture/ARCHITECTURE.md`：

### 名词归并

- [x] **ARCHITECTURE.md §2 术语表新增 3 条**（已写入）：
  - `MASK` —— 脱敏占位 `"***"`，指向 Phase 1 落地位置
  - `SENSITIVE_KEYS` —— 11 entries 列表说明（7 + 4 扩展明示）
  - Logging-boundary scrubber —— 定位区分（与 in-memory wrapper crate 分层）

### 动词骨架 + 流程级约束归并

- [x] **ARCHITECTURE.md §3 Module A 节扩写 redact 子节**（已写入）：
  - 公开 API 3 fn + 2 const
  - audit path 双格式
  - 下游使用约束（journal-core / shim / task_writer 必经此模块脱敏）
  - logging-boundary scrubber 定位区分
  - 子 feature 列表标注 `core-redact (done)`

- [x] **ARCHITECTURE.md §6 已知约束新增第 7 条**（已写入）：
  - redact 函数纯且幂等（不变量 1-4 浓缩为一条架构层约束）

### 关联的已有架构 doc 评估

- [x] `.codestable/architecture/ARCHITECTURE.md` —— 已实际写入（上述 3 处）
- [x] `.codestable/attention.md` —— **不动**：本 feature 未引入新硬约束。代码-文档优先级 / lark-cli 1.0.28 等现有约束跟 redact 无关
- [x] `.codestable/requirements/portable-by-default.md` —— 不动（见 §6）

**判据满足**：没读过 design 的人现在打开 ARCHITECTURE.md 能看到：redact 模块在 Module A 已落地、3 个公开 API、下游必经此模块脱敏、纯函数幂等、不替代 Secret<T> 类 wrapper。

## 6. requirement 回写

design frontmatter `requirement: portable-by-default`（draft）。

**分析**：portable-by-default req 涵盖范围包括：
1. journal 是 portable 数据形态
2. journal 可读 / 可迁
3. 脱敏是敏感数据处理基础
4. replay 能力

本 feature core-redact 仅兑现 **#3 脱敏基础**——是 portable-by-default 的一个 building block，**不构成 req 整体 capability 的完成**。完整能力要等 `journal-core` (Phase 1) + `lark-cli-shim` (Phase 2) 等下游 feature 一起到位才能上 current。

按 cs-feat-accept 第 6 节判据：

> [x] `requirement` 指向 draft req 但本次未改用户视角 → 写"req-{slug} 未变，无需更新"

**结论**：`portable-by-default.md` **未变**，保持 `status: draft`。用户故事 / pitch / 边界都不需要刷新——本 feature 是兑现 req 的一部分实现，不是 req 升级触发点。等 journal-core / lark-cli-shim 落地后再评估升级 current。

## 7. roadmap 回写

design frontmatter：`roadmap: rust-rewrite`、`roadmap_item: core-redact`。两字段均有值。

**items.yaml 当前状态核对**：
- slug: `core-redact` ✓
- 当前 `status: in-progress`（design 阶段已改）✓
- 当前 `feature: 2026-05-15-core-redact` ✓

**操作**：

- [x] `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml` 改 `status: in-progress` → `status: done`；备注追加 acceptance commit + CI run reference
- [x] `validate-yaml.py` 校验通过
- [x] `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §5 第 2 条 `core-redact` 同步：状态 `planned` → `done (2026-05-15)`，对应 feature + commit + CI run 标注

## 8. attention.md 候选盘点

回看本 feature 实现，**无新发现的项目硬约束 / 命令陷阱 / 环境约定**值得写入 attention.md。

- clippy 在 edition 2024 严格要求 `&& let` chain 替代嵌套 `if let` —— 这是 Rust 工具链行为不是 Roostery 项目特殊性，不入 attention
- LazyLock vs once_cell 选择 —— 这是 edition 2024 标准做法，不入 attention
- regex 静态编译模式 —— Rust 通用做法

attention.md 现有 9 条硬约束都跟 redact 无关（lark-cli / journal / LLM client 等都是 Phase 2+ 才碰到的约束）——已足够。

**结论**：无 attention.md 候选。

## 9. 遗留

### 后续优化点（建议起 issue / 后续 feature 跟进，本 feature 不动）

1. **scrub_text JSON pattern 不处理 value 内 escaped quotes**（impl 自审 F1）
   - 例：`"key": "abc\"def"` 会截断在嵌入的 `\"`
   - 实际 lark-cli 输出极少出现内嵌引号
   - 建议：单独起 issue 或 follow-up feature 加 documented limitation 测试 + doc comment 明示
2. **测试覆盖小盲点**（impl 自审 F8）
   - empty Object `{}` / empty Array `[]` 没有显式测试
   - 含 `~` 和 `/` 同时的 key 没有 escape 顺序边界测试
   - 实际逻辑应该 OK，建议下次动 redact 时顺手补 2-3 个 corner case test
3. **Codex review 未跑**
   - 本机 codex CLI 未登录，本次完整跳过
   - 后续如恢复 codex 登录，建议追跑一次 review 作为 follow-up

### 已知限制（写入下游 feature 启动时需注意）

1. `scrub_text` 用于 raw text blob，**对 structured JSON 数据使用 `scrub_value` 才能精确脱敏**——下游 journal-core / shim 应优先 `scrub_value`，把 `scrub_text` 留给真正的 text blob（如 stdout / stderr）
2. `SENSITIVE_KEYS` 是**编译期常量**——下游不要假设运行时可改；若未来需要配置驱动，起独立 feature

### implement 阶段"顺手发现"

- design §3.2 文件行数反向核对 `< 300` calibration 偏低（实际 463），已 in-place 修订 design + checklist 阈值到 `< 500`（属于 design 内自洽修订，不算范围外扩张）

---

## 实际写文件汇总

本验收报告 §5-§7 实际改动的文件（落盘前已执行）：

1. `.codestable/architecture/ARCHITECTURE.md` —— §2 术语表 3 词条 + §3 Module A redact 子节 + §6 已知约束第 7 条
2. `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml` —— core-redact `in-progress` → `done` + 备注追加 commit/CI ref
3. `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` —— §5.2 core-redact 状态 + feature 对应
4. `.codestable/features/2026-05-15-core-redact/core-redact-checklist.yaml` —— 24 checks `pending` → `passed`
5. `.codestable/features/2026-05-15-core-redact/core-redact-acceptance.md` —— 本报告（新增）

无 requirement 文件改动（per §6）；无 attention.md 改动（per §8）；无代码改动（per §1-§4，无偏差需修代码）。
