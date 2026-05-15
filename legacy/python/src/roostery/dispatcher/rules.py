"""matcher 规则表 — YAML DSL → 编译规则 → 匹配 journal envelope → 产出 RunSpec。

设计：``docs/FEISHU_HUB_DISPATCHER_DESIGN.md`` §5。

DSL 字段（每条 rule）：
- ``name``: 规则名，唯一
- ``when``: 匹配条件（多键 AND）
    * ``event_type``: 精确匹配 envelope.event_type
    * ``actor.agent``: 精确匹配 actor.agent
    * ``cwd_glob``: fnmatch 匹配 envelope.cwd
    * ``tags_includes``: list[str]，envelope.tags 必须**全部包含**
    * ``result_contains``: substring 匹配 envelope.summary 或 actor.result
    * ``summary_regex``: 正则匹配 envelope.summary
- ``action``: 动作
    * ``runner``: noop / cc_headless / codex_exec / gemini_headless / switch_by_field
    * ``prompt``: jinja2-lite 模板（仅支持 ``{{ a.b.c }}`` 路径取值）
    * ``model`` / ``timeout_s`` / ``cwd``：透传给 RunSpec
    * ``resume_id``: 续会话标识
    * ``branches``: switch_by_field 用，按 field 值映射到 runner
    * ``budget``: 覆盖默认预算（``max_calls`` / ``max_cost_cents`` / ``max_depth``）
- ``continue``: bool，匹配后是否继续尝试下一条（默认 ``False``）

`agent.dispatched*` / `dispatch.*` 这些 event 永远不参与 matcher（防自激）。

M3.A 协同模型说明：本模块只匹配 hook 单次触发（fire 模式）传入的 envelope。
不再从本地 journal tail 触发。云侧（@mention/关键词）路由由飞书 Base
Workflow ``LarkMessageTrigger`` 承担（见 M3.B 计划）。
"""
from __future__ import annotations

import fnmatch
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, Iterable, List, Mapping, Optional, Sequence, Tuple

from . import runners


# event_type 黑名单：dispatcher 自己产生的事件**绝对不许**触发新派
SELF_EVENT_PREFIXES: Tuple[str, ...] = (
    "dispatch.",
    "agent.dispatched",
)


# ---- 数据类 ---------------------------------------------------------------

