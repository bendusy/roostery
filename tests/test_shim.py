"""roostery.shim 单测。

策略：mock 真实 lark-cli 用一段 Python 子进程脚本承担，验证：
- 退出码透传；
- stdout/stderr tee 到外层 io；
- head 字节抓取与截断；
- TTY/interactive 检测分支；
- 防递归 guard；
- journal envelope 写出。
"""
import io
import json
import os
import stat
import subprocess
import sys
from pathlib import Path

import pytest

from roostery import shim


@pytest.fixture
def fake_lark_cli(tmp_path):
    """造一个可执行的"假 lark-cli"，行为由 argv 控制。"""
    script = tmp_path / "fake-lark-cli"
    script.write_text(
        "#!/usr/bin/env python3\n"
        "import sys, os\n"
        "args = sys.argv[1:]\n"
        "if '--echo-stdin' in args:\n"
        "    data = sys.stdin.read()\n"
        "    sys.stdout.write(data); sys.stdout.flush()\n"
        "if '--out' in args:\n"
        "    i = args.index('--out')\n"
        "    sys.stdout.write(args[i+1]); sys.stdout.flush()\n"
        "if '--err' in args:\n"
        "    i = args.index('--err')\n"
        "    sys.stderr.write(args[i+1]); sys.stderr.flush()\n"
        "if '--rc' in args:\n"
        "    i = args.index('--rc'); sys.exit(int(args[i+1]))\n"
        "sys.exit(0)\n",
        encoding="utf-8",
    )
    mode = os.stat(script).st_mode
    os.chmod(script, mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return script


# --- is_interactive ---------------------------------------------------------

def test_is_interactive_when_tty(monkeypatch):
    monkeypatch.setattr(os, "isatty", lambda fd: fd == 1)
    assert shim.is_interactive(["docs", "+create"], ["login"]) is True


def test_is_interactive_by_verb(monkeypatch):
    monkeypatch.setattr(os, "isatty", lambda fd: False)
    assert shim.is_interactive(["login"], ["login", "auth"]) is True
    assert shim.is_interactive(["auth", "status"], ["login", "auth"]) is True


def test_is_interactive_explicit_flag(monkeypatch):
    monkeypatch.setattr(os, "isatty", lambda fd: False)
    assert shim.is_interactive(["foo", "--interactive"], []) is True
    assert shim.is_interactive(["repl"], []) is False  # 不在 verbs 里就不算


def test_non_interactive_pipe_run(monkeypatch):
    monkeypatch.setattr(os, "isatty", lambda fd: False)
    assert shim.is_interactive(["docs", "+create"], ["login"]) is False


# --- resolve_real_cli -------------------------------------------------------

def test_resolve_real_cli_ok(fake_lark_cli, tmp_path):
    cfg = {"shim": {"real_lark_cli": str(fake_lark_cli)}}
    shim_path = tmp_path / "shim"
    shim_path.write_text("# fake")
    real = shim.resolve_real_cli(cfg, shim_path=str(shim_path))
    assert real == os.path.realpath(str(fake_lark_cli))


def test_resolve_real_cli_recursion_guard(fake_lark_cli, tmp_path):
    cfg = {"shim": {"real_lark_cli": str(fake_lark_cli)}}
    with pytest.raises(RuntimeError, match="resolves to shim itself"):
        shim.resolve_real_cli(cfg, shim_path=str(fake_lark_cli))


def test_resolve_real_cli_missing():
    cfg = {"shim": {"real_lark_cli": ""}}
    with pytest.raises(RuntimeError, match="not configured"):
        shim.resolve_real_cli(cfg, shim_path="/x")


def test_resolve_real_cli_nonexistent(tmp_path):
    cfg = {"shim": {"real_lark_cli": str(tmp_path / "missing")}}
    with pytest.raises(RuntimeError, match="not found"):
        shim.resolve_real_cli(cfg, shim_path="/x")


# --- run_non_interactive ----------------------------------------------------

def test_run_passes_stdout(fake_lark_cli):
    out = io.BytesIO()
    err = io.BytesIO()
    rc, head_out, head_err, _ = shim.run_non_interactive(
        str(fake_lark_cli), ["--out", "hello world"],
        stdout_head_cap=1024, stderr_head_cap=1024,
        stdin=subprocess.DEVNULL,
        stdout=_FakeStream(out), stderr=_FakeStream(err),
    )
    assert rc == 0
    assert head_out == b"hello world"
    assert out.getvalue() == b"hello world"


def test_run_truncates_head_but_full_tee(fake_lark_cli):
    big = "x" * 5000
    out = io.BytesIO()
    err = io.BytesIO()
    rc, head_out, _, _ = shim.run_non_interactive(
        str(fake_lark_cli), ["--out", big],
        stdout_head_cap=128, stderr_head_cap=128,
        stdin=subprocess.DEVNULL,
        stdout=_FakeStream(out), stderr=_FakeStream(err),
    )
    assert rc == 0
    assert len(head_out) == 128
    assert out.getvalue() == big.encode()


def test_run_returns_exit_code(fake_lark_cli):
    out = io.BytesIO(); err = io.BytesIO()
    rc, _, _, _ = shim.run_non_interactive(
        str(fake_lark_cli), ["--rc", "7"],
        stdout_head_cap=64, stderr_head_cap=64,
        stdin=subprocess.DEVNULL,
        stdout=_FakeStream(out), stderr=_FakeStream(err),
    )
    assert rc == 7


def test_run_separates_stdout_stderr(fake_lark_cli):
    out = io.BytesIO(); err = io.BytesIO()
    _, head_out, head_err, _ = shim.run_non_interactive(
        str(fake_lark_cli), ["--out", "OK", "--err", "WARN"],
        stdout_head_cap=64, stderr_head_cap=64,
        stdin=subprocess.DEVNULL,
        stdout=_FakeStream(out), stderr=_FakeStream(err),
    )
    assert head_out == b"OK"
    assert head_err == b"WARN"
    assert out.getvalue() == b"OK"
    assert err.getvalue() == b"WARN"


def test_run_stdin_passthrough(fake_lark_cli):
    out = io.BytesIO(); err = io.BytesIO()
    rd, wr = os.pipe()
    os.write(wr, b"markdown body\n"); os.close(wr)
    _, head_out, _, _ = shim.run_non_interactive(
        str(fake_lark_cli), ["--echo-stdin"],
        stdout_head_cap=64, stderr_head_cap=64,
        stdin=os.fdopen(rd, "rb"),
        stdout=_FakeStream(out), stderr=_FakeStream(err),
    )
    assert head_out == b"markdown body\n"


# --- build_record -----------------------------------------------------------

def test_build_record_shape(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_AGENT", "cc")
    monkeypatch.setenv("FEISHU_HUB_TAGS", "task_done")
    body = json.dumps({"data": {"message_id": "om_x"}}).encode()
    rec = shim.build_record(
        ["im", "+messages-send", "--user-id", "ou_x", "--text", "hi"],
        rc=0, stdout_head=body, stderr_head=b"",
        duration_ms=42, stdin_present=False,
    )
    assert rec["event_type"] == "lark_cli.invoke"
    assert rec["actor"]["agent"] == "cc"
    assert rec["tags"] == ["task_done"]
    assert rec["remote_refs"]["message_id"] == "om_x"
    assert rec["command"]["exit_code"] == 0
    assert rec["command"]["duration_ms"] == 42
    assert rec["privacy"]["no_journal_reason"] is None


def test_build_record_redacts_sensitive_argv():
    rec = shim.build_record(
        ["call", "--access-token", "supersecret", "--user", "x"],
        rc=0, stdout_head=b"", stderr_head=b"",
        duration_ms=1, stdin_present=False,
    )
    assert rec["command"]["argv"][2] == "***"
    assert "argv[2]" in rec["privacy"]["redacted_fields"]


# --- main() end-to-end -------------------------------------------------------

def test_main_writes_journal(monkeypatch, tmp_path, fake_lark_cli):
    home = tmp_path / "fhub"
    monkeypatch.setenv("FEISHU_HUB_HOME", str(home))
    monkeypatch.setenv("FEISHU_HUB_REAL_LARK_CLI", str(fake_lark_cli))
    monkeypatch.setenv("FEISHU_HUB_AGENT", "cc")
    monkeypatch.setattr(os, "isatty", lambda fd: False)

    rc = shim.main(["lark-cli", "im", "+messages-send", "--user-id", "ou_x",
                    "--out", '{"data":{"message_id":"om_test"}}'])
    assert rc == 0

    files = list((home / "journal").iterdir())
    assert len(files) == 1
    lines = files[0].read_text(encoding="utf-8").splitlines()
    assert len(lines) == 1
    rec = json.loads(lines[0])
    assert rec["event_type"] == "lark_cli.invoke"
    assert rec["actor"]["agent"] == "cc"
    assert rec["remote_refs"]["message_id"] == "om_test"
    assert rec["command"]["exit_code"] == 0


def test_main_skipped_on_nojournal(monkeypatch, tmp_path, fake_lark_cli):
    home = tmp_path / "fhub"
    monkeypatch.setenv("FEISHU_HUB_HOME", str(home))
    monkeypatch.setenv("FEISHU_HUB_REAL_LARK_CLI", str(fake_lark_cli))
    monkeypatch.setenv("FEISHU_HUB_NOJOURNAL", "1")
    monkeypatch.setattr(os, "isatty", lambda fd: False)

    shim.main(["lark-cli", "--out", "hi"])
    files = list((home / "journal").iterdir())
    assert len(files) == 1
    rec = json.loads(files[0].read_text(encoding="utf-8").splitlines()[0])
    assert rec["event_type"] == "lark_cli.skipped"
    assert rec["privacy"]["no_journal_reason"] == "env"


def test_main_returns_127_when_real_missing(monkeypatch, tmp_path):
    home = tmp_path / "fhub"
    monkeypatch.setenv("FEISHU_HUB_HOME", str(home))
    monkeypatch.setenv("FEISHU_HUB_REAL_LARK_CLI", str(tmp_path / "nope"))
    rc = shim.main(["lark-cli", "x"])
    assert rc == 127


# --- helpers ----------------------------------------------------------------

class _FakeStream:
    """模拟 sys.stdout：提供 .buffer 属性。"""
    def __init__(self, buf: io.BytesIO):
        self.buffer = buf
