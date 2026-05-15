"""dispatcher 主编排：event → match rules → trace/budget gates → run runner → emit result envelopes。

设计：``docs/FEISHU_HUB_DISPATCHER_DESIGN.md`` §3/§7/§8/§11。

本模块不做 IO：不读 rules.yaml、不调 lark-cli、不写飞书状态。
所有外部状态（rules、runaway_tracker、budget_state）由调用方传入；
``emit_event`` 回调用于本机 journal 审计落盘（不再用于写飞书协作状态——
M3.A 后协作状态由 hook/runner 直接调 lark-cli 写 Task / IM）。
"""
from __future__ import annotations

import os
import time
from dataclasses import dataclass
from typing import Any, Callable, Dict, List, Mapping, Optional, Sequence

from roostery import journal
from roostery import task_writer

from . import budget as budget_mod
from . import rules as rules_mod
from . import runners
from . import trace as trace_mod


# emit_event(payload_dict) — 调用方接管 journal 落盘
EmitFn = Callable[[Mapping[str, Any]], None]
# run_fn(spec, ctx) → RunResult
RunFn = Callable[[runners.RunSpec, Optional[trace_mod.TraceCtx]], runners.RunResult]


@dataclass
class DispatchContext:
    rules: Sequence[rules_mod.Rule]
    runaway: trace_mod.RunawayTracker
    budget_state: budget_mod.BudgetState
    emit: EmitFn
    run_fn: RunFn = runners.run
    max_depth: int = trace_mod.DEFAULT_MAX_DEPTH


# ---- 内部 helper --------------------------------------------------------

def _base_envelope(event_type: str, *, rule_name: str, runner: str,
                   ctx: Optional[trace_mod.TraceCtx]) -> Dict[str, Any]:
    base = {
        "event_type": event_type,
        "source": "dispatcher",
        "actor": journal.actor_from_env(),
        "tags": ["dispatch", rule_name],
        "command": {"argv": [runner]},
        "summary": None,
    }
    if ctx is not None:
        base["actor"].update(ctx.to_actor_fields())
    return base


def _refusal(emit: EmitFn, event_type: str, *, rule_name: str, runner: str,
             ctx: Optional[trace_mod.TraceCtx], reason: str) -> None:
    payload = _base_envelope(event_type, rule_name=rule_name, runner=runner, ctx=ctx)
    payload["summary"] = reason
    emit(payload)


def _maybe_append_task_step(incoming: Mapping[str, Any],
                             result_payload: Mapping[str, Any]) -> None:
    """若触发 envelope 的 actor.task_guid 存在，把 runner 结果作为一步写入飞书 Task。

    task_guid 取自 ``incoming``（原始触发事件），因为 dispatcher 输出 envelope 的
    actor 字段是用 ``journal.actor_from_env()`` 重建的，不会继承 incoming.actor。
    任何失败都仅记 stderr，不影响主流程。
    """
    actor = incoming.get("actor") or {}
    task_guid = actor.get("task_guid")
    if not task_guid:
        return
    cmd = result_payload.get("command") or {}
    runner = (cmd.get("argv") or ["?"])[0]
    exit_code = cmd.get("exit_code", "?")
    summary = result_payload.get("summary") or f"{runner} done (exit {exit_code})"
    try:
        task_writer.append_steps(task_guid, [summary])
    except Exception as e:
        import sys
        sys.stderr.write(f"[dispatcher] task step append failed: {e}\n")


# ---- 主入口 -------------------------------------------------------------

