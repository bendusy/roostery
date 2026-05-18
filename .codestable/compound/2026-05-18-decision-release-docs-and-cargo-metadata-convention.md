---
doc_type: decision
slug: release-docs-and-cargo-metadata-convention
category: convention
status: active
created: 2026-05-18
tags: [release, semver, readme, changelog, cargo-metadata, crates-io, convention, 0.1.x]
related_features: [2026-05-18-release-0.1.0-prep]
related_commits: [49a0f37]
related_decisions: []
---

# 0.1.x release 文档与 Cargo metadata 约定

## 背景

`release-0.1.0-prep` feature accept 时，首个 0.1.0 release 同时落地四件事：version bump + README 重写 + CHANGELOG 起步 + workspace Cargo.toml metadata 补齐。这四件事**每次新 minor release 都会再来一次**——但具体怎么做（README 结构、CHANGELOG 格式、metadata 字段清单、tag / push 时机、是否 publish）在每个 release 节点都会重新决策一遍，等于把已经走过的路径重复走。

brainstorm `v0.x-direction` 已敲定"0.x 占位好好做，不急上 crates.io"，0.1.x 不 publish；但 cargo metadata 字段、README 排版、CHANGELOG 分类节奏在那个 brainstorm 里没细到操作层。本决策把 release-0.1.0-prep 实施时一路验证过的具体约定固化下来，给 0.2.0 / 0.3.0 / ... 直接复用。

## 决定

0.x 阶段所有 minor release 准备工作遵循以下约定，组合成"release-{version}-prep" feature 的固定模板。

### A. crates.io publish 策略

- **0.1.x 不上 crates.io**（含 0.1.0 / 0.1.1 / ...）。不跑 `cargo publish --dry-run`，避免假行错误信息动摇决议
- **0.2.0 前夜独立 feature** 跑 `cargo publish --dry-run`，校准 metadata 后真 publish
- workspace Cargo.toml 的 metadata 字段从 0.1.0 起就**提前补齐**为 0.2.0 publish 预热，0.2.0 时不需再现场补字段

### B. README 五段结构（user-why leading）

按以下顺序排，**技术属性不作为 hero leading**：

1. **Hero**：一句话 user-why（"手机也能跟进电脑上的 agent"形式），含项目首要痛点 + 数据主权立场 + 平台关键词；副标题英文 tagline 一行
2. **状态**：当前 version + 阶段 milestone（minimal-loop / B 验收形态等）+ 前提依赖（飞书租户 / lark-cli / agent runtime）
3. **Quickstart**：3 条核心命令——install / 装机 / 第一次主动 push
4. **核心定位**：vendor-neutral / Feishu-native / local-first / developer-priority 四属性下移到此（不上 hero）
5. **跟谁错位**：保留 / 演进的对比表

末尾保留 License 一行。文件长度 ≤ 200 行，超长走 `docs/` 子目录拆分。

### C. CHANGELOG.md Keep a Changelog 1.1.0

- 文件首部固定 header + `[Unreleased]` 占位 + `[版本] - 日期` 章节倒序排列
- 每个 release 章节按 **Added / Changed / Fixed / Deprecated / Removed / Security / Documentation / Metadata** 分类（Keep a Changelog 标准 + 本项目扩展 Documentation / Metadata 两类）
- **粒度 = feature 单位**——每个 feature 一行子弹，引用 feature slug；不展开 commit 级细节（reader 想看细节走 git log + feature design doc）
- Added 段按 Phase / 模块顺序排（rust-rewrite 期间），不按重要性
- 末尾 ref-style link def 指向 GH compare URL 形式 `compare/v{prev}...HEAD` 与 `releases/tag/v{ver}`

### D. workspace Cargo.toml metadata 7 字段清单

`[workspace.package]` 段必含以下字段（除 `version` / `edition` / `license` / `authors` / `repository` 基础字段外）：

