"""IM 消息 → base record 触发路由。

设计：docs/superpowers/specs/2026-05-15-m4c-base-intent-router-design.md §1, §2
"""
from __future__ import annotations

import json
import logging
import re
from datetime import datetime, timezone
from typing import Callable, List, Optional, Tuple

from roostery.base_config import BaseConfig, resolve_by_role, resolve_by_base_token
from roostery.lark_cli import base_record_get, base_record_upsert
from roostery import nl_router
from roostery.record_writer import (
    append_product,
    cas_acquire_running,
    mirror_doc_urls,
    set_run_state,
)
from roostery.runner_registry import RunnerEntry, RunnerRegistry

_log = logging.getLogger(__name__)

_URL_RE = re.compile(
    r"https?://[\w.-]+/base/(\w+)\?[^\s]*?table=(tbl\w+)[^\s]*?record=(rec\w+)"
)
_SHORT_RE = re.compile(r"(\S[^\s]*?)\s+record:(rec\w+)")
_RUN_RE = re.compile(r"/run\s+(.+?)\s*$", re.MULTILINE | re.DOTALL)


def _parse_base_ref(text: str, configs: List[BaseConfig]) -> Optional[Tuple[str, str, str]]:
    """Returns (base_token, table_id, record_id) or None."""
    m = _URL_RE.search(text)
    if m:
        return m.group(1), m.group(2), m.group(3)
    m = _SHORT_RE.search(text.strip())
    if m:
        role, record_id = m.group(1).strip(), m.group(2)
        cfg = resolve_by_role(configs, role)
        if cfg:
            return cfg.base_token, cfg.table_id, record_id
    return None


def _resolve_bot(record: dict, cfg: BaseConfig) -> Optional[str]:
    """优先级：负责 AI > stage_to_bot[阶段]。两个字段都是 select 字段（飞书返回 list）。"""
    ai_list = record.get("负责 AI") or []
    if isinstance(ai_list, list) and ai_list:
        return ai_list[0]
    if isinstance(ai_list, str) and ai_list:
        return ai_list  # 兜底：万一是 plain string
    stage_list = record.get("阶段") or []
    stage = (stage_list[0] if isinstance(stage_list, list) and stage_list
             else stage_list if isinstance(stage_list, str) else None)
    if not stage:
        return None
    return cfg.stage_to_bot.get(stage)


def _event_message(event: dict) -> dict:
    """Return the inner message dict regardless of envelope shape.

    ``event_bridge.consume_im`` yields flat events
    (``{"message_id": ..., "chat_id": ..., "content": ...}``); some upstream /
    test fixtures pass the raw envelope (``{"event": {"message": {...}}}``).
    Accept both.
    """
    if isinstance(event, dict) and isinstance(event.get("event"), dict):
        msg = event["event"].get("message")
        if isinstance(msg, dict):
            return msg
    return event if isinstance(event, dict) else {}


def _extract_text(event: dict) -> Optional[str]:
    try:
        msg = _event_message(event)
        content = msg.get("content", "")
        if isinstance(content, str):
            # flat events 的 content 可能是 plain text 或 JSON-encoded {"text":"..."}
            stripped = content.strip()
            if stripped.startswith("{"):
                try:
                    content = json.loads(content)
                except json.JSONDecodeError:
                    return content
            else:
                return content
        if isinstance(content, dict):
            return content.get("text", "")
        return ""
    except (KeyError, TypeError):
        return None


def _dispatch_runner(bot_name: str, prompt: str,
                     on_pid: Callable[[int], None]) -> object:
    """Resolve ``bot_name`` → :class:`BotRole` → :class:`RunSpec` →
    :func:`dispatcher.runners.run`。

    bot_name 优先按 ``app_id`` 匹配，再退到 ``role``（base.yaml 里两种 form 都见过）。
    base 路径不创建飞书 task —— 产物直接落 base 行（由 record_writer 处理）。

    Returns:
        :class:`dispatcher.runners.RunResult`
    Raises:
        ValueError: 当 bots.yaml 找不到该 bot_name。
    """
    from roostery import bot_role, config
    from roostery.dispatcher import runners as _runners

    bots_path = config.root_dir() / "bots.yaml"
    bots = bot_role.load_bots(bots_path)
    bot = next(
        (b for b in bots if b.app_id == bot_name or b.role == bot_name),
        None,
    )
    if bot is None:
        raise ValueError(f"bot {bot_name!r} not found in bots.yaml")
    spec = _runners.RunSpec(
        runner=bot.runner, prompt=prompt, cwd=bot.default_cwd,
    )
    return _runners.run(spec, on_pid=on_pid)