def dispatch_event(event: Mapping[str, Any], dctx: DispatchContext) -> int:
    """处理一条 envelope；返回真正执行的 dispatch 数（被拒/无命中返回 0）。

    Steps:
        1. 跳过自身事件（rules 内部也会过滤）
        2. matcher 找出所有命中 rule
        3. 对每条命中：
            a. 推导新 trace ctx（depth+1）
            b. check_depth；超限 → emit ``dispatch.depth_exceeded`` 跳过
            c. runaway.check；超限 → emit ``dispatch.runaway`` 跳过
            d. budget.check_or_raise；超限 → emit ``dispatch.budget_exceeded`` 跳过
            e. emit ``dispatch.enqueued`` + ``dispatch.started``
            f. run_fn(spec, ctx)
            g. budget.record；emit ``dispatch.completed/failed/timeout``
    """
    hits = rules_mod.matches(dctx.rules, event)
    if not hits:
        return 0

    parent_event_id = event.get("event_id")
    parent_ctx = trace_mod.from_event(event)
    executed = 0

    for match in hits:
        rule = match.rule
        spec = match.spec
        ctx = trace_mod.child(parent_ctx, parent_event_id=parent_event_id)

        # 1) depth gate — rule 内 budget.max_depth 优先，否则用全局
        rule_max_depth = int(rule.action.budget.get("max_depth",
                                                     dctx.max_depth))
        try:
            trace_mod.check_depth(ctx, max_depth=rule_max_depth)
        except trace_mod.DepthExceeded as e:
            _refusal(dctx.emit, "dispatch.depth_exceeded",
                     rule_name=rule.name, runner=spec.runner, ctx=ctx,
                     reason=str(e))
            continue

        # 2) runaway gate
        dctx.runaway.record(ctx.trace_id)
        try:
            dctx.runaway.check(ctx.trace_id)
        except trace_mod.RunawayDetected as e:
            _refusal(dctx.emit, "dispatch.runaway",
                     rule_name=rule.name, runner=spec.runner, ctx=ctx,
                     reason=str(e))
            continue

        # 3) budget gate（事前估 cost=0；事后 record 真实 cost）
        try:
            budget_mod.check_or_raise(
                dctx.budget_state, runner=spec.runner, rule_name=rule.name,
                rule_budget=rule.action.budget,
            )
        except budget_mod.BudgetExceeded as e:
            _refusal(dctx.emit, "dispatch.budget_exceeded",
                     rule_name=rule.name, runner=spec.runner, ctx=ctx,
                     reason=str(e))
            continue

        # 4) enqueued + started（同一刻；本模块单进程串行）
        dctx.emit({**_base_envelope("dispatch.enqueued", rule_name=rule.name,
                                     runner=spec.runner, ctx=ctx),
                   "summary": f"rule={rule.name} runner={spec.runner}"})
        dctx.emit({**_base_envelope("dispatch.started", rule_name=rule.name,
                                     runner=spec.runner, ctx=ctx),
                   "command": {"argv": [spec.runner], "duration_ms": 0}})

        # 5) run
        t0 = time.time()
        try:
            result = dctx.run_fn(spec, ctx)
        except Exception as e:                      # runner 自己抛了
            payload = _base_envelope("dispatch.failed", rule_name=rule.name,
                                      runner=spec.runner, ctx=ctx)
            payload["command"]["duration_ms"] = int((time.time() - t0) * 1000)
            payload["command"]["exit_code"] = -1
            payload["summary"] = f"runner exception: {e}"
            dctx.emit(payload)
            continue

        # 6) 记账
        budget_mod.record(
            dctx.budget_state, runner=spec.runner, rule_name=rule.name,
            cost_cents=result.cost_cents or 0,
        )

        # 7) 落 dispatch.completed/failed/timeout
        if result.timed_out:
            evt = "dispatch.timeout"
        elif result.exit_code == 0:
            evt = "dispatch.completed"
        else:
            evt = "dispatch.failed"
        payload = _base_envelope(evt, rule_name=rule.name,
                                  runner=spec.runner, ctx=ctx)
        payload["command"] = {
            "argv": [spec.runner],
            "exit_code": result.exit_code,
            "duration_ms": result.duration_ms,
        }
        payload["io"] = {
            "stdout_head": result.stdout_head,
            "stderr_head": result.stderr_head,
            "stdin_present": False,
            "tty": False,
        }
        payload["summary"] = (
            result.final_text[:200] if result.final_text else
            result.stderr_head[:200]
        )
        if result.cost_cents is not None:
            payload.setdefault("metrics", {})["cost_cents"] = result.cost_cents
        if result.tokens is not None:
            payload.setdefault("metrics", {})["tokens"] = result.tokens
        dctx.emit(payload)
        executed += 1

        # 7.5) 若 incoming envelope 带 task_guid，把这次 dispatch 结果写一步进飞书 Task
        _maybe_append_task_step(event, payload)

        # 8) 回路闭环 — result_writeback 把结果发回原始会话
        _writeback(rule, event, result, ctx, dctx)

    return executed


