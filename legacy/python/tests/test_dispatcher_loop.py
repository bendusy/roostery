"""roostery.dispatcher.loop 单测：dispatch_event 编排正确性。"""
from typing import Any, Dict, List

import pytest

from roostery.dispatcher import budget, loop, rules, runners, trace


def _emit_collector():
    out: List[Dict[str, Any]] = []
    return out, out.append


def _rule(name="r", when=None, runner="noop", prompt="p", **action_kw):
    when = when or {"event_type": "agent.session_end"}
    return rules.Rule(
        name=name, when=when,
        action=rules.Action(runner=runner, prompt=prompt, **action_kw),
    )


def _ctx(rules_list, *, max_depth=3, run_fn=None):
    return loop.DispatchContext(
        rules=rules_list,
        runaway=trace.RunawayTracker(window_s=60, threshold=3),
        budget_state=budget.BudgetState(),
        emit=lambda e: events.append(e) if events is not None else None,  # set below
        max_depth=max_depth,
        run_fn=run_fn or (lambda spec, ctx: runners.RunResult(
            runner=spec.runner, exit_code=0,
            stdout="ok", stderr="", stdout_head="ok", stderr_head="",
            duration_ms=10, timed_out=False, final_text="ok-final",
        )),
    )


@pytest.fixture
def events():
    return []


@pytest.fixture
def dctx(events):
    rs = [_rule()]
    return loop.DispatchContext(
        rules=rs,
        runaway=trace.RunawayTracker(window_s=60, threshold=3),
        budget_state=budget.BudgetState(),
        emit=events.append,
        run_fn=lambda spec, ctx: runners.RunResult(
            runner=spec.runner, exit_code=0, stdout="ok", stderr="",
            stdout_head="ok", stderr_head="", duration_ms=10,
            timed_out=False, final_text="all done",
        ),
    )


# ---- 基础路径 -----------------------------------------------------------

def test_no_rule_hit_emits_nothing(events):
    dctx = loop.DispatchContext(
        rules=[_rule(when={"event_type": "X"})],
        runaway=trace.RunawayTracker(),
        budget_state=budget.BudgetState(),
        emit=events.append,
        run_fn=lambda s, c: runners.RunResult(
            runner=s.runner, exit_code=0, stdout="", stderr="",
            stdout_head="", stderr_head="", duration_ms=0, timed_out=False),
    )
    assert loop.dispatch_event({"event_type": "Y"}, dctx) == 0
    assert events == []


def test_happy_path_emits_enqueued_started_completed(dctx, events):
    n = loop.dispatch_event(
        {"event_type": "agent.session_end", "event_id": "E0"}, dctx)
    assert n == 1
    types = [e["event_type"] for e in events]
    assert types == ["dispatch.enqueued", "dispatch.started", "dispatch.completed"]
    # depth=1, trace_id 自动生成
    actor = events[-1]["actor"]
    assert actor["depth"] == 1
    assert actor["parent_event_id"] == "E0"
    assert len(actor["trace_id"]) == 32
    assert events[-1]["summary"] == "all done"


def test_self_event_is_filtered(dctx, events):
    n = loop.dispatch_event({"event_type": "dispatch.completed",
                             "event_id": "X"}, dctx)
    assert n == 0
    assert events == []


# ---- gates --------------------------------------------------------------

def test_rule_level_max_depth_overrides_global(events):
    """rule.action.budget.max_depth 优先于 dctx.max_depth。"""
    r = rules.Rule(
        name="r",
        when={"event_type": "agent.session_end"},
        action=rules.Action(runner="noop", budget={"max_depth": 1}),
    )
    dctx = loop.DispatchContext(
        rules=[r], runaway=trace.RunawayTracker(),
        budget_state=budget.BudgetState(),
        emit=events.append,
        run_fn=lambda s, c: pytest.fail("must not run"),
        max_depth=100,
    )
    event = {"event_type": "agent.session_end", "event_id": "E1",
             "actor": {"trace_id": "T", "depth": 1, "parent_event_id": "P"}}
    n = loop.dispatch_event(event, dctx)
    assert n == 0
    assert events[0]["event_type"] == "dispatch.depth_exceeded"


def test_depth_gate_blocks(events):
    rs = [_rule()]
    dctx = loop.DispatchContext(
        rules=rs, runaway=trace.RunawayTracker(),
        budget_state=budget.BudgetState(),
        emit=events.append,
        run_fn=lambda s, c: pytest.fail("runner must not be called"),
        max_depth=2,
    )
    event = {
        "event_type": "agent.session_end",
        "event_id": "E1",
        "actor": {"agent": "cc", "trace_id": "T", "depth": 2,
                  "parent_event_id": "E0"},
    }
    n = loop.dispatch_event(event, dctx)
    assert n == 0
    assert [e["event_type"] for e in events] == ["dispatch.depth_exceeded"]


