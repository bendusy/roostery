"""onboarding 测试：3 个引导任务 + identity gate + 失败隔离。"""
from __future__ import annotations

from unittest.mock import MagicMock, patch

import pytest

from roostery import onboarding


def _mock_identity_ready():
    """构造一个 is_ready=True 的 Identity。"""
    from roostery.identity import Identity
    return Identity(
        profile_name="cli_abc",
        user_open_id="ou_test_user",
        user_name="bob",
        bot_app_id="cli_abc",
        brand="feishu",
        token_status="valid",
        host="testhost",
    )


def _mock_identity_not_ready():
    from roostery.identity import Identity
    return Identity(
        profile_name=None, user_open_id=None, user_name=None,
        bot_app_id=None, brand=None, token_status="expired",
        host="testhost",
    )


def test_create_welcome_tasks_when_ready(capsys):
    with patch("roostery.onboarding.current_identity", return_value=_mock_identity_ready()), \
         patch("roostery.onboarding.task_writer") as tw:
        tw.get_or_create_for_session.return_value = tw.TaskRef(
            guid="g", url="https://applink.feishu.cn/client/todo/detail?guid=g"
        )
        refs = onboarding.create_welcome_tasks(base_url="https://feishu.cn/base/xxx")

    assert len(refs) == 3
    # 3 个任务都调了 get_or_create_for_session（assignee 应该是 mock identity 的 user_open_id）
    assert tw.get_or_create_for_session.call_count == 3
    for call in tw.get_or_create_for_session.call_args_list:
        assert call.kwargs["assignee_open_id"] == "ou_test_user"
        assert call.kwargs["agent"] == "roostery"


def test_skips_when_identity_not_ready(capsys):
    with patch("roostery.onboarding.current_identity", return_value=_mock_identity_not_ready()), \
         patch("roostery.onboarding.task_writer") as tw:
        refs = onboarding.create_welcome_tasks()

    assert refs == []
    tw.get_or_create_for_session.assert_not_called()
    err = capsys.readouterr().err
    assert "identity 不齐" in err


def test_welcome_task_contains_identity_in_description():
    """欢迎任务 description 应当含当前身份三元组（user / bot / host）。"""
    bp = onboarding._welcome_task("ou_xxx", "cli_yyy", "M5")
    assert "ou_xxx" in bp["description"]
    assert "cli_yyy" in bp["description"]
    assert "M5" in bp["description"]
    assert len(bp["steps"]) == 3


def test_try_it_task_has_empty_steps():
    """『跑一次 CC』任务故意 0 step，让 user 跑 CC 后体验 step 自动追加（在别的 task 上）。"""
    bp = onboarding._try_it_task()
    assert bp["steps"] == []
    # `claude -p` 至少出现在 summary 或 description 任意一处（实际写法可能是 summary
    # 里 `claude -p 'hi'`，description 里讲 Stop hook 流程）
    combined = f"{bp['summary']} {bp['description']}"
    assert "claude -p" in combined


def test_base_view_task_includes_url_when_provided():
    bp = onboarding._base_view_task(base_url="https://feishu.cn/base/myToken")
    assert "https://feishu.cn/base/myToken" in bp["description"]


def test_base_view_task_warns_when_no_url():
    bp = onboarding._base_view_task(base_url=None)
    assert "config.yaml" in bp["description"]
    assert "缺" in bp["description"]


def test_single_failure_does_not_block_others():
    """第 2 个任务建失败时，第 1 + 第 3 应该仍然完成。"""
    with patch("roostery.onboarding.current_identity", return_value=_mock_identity_ready()), \
         patch("roostery.onboarding.task_writer") as tw:

        def side_effect(**kwargs):
            session = kwargs.get("session", "")
            if session == "onboarding-try-it":
                raise RuntimeError("simulated lark-cli failure")
            return tw.TaskRef(guid="g_" + session, url=f"https://example/{session}")

        tw.get_or_create_for_session.side_effect = side_effect
        refs = onboarding.create_welcome_tasks()

    # 3 个任务尝试创建，2 个成功，1 个失败被隔离
    assert len(refs) == 2
