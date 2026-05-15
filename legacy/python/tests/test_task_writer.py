"""task_writer TDD：bot 创建 task + append steps，全 mock lark_cli。"""
from __future__ import annotations

from unittest.mock import patch

import pytest

from roostery.task_writer import (
    TaskRef,
    append_steps,
    create_task,
)


@patch("roostery.task_writer.run_json")
def test_create_task_calls_lark_cli_as_bot(run_json, monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_HOST", "testhost")
    run_json.return_value = {
        "guid": "abc-123",
        "url": "https://applink.feishu.cn/client/todo/detail?guid=abc-123",
    }
    ref = create_task(
        agent="cc",
        cwd="/repo/foo",
        summary="[cc] @foo",
        description="cc Stop @ /repo/foo",
        assignee_open_id="ou_xxx",
        idempotency_key="key-1",
    )
    assert isinstance(ref, TaskRef)
    assert ref.guid == "abc-123"
    assert ref.url.startswith("https://applink.feishu.cn/")

    argv = run_json.call_args.args[0]
    assert argv[0:2] == ["task", "+create"]
    assert "--as" in argv
    assert argv[argv.index("--as") + 1] == "bot"
    assert "--summary" in argv
    # M3.B host patch：summary 自动后缀 host
    assert argv[argv.index("--summary") + 1] == "[cc] @foo · testhost"
    assert "--assignee" in argv
    assert argv[argv.index("--assignee") + 1] == "ou_xxx"
    assert "--idempotency-key" in argv
    assert argv[argv.index("--idempotency-key") + 1] == "key-1"


@patch("roostery.task_writer.run_json")
@patch("roostery.identity.resolve_user_open_id", return_value=None)
def test_create_task_no_assignee(resolve_uid, run_json, monkeypatch):
    """显式 assignee 不传 + identity 也解不出（mock 返回 None）→ argv 无 --assignee。"""
    monkeypatch.setenv("FEISHU_HUB_HOST", "testhost")
    monkeypatch.delenv("FEISHU_NOTIFY_TO", raising=False)
    run_json.return_value = {"guid": "g", "url": "u"}
    create_task(agent="cc", cwd="/r", summary="s")
    argv = run_json.call_args.args[0]
    assert "--assignee" not in argv
    assert "--follower" not in argv  # 不应误加旧 flag


@patch("roostery.task_writer.run_json")
def test_create_task_uses_env_host(run_json, monkeypatch):
    """FEISHU_HUB_HOST env 决定 summary 后缀。"""
    monkeypatch.setenv("FEISHU_HUB_HOST", "axis")
    run_json.return_value = {"guid": "g", "url": "u"}
    create_task(agent="cc", cwd="/r", summary="[cc] @foo")
    argv = run_json.call_args.args[0]
    assert argv[argv.index("--summary") + 1] == "[cc] @foo · axis"


@patch("roostery.task_writer.run_json")
def test_create_task_explicit_host_wins(run_json, monkeypatch):
    """显式 host kwarg 覆盖 env。"""
    monkeypatch.setenv("FEISHU_HUB_HOST", "env-host")
    run_json.return_value = {"guid": "g", "url": "u"}
    create_task(agent="cc", cwd="/r", summary="s", host="kwarg-host")
    argv = run_json.call_args.args[0]
    assert argv[argv.index("--summary") + 1] == "s · kwarg-host"


@patch("roostery.task_writer.run_json")
def test_create_task_does_not_double_suffix(run_json, monkeypatch):
    """summary 已含 · host 时不再追加。"""
    monkeypatch.setenv("FEISHU_HUB_HOST", "axis")
    run_json.return_value = {"guid": "g", "url": "u"}
    create_task(agent="cc", cwd="/r", summary="[cc] @foo · axis")
    argv = run_json.call_args.args[0]
    # 后缀只出现一次
    assert argv[argv.index("--summary") + 1].count("· axis") == 1


@patch("roostery.task_writer.run_json")
def test_create_task_falls_back_to_hostname(run_json, monkeypatch):
    """没有 FEISHU_HUB_HOST 时使用 socket.gethostname() 首段。"""
    monkeypatch.delenv("FEISHU_HUB_HOST", raising=False)
    run_json.return_value = {"guid": "g", "url": "u"}
    create_task(agent="cc", cwd="/r", summary="s")
    argv = run_json.call_args.args[0]
    summary = argv[argv.index("--summary") + 1]
    # 不能是 "· "（空 host）；不能含点（应该是 hostname 首段）
    assert summary.startswith("s · ")
    suffix = summary.split("· ", 1)[1]
    assert suffix, "host 不应为空"
    assert "." not in suffix, "hostname 应已去 .local 等后缀"


@patch("roostery.task_writer.run_json")
def test_append_steps_omits_timestamp(run_json):
    """lark-cli 1.0.28 bug：timestamp 字段必须省略。"""
    run_json.return_value = {"code": 0, "data": {}, "msg": ""}
    append_steps(
        task_guid="g-1",
        steps=["step a", "step b"],
        idempotency_key="ik-1",
    )

    argv = run_json.call_args.args[0]
    assert argv[0:3] == ["task", "agent_task_step_info", "append_task_steps"]
    assert "--as" in argv and argv[argv.index("--as") + 1] == "bot"
    assert "--data" in argv
    # append_task_steps 是 high-risk-write，必须带 --yes（缺 --yes 会 exit 10）
    assert "--yes" in argv

    import json as _json
    data_str = argv[argv.index("--data") + 1]
    data = _json.loads(data_str)
    assert data["task_guid"] == "g-1"
    assert data["idempotent_key"] == "ik-1"
    assert len(data["task_steps"]) == 2
    assert data["task_steps"][0] == {"content": "step a"}
    assert data["task_steps"][1] == {"content": "step b"}
    # timestamp 必须不存在
    for step in data["task_steps"]:
        assert "timestamp" not in step


@patch("roostery.task_writer.run_json")
def test_append_steps_empty_no_op(run_json):
    """空 steps 列表不应触发 lark-cli 调用。"""
    append_steps(task_guid="g", steps=[])
    run_json.assert_not_called()


@patch("roostery.task_writer.run_json")
def test_create_task_threads_profile_kwarg(run_json, monkeypatch):
    """relay_writer 场景：create_task 接 profile= 透传给 run_json。"""
    monkeypatch.setenv("FEISHU_HUB_HOST", "h")
    run_json.return_value = {"guid": "g", "url": "u"}
    create_task(agent="cc", cwd="/r", summary="s",
                profile="cli_writer", idempotency_key="k")
    kw = run_json.call_args.kwargs
    assert kw.get("profile") == "cli_writer"


@patch("roostery.task_writer.run_json")
def test_create_task_no_profile_when_not_passed(run_json, monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_HOST", "h")
    run_json.return_value = {"guid": "g", "url": "u"}
    create_task(agent="cc", cwd="/r", summary="s")
    assert "profile" not in run_json.call_args.kwargs or run_json.call_args.kwargs.get("profile") is None


@patch("roostery.task_writer.run_json")
def test_append_steps_threads_profile_kwarg(run_json):
    run_json.return_value = {"ok": True}
    append_steps("g", ["step1"], profile="cli_writer", idempotency_key="k")
    kw = run_json.call_args.kwargs
    assert kw.get("profile") == "cli_writer"


@patch("roostery.task_writer.run_json")
def test_create_task_propagates_lark_cli_error(run_json):
    from roostery.lark_cli import LarkCLIError

    run_json.side_effect = LarkCLIError(
        code=10403, msg="unauthorized", argv=["task", "+create"]
    )
    with pytest.raises(LarkCLIError):
        create_task(agent="cc", cwd="/r", summary="s")


# --- T3: session cache tests ---
import os
import tempfile
from pathlib import Path
from unittest.mock import patch

from roostery.task_writer import get_or_create_for_session


def test_get_or_create_first_call_creates(tmp_path, monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))

    with patch("roostery.task_writer.run_json") as run_json:
        run_json.return_value = {"guid": "g1", "url": "u1"}
        ref = get_or_create_for_session(
            agent="cc", session="sess-A",
            cwd="/r", summary="s", assignee_open_id="ou_x",
        )
    assert ref.guid == "g1"
    # state 文件应写入
    cache_file = tmp_path / "state" / "session_tasks" / "cc-sess-A.json"
    assert cache_file.exists()
    import json
    cached = json.loads(cache_file.read_text())
    assert cached["task_guid"] == "g1"


def test_get_or_create_second_call_reuses(tmp_path, monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
    # 预置 cache
    cache_dir = tmp_path / "state" / "session_tasks"
    cache_dir.mkdir(parents=True)
    (cache_dir / "cc-sess-B.json").write_text(
        '{"task_guid":"existing","task_url":"u","created_at":"x","summary":"s"}'
    )

    with patch("roostery.task_writer.run_json") as run_json:
        ref = get_or_create_for_session(
            agent="cc", session="sess-B",
            cwd="/r", summary="new summary",
        )
    # 不应调 lark-cli
    run_json.assert_not_called()
    assert ref.guid == "existing"


def test_get_or_create_session_sanitizes_path(tmp_path, monkeypatch):
    """agent / session 含特殊字符不应被注入文件路径。"""
    monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
    with patch("roostery.task_writer.run_json") as run_json:
        run_json.return_value = {"guid": "g", "url": "u"}
        get_or_create_for_session(
            agent="cc",
            session="../../etc/passwd",
            cwd="/r", summary="s",
        )
    # 应有文件被写在 session_tasks/ 内，不应跳出 sandbox
    cache_root = tmp_path / "state" / "session_tasks"
    files = list(cache_root.glob("*.json"))
    assert len(files) == 1
    # 文件名必须不含 / 或 .. 序列
    fname = files[0].name
    assert "/" not in fname
    assert ".." not in fname
