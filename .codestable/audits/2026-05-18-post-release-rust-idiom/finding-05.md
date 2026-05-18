---
doc_type: audit-finding
slug: onboarding-memmem-useless-wrapper
audit: 2026-05-18-post-release-rust-idiom
dimension: maintainability
severity: P2
confidence: high
suggested_action: cs-refactor
tags: [onboarding, rust-idiom, dead-abstraction]
---

# Finding 05：`onboarding::memmem` 是 `<[u8]>::windows().any()` 的无效包装

## 位置

`crates/roostery/src/onboarding.rs:318-324`

## 证据

```rust
fn memmem(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}
```

调用方分布（`grep -n memmem onboarding.rs`）：

```
311: fn looks_like_roostery_shim(path: &Path) -> bool {
312:   ... memmem(&buf, b"roostery shim") ...
```

只有 1 处调用站点。

## 为什么构成问题

1. **无效抽象**：函数体只 3 行真逻辑，本质就是 `windows().any()`；包了一层名字（`memmem`）反而误导——读者会以为是 `libc::memmem` 或 `memchr` crate 风格的特殊优化实现。
2. **空检查冗余**：`windows(0)` 在 stable Rust 是 panic（debug）/ UB-ish（release），但调用方 `looks_like_roostery_shim` 传的是固定字面量 `b"roostery shim"`（非空）；唯一保护场景是"理论上将来有人传空 needle"——over-defensive。
3. **`haystack.len() < needle.len()` 守卫无效**：`windows(N)` 在 `haystack.len() < N` 时直接产出空迭代器，`.any()` 自然返回 false——内置语义已守。

## 建议改法（不在本审计动手）

**Option A**（推荐）：直接 inline 到唯一调用站点
```rust
fn looks_like_roostery_shim(path: &Path) -> bool {
    let Ok(buf) = std::fs::read(path) else { return false };
    buf.windows(b"roostery shim".len()).any(|w| w == b"roostery shim")
}
```

**Option B**：引入 `memchr` crate（业界标准 substring 搜索）—— 不推荐，本场景一次性匹配几百字节文件，没有性能价值。

**Option C**：保留 `memmem` 但删空检查和长度守卫（让标准库语义自己处理）——折中，但 1 行调用就不值得保留函数。

## 影响范围

- 改动量：删 1 函数 + 改 1 调用站点
- 公开 API 零变化（`memmem` 是 module-private `fn`）
- 行为零变化（调用方传的 needle 非空，`windows().any()` 行为一致）
- 测试零改动

## 关联

- 决策 `2026-05-18-decision-rust-idiom-first.md`（Rust 惯用法优先，反对 Python parity 式重新发明轮子）
- finding-04（onboarding split 时可以一起做）
