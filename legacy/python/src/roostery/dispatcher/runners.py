"""三家 agent CLI headless 调度封装。

每个 runner 接 ``RunSpec``、返回 ``RunResult``；子进程内部 env 注入 trace 上下文
（``trace.TraceCtx.to_env()``），让受派 agent 的 hook 把 trace 自动回填到 journal。

设计：``docs/FEISHU_HUB_DISPATCHER_DESIGN.md`` §6, §11
依赖：``shutil``（解析二进制）、``subprocess``（执行）；**不 import GA**。
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Dict, List, Mapping, Optional, Sequence

from . import trace as trace_mod


DEFAULT_TIMEOUT_S = 600         # 10 min；rules 可覆盖
TRUNCATE_HEAD_BYTES = 2048


# ---- 数据类 -------------------------------------------------------------

@dataclass(frozen=True)
class RunSpec:
    runner: str                                # cc_headless / codex_exec / gemini_headless
    prompt: str
    cwd: Optional[str] = None
    extra_env: Mapping[str, str] = field(default_factory=dict)
    timeout_s: int = DEFAULT_TIMEOUT_S
    model: Optional[str] = None
    # 续会话（per-runner 语义不同）
    resume_id: Optional[str] = None
    # 追加 argv，用于 rules 透传特殊 flag
    extra_argv: Sequence[str] = ()


@dataclass(frozen=True)
class RunResult:
    runner: str
    exit_code: int
    stdout: str
    stderr: str
    stdout_head: str
    stderr_head: str
    duration_ms: int
    timed_out: bool
    cost_cents: Optional[int] = None
    tokens: Optional[int] = None
    final_text: Optional[str] = None  # CC json 解析后的最终文本（如能拿到）
    aborted: bool = False
    abort_reason: Optional[str] = None
    adjust_attempts: int = 0


# ---- 内部 helper --------------------------------------------------------

def _binary(name: str, env_key: str) -> Optional[str]:
    return os.getenv(env_key) or shutil.which(name)


def _head(text: str, cap: int = TRUNCATE_HEAD_BYTES) -> str:
    if not text:
        return ""
    b = text.encode("utf-8", errors="replace")
    if len(b) <= cap:
        return text
    return b[:cap].decode("utf-8", errors="replace") + "\n... [truncated]"


# 允许 forward 的 env 列表 — 不整盘 copy，避免 FEISHU_HUB_AGENT/SESSION/TURN
# 这种父 hook 状态串到子 agent。
SAFE_ENV_FORWARD = (
    # POSIX 基本
    "USER", "LOGNAME", "SHELL", "TMPDIR",
    # XDG 规范（CC/Codex/Gemini 部分配置走 XDG）
    "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    # 代理
    "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY",
    "http_proxy", "https_proxy", "all_proxy", "no_proxy",
    # TLS / CA
    "SSL_CERT_FILE", "SSL_CERT_DIR", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE",
    # API keys（三家 + Google 通用）
    "ANTHROPIC_API_KEY", "OPENAI_API_KEY",
    "GEMINI_API_KEY", "GOOGLE_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
    # 自定义 base URL（公司内网代理常需要）
    "ANTHROPIC_BASE_URL", "OPENAI_BASE_URL",
    # 各家自定义配置目录（CC 0.4+ / Codex 0.130+ / Gemini 0.4+ 都暴露过）
    "CLAUDE_CONFIG_DIR", "ANTHROPIC_CONFIG_DIR",
    "CODEX_HOME", "CODEX_CONFIG_DIR",
    "GEMINI_HOME", "GEMINI_CONFIG_DIR",
)


def _prep_env(spec: RunSpec, ctx: Optional[trace_mod.TraceCtx],
              agent_name: str) -> Dict[str, str]:
    """构造干净 env：必备基础 + SAFE_ENV_FORWARD allowlist + trace + 调用方 extra。"""
    base = {
        "PATH": os.environ.get("PATH", "/usr/local/bin:/usr/bin:/bin"),
        "HOME": os.environ.get("HOME", ""),
        "LANG": os.environ.get("LANG", "en_US.UTF-8"),
        "TERM": os.environ.get("TERM", "dumb"),
    }
    for k in SAFE_ENV_FORWARD:
        if k in os.environ:
            base[k] = os.environ[k]
    base["FEISHU_HUB_AGENT"] = agent_name
    if ctx is not None:
        base.update(ctx.to_env())
    base.update(spec.extra_env)
    return base


def _run(cmd: Sequence[str], spec: RunSpec, agent_name: str,
         ctx: Optional[trace_mod.TraceCtx],
         on_pid: Optional[Callable[[int], None]] = None) -> RunResult:
    env = _prep_env(spec, ctx, agent_name)
    t0 = time.time()
    timed_out = False
    try:
        proc = subprocess.Popen(
            list(cmd),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
            cwd=spec.cwd,
        )
    except FileNotFoundError as e:
        duration_ms = int((time.time() - t0) * 1000)
        return RunResult(
            runner=spec.runner, exit_code=127, stdout="",
            stderr=f"binary not found: {e}", stdout_head="",
            stderr_head=f"binary not found: {e}",
            duration_ms=duration_ms, timed_out=False,
        )
    if on_pid is not None:
        try:
            on_pid(proc.pid)
        except Exception:
            pass  # 回调异常不影响 runner 主路径
    try:
        stdout, stderr = proc.communicate(timeout=spec.timeout_s)
        rc = proc.returncode
    except subprocess.TimeoutExpired:
        timed_out = True
        proc.kill()
        stdout, stderr = proc.communicate()
        rc = -1
    duration_ms = int((time.time() - t0) * 1000)
    return RunResult(
        runner=spec.runner,
        exit_code=rc,
        stdout=stdout or "",
        stderr=stderr or "",
        stdout_head=_head(stdout or ""),
        stderr_head=_head(stderr or ""),
        duration_ms=duration_ms,
        timed_out=timed_out,
    )


# ---- 具体 runner --------------------------------------------------------

def cc_headless(spec: RunSpec,
                ctx: Optional[trace_mod.TraceCtx] = None,
                *, on_pid: Optional[Callable[[int], None]] = None) -> RunResult:
    """``claude -p <prompt> --output-format json [--resume id]``。

    CC json 输出含 ``cost_usd`` / ``num_turns`` / ``result``，会解出 final_text + cost。
    """
    bin_ = _binary("claude", "FEISHU_HUB_CC_BIN")
    if not bin_:
        return _missing("cc_headless", "claude binary not found")
    cmd: List[str] = [bin_, "-p", spec.prompt, "--output-format", "json"]
    if spec.model:
        cmd += ["--model", spec.model]
    if spec.resume_id:
        cmd += ["--resume", spec.resume_id]
    cmd += list(spec.extra_argv)
    r = _run(cmd, spec, agent_name="cc", ctx=ctx, on_pid=on_pid)
    return _enrich_cc(r)


def _enrich_cc(r: RunResult) -> RunResult:
    if r.exit_code != 0 or not r.stdout.strip():
        return r
    try:
        body = json.loads(r.stdout)
    except json.JSONDecodeError:
        return r
    if not isinstance(body, dict):
        return r
    final = body.get("result") or body.get("text") or None
    cost_usd = body.get("cost_usd") or body.get("total_cost_usd")
    cost_cents = int(round(float(cost_usd) * 100)) if cost_usd is not None else None
    tokens = body.get("num_tokens") or body.get("total_tokens")
    return RunResult(
        runner=r.runner,
        exit_code=r.exit_code,
        stdout=r.stdout,
        stderr=r.stderr,
        stdout_head=r.stdout_head,
        stderr_head=r.stderr_head,
        duration_ms=r.duration_ms,
        timed_out=r.timed_out,
        cost_cents=cost_cents,
        tokens=int(tokens) if isinstance(tokens, (int, float)) else None,
        final_text=str(final) if final is not None else None,
    )


def codex_exec(spec: RunSpec,
               ctx: Optional[trace_mod.TraceCtx] = None,
               *, on_pid: Optional[Callable[[int], None]] = None) -> RunResult:
    """``codex exec <prompt>``。Codex CLI 0.130 无 hook，靠 wrapper 抓退出码。"""
    bin_ = _binary("codex", "FEISHU_HUB_CODEX_BIN")
    if not bin_:
        return _missing("codex_exec", "codex binary not found")
    cmd: List[str] = [bin_, "exec", spec.prompt]
    if spec.model:
        cmd += ["--model", spec.model]
    if spec.resume_id == "--last":
        cmd = [bin_, "resume", "--last"]
        # codex resume 不接 prompt；把 prompt 当 stdin 输入？暂仅支持新会话
    cmd += list(spec.extra_argv)
    return _run(cmd, spec, agent_name="codex", ctx=ctx, on_pid=on_pid)


def gemini_headless(spec: RunSpec,
                    ctx: Optional[trace_mod.TraceCtx] = None,
                    *, on_pid: Optional[Callable[[int], None]] = None) -> RunResult:
    """``gemini -p <prompt> --output-format text``。"""
    bin_ = _binary("gemini", "FEISHU_HUB_GEMINI_BIN")
    if not bin_:
        return _missing("gemini_headless", "gemini binary not found")
    cmd: List[str] = [bin_, "-p", spec.prompt, "--output-format", "text"]
    if spec.model:
        cmd += ["-m", spec.model]
    cmd += list(spec.extra_argv)
    r = _run(cmd, spec, agent_name="gemini", ctx=ctx, on_pid=on_pid)
    if r.exit_code == 0:
        r = RunResult(
            runner=r.runner, exit_code=r.exit_code, stdout=r.stdout,
            stderr=r.stderr, stdout_head=r.stdout_head, stderr_head=r.stderr_head,
            duration_ms=r.duration_ms, timed_out=r.timed_out,
            final_text=r.stdout.strip() or None,
        )
    return r


def noop(spec: RunSpec, ctx: Optional[trace_mod.TraceCtx] = None,
         *, on_pid: Optional[Callable[[int], None]] = None) -> RunResult:
    """测试/dry-run 用：不调子进程，回声 prompt 头部。"""
    text = spec.prompt
    return RunResult(
        runner="noop", exit_code=0, stdout=text, stderr="",
        stdout_head=_head(text), stderr_head="",
        duration_ms=0, timed_out=False, final_text=text,
    )


def _missing(runner: str, msg: str) -> RunResult:
    return RunResult(runner=runner, exit_code=127, stdout="",
                     stderr=msg, stdout_head="", stderr_head=msg,
                     duration_ms=0, timed_out=False)


# ---- 注册表 -------------------------------------------------------------

RUNNERS: Dict[str, Callable[..., RunResult]] = {
    "cc_headless": cc_headless,
    "codex_exec": codex_exec,
    "gemini_headless": gemini_headless,
    "noop": noop,
}


def run(spec: RunSpec,
        ctx: Optional[trace_mod.TraceCtx] = None,
        *, on_pid: Optional[Callable[[int], None]] = None) -> RunResult:
    fn = RUNNERS.get(spec.runner)
    if fn is None:
        return _missing(spec.runner, f"unknown runner: {spec.runner}")
    return fn(spec, ctx, on_pid=on_pid)
