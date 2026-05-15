"""hitl_router — IM event → 是否要 abort 当前 chat 上的 runner（R5 POC 路线 B）。

POC 假设：1 chat = 1 active runner。收到 IM event，按 chat_id 查 registry。

过滤顺序：
1. event.content 以 ABORT_KEYWORDS 任一前缀开头（strip 后）
2. registry.lookup_by_chat_id(chat_id) 命中
命中后副作用：write_abort_sentinel(task_guid, reason) + os.kill(pid, SIGTERM)。

注：POC 不做"防 bot 自评论"过滤——bot 的 im.message.receive_v1 stream 通常
不含自己发的 reply（Feishu side filters）。如果将来 cross-bot 路径下 bot A
跑活时 bot B 在同 chat reply 误触发，再加 sender_id 过滤。
"""
from __future__ import annotations

import logging
import os
import signal
import threading
from dataclasses import dataclass
from typing import Any, Dict, Optional

from .runner_registry import RunnerRegistry


_log = logging.getLogger(__name__)

ABORT_GRACE_S = 10.0  # SIGTERM 之后等多久仍未退 → SIGKILL 兜底


def _schedule_sigkill(pid: int, grace_s: float = ABORT_GRACE_S) -> threading.Timer:
    """SIGTERM 之后 grace_s 兜底 SIGKILL。pid 已死时 silently 吞 ProcessLookupError。"""
    def _kill9() -> None:
        try:
            os.kill(pid, signal.SIGKILL)
            _log.warning("hitl_router: SIGKILL fallback fired for pid=%s", pid)
        except ProcessLookupError:
            pass  # 已优雅退，理想路径
        except PermissionError:
            _log.error("hitl_router: SIGKILL fallback PermissionError pid=%s", pid)
    t = threading.Timer(grace_s, _kill9)
    t.daemon = True
    t.start()
    return t


ABORT_KEYWORDS = ("/stop", "/abort", "停", "中止")

ADJUST_PREFIX = ("/adjust ", "/adjust\n", "调整 ", "调整\n")


@dataclass(frozen=True)
class AbortDecision:
    chat_id: str
    task_guid: str
    runner_pid: int
    reason: str


def _matched_keyword(text: str) -> Optional[str]:
    s = text.strip()
    for kw in ABORT_KEYWORDS:
        if s.startswith(kw):
            return kw
    return None


def _extract_adjust(text: str) -> Optional[str]:
    """text 以 ADJUST_PREFIX 任一开头 → 返回剥前缀后的 body；空 body 返回 None。"""
    s = text.strip()
    for p in ADJUST_PREFIX:
        if s.startswith(p):
            body = s[len(p):].strip()
            return body or None
    return None


def dispatch(
    envelope: Dict[str, Any],
    *,
    registry: RunnerRegistry,
) -> Optional[AbortDecision]:
    """返回 AbortDecision 表示命中并已发 SIGTERM；返回 None 表示忽略此 event。"""
    content = envelope.get("content") or ""
    if not isinstance(content, str):
        return None

    # adjust 优先（含 body，更具体）
    supplement = _extract_adjust(content)
    if supplement is not None:
        chat_id = envelope.get("chat_id") or ""
        if not chat_id:
            return None
        entry = registry.lookup_by_chat_id(chat_id)
        if entry is None:
            return None
        registry.write_adjust_sentinel(entry.task_guid, supplement)
        try:
            os.kill(entry.runner_pid, signal.SIGTERM)
        except ProcessLookupError:
            _log.warning("hitl_router: pid %s already gone for adjust chat=%s task=%s",
                         entry.runner_pid, chat_id, entry.task_guid)
        except PermissionError:
            _log.error("hitl_router: PermissionError kill pid=%s adjust chat=%s",
                       entry.runner_pid, chat_id)
            return None
        else:
            _schedule_sigkill(entry.runner_pid)
        return AbortDecision(
            chat_id=chat_id, task_guid=entry.task_guid,
            runner_pid=entry.runner_pid, reason=f"/adjust: {supplement}",
        )

    # abort 路径（既有）
    kw = _matched_keyword(content)
    if kw is None:
        return None
    chat_id = envelope.get("chat_id") or ""
    if not chat_id:
        return None
    entry = registry.lookup_by_chat_id(chat_id)
    if entry is None:
        return None
    if entry.runner_pid <= 0:
        # 防御：pid<=0 时 os.kill 会杀整个 process group（含 daemon）
        # 通常出现在 register 早于 on_pid 的窗口；忽略此次 abort，runner
        # 启动后用户重发即可
        _log.warning("hitl_router: skip abort (pid=%s not ready) chat=%s task=%s",
                     entry.runner_pid, chat_id, entry.task_guid)
        return None
    registry.write_abort_sentinel(entry.task_guid, kw)
    try:
        os.kill(entry.runner_pid, signal.SIGTERM)
    except ProcessLookupError:
        _log.warning("hitl_router: pid %s already gone for chat=%s task=%s",
                     entry.runner_pid, chat_id, entry.task_guid)
    except PermissionError:
        _log.error("hitl_router: PermissionError kill pid=%s chat=%s",
                   entry.runner_pid, chat_id)
        return None
    else:
        _schedule_sigkill(entry.runner_pid)
    return AbortDecision(
        chat_id=chat_id, task_guid=entry.task_guid,
        runner_pid=entry.runner_pid, reason=kw,
    )
