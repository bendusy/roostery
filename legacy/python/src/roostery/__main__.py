"""``python -m roostery <subcmd>`` 入口。

子命令：
- ``init``：建目录、解析真实 lark-cli 路径、写默认 config.yaml、部署 hook 脚本。
- ``shim``：以模块形式跑 shim（等价于 ``python -m roostery.shim``，便于测试）。
"""
from __future__ import annotations

import argparse
import os
import shutil
import stat
import subprocess
import sys
from pathlib import Path
from typing import List, Optional

from . import config as cfgmod
from . import shim as shim_mod

TEMPLATES_DIR = Path(__file__).parent / "templates"


def _resolve_lark_cli(shim_self: Optional[Path] = None) -> str:
    """用 `command -v` + readlink -f 拿真实 lark-cli 路径，拒绝指向 shim。"""
    try:
        out = subprocess.check_output(
            ["/usr/bin/env", "bash", "-lc", "command -v lark-cli"],
            text=True,
        ).strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        out = ""
    if not out:
        raise RuntimeError(
            "lark-cli not found on PATH; install it first: "
            "https://github.com/larksuite/cli"
        )
    real = os.path.realpath(out)
    if shim_self and os.path.realpath(shim_self) == real:
        raise RuntimeError(
            f"lark-cli on PATH already points at shim itself ({real}); "
            "rename the shim first"
        )
    if not os.path.exists(real):
        raise RuntimeError(f"lark-cli resolved to non-existent path: {real}")
    return real


def _ensure_dirs(root: Path) -> None:
    for sub in ("journal", "state/reports", "bin"):
        (root / sub).mkdir(parents=True, exist_ok=True)


