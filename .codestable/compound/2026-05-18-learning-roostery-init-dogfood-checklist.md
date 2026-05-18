---
doc_type: learning
category: technique
slug: roostery-init-dogfood-checklist
status: active
created: 2026-05-18
tags: [onboarding, init, shim, dogfood, real-lark-cli, lark-cli, ux]
related_features: [2026-05-18-roostery-init, 2026-05-18-init-real-lark-cli-override]
related_issues: [2026-05-18-init-shim-conflicts-npm-prefix]
---

# `roostery init` 真机 dogfood checklist

## 场景

每次大版本 / Phase milestone 完成（如 0.1.0 release 准备、Phase 5/6/7 收尾、shim 安装链路改动）都要重新跑一次完整真机装机验证。这里把"被动 CC SessionEnd 真飞书出 task"那一条端到端链路的 dogfood 步骤固化下来，避免下次又凭记忆重组。

## 何时跑

- ✅ release 前最后一次完整验收
- ✅ feature 改动到 `crates/roostery/src/onboarding.rs` / `src/bin/shim.rs` / `src/hooks_merge.rs` / `templates/agent_stop_notify.sh` 任意一个
- ✅ feature 改动到 `LarkCli::new` / shim subprocess env 解析（影响 forward 行为）
- ✅ lark-cli 升级 pin（attention.md "1.0.28 baseline / 1.0.29 实测兼容"演进）
- ❌ 纯 lib 改动（如 dispatcher / journal 内部重构）不必走

## checklist（按顺序，每步打勾再下一步）

### 1. 前置：环境探针

```bash
# 1.1 lark-cli 在 PATH 上
which lark-cli  # 期望：~/.local/bin/lark-cli 或 brew prefix 位置

# 1.2 lark-cli 已认证（未认证 → lark-cli auth login 走完）
lark-cli auth status | head -20  # 期望 tokenStatus = "valid" 或 "needs_refresh"

# 1.3 roostery + shim 二进制构建
cd /Users/<u>/Projects/roostery
cargo build --release --bin roostery -p roostery
cargo build --release --bin shim -p roostery
ls -la target/release/{roostery,shim}  # 期望两个文件都在
```

### 2. smoke gate 验证（无副作用）

```bash
./target/release/roostery smoke 2>&1 | tail -5
```

期望：JSON 报告 `"all_ok": true`，6 条 probe 全绿。失败 → 先解决 lark-cli auth / 版本问题再继续。

### 3. dry-run init 预览（无副作用）

```bash
# 3.1 真 lark-cli 路径（npm 装的情况）
REAL_LARK_CLI=/Users/<u>/.local/lib/node_modules/@larksuite/cli/scripts/run.js
# 或 brew 装的情况
# REAL_LARK_CLI=$(realpath $(which lark-cli))

# 3.2 dry-run
./target/release/roostery init --dry-run --real-lark-cli $REAL_LARK_CLI 2>&1 | tail -15
```

期望输出：
- `real: <REAL_LARK_CLI> (from flag)` ← 验证 override 生效
- `identity: ✓ profile=... user=...` ← lark-cli auth 链通
- 3 agents 检测（cc/codex/gemini）状态各异（installed / NotInstalled）
- `⚠ DRY RUN — no files were modified.`

dry-run 跑不通 → 不能跑 live。回去看错误信息。

### 4. live init（破坏性 — 改用户 ~/.local/bin/lark-cli）

```bash
# 4.1 必须先 backup 已有 npm symlink（若 ~/.local/bin/lark-cli 不是 roostery shim）
ls -la ~/.local/bin/lark-cli  # 如显示 -> ../lib/.../run.js 是 npm symlink
mv ~/.local/bin/lark-cli ~/.local/bin/lark-cli.npm-bak

# 4.2 跑 live init
./target/release/roostery init --real-lark-cli $REAL_LARK_CLI 2>&1 | tail -15
```

期望产物：
- `~/.local/bin/lark-cli` ← roostery shim binary（~2.4 MB ELF / Mach-O）
- `~/.roostery/env` 内容 `export ROOSTERY_REAL_LARK_CLI='<REAL_LARK_CLI>'`
- `~/.roostery/scripts/agent_stop_notify.sh` ← 10 行极简 wrapper（grep `roostery bot stop-hook` 命中）
- `~/.zshrc`（或 `~/.bashrc`）含 `# >>> roostery >>>` marker block + `source ~/.roostery/env`
- `~/.claude/settings.json` SessionEnd hook 含 `ROOSTERY_AGENT=cc ~/.roostery/scripts/agent_stop_notify.sh` 条目（与既有条目共存，hooks_merge 不覆盖）

### 5. shim transparent forward 验证

**注意**：新开一个 shell（让 ~/.zshrc 自动 source）或 `source ~/.roostery/env`，否则当前 shell 的 shim 进程拿不到 `ROOSTERY_REAL_LARK_CLI` env。

```bash
source ~/.roostery/env  # 同 shell 验证用
lark-cli --version  # 期望：lark-cli version 1.0.29（或当前 pin 版本）
tail -3 ~/.roostery/journal/$(date +%Y-%m-%d).jsonl  # 期望最后一条含 source="shim" action="lark-cli:--version"
```

