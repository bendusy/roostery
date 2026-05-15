import dataclasses
import os
import signal

import pytest

from roostery.hitl_router import dispatch, AbortDecision, ABORT_KEYWORDS
from roostery.runner_registry import RunnerEntry, RunnerRegistry


@pytest.fixture
def registry(tmp_path, monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
    return RunnerRegistry()


def _envelope(content="/stop", chat_id="oc_x", sender="ou_user"):
    return {
        "content": content,
        "chat_id": chat_id,
        "message_id": "om_x",
        "sender_id": sender,
    }


def _entry(*, chat_id, pid):
    return RunnerEntry(
        task_guid=f"t-{chat_id}", task_url="u", runner_pid=pid,
        bot_app_id="cli_x", chat_id=chat_id, source_message_id="om_x",
        started_at="2026-05-13T22:30:00+08:00",
    )


def test_dispatch_keyword_hit_kills_runner(registry, monkeypatch):
    killed = []
    monkeypatch.setattr(os, "kill", lambda pid, sig: killed.append((pid, sig)))
    registry.register(_entry(chat_id="oc_x", pid=11111))
    decision = dispatch(_envelope(content="/stop kill"), registry=registry)
    assert isinstance(decision, AbortDecision)
    assert decision.chat_id == "oc_x"
    assert decision.runner_pid == 11111
    assert decision.reason == "/stop"
    assert killed == [(11111, signal.SIGTERM)]
    # sentinel 已写
    assert registry.read_abort_sentinel("t-oc_x") == "/stop"


def test_dispatch_returns_none_when_no_keyword(registry, monkeypatch):
    monkeypatch.setattr(os, "kill", lambda pid, sig: None)
    registry.register(_entry(chat_id="oc_x", pid=11111))
    assert dispatch(_envelope(content="just chatting"), registry=registry) is None
    assert registry.read_abort_sentinel("t-oc_x") is None


def test_dispatch_returns_none_when_chat_has_no_runner(registry, monkeypatch):
    monkeypatch.setattr(os, "kill", lambda pid, sig: None)
    # 没 register
    assert dispatch(_envelope(content="/stop"), registry=registry) is None


def test_dispatch_returns_none_on_empty_chat_id(registry, monkeypatch):
    monkeypatch.setattr(os, "kill", lambda pid, sig: None)
    registry.register(_entry(chat_id="oc_x", pid=11111))
    assert dispatch(_envelope(content="/stop", chat_id=""),
                    registry=registry) is None


def test_dispatch_handles_dead_pid_gracefully(registry, monkeypatch):
    def fake_kill(pid, sig):
        raise ProcessLookupError(f"pid {pid} gone")
    monkeypatch.setattr(os, "kill", fake_kill)
    registry.register(_entry(chat_id="oc_x", pid=999999))
    # 即使 kill 抛 ProcessLookupError，sentinel 仍写、decision 仍返回（POC 容错）
    decision = dispatch(_envelope(content="/stop"), registry=registry)
    assert decision is not None
    assert registry.read_abort_sentinel("t-oc_x") == "/stop"


@pytest.mark.parametrize("kw", ABORT_KEYWORDS)
def test_dispatch_recognizes_all_keywords(registry, monkeypatch, kw):
    monkeypatch.setattr(os, "kill", lambda pid, sig: None)
    registry.register(_entry(chat_id="oc_x", pid=11111))
    decision = dispatch(_envelope(content=f"{kw} 求你了"), registry=registry)
    assert decision is not None and decision.reason == kw


def test_dispatch_ignores_keyword_in_middle_of_message(registry, monkeypatch):
    """user 说"快/stop 啊"不算干预——必须以关键词起头。"""
    monkeypatch.setattr(os, "kill", lambda pid, sig: None)
    registry.register(_entry(chat_id="oc_x", pid=11111))
    assert dispatch(_envelope(content="快 /stop 啊"),
                    registry=registry) is None


def test_schedule_sigkill_fires_after_grace_when_pid_alive(monkeypatch):
    """grace 内子进程没退 → SIGKILL 真被发出。"""
    import subprocess
    import sys
    import time
    from roostery.hitl_router import _schedule_sigkill

    # 起一个 SIGTERM-immune 子进程：trap SIGTERM 不做事，sleep 60
    code = (
        "import signal, time; "
        "signal.signal(signal.SIGTERM, lambda *_: None); "
        "time.sleep(60)"
    )
    proc = subprocess.Popen([sys.executable, "-c", code])
    try:
        # 给它 50ms 起来
        time.sleep(0.05)
        assert proc.poll() is None, "child should still be alive"
        # 发 SIGTERM（被 trap 吸收）+ schedule SIGKILL with tiny grace
        os.kill(proc.pid, signal.SIGTERM)
        timer = _schedule_sigkill(proc.pid, grace_s=0.2)
        timer.join(timeout=1.0)
        # SIGKILL 应已发出；子进程 0.5s 内死
        for _ in range(50):
            if proc.poll() is not None:
                break
            time.sleep(0.01)
        assert proc.poll() is not None, "SIGKILL did not kill the child"
        assert proc.returncode == -9
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=2)


def test_schedule_sigkill_silently_swallows_when_pid_already_dead(monkeypatch):
    """grace 时 pid 已死 → ProcessLookupError 被吞，不抛。"""
    import time
    from roostery.hitl_router import _schedule_sigkill

    # 用一个保证不存在的 pid（极大值）
    timer = _schedule_sigkill(99999999, grace_s=0.05)
    timer.join(timeout=1.0)
    # 不应抛——能跑到这里就过


