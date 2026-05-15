"""运行时解耦 smoke：clean 子进程 import roostery 不应拉起 GA 任何模块。

兜住 AST 检查抓不到的情形（``importlib.import_module``、``exec`` 动态加载、
顶层副作用偷偷 ``__import__`` 等）。
"""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent / 'src'
FORBIDDEN = {"ga", "agent_loop", "mykey", "bbs", "frontends",
             "agentmain", "reflect"}


def _run_in_clean_python(snippet: str) -> dict:
    proc = subprocess.run(
        [sys.executable, "-I", "-c", snippet],
        cwd=str(PROJECT_ROOT),
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert proc.returncode == 0, (
        f"clean-python snippet failed: stderr={proc.stderr!r} stdout={proc.stdout!r}"
    )
    return json.loads(proc.stdout)


_SNIPPET = (
    "import sys; "
    "sys.path.insert(0, %r); "
    "import roostery.{module}; "
    "import json; "
    "print(json.dumps(sorted(k for k in sys.modules if k.split('.')[0] in {forbidden})))"
)


def _check_module(name: str) -> list:
    snippet = (
        f"import sys\n"
        f"sys.path.insert(0, {str(PROJECT_ROOT)!r})\n"
        f"import roostery.{name}\n"
        f"import json\n"
        f"print(json.dumps(sorted(k for k in sys.modules "
        f"if k.split('.')[0] in {sorted(FORBIDDEN)!r})))\n"
    )
    return _run_in_clean_python(snippet)


def test_journal_does_not_pull_ga():
    leaked = _check_module("journal")
    assert leaked == [], f"journal import leaks GA modules: {leaked}"


def test_redact_does_not_pull_ga():
    assert _check_module("redact") == []


def test_remoterefs_does_not_pull_ga():
    assert _check_module("remoterefs") == []


def test_config_does_not_pull_ga():
    assert _check_module("config") == []


def test_shim_does_not_pull_ga():
    assert _check_module("shim") == []


def test_lark_cli_does_not_pull_ga():
    assert _check_module("lark_cli") == []


def test_git_log_does_not_pull_ga():
    assert _check_module("git_log") == []


def test_llm_summary_import_does_not_pull_ga():
    """llm_summary 即便被 import，也只在调 make_ga_summarizer() 时才尝试 GA。"""
    assert _check_module("llm_summary") == []


def test_main_does_not_pull_ga():
    assert _check_module("__main__") == []
