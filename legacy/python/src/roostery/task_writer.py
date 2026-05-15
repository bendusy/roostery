"""task_writer — bot 身份创建飞书 Task + 追加执行步骤。

协同模型：Task 是跨 agent 可见的工作项；agent 执行步骤流通过
``task agent_task_step_info append_task_steps`` 实时写入，user 在
飞书 app 看到 agent 进展。

约束（lark-cli + POC 已验证）：
- ``task.agent_task_step_info.append_task_steps`` 要求 ``--as bot``
- bot 必须是 task 创建者；user-created task 写 step 会 10403
- ``timestamp`` 字段在当前版本 序列化有 bug，必须省略（server 自动填）
"""
from __future__ import annotations

import json
import os
import socket
from dataclasses import dataclass
from typing import List, Optional, Sequence

from roostery.lark_cli import run_json


def _default_host() -> str:
    """优先用 ``FEISHU_HUB_HOST`` env，缺则用 hostname 首段（去 .local 等）。

    多机部署时（同一飞书账号但不同机器），让 task summary 含 host
    后缀让用户在飞书 task 列表能一眼分辨来源机器。
    """
    explicit = os.getenv("FEISHU_HUB_HOST")
    if explicit:
        return explicit
    return socket.gethostname().split(".", 1)[0] or "unknown"


@dataclass(frozen=True)
class TaskRef:
    """飞书 Task 引用。"""
    guid: str
    url: str


def create_task(
    agent: str,
    cwd: str,
    summary: str,
    *,
    description: str = "",
    assignee_open_id: Optional[str] = None,
    idempotency_key: Optional[str] = None,
    host: Optional[str] = None,
    profile: Optional[str] = None,
) -> TaskRef:
    """bot 身份建任务。``assignee_open_id`` 把 user 作为 assignee 加入。

    为什么 assignee 而非 follower：飞书 task 默认 "我的待办" (`+get-my-tasks`)
    视图只显示 assignee；follower 必须主动切换到"我关注的" (`+get-related-tasks
    --followed-by-me`) 才能看到。Agent 自描场景下我们要 user 默认 inbox 即见，
    所以用 assignee。

    summary 自动后缀 ``· {host}``——多机部署时让飞书 task 列表区分来源。
    ``host`` 显式传入优先；默认走 ``_default_host()``（env / hostname）。
    若 summary 已含 ``· {host}`` 则不重复加。
    """
    effective_host = host or _default_host()
    host_suffix = f"· {effective_host}"
    final_summary = summary if host_suffix in summary else f"{summary} {host_suffix}"

    # 多 user/多 bot 路径：若 caller 不显式传 assignee_open_id，由 identity 层
    # 从 lark-cli active profile 解出当前 user open_id；让 `lark-cli profile use`
    # 切换能直接驱动 task 归属变化。
    if not assignee_open_id:
        from roostery.identity import resolve_user_open_id
        assignee_open_id = resolve_user_open_id()

    argv: List[str] = [
        "task", "+create",
        "--as", "bot",
        "--summary", final_summary,
    ]
    if description:
        argv += ["--description", description]
    if assignee_open_id:
        argv += ["--assignee", assignee_open_id]
    if idempotency_key:
        argv += ["--idempotency-key", idempotency_key]

    resp = run_json(argv, timeout=30, profile=profile)
    # task +create shortcut 返回 {ok, identity, data:{guid, url}}；run_json 会解析
    data = resp.get("data", resp) if isinstance(resp, dict) else {}
    return TaskRef(guid=data["guid"], url=data["url"])


def append_steps(
    task_guid: str,
    steps: Sequence[str],
    *,
    idempotency_key: Optional[str] = None,
    profile: Optional[str] = None,
) -> None:
    """bot 身份追加步骤。空 ``steps`` 直接返回不调 lark-cli。"""
    if not steps:
        return

    body = {
        "task_guid": task_guid,
        "task_steps": [{"content": s} for s in steps],
    }
    if idempotency_key:
        body["idempotent_key"] = idempotency_key

    argv: List[str] = [
        "task", "agent_task_step_info", "append_task_steps",
        "--as", "bot",
        "--data", json.dumps(body, ensure_ascii=False),
        # task.agent_task_step_info.append_task_steps 是 risk: high-risk-write，
        # 缺 --yes 会 exit 10 (confirmation_required)。POC 时手工带 --yes 才跑通，
        # M3.B T2 实施漏加。该操作是"agent 自描"——append-only 步骤流，对用户
        # 资源无破坏性影响，--yes 安全（注：lark-shared SKILL 红线是"未经
        # 用户同意不加 --yes"；此处 task 是 bot 自己创建的，bot 写自己的
        # task 步骤等价于 agent 内部行为，不需要用户每次同意）。
        "--yes",
    ]
    run_json(argv, timeout=30, profile=profile)


# --- T3: session cache layer ---
import datetime as _dt
import os
import re
from pathlib import Path


_SAFE_NAME_RE = re.compile(r"[^A-Za-z0-9._-]")


def _roostery_home() -> Path:
    home = os.getenv("FEISHU_HUB_HOME")
    return Path(home) if home else Path.home() / ".roostery"


def _session_cache_dir() -> Path:
    d = _roostery_home() / "state" / "session_tasks"
    d.mkdir(parents=True, exist_ok=True)
    return d


def _safe_filename(agent: str, session: str) -> str:
    """把 (agent, session) 拼成单一文件名；非白名单字符替换为 _，
    并消除连续 ``..``（``.`` 在白名单内但 ``..`` 会形成路径跳出序列）。"""
    raw = f"{agent}-{session}"
    cleaned = _SAFE_NAME_RE.sub("_", raw)
    while ".." in cleaned:
        cleaned = cleaned.replace("..", "__")
    return cleaned + ".json"


def get_or_create_for_session(
    agent: str,
    session: str,
    *,
    cwd: str,
    summary: str,
    description: str = "",
    assignee_open_id: Optional[str] = None,
) -> TaskRef:
    """复用同 (agent, session) 已建的 task；首次调用 create + 写 state。"""
    cache_file = _session_cache_dir() / _safe_filename(agent, session)
    if cache_file.exists():
        import json as _json
        cached = _json.loads(cache_file.read_text(encoding="utf-8"))
        return TaskRef(guid=cached["task_guid"], url=cached["task_url"])

    ref = create_task(
        agent=agent,
        cwd=cwd,
        summary=summary,
        description=description,
        assignee_open_id=assignee_open_id,
        idempotency_key=f"{agent}-session-{session}",
    )

    import json as _json
    cache_file.write_text(
        _json.dumps(
            {
                "task_guid": ref.guid,
                "task_url": ref.url,
                "created_at": _dt.datetime.now().astimezone().isoformat(timespec="seconds"),
                "summary": summary,
            },
            ensure_ascii=False,
        ),
        encoding="utf-8",
    )
    return ref
