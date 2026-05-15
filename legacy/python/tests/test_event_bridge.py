"""roostery.event_bridge — lark-cli event consume 子进程编排。

契约（来自 M3.C T1 POC 实测）：

1. 启动 ``lark-cli event consume im.message.receive_v1 --as bot [--max-events N] [--timeout Ts]``
2. 父进程必须**先**阻塞 stderr 等 ``[event] ready event_key=...`` 这一行
3. 之后逐行读 stdout，每行一个 JSON envelope
4. stdin 必须给一个**永不 EOF**的源；DEVNULL 实测会让 lark-cli 立刻
   ``[event] stdin closed — shutting down``。本模块用 ``tail -f /dev/null``
   子进程的 stdout 做 keepalive
5. 退出时发 ``SIGTERM`` 等 graceful exit（``[event] exited - received ...``），**不**用 SIGKILL
   （否则 PreConsume 订阅会泄漏到下次启动）

本模块用 ``subprocess.Popen`` 而非真跑 lark-cli——测试全用 mock。
"""
from __future__ import annotations

import io
import json
import signal
from types import SimpleNamespace
from typing import List

import pytest

from roostery import event_bridge as eb


# ---------------------------------------------------------------------------
# helpers — mock Popen
# ---------------------------------------------------------------------------

class _FakePopen:
    """模拟 lark-cli event consume 的子进程。

    stderr 先吐若干行（含 ready marker），stdout 吐 N 条 NDJSON，之后两边都 EOF。
    """

    def __init__(self, *, stderr_lines: List[str], stdout_lines: List[str]):
        self.stderr = io.StringIO("".join(stderr_lines))
        self.stdout = io.StringIO("".join(stdout_lines))
        self.stdin = io.BytesIO()
        self.returncode = 0
        self.signals: List[int] = []
        self.waited = False

    def send_signal(self, sig: int) -> None:
        self.signals.append(sig)

    def terminate(self) -> None:
        self.signals.append(signal.SIGTERM)

    def wait(self, timeout: float = 0) -> int:
        self.waited = True
        return self.returncode

    def poll(self):
        return self.returncode if self.waited else None


@pytest.fixture
def fake_subprocess(monkeypatch):
    """劫持 ``subprocess.Popen`` 返回 :class:`_FakePopen`，记录调用 argv。

    ``lark_argv``: 仅记录 lark-cli 那一次的 argv（忽略 tail keepalive）。
    """
    calls: List[List[str]] = []
    handle = SimpleNamespace(proc=None, lark_argv=None)

    def factory(stderr_lines: List[str], stdout_lines: List[str]):
        def _popen(argv, **kw):
            calls.append(list(argv))
            if argv and argv[0] == "lark-cli":
                handle.lark_argv = list(argv)
                proc = _FakePopen(stderr_lines=stderr_lines, stdout_lines=stdout_lines)
                handle.proc = proc
                return proc
            return _FakePopen(stderr_lines=[], stdout_lines=[])
        monkeypatch.setattr(eb.subprocess, "Popen", _popen)

    return SimpleNamespace(factory=factory, calls=calls, handle=handle)


# ---------------------------------------------------------------------------
# argv 构造
# ---------------------------------------------------------------------------

def test_consume_argv_includes_event_key_and_as_bot(fake_subprocess):
    fake_subprocess.factory(
        stderr_lines=["[event] ready event_key=im.message.receive_v1\n"],
        stdout_lines=[],
    )
    list(eb.consume_im(max_events=0, timeout=""))
    assert fake_subprocess.calls, "Popen should have been called"
    argv = fake_subprocess.handle.lark_argv
    assert argv[:5] == ["lark-cli", "event", "consume", "im.message.receive_v1", "--as"]
    assert argv[5] == "bot"


def test_consume_argv_includes_max_events_when_set(fake_subprocess):
    fake_subprocess.factory(
        stderr_lines=["[event] ready event_key=im.message.receive_v1\n"],
        stdout_lines=[],
    )
    list(eb.consume_im(max_events=3))
    argv = fake_subprocess.handle.lark_argv
    assert "--max-events" in argv
    assert argv[argv.index("--max-events") + 1] == "3"


