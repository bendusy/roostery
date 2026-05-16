---
doc_type: decision
category: convention
slug: rust-module-organization
status: active
created: 2026-05-16
tags: [rust, module-layout, codebase-structure]
---

# Rust 模块组织约定

## 背景

`crates/roostery/src/` 从 Phase 0 起逐文件增长。core-redact design §2.5 + journal-core design §2.5 都 flag 过同一件事："等文件数到 5+ 时需要决定模块组织约定，否则会出现 inline `pub mod foo;` / `mod.rs` / 子目录混用"。

journal-core 落地后 `src/` 已 5 文件（`main.rs` / `lib.rs` / `redact.rs` / `journal.rs` / `paths.rs`）。再不归档，下个 feature 的 AI 会现场拍脑袋选一种结构，导致整个 crate 的组织风格逐 feature 漂移。

## 决定

`crates/roostery/src/` 采用 **flat-first，子目录按容量阈值升级** 策略，具体三档：

### 档 1：单文件 inline pub mod（默认）

模块代码自然落进 `crates/roostery/src/{name}.rs`，在 `lib.rs` 用 `pub mod {name};` 暴露。

适用：**单文件 < 500 行**（含 inline tests）；模块对外暴露的公开项 ≤ ~8 个。

例：`redact.rs`、`journal.rs`、`paths.rs`。

### 档 2：子目录 + `mod.rs`

模块拆成 `crates/roostery/src/{name}/mod.rs` + 多个同目录兄弟文件。`mod.rs` 只做 `pub mod` re-export + 模块级 doc + 极少跨子模块的胶水类型；不写业务逻辑。

适用：单文件超 500 行 **且** 内部能明确切出 2+ 个有独立测试价值的子文件；或公开项 > 8 个想分组。

例（未来）：`dispatcher/mod.rs` + `dispatcher/trace.rs` + `dispatcher/budget.rs` + `dispatcher/runners.rs`。

### 档 3：独立 crate（workspace member）

适用：模块需独立发版 / 有独立 feature flag / 跨二进制复用 / 编译期严重拖慢主 crate。

例（计划中）：`bin/shim` 独立二进制；如 `journal` 未来要给外部社区做 jsonl reader 库可能独立成 `crates/roostery-journal/`。

### 命名

- 文件名 / 模块名 `snake_case`
- 子目录名 = 模块名（不加 `_mod` / `_lib` 后缀）
- 测试模块统一 `#[cfg(test)] mod tests { ... }` 写在被测文件末尾（inline tests）；超过文件 50% 行数时拆到 `crates/roostery/tests/{name}.rs`（integration tests）

### 升档触发与执行

- 单文件即将 > 500 行 → 该文件所属的下一个 feature 在 design §2.5 评估是否升档 2，按"只搬不改行为"标准独立成 step 跑
- 不在功能 feature 里偷偷重组——结构变更必须独立 commit，编译器全程绿灯

## 为什么这样选

1. **Rust idiom 偏好 flat**：Cargo 项目惯例从单文件 crate 起步，子目录是被动响应规模而非主动设计选择。模仿 `serde` / `tokio` 等成熟 crate 早期形态
2. **mod.rs 选择而非 `{name}.rs` + `{name}/`**：Rust 2018+ 两种写法都支持；但 `mod.rs` 形式**让"这个目录是个模块"在文件树里更显眼**——在 IDE 里展开目录看不到 `{name}.rs` 兄弟时不会困惑模块入口在哪
3. **500 行硬阈值而非 300 / 400**：Rust 含 derive macro、inline tests、文档注释，行数密度本就比 Python / Go 低。core-redact 461 行 + journal-core 382 行都未触发升档，验证阈值合理
4. **子目录必经 design §2.5 审视**：把"是否拆"的决策点钉在 feature design 阶段而非 implement 中途——避免边写边重组导致 commit 里行为变更和结构变更混杂

## 考虑过的替代方案

| 方案 | 为什么没选 |
|---|---|
| **从第 1 个非平凡模块就建子目录**（如 `src/redact/mod.rs`）| 过度设计；Phase 1 的模块都 < 500 行，子目录里只塞一个文件没价值，反而增加导航深度 |
| **`{name}.rs` + `{name}/` 兄弟形式**（Rust 2018 风格）| 同样合法但 mod.rs 形式在 IDE / `tree` 输出里更明显标记模块根；本项目主开发环境（CC + 终端 `ls`）下 mod.rs 的可发现性优势具体 |
| **按 phase 分子目录**（`src/phase1/redact.rs` / `src/phase2/lark_cli.rs`）| phase 是 roadmap 概念不是稳定结构；roadmap 调整时整批文件要搬，且模块功能本身跟 phase 无关 |
| **按 roadmap module 分子目录**（`src/module_a/redact.rs`）| roadmap §3 的 Module A-H 是规划聚合，不是 Rust 模块边界——同 Module 的 feature 之间未必有共享代码或互相 import 关系（如 redact / journal / remoterefs 都在 Module A 但互相独立） |
| **不归档，用 review 兜底**| AI 没有决策上下文给出"合理但与项目规约冲突"的方案；review 是兜底不是预防 |

## 影响 / 后续约束

- **feature design §2.5 强制检查项**：写到"要落新文件的目录"评估时，必须按本档位三档做出明确归类，不允许"暂时单文件以后再说"含糊
- **新模块加入触发**：任何子 feature 引入 `crates/roostery/src/` 新文件时，要在 design §2.5 显式声明走档 1（默认）/ 档 2 / 档 3 哪一档
- **升档动作必须独立 commit**：从档 1 升到档 2 的重组（拆文件 / 移动 mod 声明）按 cs-feat-design §2.5 "只搬不改行为"标准做，独立 step + 独立 commit，不混在功能 feature 里
- **不溯及既往**：现有 5 个文件全在档 1 区间，不强制重组；下次新增 / 现有文件升档时按本规约
- **审视周期**：Phase 2（lark-cli-wrapper / smoke / shim）落地后回看一次——届时 src/ 会有 ~8 文件，验证档位阈值是否仍合理；不合理走本 decision 的 `update` 流程（不 supersede，结论本身没变）

## 相关文档

- `.codestable/features/2026-05-15-core-redact/core-redact-design.md` §2.5：首次 flag 本约定的需要
- `.codestable/features/2026-05-15-journal-core/journal-core-design.md` §2.5：第二次 flag，触发本归档
- `.codestable/architecture/ARCHITECTURE.md` §3 Module A-H：roadmap-level module 划分（注意：与本 Rust 文件 / 目录约定是**不同维度**——roadmap module 是规划聚合，本约定是 Rust 物理结构）
