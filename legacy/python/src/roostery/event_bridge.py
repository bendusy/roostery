"""event_bridge — lark-cli event consume 子进程编排（M3.C T3）。

主入口 :func:`consume_im` 是一个生成器：yield 解析后的 im.message.receive_v1 envelope。

契约（实测，见 memory ``feishu-m3c-poc-facts``）：

1. 启动 ``lark-cli event consume im.message.receive_v1 --as bot``（可选 ``--profile``/``--max-events``/``--timeout``）
2. 阻塞读 stderr 直到 ``[event] ready event_key=`` 行
3. 之后逐行读 stdout，每行一个 JSON envelope（json.loads 失败的行跳过）
4. stdin 必须是不会 EOF 的源——本模块用 ``tail -f /dev/null`` 子进程的 stdout
   （DEVNULL 实测会让 lark-cli 立刻 graceful exit）
5. 生成器 close 时发 SIGTERM 并 wait（不是 SIGKILL——否则 PreConsume 订阅泄漏）
"""
from __future__ import annotations

import json
import os
import signal
import subprocess
from typing import Iterator, List, Optional


READY_MARKER = "[event] ready event_key="


class EventConsumeError(RuntimeError):
    """lark-cli event consume 启动失败或异常退出。"""


def _build_argv(
    *,
    profile: Optional[str],
    max_events: int,
    timeout: str,
) -> List[str]:
    argv = ["lark-cli"]
    if profile:
        argv += ["--profile", profile]
    argv += ["event", "consume", "im.message.receive_v1", "--as", "bot"]
    if max_events and max_events > 0:
        argv += ["--max-events", str(max_events)]
    if timeout:
        argv += ["--timeout", timeout]
    return argv


def _start_stdin_keepalive() -> subprocess.Popen:
    """启 ``tail -f /dev/null`` 作为下游 lark-cli 的 stdin 源。"""
    return subprocess.Popen(
        ["tail", "-f", "/dev/null"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )


def consume_im(
    *,
    profile: Optional[str] = None,
    max_events: int = 0,
    timeout: str = "",
) -> Iterator[dict]:
    """启动 lark-cli event consume，yield 每个 IM 事件 envelope。

    Args:
        profile: 指定 lark-cli profile name（多 bot 场景必填）；为 None 时用 active profile
        max_events: ``--max-events N``；0 表示不限制（长进程）
        timeout: ``--timeout 30s`` 字面值；空表示不限制
    """
    argv = _build_argv(profile=profile, max_events=max_events, timeout=timeout)
    keepalive = _start_stdin_keepalive()
    proc = subprocess.Popen(
        argv,
        stdin=keepalive.stdout,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        bufsize=1,
        text=True,
    )
    try:
        # 阻塞等 ready marker
        ready = False
        for line in proc.stderr:
            if READY_MARKER in line:
                ready = True
                break
        if not ready:
            raise EventConsumeError(
                f"lark-cli event consume exited before ready marker (argv={argv})"
            )

        for line in proc.stdout:
            line = line.strip()
            if not line:
                continue
            try:
                yield json.loads(line)
            except json.JSONDecodeError:
                continue
    finally:
        try:
            proc.send_signal(signal.SIGTERM)
            proc.wait(timeout=10)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            pass
        try:
            keepalive.terminate()
            keepalive.wait(timeout=5)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            pass
