"""roostery.bot_bridge — 单 bot daemon：consume_im → handle_event 循环。

M3.E：relay_task 已移到 handle_event 内部；本 bridge 测试不再涉及 relay_task。
"""
from __future__ import annotations

from typing import List
from unittest.mock import MagicMock

from roostery import bot_bridge as bb
from roostery import bot_role as br
from roostery.bot_runner import BotAction


def _bot(**over) -> br.BotRole:
    base = dict(
        app_id="cli_aaa",
        role="reviewer",
        mention_alias="审核Bot",
        runner="cc_headless",
        default_cwd="/tmp/x",
        prompt_template="x",
    )
    base.update(over)
    return br.BotRole(**base)


def _ok_action(**over) -> BotAction:
    base = dict(
        bot_app_id="cli_aaa",
        chat_id="oc_test",
        source_message_id="om_xxx",
        reply_message_id="om_reply",
        runner_exit_code=0,
        timed_out=False,
    )
    base.update(over)
    return BotAction(**base)


# ---------------------------------------------------------------------------

def test_run_bot_dispatches_each_event_to_handle_event(monkeypatch):
    bot = _bot()
    events = [
        {"message_id": "om_1", "chat_id": "oc_test"},
        {"message_id": "om_2", "chat_id": "oc_test"},
    ]
    seen: List[dict] = []

    def fake_consume(*, profile, max_events, timeout):
        assert profile == "cli_aaa"
        yield from events

    def fake_handler(ev, b):
        seen.append(ev)
        return _ok_action(source_message_id=ev["message_id"])

    monkeypatch.setattr(bb, "consume_im", fake_consume)
    monkeypatch.setattr(bb, "handle_event", fake_handler)

    actions = list(bb.run_bot(bot, max_events=2, timeout="30s"))
    assert seen == events
    assert [a.source_message_id for a in actions] == ["om_1", "om_2"]


def test_run_bot_swallows_handler_exception_continues(monkeypatch):
    """单条事件处理崩了不能让 daemon 整体死。"""
    bot = _bot()
    events = [{"message_id": "om_1"}, {"message_id": "om_2"}]

    def fake_consume(**_):
        yield from events

    calls: List[str] = []

    def fake_handler(ev, b):
        calls.append(ev["message_id"])
        if ev["message_id"] == "om_1":
            raise RuntimeError("boom")
        return _ok_action(source_message_id="om_2")

    monkeypatch.setattr(bb, "consume_im", fake_consume)
    monkeypatch.setattr(bb, "handle_event", fake_handler)

    actions = list(bb.run_bot(bot))
    # 两条都尝试过，第二条成功
    assert calls == ["om_1", "om_2"]
    assert len(actions) == 1
    assert actions[0].source_message_id == "om_2"


def test_run_bot_skips_when_handle_event_returns_none(monkeypatch):
    """unmatched 事件 handle_event 返回 None；daemon 不应 yield None。"""
    bot = _bot()

    def fake_consume(**_):
        yield {"message_id": "om_irrelevant"}

    monkeypatch.setattr(bb, "consume_im", fake_consume)
    monkeypatch.setattr(bb, "handle_event", lambda ev, b: None)

    actions = list(bb.run_bot(bot))
    assert actions == []


def test_run_bot_does_not_reference_bot_relay_task(monkeypatch):
    """M3.E：bot_bridge 不再 import bot_relay_task；relay_task 归 handle_event。

    若 bot_bridge 还在调 relay_task，下面 mock 不到属性会让 setattr 报 AttributeError，
    所以这里只验证模块属性不存在即可。
    """
    assert not hasattr(bb, "bot_relay_task"), \
        "bot_bridge should not import bot_relay_task in M3.E"


def test_run_bot_calls_cleanup_orphans_on_start(monkeypatch, tmp_path):
    monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
    from roostery import runner_registry
    cleaned = []
    monkeypatch.setattr(
        runner_registry.RunnerRegistry, "cleanup_orphans",
        lambda self: cleaned.append(True) or 0,
    )
    monkeypatch.setattr(bb, "consume_im", lambda **kw: iter([]))
    list(bb.run_bot(_bot()))
    assert cleaned == [True]