def _deploy_hook_script(root: Path) -> Path:
    src = TEMPLATES_DIR / "agent-stop-notify.sh"
    dst = root / "bin" / "agent-stop-notify.sh"
    shutil.copyfile(src, dst)
    mode = os.stat(dst).st_mode
    os.chmod(dst, mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return dst


def cmd_init(args: argparse.Namespace) -> int:
    from . import agent_detect, hooks_merge
    root = cfgmod.root_dir()
    _ensure_dirs(root)

    # 解析 real lark-cli
    real = ""
    try:
        real = _resolve_lark_cli()
    except RuntimeError as e:
        sys.stderr.write(f"[roostery init] {e}\n")
        if not args.allow_missing_lark_cli:
            return 2

    # 读现有 config，没有则用默认
    cfg = cfgmod.load(apply_env=False)
    if real:
        cfg["shim"]["real_lark_cli"] = real

    # 交互式补全（M1a：仅在 --no-prompt 未给时询问）
    if not args.no_prompt:
        if not cfg["notify_receive_id"]:
            cfg["notify_receive_id"] = _prompt(
                "飞书通知接收方 open_id（可留空，后续填）: "
            ).strip()
        if not cfg["daily_report"]["root_folder_token"]:
            cfg["daily_report"]["root_folder_token"] = _prompt(
                "日报根文件夹 folder_token（可留空，后续填）: "
            ).strip()

    path = cfgmod.save(cfg)
    hook = _deploy_hook_script(root)

    # M3.E：auto-detect 三家 AI agent CLI + 自动 hook（除非 --no-install-hooks）
    skip_list = [s.strip() for s in (args.skip_agent or "").split(",") if s.strip()]
    detect_results = agent_detect.detect_all(skip=skip_list)
    installed = agent_detect.installed_only(detect_results)

    hook_actions: List[str] = []
    if not args.no_install_hooks:
        for r in installed:
            try:
                hooks_merge.apply_template(
                    template_name=r.spec.template,
                    target_path=r.spec.hooks_target,
                    hook_script=str(hook),
                )
                hook_actions.append(f"  ✓ {r.spec.name:7} → {r.spec.hooks_target}")
            except Exception as e:    # noqa: BLE001
                hook_actions.append(f"  ✗ {r.spec.name:7}: {e}")

    print(f"[roostery init] root         = {root}")
    print(f"[roostery init] config       = {path}")
    print(f"[roostery init] real_lark_cli= {cfg['shim']['real_lark_cli'] or '(not set)'}")
    print(f"[roostery init] hook script  = {hook}")
    print(f"[roostery init] detected agents:")
    print(agent_detect.describe(detect_results))
    if hook_actions:
        print(f"[roostery init] hooks merged:")
        for line in hook_actions:
            print(line)
    elif args.no_install_hooks:
        print(f"[roostery init] hooks merge skipped (--no-install-hooks)")
    else:
        print(f"[roostery init] hooks merged: (无装的 agent；先装 claude / codex / gemini 再重跑 init)")
    print()

    # M3.E：装机末尾自动建飞书侧引导任务（产品式 onboarding）
    if not args.no_guide:
        try:
            from . import onboarding
            bt = cfg.get("bitable") or {}
            base_url = (
                f"https://feishu.cn/base/{bt.get('base_token')}?table={bt.get('table_id')}"
                if bt.get("base_token") and bt.get("table_id") else None
            )
            refs = onboarding.create_welcome_tasks(base_url=base_url)
            if refs:
                print(f"[roostery init] 引导任务已建到飞书 inbox（{len(refs)} 条），打开飞书"
                      f"\"我的待办\"即可看到。")
        except Exception as e:    # noqa: BLE001
            sys.stderr.write(f"[roostery init] onboarding 跳过：{e}\n")

    return 0


def _prompt(text: str) -> str:
    try:
        return input(text)
    except (EOFError, KeyboardInterrupt):
        return ""


def cmd_shim(args: argparse.Namespace) -> int:
    sub = list(args.argv or [])
    if sub and sub[0] == "--":
        sub = sub[1:]
    return shim_mod.main(["lark-cli", *sub])


def cmd_guide(args: argparse.Namespace) -> int:
    """在飞书侧建 3 个引导任务（产品式 onboarding）。可作 init 末尾自动 + 单独重跑。"""
    from . import onboarding

    cfg = cfgmod.load(apply_env=False) or {}
    bt = cfg.get("bitable") or {}
    base_token = (bt or {}).get("base_token")
    table_id = (bt or {}).get("table_id")
    base_url = (
        f"https://feishu.cn/base/{base_token}?table={table_id}"
        if base_token and table_id else None
    )

    refs = onboarding.create_welcome_tasks(base_url=base_url)
    if not refs:
        print("[guide] 0 个引导任务建成（identity 不齐？跑 `python -m roostery whoami` 看）")
        return 1
    print()
    print(f"完成：{len(refs)} 个引导任务已建到你飞书 inbox。打开飞书 → 我的待办 即可看到。")
    return 0


def cmd_indexer(args: argparse.Namespace) -> int:
    """从飞书 Task 列表反向刷出 Base 索引表（M3.D）。"""
    from . import base_indexer

    cfg = cfgmod.load(apply_env=False) or {}
    bt = cfg.get("bitable") or {}
    base_token = args.base_token or bt.get("base_token")
    table_id = args.table_id or bt.get("table_id")
    if not (base_token and table_id):
        print("[indexer] missing bitable.base_token / table_id in config.yaml; "
              "use --base-token / --table-id 显式传，或 `python -m roostery init` 先配。",
              file=sys.stderr)
        return 2

    if args.indexer_cmd == "migrate-schema":
        actions = base_indexer.ensure_schema(base_token=base_token, table_id=table_id)
        for name, act in actions.items():
            print(f"  {name:18} → {act}")
        bad = [n for n, a in actions.items() if a.startswith("failed") or a == "type_conflict"]
        return 1 if bad else 0

    if args.indexer_cmd == "status":
        run_info = base_indexer.load_last_run()
        cursor_us = base_indexer.load_cursor()
        if run_info:
            print(f"last_run: {run_info['started_at']} -> {run_info['finished_at']}")
            print(f"  full={run_info['full']}  succeeded={run_info['succeeded']}  "
                  f"skipped={run_info['skipped']}  failed={len(run_info['failed'])}")
            for f in run_info["failed"][:10]:
                print(f"    [fail] {f.get('guid', '?')[:8]} | {f.get('err', '')}")
        else:
            print("(尚未跑过 indexer)")
        print(f"cursor_us: {cursor_us}")
        return 0

    # run
    summary = base_indexer.run_indexer(
        base_token=base_token, table_id=table_id, full=args.full,
    )
    print(f"indexer run finished ({'full' if args.full else 'incremental'})")
    print(f"  succeeded={summary.succeeded}  skipped={summary.skipped}  "
          f"failed={len(summary.failed)}  total={summary.total}")
    for f in summary.failed[:10]:
        print(f"  [fail] {f.get('guid','?')[:8]} | {f.get('err','')}")

    # M4.C Phase 6: reconcile stale running rows on role Bases.
    # 失败 isolated（无 config/bases/*.yaml 时空 list；单个 Base 错走 LarkCLIError
    # 被 reconcile 内部 swallow），不影响 indexer 退出码。
    try:
        n = base_indexer.reconcile_stale_running()
        if n:
            print(f"  reconcile: fixed {n} stale running row(s)")
    except Exception as e:  # noqa: BLE001
        print(f"  [warn] reconcile_stale_running raised: {e}", file=sys.stderr)

    return 0 if not summary.failed else 1


def cmd_whoami(args: argparse.Namespace) -> int:
    """打印当前 roostery 解出的 (profile / user / bot / host) 身份。

    多 user / 多 bot 场景下用 ``lark-cli profile use <name>`` 切换后重跑本命令验证。
    """
    from . import identity as ident_mod
    ident = ident_mod.current_identity()
    print(ident.describe())
    if not ident.is_ready:
        print()
        print("[warn] identity 不完整。检查清单：")
        if not ident.profile_name:
            print("  - lark-cli profile list 无 active profile：跑 `lark-cli config init`")
        if not ident.bot_app_id:
            print("  - 无 bot appId：可能 profile 配置文件损坏")
        if not ident.user_open_id:
            print("  - 无 user open_id：跑 `lark-cli auth login --scope ...`")
        if ident.token_status and ident.token_status != "valid":
            print(f"  - token 状态={ident.token_status}：跑 `lark-cli auth login --domain ...` 刷新")

    # 显示所有 profile 帮助多 bot 切换
    profiles = ident_mod.list_profiles()
    if len(profiles) > 1:
        print()
        print(f"共 {len(profiles)} 个 profile（active 已标 ✓）：")
        for p in profiles:
            mark = "✓" if p.get("active") else " "
            print(f"  [{mark}] {p.get('name')} | brand={p.get('brand')} user={p.get('user','-')}")
        print("\n切换：lark-cli profile use <name>")
    return 0 if ident.is_ready else 1


def _status_identity():
    """testable hook：identity 解析；测试可 monkeypatch 返回 None / 假身份。"""
    from . import identity as ident_mod
    return ident_mod.current_identity()


def _status_daemon_pids() -> List[int]:
    """探测本机是否有 'roostery bot-bridge' daemon 进程在跑。失败返回 []。"""
    try:
        r = subprocess.run(
            ["pgrep", "-f", "roostery bot-bridge"],
            capture_output=True, text=True, timeout=3,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return []
    if r.returncode != 0:
        return []
    out: List[int] = []
    for line in r.stdout.splitlines():
        try:
            out.append(int(line.strip()))
        except ValueError:
            continue
    # 排除自己（status 命令本身的 pid 不算 daemon）
    me = os.getpid()
    return [p for p in out if p != me]


def cmd_status(args: argparse.Namespace) -> int:
    """看板：identity / bots.yaml / m3c 接力链 / daemon 进程。"""
    from . import bot_role, config

    # 1. Identity
    print("=== Identity ===")
    ident = _status_identity()
    if ident is None:
        print("  (skipped)")
    else:
        print(f"  {ident.describe()}")

    # 2. bots.yaml
    print("\n=== bots.yaml ===")
    bots_path = config.root_dir() / "bots.yaml"
    if not bots_path.exists():
        print(f"  (无 {bots_path}；cp roostery/templates/bots.yaml.tmpl {bots_path})")
    else:
        try:
            bots = bot_role.load_bots(bots_path)
        except bot_role.BotRoleConfigError as e:
            print(f"  [err] {e}")
            bots = []
        if not bots:
            print("  (空)")
        for b in bots:
            tag = f" → {b.next_bot_mention}" if b.next_bot_mention else ""
            print(f"  - role={b.role:<10} mention={b.mention_alias:<10} "
                  f"app={b.app_id[-12:]} runner={b.runner}{tag}")

    # 3. M3.C 接力链 cache
    print("\n=== M3.C 接力链（state/m3c_chats）===")
    chats_dir = config.root_dir() / "state" / "m3c_chats"
    if not chats_dir.exists():
        print("  (无)")
    else:
        import json as _json
        entries = sorted(chats_dir.glob("*.json"))
        if not entries:
            print("  (无)")
        for p in entries:
            try:
                data = _json.loads(p.read_text(encoding="utf-8"))
                short = p.stem[-12:]
                print(f"  - chat=...{short}  task={data.get('guid','?')}  {data.get('url','')}")
            except (OSError, ValueError):
                continue

    # 4. daemon 进程
    print("\n=== bot-bridge daemon ===")
    pids = _status_daemon_pids()
    if not pids:
        print("  无进程在跑")
    else:
        print(f"  {len(pids)} 个 daemon：pid={pids}")

    return 0


def cmd_bot_bridge(args: argparse.Namespace) -> int:
    """启动单 bot daemon：消费 IM 事件 → 路由到 runner → thread reply（M3.C）。"""
    from . import bot_bridge, bot_role, config

    bots_path = Path(args.bots_file) if args.bots_file else (config.root_dir() / "bots.yaml")
    try:
        bots = bot_role.load_bots(bots_path)
    except bot_role.BotRoleConfigError as e:
        print(f"[err] bots.yaml 解析失败：{e}", file=sys.stderr)
        return 2

    if not bots:
        print(f"[err] 未找到任何 bot 配置：{bots_path}", file=sys.stderr)
        print("    cp docs/examples/bots.yaml.example ~/.feishu_hub/bots.yaml", file=sys.stderr)
        return 2

    # 按 --role 或 --app-id 选 bot
    selected = None
    for b in bots:
        if args.role and b.role == args.role:
            selected = b
            break
        if args.app_id and b.app_id == args.app_id:
            selected = b
            break
    if selected is None and not (args.role or args.app_id) and len(bots) == 1:
        selected = bots[0]
    if selected is None:
        print(f"[err] 多个 bot，需指定 --role 或 --app-id；可选：", file=sys.stderr)
        for b in bots:
            print(f"  - role={b.role} app_id={b.app_id}", file=sys.stderr)
        return 2

    print(f"[bot-bridge] start role={selected.role} app_id={selected.app_id} "
          f"timeout={args.timeout or 'none'} max_events={args.max_events or 'none'}",
          file=sys.stderr)
    n = 0
    for action in bot_bridge.run_bot(
        selected,
        max_events=args.max_events,
        timeout=args.timeout,
        parallel=not args.sequential,
    ):
        n += 1
        print(f"[bot-bridge] #{n} src={action.source_message_id} "
              f"reply={action.reply_message_id} exit={action.runner_exit_code} "
              f"timeout={action.timed_out}", file=sys.stderr)
    print(f"[bot-bridge] done, total={n}", file=sys.stderr)
    return 0


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="roostery")
    sub = p.add_subparsers(dest="cmd", required=True)

    p_init = sub.add_parser("init", help="初始化 ~/.feishu_hub")
    p_init.add_argument("--no-prompt", action="store_true",
                        help="跳过交互式提示，所有空字段保留为空")
    p_init.add_argument("--allow-missing-lark-cli", action="store_true",
                        help="即使本机未装 lark-cli 也不报错（仅用于 CI/测试）")
    p_init.add_argument("--install-hooks", action="store_true",
                        help="[DEPRECATED] 默认就 auto-detect 装 hook；保留兼容旧脚本")
    p_init.add_argument("--no-install-hooks", action="store_true",
                        help="跳过 hook 合并（即使检测到 agent）")
    p_init.add_argument("--skip-agent", default="",
                        help="逗号分隔的 agent 名（cc/codex/gemini），跳过自动 hook，如 --skip-agent codex,gemini")
    p_init.add_argument("--no-guide", action="store_true",
                        help="跳过末尾在飞书侧建引导任务")
    p_init.add_argument("--cc-settings", default="~/.claude/settings.json",
                        help="[DEPRECATED] 现在用 agent_detect.AGENTS 表，不再通过 flag 覆盖")
    p_init.add_argument("--codex-hooks", default="~/.codex/hooks.json",
                        help="[DEPRECATED] 同上")
    p_init.set_defaults(func=cmd_init)

    p_shim = sub.add_parser("shim", help="以模块形式运行 shim")
    p_shim.add_argument("argv", nargs=argparse.REMAINDER)
    p_shim.set_defaults(func=cmd_shim)

    p_whoami = sub.add_parser(
        "whoami",
        help="打印当前 roostery 身份（profile / user / bot / host）—— 多 user/多 bot 切换后用这个确认",
    )
    p_whoami.set_defaults(func=cmd_whoami)

    p_guide = sub.add_parser(
        "guide",
        help="在你飞书 inbox 自动建 3 个引导任务（init 末尾自动跑；可单独 re-run）",
    )
    p_guide.set_defaults(func=cmd_guide)

    p_indexer = sub.add_parser(
        "indexer",
        help="从飞书 Task 列表反向刷出 Base 索引表（M3.D）",
    )
    indexer_sub = p_indexer.add_subparsers(dest="indexer_cmd", required=True)
    p_indexer_run = indexer_sub.add_parser("run", help="拉 task → upsert Base 行")
    p_indexer_run.add_argument("--full", action="store_true",
                               help="全量校准（不用 cursor）")
    p_indexer_run.add_argument("--base-token", default="",
                               help="覆盖 config.yaml.bitable.base_token")
    p_indexer_run.add_argument("--table-id", default="",
                               help="覆盖 config.yaml.bitable.table_id")
    indexer_sub.add_parser("status", help="看上次跑的 last_run.json + cursor")
    indexer_sub.add_parser("migrate-schema",
                           help="幂等建 M3.D 所需 Base 字段（task_guid / host / ...）")
    p_indexer.set_defaults(func=cmd_indexer, base_token="", table_id="", full=False)

    p_bot = sub.add_parser(
        "bot-bridge",
        help="启动单 bot daemon：监听 IM @mention → 调本机 runner → thread reply (M3.C)",
    )
    p_bot.add_argument("--role", default="",
                       help="按 role 选 bot（如 reviewer / scribe）；与 --app-id 二选一")
    p_bot.add_argument("--app-id", default="",
                       help="按 lark-cli profile / bot app_id 精确选；优先级低于 --role")
    p_bot.add_argument("--bots-file", default="",
                       help="bots.yaml 路径，默认 ~/.feishu_hub/bots.yaml")
    p_bot.add_argument("--timeout", default="",
                       help="lark-cli event consume --timeout（如 5m / 30s）；空=不限")
    p_bot.add_argument("--max-events", type=int, default=0,
                       help="lark-cli event consume --max-events；0=不限")
    p_bot.add_argument("--sequential", action="store_true",
                       help="同步模式：handle_event 阻塞主循环（默认 parallel，让 R5 HITL /stop 即时响应）")
    p_bot.set_defaults(func=cmd_bot_bridge)

    p_status = sub.add_parser(
        "status",
        help="看板：identity / bots.yaml / M3.C 接力链 / bot-bridge daemon 进程",
    )
    p_status.set_defaults(func=cmd_status)

    return p


def main(argv: Optional[List[str]] = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