def _build_prompt(bot: str, record: dict, record_id: str) -> str:
    return (
        f"你是 {bot} bot。当前 base 行 record_id={record_id}。\n"
        f"全字段：\n{json.dumps(record, ensure_ascii=False, indent=2)}\n\n"
        f"完成当前阶段工作；产物会被自动 append 到「产物」字段。"
    )


def try_handle(event: dict, *, configs: List[BaseConfig],
               registry: RunnerRegistry,
               reply_fn: Callable[[str], None]) -> bool:
    """If IM message matches /run <base_ref>, consume and trigger; return True.

    Return False to let caller route the event to legacy R5 IM path.
    """
    text = _extract_text(event)
    if not text:
        return False
    m = _RUN_RE.search(text)
    if not m:
        return False
    base_ref = m.group(1).strip()

    parsed = _parse_base_ref(base_ref, configs)
    if not parsed:
        reply_fn("base_ref 无法解析。支持：① base 行链接 ② `{role} record:recXXX`")
        return True
    base_token, table_id, record_id = parsed

    cfg = resolve_by_base_token(configs, base_token)
    if not cfg:
        reply_fn(f"base_token {base_token} 未注册")
        return True

    if registry.lookup_by_record_id(record_id):
        reply_fn("该 record 已有 runner 在跑")
        return True

    rec = base_record_get(base_token=base_token, table_id=table_id, record_id=record_id)
    bot = _resolve_bot(rec, cfg)
    if not bot:
        stage_list = rec.get("阶段") or ["(空)"]
        stage = stage_list[0] if isinstance(stage_list, list) and stage_list else stage_list
        reply_fn(f"阶段「{stage}」未绑 bot")
        return True

    marker, status = cas_acquire_running(
        record_id=record_id, base_token=base_token, table_id=table_id,
    )
    if status == "non_idle":
        reply_fn("该行非 idle，手动改回 idle 再 /run")
        return True
    if status == "concurrent_conflict":
        reply_fn("并发冲突，本次放弃")
        return True

    msg = _event_message(event)
    chat_id = msg.get("chat_id", "")
    msg_id = msg.get("message_id", "")
    # 不早 register（pid=0 时 hitl_router 拿到会 os.kill(0,SIGTERM) 杀进程组）。
    # 跟 R5 bot_runner.handle_event 一致：仅在 _on_pid 里 register 真 pid。
    # 副作用：cas 期间（~5s）+ Popen 启动前的 /stop 不会命中——可接受，
    # runner 启动后用户重发即可（hitl_router 加了 pid<=0 防御兜底）。
    entry_template = dict(
        task_guid=f"base-{record_id}",
        task_url=f"https://feishu.cn/base/{base_token}?table={table_id}&record={record_id}",
        bot_app_id="cli_local", chat_id=chat_id,
        source_message_id=msg_id,
        started_at=datetime.now(timezone.utc).isoformat(timespec="seconds"),
        record_id=record_id, base_token=base_token, table_id=table_id,
    )
    entry = RunnerEntry(runner_pid=0, **entry_template)  # 占位（cleanup 用其 task_guid）

    def _on_pid(pid: int) -> None:
        registry.register(RunnerEntry(runner_pid=pid, **entry_template))

    result = None
    try:
        append_product(record_id=record_id, text=f"--- {bot} 启动 ---",
                       base_token=base_token, table_id=table_id)
        result = _dispatch_runner(bot, _build_prompt(bot, rec, record_id), _on_pid)
    except Exception:
        _log.exception("dispatch failed: record=%s bot=%s", record_id, bot)
        reply_fn(f"runner 启动/执行异常，已回滚 (record={record_id})")
    finally:
        _cleanup_after_runner(entry=entry, bot=bot, result=result,
                              registry=registry, reply_fn=reply_fn, cfg=cfg)
    return True


