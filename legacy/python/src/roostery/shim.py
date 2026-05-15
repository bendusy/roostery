"""lark-cli shim：流式 tee + TTY 直通 + journal envelope。

设计：``docs/FEISHU_OFFICE_HUB_DESIGN_V2.md`` §8。

关键约束：
- stdin/stdout/stderr 任一为 TTY，或 argv[0] 命中 ``interactive_verbs`` → ``os.execv`` 直通；
- 非交互：``Popen`` + 两个 pump 线程实时 tee；
- shim 与 real 路径相同 → 立即报错，防递归调用。
"""
from __future__ import annotations

import os
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any, Callable, Dict, Iterable, List, Optional, Sequence, Tuple

from . import config as cfgmod
from . import journal, redact, remoterefs

NO_JOURNAL_ENV = "FEISHU_HUB_NOJOURNAL"


def is_interactive(
    argv: Sequence[str],
    interactive_verbs: Iterable[str],
    *,
    isatty_fn: Optional[Callable[[int], bool]] = None,
) -> bool:
    """判断是否走 ``os.execv`` 直通路径。"""
    check = isatty_fn if isatty_fn is not None else os.isatty
    try:
        if any(check(fd) for fd in (0, 1, 2)):
            return True
    except OSError:
        pass
    if argv and argv[0] in set(interactive_verbs):
        return True
    if any(a in ("--interactive", "--repl", "-i") for a in argv):
        return True
    return False


def resolve_real_cli(cfg: Dict[str, Any], *, shim_path: str) -> str:
    """从 config + env 取真实 lark-cli 路径，做防递归校验。"""
    real = cfg.get("shim", {}).get("real_lark_cli", "") or ""
    if not real:
        raise RuntimeError(
            "roostery: shim.real_lark_cli not configured; "
            "run `python -m roostery init` or set FEISHU_HUB_REAL_LARK_CLI"
        )
    real_resolved = os.path.realpath(real)
    shim_resolved = os.path.realpath(shim_path)
    if real_resolved == shim_resolved:
        raise RuntimeError(
            f"roostery: real_lark_cli ({real}) resolves to shim itself; abort"
        )
    if not os.path.exists(real_resolved):
        raise RuntimeError(f"roostery: real_lark_cli not found: {real}")
    return real_resolved


class _Null:
    def write(self, _): return None
    def flush(self): return None


def _bin_stream(s):
    """取得二进制可写流；pytest capture 等没有 .buffer 时退化为 sink。"""
    buf = getattr(s, "buffer", None)
    if buf is not None:
        return buf
    return _Null()


def _pump(src, dst_bin, cap: int, sink: bytearray) -> None:
    """从 src 读出，同时写入 dst_bin 与 head sink。"""
    while True:
        chunk = src.read(4096)
        if not chunk:
            break
        try:
            dst_bin.write(chunk)
            dst_bin.flush()
        except (BrokenPipeError, ValueError):
            pass
        if len(sink) < cap:
            need = cap - len(sink)
            sink.extend(chunk[:need])


def run_non_interactive(
    real_cli: str,
    sub_argv: Sequence[str],
    *,
    stdout_head_cap: int,
    stderr_head_cap: int,
    stdin=None,
    stdout=None,
    stderr=None,
) -> Tuple[int, bytes, bytes, int]:
    """运行非交互命令，返回 (rc, stdout_head, stderr_head, duration_ms)。"""
    if stdin is None:
        stdin = sys.stdin
        # 被 pytest / 非 TTY 包裹时，sys.stdin 可能没有 fileno；
        # 这种情形对 lark-cli 等价于"无 stdin"，直接给 DEVNULL。
        try:
            stdin.fileno()
        except (AttributeError, OSError, ValueError):
            stdin = subprocess.DEVNULL
    out_bin = _bin_stream(stdout if stdout is not None else sys.stdout)
    err_bin = _bin_stream(stderr if stderr is not None else sys.stderr)

    t0 = time.time()
    proc = subprocess.Popen(
        [real_cli, *list(sub_argv)],
        stdin=stdin,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        bufsize=0,
    )
    stdout_head = bytearray()
    stderr_head = bytearray()
    t_out = threading.Thread(
        target=_pump,
        args=(proc.stdout, out_bin, stdout_head_cap, stdout_head),
    )
    t_err = threading.Thread(
        target=_pump,
        args=(proc.stderr, err_bin, stderr_head_cap, stderr_head),
    )
    t_out.start()
    t_err.start()
    rc = proc.wait()
    t_out.join()
    t_err.join()
    duration_ms = int((time.time() - t0) * 1000)
    return rc, bytes(stdout_head), bytes(stderr_head), duration_ms