def test_run_bot_routes_event_to_hitl_router_first(monkeypatch):
    """abort 命中的事件不进 handle_event。"""
    events = [
        {"message_id": "om_1", "chat_id": "oc_x"},  # 假装 hitl 命中
        {"message_id": "om_2", "chat_id": "oc_x"},  # 进 handle_event
    ]
    seen = []

    def fake_consume(**_):
        yield from events

    def fake_handle(ev, b):
        seen.append(ev["message_id"])
        return _ok_action(source_message_id=ev["message_id"])

    from roostery import hitl_router
    calls = []
    def fake_dispatch(envelope, *, registry):
        calls.append(envelope["message_id"])
        if envelope["message_id"] == "om_1":
            from roostery.hitl_router import AbortDecision
            return AbortDecision(chat_id="oc_x", task_guid="t",
                                 runner_pid=1, reason="/stop")
        return None

    monkeypatch.setattr(bb, "consume_im", fake_consume)
    monkeypatch.setattr(bb, "handle_event", fake_handle)
    monkeypatch.setattr(hitl_router, "dispatch", fake_dispatch)

    actions = list(bb.run_bot(_bot()))
    assert calls == ["om_1", "om_2"]
    assert seen == ["om_2"]  # om_1 被 hitl 拦
    assert [a.source_message_id for a in actions] == ["om_2"]


def test_run_bot_parallel_dispatches_handle_event_in_threads(monkeypatch):
    """parallel=True：handle_event 在 worker thread 跑；可乱序但全跑过。"""
    import threading
    events = [{"message_id": f"om_{i}", "chat_id": "oc_x"} for i in range(3)]
    main_tid = threading.get_ident()
    worker_tids = []

    def fake_consume(**_):
        yield from events

    def fake_handle(ev, b):
        worker_tids.append(threading.get_ident())
        return _ok_action(source_message_id=ev["message_id"])

    monkeypatch.setattr(bb, "consume_im", fake_consume)
    monkeypatch.setattr(bb, "handle_event", fake_handle)

    actions = list(bb.run_bot(_bot(), parallel=True))
    # 全跑过
    msgs = sorted(a.source_message_id for a in actions)
    assert msgs == ["om_0", "om_1", "om_2"]
    # handle_event 不在主线程
    assert all(tid != main_tid for tid in worker_tids)

# ---------------------------------------------------------------------------
# Phase 5: base_intent_router hook

def _reset_base_cache(monkeypatch):
    """Clear bot_bridge's lazy base_config cache between tests."""
    monkeypatch.setattr(bb, "_BASE_CONFIGS_CACHE", None)


def test_run_sync_calls_base_intent_router_first(monkeypatch):
    """When base_intent_router consumes the event, handle_event must not run."""
    _reset_base_cache(monkeypatch)
    from roostery import base_config as _bc, base_intent_router as _bir

    event = {"message_id": "om_base", "chat_id": "oc_x", "content": "/run X"}
    monkeypatch.setattr(_bc, "load_all", lambda: [object()])  # non-empty configs

    calls = {"try_handle": 0, "handle": 0}

    def fake_try_handle(ev, *, configs, registry, reply_fn):
        calls["try_handle"] += 1
        return True

    def fake_handle(ev, b):
        calls["handle"] += 1
        return _ok_action()

    monkeypatch.setattr(bb, "consume_im", lambda **_: iter([event]))
    monkeypatch.setattr(_bir, "try_handle", fake_try_handle)
    monkeypatch.setattr(bb, "handle_event", fake_handle)

    actions = list(bb.run_bot(_bot()))
    assert calls["try_handle"] == 1
    assert calls["handle"] == 0
    assert actions == []


