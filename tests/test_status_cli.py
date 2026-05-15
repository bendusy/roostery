"""``python -m roostery status`` —— 看板视图。

输出区段：
1. Identity（profile / user / bot / host / token 状态）
2. bots.yaml 配置（每个 bot 一行）
3. M3.C 接力链状态（``state/m3c_chats/*.json``）
4. daemon 进程探测（仅显示是否有 ``roostery bot-bridge`` 在跑）
"""
from __future__ import annotations

import io
import json
from contextlib import redirect_stdout
from pathlib import Path
from types import SimpleNamespace

import pytest

yaml = pytest.importorskip("yaml")

from roostery import __main__ as fhmain
from roostery.task_writer import TaskRef


def _run_status(args_overrides=None) -> str:
    args = SimpleNamespace()
    if args_overrides:
        for k, v in args_overrides.items():
            setattr(args, k, v)
    buf = io.StringIO()
    with redirect_stdout(buf):
        fhmain.cmd_status(args)
    return buf.getvalue()


@pytest.fixture
def isolated_home(tmp_path, monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
    return tmp_path


def test_status_prints_identity_section(isolated_home, monkeypatch):
    from roostery import identity as ident
    fake = ident.Identity(
        profile_name="cli_test", user_open_id="ou_xxx_user",
        user_name="alice", bot_app_id="cli_test",
        brand="feishu", token_status="valid", host="ax",
    )
    monkeypatch.setattr(fhmain, "_status_identity", lambda: fake)
    out = _run_status()
    assert "Identity" in out
    assert "cli_test" in out
    assert "alice" in out


def test_status_prints_bots_section(isolated_home, monkeypatch):
    (isolated_home / "bots.yaml").write_text(
        "bots:\n"
        "  - app_id: cli_a\n"
        "    role: reviewer\n"
        "    mention_alias: 审核Bot\n"
        "    runner: cc_headless\n"
        "    default_cwd: /tmp/x\n"
        "    prompt_template: 'p'\n"
        "  - app_id: cli_b\n"
        "    role: scribe\n"
        "    mention_alias: 沉淀Bot\n"
        "    runner: cc_headless\n"
        "    default_cwd: /tmp/y\n"
        "    prompt_template: 'p'\n",
        encoding="utf-8",
    )
    monkeypatch.setattr(fhmain, "_status_identity", lambda: None)
    monkeypatch.setattr(fhmain, "_status_daemon_pids", lambda: [])
    out = _run_status()
    assert "reviewer" in out and "scribe" in out
    assert "审核Bot" in out and "沉淀Bot" in out


def test_status_prints_relay_chats_section(isolated_home, monkeypatch):
    chats_dir = isolated_home / "state" / "m3c_chats"
    chats_dir.mkdir(parents=True)
    (chats_dir / "oc_aaa.json").write_text(
        json.dumps({"guid": "g1", "url": "https://t/1"}), encoding="utf-8")
    (chats_dir / "oc_bbb.json").write_text(
        json.dumps({"guid": "g2", "url": "https://t/2"}), encoding="utf-8")
    monkeypatch.setattr(fhmain, "_status_identity", lambda: None)
    monkeypatch.setattr(fhmain, "_status_daemon_pids", lambda: [])
    out = _run_status()
    assert "M3.C" in out
    assert "g1" in out and "g2" in out
    # chat_id 末 8 字符短化
    assert "...oc_aaa" in out or "_aaa" in out or "oc_aaa" in out


def test_status_reports_no_daemon_when_pids_empty(isolated_home, monkeypatch):
    monkeypatch.setattr(fhmain, "_status_identity", lambda: None)
    monkeypatch.setattr(fhmain, "_status_daemon_pids", lambda: [])
    out = _run_status()
    assert "bot-bridge" in out.lower() or "daemon" in out.lower()
    assert "无" in out or "not running" in out.lower() or "0" in out


def test_status_reports_running_daemon(isolated_home, monkeypatch):
    monkeypatch.setattr(fhmain, "_status_identity", lambda: None)
    monkeypatch.setattr(fhmain, "_status_daemon_pids", lambda: [4242, 4243])
    out = _run_status()
    assert "4242" in out and "4243" in out
