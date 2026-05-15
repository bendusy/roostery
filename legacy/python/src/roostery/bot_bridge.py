"""bot_bridge — 单 bot daemon orchestrator（M3.C → M3.E → R5 POC）。

把 :func:`event_bridge.consume_im` 的事件流喂给 :func:`bot_runner.handle_event`。
单条事件异常不影响后续。一个 daemon = 一个 lark-cli profile = 一个 bot。

**R5 POC（路线 B）**：

- 启动时 :meth:`runner_registry.RunnerRegistry.cleanup_orphans` 清孤儿
- 每条 IM event 先送给 :func:`hitl_router.dispatch`；命中 abort 就跳过 handle_event
- ``parallel=True`` 模式下 handle_event 在 daemon worker thread 跑——让主线程
  持续读 IM、HITL ``/stop`` 不被 runner 阻塞
"""
from __future__ import annotations

import logging
import queue
import threading
from typing import Iterator, Optional

from . import base_config, base_intent_router, hitl_router, runner_registry
from .bot_role import BotRole
from .bot_runner import BotAction, handle_event
from .event_bridge import consume_im


_log = logging.getLogger(__name__)
_DONE = object()

# Lazy-loaded base configs (per process). Cleared on demand by tests.
_BASE_CONFIGS_CACHE: Optional[list] = None


def _get_base_configs() -> list:
    global _BASE_CONFIGS_CACHE
    if _BASE_CONFIGS_CACHE is None:
        try:
            _BASE_CONFIGS_CACHE = base_config.load_all()
        except Exception:
            _log.exception("base_config.load_all failed; base trigger disabled")
            _BASE_CONFIGS_CACHE = []
    return _BASE_CONFIGS_CACHE


def _try_base_intent(event: dict, registry: runner_registry.RunnerRegistry) -> bool:
    """Route ``/run <base_ref>`` IM messages to base_intent_router.

    Returns True when the event was consumed (caller skips legacy handle_event).
    """
    configs = _get_base_configs()
    if not configs:
        return False
    msg_id = event.get("message_id", "")

    def _reply(text: str) -> None:
        from roostery import lark_cli
        try:
            lark_cli.im_messages_reply(message_id=msg_id, text=text)
        except Exception:
            _log.exception("base reply failed: msg=%s", msg_id)

    try:
        # 1) 先 try /run 显式协议
        if base_intent_router.try_handle(
            event, configs=configs, registry=registry, reply_fn=_reply,
        ):
            return True

        # 2) fall-through 到 NL parser
        return base_intent_router.try_handle_nl(
            event, configs=configs, registry=registry, reply_fn=_reply,
        )
    except Exception:
        _log.exception("base_intent_router.try_handle failed: msg=%s", msg_id)
        return False


def run_bot(
    bot: BotRole,
    *,
    max_events: int = 0,
    timeout: str = "",
    parallel: bool = False,
) -> Iterator[BotAction]:
    """长跑：消费 IM 事件流，路由 hitl_router → handle_event；yield 每次 BotAction。

    Args:
        bot: 当前 daemon 服务的角色（``app_id`` 同时是 lark-cli profile name）
        max_events: ``--max-events N``；0 = 不限制
        timeout: lark-cli ``--timeout`` 字面值；空 = 不限制
        parallel: True 时 handle_event 在 worker thread 跑（生产 daemon 用），
            False 时同步顺序（测试 / 简单用例用，保持向后兼容）
    """
    registry = runner_registry.RunnerRegistry()
    try:
        registry.cleanup_orphans()
    except Exception:
        _log.exception("runner_registry.cleanup_orphans failed")

    if not parallel:
        yield from _run_sync(bot, registry, max_events=max_events, timeout=timeout)
        return
    yield from _run_parallel(bot, registry, max_events=max_events, timeout=timeout)


def _run_sync(
    bot: BotRole,
    registry: runner_registry.RunnerRegistry,
    *,
    max_events: int,
    timeout: str,
) -> Iterator[BotAction]:
    for event in consume_im(profile=bot.app_id, max_events=max_events, timeout=timeout):
        if _is_abort(event, registry):
            continue
        if _try_base_intent(event, registry):
            continue
        try:
            action = handle_event(event, bot)
        except Exception:
            _log.exception("handle_event failed: bot=%s msg=%s",
                           bot.app_id, event.get("message_id"))
            continue
        if action is None:
            continue
        yield action


def _run_parallel(
    bot: BotRole,
    registry: runner_registry.RunnerRegistry,
    *,
    max_events: int,
    timeout: str,
) -> Iterator[BotAction]:
    actions_q: "queue.Queue" = queue.Queue()
    workers: list[threading.Thread] = []

    def _worker(event: dict) -> None:
        # M4.D fix: base path 也 offload 到 worker，避免 _try_base_intent 同步
        # 阻塞 feeder（runner 跑 N 秒期间 /stop 等后续 event 收不到）。
        try:
            if _try_base_intent(event, registry):
                actions_q.put(None)
                return
        except Exception:
            _log.exception("base_intent worker failed: bot=%s msg=%s",
                           bot.app_id, event.get("message_id"))
            # 失败 fall through 到 R5 path
        try:
            a = handle_event(event, bot)
        except Exception:
            _log.exception("handle_event failed: bot=%s msg=%s",
                           bot.app_id, event.get("message_id"))
            a = None
        actions_q.put(a)

    def _feeder() -> None:
        try:
            for event in consume_im(profile=bot.app_id,
                                    max_events=max_events, timeout=timeout):
                if _is_abort(event, registry):
                    actions_q.put(None)  # 占位防 main loop 提前 break
                    continue
                t = threading.Thread(target=_worker, args=(event,), daemon=True)
                t.start()
                workers.append(t)
        finally:
            for w in list(workers):
                w.join()
            actions_q.put(_DONE)

    t_feed = threading.Thread(target=_feeder, daemon=True)
    t_feed.start()

    while True:
        a = actions_q.get()
        if a is _DONE:
            break
        if a is None:
            continue
        yield a


def _is_abort(event: dict, registry: runner_registry.RunnerRegistry) -> bool:
    """call hitl_router.dispatch；命中返回 True。异常不阻塞主路径。"""
    try:
        decision = hitl_router.dispatch(event, registry=registry)
    except Exception:
        _log.exception("hitl_router.dispatch failed: msg=%s",
                       event.get("message_id"))
        return False
    return decision is not None
