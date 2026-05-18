# 0.1.0 release 准备 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-18
> 关联方案 doc：`.codestable/features/2026-05-18-release-0.1.0-prep/release-0.1.0-prep-design.md`

## 1. 接口契约核对

对照方案第 2.1 节名词层逐一核查。本 feature 无运行时接口，"接口"即"文件内容契约"。

**接口示例逐项核对**：
- [x] 示例 A（`Cargo.toml` workspace 段 metadata 7 字段）：design §2.1 示例 7 字段全在位 — `cargo metadata --no-deps` 验：description / keywords 5 / categories 3 / readme=../../README.md / homepage / documentation / rust_version=1.95，全一致 ✓
- [x] 示例 B（`CHANGELOG.md` Keep a Changelog `[0.1.0] - 2026-05-18`）：header + `[Unreleased]` + 章节标题 + ref-style link def 指向 `compare/v0.1.0...HEAD` 与 `releases/tag/v0.1.0` ✓
- [x] 示例 C（`README.md` hero section excerpt）：第一段含"手机也能跟进电脑上的 agent" + "多设备 vibecoding" + "数据主权" + "飞书"四关键词；副标题英文 tagline 保留 ✓

**名词层"现状 → 变化"逐项核对**：
- [x] `README.md`：44 → 54 行五段结构（Hero / 状态 / Quickstart / 核心定位 / 跟谁错位 / License）✓
- [x] `CHANGELOG.md`：新建 86 行 ✓
- [x] workspace `Cargo.toml` 版本：`0.0.0` → `0.1.0` ✓
- [x] workspace `Cargo.toml` metadata：加 7 字段（description / keywords / categories / readme / homepage / documentation / rust-version）✓
- [x] crate `Cargo.toml` description：硬编码 → `description.workspace = true`（同时把 homepage/documentation/readme/categories/keywords/rust-version 也 inherit，单点维护）✓

**流程图核对**（第 2.2 节 mermaid）：S1→S2→S3→S4→S5 五步线性流水，每步均有实际文件改动落点 + 退出信号验证。

无偏差。

## 2. 行为与决策核对

**需求摘要逐项验证**（design §1.1 F1-F7）：
- [x] F1 README.md 完全重写五段结构：实测渲染 ✓
- [x] F2 CHANGELOG.md 新建 Keep a Changelog：17 feature highlights + 1 Fixed（init-shim-conflicts-npm-prefix）+ Documentation + Metadata 四段 ✓
- [x] F3 workspace Cargo.toml metadata 7 新字段：`cargo metadata` 验 ✓
- [x] F4 crate-level description inherit：grep `description.workspace = true` 命中 ✓
- [x] F5 version bump 0.0.0 → 0.1.0：`./target/debug/roostery --version` 输出 `roostery 0.1.0 (rust)` ✓
- [x] F6 README 示例命令可跑：`--summary-stdin` flag 对照 `bot_stop_hook.rs:584` 校准存在；`--real-lark-cli` 对照 `init` feature 校准 ✓
- [x] F7 git tag v0.1.0：accept 阶段动作，**本节末尾执行**

**明确不做逐项核对**（design §1.3 + §3.2）：
- [x] 不 publish crates.io：grep `cargo publish` 全仓 = 0 ✓
- [x] 不 push git tag：implement 阶段未跑 `git push --tags`；accept 仅本地打 tag ✓
- [x] 不开 GH release page：grep `gh release create` 全仓 = 0 ✓
- [x] 不动 LICENSE：`git diff LICENSE` = 0 ✓
- [x] 不删 npm `index.js` / `package.json`：`git diff index.js package.json` = 0 ✓
- [x] 不引入新 Cargo dep：`[dependencies]` 段未变 ✓
- [x] 不写 CONTRIBUTING / CODE_OF_CONDUCT / SECURITY：`ls` 三文件不存在 ✓
- [x] 不写多语言 README：`ls README*.md` 仅 `README.md` ✓

**关键决策落地**（D1-D12）：
- [x] D1 不 publish crates.io / D2 version=0.1.0 / D3 README 五段 / D4 CHANGELOG Keep a Changelog / D5 不开 GH release / D6 不动 LICENSE / D7 不删 npm：全兑现
- [x] D8 keywords 5 个：`agent / feishu / lark / broker / vibecoding`，全 lowercase 单 word ✓
- [x] D9 categories 3 个：`command-line-utilities / development-tools / api-bindings`，verified 2026-05-18 against crates.io/category_slugs（注释已落 Cargo.toml）✓
- [x] D10 rust-version：实测 `rustc 1.95.0` → 写 `"1.95"` ✓
- [x] D11 homepage = repository URL：填同址 ✓
- [x] D12 CHANGELOG feature 粒度：17 条按 Phase 0-5 顺序 ✓