journal 自动落档 + lark-cli 返回真实版本号 = shim 拦截 + forward + journal 三件齐备。

### 6. 被动 CC SessionEnd 路径模拟真飞书出 task

**注意**：每次 Bash 调用都是新 shell，env 不持久——这一步要显式带 env。

```bash
echo '{"cwd":"<test-cwd>","session_id":"dogfood-<date>-passive","prompt_response":"<dogfood 标记>","hook_event_name":"SessionEnd"}' \
  | ROOSTERY_AGENT=cc \
    ROOSTERY_NOTIFY_TO=<your-open-id> \
    ROOSTERY_REAL_LARK_CLI=$REAL_LARK_CLI \
    ./target/release/roostery bot stop-hook --json --strict
```

期望 stdout JSON：
```json
{"status":"success",
 "task_url":"https://applink.feishu.cn/client/todo/detail?guid=...",
 "task_guid":"<uuid>",
 "fallback_used":false}
```

exit=0。手动用飞书 app / 网页打开 task_url 看 task 创建 + step 内容。

### 7. 主动反向 push 验证（与 §6 走同一核心 lib fn，但模拟反向调用）

```bash
./target/release/roostery bot push \
  --agent dogfood --session $(date +%Y-%m-%d-active) \
  --cwd "$(pwd)" \
  --summary "<dogfood 标记>" \
  --assignee-open-id <your-open-id> \
  --json --strict
```

同 §6 期望。

### 8. cleanup（可选）

dogfood 结束想恢复原状：

```bash
# 移除 shim
rm ~/.local/bin/lark-cli
# 恢复 npm symlink
mv ~/.local/bin/lark-cli.npm-bak ~/.local/bin/lark-cli  # 若 §4.1 backup 过
# 或重新 npm install -g @larksuite/cli

# 清 roostery 装机产物（保留 journal 留审计）
rm -rf ~/.roostery/scripts ~/.roostery/env
# 手动从 ~/.zshrc 删 # >>> roostery >>> ... # <<< roostery <<< 整段

# 清 ~/.claude/settings.json SessionEnd 里 ROOSTERY_AGENT 条目（手编 JSON）
```

journal 留着（`~/.roostery/journal/`）—— audit trail 有价值。

## 常见踩坑（按发生频率排序）

### P1: `ShimTargetConflict`（最高频）

```
[roostery init] shim target /Users/<u>/.local/bin/lark-cli exists and is not a roostery shim;
back it up and remove first, then re-run `roostery init`
```

**原因**：已有 npm symlink / brew binary / 旧 Python feishu_hub shim 占着 `~/.local/bin/lark-cli`。
**修法**：`mv ~/.local/bin/lark-cli ~/.local/bin/lark-cli.bak` 再重跑 init。

### P2: `LarkCliCollidesShimTarget`（feature 落地前撞，已修）

PATH 上唯一 lark-cli == shim target。已被 init-real-lark-cli-override feature 根治；现在 init 会拒装并给 hint。**走 --real-lark-cli flag 绕过**。

### P3: shim forward 失败：`ROOSTERY_REAL_LARK_CLI not set`

```
[roostery] ROOSTERY_REAL_LARK_CLI not set; run `roostery init`
```

**原因**：当前 shell 没 source `~/.roostery/env`。
**修法**：开新 shell；或同 shell 内 `source ~/.roostery/env`；或在调用前显式 `ROOSTERY_REAL_LARK_CLI=<path> <command>`。

### P4: identity unavailable（non-fatal warning）

```
identity: (unavailable — lark-cli auth status failed: ...)
```

**原因**：init 子进程调 `lark-cli auth status` 时 env 还没设好（init 写 env 之前用户 shell 已经在新进程跑 init）。**non-fatal**，init 仍 OK；用户后续真跑 agent 时 identity 已可用。

## 与 feature 边界

本 checklist 是**操作步骤**——既不是 design / accept doc 也不是 test suite。文件级集成测试覆盖 §1-§6 各点（`tests/onboarding_integration.rs` + `tests/bot_cli_integration.rs`），但完整环境的真飞书写入只能手工跑。

未来如果发现需要每次 release 自动化跑：写 `scripts/dogfood-init.sh` 包装本 checklist 步骤，加 CI nightly job（需注入飞书 token secret）。**本期不做**——0.1.0 release 之前手工跑 1-2 次足够。

## 相关

- feature `2026-05-18-roostery-init` — onboarding pipeline 落地
- feature `2026-05-18-init-real-lark-cli-override` — UX hole 修复 + 本 checklist 抽离的原始 dogfood 场景
- issue `2026-05-18-init-shim-conflicts-npm-prefix` — npm prefix == shim target collision 触发 dogfood 发现
- ARCHITECTURE.md §6 #10（`ROOSTERY_REAL_LARK_CLI` 持久化路径）+ #19（`ROOSTERY_LARK_CLI_BIN` 双语义复用）
