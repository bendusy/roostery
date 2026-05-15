"""roostery.dispatcher.budget 单测。"""
import datetime as _dt
import json
from pathlib import Path

import pytest

yaml = pytest.importorskip("yaml")

from roostery import config as cfgmod
from roostery.dispatcher import budget


@pytest.fixture
def fhub_home(monkeypatch, tmp_path):
    home = tmp_path / "fhub"
    monkeypatch.setenv(cfgmod.ENV_ROOT, str(home))
    return home


# ---- Bucket -------------------------------------------------------------

def test_bucket_would_exceed_calls():
    b = budget.Bucket(max_calls=2)
    assert b.would_exceed() is None
    b.consume()
    b.consume()
    assert "calls" in b.would_exceed()


def test_bucket_would_exceed_cost():
    b = budget.Bucket(max_cost_cents=10)
    b.consume(cost_cents=5)
    assert b.would_exceed(cost_cents=4) is None
    assert "cost_cents" in b.would_exceed(cost_cents=6)


def test_bucket_no_limits_never_exceeds():
    b = budget.Bucket()
    for _ in range(10):
        b.consume(cost_cents=100)
    assert b.would_exceed(cost_cents=10**9) is None


# ---- load / save / roll-over --------------------------------------------

def test_load_returns_defaults_when_missing(fhub_home):
    s = budget.load()
    assert s.day == _dt.date.today().isoformat()
    assert "global" in s.buckets
    assert s.buckets["global"].max_calls == budget.DEFAULT_LIMITS["global"]["max_calls"]


def test_save_load_roundtrip(fhub_home):
    s = budget.load()
    s.buckets["global"].consume(cost_cents=42)
    s.by_rule["my_rule"] = budget.Bucket(max_calls=5)
    s.by_rule["my_rule"].consume()
    budget.save(s)
    s2 = budget.load()
    assert s2.buckets["global"].cost_cents == 42
    assert s2.by_rule["my_rule"].calls == 1
    assert s2.by_rule["my_rule"].max_calls == 5


def test_roll_over_resets_buckets(fhub_home):
    s = budget.BudgetState(day="2020-01-01")
    s.buckets["global"].consume(cost_cents=100)
    s.by_rule["x"] = budget.Bucket(max_calls=3)
    s.by_rule["x"].consume()
    rolled = s.roll_over_if_needed()
    assert rolled is True
    assert s.day == _dt.date.today().isoformat()
    assert s.buckets["global"].calls == 0
    assert s.buckets["global"].cost_cents == 0
    assert s.by_rule["x"].calls == 0


def test_roll_over_no_op_same_day():
    today = _dt.date.today().isoformat()
    s = budget.BudgetState(day=today)
    s.buckets["global"].consume()
    assert s.roll_over_if_needed() is False
    assert s.buckets["global"].calls == 1


def test_load_recovers_from_corrupt_state(fhub_home):
    fhub_home.mkdir(parents=True, exist_ok=True)
    (fhub_home / "state").mkdir()
    (fhub_home / "state" / "budget.json").write_text("not json")
    s = budget.load()
    assert s.day == _dt.date.today().isoformat()


# ---- check_or_raise / record --------------------------------------------

def test_check_passes_within_limits():
    s = budget.BudgetState()
    budget.check_or_raise(s, runner="cc_headless", rule_name="r1")  # 不抛


def test_check_raises_on_global_calls():
    s = budget.BudgetState()
    s.buckets["global"].max_calls = 1
    s.buckets["global"].calls = 1
    with pytest.raises(budget.BudgetExceeded) as exc:
        budget.check_or_raise(s, runner="cc_headless", rule_name="r1")
    assert exc.value.bucket_name == "global"


def test_check_raises_on_per_runner_cost():
    s = budget.BudgetState()
    s.buckets["cc"].max_cost_cents = 50
    s.buckets["cc"].cost_cents = 30
    with pytest.raises(budget.BudgetExceeded) as exc:
        budget.check_or_raise(s, runner="cc_headless", rule_name="r1", cost_cents=25)
    assert exc.value.bucket_name == "cc"


def test_check_raises_on_per_rule_calls():
    s = budget.BudgetState()
    rb = {"max_calls": 1}
    budget.check_or_raise(s, runner="cc_headless", rule_name="r1", rule_budget=rb)
    budget.record(s, runner="cc_headless", rule_name="r1")
    with pytest.raises(budget.BudgetExceeded) as exc:
        budget.check_or_raise(s, runner="cc_headless", rule_name="r1", rule_budget=rb)
    assert "rule:r1" in exc.value.bucket_name


def test_check_updates_rule_budget_in_place():
    s = budget.BudgetState()
    budget.check_or_raise(s, runner="cc_headless", rule_name="r1",
                          rule_budget={"max_calls": 5})
    assert s.by_rule["r1"].max_calls == 5
    # 后续把 cap 改小（rules.yaml 编辑后 reload）
    budget.check_or_raise(s, runner="cc_headless", rule_name="r1",
                          rule_budget={"max_calls": 2})
    assert s.by_rule["r1"].max_calls == 2


def test_check_ignores_unknown_runner_per_runner_bucket():
    s = budget.BudgetState()
    # noop 这种 runner 没对应 bucket，不会拦
    budget.check_or_raise(s, runner="noop", rule_name="r1")


def test_record_increments_all_relevant_buckets():
    s = budget.BudgetState()
    budget.check_or_raise(s, runner="cc_headless", rule_name="r1",
                          rule_budget={"max_calls": 10})
    budget.record(s, runner="cc_headless", rule_name="r1", cost_cents=7)
    assert s.buckets["global"].calls == 1
    assert s.buckets["global"].cost_cents == 7
    assert s.buckets["cc"].calls == 1
    assert s.buckets["cc"].cost_cents == 7
    assert s.by_rule["r1"].calls == 1


def test_record_skipped_for_unregistered_rule():
    s = budget.BudgetState()
    budget.record(s, runner="cc_headless", rule_name="never_seen")
    assert "never_seen" not in s.by_rule


# ---- end-to-end -----------------------------------------------------------

def test_check_or_raise_rolls_over_stale_state():
    """tail 过午夜：state.day 是昨天，check 应自动 roll。"""
    s = budget.BudgetState(day="2020-01-01")
    s.buckets["global"].calls = 999       # 昨天已耗尽
    s.buckets["global"].max_calls = 100
    # 不抛：应先 roll-over 把 calls 归零
    budget.check_or_raise(s, runner="cc_headless", rule_name="r1")
    assert s.buckets["global"].calls == 0
    assert s.day == _dt.date.today().isoformat()


def test_record_rolls_over_stale_state():
    s = budget.BudgetState(day="2020-01-01")
    s.buckets["global"].calls = 999
    budget.record(s, runner="cc_headless", rule_name="r1", cost_cents=1)
    # 第一次记账：先 roll 归零，再加 1
    assert s.day == _dt.date.today().isoformat()
    assert s.buckets["global"].calls == 1
    assert s.buckets["global"].cost_cents == 1


def test_end_to_end_persistence(fhub_home):
    s = budget.load()
    budget.check_or_raise(s, runner="cc_headless", rule_name="r1",
                          rule_budget={"max_calls": 3})
    budget.record(s, runner="cc_headless", rule_name="r1", cost_cents=10)
    budget.save(s)
    s2 = budget.load()
    assert s2.buckets["cc"].calls == 1
    assert s2.by_rule["r1"].calls == 1
