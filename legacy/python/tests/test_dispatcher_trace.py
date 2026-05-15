"""roostery.dispatcher.trace 单测。"""
import os

import pytest

from roostery import journal
from roostery.dispatcher import trace


# ---- new_trace_id / TraceCtx ---------------------------------------------

def test_new_trace_id_unique_and_hex():
    a = trace.new_trace_id()
    b = trace.new_trace_id()
    assert a != b
    assert len(a) == 32
    int(a, 16)  # 必须是 hex


def test_ctx_to_env_roundtrip():
    ctx = trace.TraceCtx(trace_id="abc", depth=2, parent_event_id="EVT")
    env = ctx.to_env()
    assert env[trace.ENV_TRACE_ID] == "abc"
    assert env[trace.ENV_DEPTH] == "2"
    assert env[trace.ENV_PARENT] == "EVT"
    back = trace.from_env(env)
    assert back == ctx


def test_ctx_to_env_skips_parent_when_none():
    ctx = trace.TraceCtx(trace_id="t", depth=1, parent_event_id=None)
    env = ctx.to_env()
    assert trace.ENV_PARENT not in env


def test_from_env_returns_none_when_no_trace():
    assert trace.from_env({}) is None
    assert trace.from_env({"FEISHU_HUB_DEPTH": "1"}) is None


def test_from_env_tolerates_bad_depth():
    env = {trace.ENV_TRACE_ID: "x", trace.ENV_DEPTH: "garbage"}
    ctx = trace.from_env(env)
    assert ctx is not None
    assert ctx.depth == 0


# ---- from_event ----------------------------------------------------------

def test_from_event_extracts_actor_trace():
    evt = {"actor": {"agent": "cc", "trace_id": "T", "depth": 2,
                     "parent_event_id": "P"}}
    ctx = trace.from_event(evt)
    assert ctx.trace_id == "T"
    assert ctx.depth == 2
    assert ctx.parent_event_id == "P"


def test_from_event_returns_none_when_actor_missing_trace():
    assert trace.from_event({"actor": {"agent": "cc"}}) is None
    assert trace.from_event({}) is None


# ---- child / depth gating -------------------------------------------------

def test_child_initializes_root():
    ctx = trace.child(None, parent_event_id="E0")
    assert ctx.depth == 1
    assert ctx.parent_event_id == "E0"
    assert len(ctx.trace_id) == 32


def test_child_increments_depth_and_keeps_trace_id():
    root = trace.TraceCtx(trace_id="T", depth=1, parent_event_id=None)
    nxt = trace.child(root, parent_event_id="E1")
    assert nxt.trace_id == "T"
    assert nxt.depth == 2
    assert nxt.parent_event_id == "E1"


def test_check_depth_passes_below_limit():
    ctx = trace.TraceCtx(trace_id="T", depth=2, parent_event_id=None)
    trace.check_depth(ctx, max_depth=3)  # 不抛


def test_check_depth_raises_at_limit():
    ctx = trace.TraceCtx(trace_id="T", depth=3, parent_event_id=None)
    with pytest.raises(trace.DepthExceeded) as exc:
        trace.check_depth(ctx, max_depth=3)
    assert exc.value.max_depth == 3
    assert exc.value.ctx is ctx


def test_check_depth_handles_none():
    trace.check_depth(None, max_depth=1)  # 链路起点不抛


# ---- RunawayTracker ------------------------------------------------------

def test_runaway_records_count():
    clock = [100.0]
    t = trace.RunawayTracker(window_s=60, threshold=5, clock=lambda: clock[0])
    for i in range(3):
        clock[0] += 1
        assert t.record("T1") == i + 1


def test_runaway_evicts_old():
    clock = [0.0]
    t = trace.RunawayTracker(window_s=10, threshold=100, clock=lambda: clock[0])
    t.record("T")
    clock[0] = 5
    t.record("T")
    clock[0] = 15        # 第一条已出窗
    assert t.record("T") == 2


def test_runaway_check_raises_at_threshold():
    clock = [0.0]
    t = trace.RunawayTracker(window_s=60, threshold=3, clock=lambda: clock[0])
    for _ in range(3):
        clock[0] += 1
        t.record("T")
    with pytest.raises(trace.RunawayDetected) as exc:
        t.check("T")
    assert exc.value.trace_id == "T"
    assert exc.value.count == 3


def test_runaway_isolates_by_trace_id():
    clock = [0.0]
    t = trace.RunawayTracker(window_s=60, threshold=2, clock=lambda: clock[0])
    t.record("A"); t.record("A")
    t.record("B")
    with pytest.raises(trace.RunawayDetected):
        t.check("A")
    t.check("B")  # 不抛


# ---- journal actor_from_env 集成 ----------------------------------------

def test_journal_actor_includes_trace(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_AGENT", "codex")
    monkeypatch.setenv(trace.ENV_TRACE_ID, "T_int")
    monkeypatch.setenv(trace.ENV_DEPTH, "2")
    monkeypatch.setenv(trace.ENV_PARENT, "EVT_PARENT")
    actor = journal.actor_from_env()
    assert actor["agent"] == "codex"
    assert actor["trace_id"] == "T_int"
    assert actor["depth"] == 2
    assert actor["parent_event_id"] == "EVT_PARENT"


def test_journal_actor_skips_trace_fields_when_unset(monkeypatch):
    for k in (trace.ENV_TRACE_ID, trace.ENV_DEPTH, trace.ENV_PARENT):
        monkeypatch.delenv(k, raising=False)
    actor = journal.actor_from_env()
    assert "trace_id" not in actor
    assert "depth" not in actor
    assert "parent_event_id" not in actor


def test_journal_actor_tolerates_bad_depth_env(monkeypatch):
    monkeypatch.setenv(trace.ENV_TRACE_ID, "T")
    monkeypatch.setenv(trace.ENV_DEPTH, "not_a_number")
    actor = journal.actor_from_env()
    assert actor["depth"] == 0
