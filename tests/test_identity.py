"""roostery.identity 测试：mock lark-cli subprocess，验证 profile/auth 解析 + fallback 顺序。"""
from __future__ import annotations

import json
from unittest.mock import patch

import pytest

from roostery import identity as ident_mod
from roostery.identity import (
    Identity,
    current_identity,
    list_profiles,
    resolve_user_open_id,
)


def _mock_run(auth_payload=None, profiles_payload=None):
    """返回一个能模拟 lark-cli auth status + profile list 的 side_effect。"""
    def fn(argv, timeout=10):
        if argv[:2] == ["auth", "status"]:
            return auth_payload
        if argv[:2] == ["profile", "list"]:
            return profiles_payload
        return None
    return fn


def test_current_identity_full_active_profile(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_HOST", "M4")
    with patch.object(ident_mod, "_run_lark_cli") as mocked:
        mocked.side_effect = _mock_run(
            auth_payload={
                "appId": "cli_abc123",
                "userOpenId": "ou_xxx",
                "userName": "dustben",
                "brand": "feishu",
                "tokenStatus": "valid",
            },
            profiles_payload=[
                {"name": "cli_abc123", "active": True, "brand": "feishu"},
                {"name": "cli_def456", "active": False, "brand": "feishu"},
            ],
        )
        ident = current_identity()

    assert isinstance(ident, Identity)
    assert ident.profile_name == "cli_abc123"
    assert ident.user_open_id == "ou_xxx"
    assert ident.user_name == "dustben"
    assert ident.bot_app_id == "cli_abc123"
    assert ident.host == "M4"
    assert ident.is_ready is True
    assert ident.short_user == "dustben"
    assert ident.short_bot == "abc123"


def test_current_identity_no_lark_cli(monkeypatch):
    """lark-cli 未装/未配置时返回 host-only Identity，不抛。"""
    monkeypatch.setenv("FEISHU_HUB_HOST", "testhost")
    with patch.object(ident_mod, "_run_lark_cli", return_value=None):
        ident = current_identity()

    assert ident.profile_name is None
    assert ident.user_open_id is None
    assert ident.bot_app_id is None
    assert ident.host == "testhost"
    assert ident.is_ready is False
    assert ident.short_user == "anon"
    assert ident.short_bot == "no-bot"


def test_current_identity_token_invalid(monkeypatch):
    """token 过期时 is_ready=False。"""
    monkeypatch.setenv("FEISHU_HUB_HOST", "m")
    with patch.object(ident_mod, "_run_lark_cli") as mocked:
        mocked.side_effect = _mock_run(
            auth_payload={"appId": "cli_x", "userOpenId": "ou_x", "tokenStatus": "expired"},
            profiles_payload=[{"name": "cli_x", "active": True}],
        )
        ident = current_identity()

    assert ident.is_ready is False  # token 状态非 valid


def test_resolve_user_open_id_explicit_wins(monkeypatch):
    monkeypatch.setenv("FEISHU_NOTIFY_TO", "env_user")
    with patch.object(ident_mod, "_run_lark_cli") as m:
        m.side_effect = _mock_run(
            auth_payload={"userOpenId": "auth_user", "appId": "cli_x", "tokenStatus": "valid"},
            profiles_payload=[],
        )
        assert resolve_user_open_id(explicit="explicit_user") == "explicit_user"


def test_resolve_user_open_id_env_over_auth(monkeypatch):
    monkeypatch.setenv("FEISHU_NOTIFY_TO", "env_user")
    with patch.object(ident_mod, "_run_lark_cli") as m:
        m.side_effect = _mock_run(
            auth_payload={"userOpenId": "auth_user", "appId": "cli_x", "tokenStatus": "valid"},
            profiles_payload=[],
        )
        assert resolve_user_open_id() == "env_user"


def test_resolve_user_open_id_auth_when_no_env(monkeypatch):
    monkeypatch.delenv("FEISHU_NOTIFY_TO", raising=False)
    with patch.object(ident_mod, "_run_lark_cli") as m:
        m.side_effect = _mock_run(
            auth_payload={"userOpenId": "auth_user", "appId": "cli_x", "tokenStatus": "valid"},
            profiles_payload=[],
        )
        assert resolve_user_open_id() == "auth_user"


def test_resolve_user_open_id_none_when_all_empty(monkeypatch):
    monkeypatch.delenv("FEISHU_NOTIFY_TO", raising=False)
    with patch.object(ident_mod, "_run_lark_cli", return_value=None):
        # config.yaml notify_receive_id 也可能存在；mock 它返回空
        with patch("roostery.config.load", return_value={}):
            assert resolve_user_open_id() is None


def test_list_profiles_returns_array():
    with patch.object(ident_mod, "_run_lark_cli", return_value=[
        {"name": "a", "active": True}, {"name": "b", "active": False},
    ]):
        assert len(list_profiles()) == 2


def test_identity_describe_format():
    ident = Identity(
        profile_name="cli_abc",
        user_open_id="ou_xxx",
        user_name="bob",
        bot_app_id="cli_abc",
        brand="feishu",
        token_status="valid",
        host="laptop",
    )
    d = ident.describe()
    assert "profile=cli_abc" in d
    assert "user=bob" in d
    assert "host=laptop" in d
    assert d.startswith("✓")  # is_ready