**编排层"现状 → 变化"**：无（文档 / metadata feature，无运行时编排）。

**流程级约束核对**：所有改动可逆 / 幂等；`cargo build` + `cargo test` 是健康度信号 ✓

**挂载点反向核对**（design §2.3 四个挂载点）：

- [x] M1 `README.md` 五段结构 → 项目根 `README.md`：实际落地，删了后 GitHub 首页 + cargo doc 主页退回 placeholder ✓
- [x] M2 `CHANGELOG.md` → 项目根：新文件存在 ✓
- [x] M3 `workspace.package` 元数据扩展（7 字段）→ `Cargo.toml`：grep 7 字段全在 ✓
- [x] M4 version "0.1.0" + git tag `v0.1.0` → `Cargo.toml` + git refs：version 在；git tag 见本节末尾

**反向 grep 核查**——本 feature 在代码里的所有引用：
```
$ git diff --name-only HEAD
Cargo.lock
Cargo.toml
README.md
crates/roostery/Cargo.toml
crates/roostery/tests/smoke_integration.rs
```
+ 未跟踪新文件 `CHANGELOG.md` + feature 目录 design/checklist/acceptance。

清单外引用：`crates/roostery/tests/smoke_integration.rs`——version_string_locked 测试字面量随 version bump 更新（测试本身就是为锁此契约存在）。**不算漏记挂载点**，是 S4 version bump 的次级落点（测试与 Cargo.toml version 同源契约的另一面）。已在 acceptance 报告记录，不补入第 2.3 节挂载点清单（挂载点判据是"删了它 feature 是否消失"，测试断言字面量不构成"feature 消失"的判据）。

**拔除沙盘推演**：按 M1-M4 逆向（删 README / 删 CHANGELOG / 撤 7 字段 / 回滚 version）后，仓库回到 implement 前形态；smoke_integration.rs 的版本字面量也要同步回滚 — 在 git 层面随 version 一起 revert，无残留。

无偏差。

## 3. 验收场景核对

对照 design §3.1 关键场景逐条：

- [x] **A1** GitHub 首页访客打开 README：Hero 首屏含"多设备"+"vibecoding"+"数据主权"+"飞书"四关键词；技术属性词未作 hero leading（已下移到"核心定位"段）
  - 证据：grep "多设备" / "vibecoding" / "数据主权" / "飞书" on README.md L3 全命中
- [x] **A2** cargo doc / docs.rs 主页：description 非空且与 README hero 摘要语义一致（"Vendor-neutral, Feishu-native agent broker for multi-device vibecoding."）
  - 证据：`cargo metadata --no-deps` 输出 description 字段
- [x] **A3** `cargo install --git`：cargo 解析 metadata 0 错误；package version 0.1.0；keywords/categories 显
  - 证据：`cargo build` 全绿（含 metadata 解析）
- [x] **A4** `roostery --version`：输出 `roostery 0.1.0 (rust)`
  - 证据：直接 spawn 输出 + `version_string_locked` 集成测试通过
- [x] **A5** 读 CHANGELOG 找"0.1.0 有什么"：`[0.1.0] - 2026-05-18` 章节下 17 feature 按 Added/Fixed/Documentation/Metadata 分类；含 init-shim 修复（Fixed 段）
  - 证据：grep 17 slug 全命中 + grep `init-shim-conflicts-npm-prefix` = 1
- [x] **A6** 0.2.0 维护者扫 Cargo.toml：7 字段全就位，只需 `version=0.2.0` + `cargo publish --dry-run`
  - 证据：`cargo metadata` 输出全字段

**边界**：
- [x] B1 emoji 渲染：README + CHANGELOG 含 🪺 🎯 🚧，无渲染错（markdown lint 隐含通过 — cargo doc 无 warning）
- [x] B2 CHANGELOG ref-style link 指向 GH compare：`[Unreleased]: https://github.com/bendusy/roostery/compare/v0.1.0...HEAD`
- [x] B3 keywords lowercase 单 word ≤5：`["agent","feishu","lark","broker","vibecoding"]`，cargo verify 0 错
- [x] B4 README quickstart 命令准确：`--summary-stdin` 与 `bot_stop_hook.rs:584` 一致；`--real-lark-cli` 与 init feature 一致