def _writeback(rule, event, result, ctx, dctx) -> None:
    """把 dispatch 结果按 rule.action.result_writeback 投递回去。

    支持的 ``kind``：
      - ``feishu_im``：调 lark_cli.im_send_text 发飞书 IM
        必填字段：``target``（jinja-lite 模板；可解析 ``{{trigger.actor.session}}`` 等）
        可选字段：``prefix``（结果文本前缀）、``max_chars``（默认 1500）、
                ``on``（``completed`` / ``failed`` / ``both``，默认 ``both``）

    任何失败都吞掉并写 ``dispatch.writeback_failed`` envelope，不影响主流程。
    """
    wb = rule.action.result_writeback
    if not wb:
        return
    kind = wb.get("kind") or ""
    on = (wb.get("on") or "both").lower()
    completed = result.exit_code == 0 and not result.timed_out
    if on == "completed" and not completed:
        return
    if on == "failed" and completed:
        return
    if kind == "feishu_im":
        _writeback_feishu_im(rule, event, result, wb, ctx, dctx)
        return
    dctx.emit({**_base_envelope("dispatch.writeback_failed",
                                  rule_name=rule.name, runner=rule.action.runner,
                                  ctx=ctx),
               "summary": f"unknown writeback kind: {kind}"})


def _writeback_feishu_im(rule, event, result, wb, ctx, dctx) -> None:
    import hashlib
    from roostery import lark_cli
    from roostery.dispatcher import rules as rules_mod  # lazy 防循环
    target_tpl = wb.get("target") or ""
    target = rules_mod.render(target_tpl, {"trigger": event})
    target = (target or "").strip()
    if not target:
        # 兜底用 config.notify_receive_id
        from roostery import config as cfgmod
        target = (cfgmod.load().get("notify_receive_id") or "").strip()
    if not target:
        dctx.emit({**_base_envelope("dispatch.writeback_failed",
                                      rule_name=rule.name, runner=rule.action.runner,
                                      ctx=ctx),
                   "summary": "feishu_im: no target"})
        return
    prefix = wb.get("prefix") or ""
    max_chars = int(wb.get("max_chars") or 1500)
    body = (result.final_text or result.stdout_head
            or result.stderr_head or "(no output)").strip()
    text = f"{prefix}{body}"[:max_chars]
    # idempotency-key 飞书侧 ≤ 50 chars；用 md5 短哈希保证唯一 + 长度安全
    key_raw = f"wb-{event.get('event_id','x')}-{rule.name}"
    key_short = "wb-" + hashlib.md5(key_raw.encode("utf-8")).hexdigest()
    try:
        mid = lark_cli.im_send_text(
            user_id=target, text=text,
            idempotency_key=key_short,
        )
        dctx.emit({**_base_envelope("dispatch.writeback_done",
                                      rule_name=rule.name, runner=rule.action.runner,
                                      ctx=ctx),
                   "summary": f"feishu_im → {target} mid={mid}"})
    except Exception as e:
        dctx.emit({**_base_envelope("dispatch.writeback_failed",
                                      rule_name=rule.name, runner=rule.action.runner,
                                      ctx=ctx),
                   "summary": f"feishu_im to {target} failed: {e}"})