def test_runaway_gate_blocks(events):
    rs = [_rule()]
    runaway = trace.RunawayTracker(window_s=60, threshold=2)
    # 预先把 trace 的窗口塞满
    runaway.record("T_PRESET"); runaway.record("T_PRESET")
    dctx = loop.DispatchContext(
        rules=rs, runaway=runaway,
        budget_state=budget.BudgetState(),
        emit=events.append,
        run_fn=lambda s, c: pytest.fail("runner must not be called"),
    )
    event = {
        "event_type": "agent.session_end",
        "event_id": "E1",
        "actor": {"trace_id": "T_PRESET", "depth": 1},
    }
    n = loop.dispatch_event(event, dctx)
    assert n == 0
    assert [e["event_type"] for e in events] == ["dispatch.runaway"]


def test_budget_gate_blocks(events):
    rs = [_rule(runner="cc_headless")]
    bs = budget.BudgetState()
    bs.buckets["global"].max_calls = 1
    bs.buckets["global"].calls = 1   # 已用满
    dctx = loop.DispatchContext(
        rules=rs, runaway=trace.RunawayTracker(),
        budget_state=bs, emit=events.append,
        run_fn=lambda s, c: pytest.fail("runner must not be called"),
    )
    n = loop.dispatch_event({"event_type": "agent.session_end", "event_id": "E"},
                            dctx)
    assert n == 0
    assert [e["event_type"] for e in events] == ["dispatch.budget_exceeded"]


# ---- failure / timeout --------------------------------------------------

def test_runner_failure_emits_failed(events):
    rs = [_rule()]
    dctx = loop.DispatchContext(
        rules=rs, runaway=trace.RunawayTracker(),
        budget_state=budget.BudgetState(), emit=events.append,
        run_fn=lambda s, c: runners.RunResult(
            runner=s.runner, exit_code=2, stdout="", stderr="bad",
            stdout_head="", stderr_head="bad", duration_ms=5, timed_out=False,
        ),
    )
    loop.dispatch_event({"event_type": "agent.session_end", "event_id": "E"}, dctx)
    types = [e["event_type"] for e in events]
    assert types[-1] == "dispatch.failed"
    assert events[-1]["command"]["exit_code"] == 2
    assert "bad" in events[-1]["summary"]


def test_runner_timeout_emits_timeout(events):
    rs = [_rule()]
    dctx = loop.DispatchContext(
        rules=rs, runaway=trace.RunawayTracker(),
        budget_state=budget.BudgetState(), emit=events.append,
        run_fn=lambda s, c: runners.RunResult(
            runner=s.runner, exit_code=-1, stdout="", stderr="",
            stdout_head="", stderr_head="", duration_ms=600000, timed_out=True,
        ),
    )
    loop.dispatch_event({"event_type": "agent.session_end", "event_id": "E"}, dctx)
    assert events[-1]["event_type"] == "dispatch.timeout"


def test_runner_exception_emits_failed(events):
    rs = [_rule()]

    def boom(*a, **kw):
        raise RuntimeError("kaboom")

    dctx = loop.DispatchContext(
        rules=rs, runaway=trace.RunawayTracker(),
        budget_state=budget.BudgetState(), emit=events.append,
        run_fn=boom,
    )
    loop.dispatch_event({"event_type": "agent.session_end", "event_id": "E"}, dctx)
    final = events[-1]
    assert final["event_type"] == "dispatch.failed"
    assert "kaboom" in final["summary"]


# ---- multi-hit / continue -----------------------------------------------

def test_multi_continue_fans_out(events):
    r1 = rules.Rule(name="a",
                    when={"event_type": "agent.session_end"},
                    action=rules.Action(runner="noop"),
                    cont=True)
    r2 = rules.Rule(name="b",
                    when={"event_type": "agent.session_end"},
                    action=rules.Action(runner="noop"))
    dctx = loop.DispatchContext(
        rules=[r1, r2], runaway=trace.RunawayTracker(),
        budget_state=budget.BudgetState(), emit=events.append,
        run_fn=lambda s, c: runners.RunResult(
            runner=s.runner, exit_code=0, stdout="", stderr="",
            stdout_head="", stderr_head="", duration_ms=0, timed_out=False),
    )
    n = loop.dispatch_event({"event_type": "agent.session_end", "event_id": "E"},
                            dctx)
    assert n == 2
    enqueued = [e for e in events if e["event_type"] == "dispatch.enqueued"]
    assert [e["tags"][1] for e in enqueued] == ["a", "b"]