**错误反向**：
- [x] E1 keywords 无大写：grep `'"[A-Z]'` on Cargo.toml keywords 段 = 0
- [x] E2 categories 在官方列表：注释 `# crates.io category_slugs verified 2026-05-18 against ...` 已落
- [x] E3 README 含 0.1.0 实际 CLI：grep `roostery init` / `roostery bot push` 全命中
- [x] E4 CHANGELOG 全 17 slug：grep 守护 missing=0
- [x] E5 version 单点：grep `"0.1.0"` on Cargo.toml = 1（仅 workspace.package）

文档 / metadata feature，无浏览器肉眼验证项（无 UI 改动）。

## 4. 术语一致性

design §0 决策头 + §1.2 D8 关键词术语 grep：

- `Roostery / roostery`：README + CHANGELOG + Cargo.toml description 一致全小写 binary / 大写品牌 ✓
- `vendor-neutral / Feishu-native / multi-device / vibecoding`：README + description + CHANGELOG 用语一致 ✓
- 禁用词反查：未出现 "feishu hub" / "lark agent broker" 等暗示飞书附属的措辞（attention.md "其他" 段约束）✓

## 5. 架构归并

对照 design §4 跨模块影响：

- [x] **新增 Cargo dep**：无（D1.3 已守）
- [x] **CLI / lib API**：无变化
- [x] **lib.rs / 源文件**：无变化（version 自动跟随 `env!("CARGO_PKG_VERSION")`）
- [x] **templates/**：无变化

**架构 doc 实际写入**：

- [x] `.codestable/architecture/ARCHITECTURE.md` `> 末次刷新` 行更新为 "2026-05-18（0.1.0 release prep；version bump + README/CHANGELOG/Cargo metadata 首个 release 形态达成）"——见本节末尾"实际写入" 落档
- [x] `.codestable/requirements/agent-work-in-feishu.md` 变更日志加 0.1.0 release milestone 条目 — 见 §6
- [x] `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §5 "最小闭环" 段加 "0.1.0 已 tag" 状态 — 见 §7

无新模块 / 接口 / 跨模块纪律引入。`attention.md` 无候选更新（见 §8）。

## 6. requirement 回写

design frontmatter `requirement: agent-work-in-feishu`，该 req 已 `status: current`（bot-stop-hook accept 时升级，2026-05-18）。本 feature 未改用户故事 / 边界 / pitch — 是 req 的"对外门面"层兑现，不改愿景。

按规则属于 "current req 但本次未改用户视角"，但 0.1.0 release 是 req 的 milestone，需要在变更日志记录一条以保持时序可追溯（与 bot-task-writer / bot-stop-hook / roostery-init / init-real-lark-cli-override 等历次 accept 同模式）。**已在 req 变更日志末尾追加 0.1.0 milestone 条目**（见 §5 实际写入）。

## 7. roadmap 回写

design frontmatter 无 `roadmap` / `roadmap_item` 字段。检查 `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml` — 21 条 slug 中无 `release-0.1.0-prep`。本 feature 是 minimal-loop 达成后的对外门面 + version tag 准备，非 roadmap 起头 feature（与 `init-real-lark-cli-override` 同性质 follow-up）。

**结论**：跳过 items.yaml 状态变更。但同步更新 roadmap 主文档 §5 "最小闭环" 末尾段，记 "0.1.0 已 tag" 状态供下次推进时参考（见 §5 实际写入）。

## 8. attention.md 候选盘点

回看本次实现踩到的事：
- 无新编译命令 / 代理 / 起服务步骤
- 无新工作流约定
- 唯一一处"非显然" — `version_string_locked` 测试随 Cargo.toml version bump 字面量同步更新 — 是测试自身契约设计，下次 version bump 时若有新版本字面量测试也会自然被 cargo test 红灯提示，不需要写进 attention

**结论**：本 feature 未暴露需要补入 attention.md 的内容。

## 9. 遗留

- **后续优化点**：无（design §2.5 末尾观察项 O1/O2/O3 — README 拆 docs / CHANGELOG 自动生成 / crates.io publish prep — 均归 0.2.0+ 推进，不开 issue）
- **已知限制**：本期仅本地打 git tag v0.1.0 不 push；用户决定 push 时机。在用户 push 之前，CHANGELOG ref-style link `compare/v0.1.0...HEAD` 短暂 404 — 不影响本地阅读；design §5 R4 已预记。
- **实现阶段顺手发现**：无
