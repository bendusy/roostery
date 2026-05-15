"""roostery.__main__ 单测（init 子命令）。"""
import os
import stat
from pathlib import Path

import pytest

yaml = pytest.importorskip("yaml")

from roostery import __main__ as m  # noqa: E402
from roostery import config as cfgmod  # noqa: E402


def test_init_creates_dirs_and_config(monkeypatch, tmp_path):
    home = tmp_path / "fhub"
    monkeypatch.setenv(cfgmod.ENV_ROOT, str(home))
    monkeypatch.setattr(m, "_resolve_lark_cli", lambda shim_self=None: "/usr/bin/true")

    rc = m.main(["init", "--no-prompt", "--no-install-hooks", "--no-guide"])
    assert rc == 0

    assert (home / "journal").is_dir()
    assert (home / "state" / "reports").is_dir()
    assert (home / "bin").is_dir()
    cfg_path = home / "config.yaml"
    assert cfg_path.is_file()

    cfg = cfgmod.load(path=cfg_path, apply_env=False)
    assert cfg["shim"]["real_lark_cli"] == "/usr/bin/true"


def test_init_deploys_hook_script_executable(monkeypatch, tmp_path):
    home = tmp_path / "fhub"
    monkeypatch.setenv(cfgmod.ENV_ROOT, str(home))
    monkeypatch.setattr(m, "_resolve_lark_cli", lambda shim_self=None: "/usr/bin/true")

    m.main(["init", "--no-prompt", "--no-install-hooks", "--no-guide"])
    hook = home / "bin" / "agent-stop-notify.sh"
    assert hook.is_file()
    mode = os.stat(hook).st_mode
    assert mode & stat.S_IXUSR, "hook script must be executable"
    content = hook.read_text(encoding="utf-8")
    assert "python3 -m roostery.stop_hook" in content
    assert "FEISHU_NOTIFY_TO" in content


def test_init_preserves_existing_user_values(monkeypatch, tmp_path):
    home = tmp_path / "fhub"
    monkeypatch.setenv(cfgmod.ENV_ROOT, str(home))
    # 预置一份既有 config
    home.mkdir()
    cfgmod.save(
        {"notify_receive_id": "ou_existing",
         "daily_report": {"root_folder_token": "fldcnOLD"}},
        path=home / "config.yaml",
    )
    monkeypatch.setattr(m, "_resolve_lark_cli", lambda shim_self=None: "/usr/bin/true")

    m.main(["init", "--no-prompt", "--no-install-hooks", "--no-guide"])

    cfg = cfgmod.load(path=home / "config.yaml", apply_env=False)
    assert cfg["notify_receive_id"] == "ou_existing"
    assert cfg["daily_report"]["root_folder_token"] == "fldcnOLD"
    # real_lark_cli 应该被刷新
    assert cfg["shim"]["real_lark_cli"] == "/usr/bin/true"


def test_init_missing_lark_cli_with_flag_ok(monkeypatch, tmp_path):
    home = tmp_path / "fhub"
    monkeypatch.setenv(cfgmod.ENV_ROOT, str(home))

    def _raise(shim_self=None):
        raise RuntimeError("lark-cli not found on PATH")

    monkeypatch.setattr(m, "_resolve_lark_cli", _raise)

    rc = m.main(["init", "--no-prompt", "--no-install-hooks", "--no-guide", "--allow-missing-lark-cli"])
    assert rc == 0
    cfg = cfgmod.load(path=home / "config.yaml", apply_env=False)
    assert cfg["shim"]["real_lark_cli"] == ""


def test_init_missing_lark_cli_strict_fails(monkeypatch, tmp_path):
    home = tmp_path / "fhub"
    monkeypatch.setenv(cfgmod.ENV_ROOT, str(home))

    def _raise(shim_self=None):
        raise RuntimeError("lark-cli not found on PATH")

    monkeypatch.setattr(m, "_resolve_lark_cli", _raise)
    rc = m.main(["init", "--no-prompt", "--no-install-hooks", "--no-guide"])
    assert rc == 2


def test_shim_subcommand_runs(monkeypatch, tmp_path):
    home = tmp_path / "fhub"
    monkeypatch.setenv(cfgmod.ENV_ROOT, str(home))
    # 准备 fake lark-cli + config
    fake = tmp_path / "fake-lark-cli"
    fake.write_text("#!/usr/bin/env python3\nimport sys; sys.exit(0)\n")
    os.chmod(fake, 0o755)
    monkeypatch.setenv(cfgmod.ENV_REAL_LARK_CLI, str(fake))
    monkeypatch.setattr(os, "isatty", lambda fd: False)

    rc = m.main(["shim", "--", "im", "+messages-send"])
    assert rc == 0
