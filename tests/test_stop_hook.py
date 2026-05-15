from unittest.mock import patch

from roostery.stop_hook import run


@patch("roostery.stop_hook.task_writer")
def test_run_creates_task_and_appends(task_writer, capsys):
    task_writer.get_or_create_for_session.return_value = task_writer.TaskRef(
        guid="g-1", url="https://applink.feishu.cn/client/todo/detail?guid=g-1"
    )
    rc = run(
        agent="cc",
        session="sess-1",
        cwd="/repo/foo",
        summary="完成代码审查",
        assignee_open_id="ou_x",
    )
    assert rc == 0
    task_writer.get_or_create_for_session.assert_called_once()
    task_writer.append_steps.assert_called_once()


@patch("roostery.stop_hook.task_writer")
@patch("roostery.stop_hook._send_im_fallback")
def test_run_fallback_on_lark_cli_failure(send_im, task_writer):
    from roostery.lark_cli import LarkCLIError

    task_writer.get_or_create_for_session.side_effect = LarkCLIError(
        code=10000, msg="boom", argv=["x"]
    )
    rc = run(
        agent="cc",
        session="s",
        cwd="/r",
        summary="x",
        assignee_open_id="ou_x",
    )
    assert rc == 0  # 仍然 0，不阻塞 agent
    send_im.assert_called_once()


@patch("roostery.stop_hook.task_writer")
@patch("roostery.identity.resolve_user_open_id", return_value=None)
def test_run_no_follower_skips_task(resolve_uid, task_writer, capsys):
    """assignee 显式为空 + identity 也解不出 → 不调 task_writer，exit 0。"""
    rc = run(agent="cc", session="s", cwd="/r", summary="x", assignee_open_id="")
    assert rc == 0
    task_writer.get_or_create_for_session.assert_not_called()
