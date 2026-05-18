---
doc_type: learning
category: technique
slug: ci-rustfmt-style-drift
status: active
created: 2026-05-18
tags: [rust, rustfmt, ci, formatting, debugging]
related_features: [2026-05-18-bot-task-writer]
related_commits: [2f67079, 083b8ba]
---

# CI rustfmt 与本地 rustfmt 输出偶有偏差

## 场景

`cargo fmt --all --check` 本地全过，推到 GitHub Actions CI 上的同一 cargo `cargo fmt --all --check` 报 diff，PR 卡住。

实际遇到的 diff 类型（feature `2026-05-18-bot-task-writer` `parse_task_response` 中的 `.ok_or(struct literal)` 多行布局）：

```rust
// 本地 rustfmt 接受这种 method-chain 紧凑形式：
let guid = data.get("guid").and_then(|v| v.as_str()).ok_or(TaskWriterError::ResponseShapeUnexpected {
    expected: "data.guid",
    raw_head: raw_head.clone(),
})?;

// CI rustfmt 要求展开成：
let guid = data.get("guid").and_then(|v| v.as_str()).ok_or(
    TaskWriterError::ResponseShapeUnexpected {
        expected: "data.guid",
        raw_head: raw_head.clone(),
    },
)?;
```

两次本地 `cargo fmt` 都不会主动改成第二种；CI 的 `cargo fmt --check` 却报第一种是 diff。

## 根因（推测）

- **toolchain 版本**：`rust-toolchain.toml` 锁了 `stable` channel 但具体 patch 版本随系统更新漂移。CI runner 的 rustc / rustfmt 版本可能比本地新（或反之），不同 patch 版本里 rustfmt 对"method chain + struct literal 续行长度阈值"的判定有过细微调整
- **行宽阈值边界**：rustfmt 默认 `max_width = 100`，本例代码行宽在 95-105 之间游走，刚好踩在边界上——本地一台机器算出"在阈值内不折"，CI 算出"超阈值要折"
- **本仓库无 `rustfmt.toml`**：未固定 `max_width` / `chain_width` 等参数，全靠默认值（默认值会随 rustfmt 版本演进微调）

## 应对方法（已实践）

### 1. CI fail 时直接走 CI 的格式（推荐）

不与 CI 争对错——CI 是 release gate，本地是开发体验。把 CI diff 报告里的"应该这样"那段贴回本地代码，再 `cargo fmt --check` 通过 → push。代价：本地代码偶尔出现"看起来过度折行"的片段，但不阻塞 PR。

实际操作步骤：
1. 看 CI fmt job 输出里的 `diff` 块
2. 把 `+` 那侧的内容贴回本地源码
3. 本地 `cargo fmt --check` 确认 idempotent（不会被本地 rustfmt 改回去）
4. push 一个 `fix(...): apply CI rustfmt style on ...` commit

### 2. 不要主动统一本地 rustfmt 版本到 CI（不推荐）

理论上可在 `rust-toolchain.toml` 把 channel pin 到具体 patch 版本（`stable-1.95.0` 而非 `stable`），但代价：
- 阻止本地用户用最新 stable
- 每次 stable 升级要手动 bump toolchain
- 修一个偶发格式问题付出长期维护成本，不划算

### 3. 不要加 rustfmt.toml 试图 freeze 默认值（看场景）

写一份 `rustfmt.toml` 显式锁所有 `max_width` / `chain_width` / `fn_call_width` 等，可以让 CI 和本地输出确定一致。但代价：
- 把"用 rustfmt 默认值"这个简单的项目共识，换成"维护一份格式规约文件"
- rustfmt 默认值演进时被反向冻结，新 idiom 接不上

bot-task-writer feature 选择不走方案 3——单次偶发不值得加配置文件。如未来同类问题多次复发（≥3 次），再开 cs-decide 评估锁 `rustfmt.toml`。

## 复发判据

下次又撞到 CI rustfmt 报 diff 但本地通过时：

- ✅ 这种偏差再撞，照 §应对方法 §1 处理（直接采纳 CI 输出）
- ❌ 不要花时间 debug 谁对谁错（rustfmt 的 method chain 续行启发式不是稳定接口）
- ⚠️ 累计第 3 次时升级到 cs-decide：评估锁 rustfmt.toml 的代价/收益

## 反场景

- ❌ **不适用于 clippy warning 差异**——clippy lint 输出差异通常意味着 toolchain 版本错位严重（CI 用新版多了几条 lint），需要本地 `rustup update` 同步，不是格式偏差问题
- ❌ **不适用于"本地 cargo fmt 改了文件但 CI 还报 diff"**——这种是 git working tree 没 add 干净，不是版本偏差

## 相关

- bot-task-writer feature commit `2f67079` ("fix(bot-task-writer): apply CI rustfmt style on parse_task_response") — 本 learning 抽离的原始事件
- bot-task-writer feature commit `083b8ba` — CI 绿后挂的 commit，确认 §应对方法 §1 行得通
- bot-task-writer-acceptance.md §8 attention.md 候选 1（评估时建议归 cs-learn 而非 attention，本文件兑现这条建议）
