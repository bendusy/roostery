"""roostery.smoke 单测（mock subprocess）。"""
import json
import subprocess
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

yaml = pytest.importorskip("yaml")

from roostery import config as cfgmod  # noqa: E402
from roostery import smoke  # noqa: E402


@pytest.fixture
def fhub_home(monkeypatch, tmp_path):
    home = tmp_path / "fhub"
    monkeypatch.setenv(cfgmod.ENV_ROOT, str(home))
    cfgmod.save({"shim": {"real_lark_cli": "/path/to/lark-cli"}},
                path=home / "config.yaml")
    return home


def _cp(rc=0, stdout="", stderr=""):
    cp = MagicMock(spec=subprocess.CompletedProcess)
    cp.returncode = rc
    cp.stdout = stdout
    cp.stderr = stderr
    return cp


def test_all_ok_when_all_help_succeeds(fhub_home):
    with patch.object(smoke.subprocess, "run",
                      return_value=_cp(0, "=== Dry Run ===\n{}", "")):
        result = smoke.run()
    assert result["all_ok"] is True
    assert len(result["probes"]) == len(smoke.PROBES)
    for name, probe in result["probes"].items():
        assert probe["ok"], f"{name} should pass"


def test_persists_state_file(fhub_home):
    with patch.object(smoke.subprocess, "run", return_value=_cp(0, "=== Dry Run ===\n{}", "")):
        smoke.run()
    state = json.loads((fhub_home / "state" / "smoke.json").read_text())
    assert state["all_ok"] is True
    assert "probes" in state


def test_missing_binary_reports_not_ok(fhub_home):
    with patch.object(smoke.subprocess, "run",
                      side_effect=FileNotFoundError("nope")):
        result = smoke.run()
    assert result["all_ok"] is False
    for p in result["probes"].values():
        assert p["ok"] is False
        assert "binary not found" in p["reason"]


def test_timeout_reports_not_ok(fhub_home):
    with patch.object(smoke.subprocess, "run",
                      side_effect=subprocess.TimeoutExpired("x", 1)):
        result = smoke.run()
    assert result["all_ok"] is False
    for p in result["probes"].values():
        assert "timeout" in p["reason"]


def test_unknown_flag_is_detected(fhub_home):
    """smoke 改 dry-run 后能直接 catch 哑 flag / flag rename。"""
    with patch.object(smoke.subprocess, "run",
                      return_value=_cp(2, "", "Error: unknown flag: --format")):
        result = smoke.run()
    assert result["all_ok"] is False
    for p in result["probes"].values():
        assert "flag/command mismatch" in p["reason"]


def test_missing_dry_run_marker_marks_not_ok(fhub_home):
    """lark-cli 升级后 --dry-run 输出格式变化。"""
    with patch.object(smoke.subprocess, "run",
                      return_value=_cp(0, "Some unexpected format", "")):
        result = smoke.run()
    assert result["all_ok"] is False


def test_unrecognized_command_marks_not_ok(fhub_home):
    """lark-cli 升级后命令消失：unknown command。"""
    def fake_run(argv, **kw):
        if "+messages-send" in argv[1:]:
            return _cp(1, "", "Error: unknown command\n")
        return _cp(0, "=== Dry Run ===\n{}", "")

    with patch.object(smoke.subprocess, "run", side_effect=fake_run):
        result = smoke.run()
    assert result["all_ok"] is False
    assert result["probes"]["im_messages_send"]["ok"] is False
    assert result["probes"]["docs_create_v2"]["ok"] is True


def test_load_last_returns_none_when_absent(fhub_home):
    assert smoke.load_last() is None


def test_load_last_roundtrip(fhub_home):
    with patch.object(smoke.subprocess, "run", return_value=_cp(0, "=== Dry Run ===\n{}", "")):
        smoke.run()
    last = smoke.load_last()
    assert last is not None and last["all_ok"] is True


def test_ensure_ready_raises_when_never_run(fhub_home):
    with pytest.raises(RuntimeError, match="never run"):
        smoke.ensure_ready_or_raise()


def test_ensure_ready_raises_when_last_failed(fhub_home):
    with patch.object(smoke.subprocess, "run",
                      side_effect=FileNotFoundError("nope")):
        smoke.run()
    with pytest.raises(RuntimeError, match="failures"):
        smoke.ensure_ready_or_raise()


def test_ensure_ready_ok_when_last_passed(fhub_home):
    with patch.object(smoke.subprocess, "run", return_value=_cp(0, "=== Dry Run ===\n{}", "")):
        smoke.run()
    smoke.ensure_ready_or_raise()  # 不抛即通过


def test_binary_env_override(monkeypatch, fhub_home):
    monkeypatch.setenv("FEISHU_HUB_LARK_CLI_BIN", "/custom/path")
    with patch.object(smoke.subprocess, "run",
                      return_value=_cp(0, "=== Dry Run ===\n{}", "")) as mk:
        smoke.run()
    # 所有 probe 都用 /custom/path
    for call in mk.call_args_list:
        argv = call.args[0]
        assert argv[0] == "/custom/path"


def test_main_returns_zero_on_success(fhub_home, capsys):
    with patch.object(smoke.subprocess, "run", return_value=_cp(0, "=== Dry Run ===\n{}", "")):
        rc = smoke.main()
    assert rc == 0
    out = capsys.readouterr().out
    assert "all_ok" in out


def test_main_returns_one_on_failure(fhub_home, capsys):
    with patch.object(smoke.subprocess, "run",
                      side_effect=FileNotFoundError("nope")):
        rc = smoke.main()
    assert rc == 1
