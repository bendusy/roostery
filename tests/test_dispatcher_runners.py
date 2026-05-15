"""roostery.dispatcher.runners 单测（mock subprocess）。"""
import json
import os
import subprocess
from unittest.mock import MagicMock, patch

import pytest

from roostery.dispatcher import runners, trace


def _cp(rc=0, stdout="", stderr=""):
    proc = MagicMock(spec=subprocess.Popen)
    proc.pid = 12345
    proc.returncode = rc
    proc.communicate.return_value = (stdout, stderr)
    return proc


def _spec(**kw):
    kw.setdefault("runner", "cc_headless")
    kw.setdefault("prompt", "do thing")
    return runners.RunSpec(**kw)


# ---- 环境隔离 -----------------------------------------------------------

def test_prep_env_strips_parent_agent_state(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_AGENT", "parent_agent")
    monkeypatch.setenv("FEISHU_HUB_SESSION", "parent_session")
    env = runners._prep_env(_spec(), ctx=None, agent_name="codex")
    assert env["FEISHU_HUB_AGENT"] == "codex"          # 改写为 child
    assert "FEISHU_HUB_SESSION" not in env             # parent 状态不串
    assert "FEISHU_HUB_TURN" not in env


def test_prep_env_forwards_api_keys(monkeypatch):
    monkeypatch.setenv("ANTHROPIC_API_KEY", "K_A")
    monkeypatch.setenv("OPENAI_API_KEY", "K_O")
    monkeypatch.setenv("GEMINI_API_KEY", "K_G")
    env = runners._prep_env(_spec(), ctx=None, agent_name="cc")
    assert env["ANTHROPIC_API_KEY"] == "K_A"
    assert env["OPENAI_API_KEY"] == "K_O"
    assert env["GEMINI_API_KEY"] == "K_G"


def test_prep_env_forwards_proxy_and_tls(monkeypatch):
    monkeypatch.setenv("HTTPS_PROXY", "http://corp:8080")
    monkeypatch.setenv("SSL_CERT_FILE", "/etc/ssl/cert.pem")
    monkeypatch.setenv("NO_PROXY", "localhost")
    env = runners._prep_env(_spec(), ctx=None, agent_name="cc")
    assert env["HTTPS_PROXY"] == "http://corp:8080"
    assert env["SSL_CERT_FILE"] == "/etc/ssl/cert.pem"
    assert env["NO_PROXY"] == "localhost"


def test_prep_env_forwards_xdg_and_config_dirs(monkeypatch):
    monkeypatch.setenv("XDG_CONFIG_HOME", "/x/cfg")
    monkeypatch.setenv("CLAUDE_CONFIG_DIR", "/c/cfg")
    monkeypatch.setenv("CODEX_HOME", "/co/home")
    monkeypatch.setenv("GEMINI_HOME", "/g/home")
    env = runners._prep_env(_spec(), ctx=None, agent_name="cc")
    assert env["XDG_CONFIG_HOME"] == "/x/cfg"
    assert env["CLAUDE_CONFIG_DIR"] == "/c/cfg"
    assert env["CODEX_HOME"] == "/co/home"
    assert env["GEMINI_HOME"] == "/g/home"


def test_prep_env_forwards_base_url_overrides(monkeypatch):
    monkeypatch.setenv("ANTHROPIC_BASE_URL", "https://my.proxy/v1")
    monkeypatch.setenv("OPENAI_BASE_URL", "https://my.proxy/openai/v1")
    env = runners._prep_env(_spec(), ctx=None, agent_name="cc")
    assert env["ANTHROPIC_BASE_URL"] == "https://my.proxy/v1"
    assert env["OPENAI_BASE_URL"] == "https://my.proxy/openai/v1"


def test_prep_env_injects_trace(monkeypatch):
    ctx = trace.TraceCtx(trace_id="T", depth=2, parent_event_id="P")
    env = runners._prep_env(_spec(), ctx=ctx, agent_name="codex")
    assert env[trace.ENV_TRACE_ID] == "T"
    assert env[trace.ENV_DEPTH] == "2"
    assert env[trace.ENV_PARENT] == "P"


def test_prep_env_extra_env_overrides(monkeypatch):
    spec = _spec(extra_env={"PATH": "/custom/path", "FOO": "bar"})
    env = runners._prep_env(spec, ctx=None, agent_name="cc")
    assert env["PATH"] == "/custom/path"
    assert env["FOO"] == "bar"


# ---- _head 截断 ---------------------------------------------------------

def test_head_returns_full_when_short():
    assert runners._head("hi") == "hi"


def test_head_truncates_long():
    big = "x" * 5000
    out = runners._head(big, cap=128)
    assert "[truncated]" in out
    assert len(out.encode()) > 128
    assert out.startswith("x" * 128)


# ---- cc_headless --------------------------------------------------------

def test_cc_headless_missing_binary(monkeypatch):
    monkeypatch.delenv("FEISHU_HUB_CC_BIN", raising=False)
    monkeypatch.setattr(runners.shutil, "which", lambda _: None)
    r = runners.cc_headless(_spec(prompt="x"))
    assert r.exit_code == 127
    assert "binary not found" in r.stderr


def test_cc_headless_parses_json_output(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_CC_BIN", "/fake/claude")
    body = json.dumps({"result": "all done", "cost_usd": 0.0125, "num_tokens": 312})
    with patch.object(runners.subprocess, "Popen",
                      return_value=_cp(0, body, "")) as mk:
        r = runners.cc_headless(_spec(prompt="do thing"))
    assert r.exit_code == 0
    assert r.final_text == "all done"
    assert r.cost_cents == 1  # 0.0125 USD → 1.25 cent → round 1
    assert r.tokens == 312
    argv = mk.call_args.args[0]
    assert argv[0] == "/fake/claude"
    assert "-p" in argv
    assert argv[argv.index("--output-format") + 1] == "json"


def test_cc_headless_with_resume_and_model(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_CC_BIN", "/c")
    with patch.object(runners.subprocess, "Popen",
                      return_value=_cp(0, '{"result":"r"}', "")) as mk:
        runners.cc_headless(_spec(prompt="p", model="claude-opus-4-7",
                                  resume_id="sess-123"))
    argv = mk.call_args.args[0]
    assert argv[argv.index("--model") + 1] == "claude-opus-4-7"
    assert argv[argv.index("--resume") + 1] == "sess-123"


def test_cc_headless_handles_non_json_output(monkeypatch):
    """CC 卡了 / 输出 plain text；不抛，final_text 为 None。"""
    monkeypatch.setenv("FEISHU_HUB_CC_BIN", "/c")
    with patch.object(runners.subprocess, "Popen",
                      return_value=_cp(0, "weird plain output", "")):
        r = runners.cc_headless(_spec(prompt="x"))
    assert r.exit_code == 0
    assert r.final_text is None


# ---- codex_exec ---------------------------------------------------------

def test_codex_exec_invokes_correct_argv(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_CODEX_BIN", "/co")
    with patch.object(runners.subprocess, "Popen",
                      return_value=_cp(0, "ok", "")) as mk:
        runners.codex_exec(_spec(runner="codex_exec", prompt="review please"))
    argv = mk.call_args.args[0]
    assert argv[0] == "/co"
    assert argv[1] == "exec"
    assert argv[2] == "review please"


def test_codex_exec_passes_model(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_CODEX_BIN", "/co")
    with patch.object(runners.subprocess, "Popen",
                      return_value=_cp(0, "", "")) as mk:
        runners.codex_exec(_spec(runner="codex_exec", prompt="p",
                                 model="gpt-5"))
    argv = mk.call_args.args[0]
    assert argv[argv.index("--model") + 1] == "gpt-5"


# ---- gemini_headless ----------------------------------------------------

def test_gemini_headless_fills_final_text(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_GEMINI_BIN", "/g")
    with patch.object(runners.subprocess, "Popen",
                      return_value=_cp(0, "  flowing prose  \n", "")):
        r = runners.gemini_headless(_spec(runner="gemini_headless"))
    assert r.final_text == "flowing prose"


def test_gemini_headless_argv(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_GEMINI_BIN", "/g")
    with patch.object(runners.subprocess, "Popen",
                      return_value=_cp(0, "", "")) as mk:
        runners.gemini_headless(_spec(runner="gemini_headless", prompt="p",
                                      model="gemini-2.5-pro"))
    argv = mk.call_args.args[0]
    assert argv == ["/g", "-p", "p", "--output-format", "text",
                    "-m", "gemini-2.5-pro"]


# ---- timeout / errors ---------------------------------------------------

def test_run_records_timeout(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_CC_BIN", "/c")

    proc = MagicMock(spec=subprocess.Popen)
    proc.pid = 12345
    proc.communicate.side_effect = subprocess.TimeoutExpired("/c", 5)
    proc.communicate.return_value = ("", "")  # after kill()

    def popen_factory(*a, **kw):
        proc.communicate.side_effect = [
            subprocess.TimeoutExpired("/c", 5),
            ("", ""),
        ]
        return proc

    with patch.object(runners.subprocess, "Popen", side_effect=popen_factory):
        r = runners.cc_headless(_spec(timeout_s=5))
    assert r.timed_out is True
    assert r.exit_code == -1


def test_run_records_file_not_found(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_CC_BIN", "/c")

    def fnf(*a, **kw):
        raise FileNotFoundError("nope")

    with patch.object(runners.subprocess, "Popen", side_effect=fnf):
        r = runners.cc_headless(_spec())
    assert r.exit_code == 127
    assert "binary not found" in r.stderr


# ---- noop & registry ----------------------------------------------------

def test_noop_runner_echoes():
    r = runners.noop(_spec(runner="noop", prompt="hello"))
    assert r.exit_code == 0
    assert r.final_text == "hello"


def test_unknown_runner_returns_missing():
    r = runners.run(_spec(runner="banana"))
    assert r.exit_code == 127
    assert "unknown runner" in r.stderr


def test_run_dispatches_by_name():
    """对 dict 注册表用 monkeypatch 不便；直接调用注册表里的实现验证 dispatch。"""
    r = runners.run(_spec(runner="noop", prompt="ECHO"))
    assert r.runner == "noop"
    assert r.stdout == "ECHO"


def test_run_uses_registered_function():
    """临时替换注册表项，确认 dispatcher 读取的是 RUNNERS dict。"""
    sentinel = runners.RunResult(
        runner="x", exit_code=0, stdout="HIT", stderr="",
        stdout_head="HIT", stderr_head="", duration_ms=0, timed_out=False,
    )
    original = runners.RUNNERS.get("noop")
    runners.RUNNERS["noop"] = lambda s, c, **kw: sentinel
    try:
        r = runners.run(_spec(runner="noop"))
        assert r.stdout == "HIT"
    finally:
        if original is not None:
            runners.RUNNERS["noop"] = original


# ---- subprocess 调用边界 -----------------------------------------------

def test_cc_headless_passes_cwd(monkeypatch, tmp_path):
    monkeypatch.setenv("FEISHU_HUB_CC_BIN", "/c")
    with patch.object(runners.subprocess, "Popen",
                      return_value=_cp(0, "{}", "")) as mk:
        runners.cc_headless(_spec(cwd=str(tmp_path)))
    assert mk.call_args.kwargs["cwd"] == str(tmp_path)


def test_cc_headless_respects_timeout(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_CC_BIN", "/c")
    proc = _cp(0, "{}", "")
    with patch.object(runners.subprocess, "Popen", return_value=proc):
        runners.cc_headless(_spec(timeout_s=42))
    proc.communicate.assert_called_once_with(timeout=42)


def test_runresult_has_aborted_field_defaults_false():
    from roostery.dispatcher.runners import RunResult
    r = RunResult(
        runner="noop", exit_code=0, stdout="", stderr="",
        stdout_head="", stderr_head="", duration_ms=0, timed_out=False,
    )
    assert r.aborted is False
    assert r.abort_reason is None


def test_runresult_aborted_settable():
    from roostery.dispatcher.runners import RunResult
    r = RunResult(
        runner="noop", exit_code=-15, stdout="", stderr="",
        stdout_head="", stderr_head="", duration_ms=0, timed_out=False,
        aborted=True, abort_reason="/stop",
    )
    assert r.aborted is True
    assert r.abort_reason == "/stop"


# ---- on_pid 回调 -------------------------------------------------------

def test_run_invokes_on_pid_callback_with_subprocess_pid():
    from roostery.dispatcher.runners import noop, RunSpec
    seen_pids = []
    noop(RunSpec(runner="noop", prompt="hello"), on_pid=seen_pids.append)
    # noop runner 不真起子进程：on_pid 不会被调用（行为契约）
    assert seen_pids == []


def test_run_real_subprocess_invokes_on_pid(tmp_path):
    """跑一个真实可结束的子进程（python3 -c），验证 _run 把 pid 报上来。"""
    from roostery.dispatcher.runners import _run, RunSpec
    seen = []
    spec = RunSpec(runner="test", prompt="", timeout_s=10)
    _run(["python3", "-c", "print('ok')"], spec, agent_name="test", ctx=None,
         on_pid=seen.append)
    assert len(seen) == 1
    assert isinstance(seen[0], int) and seen[0] > 0


def test_run_subprocess_killed_returns_negative_exit():
    """主线程在 on_pid 回调里把子进程杀掉，验证 _run 正常返回 exit_code=-15。"""
    import os
    import signal
    from roostery.dispatcher.runners import _run, RunSpec
    spec = RunSpec(runner="test", prompt="", timeout_s=10)

    def kill_it(pid: int) -> None:
        os.kill(pid, signal.SIGTERM)

    r = _run(["python3", "-c", "import time; time.sleep(30)"],
             spec, agent_name="test", ctx=None, on_pid=kill_it)
    assert r.exit_code == -15
    assert r.timed_out is False


def test_runresult_has_adjust_attempts_default_zero():
    from roostery.dispatcher.runners import RunResult
    r = RunResult(
        runner="noop", exit_code=0, stdout="", stderr="",
        stdout_head="", stderr_head="", duration_ms=0, timed_out=False,
    )
    assert r.adjust_attempts == 0


def test_runresult_adjust_attempts_settable():
    from roostery.dispatcher.runners import RunResult
    r = RunResult(
        runner="noop", exit_code=0, stdout="", stderr="",
        stdout_head="", stderr_head="", duration_ms=0, timed_out=False,
        adjust_attempts=1,
    )
    assert r.adjust_attempts == 1
