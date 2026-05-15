"""``python -m roostery.dispatcher`` — 反向 hook 调度入口。

2 种触发模式：

- ``fire``：单次。stdin 接收一条 envelope JSON（或 hook 原始 JSON 转 envelope），
  立刻 dispatch。CC/Codex/Gemini Stop/SessionEnd hook 直触用这个。
- ``replay``：调试。从已有 journal jsonl 按 event_id 选一条，重新跑一遍。

事件源是 lark-cli event consume（飞书=共享状态机），本地 journal 仅作审计缓存，
不作为协作事件源——故不提供 tail 模式。

所有模式共享同一份 rules.yaml + RunawayTracker + BudgetState。
"""
from __future__ import annotations

import argparse
import datetime as _dt
import json
import sys
from pathlib import Path
from typing import Any, Dict, List, Optional

from roostery import config as cfgmod
from roostery import journal

from . import budget as budget_mod
from . import loop
from . import rules as rules_mod
from . import trace as trace_mod


DEFAULT_RULES_FILE = "rules.yaml"


# ---- 公用 -------------------------------------------------------------

def _rules_path(arg: Optional[str]) -> Path:
    if arg:
        return Path(arg).expanduser()
    return cfgmod.root_dir() / DEFAULT_RULES_FILE


def _load_rules(arg: Optional[str]) -> List[rules_mod.Rule]:
    p = _rules_path(arg)
    if not p.exists():
        return []
    return rules_mod.load_rules_file(p)


def _build_emit() -> "loop.EmitFn":
    """默认 emit：写回 journal jsonl。"""
    def _emit(payload: Dict[str, Any]) -> None:
        try:
            journal.append(payload)
        except Exception as e:
            sys.stderr.write(f"[dispatcher] journal emit failed: {e}\n")
    return _emit


def _build_dctx(rules_arg: Optional[str], *,
                max_depth: Optional[int] = None) -> loop.DispatchContext:
    rules_list = _load_rules(rules_arg)
    return loop.DispatchContext(
        rules=rules_list,
        runaway=trace_mod.RunawayTracker(),
        budget_state=budget_mod.load(),
        emit=_build_emit(),
        max_depth=max_depth or trace_mod.DEFAULT_MAX_DEPTH,
    )


def _save_budget(dctx: loop.DispatchContext) -> None:
    try:
        budget_mod.save(dctx.budget_state)
    except Exception as e:
        sys.stderr.write(f"[dispatcher] budget save failed: {e}\n")


# ---- fire -------------------------------------------------------------

def cmd_fire(args: argparse.Namespace) -> int:
    """从 stdin / 文件读一条 envelope，dispatch 一次。"""
    if args.event_file == "-" or not args.event_file:
        raw = sys.stdin.read()
    else:
        raw = Path(args.event_file).expanduser().read_text(encoding="utf-8")
    if not raw.strip():
        sys.stderr.write("[dispatcher fire] no input\n")
        return 2
    try:
        event = json.loads(raw)
    except json.JSONDecodeError as e:
        sys.stderr.write(f"[dispatcher fire] invalid JSON: {e}\n")
        return 2
    if not isinstance(event, dict):
        sys.stderr.write("[dispatcher fire] event must be an object\n")
        return 2

    dctx = _build_dctx(args.rules, max_depth=args.max_depth)
    n = loop.dispatch_event(event, dctx)
    _save_budget(dctx)
    print(f"[dispatcher fire] dispatched {n}")
    return 0


# ---- replay -----------------------------------------------------------

def cmd_replay(args: argparse.Namespace) -> int:
    eid = args.event_id
    target_date: Optional[_dt.date] = None
    if args.date:
        target_date = _dt.date.fromisoformat(args.date)
    # 默认搜近 30 天
    dates: List[_dt.date] = []
    today = _dt.date.today()
    if target_date:
        dates.append(target_date)
    else:
        for delta in range(30):
            dates.append(today - _dt.timedelta(days=delta))
    found = None
    for d in dates:
        for r in journal.read_day(date=d):
            if r.get("event_id") == eid:
                found = r
                break
        if found:
            break
    if not found:
        sys.stderr.write(f"[dispatcher replay] event_id {eid} not found\n")
        return 2
    dctx = _build_dctx(args.rules, max_depth=args.max_depth)
    n = loop.dispatch_event(found, dctx)
    _save_budget(dctx)
    print(f"[dispatcher replay] dispatched {n}")
    return 0


# ---- test-rule（dry run）---------------------------------------------

def cmd_test_rule(args: argparse.Namespace) -> int:
    rs = _load_rules(args.rules)
    raw = (sys.stdin.read() if args.event_file == "-"
           else Path(args.event_file).read_text(encoding="utf-8"))
    event = json.loads(raw)
    matches = rules_mod.matches(rs, event)
    if not matches:
        print("[dispatcher test-rule] no match")
        return 0
    for m in matches:
        print(f"  rule={m.rule.name} runner={m.spec.runner}")
        print(f"  prompt={m.spec.prompt!r}")
    return 0


# ---- parser -----------------------------------------------------------

def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="roostery.dispatcher")
    sub = p.add_subparsers(dest="cmd", required=True)

    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--rules", help="rules.yaml 路径；默认 $FEISHU_HUB_HOME/rules.yaml")
    common.add_argument("--max-depth", type=int,
                        help="trace 链深度上限（默认 3）")

    p_fire = sub.add_parser("fire", parents=[common],
                            help="单条 envelope 触发（hook 直触用）")
    p_fire.add_argument("--event-file", default="-",
                        help='"-" 表示 stdin（默认）')
    p_fire.set_defaults(func=cmd_fire)

    p_replay = sub.add_parser("replay", parents=[common],
                              help="按 event_id 从 journal 重派")
    p_replay.add_argument("event_id")
    p_replay.add_argument("--date", help="YYYY-MM-DD，缩小搜索（默认搜近 30 天）")
    p_replay.set_defaults(func=cmd_replay)

    p_tr = sub.add_parser("test-rule", parents=[common],
                          help="干 run：只匹配规则不真派")
    p_tr.add_argument("--event-file", default="-")
    p_tr.set_defaults(func=cmd_test_rule)

    return p


def main(argv: Optional[List[str]] = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
