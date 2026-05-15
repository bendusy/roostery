"""onboarding — 装机完成后在飞书侧自动建 3 个引导任务 + 1 个 Base 看板示例行。

产品理念：**安装的最后一步在飞书产品面上完成**——用户首次打开飞书 app
"我的待办"立即看到 3 条解释 roostery 是什么 / 怎么用的任务，每条都是
一个 agent step 流示范 + 跳链按钮。用户不需要读 README 就能上手。

调用方：``python -m roostery init`` 末尾默认调；``python -m roostery guide``
可单独重跑（场景：清理过引导后想重新看示例）。

3 条引导任务的设计：
1. **"欢迎"任务**：3 个 step 自述当前 (user, bot, host) 三元身份，证明
   "你看到的就是 agent 自动建的任务"
2. **"跑一次 CC 触发 Stop hook"任务**：0 step（空 task 等用户跑命令后
   Stop hook 自动 append）
3. **"打开 Base 看看板"任务**：含 task_url 跳到 Base bitable URL，加
   1 条 step 解释 indexer 用法

不创建任务的兜底：identity.is_ready 假时 print 警告，return 不抛。
"""
from __future__ import annotations

import datetime as _dt
from typing import Dict, List, Optional

from roostery import task_writer
from roostery.identity import current_identity
from roostery.task_writer import TaskRef


def _welcome_task(user_open_id: str, bot_app_id: str, host: str) -> Dict[str, str]:
    return {
        "summary": "👋 欢迎使用 roostery — 这条任务由 agent 自动创建",
        "description": (
            "你正在看的这个任务就是 ``lark-cli task +create`` 自动建的；"
            "下面的执行步骤是 ``task agent_task_step_info append_task_steps`` 自动追加的。"
            "今后每次 CC / Codex / GA 在本机完成任务，都会按 (agent, session) 维度"
            "复用或新建任务到你这个收件箱，让 agent 的工作流跨终端可见、可跟进。\n\n"
            "本机当前身份：\n"
            f"- profile: lark-cli active profile\n"
            f"- user: {user_open_id}\n"
            f"- bot:  {bot_app_id}\n"
            f"- host: {host}\n\n"
            "切换身份：`lark-cli profile use <name>` + `python -m roostery whoami` 验证。"
        ),
        "steps": [
            f"已识别 user open_id = {user_open_id}",
            f"已识别 bot app_id = {bot_app_id}（roostery 跟随当前 lark-cli profile，不发明新 identity）",
            f"已识别 host = {host}（多机部署时 task summary 自动后缀 · {host}）",
        ],
        "session": "onboarding-welcome",
    }


def _try_it_task() -> Dict[str, str]:
    return {
        "summary": "🚀 试试：回到终端跑一句 `claude -p 'hi'`",
        "description": (
            "Stop hook 已经装好。你只要让 CC 正常完成一次会话，就会触发：\n\n"
            "1. shell 脚本 ``~/.feishu_hub/bin/agent-stop-notify.sh`` 抓 stdin JSON\n"
            "2. 调 ``python -m roostery.stop_hook``\n"
            "3. ``task_writer.get_or_create_for_session`` 复用或新建任务\n"
            "4. ``append_steps`` 在那个任务里追加这次会话的摘要\n\n"
            "你应该会在飞书 inbox 看到一个**新的**任务（不是这条），summary 形如 "
            "``[cc] @ <你的项目名> · <你的机器名>``。"
        ),
        "steps": [],  # 故意留空，让用户跑了 CC 之后才看到任务被填满（**别的**任务）
        "session": "onboarding-try-it",
    }


def _base_view_task(base_url: Optional[str]) -> Dict[str, str]:
    base_hint = (
        f"打开 Base：{base_url}\n\n"
        if base_url
        else "Base 看板 URL 没配（config.yaml 缺 bitable.base_token）。请先建好 Base 表，再跑 `python -m roostery indexer migrate-schema`。\n\n"
    )
    return {
        "summary": "📊 跨工作项统计在 Base — 跑一次 indexer 看效果",
        "description": (
            "飞书 Task 适合一项一项跟进；但你想问『按 agent 统计耗时』『哪台机器跑得多』"
            "『按项目分桶看消耗』时，需要跨工作项查询能力——那是飞书 Base 的强项。\n\n"
            "roostery indexer 把 Task 列表反向刷到 Base 索引表，建好的看板/网格/甘特"
            "视图能任意切片：\n\n"
            f"{base_hint}"
            "操作：\n"
            "1. `python -m roostery indexer migrate-schema`（首次：建 6 个新字段）\n"
            "2. `python -m roostery indexer run --full`（全量刷一次）\n"
            "3. 在飞书 Base UI 建看板视图：分组键 = ``Agent`` / ``host`` / ``状态``\n\n"
            "之后 30min 一次 cron 自动增量同步（M3.E）。"
        ),
        "steps": [
            "indexer 触发：`python -m roostery indexer run` (增量) / `--full` (全量校准)",
            "看板维度：Agent / host / 状态 / 创建时间 / 耗时(s) / 成本(¢) / Tokens",
            "Task → Base 单向；Base 改了不回写 Task（M3.A 协同模型）",
        ],
        "session": "onboarding-base-view",
    }


def create_welcome_tasks(*, base_url: Optional[str] = None) -> List[TaskRef]:
    """主入口。创建 3 个引导任务并返回 TaskRef 列表（按上述顺序）。

    如果 identity 不齐（没登录 / 没建 profile），打印警告并 return []。
    任何单条建任失败不阻塞其他条（best-effort）。
    """
    import sys

    ident = current_identity()
    if not ident.is_ready:
        print(f"[onboarding] identity 不齐（{ident.describe()}）—— 跳过引导任务", file=sys.stderr)
        return []

    user_open_id = ident.user_open_id
    bot_app_id = ident.bot_app_id
    host = ident.host

    blueprints = [
        _welcome_task(user_open_id, bot_app_id, host),
        _try_it_task(),
        _base_view_task(base_url),
    ]

    created: List[TaskRef] = []
    for bp in blueprints:
        try:
            ref = task_writer.get_or_create_for_session(
                agent="roostery",
                session=bp["session"],
                cwd="~",
                summary=bp["summary"],
                description=bp["description"],
                assignee_open_id=user_open_id,
            )
            if bp["steps"]:
                task_writer.append_steps(
                    ref.guid,
                    steps=bp["steps"],
                    idempotency_key=f"{bp['session']}-step-bundle",
                )
            created.append(ref)
            print(f"[onboarding] ✓ {bp['summary'][:50]} → {ref.url}")
        except Exception as e:  # noqa: BLE001
            print(f"[onboarding] ✗ {bp['summary'][:30]}: {e}", file=sys.stderr)

    return created
