---
doc_type: issue-report
issue: 2026-05-18-init-shim-conflicts-npm-prefix
status: resolved
severity: P2
summary: roostery init 在 npm 全局 prefix == 默认 shim target 路径时 RealLarkCliMissing fail；resolve_real_lark_cli 排除 shim_target 后无候选。**Resolved by feature 2026-05-18-init-real-lark-cli-override (commit aa06807, CI #26036982700)** — 加 `--real-lark-cli` flag + 复用 `ROOSTERY_LARK_CLI_BIN` env + resolve early-gate + 错误信息 3 sub-variant；真机 dogfood 完整跑通（含被动 CC SessionEnd 真飞书出 task）
tags: [onboarding, init, shim, lark-cli, dogfood, ux, resolved]
related_features: [2026-05-18-roostery-init, 2026-05-18-bot-stop-hook, 2026-05-18-init-real-lark-cli-override]
resolved_by: 2026-05-18-init-real-lark-cli-override
resolved_at: 2026-05-18
resolved_commit: aa06807
discovered_during: bot-stop-hook 真机 dogfood
---

# init 在 npm prefix == shim target 时 RealLarkCliMissing — Issue Report

## 1. 问题现象

跑 `./target/release/roostery init --dry-run`（也包括 live mode）立即终止，stderr 输出：

```
[roostery init] no real `lark-cli` found on PATH (excluding shim target); install lark-cli and ensure it is on PATH before running `roostery init`
```

进程退出 code = 1。整个 init 流程零文件副作用（OnboardingError::RealLarkCliMissing 在 onboarding::run 早期 gate 处返还，未触发任何写操作）。

## 2. 复现步骤

**前置环境**：通过 `npm install -g @larksuite/cli` 装 lark-cli（npm 默认全局 prefix = `~/.local` 时落脚 `~/.local/bin/lark-cli` 作为指向 `~/.local/lib/node_modules/@larksuite/cli/scripts/run.js` 的 symlink）。`~/.local/bin` 在 PATH 上。

1. `which lark-cli` 确认输出 = `/Users/ben/.local/bin/lark-cli`（npm prefix bin = roostery 默认 shim target 路径）
2. `ls -la /Users/ben/.local/bin/lark-cli` 确认是 symlink 到 `../lib/node_modules/@larksuite/cli/scripts/run.js`
3. `./target/release/roostery init --dry-run`
4. 观察到：上面那条错误，exit 1

**复现频率**：稳定 100%（任何把 npm 全局 prefix 设到 `~/.local` 的用户都会撞——这是 npm `~/.local` prefix 模式的默认安装位）

## 3. 期望 vs 实际

**期望行为**：`roostery init --dry-run` 检测到 lark-cli 已装、版本 1.0.29、auth 已登录，输出"会装哪些文件 / 改哪些 hook"的 dry-run 报告。完整 init 应能把 shim 装到目标位置，把原 npm symlink 备份或显式记录为 `ROOSTERY_REAL_LARK_CLI`。

**实际行为**：因为 `resolve_real_lark_cli` 的实现假设"真 lark-cli 在 PATH 上**且不在** shim target 位置"，当二者重合（npm 全局 install 的常见情况）时，它把 `which::which_all("lark-cli")` 唯一的候选过滤掉，返 `RealLarkCliMissing`。init 直接 bail，不进入任何后续步骤。

## 4. 环境信息

- **涉及模块 / 功能**：`crates/roostery/src/onboarding.rs::resolve_real_lark_cli`（feature `2026-05-18-roostery-init` 落地）
- **相关文件 / 函数**：
  - `crates/roostery/src/onboarding.rs:417-427` `resolve_real_lark_cli(shim_target)` — 错误源
  - `crates/roostery/src/onboarding.rs:85-88` `OnboardingError::RealLarkCliMissing` — 抛出位置
  - `crates/roostery/src/onboarding.rs:134-138` `InitOptions` — 目前 2 字段（`dry_run` / `skip_agents`），无 real_lark_cli override
  - `crates/roostery/src/main.rs:99-107` `InitArgs` clap struct — 目前 2 flag（`--dry-run` / `--skip-agent`）
- **运行环境**：
  - macOS Darwin 25.4.0
  - lark-cli `1.0.29` via npm
  - PATH 顺序：`.smux/bin` → `.claude/bin` → `.codeium/windsurf/bin` → `.local/bin` → ...
  - rustc stable（最新 release build）
  - auth 已登录 dustben / ou_ababf07a..., tokenStatus needs_refresh（自动刷成功，roostery smoke 6 probe 全绿不受影响）
- **触发情境**：bot-stop-hook feature 验收后真机 dogfood 首次跑 `roostery init`，撞到本问题

## 5. 严重程度

**P2 中等** — 影响 onboarding UX 的核心一步（且是 `agent-work-in-feishu` req 的 B 用户首次装机入口），但**有 workaround**：用户可在另一个 PATH 位置（如 `~/.smux/bin/`）建一个 lark-cli symlink，让 `which::which_all` 找到非 shim target 的候选。或绕过 init 直接用 `roostery bot push` 走反向 CLI（不依赖 shim）。

不进 P1 因为：(a) 仅影响首次装机；(b) workaround 简单；(c) 0.1.0 新增能力（反向 push CLI）不依赖 init 跑通，被动 hook 路径才依赖。

不进 P3 因为：(a) npm `~/.local` prefix 是越来越多 user 的默认（替代 `/usr/local` 避免 sudo）；(b) 撞到的人一开始猜不到原因（错误信息说 "install lark-cli and ensure it is on PATH" 但 lark-cli 明明已经在 PATH 上）。

## 备注

### 根治方向（**不在本 issue 修，留给后续 feature**）

用户明示本 issue 仅 report，根治方向后续走独立 feature（如 `init-real-lark-cli-override` 或合并到 onboarding 改进 feature）。候选方案（design 阶段定）：

1. **方案 A**：`InitArgs` 加 `--real-lark-cli <path>` flag，`InitOptions.real_lark_cli: Option<PathBuf>`，`resolve_real_lark_cli` 优先用显式值
2. **方案 B**：检 `ROOSTERY_REAL_LARK_CLI_BIN` env 作 override（与现有 `ROOSTERY_LARK_CLI_BIN` 测试 hook env 区分；后者已在测试代码用）
3. **方案 C**：A + B 都加（flag 用于一次性 / env 用于 CI / shell 永久 export）
4. **改善 fallback 信息**：错误信息明确告诉用户"探到 lark-cli 在 PATH 但等于 shim target；用 --real-lark-cli 或 ROOSTERY_REAL_LARK_CLI_BIN 显式指定"，避免下个撞到的用户也卡

### 错误信息可改善（design 时定）

现错误："no real `lark-cli` found on PATH (excluding shim target); install lark-cli and ensure it is on PATH before running `roostery init`"

建议：明确区分"PATH 上一个都没有"与"PATH 上有但全部等于 shim target"两种 case，分别给不同提示。

### Workaround（dogfood 期可绕过）

```bash
# 一次性绕过：让 init 找到非 shim 的候选
ln -s /Users/ben/.local/lib/node_modules/@larksuite/cli/scripts/run.js \
      /Users/ben/.smux/bin/lark-cli
# 之后 ~/.smux/bin/lark-cli 在 PATH 更前段，which::which_all 返回它作 real path
./target/release/roostery init
# init 完成后可保留这个 symlink 当 backup；或确认 ~/.roostery/env 写了正确 ROOSTERY_REAL_LARK_CLI 后清掉
```

### 与已有 feature 关系

- feature `2026-05-18-roostery-init` 是引入 `resolve_real_lark_cli` 的 feature（commit 已合入主分支），未来 fix feature 会改它
- feature `2026-05-18-bot-stop-hook`（本次 dogfood 触发发现）不直接受影响——bot-stop-hook 的反向 push CLI 路径（`roostery bot push`）不依赖 shim / init，0.1.0 触发判据的"反向调用"维度仍可单独验证；只有被动 hook 路径（CC SessionEnd → shim 接管 lark-cli）要求 init 先跑通
