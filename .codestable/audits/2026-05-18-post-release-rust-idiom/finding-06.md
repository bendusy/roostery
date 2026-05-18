---
doc_type: audit-finding
slug: bot-stop-hook-push-fallback-duplication
audit: 2026-05-18-post-release-rust-idiom
dimension: maintainability
severity: P2
confidence: medium
suggested_action: cs-refactor
tags: [bot-stop-hook, rust-idiom, error-handling]
---

# Finding 06：`bot_stop_hook::push` 两处 `finish_with_fallback` 7-arg 调用重复

## 位置

`crates/roostery/src/bot_stop_hook.rs:386-401` 和 `408-422`（`pub async fn push` 内部）

## 证据

```rust
// 站点 1（line 386-401）—— task_writer.get_or_create_for_session 失败
let task_ref = match task_result {
    Ok(t) => t,
    Err(e) => {
        tracing::warn!(error = %e, "task_writer.get_or_create_for_session failed");
        outcome.errors.push(format!("task_writer: {e}"));
        return finish_with_fallback(
            outcome, runner, &receive_id, &req, &step_text, &basename, &opts,
        ).await;
    }
};

// 站点 2（line 408-422）—— append_steps 失败
if let Err(e) = ... {
    tracing::warn!(error = %e, "bot_task_writer::append_steps failed; will try IM fallback");
    outcome.errors.push(format!("append_steps: {e}"));
    outcome.task_url = Some(task_ref.url.clone());
    outcome.task_guid = Some(task_ref.guid.as_str().to_string());
    return finish_with_fallback(
        outcome, runner, &receive_id, &req, &step_text, &basename, &opts,
    ).await;
}
```

两处 `finish_with_fallback(outcome, runner, &receive_id, &req, &step_text, &basename, &opts)` 参数清单 100% 一致。

## 为什么构成问题

1. **7-arg 函数调用 × 2**：参数清单本身是 finding（参数过多通常是抽象边界划错信号），重复调用放大此问题。
2. **改 fallback 签名要改 2 处**：未来加 `--fallback-mode` flag 或新参数时双站点同步改，易漏。
3. **`?` operator 缺位**：站点 1 / 2 各有 `let Err = ... { return fallback }` 模式——Rust 惯用法是 `?` + `From` impl + 顶层兜底，但这里 `outcome.errors.push(format!(...))` 副作用让 `?` 不直接适用。

但**置信度 medium**：相比 finding-01 那种纯 boilerplate 重复，本处每个站点有特定 side-effect（不同 log 信息、不同 errors push 字符串、站点 2 还要 stash task_url/guid）——简单提取 closure 不一定让代码更清晰，可能反而隐藏意图。

## 建议改法（cs-refactor 阶段斟酌）

**Option A**——闭包捕获不变参数：

```rust
let mut go_fallback = |outcome: PushOutcome| async {
    finish_with_fallback(
        outcome, runner, &receive_id, &req, &step_text, &basename, &opts,
    ).await
};

let task_ref = match task_result {
    Ok(t) => t,
    Err(e) => {
        tracing::warn!(error = %e, "task_writer failed");
        outcome.errors.push(format!("task_writer: {e}"));
        return go_fallback(outcome).await;
    }
};
```

注意：async closure 是 nightly only；稳定版需用 trait object 或本地 helper fn。

**Option B**——把 fallback context 打包成 struct：

```rust
struct FallbackCtx<'a> {
    runner: &'a dyn LarkRunner,
    receive_id: &'a str,
    req: &'a PushRequest,
    step_text: &'a str,
    basename: &'a str,
    opts: &'a PushOptions,
}

async fn finish_with_fallback(outcome: PushOutcome, ctx: &FallbackCtx<'_>) -> PushOutcome { ... }
```

参数从 7 减到 2（outcome + ctx），调用清爽。

**Option C**——重构 `push` 主流程改 `Result` 链 + match-once：

```rust
async fn push_inner(...) -> Result<PushOutcome, (PushOutcome, String)> {
    let task_ref = bot_task_writer::get_or_create_for_session(...)
        .await
        .map_err(|e| (outcome.with_error(format!("task_writer: {e}")), "task_writer"))?;
    let aso = ...;
    bot_task_writer::append_steps(...)
        .await
        .map_err(|e| (outcome.with_task(...).with_error(format!("append_steps: {e}")), "append_steps"))?;
    Ok(outcome.success(...))
}

pub async fn push(...) -> PushOutcome {
    match push_inner(...).await {
        Ok(o) => o,
        Err((o, _stage)) => finish_with_fallback(o, runner, &receive_id, ...).await,
    }
}
```

最 Rust 惯用法但改动量也最大。

**推荐**：finding-02 拆 `push.rs` 子文件时一起做 Option B 或 C，单独做不值得。

## 影响范围

- 与 finding-02 联动；不联动则改动量小（10-20 行）
- 公开 API 零变化
- 行为零变化（fallback 调用次数、顺序、参数都一致）
- 集成测试 `bot_cli_integration.rs` 4 测试零改动

## 关联

- finding-02（bot_stop_hook 文件拆分；本 finding 在 `push.rs` 子文件内自然解决）