def test_dispatch_schedules_sigkill_after_sigterm(monkeypatch, tmp_path):
    """命中 abort 后既写 sentinel 又 schedule SIGKILL。"""
    import threading
    from roostery import hitl_router as hr
    from roostery.runner_registry import RunnerEntry, RunnerRegistry

    monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))

    sigs = []
    monkeypatch.setattr(os, "kill", lambda pid, sig: sigs.append((pid, sig)))

    scheduled = []
    def fake_schedule(pid, grace_s=hr.ABORT_GRACE_S):
        scheduled.append((pid, grace_s))
        t = threading.Timer(60.0, lambda: None)
        t.daemon = True
        return t
    monkeypatch.setattr(hr, "_schedule_sigkill", fake_schedule)

    reg = RunnerRegistry()
    reg.register(RunnerEntry(
        task_guid="t1", task_url="u", runner_pid=42424,
        bot_app_id="cli_x", chat_id="oc_y",
        source_message_id="om_x", started_at="2026-05-13T00:00:00+08:00",
    ))

    env = {"content": "/stop", "chat_id": "oc_y", "message_id": "om_z",
           "sender_id": "ou_user"}
    decision = hr.dispatch(env, registry=reg)
    assert decision is not None
    assert (42424, signal.SIGTERM) in sigs
    assert scheduled == [(42424, hr.ABORT_GRACE_S)]


def test_extract_adjust_recognizes_prefix():
    from roostery.hitl_router import _extract_adjust
    assert _extract_adjust("/adjust 加点细节") == "加点细节"
    assert _extract_adjust("调整 跑短点") == "跑短点"
    assert _extract_adjust("/adjust\n多行调整") == "多行调整"


def test_extract_adjust_returns_none_for_no_match():
    from roostery.hitl_router import _extract_adjust
    assert _extract_adjust("just chatting") is None
    assert _extract_adjust("/stop") is None
    assert _extract_adjust("/adjust") is None  # 缺 body / 缺分隔符


def test_extract_adjust_returns_none_for_empty_body():
    from roostery.hitl_router import _extract_adjust
    assert _extract_adjust("/adjust   ") is None
    assert _extract_adjust("/adjust \n") is None


def test_dispatch_adjust_path_writes_adjust_sentinel(registry, monkeypatch):
    """命中 /adjust：写 .adjust sentinel（不是 .abort）+ SIGTERM + reason=/adjust: ..."""
    import os, signal, threading
    monkeypatch.setattr(os, "kill", lambda pid, sig: None)
    monkeypatch.setattr(
        "roostery.hitl_router._schedule_sigkill",
        lambda pid, grace_s=10.0: threading.Timer(60.0, lambda: None),
    )
    registry.register(_entry(chat_id="oc_x", pid=11111))
    decision = dispatch(_envelope(content="/adjust 跑短点"),
                       registry=registry)
    assert decision is not None
    assert decision.reason == "/adjust: 跑短点"
    assert registry.read_adjust_sentinel("t-oc_x") == "跑短点"
    assert registry.read_abort_sentinel("t-oc_x") is None  # 不写 abort


def test_dispatch_abort_path_unchanged(registry, monkeypatch):
    """命中 /stop：仍写 .abort sentinel，不写 .adjust。"""
    import os, signal, threading
    monkeypatch.setattr(os, "kill", lambda pid, sig: None)
    monkeypatch.setattr(
        "roostery.hitl_router._schedule_sigkill",
        lambda pid, grace_s=10.0: threading.Timer(60.0, lambda: None),
    )
    registry.register(_entry(chat_id="oc_x", pid=11111))
    decision = dispatch(_envelope(content="/stop"), registry=registry)
    assert decision is not None
    assert decision.reason == "/stop"
    assert registry.read_abort_sentinel("t-oc_x") == "/stop"
    assert registry.read_adjust_sentinel("t-oc_x") is None


def test_dispatch_skips_when_pid_zero(registry, monkeypatch):
    """防御性回归：pid<=0 时不能触发 os.kill(0,...)，否则杀整个进程组（含 daemon）。

    M4.E e2e 暴露：try_handle 早 register pid=0 → /stop 在 _on_pid 之前到 →
    hitl_router 拿 pid=0 → os.kill(0, SIGTERM) 杀整个 process group → daemon 死。
    现已两层防御：(1) try_handle 不再早 register；(2) hitl_router pid<=0 直接 skip。
    """
    killed = []
    monkeypatch.setattr(os, "kill", lambda pid, sig: killed.append((pid, sig)))
    # entry with pid=0 (registered before on_pid fired)
    registry.register(_entry(chat_id="oc_x", pid=0))
    decision = dispatch(_envelope(content="/stop"), registry=registry)
    assert decision is None
    assert killed == []  # 没动 os.kill —— 没有 pid=0 group 灾难
    # 也没写 sentinel（runner 还没真起来，写 sentinel 没意义）
    assert registry.read_abort_sentinel("t-oc_x") is None


def test_dispatch_skips_when_pid_negative(registry, monkeypatch):
    """同样防御 -1 等任何 invalid pid。"""
    killed = []
    monkeypatch.setattr(os, "kill", lambda pid, sig: killed.append((pid, sig)))
    registry.register(_entry(chat_id="oc_x", pid=-1))
    assert dispatch(_envelope(content="/stop"), registry=registry) is None
    assert killed == []
