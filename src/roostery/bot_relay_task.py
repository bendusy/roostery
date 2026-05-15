"""bot_relay_task — 每次 bot 接力同步写飞书 Task 步骤（M3.C → M3.E）。

把 IM 事件 + runner 结果落成飞书 task 的 step 流：

- 每个 chat_id 在本机 cache 一份 ``~/.feishu_hub/state/m3c_chats/<chat_id>.json``，
  内含该 chat 的 task ``guid`` + ``url``
- ``record_start(bot, event, message_brief)`` 在 runner 跑**之前**调一次：
  首次见到 chat → 建 task；append 起始 step（🚀 + role + brief）
- ``record_end(bot, action, result_text)`` 在 runner 跑**之后**调一次：
  append 完成 / 超时 / 失败 step（✅ / ⚠️ / ❌）到同一 task

user 在飞书 task 详情页能实时看到 agent "已收到" → "已完成 / 超时 / 失败" 的过程。

``bot.relay_writer_app_id`` 非空时所有 lark-cli 调用加 ``--profile``，把 relay_task
都绑到指定身份（跨机/跨角色收敛到同一 task guid）。这是 per-bot-app idempotency
的补丁；不是长期方案（见 docs/FEISHU_HUB_REQUIREMENTS.md R12 / R14）。
"""
from __future__ import annotations

import json
import os
import re
from pathlib import Path
from typing import TYPE_CHECKING, Any, Dict, Optional

from . import task_writer
from .bot_role import BotRole
from .task_writer import TaskRef

if TYPE_CHECKING:
    from .bot_runner import BotAction


_SAFE = re.compile(r"[^A-Za-z0-9._-]")

# step 文案截断长度（飞书 task UI 一行能舒服显示的）
_BRIEF_MAX = 80
_RESULT_MAX = 200


def _state_root() -> Path:
    home = os.getenv("FEISHU_HUB_HOME")
    base = Path(home) if home else Path.home() / ".roostery"
    d = base / "state" / "m3c_chats"
    d.mkdir(parents=True, exist_ok=True)
    return d


def _cache_path(chat_id: str) -> Path:
    safe = _SAFE.sub("_", chat_id) or "unknown"
    return _state_root() / f"{safe}.json"


def _load_cached(chat_id: str) -> Optional[TaskRef]:
    p = _cache_path(chat_id)
    if not p.exists():
        return None
    try:
        data = json.loads(p.read_text(encoding="utf-8"))
        return TaskRef(guid=data["guid"], url=data["url"])
    except (json.JSONDecodeError, KeyError, OSError):
        return None


def _save_cached(chat_id: str, ref: TaskRef) -> None:
    _cache_path(chat_id).write_text(
        json.dumps({"guid": ref.guid, "url": ref.url}, ensure_ascii=False),
        encoding="utf-8",
    )


def _short_chat(chat_id: str) -> str:
    return chat_id[-8:] if chat_id else "?"


def _short_sender(event: Dict[str, Any]) -> str:
    sid = event.get("sender_id") or ""
    return sid[-6:] if sid else "user"


def _format_start_step(bot: BotRole, event: Dict[str, Any], message_brief: str) -> str:
    """🚀 起始 step：runner 跑之前调，飞书 task 详情页立即显示 "已收到"。"""
    sender = _short_sender(event)
    brief = (message_brief or "")[:_BRIEF_MAX]
    return f"🚀 [{bot.role}] 收到 @{sender}：{brief}"


def _format_end_step(bot: BotRole, action: "BotAction", result_text: str) -> str:
    """完成态 step：emoji + role + 状态描述 + 摘要前 200 字。

    优先级：aborted > timed_out > exit_code != 0 > adjusted+success > success
    """
    if action.aborted:
        reason = action.abort_reason or "(unknown)"
        return f"⚠️ [{bot.role}] 用户请求中止 (via {reason})"
    if action.timed_out:
        return f"⚠️ [{bot.role}] 超时 (exit={action.runner_exit_code}, no completion signal)"
    if action.runner_exit_code != 0:
        snippet = (result_text or "")[:_RESULT_MAX]
        return f"❌ [{bot.role}] 失败 (exit={action.runner_exit_code})：{snippet}"
    snippet = (result_text or "(empty)")[:_RESULT_MAX]
    if action.adjust_attempts > 0:
        return f"✅ [{bot.role}] 调整后完成（#{action.adjust_attempts} 轮调整）：{snippet}"
    return f"✅ [{bot.role}] 完成：{snippet}"


def _ensure_task(chat_id: str, bot: BotRole, writer_profile: Optional[str]) -> TaskRef:
    cached = _load_cached(chat_id)
    if cached:
        return cached
    ref = task_writer.create_task(
        agent="roostery.bot_relay",
        cwd=bot.default_cwd,
        summary=f"M3.C 接力链 · {_short_chat(chat_id)}",
        description=f"IM chat_id={chat_id}（roostery.bot_relay_task 自动建）",
        idempotency_key=f"m3c-relay-task:{chat_id}",
        profile=writer_profile,
    )
    _save_cached(chat_id, ref)
    return ref


def record_start(
    *,
    bot: BotRole,
    event: Dict[str, Any],
    message_brief: str,
) -> Optional[TaskRef]:
    """Runner 跑之前调一次：建 task（若无 cache）+ append 起始 step。

    返回 TaskRef 供调用方拼 reply URL；chat_id 缺失返回 None。
    """
    chat_id = event.get("chat_id") or ""
    if not chat_id:
        return None
    writer_profile = bot.relay_writer_app_id or None
    ref = _ensure_task(chat_id, bot, writer_profile)
    step = _format_start_step(bot, event, message_brief)
    idem = f"m3c-step-start:{event.get('message_id', '')}:{bot.app_id}"
    task_writer.append_steps(
        ref.guid, [step], idempotency_key=idem, profile=writer_profile,
    )
    return ref


def record_end(
    *,
    bot: BotRole,
    action: "BotAction",
    result_text: str,
) -> Optional[TaskRef]:
    """Runner 跑之后调一次：append 完成 / 超时 / 失败 step。"""
    chat_id = action.chat_id or ""
    if not chat_id:
        return None
    writer_profile = bot.relay_writer_app_id or None
    ref = _ensure_task(chat_id, bot, writer_profile)
    step = _format_end_step(bot, action, result_text)
    idem = f"m3c-step-end:{action.source_message_id}:{bot.app_id}"
    task_writer.append_steps(
        ref.guid, [step], idempotency_key=idem, profile=writer_profile,
    )
    return ref


def record_adjust(
    *,
    bot: BotRole,
    task_ref: TaskRef,
    adjust_text: str,
    attempt: int,
) -> None:
    """user `/adjust <body>` 触发后，杀 runner 重启前 append 一条 step。"""
    writer_profile = bot.relay_writer_app_id or None
    brief = (adjust_text or "")[:80]
    step = f"🔄 [{bot.role}] 用户调整请求 #{attempt} (via /adjust: {brief})"
    idem = f"m3g-step-adjust:{task_ref.guid}:{attempt}"
    task_writer.append_steps(
        task_ref.guid, [step], idempotency_key=idem, profile=writer_profile,
    )
