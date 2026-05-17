---
doc_type: decision
category: convention
slug: rust-module-organization
status: active
created: 2026-05-16
updated: 2026-05-17
tags: [rust, module-layout, codebase-structure, cargo-bin-target]
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

例（计划中）：如 `journal` 未来要给外部社区做 jsonl reader 库可能独立成 `crates/roostery-journal/`。

### 档 4：Cargo bin target（自 lark-cli-shim 起补档）

主 crate 内的辅助二进制（shim / 安装钩子 / 工具脚本等不是 user-facing 主程序的 bin），落 `crates/roostery/src/bin/{name}.rs`——Cargo 自动发现机制 + `Cargo.toml` 显式 `[[bin]] name = "{name}" path = "src/bin/{name}.rs"` 段稳定名字。同 crate 自动复用 lib 模块（`use roostery::journal::...`）零成本。

适用：

- 辅助二进制（非 user-facing 主程序）
- 单文件 < 500 行（含 inline tests）
- 同 crate lib 模块复用成本低于独立 crate 的隔离收益

例：`crates/roostery/src/bin/shim.rs`（feature `2026-05-17-lark-cli-shim`，PATH-prefix 透传 + journal 写入，单文件 ~310 产品代码 + ~210 内联测试）。

**与档 1-3 的关系**：

- 主程序 bin（user-facing CLI，本项目的 `roostery`）：用 Cargo 默认的 `src/main.rs`，**不**进 `src/bin/`
- 一个档 4 的 bin 预估超 500 行或内部模块化需求显著 → 升级到 `src/bin/{name}/main.rs` + 子模块（仍同 crate；Cargo 自动识别）
- 当 bin 不需要 lib 的 transitive deps（如 shim 不用 tokio 但同 crate 已传递引入 tokio）且**二进制 size 敏感**——若 release LTO 后实测 > 5 MB → 升档 3 独立 workspace crate

### 命名

- 文件名 / 模块名 `snake_case`
- 子目录名 = 模块名（不加 `_mod` / `_lib` 后缀）
- 测试模块统一 `#[cfg(test)] mod tests { ... }` 写在被测文件末尾（inline tests）；超过文件 50% 行数时拆到 `crates/roostery/tests/{name}.rs`（integration tests）

### 升档触发与执行

- 单文件即将 > 500 行 → 该文件所属的下一个 feature 在 design §2.5 评估是否升档 2，按"只搬不改行为"标准独立成 step 跑
- 档 4 bin 单文件即将 > 500 行 → 升级到 `src/bin/{name}/main.rs` + 子模块（同 crate）
- 档 4 bin release LTO 后二进制 > 5 MB（size-sensitive 场景如 shim 每次 lark-cli 调用都启动） → 评估升 档 3 独立 crate
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
| **档 4 辅助 bin 直接独立 crate**（2026-05-17 评估）| 增加 workspace member 配置成本 + 失去同 crate lib 模块零成本复用；只在 size-sensitive 且实测超阈值时才值得，作为档 4 升档触发而非默认选择 |
| **档 4 不用 `src/bin/` 而用 subcommand 嵌进 `src/main.rs`**（2026-05-17 评估）| user-facing CLI 和辅助工具混在同一二进制；shim 装机点是 `~/.local/bin/lark-cli`（独立路径），与 `roostery init` 等 user-facing 命令本质不同——subcommand 形式无法满足 PATH-prefix 透传场景 |
| **档 4 不显式声明 `[[bin]]` 段，只靠 Cargo 文件名自动发现**（2026-05-17 评估）| Cargo 确实能自动发现 `src/bin/*.rs`，但显式段把"二进制名"钉成稳定契约（agent runtime 装机脚本依赖名字稳定），避免未来重命名 `src/bin/shim.rs` 时静默改变 bin 名 |

## 影响 / 后续约束

- **feature design §2.5 强制检查项**：写到"要落新文件的目录"评估时，必须按本档位三档做出明确归类，不允许"暂时单文件以后再说"含糊
- **新模块加入触发**：任何子 feature 引入 `crates/roostery/src/` 新文件时，要在 design §2.5 显式声明走档 1（默认）/ 档 2 / 档 3 哪一档
- **升档动作必须独立 commit**：从档 1 升到档 2 的重组（拆文件 / 移动 mod 声明）按 cs-feat-design §2.5 "只搬不改行为"标准做，独立 step + 独立 commit，不混在功能 feature 里
- **不溯及既往**：现有 5 个文件全在档 1 区间，不强制重组；下次新增 / 现有文件升档时按本规约
- **新加 bin target 触发**：任何子 feature 引入 `crates/roostery/src/bin/` 新文件时，design §2.5 显式声明走档 4（默认）/ 升档 3（size-sensitive 实测验证后）；不允许"先放 src/bin/ 以后看情况"含糊
- **审视周期**：
  - Phase 2（lark-cli-wrapper / smoke / shim）落地后回看一次——届时 src/ 会有 ~8 文件，验证档位阈值是否仍合理；不合理走本 decision 的 `update` 流程（不 supersede，结论本身没变）
  - **2026-05-17 update**：Phase 2 收尾审视——三档（档 1/2/3）阈值经 redact / journal / lark_cli 验证合理；新增第 4 档（Cargo bin target）覆盖 shim 路径。下次审视点 Phase 5（bot bridge feature 落地后，预计触发新的辅助 bin 如 stop-hook 脚本嵌入）

## 相关文档

- `.codestable/features/2026-05-15-core-redact/core-redact-design.md` §2.5：首次 flag 本约定的需要
- `.codestable/features/2026-05-15-journal-core/journal-core-design.md` §2.5：第二次 flag，触发本归档
- `.codestable/features/2026-05-17-lark-cli-shim/lark-cli-shim-design.md` §2.5：识别出档 4 Cargo bin target，跑通后由 acceptance 阶段触发本 decision update（2026-05-17）
- `.codestable/architecture/ARCHITECTURE.md` §3 Module A-H：roadmap-level module 划分（注意：与本 Rust 文件 / 目录约定是**不同维度**——roadmap module 是规划聚合，本约定是 Rust 物理结构）