def test_run_sync_falls_through_when_base_not_consumed(monkeypatch):
    """try_handle returns False → handle_event still runs (R5 path)."""
    _reset_base_cache(monkeypatch)
    from roostery import base_config as _bc, base_intent_router as _bir

    event = {"message_id": "om_legacy", "chat_id": "oc_x", "content": "hi"}
    monkeypatch.setattr(_bc, "load_all", lambda: [object()])

    seen = []

    monkeypatch.setattr(_bir, "try_handle",
                        lambda ev, *, configs, registry, reply_fn: False)

    def fake_handle(ev, b):
        seen.append(ev["message_id"])
        return _ok_action(source_message_id=ev["message_id"])

    monkeypatch.setattr(bb, "consume_im", lambda **_: iter([event]))
    monkeypatch.setattr(bb, "handle_event", fake_handle)

    actions = list(bb.run_bot(_bot()))
    assert seen == ["om_legacy"]
    assert [a.source_message_id for a in actions] == ["om_legacy"]


def test_run_sync_skips_base_router_when_no_configs(monkeypatch):
    """Empty base_config list → hook is transparent (zero R5 regression)."""
    _reset_base_cache(monkeypatch)
    from roostery import base_config as _bc, base_intent_router as _bir

    event = {"message_id": "om_legacy", "chat_id": "oc_x", "content": "hi"}
    monkeypatch.setattr(_bc, "load_all", lambda: [])  # empty

    try_handle_calls = []

    def fake_try_handle(ev, *, configs, registry, reply_fn):
        try_handle_calls.append(ev)
        return True

    monkeypatch.setattr(_bir, "try_handle", fake_try_handle)
    monkeypatch.setattr(bb, "consume_im", lambda **_: iter([event]))
    monkeypatch.setattr(bb, "handle_event", lambda ev, b: _ok_action())

    list(bb.run_bot(_bot()))
    assert try_handle_calls == []  # short-circuited before try_handle


def test_run_parallel_calls_base_intent_router_first(monkeypatch):
    """parallel=True：base_intent_router 也优先，命中后 handle_event 不跑。"""
    _reset_base_cache(monkeypatch)
    from roostery import base_config as _bc, base_intent_router as _bir

    events = [
        {"message_id": "om_base", "chat_id": "oc_x", "content": "/run X"},
        {"message_id": "om_other", "chat_id": "oc_x", "content": "hi"},
    ]
    monkeypatch.setattr(_bc, "load_all", lambda: [object()])

    handled = []
    base_seen = []

    def fake_try_handle(ev, *, configs, registry, reply_fn):
        base_seen.append(ev["message_id"])
        return ev["message_id"] == "om_base"

    def fake_handle(ev, b):
        handled.append(ev["message_id"])
        return _ok_action(source_message_id=ev["message_id"])

    monkeypatch.setattr(bb, "consume_im", lambda **_: iter(events))
    monkeypatch.setattr(_bir, "try_handle", fake_try_handle)
    monkeypatch.setattr(bb, "handle_event", fake_handle)

    actions = list(bb.run_bot(_bot(), parallel=True))
    assert sorted(base_seen) == ["om_base", "om_other"]
    assert handled == ["om_other"]
    assert [a.source_message_id for a in actions] == ["om_other"]


