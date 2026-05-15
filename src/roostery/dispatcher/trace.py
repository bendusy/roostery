"""trace_id / depth / parent_event_id 传播。

dispatcher 派子进程时把 trace 信息塞进 env；受派 agent 的 hook 把这些字段
回填到 journal envelope 的 ``actor.trace_id`` / ``actor.depth`` /
``actor.parent_event_id``。dispatcher 在 enqueue 前据此防死循环。

详见 ``docs/FEISHU_HUB_DISPATCHER_DESIGN.md`` §7。
"""
from __future__ import annotations

import os
import secrets
import time
from dataclasses import dataclass
from typing import Any, Dict, Mapping, Optional

ENV_TRACE_ID = "FEISHU_HUB_TRACE_ID"
ENV_DEPTH = "FEISHU_HUB_DEPTH"
ENV_PARENT = "FEISHU_HUB_PARENT_EVENT_ID"

DEFAULT_MAX_DEPTH = 3
DEFAULT_RUNAWAY_WINDOW_S = 300       # 5min
DEFAULT_RUNAWAY_THRESHOLD = 10


@dataclass(frozen=True)
class TraceCtx:
    """一次 dispatch 链路上的 trace 上下文。"""
    trace_id: str
    depth: int
    parent_event_id: Optional[str]

    def to_env(self) -> Dict[str, str]:
        env = {
            ENV_TRACE_ID: self.trace_id,
            ENV_DEPTH: str(self.depth),
        }
        if self.parent_event_id:
            env[ENV_PARENT] = self.parent_event_id
        return env

    def to_actor_fields(self) -> Dict[str, Any]:
        out: Dict[str, Any] = {"trace_id": self.trace_id, "depth": self.depth}
        if self.parent_event_id:
            out["parent_event_id"] = self.parent_event_id
        return out


def new_trace_id() -> str:
    """16 字节 hex（与 ULID 区分；trace_id 不需要排序，纯随机即可）。"""
    return secrets.token_hex(16)


def from_env(env: Optional[Mapping[str, str]] = None) -> Optional[TraceCtx]:
    """从环境变量构造 ``TraceCtx``；缺 trace_id 返回 ``None``。"""
    e = env if env is not None else os.environ
    tid = e.get(ENV_TRACE_ID)
    if not tid:
        return None
    try:
        depth = int(e.get(ENV_DEPTH, "0"))
    except ValueError:
        depth = 0
    parent = e.get(ENV_PARENT) or None
    return TraceCtx(trace_id=tid, depth=depth, parent_event_id=parent)


def from_event(event: Mapping[str, Any]) -> Optional[TraceCtx]:
    """从一条 journal envelope 抽 trace 上下文；没有则 ``None``。"""
    actor = event.get("actor") or {}
    tid = actor.get("trace_id")
    if not tid:
        return None
    depth_raw = actor.get("depth", 0)
    try:
        depth = int(depth_raw)
    except (TypeError, ValueError):
        depth = 0
    return TraceCtx(
        trace_id=tid,
        depth=depth,
        parent_event_id=actor.get("parent_event_id"),
    )


def child(ctx: Optional[TraceCtx], *, parent_event_id: Optional[str]) -> TraceCtx:
    """生成下一层 trace 上下文。

    入参 ``ctx`` 为 ``None`` 表示这是链路起点：自动 ``new_trace_id()``。
    ``parent_event_id`` 必填（dispatcher 来源 event 的 ULID）。
    """
    if ctx is None:
        return TraceCtx(trace_id=new_trace_id(), depth=1,
                        parent_event_id=parent_event_id)
    return TraceCtx(trace_id=ctx.trace_id, depth=ctx.depth + 1,
                    parent_event_id=parent_event_id)


# ---- 死循环防御 ---------------------------------------------------------

class DepthExceeded(RuntimeError):
    """深度超限。"""

    def __init__(self, ctx: TraceCtx, max_depth: int):
        super().__init__(f"depth {ctx.depth} ≥ max_depth {max_depth} for trace {ctx.trace_id}")
        self.ctx = ctx
        self.max_depth = max_depth


class RunawayDetected(RuntimeError):
    """同 trace 在窗口内事件失控。"""

    def __init__(self, trace_id: str, count: int, window_s: int):
        super().__init__(f"trace {trace_id} fired {count} dispatches in {window_s}s")
        self.trace_id = trace_id
        self.count = count
        self.window_s = window_s


def check_depth(ctx: Optional[TraceCtx], *, max_depth: int = DEFAULT_MAX_DEPTH) -> None:
    if ctx is None:
        return
    if ctx.depth >= max_depth:
        raise DepthExceeded(ctx, max_depth)


class RunawayTracker:
    """内存版滑动窗口；dispatcher 单实例进程内用够了。

    多进程持久化版本如果将来要做，可挂 sqlite / leveldb。
    """

    def __init__(self,
                 window_s: int = DEFAULT_RUNAWAY_WINDOW_S,
                 threshold: int = DEFAULT_RUNAWAY_THRESHOLD,
                 clock: Optional[Any] = None):
        self._window_s = window_s
        self._threshold = threshold
        self._clock = clock or time.monotonic
        self._fires: Dict[str, list] = {}

    def record(self, trace_id: str) -> int:
        """登记一次 dispatch；返回当前窗口内计数。"""
        now = self._clock()
        bucket = self._fires.setdefault(trace_id, [])
        cutoff = now - self._window_s
        # 清理旧；窗口端点取闭区间（ts >= cutoff 保留）
        idx = 0
        for ts in bucket:
            if ts >= cutoff:
                break
            idx += 1
        if idx:
            del bucket[:idx]
        bucket.append(now)
        return len(bucket)

    def check(self, trace_id: str) -> None:
        """超阈值抛 ``RunawayDetected``。"""
        bucket = self._fires.get(trace_id)
        if bucket is None:
            return
        if len(bucket) >= self._threshold:
            raise RunawayDetected(trace_id, len(bucket), self._window_s)