def _cleanup_after_runner(
    *, entry: RunnerEntry, bot: str, result: object,
    registry: RunnerRegistry, reply_fn: Callable[[str], None],
    cfg: Optional[BaseConfig] = None,
) -> None:
    """Write final state + product tail + unregister; never raises.

    ``result`` is :class:`roostery.dispatcher.runners.RunResult`，
    或 ``None``（dispatch 阶段抛异常的情况）。
    """
    record_id = entry.record_id or ""
    base_token = entry.base_token or ""
    table_id = entry.table_id or ""

    if result is None:
        state = "failed"
        tail = f"--- {bot} 启动失败（dispatch 异常）---"
    elif getattr(result, "aborted", False):
        state = "aborted"
        reason = getattr(result, "abort_reason", "") or ""
        tail = f"--- {bot} aborted ({reason}) ---"
    elif getattr(result, "exit_code", 0) == 0:
        state = "done"
        head = getattr(result, "stdout_head", "") or getattr(result, "stdout", "")
        tail = f"--- {bot} 完成 ---\n{head}"
    else:
        state = "failed"
        ec = getattr(result, "exit_code", -1)
        head = getattr(result, "stderr_head", "") or getattr(result, "stderr", "")
        tail = f"--- {bot} failed (exit {ec}) ---\n{head}"

    try:
        set_run_state(record_id=record_id, state=state,
                      base_token=base_token, table_id=table_id)
    except Exception:
        _log.exception("cleanup set_run_state failed: record=%s", record_id)

    if (state == "done" and cfg is not None
            and result is not None and not getattr(result, "aborted", False)
            and getattr(result, "exit_code", 0) == 0):
        target = cfg.output_mirror.get(bot)
        if target:
            try:
                stdout = getattr(result, "stdout", "") or ""
                n = mirror_doc_urls(
                    record_id=record_id, target_field=target, stdout=stdout,
                    base_token=base_token, table_id=table_id,
                )
                if n:
                    _log.info("mirror_doc_urls: record=%s field=%s n=%d",
                              record_id, target, n)
            except Exception:
                _log.exception("mirror_doc_urls failed: record=%s", record_id)

    try:
        append_product(record_id=record_id, text=tail,
                       base_token=base_token, table_id=table_id)
    except Exception:
        _log.exception("cleanup append_product failed: record=%s", record_id)

    try:
        registry.unregister(entry.task_guid)
    except Exception:
        _log.exception("cleanup unregister failed: task=%s", entry.task_guid)


def try_handle_nl(event: dict, *, configs: List[BaseConfig],
                  registry: RunnerRegistry,
                  reply_fn: Callable[[str], None]) -> bool:
    """NL → 自动建 Base 行 → 组装 fake event → 调 try_handle 复用 /run 路径。

    Return:
    - True 表示消费了 event（成功路由 / 已追问 / 已报错）
    - False 表示没识别出任何 role，应让上层继续 fall-through 到其他 handler
    """
    text = _extract_text(event)
    if not text:
        return False

    result, tried_and_failed = nl_router.parse(text, configs)
    if result is None:
        if tried_and_failed:
            reply_fn(
                "没看懂这个任务。你可以直接说『公众号写一篇 AI 产品设计』，"
                "或者用 /run 命令。"
            )
            return True  # consumed
        return False  # silent fall-through

    if result.confidence < 0.7:
        reply_fn(
            f"我理解你想让『{result.role}』做『{result.title}』，对吗？"
            "回复『是』确认，或告诉我具体要做什么。"
        )
        return True

    cfg = resolve_by_role(configs, result.role)
    if cfg is None:  # paranoia: parse 给出的 role 必在 configs 里
        return False

    try:
        record_id = base_record_upsert(
            base_token=cfg.base_token,
            table_id=cfg.table_id,
            fields={
                "任务标题": result.title,
                "阶段": result.initial_stage,
                "运行状态": "idle",
                "备注": result.raw_text,
            },
        )
    except Exception as e:  # lark-cli 异常 / 字段不存在 / 鉴权失败
        _log.exception("nl_router upsert failed: role=%s title=%r",
                       result.role, result.title)
        reply_fn(f"建行失败：{type(e).__name__}: {e}")
        return True

    # 组装 fake event：复用原 chat_id/sender 让 reply_fn 仍贴在原 thread
    msg = _event_message(event)
    fake_event = {
        "message_id": msg.get("message_id", ""),
        "chat_id": msg.get("chat_id", ""),
        "content": f"/run {result.role} record:{record_id}",
    }
    # 把 sender_id / open_id 一并 propagate（如果原 event 有）
    for k in ("sender_id", "open_id"):
        if k in msg:
            fake_event[k] = msg[k]

    consumed = try_handle(fake_event, configs=configs, registry=registry, reply_fn=reply_fn)
    if not consumed:
        reply_fn(f"已建行（record_id={record_id}），但 /run 触发失败；请手动 /run {result.role} record:{record_id}")
    return True