def test_run_parallel_feeder_not_blocked_by_long_base_intent(monkeypatch):
    """parallel=True：base_intent_router 长跑（runner sleep）期间 feeder
    必须能继续读后续 event（如 /stop），否则 hitl_router 收不到打断。
    M4.E e2e 发现 critical bug：原实现把 _try_base_intent 同步放在 feeder。"""
    import threading
    import time as _time
    _reset_base_cache(monkeypatch)
    from roostery import base_config as _bc, base_intent_router as _bir

    events = [
        {"message_id": "om_base", "chat_id": "oc_x", "content": "/run X"},
        {"message_id": "om_followup", "chat_id": "oc_x", "content": "/stop"},
    ]
    monkeypatch.setattr(_bc, "load_all", lambda: [object()])

    base_started = threading.Event()
    base_release = threading.Event()
    handled = []

    def fake_try_handle(ev, *, configs, registry, reply_fn):
        if ev["message_id"] == "om_base":
            base_started.set()
            # 故意阻塞，模拟 runner sleep 30s
            base_release.wait(timeout=2.0)
            return True
        return False  # om_followup 让 _is_abort 兜（fake handle_event 处理）

    def fake_handle(ev, b):
        handled.append(ev["message_id"])
        return _ok_action(source_message_id=ev["message_id"])

    monkeypatch.setattr(bb, "consume_im", lambda **_: iter(events))
    monkeypatch.setattr(_bir, "try_handle", fake_try_handle)
    monkeypatch.setattr(bb, "handle_event", fake_handle)

    # 用线程跑 run_bot，主线程等 base_started 后立即检查 om_followup 是否已被 handle
    actions_iter = bb.run_bot(_bot(), parallel=True)
    results = []
    def consume():
        for a in actions_iter:
            results.append(a)
    t = threading.Thread(target=consume, daemon=True)
    t.start()

    # 等 base 进 sleep
    assert base_started.wait(timeout=2.0), "base intent worker 没启动"
    # 关键断言：base 还没完成时，om_followup 应该已经被 feeder 读到 + worker 处理
    _time.sleep(0.3)  # 给 worker 一点时间
    assert "om_followup" in handled, \
        "feeder 被 base 阻塞了：om_followup 在 base 完成前未被 handle_event"

    base_release.set()
    t.join(timeout=3.0)


# ---------------------------------------------------------------------------
# Phase 6: _try_base_intent 二段调度

def test_try_base_intent_run_path_consumes_event_skips_nl(monkeypatch):
    """显式 /run 协议被 try_handle 消费时，try_handle_nl 不应被调用。"""
    from roostery import bot_bridge

    monkeypatch.setattr(bot_bridge, "_get_base_configs", lambda: [object()])

    nl_called = [False]

    def fake_try_handle(event, *, configs, registry, reply_fn):
        return True  # consumed

    def fake_try_handle_nl(event, *, configs, registry, reply_fn):
        nl_called[0] = True
        return True

    monkeypatch.setattr(bot_bridge.base_intent_router, "try_handle", fake_try_handle)
    monkeypatch.setattr(bot_bridge.base_intent_router, "try_handle_nl", fake_try_handle_nl)

    event = {"content": "/run X record:rec1"}
    registry = MagicMock()
    consumed = bot_bridge._try_base_intent(event, registry)

    assert consumed is True
    assert nl_called[0] is False


def test_try_base_intent_falls_through_to_nl_when_run_not_consumed(monkeypatch):
    """try_handle 返回 False 时，二段调度应试 try_handle_nl。"""
    from roostery import bot_bridge

    monkeypatch.setattr(bot_bridge, "_get_base_configs", lambda: [object()])

    nl_called = [False]

    def fake_try_handle(event, *, configs, registry, reply_fn):
        return False  # no /run match

    def fake_try_handle_nl(event, *, configs, registry, reply_fn):
        nl_called[0] = True
        return True  # NL consumed

    monkeypatch.setattr(bot_bridge.base_intent_router, "try_handle", fake_try_handle)
    monkeypatch.setattr(bot_bridge.base_intent_router, "try_handle_nl", fake_try_handle_nl)

    event = {"content": "公众号写一篇 AI"}
    registry = MagicMock()
    consumed = bot_bridge._try_base_intent(event, registry)

    assert consumed is True
    assert nl_called[0] is True


def test_try_base_intent_returns_false_when_neither_consumes(monkeypatch):
    """两段都未消费，让 R5 legacy IM path 接管。"""
    from roostery import bot_bridge

    monkeypatch.setattr(bot_bridge, "_get_base_configs", lambda: [object()])
    monkeypatch.setattr(bot_bridge.base_intent_router, "try_handle", lambda *a, **kw: False)
    monkeypatch.setattr(bot_bridge.base_intent_router, "try_handle_nl", lambda *a, **kw: False)

    event = {"content": "天气真好"}
    registry = MagicMock()
    consumed = bot_bridge._try_base_intent(event, registry)

    assert consumed is False