def test_consume_argv_includes_timeout_when_set(fake_subprocess):
    fake_subprocess.factory(
        stderr_lines=["[event] ready event_key=im.message.receive_v1\n"],
        stdout_lines=[],
    )
    list(eb.consume_im(timeout="30s"))
    argv = fake_subprocess.handle.lark_argv
    assert "--timeout" in argv
    assert argv[argv.index("--timeout") + 1] == "30s"


def test_consume_supports_profile_override(fake_subprocess):
    """多 bot 场景：用 --profile 指定哪个 lark-cli profile。"""
    fake_subprocess.factory(
        stderr_lines=["[event] ready event_key=im.message.receive_v1\n"],
        stdout_lines=[],
    )
    list(eb.consume_im(profile="cli_xxx"))
    argv = fake_subprocess.handle.lark_argv
    assert "--profile" in argv
    assert argv[argv.index("--profile") + 1] == "cli_xxx"


# ---------------------------------------------------------------------------
# ready marker / yielding
# ---------------------------------------------------------------------------

def test_consume_yields_parsed_events(fake_subprocess):
    e1 = {"message_id": "om_1", "chat_id": "oc_a", "content": "hi"}
    e2 = {"message_id": "om_2", "chat_id": "oc_a", "content": "yo"}
    fake_subprocess.factory(
        stderr_lines=[
            "[event] consuming as cli_xxx\n",
            "[event] ready event_key=im.message.receive_v1\n",
            "[source] feishu-websocket: connected\n",
        ],
        stdout_lines=[json.dumps(e1) + "\n", json.dumps(e2) + "\n"],
    )
    events = list(eb.consume_im())
    assert events == [e1, e2]


def test_consume_skips_malformed_json_lines(fake_subprocess):
    e1 = {"message_id": "om_1"}
    fake_subprocess.factory(
        stderr_lines=["[event] ready event_key=im.message.receive_v1\n"],
        stdout_lines=["not-json\n", json.dumps(e1) + "\n", "\n"],
    )
    events = list(eb.consume_im())
    assert events == [e1]


def test_consume_raises_when_ready_marker_missing(fake_subprocess):
    """stderr 里没出现 ready marker 就 EOF —— 视为启动失败。"""
    fake_subprocess.factory(
        stderr_lines=["[event] consuming as cli_xxx\n", "[error] bad token\n"],
        stdout_lines=[],
    )
    with pytest.raises(eb.EventConsumeError):
        list(eb.consume_im())


# ---------------------------------------------------------------------------
# 退出语义
# ---------------------------------------------------------------------------

def test_consume_sends_sigterm_on_generator_close(fake_subprocess):
    fake_subprocess.factory(
        stderr_lines=["[event] ready event_key=im.message.receive_v1\n"],
        stdout_lines=[json.dumps({"message_id": "om_1"}) + "\n"],
    )
    gen = eb.consume_im()
    next(gen)  # 拿一个事件
    gen.close()  # 模拟调用方提前退出
    proc = fake_subprocess.handle.proc
    assert signal.SIGTERM in proc.signals
    assert proc.waited is True


def test_consume_uses_non_eof_stdin(monkeypatch):
    """stdin 不能让 lark-cli 见到 EOF（实测：DEVNULL 会触发 graceful exit）。

    本测试验证调用方传给 lark-cli Popen 的 stdin 不是 DEVNULL、不是 parent stdin、
    也不是 PIPE+close —— 必须是另一个长进程（``tail -f /dev/null``）的 stdout。
    """
    captured: dict = {}
    popen_calls: List[List[str]] = []

    def spy(argv, **kw):
        popen_calls.append(list(argv))
        if argv[0] == "lark-cli":
            captured.update(kw)
            return _FakePopen(
                stderr_lines=["[event] ready event_key=im.message.receive_v1\n"],
                stdout_lines=[],
            )
        # tail keepalive 子进程
        return _FakePopen(stderr_lines=[], stdout_lines=[])

    monkeypatch.setattr(eb.subprocess, "Popen", spy)
    list(eb.consume_im())
    # 应该启动了 2 个 Popen：tail keepalive + lark-cli
    assert any(c[0] == "tail" for c in popen_calls), \
        f"expected a tail keepalive Popen, got: {popen_calls}"
    assert captured.get("stdin") not in (None, eb.subprocess.DEVNULL, eb.subprocess.PIPE)