def build_record(
    sub_argv: Sequence[str],
    rc: int,
    stdout_head: bytes,
    stderr_head: bytes,
    duration_ms: int,
    *,
    stdin_present: bool,
) -> Dict[str, Any]:
    argv_red, redacted = redact.scrub_argv(sub_argv)
    stdout_red = redact.scrub_text(stdout_head)
    stderr_red = redact.scrub_text(stderr_head)
    return {
        "event_type": "lark_cli.invoke",
        "source": "shim",
        "actor": journal.actor_from_env(),
        "cwd": os.getcwd(),
        "command": {
            "argv": argv_red,
            "duration_ms": duration_ms,
            "exit_code": rc,
        },
        "io": {
            "stdout_head": stdout_red,
            "stderr_head": stderr_red,
            "stdin_present": stdin_present,
            "tty": False,
        },
        "remote_refs": remoterefs.extract(list(sub_argv), stdout_head),
        "summary": None,
        "tags": journal.tags_from_env(),
        "privacy": {"redacted_fields": redacted, "no_journal_reason": None},
    }


def main(argv: Optional[Sequence[str]] = None) -> int:
    """shim 入口；返回 real lark-cli 的退出码（异常时返回 127）。"""
    if argv is None:
        argv = sys.argv
    sub_argv = list(argv[1:])
    shim_path = argv[0]

    try:
        cfg = cfgmod.load()
        real_cli = resolve_real_cli(cfg, shim_path=shim_path)
    except RuntimeError as e:
        sys.stderr.write(f"[roostery] {e}\n")
        return 127

    no_journal = os.getenv(NO_JOURNAL_ENV) == "1"
    interactive_verbs = cfg["shim"].get("interactive_verbs", [])

    if is_interactive(sub_argv, interactive_verbs):
        if not no_journal:
            try:
                journal.append_skipped(sub_argv, reason="interactive")
            except Exception as e:
                sys.stderr.write(f"[roostery] journal skipped-write failed: {e}\n")
        os.execv(real_cli, [real_cli, *sub_argv])  # 不返回
        return 0  # 仅为类型完整

    stdin_present = False
    try:
        stdin_present = not sys.stdin.isatty()
    except (AttributeError, ValueError):
        pass

    try:
        rc, out_head, err_head, dur = run_non_interactive(
            real_cli,
            sub_argv,
            stdout_head_cap=int(cfg["shim"]["stdout_head_bytes"]),
            stderr_head_cap=int(cfg["shim"]["stderr_head_bytes"]),
        )
    except FileNotFoundError:
        sys.stderr.write(f"[roostery] real lark-cli not found: {real_cli}\n")
        return 127

    if no_journal:
        try:
            journal.append_skipped(sub_argv, reason="env")
        except Exception as e:
            sys.stderr.write(f"[roostery] journal skipped-write failed: {e}\n")
    else:
        try:
            rec = build_record(sub_argv, rc, out_head, err_head, dur,
                               stdin_present=stdin_present)
            journal.append(rec)
        except Exception as e:
            sys.stderr.write(f"[roostery] journal write failed: {e}\n")

    return rc


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main(sys.argv))
