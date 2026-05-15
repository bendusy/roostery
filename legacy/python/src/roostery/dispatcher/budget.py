"""调度预算闸 — 每日滚动的 bucket 计数器。

bucket 维度：``global`` / per-runner（cc / codex / gemini）/ per-rule。
持久化：``~/.feishu_hub/state/budget.json``，原子写。跨天自动 roll-over。

设计：``docs/FEISHU_HUB_DISPATCHER_DESIGN.md`` §8。
"""
from __future__ import annotations

import datetime as _dt
import json
import os
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Mapping, Optional

from roostery import config as cfgmod


DEFAULT_LIMITS: Dict[str, Dict[str, int]] = {
    "global":  {"max_calls": 200, "max_cost_cents": 2000},
    "cc":      {"max_calls": 100, "max_cost_cents": 1000},
    "codex":   {"max_calls": 100, "max_cost_cents": 1000},
    "gemini":  {"max_calls": 200, "max_cost_cents": 500},
}


@dataclass
class Bucket:
    calls: int = 0
    cost_cents: int = 0
    max_calls: Optional[int] = None
    max_cost_cents: Optional[int] = None

    def would_exceed(self, *, calls: int = 1, cost_cents: int = 0) -> Optional[str]:
        if self.max_calls is not None and self.calls + calls > self.max_calls:
            return f"calls {self.calls + calls} > max_calls {self.max_calls}"
        if self.max_cost_cents is not None and self.cost_cents + cost_cents > self.max_cost_cents:
            return f"cost_cents {self.cost_cents + cost_cents} > max_cost_cents {self.max_cost_cents}"
        return None

    def consume(self, *, calls: int = 1, cost_cents: int = 0) -> None:
        self.calls += calls
        self.cost_cents += cost_cents


class BudgetExceeded(RuntimeError):
    def __init__(self, bucket_name: str, reason: str):
        super().__init__(f"budget bucket {bucket_name!r}: {reason}")
        self.bucket_name = bucket_name
        self.reason = reason


# ---- 持久化 -------------------------------------------------------------

def _state_path() -> Path:
    return cfgmod.root_dir() / "state" / "budget.json"


def _today() -> str:
    return _dt.date.today().isoformat()


def _default_buckets() -> Dict[str, Bucket]:
    out: Dict[str, Bucket] = {}
    for name, limits in DEFAULT_LIMITS.items():
        out[name] = Bucket(
            max_calls=limits.get("max_calls"),
            max_cost_cents=limits.get("max_cost_cents"),
        )
    return out


@dataclass
class BudgetState:
    day: str = field(default_factory=_today)
    buckets: Dict[str, Bucket] = field(default_factory=_default_buckets)
    by_rule: Dict[str, Bucket] = field(default_factory=dict)

    @classmethod
    def from_dict(cls, raw: Mapping[str, Any]) -> "BudgetState":
        return cls(
            day=raw.get("day") or _today(),
            buckets={k: Bucket(**v) for k, v in (raw.get("buckets") or {}).items()},
            by_rule={k: Bucket(**v) for k, v in (raw.get("by_rule") or {}).items()},
        )

    def to_dict(self) -> Dict[str, Any]:
        return {
            "day": self.day,
            "buckets": {k: asdict(v) for k, v in self.buckets.items()},
            "by_rule": {k: asdict(v) for k, v in self.by_rule.items()},
        }

    def roll_over_if_needed(self) -> bool:
        today = _today()
        if self.day == today:
            return False
        self.day = today
        for b in self.buckets.values():
            b.calls = 0
            b.cost_cents = 0
        for b in self.by_rule.values():
            b.calls = 0
            b.cost_cents = 0
        return True


def load() -> BudgetState:
    p = _state_path()
    if not p.exists():
        return BudgetState()
    try:
        raw = json.loads(p.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return BudgetState()
    state = BudgetState.from_dict(raw)
    state.roll_over_if_needed()
    # 给可能漏掉的默认 bucket 补上 max_*
    for name, limits in DEFAULT_LIMITS.items():
        if name not in state.buckets:
            state.buckets[name] = Bucket(
                max_calls=limits.get("max_calls"),
                max_cost_cents=limits.get("max_cost_cents"),
            )
    return state


def save(state: BudgetState) -> Path:
    p = _state_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    tmp = p.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(state.to_dict(), ensure_ascii=False, indent=2),
                   encoding="utf-8")
    os.replace(tmp, p)
    return p


# ---- 入队前检查 ---------------------------------------------------------

def _bucket_name_for_runner(runner: str) -> Optional[str]:
    if runner == "cc_headless":
        return "cc"
    if runner == "codex_exec":
        return "codex"
    if runner == "gemini_headless":
        return "gemini"
    return None


def check_or_raise(
    state: BudgetState,
    *,
    runner: str,
    rule_name: str,
    rule_budget: Optional[Mapping[str, Any]] = None,
    cost_cents: int = 0,
) -> None:
    """在 enqueue 前调用；超额抛 ``BudgetExceeded``。

    每次调用都跑一次 ``roll_over_if_needed()``，保证 tail 长进程过午夜也能 roll。
    """
    state.roll_over_if_needed()
    rb = dict(rule_budget or {})
    # 1. global
    if reason := state.buckets["global"].would_exceed(cost_cents=cost_cents):
        raise BudgetExceeded("global", reason)
    # 2. per-runner
    name = _bucket_name_for_runner(runner)
    if name and name in state.buckets:
        if reason := state.buckets[name].would_exceed(cost_cents=cost_cents):
            raise BudgetExceeded(name, reason)
    # 3. per-rule（动态）
    if rb:
        rb_bucket = state.by_rule.setdefault(rule_name, Bucket(
            max_calls=rb.get("max_calls"),
            max_cost_cents=rb.get("max_cost_cents"),
        ))
        # 允许用户更新 rule budget 上限（rules.yaml 改了）
        if rb.get("max_calls") is not None:
            rb_bucket.max_calls = rb["max_calls"]
        if rb.get("max_cost_cents") is not None:
            rb_bucket.max_cost_cents = rb["max_cost_cents"]
        if reason := rb_bucket.would_exceed(cost_cents=cost_cents):
            raise BudgetExceeded(f"rule:{rule_name}", reason)


def record(
    state: BudgetState,
    *,
    runner: str,
    rule_name: str,
    cost_cents: int = 0,
) -> None:
    """dispatch 完成后调用，落账；同样在记账前 roll-over。"""
    state.roll_over_if_needed()
    state.buckets["global"].consume(cost_cents=cost_cents)
    name = _bucket_name_for_runner(runner)
    if name and name in state.buckets:
        state.buckets[name].consume(cost_cents=cost_cents)
    if rule_name in state.by_rule:
        state.by_rule[rule_name].consume(cost_cents=cost_cents)