1. `description` — 一句话，与 README hero 摘要语义一致
2. `keywords` — 5 个（crates.io 限），lowercase 单 word `[a-z0-9_-]`，按"通用 → 主舞台 → 同义 → 定位 → 差异化用户视角"语义排列
3. `categories` — ≤ 5 个，**只从 crates.io 官方 category_slugs 列表选**，注释标 verified 日期
4. `readme = "README.md"`
5. `homepage` — 暂同 `repository`（未来若有 docs 网站再拆）
6. `documentation` — 暂同 `repository`（未来若有 docs.rs / GH Pages 再拆）
7. `rust-version` — 读 `rust-toolchain.toml` + 实测 `cargo build` 校准最低可用版本

**crate-level `Cargo.toml`** 所有可继承字段一律 `.workspace = true`，单点维护原则——description / homepage / documentation / readme / categories / keywords / rust-version 全 inherit。

### E. version bump 与 git tag

- `version` 单点在 `workspace.package`（`grep '"X.Y.Z"' Cargo.toml` 应只命中一处）
- crate-level `version.workspace = true`
- `roostery --version` 自动跟随 `env!("CARGO_PKG_VERSION")`
- 集成测试 `version_string_locked` 字面量与 Cargo.toml 同步更新（测试设计意图就是锁此契约）
- **accept 阶段**打本地 `git tag -a v{version} -m "{version}: {milestone 一句话}"`
- **不在 implement / accept 阶段 push tag**——push 时机用户自决
- **不在本期开 GH release page**——release notes / binary artifacts 上传是独立 follow-up feature

### F. 明确不做（reverse checklist）

每次 release-{ver}-prep feature 都跑下面 grep 守护：

- `cargo publish` 全仓出现次数 = 0（0.1.x 期间）
- `git push --tags` / `gh release create` 在 implement 产物 / commit message = 0
- LICENSE / index.js / package.json diff = 0
- `[dependencies]` 段未增减
- 新增文件清单只允许 `CHANGELOG.md`（首次）；CONTRIBUTING / CODE_OF_CONDUCT / SECURITY 推到首批外部 contributor 进来后做
- README*.md 仅一个文件（不写多语言 README）

## 为什么这样选

**A 不上 crates.io**：brainstorm `v0.x-direction` 已决议"占位好好做不急 publish"。0.1.x 是迭代期，名字 / 接口 / 模块边界都可能动；过早 publish 把不稳定接口锁进 crates.io 索引代价高于收益。0.2.0 时项目骨架稳定，再做 publish 决策。**字段提前补齐**是因为 metadata 字段补齐成本低且无副作用，临到 publish 现场补容易遗漏。

**B 五段结构 user-why leading**：observed 业界趋势——GH trending Rust 项目首屏 hero 普遍是"做什么 / 解决什么痛点"而非"我们用了什么技术"。原 README "vendor-neutral / Feishu-native" 等属性词作为 hero leading 把 B 类用户首次接触语境放在错误层级——他们关心的是"这能解决我什么问题"，技术属性是 second-order 决策辅助信息。

**C Keep a Changelog 1.1.0**：业界事实标准（Rust 主流项目大量遵循），reader 心智成本低；feature 单位粒度匹配本项目 `cs-feat` 工作流的最小交付单位，每个 feature 在 CHANGELOG 一行恰好对应 design / acceptance / commit 三元组。commit 级粒度会让 CHANGELOG 几百行散开，按重要性排让"何时落地"维度丢失。