@dataclass(frozen=True)
class Action:
    runner: str
    prompt: str = ""
    model: Optional[str] = None
    cwd: Optional[str] = None
    resume_id: Optional[str] = None
    timeout_s: Optional[int] = None
    branches: Mapping[str, str] = field(default_factory=dict)
    branch_field: Optional[str] = None      # switch_by_field 时取该路径决定 runner
    budget: Mapping[str, Any] = field(default_factory=dict)
    # 结果回写（②回路闭环）：dispatch.completed/failed 后把 summary 发回去
    result_writeback: Mapping[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class Rule:
    name: str
    when: Mapping[str, Any]
    action: Action
    cont: bool = False


@dataclass(frozen=True)
class Match:
    rule: Rule
    spec: runners.RunSpec


# ---- DSL 加载 ------------------------------------------------------------

def load_rules_file(path: Path) -> List[Rule]:
    import yaml  # 延迟 import；与 config.py 同策略
    raw = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    version = raw.get("version", 1)
    if version != 1:
        raise ValueError(f"rules.yaml: unsupported version {version}")
    rules_raw = raw.get("rules") or []
    return [compile_rule(r) for r in rules_raw]


def compile_rule(raw: Mapping[str, Any]) -> Rule:
    name = raw.get("name") or ""
    if not name:
        raise ValueError("rule missing 'name'")
    when = raw.get("when") or {}
    action_raw = raw.get("action") or {}
    runner = action_raw.get("runner")
    if not runner:
        raise ValueError(f"rule '{name}' missing action.runner")
    action = Action(
        runner=runner,
        prompt=action_raw.get("prompt", ""),
        model=action_raw.get("model"),
        cwd=action_raw.get("cwd"),
        resume_id=action_raw.get("resume_id"),
        timeout_s=action_raw.get("timeout_s"),
        branches=action_raw.get("branches") or {},
        branch_field=action_raw.get("branch_field") or action_raw.get("field"),
        budget=action_raw.get("budget") or {},
        result_writeback=action_raw.get("result_writeback") or {},
    )
    return Rule(name=name, when=when, action=action,
                cont=bool(raw.get("continue", False)))


# ---- 匹配 ---------------------------------------------------------------

def _is_self_event(event: Mapping[str, Any]) -> bool:
    et = event.get("event_type") or ""
    return et.startswith(SELF_EVENT_PREFIXES)


def _pluck(obj: Any, path: str) -> Any:
    """点路径取值，支持 ``actor.agent`` / ``command.argv`` / ``actor.depth``。"""
    cur = obj
    for key in path.split("."):
        if isinstance(cur, Mapping) and key in cur:
            cur = cur[key]
        else:
            return None
    return cur


def _match_one(rule: Rule, event: Mapping[str, Any]) -> bool:
    when = rule.when
    # event_type
    et_want = when.get("event_type")
    if et_want and event.get("event_type") != et_want:
        return False
    # actor.agent
    agent_want = when.get("actor.agent")
    if agent_want and _pluck(event, "actor.agent") != agent_want:
        return False
    # cwd_glob
    glob = when.get("cwd_glob")
    if glob:
        cwd = event.get("cwd") or ""
        if not fnmatch.fnmatch(cwd, glob):
            return False
    # tags_includes
    needs = when.get("tags_includes") or []
    if needs:
        tags = event.get("tags") or []
        if not all(t in tags for t in needs):
            return False
    # result_contains
    needle = when.get("result_contains")
    if needle:
        haystack = " ".join(str(x) for x in (
            event.get("summary") or "",
            _pluck(event, "actor.result") or "",
            (_pluck(event, "io.stdout_head") or "")[:1000],
        ))
        if needle not in haystack:
            return False
    # summary_regex
    pat = when.get("summary_regex")
    if pat and not re.search(pat, event.get("summary") or ""):
        return False
    return True


# ---- 模板渲染（极简，避免引 jinja2） -----------------------------------
#
# 支持：
#   {{ a.b.c }}                            # 路径取值
#   {{ a.b.c | default("fallback") }}      # 缺失/None 时 fallback
#   {{ a.b.c | default('单引号也行') }}
#
# 不支持 jinja2 的循环 / if / 过滤器链 / 表达式 —— 生产再升级走 jinja2。

_TPL_RE = re.compile(r"\{\{\s*([^}]+?)\s*\}\}")
_DEFAULT_RE = re.compile(
    r"^(?P<path>[^\s|]+)\s*\|\s*default\s*\(\s*"
    r"(?P<quote>[\"'])(?P<fallback>.*?)(?P=quote)\s*\)\s*$"
)


def render(template: str, ctx: Mapping[str, Any]) -> str:
    """``{{ a.b.c }}`` / ``{{ a.b.c | default("x") }}``；缺失 + 无 default → 空串。"""
    def _sub(m):
        expr = m.group(1).strip()
        d = _DEFAULT_RE.match(expr)
        if d:
            val = _pluck(ctx, d.group("path"))
            if val is None or val == "":
                return d.group("fallback")
            return str(val)
        val = _pluck(ctx, expr)
        return "" if val is None else str(val)
    return _TPL_RE.sub(_sub, template or "")


# ---- 编译 action → RunSpec ---------------------------------------------

def to_spec(action: Action, event: Mapping[str, Any]) -> runners.RunSpec:
    """根据 action + event 构造 RunSpec；switch_by_field 在此解析。"""
    runner = action.runner
    ctx = {"trigger": event, "fields": event.get("fields") or {},
           "cwd": event.get("cwd")}
    if runner == "switch_by_field":
        if not action.branch_field:
            raise ValueError("switch_by_field requires 'field' in action")
        value = str(_pluck(ctx, action.branch_field) or "")
        runner = action.branches.get(value)
        if not runner:
            raise ValueError(
                f"switch_by_field: no branch for field={action.branch_field!r}"
                f" value={value!r}"
            )
    prompt = render(action.prompt, ctx)
    return runners.RunSpec(
        runner=runner,
        prompt=prompt,
        cwd=render(action.cwd, ctx) if action.cwd else None,
        model=action.model,
        resume_id=action.resume_id,
        timeout_s=action.timeout_s or runners.DEFAULT_TIMEOUT_S,
    )


def matches(rules: Sequence[Rule], event: Mapping[str, Any]) -> List[Match]:
    """对一个 event 返回**所有**命中规则（按 continue=True 链式扩展）。"""
    if _is_self_event(event):
        return []
    out: List[Match] = []
    for rule in rules:
        if not _match_one(rule, event):
            continue
        try:
            spec = to_spec(rule.action, event)
        except ValueError:
            # 规则错配（如 switch_by_field 缺 branch），跳过本条
            continue
        out.append(Match(rule=rule, spec=spec))
        if not rule.cont:
            break
    return out
