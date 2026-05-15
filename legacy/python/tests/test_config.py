"""roostery.config 单测。"""
import os

import pytest

yaml = pytest.importorskip("yaml")

from roostery import config as cfgmod  # noqa: E402


def test_load_returns_defaults_when_missing(tmp_path):
    cfg = cfgmod.load(path=tmp_path / "nope.yaml", apply_env=False)
    assert cfg["notify_receive_id_type"] == "open_id"
    assert cfg["shim"]["interactive_verbs"] == ["login", "logout", "auth", "config"]
    assert cfg["daily_report"]["monthly_subfolder"] is True


def test_save_then_load_roundtrip(tmp_path):
    p = tmp_path / "config.yaml"
    cfg = cfgmod.load(path=p, apply_env=False)
    cfg["notify_receive_id"] = "ou_test"
    cfg["shim"]["real_lark_cli"] = "/tmp/lark-cli"
    cfgmod.save(cfg, path=p)
    cfg2 = cfgmod.load(path=p, apply_env=False)
    assert cfg2["notify_receive_id"] == "ou_test"
    assert cfg2["shim"]["real_lark_cli"] == "/tmp/lark-cli"
    # 默认字段仍存在（deep merge）
    assert cfg2["shim"]["stdout_head_bytes"] == 2048


def test_save_is_atomic(tmp_path):
    p = tmp_path / "config.yaml"
    cfgmod.save({"a": 1}, path=p)
    assert p.exists()
    assert not (tmp_path / "config.yaml.tmp").exists()


def test_env_override_real_lark_cli(tmp_path, monkeypatch):
    monkeypatch.setenv(cfgmod.ENV_REAL_LARK_CLI, "/from/env/lark-cli")
    cfg = cfgmod.load(path=tmp_path / "missing.yaml")
    assert cfg["shim"]["real_lark_cli"] == "/from/env/lark-cli"


def test_env_override_notify(tmp_path, monkeypatch):
    monkeypatch.setenv(cfgmod.ENV_NOTIFY_TO, "ou_env")
    cfg = cfgmod.load(path=tmp_path / "missing.yaml")
    assert cfg["notify_receive_id"] == "ou_env"


def test_partial_yaml_merged_with_defaults(tmp_path):
    p = tmp_path / "config.yaml"
    p.write_text("notify_receive_id: ou_partial\n", encoding="utf-8")
    cfg = cfgmod.load(path=p, apply_env=False)
    assert cfg["notify_receive_id"] == "ou_partial"
    # daily_report 子键不应被丢
    assert cfg["daily_report"]["monthly_subfolder"] is True
    assert cfg["shim"]["stdout_head_bytes"] == 2048


def test_root_dir_respects_env(monkeypatch, tmp_path):
    monkeypatch.setenv(cfgmod.ENV_ROOT, str(tmp_path))
    assert cfgmod.root_dir() == tmp_path
    assert cfgmod.config_path() == tmp_path / "config.yaml"