**D 7 字段清单**：完整满足 crates.io publish 元数据要求（参 [Cargo manifest reference](https://doc.rust-lang.org/cargo/reference/manifest.html)），同时让 docs.rs / cargo doc 主页有完整可读形态。inherit 模式让一次维护 N 处生效——`description` 改一处所有 crate 同步。

**E version 单点 + 本地 tag 不 push**：单点避免漂移（Cargo.toml / lib.rs / docs / README 各处可能不同步是典型 Rust 项目 bug）。本地 tag 让 0.x 阶段 release 时机灵活：tag 是 commit 的稳定锚点，但 push 是发布动作——把"发布"决策权留给用户。

**F 反向核对**：design 阶段越是明确"什么不做"，implement / accept 越不会犯。grep 守护让"不做"可机器验证，不靠人记忆。

## 考虑过的替代方案

**A1 0.1.0 直接上 crates.io**：被 brainstorm `v0.x-direction` 早期决议否决。理由：(i) 命名占位用 GitHub URL 已足够，crates.io 占位多此一举；(ii) 0.1.x 期间接口可能 breaking，crates.io 公开后再 breaking 对早期 follower 不友好；(iii) Rust 项目业界 0.x 阶段不上 crates.io 是常见做法（如 ratatui / dioxus 早期）。

**A2 0.1.0 跑 `cargo publish --dry-run`** 不真 publish 只验 metadata：被 design §0 D1 决议否决。理由：dry-run 失败会输出红色错误信息，可能动摇"占位好好做"的决议（"既然 dry-run 都过不了我们就 publish 修一下吧"是典型 scope creep）。0.2.0 前夜独立 feature 一次性做更干净。

**B1 README 把技术属性放 hero**：原 README 形态。被 design §1.2 D3 否决——技术属性需要前置 context 才能判断好坏，user-why 是任意访客 60 秒看完就懂的层级。

**B2 README 写多语言版本（README.zh-CN.md / README.en.md）**：被 design §1.3 否决。理由：单文件中英混排（沿用现风格）维护成本最低；多语言版本一旦走起来就是"每次改 README 都要改 N 份"的债，0.1.x 阶段不开此口子。

**C1 CHANGELOG 自动从 git log 生成**（auto-changelog / git-cliff）：被 design §2.5 O2 推到 0.2.0+。理由：0.1.0 是首版手工写一次定基线；后续若 release 节奏加快再考虑工具化。早期工具化容易把"什么是值得写进 CHANGELOG 的"决策权交给工具，不一定符合 reader 视角。

**C2 CHANGELOG 按重要性排（minimal-loop 优先）**而非按时间 / Phase：被 implement U1 决议否决。Phase 顺序对应仓库实际推进路径，reader 顺读就能跟随 Rust 重写脉络；按重要性需要 reader 先理解项目优先级体系，认知成本更高。

**D1 keywords 多于 5 个**：crates.io 硬限制 ≤ 5，无替代余地。选词原则按"通用 → 主舞台 → 同义 → 定位 → 差异化"语义维度排列，让搜索覆盖最广 5 类 query。

**D2 categories 写非官方 slug**：crates.io publish 会 reject，没选择。注释标 verified 日期避免未来 crates.io 调整官方列表时盲选导致 reject。

**E1 implement / accept 阶段 push tag**：被 design §1.3 否决。理由：push 是发布动作，影响其他人（subscribe 仓库的 watcher 会收到 release 通知），权限交给用户。本地 tag 是 commit 锚点不是发布动作。

**E2 accept 阶段同步开 GH release page** 上传 binary artifacts：被 design §1.3 D5 推到 follow-up。理由：GH release page 涉及 release notes 撰写 + binary 预编译 + 多平台 artifact 上传，是独立工作量，0.1.0 首版先聚焦"git ref + 文档形态"，发布工程化下一版做。

## 影响 / 后续约束

- **0.2.0 / 0.3.0 / ... release-{version}-prep feature** 直接复用本约定的 A-F 六段，design doc 可以引本决策免重复阐述；只需补充本期 release 特有的"做了什么"
- **新模块 / 新 feature** 不直接受影响——本约定只在 release 节点 fire，不约束日常开发
- **crates.io publish 决策点**在 0.2.0 前夜独立 feature 中重新评估——届时本约定的"A 0.1.x 不上 crates.io"段会被 supersede（届时新建 decision: `0.2.0+ publish 策略`，本决议 status 改 `superseded`）
- **README 长度 > 200 行**时触发 design §2.5 O1 微重构——拆 `docs/` 子目录，本约定 B 段保持不变只是承载文件位置变化
- **CHANGELOG 写到 1.0.0+** 后若 release 节奏加快考虑 auto-changelog 工具——届时新建 decision: `CHANGELOG 自动化策略` 部分 supersede 本约定 C 段
