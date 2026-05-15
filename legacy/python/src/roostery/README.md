# roostery — agent ↔ 飞书办公中台

让本机 AI agent（CC / Codex / GA / fsapp ...）跟飞书生态打通的胶水层。**不是飞书 SDK 的包装**——飞书侧的事情交给官方 [`lark-cli`](https://github.com/larksuite/cli)，本模块只做"飞书外的事"：本机审计、安装、配置、hook 桥接、本机执行流编排。

---

## 一句话心智模型

> **飞书是共享状态机，`lark-cli` 是 agent 对飞书的系统调用；`roostery` 只保留执行桥和审计缓存。**

详细心智模型与红线见 `docs/LARK_CLI_CAPABILITIES.md` 以及 memory `feishu_collaboration_model.md`。**违反这条模型的代码会被持续删除**——见下方 M3.A 变更说明。

### 状态归处

| 状态 | 归谁 |
|---|---|
| 工作项生命周期、agent 执行步骤流 | 飞书 Task（`lark-cli task +create / agent_task_step_info append_task_steps`） |
| 跨 agent 即时上下文 | 飞书 IM thread（`lark-cli im +messages-reply --thread`） |
| 协作痕迹、评论 | 飞书 Docs 评论 / 群消息 |
| 索引、统计、看板视图 | 飞书 Base（不是事实源，是索引层） |
| 云侧路由（@mention / 关键词 / 定时） | 飞书 Base Workflow（`LarkMessageTrigger` / `TimerTrigger`） |
| 本机进程、模型调用、重试预算 | 本地（dispatcher.runners / budget） |
| 审计、排错重放 | 本地 journal jsonl |

---

## 模块结构（M3.A 之后）

```
roostery/
├── lark_cli.py         # 稳定封装 lark-cli 调用（subprocess + JSON 解析 + 异常归一化）
├── shim.py             # ~/.local/bin/lark-cli shim：透传 + 脱敏 + 审计
├── journal.py          # 本机 jsonl 审计落盘（不是协作事实源）
├── redact.py           # 敏感字段脱敏
├── remoterefs.py       # 从 stdout 抽取飞书远端 token（doc_token / record_id 等）
├── config.py           # ~/.feishu_hub/config.yaml 读写
├── hooks_merge.py      # CC/Codex hook 配置合并
├── smoke.py            # 升级后回归探针（im +messages-send / docs +create 等已验证命令）
├── lark_cli.py         # 同上（高层包裹）
├── git_log.py          # 多仓库 git log 聚合（日报数据源）
├── llm_summary.py      # 日报摘要（调 GA llmcore；唯一允许 import ga 的文件）
├── daily_report.py     # 本机复盘日报（与飞书 lark-workflow-standup-report 互补，不冲突）
├── task_writer.py      # bot 创建 task + 追加执行步骤 + session 缓存（M3.B 主路径）
├── stop_hook.py        # shell→python 桥，task_writer 主路径 + IM 兜底（shell 调不了复杂 JSON）
├── templates/          # CC/Codex 配置片段 + Stop hook 脚本
└── dispatcher/         # 本机执行桥（M3.A 后 thin bridge）
    ├── cli.py          # fire（hook 单次）+ replay（调试）+ test-rule
    ├── loop.py         # event → match rules → trace/budget → run runner → emit
    ├── rules.py        # 本机 hook → runner 匹配
    ├── runners.py      # cc_headless / codex_exec / gemini_headless / noop
    ├── trace.py        # trace_id / depth / parent_event_id 链路保护
    └── budget.py       # 调用次数 + 成本预算
```

---

## 快速上手

### 安装与配置

```bash
# 1. 装 lark-cli + 它的 skill 包（飞书原生命令面）
npm install -g @larksuite/cli
npx skills add larksuite/cli -y -g

# 2. 登录飞书（按需）
lark-cli auth login --domain im,task,docs,base,drive,vc

# 3. 初始化 roostery
python -m roostery init
```

`init` 会：
- 建 `~/.feishu_hub/` 目录树（journal / state / bin）
- 部署 `agent-stop-notify.sh` 到 `~/.feishu_hub/bin/`
- 把 CC/Codex Stop hook 配置 merge 进 `~/.claude/settings.json` / `~/.codex/hooks.json`
- 在 PATH 前部署 lark-cli shim（透传真实 lark-cli + 落 journal）
- 引导填 `notify_receive_id`、`daily_report.root_folder_token` 等

### 升级后回归

```bash
python -m roostery smoke
```

跑全部已验证的 lark-cli 命令矩阵（`im +messages-send` / `docs +create v2` / `docs +update overwrite` / `drive files list` / `drive +create-folder` / `drive move`），任一失败时 init 与 daily_report 拒绝执行，错误信息指向具体命令。

---

## 给 AI agent 的安装协议（machine-readable onboarding）

> **如果你是被用户邀请来帮忙装这个模块的 AI agent**（CC / Codex / GA / Cursor / 任何带 Bash 工具的 agent），照本节执行。每一步给你：要跑的命令 / 期望输出特征 / 失败时往哪查 / 何时必须停下问用户。
>
> **设计前提**：用户已经把这个 repo 拉到本地，且当前 shell 在 repo 根目录。

### 0. 前置检查（30 秒）

```bash
node --version   # 期望 ≥ 18
python3 --version   # 期望 ≥ 3.10
which jq || echo "MISSING: jq"   # macOS: brew install jq；Linux: apt install jq
git rev-parse --is-inside-work-tree   # 期望 true
```

**失败处理**：缺什么装什么；jq 缺失会让 Stop hook 静默退化为"无 summary"，不影响主流程但弱化体验。

### 1. 装 lark-cli + 它的 skill 包

```bash
npm install -g @larksuite/cli
lark-cli --version   # 期望出 1.0.28 或更高
npx skills add larksuite/cli -y -g
```

**为什么**：lark-cli 是飞书官方 CLI；skill 包给 lark-cli 配套"AI agent 友好的工具说明"（你后续可能会读到 `~/.local/lib/node_modules/@larksuite/cli/skills/` 或类似路径下的 SKILL.md）。

**停下问用户**：如果 lark-cli 版本 ≥ 1.1.0，**先停下报告版本号**——本仓库锁 1.0.28 实测兼容，更高版本可能有 schema 漂移（特别是 `task agent_task_step_info append_task_steps` 的 timestamp 字段）。

### 2. 让用户建一个自己的飞书 app（无法替代）

```bash
lark-cli config init --new
```

这条命令会输出一个**授权 URL**，你必须：
1. 把 URL 从命令输出里抽出来
2. 发给用户让他在浏览器打开
3. 等用户在飞书开放平台完成 app 创建 + 授权
4. 命令会自己退出

**为什么用户必须自己建**：飞书 app 是用户级资源（appId/appSecret 归属用户飞书账号），你不能给他用别人的 appId，也没有"官方共享 app"。

**期望输出**：`~/.config/lark-cli/` 或类似路径出现配置文件；`lark-cli auth status` 不再报"no app configured"。

### 3. 用户身份授权（user_access_token）

```bash
lark-cli auth login --scope "task:task:write task:task:read im:message im:message.send_as_user contact:user.base:readonly docs:document:write_only docs:document.content:read drive:file:upload drive:file:download base:app:read base:record:read base:record:create base:record:update"
```

这条同样会输出授权 URL；同步骤 2 的处理方式。

**为什么这些 scope**：
- `task:*` — M3.B 主路径写飞书任务
- `im:*` — IM 兜底（task 路径失败时）
- `contact:user.base:readonly` — 拿用户自己的 open_id（下一步用）
- `docs:*` + `drive:*` — daily_report 写日报 docx
- `base:*` — M2 历史兼容（M3.D 索引层后会用到）

**期望输出**：`lark-cli auth status --jq '.tokenStatus'` 返回 `"valid"`。

### 4. 拿用户的 open_id 设环境变量

```bash
USER_OPEN_ID=$(lark-cli contact +get-user --as user --jq '.open_id')
echo "User open_id: $USER_OPEN_ID"
```

把这一行加进用户的 shell rc（`~/.zshrc` / `~/.bashrc`）：

```bash
export FEISHU_NOTIFY_TO=ou_xxxxx   # 把 $USER_OPEN_ID 实际值填进去
```

**为什么**：Stop hook 用 `FEISHU_NOTIFY_TO` 决定把 task follower 设为谁。如果不设，hook 静默退出（无错但也无飞书侧产出）。

### 5. 初始化 roostery 本机配置

```bash
python3 -m roostery init
```

这条会：
- 建 `~/.feishu_hub/{journal,state,bin}/` 目录树
- 部署 `~/.feishu_hub/bin/agent-stop-notify.sh`（Stop hook 用）
- 把 CC `~/.claude/settings.json` + Codex `~/.codex/hooks.json` 的 Stop hook merge 进去（不覆盖已有 hooks）
- 在 PATH 前部署 lark-cli shim（透传真实 lark-cli + 落 journal）
- 引导用户填 `notify_receive_id`（如果环境变量 `FEISHU_NOTIFY_TO` 已设，自动用它）

**停下问用户**：如果 init 提问 `daily_report.root_folder_token`（飞书云空间根文件夹），用户没建过的话直接回车跳过即可，daily_report 走个人空间。

### 6. 跑 smoke 探针验收

```bash
python3 -m roostery smoke
```

**期望输出**：6 个 probe 全 `ok: true`（`im_messages_send` / `docs_create_v2` / `docs_update_overwrite` / `drive_files_list` / `drive_create_folder` / `drive_move`）。

**任一失败就停下**：把失败 probe 名 + 它返回的飞书 error code 给用户，多半是 scope 缺漏或 token 过期。**不要**自己试着 `auth login --recommend` 拿全 scope——那会授权一堆用不到的权限，违反最小授权。

### 7. 跑一次真实 CC 会话验收端到端

```bash
# 跑任意 CC headless 命令，让它正常退出触发 Stop hook
claude -p "say hello" >/dev/null
```

**期望**：用户飞书 app 出现一条新任务，标题形如 `[cc] @<basename(cwd)>`，点开能看到 1 条 step（任务摘要）。

**如果飞书没出现任务**：
```bash
tail -20 ~/.feishu_hub/journal/$(date +%Y-%m-%d).jsonl   # 看 stop_hook 那次调用的 envelope
```
日志里如果有 `lark_cli.invoke` 但是 `exit_code != 0`，看 `io.stderr_head` 字段拿 lark-cli 错误码，再针对性排查。

### 8. 把成功状态告诉用户

完成后给用户报告：
- lark-cli 版本号
- user open_id
- 测试 task 在飞书 app 里的 URL（从 journal 里抽 task_guid 拼 `https://applink.feishu.cn/client/todo/detail?guid=<guid>`）
- 一条总结："Stop hook 已激活，下次 CC/Codex 完成任务会自动写飞书任务流"

### 失败兜底

任何一步卡住超过 3 次重试，**停下报告"BLOCKED on step N: <错误摘要>"，让用户决定**。不要在用户没看到的情况下静默 fallback 到旧路径（比如直接发 IM text），这违反"对用户透明"原则。

### 多 user / 多 bot / 多机（统一通过 lark-cli profile）

**心智模型**：lark-cli 的 **profile** 就是 roostery 的身份单位——每个 profile = `(appId, brand, user)` 元组。roostery 不发明 identity 概念，所有"是哪个 user / 是哪个 bot"问题都通过 `lark-cli profile` 解。

```bash
# 一行看清当前活跃身份
python -m roostery whoami
# ✓ profile=cli_a9449f8a2f799bdb user=dustben (ou_xxx) bot=a9449f8a (cli_xxx) host=M4 token=valid

# 切到另一个 bot/user（lark-cli 1.0.28 已支持）
lark-cli profile use <name>
python -m roostery whoami   # 跟随更新
```

适用场景：

| 场景 | 怎么配 |
|---|---|
| 一台机器、一个用户、一个 bot | 装好后默认 profile 即可，不用切 |
| 一个用户、多个 bot（如 reviewer/scribe 分工 → M3.C） | `lark-cli profile add` 每个 bot 一个 profile；跑 agent 前 `profile use` 切相应 bot |
| 一个用户、多台机器（mac + axis） | 每台机器**各建独立 app**（不要共享 appSecret），各自 `lark-cli config init` 配自己的 profile |
| 同台机器、多个用户（家庭 + 工作） | 每个 user 一个 profile，按需 `profile use` 切 |

**默认行为**（M3.B 后）：
- task_writer.create_task 不显式传 `assignee_open_id` 时，从 `lark-cli auth status` 取当前 active profile 的 user_open_id → 切 profile 自动改变 task 归属
- summary 自动后缀 `· {hostname}` 区分机器；可 `export FEISHU_HUB_HOST=axis` 覆盖
- IM fallback 也走同一 active profile，多 bot 时不会跨身份发消息

**多机协同**（一台 bot @mention 另一台 bot）属 M3.C 范围；本期已通过 host suffix 让飞书 task UI 区分来源。

### M3.D — Base 反向 indexer（2026-05-13）

**它解决了什么问题**：飞书 Task UI 适合"一项一项处理"（user 在收件箱点开任务看时间线），但**不适合跨工作项查询**（按 agent 统计耗时？按 host 分桶看哪台机器跑得多？按 cwd 看每个项目消耗？）。Task 是事实源，Base 是索引层——M3.D 把 Task 列表定期刷到 Base，看板/网格/甘特视图按任意维度切片。

**协同模型再确认**（M3.A 红线）：
- Task → Base **单向**派生；用户在 Base 改了什么**不**回写 Task
- 只索引 ``creator.type == "app"`` 的 task（agent 建的）；user 手建的工作 task 不进 Base 避免污染

**用法**：

```bash
# 1. 首次：建 M3.D 所需字段（task_guid / host / creator_app_id / assignee / task_url / last_synced）
python -m roostery indexer migrate-schema

# 2. 增量刷（默认；用 cursor.json 推进）
python -m roostery indexer run

# 3. 全量校准（不依赖 cursor；周日跑一次防漂移）
python -m roostery indexer run --full

# 4. 看上次跑的结果 + 当前 cursor
python -m roostery indexer status
```

**自动化**：M3.E 会加 launchd plist 30min 一次 cron；当前手工触发或集成到现有 cron。

**飞书 Base 看板视图配置**（用户在飞书 UI 手建一次即可）：

| 视图名 | 类型 | 分组键 | 过滤 |
|---|---|---|---|
| 今日活跃 | 网格 | — | ``last_synced >= TODAY()`` |
| 按 Agent 分组 | 看板 | Agent | — |
| 按机器分组 | 看板 | host | — |
| 任务时间线 | 甘特 | — | 开始字段=创建时间, 结束=完成时间 |
| 失败/超时 | 网格 | — | ``状态 ∈ {失败, 超时}`` |

**对下游的影响**：
- 之前直接读 Base ``agent_tasks`` 表的脚本：M3.B 之前数据是 dispatcher 直接写的；M3.D 之后**只有 agent 创建的 task 才进 Base**。M2 时期的旧行不会被 indexer 覆盖（旧行 task_guid 字段为空）。
- 想看真正实时的 agent 状态：飞书 Task UI > Base（Base 有 30min 延迟）；Base 强项是统计 / 跨工作项视图。

---

## 变更历史

### M3.A — 清理与降维（2026-05-13）

把违反"飞书=共享状态机"协同模型的本地代码删干净，dispatcher 瘦身成"事件→本机 runner"的薄桥。

**它解决了什么问题**：

| 之前的痛 | 根因 | M3.A 做了什么 |
|---|---|---|
| dispatcher 把所有 agent 事件落到本地 bitable_writer，再写飞书 Base 表 | 把"共享状态"回退成"本机派生" — 跨 agent 看不到本地文件，飞书侧总落后一步 | **删 `dispatcher/bitable_writer.py`**；状态写飞书由 hook/runner 直接调 lark-cli（M3.B 落地）|
| dispatcher 长进程 `tail -F` 本地 journal jsonl 当事件源 | 本地 journal 是审计缓存，不是协作事件源；事件源应该是 `lark-cli event consume <key>` | **删 dispatcher `tail` 子命令** + 相关 checkpoint 文件 + launchd 集成 |
| `dispatcher.emit` 被 `bitable_writer.wrap_emit` 强制包一层 | dispatcher 越界写飞书 | **解开 wrap_emit**；emit 回调只用于本机 journal 审计 |
| 设计文档跟代码现状脱节，"3 种触发模式" / "tail by launchd" 等过时描述持续误导 | 历史包袱 | **5 处文档同步**：`loop.py` / `rules.py` docstring + `DISPATCHER_DESIGN.md` + `M2_COVERAGE.md` + `CC_HEADLESS_AUTH.md` + `M3_DESIGN.md` v2 标作废 |

**它没做什么**（留给 M3.B/C）：

- 没建立 task 域主路径（CC/Codex Stop hook 现在仍写本地 journal + 发 IM 文本；不写飞书 Task）
- 没启用 bot@bot 协作（M3.C）
- M2 bitable `agent_tasks` 表数据不动——保留但**降级为索引层**，M3.B 后由独立 indexer 从飞书 Task 列表反向刷出

**数据**：

- 1 文件删除（166 行）+ 2 文件大幅收缩（cli.py 316→194 / 老 cli 测试 208→153）
- 新增 1 个契约测试文件（`tests/test_roostery_dispatcher_cli_m3a.py`）固化"不再 import bitable_writer / 不再有 tail 子命令"
- 7 个 commit，349/349 测试 PASS，smoke 6/6 PASS
- 实施计划：`docs/superpowers/plans/2026-05-13-feishu-hub-m3a-cleanup.md`

**对下游的影响**：

- 用过 `python -m roostery.dispatcher tail` 的 launchd plist / cron / shell 脚本：**会断**。改用 `python -m roostery.dispatcher fire` 单次直触（接 CC/Codex Stop hook 调用），或 `replay` 重放本机 journal 调试。
- 直接 import `roostery.dispatcher.bitable_writer` 的代码：**会断**。M3.B 完成后通过 `lark-cli task +create` 路径替代。

### M3.B — Task 域主路径接入（2026-05-13）

把 CC/Codex Stop hook 的飞书侧从"发 IM 纯文本"升级到"创建飞书 Task + 追加执行步骤"。agent 工作流首次在飞书 UI 以工作项形式可见、可跟进。

**它解决了什么问题**：

| 之前 | M3.B 后 |
|---|---|
| Stop hook 只发 IM text，用户在飞书看到一行通知，但**点不动**，看不到 agent 做了哪几步 | bot 创建/复用 task（user 作 follower），bot 追加执行步骤；user 在飞书 Task UI 直接点开看时间线 |
| 同一 session 多次触发 hook 各发独立 IM，刷屏 | 同 `(agent, session)` 复用 task，追加 step；本地缓存 `~/.feishu_hub/state/session_tasks/<agent>-<session>.json` |
| `bitable_demo.py` 建 agent_tasks 表，把 Base 当 agent 执行流的事实源 | 删除；Base 留 M3.D 作为索引层从 task 列表反向刷出 |

**关键约束**（lark-cli 1.0.28 POC 已踩坑、代码已固化）：
- `task agent_task_step_info append_task_steps` 必须 `--as bot`
- bot 必须是 task 创建者（user 创建的 task → bot 写 step 会 10403）
- `timestamp` 字段在 1.0.28 序列化 bug，必须省略

**新模块**：`roostery/task_writer.py`（bot 创建任务 + 追加步骤 + session 缓存 3 个 API）+ `roostery/stop_hook.py`（shell→python 桥，避免 shell 拼复杂 JSON）。

**降级路径**：lark-cli 调用失败时自动 fallback 到 IM text，保证 agent 不会因 Task 路径故障静默丢失通知。

**对下游的影响**：
- Stop hook 的 IM 文本通知形态变了——现在主路径是飞书 Task；只有 lark-cli 失败时才回退发 IM
- 直接 import 或运行 `python -m roostery.bitable_demo` 的脚本：**会断**。M3.D 反向 indexer 落地前先用 lark-cli `task +get-related-tasks` 查询

### M3.C — bot@bot 协作（2026-05-14）

多个 bot daemon 通过群里 `@mention` 互相接力。一个 bot daemon = 一个 lark-cli profile = 一个角色（reviewer / scribe / ...）。daemon 收到 `@me` 就调本机 runner，把结果 `--reply-in-thread` 回到原 thread；若配了 `next_bot_mention` 则末尾追加 `<下一个bot> 请接力。`，由下游 bot 的 daemon 接收事件继续。每次接力同步落一条 step 到飞书 Task（按 chat_id 维度复用），管理者从飞书"我的待办"能看完整接力链。

**新模块**：

| 模块 | 责任 |
|---|---|
| `roostery/bot_role.py` | bots.yaml 加载 + 单 bot 事件匹配（chat_type=group + message_type=text + 内容以 `@<mention_alias>` 起头） |
| `roostery/event_bridge.py` | `lark-cli event consume im.message.receive_v1 --as bot --profile X` 子进程编排（ready marker / NDJSON / stdin keepalive / SIGTERM graceful exit） |
| `roostery/bot_runner.py` | event → 模板化 prompt → 调 runner → 模板化 reply → `im +messages-reply --reply-in-thread`（runner 失败 / 超时也回 IM，对人可见） |
| `roostery/bot_relay_task.py` | 每次接力同步飞书 Task 步骤；按 chat_id 复用同一 task（cache `~/.feishu_hub/state/m3c_chats/<chat_id>.json`） |
| `roostery/bot_bridge.py` | daemon orchestrator：consume_im → handle_event 循环；单条事件异常被吞，daemon 整体续跑 |

**新 CLI**：

- `python -m roostery bot-bridge --role reviewer --timeout 5m` 启 daemon
- `python -m roostery status` 看板：identity / bots.yaml / 接力链 / daemon 进程

**配置文件**：`~/.feishu_hub/bots.yaml`（模板：`roostery/templates/bots.yaml.tmpl`）。每个 bot 一条 `{app_id, role, mention_alias, runner, default_cwd, prompt_template, reply_template?, chat_whitelist?, next_bot_mention?}`。

**关键约束**（lark-cli 1.0.28 实测，违反会静默踩坑）：

- `lark-cli event consume` 的 stdin **必须**给永不 EOF 的源——`/dev/null` 会让 lark-cli 立刻 `[event] stdin closed — shutting down`。本模块用 `tail -f /dev/null` 子进程的 stdout 做 keepalive
- 退出 daemon 必须 SIGTERM 等 graceful exit，不能 SIGKILL（PreConsume 订阅会泄漏到下次启动）
- `im +messages-reply` 的真实 flag 是 `--reply-in-thread`，不是 plan/POC memory 里早期写的 `--thread`
- `--profile X` 是 lark-cli **global** flag，必须在子命令前（`lark-cli --profile X im ...`）
- 一个 bot app 全局只能有**一个** event bus；同 app 跨机抢 bus 会冲突——双 bot 真机 e2e 要求两个 bot app 装在同一台机

**未做**：双 bot 双机真机 e2e（idempotency dedupe 服务端已验证，链路只剩跨机 e2e 没跑）。代码层全套 473 tests 绿，单机单 bot smoke 通过（2026-05-14：run 2 拿到 src=om_xxx → reply=om_yyy → relay_task=fa180257-...）。

#### 北极星：agent 网络 / 数字分身（M3.C 是这条路的第一步）

> 真正的目标：**每个用户都有自己的 agent 在飞书代表他，整个团队变成一个 agent 网络**——人提出目标 / 拍板，agent 在前面跑事；agent 之间通过飞书 IM 互相 @ 接力交付；过程对人完全可见、能插话、能打断、能纠偏；重要事不会漏。

M3.C 5 模块（event_bridge / bot_role / bot_runner / bot_relay_task / bot_bridge）= 这条路的第一步，**做了底座的"反向触发 → 本机 AI 跑活 → 写回飞书"链路**。还差很多（HITL 中断 / workflow 复用 / SOP 传递 / multi-identity 分类），见 `docs/FEISHU_HUB_REQUIREMENTS.md`。

#### 多机部署的工程实现细节

**这套不是主从，是「角色分片 + 飞书做 message broker」。**

```
飞书 IM thread                ←  调度面（人 / 上游 bot 用 @mention 做路由）
   │
   ├─ @reviewer    →  机器 A 上的 reviewer bot daemon （消费 app_A 的 event bus）
   ├─ @scribe      →  机器 B 上的 scribe   bot daemon （消费 app_B 的 event bus）
   └─ @sandbox     →  机器 C 上的 sandbox  bot daemon （消费 app_C 的 event bus）
```

工程性质：

1. **路由原语是 IM @mention，不是 IP/SSH/hostname**。调度做在飞书侧。
2. **leader election 由飞书物理约束代办**。"一个 bot app 全局只能有一个 event bus" → 不需要选主代码。
3. **加机器 = 声明式**。飞书开放平台新建 app → 那台机 `lark-cli profile add` → bots.yaml 加一条 → 启 daemon。
4. **跨机共享状态靠 server-side idempotency + 统一身份写手**。`bot_relay_task` 的 chat → task 映射用 `idempotency_key='m3c-relay-task:<chat_id>'`；飞书 task `--idempotency-key` 是 **per-bot-app namespace**——通过 `BotRole.relay_writer_app_id` 字段把所有机器的 relay_task 绑到同一身份写手，在 TTL ~30min 内跨机收敛同 guid。
5. **机器宕机/掉线 = 那个 bot 没响应**。飞书 thread "等待 reply" 肉眼可见；`python -m roostery status` 看 daemon 死活。不需要心跳服务。
6. **机器之间不需要任何网络通道**。所有通信走飞书 IM。

引申：飞书 IM thread = 这套系统的 message broker；飞书 task = 工作流引擎。不需 Kafka / NATS / Temporal。

**注意 ⚠️**：`relay_writer_app_id` 是当前 per-bot-app idempotency 限制的补丁，**不是长期方案**。等 workflow template 概念落地后会重构。详见 REQUIREMENTS R12/R14。

详细心智模型见 memory `feishu_m3c_multimachine.md`。

---

## 进一步阅读

- **协同模型**（必读）：`docs/LARK_CLI_CAPABILITIES.md`
- **dispatcher 设计**：`docs/FEISHU_HUB_DISPATCHER_DESIGN.md`（含 M3.A 后变更节）
- **M2 历史覆盖**：`docs/FEISHU_HUB_M2_COVERAGE.md`
- **M3 v2 设计稿**（部分作废）：`docs/FEISHU_HUB_M3_DESIGN.md`
- **场景清单**：`docs/FEISHU_HUB_SCENARIOS.md`
- **CC headless 认证**：`docs/CC_HEADLESS_AUTH.md`
- **lark-cli 官方 skill 仓库**：https://github.com/larksuite/cli/tree/main/skills

---

## 贡献指引

写 / 改 `roostery/*` 任何代码前，先答这两题：

1. **这个状态归飞书拿还是本地拿？** 答案在飞书侧才动飞书代码；答案在本地侧才动本地代码。
2. **本地要做的事，是不是 `lark-cli` 子进程契约（NDJSON 流 / ready marker / stdin EOF / PreConsume 反订阅）已经做了？** 是 → 删掉重写。

违反这两条的 PR 会被 review 退回。