# ---- 预算落账 -----------------------------------------------------------

def test_records_cost_into_budget(events):
    rs = [_rule(runner="cc_headless")]
    bs = budget.BudgetState()
    dctx = loop.DispatchContext(
        rules=rs, runaway=trace.RunawayTracker(),
        budget_state=bs, emit=events.append,
        run_fn=lambda s, c: runners.RunResult(
            runner=s.runner, exit_code=0, stdout="", stderr="",
            stdout_head="", stderr_head="", duration_ms=0, timed_out=False,
            cost_cents=12, tokens=200, final_text="x",
        ),
    )
    loop.dispatch_event({"event_type": "agent.session_end", "event_id": "E"}, dctx)
    assert bs.buckets["global"].cost_cents == 12
    assert bs.buckets["cc"].cost_cents == 12
    # metrics 字段
    assert events[-1]["metrics"]["cost_cents"] == 12
    assert events[-1]["metrics"]["tokens"] == 200


# ---- trace 链路传播 ---------------------------------------------------

def test_existing_trace_continues_and_increments_depth(events):
    rs = [_rule()]
    dctx = loop.DispatchContext(
        rules=rs, runaway=trace.RunawayTracker(),
        budget_state=budget.BudgetState(), emit=events.append,
        run_fn=lambda s, c: runners.RunResult(
            runner=s.runner, exit_code=0, stdout="", stderr="",
            stdout_head="", stderr_head="", duration_ms=0, timed_out=False),
    )
    event = {"event_type": "agent.session_end", "event_id": "E5",
             "actor": {"trace_id": "T_KEEP", "depth": 1, "parent_event_id": "E4"}}
    loop.dispatch_event(event, dctx)
    final = events[-1]
    assert final["actor"]["trace_id"] == "T_KEEP"
    assert final["actor"]["depth"] == 2
    assert final["actor"]["parent_event_id"] == "E5"


# ---- T5: runner 完成 → 追加 task step ----------------------------------

def test_runner_completion_appends_task_step_when_actor_has_task_guid(events, monkeypatch):
    """envelope.actor.task_guid 存在时，runner 完成会触发 task_writer.append_steps。"""
    from unittest.mock import MagicMock

    # 直接对当前 loop 模块对象 setattr，规避 cli_m3a 测试清 sys.modules 后
    # `patch("roostery.dispatcher.loop.task_writer")` 命中新缓存而非测试持有的旧模块的问题。
    tw = MagicMock()
    monkeypatch.setattr(loop, "task_writer", tw)

    r = _rule(when={"event_type": "task_done"})
    dctx = loop.DispatchContext(
        rules=[r],
        runaway=trace.RunawayTracker(window_s=60, threshold=3),
        budget_state=budget.BudgetState(),
        emit=events.append,
        run_fn=lambda spec, ctx: runners.RunResult(
            runner=spec.runner, exit_code=0, stdout="done", stderr="",
            stdout_head="done", stderr_head="", duration_ms=10,
            timed_out=False, final_text="done-final",
        ),
    )
    incoming = {
        "event_type": "task_done",
        "event_id": "E_T5_1",
        "actor": {"agent": "cc", "task_guid": "g-xyz"},
    }
    loop.dispatch_event(incoming, dctx)
    tw.append_steps.assert_called_once()
    call_args = tw.append_steps.call_args
    first = call_args.args[0] if call_args.args else call_args.kwargs.get("task_guid")
    assert first == "g-xyz"


def test_runner_completion_skips_when_no_task_guid(events, monkeypatch):
    """无 task_guid 时 task_writer.append_steps 不应被调用。"""
    from unittest.mock import MagicMock

    tw = MagicMock()
    monkeypatch.setattr(loop, "task_writer", tw)

    r = _rule(when={"event_type": "task_done"})
    dctx = loop.DispatchContext(
        rules=[r],
        runaway=trace.RunawayTracker(window_s=60, threshold=3),
        budget_state=budget.BudgetState(),
        emit=events.append,
        run_fn=lambda spec, ctx: runners.RunResult(
            runner=spec.runner, exit_code=0, stdout="", stderr="",
            stdout_head="", stderr_head="", duration_ms=1,
            timed_out=False, final_text="",
        ),
    )
    loop.dispatch_event(
        {"event_type": "task_done", "event_id": "E_T5_2",
         "actor": {"agent": "cc"}},
        dctx,
    )
    tw.append_steps.assert_not_called()
