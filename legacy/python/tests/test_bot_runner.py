"""roostery.bot_runner — IM event → runner → thread reply。"""
from __future__ import annotations

from types import SimpleNamespace

import pytest

from roostery import bot_role as br
from roostery import bot_runner as r
from roostery.dispatcher.runners import RunResult, RunSpec
from roostery.task_writer import TaskRef


@pytest.fixture(autouse=True)
def _isolate_relay_task(monkeypatch):
    """默认 mock bot_relay_task.record_start / record_end，防止真打飞书 API。

    单独需要观察 record_start/end 行为的测试自己再覆盖 monkeypatch.setattr。
    """
    monkeypatch.setattr(r.bot_relay_task, "record_start", lambda **kw: None)
    monkeypatch.setattr(r.bot_relay_task, "record_end", lambda **kw: None)


def _bot(**over) -> br.BotRole:
    base = dict(
        app_id="cli_aaa",
        role="reviewer",
        mention_alias="审核Bot",
        runner="cc_headless",
        default_cwd="/tmp/x",
        prompt_template="请审核：{message}",
        reply_template="{result}",
    )
    base.update(over)
    return br.BotRole(**base)


def _ev(**over) -> dict:
    base = {
        "chat_type": "group",
        "message_type": "text",
        "chat_id": "oc_test",
        "content": "@审核Bot  请检查 hello world",
        "sender_id": "ou_user",
        "message_id": "om_xxx",
    }
    base.update(over)
    return base


def _ok_result(text: str = "✅ 通过") -> RunResult:
    return RunResult(
        runner="cc_headless",
        exit_code=0,
        stdout=text,
        stderr="",
        stdout_head=text,
        stderr_head="",
        duration_ms=12,
        timed_out=False,
        final_text=text,
    )


# ---------------------------------------------------------------------------

def test_handle_event_returns_none_when_event_does_not_match():
    bot = _bot()
    ev = _ev(content="hi @沉淀Bot something")  # @不是这个 bot
    result = r.handle_event(ev, bot, runner=lambda s, **kw: _ok_result(), replier=None)
    assert result is None


def test_handle_event_formats_prompt_with_stripped_body():
    bot = _bot()
    ev = _ev(content="@审核Bot  please review this")
    captured: dict = {}

    def fake_runner(spec: RunSpec, **kw) -> RunResult:
        captured["prompt"] = spec.prompt
        captured["cwd"] = spec.cwd
        captured["runner"] = spec.runner
        return _ok_result()

    def fake_replier(**kw):
        return "om_reply_1"

    r.handle_event(ev, bot, runner=fake_runner, replier=fake_replier)
    assert captured["prompt"] == "请审核：please review this"
    assert captured["cwd"] == "/tmp/x"
    assert captured["runner"] == "cc_headless"


def test_handle_event_replies_in_thread_with_runner_result():
    bot = _bot(reply_template="审核结果：{result}")
    ev = _ev()
    captured: dict = {}

    def fake_replier(**kw):
        captured.update(kw)
        return "om_reply_1"

    action = r.handle_event(
        ev, bot,
        runner=lambda s, **kw: _ok_result("✅ 通过"),
        replier=fake_replier,
    )
    assert captured["message_id"] == "om_xxx"
    assert captured["text"] == "审核结果：✅ 通过"
    assert captured["thread"] is True
    assert captured["profile"] == "cli_aaa"
    assert action.reply_message_id == "om_reply_1"
    assert action.runner_exit_code == 0


def test_handle_event_appends_next_bot_mention_to_reply():
    bot = _bot(
        reply_template="审核结果：{result}",
        next_bot_mention="@沉淀Bot",
    )
    ev = _ev()
    captured: dict = {}

    def fake_replier(**kw):
        captured.update(kw)
        return "om_reply_1"

    r.handle_event(
        ev, bot,
        runner=lambda s, **kw: _ok_result("✅ 通过"),
        replier=fake_replier,
    )
    # next_bot_mention 应被追加到 reply text 末尾（独立一行）
    assert "@沉淀Bot" in captured["text"]
    assert captured["text"].endswith("@沉淀Bot 请接力。")


def test_handle_event_reports_runner_failure_in_reply():
    bot = _bot()
    ev = _ev()
    bad = RunResult(
        runner="cc_headless", exit_code=1,
        stdout="", stderr="boom",
        stdout_head="", stderr_head="boom",
        duration_ms=5, timed_out=False, final_text=None,
    )
    captured: dict = {}

    def fake_replier(**kw):
        captured.update(kw)
        return "om_reply_err"

    action = r.handle_event(
        ev, bot, runner=lambda s, **kw: bad, replier=fake_replier,
    )
    assert action.runner_exit_code == 1
    assert "runner failed" in captured["text"] or "boom" in captured["text"]


def test_handle_event_skips_when_runner_timed_out():
    """超时的 runner result：仍然回 IM（plan C3=C），但带 timeout 标识。"""
    bot = _bot()
    ev = _ev()
    timed = RunResult(
        runner="cc_headless", exit_code=-1,
        stdout="", stderr="",
        stdout_head="", stderr_head="",
        duration_ms=600_000, timed_out=True, final_text=None,
    )
    captured: dict = {}

    def fake_replier(**kw):
        captured.update(kw)
        return "om_reply_timeout"

    action = r.handle_event(
        ev, bot, runner=lambda s, **kw: timed, replier=fake_replier,
    )
    assert action.timed_out is True
    assert "timeout" in captured["text"].lower() or "超时" in captured["text"]


# ---------------------------------------------------------------------------
# M3.E: record_start / record_end 调用顺序 + reply 含 task URL
# ---------------------------------------------------------------------------

def test_handle_event_calls_record_start_before_runner_and_end_after(monkeypatch):
    """M3.E：handle_event 调用顺序 record_start → runner → record_end → reply。"""
    bot = _bot()
    ev = _ev()
    order = []

    def fake_start(*, bot, event, message_brief):
        order.append("record_start")
        return TaskRef(guid="g_test",
                       url="https://applink.feishu.cn/client/todo/detail?guid=g_test")

    def fake_runner(spec, **kw):
        order.append("runner")
        return _ok_result("done")

    def fake_end(*, bot, action, result_text):
        order.append("record_end")
        return None

    def fake_replier(**kw):
        order.append("reply")
        return "om_reply"

    monkeypatch.setattr(r.bot_relay_task, "record_start", fake_start)
    monkeypatch.setattr(r.bot_relay_task, "record_end", fake_end)

    r.handle_event(ev, bot, runner=fake_runner, replier=fake_replier)
    assert order == ["record_start", "runner", "record_end", "reply"]


def test_handle_event_reply_text_contains_task_url(monkeypatch):
    """reply 文本末尾应追加 task URL，方便 user 点开看完整进度。"""
    bot = _bot()
    ev = _ev()
    captured = {}

    def fake_start(*, bot, event, message_brief):
        return TaskRef(guid="g_test",
                       url="https://applink.feishu.cn/client/todo/detail?guid=g_test")

    monkeypatch.setattr(r.bot_relay_task, "record_start", fake_start)
    monkeypatch.setattr(r.bot_relay_task, "record_end", lambda **kw: None)

    def fake_replier(**kw):
        captured.update(kw)
        return "om_reply"

    r.handle_event(ev, bot,
                   runner=lambda s, **kw: _ok_result("✅ 通过"),
                   replier=fake_replier)
    text = captured["text"]
    assert "https://applink.feishu.cn/client/todo/detail?guid=g_test" in text
    assert "查看完整进度" in text


def test_handle_event_reply_without_task_url_when_record_start_returns_none(monkeypatch):
    """relay_task 失败（chat_id 缺）时不附 URL，但 reply 仍发出。"""
    bot = _bot()
    ev = _ev()
    captured = {}

    monkeypatch.setattr(r.bot_relay_task, "record_start", lambda **kw: None)
    monkeypatch.setattr(r.bot_relay_task, "record_end", lambda **kw: None)

    def fake_replier(**kw):
        captured.update(kw)
        return "om_reply"

    r.handle_event(ev, bot,
                   runner=lambda s, **kw: _ok_result("ok"),
                   replier=fake_replier)
    text = captured["text"]
    assert "applink.feishu.cn" not in text
    assert "ok" in text  # 主体 reply 还在


def test_handle_event_continues_when_record_start_raises(monkeypatch):
    """relay_task 异常不应阻塞 runner / reply 主路径。"""
    bot = _bot()
    ev = _ev()
    captured = {}

    def boom_start(**kw):
        raise RuntimeError("飞书 task API down")

    monkeypatch.setattr(r.bot_relay_task, "record_start", boom_start)
    monkeypatch.setattr(r.bot_relay_task, "record_end", lambda **kw: None)

    def fake_replier(**kw):
        captured.update(kw)
        return "om_reply"

    action = r.handle_event(ev, bot,
                            runner=lambda s, **kw: _ok_result("survived"),
                            replier=fake_replier)
    # runner 跑了、reply 发了
    assert action is not None
    assert "survived" in captured["text"]


def test_handle_event_continues_when_record_end_raises(monkeypatch):
    """record_end 异常也不阻塞主路径。"""
    bot = _bot()
    ev = _ev()
    captured = {}

    monkeypatch.setattr(
        r.bot_relay_task, "record_start",
        lambda **kw: TaskRef(guid="g", url="https://applink.feishu.cn/x"),
    )

    def boom_end(**kw):
        raise RuntimeError("flaky")

    monkeypatch.setattr(r.bot_relay_task, "record_end", boom_end)

    def fake_replier(**kw):
        captured.update(kw)
        return "om_reply"

    action = r.handle_event(ev, bot,
                            runner=lambda s, **kw: _ok_result("survived"),
                            replier=fake_replier)
    assert action is not None
    # URL 还是附了（record_start 成功）
    assert "applink.feishu.cn" in captured["text"]


# ---------------------------------------------------------------------------
# R5 HITL POC T7: runner_registry + abort sentinel
# ---------------------------------------------------------------------------

def test_handle_event_reads_abort_sentinel_into_result(monkeypatch, tmp_path):
    """R5 T7: runner 完成后读 sentinel → result.aborted=True，reply 含中止文案。"""
    from roostery import runner_registry as rr

    bot = br.BotRole(
        app_id="cli_x", role="r", mention_alias="r",
        runner="noop", prompt_template="{message}",
        reply_template="{result}", default_cwd=".",
    )
    event = {"message_id": "om_x", "chat_id": "oc_x",
             "message_type": "text", "content": "@r hi",
             "sender_id": "ou_user", "chat_type": "group"}

    task_ref = TaskRef(guid="g_abort_test", url="https://applink.feishu.cn/t/g_abort_test")

    monkeypatch.setattr(r.bot_relay_task, "record_start", lambda **kw: task_ref)
    monkeypatch.setattr(r.bot_relay_task, "record_end", lambda **kw: None)

    # 用 tmp_path 做 state 目录，预写 sentinel
    registry = rr.RunnerRegistry(root=tmp_path)
    registry.write_abort_sentinel("g_abort_test", "/stop by user")

    # patch RunnerRegistry 构造函数，让 handle_event 拿到我们的 registry
    monkeypatch.setattr(rr, "RunnerRegistry", lambda: registry)

    captured: dict = {}

    def fake_replier(**kw):
        captured.update(kw)
        return "om_reply"

    action = r.handle_event(event, bot,
                            runner=lambda s, **kw: _ok_result("ok"),
                            replier=fake_replier)

    assert action is not None
    assert action.aborted is True
    assert action.abort_reason == "/stop by user"
    assert "中止" in captured["text"] or "aborted" in captured["text"].lower()


def test_handle_event_unregisters_on_success(monkeypatch, tmp_path):
    """R5 T7: runner 成功结束 → registry 里无残留 entry 文件。"""
    from roostery import runner_registry as rr

    bot = br.BotRole(
        app_id="cli_x", role="r", mention_alias="r",
        runner="noop", prompt_template="{message}",
        reply_template="{result}", default_cwd=".",
    )
    event = {"message_id": "om_y", "chat_id": "oc_y",
             "message_type": "text", "content": "@r hello",
             "sender_id": "ou_user2", "chat_type": "group"}

    task_ref = TaskRef(guid="g_unreg_test", url="https://applink.feishu.cn/t/g_unreg_test")

    monkeypatch.setattr(r.bot_relay_task, "record_start", lambda **kw: task_ref)
    monkeypatch.setattr(r.bot_relay_task, "record_end", lambda **kw: None)

    registry = rr.RunnerRegistry(root=tmp_path)
    monkeypatch.setattr(rr, "RunnerRegistry", lambda: registry)

    pids_registered: list = []

    def fake_runner(spec, **kw):
        # on_pid 会由 handle_event 传进来，这里模拟 pid 注册
        on_pid = kw.get("on_pid")
        if on_pid:
            on_pid(12345)
            pids_registered.append(12345)
        return _ok_result("done")

    def fake_replier(**kw):
        return "om_reply"

    action = r.handle_event(event, bot, runner=fake_runner, replier=fake_replier)

    assert action is not None
    assert action.aborted is False
    # runner 运行完成后，registry 应该已清理 entry 文件
    assert registry.lookup("g_unreg_test") is None


# ---------------------------------------------------------------------------
# M3.G T5: handle_event adjust 重启循环
# ---------------------------------------------------------------------------

def test_handle_event_adjust_path_restarts_runner_once(tmp_path, monkeypatch):
    """user /adjust → runner 第二次调用 with 拼接 prompt + adjust_attempts=1。"""
    monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))

    from roostery import bot_runner, runner_registry
    from roostery.bot_role import BotRole
    from roostery.dispatcher.runners import RunSpec, RunResult
    from roostery.task_writer import TaskRef

    bot = BotRole(
        app_id="cli_x", role="r", mention_alias="r",
        runner="noop", prompt_template="{message}",
        reply_template="{result}", default_cwd=".",
    )
    event = {"message_id": "om_x", "chat_id": "oc_x",
             "message_type": "text", "content": "@r hi",
             "sender_id": "ou_user", "chat_type": "group"}

    fake_ref = TaskRef(guid="t1", url="u")
    monkeypatch.setattr(bot_runner.bot_relay_task, "record_start",
                        lambda **kw: fake_ref)
    monkeypatch.setattr(bot_runner.bot_relay_task, "record_end",
                        lambda **kw: None)
    monkeypatch.setattr(bot_runner.bot_relay_task, "record_adjust",
                        lambda **kw: None)
    monkeypatch.setattr(bot_runner, "_default_replier",
                        lambda **kw: "om_reply")

    call_count = {"n": 0}
    seen_prompts = []
    reg = runner_registry.RunnerRegistry(root=tmp_path)
    monkeypatch.setattr(runner_registry, "RunnerRegistry", lambda: reg)

    def fake_runner(spec, *, on_pid=None):
        call_count["n"] += 1
        seen_prompts.append(spec.prompt)
        if call_count["n"] == 1:
            # 第一轮：写 adjust sentinel 模拟 hitl_router 命中
            reg.write_adjust_sentinel("t1", "跑短点")
            return RunResult(
                runner="noop", exit_code=-15, stdout="", stderr="",
                stdout_head="", stderr_head="", duration_ms=100, timed_out=False,
            )
        # 第二轮：正常完成
        return RunResult(
            runner="noop", exit_code=0, stdout="OK", stderr="",
            stdout_head="OK", stderr_head="", duration_ms=200, timed_out=False,
            final_text="OK",
        )

    action = bot_runner.handle_event(event, bot, runner=fake_runner)
    assert call_count["n"] == 2
    assert "[用户调整]: 跑短点" in seen_prompts[1]
    assert action.adjust_attempts == 1
    assert action.aborted is False


def test_handle_event_adjust_then_adjust_exceeds_max(tmp_path, monkeypatch):
    """两次 adjust 超 ADJUST_MAX=1：第二次降级为 abort。"""
    monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))

    from roostery import bot_runner, runner_registry
    from roostery.bot_role import BotRole
    from roostery.dispatcher.runners import RunSpec, RunResult
    from roostery.task_writer import TaskRef

    bot = BotRole(
        app_id="cli_x", role="r", mention_alias="r",
        runner="noop", prompt_template="{message}",
        reply_template="{result}", default_cwd=".",
    )
    event = {"message_id": "om_x", "chat_id": "oc_x",
             "message_type": "text", "content": "@r hi",
             "sender_id": "ou_user", "chat_type": "group"}

    fake_ref = TaskRef(guid="t1", url="u")
    monkeypatch.setattr(bot_runner.bot_relay_task, "record_start",
                        lambda **kw: fake_ref)
    monkeypatch.setattr(bot_runner.bot_relay_task, "record_end",
                        lambda **kw: None)
    monkeypatch.setattr(bot_runner.bot_relay_task, "record_adjust",
                        lambda **kw: None)
    monkeypatch.setattr(bot_runner, "_default_replier",
                        lambda **kw: "om_reply")

    reg = runner_registry.RunnerRegistry(root=tmp_path)
    monkeypatch.setattr(runner_registry, "RunnerRegistry", lambda: reg)
    counter = {"n": 0}

    def fake_runner(spec, *, on_pid=None):
        counter["n"] += 1
        reg.write_adjust_sentinel("t1", f"调整{counter['n']}")
        return RunResult(
            runner="noop", exit_code=-15, stdout="", stderr="",
            stdout_head="", stderr_head="", duration_ms=10, timed_out=False,
        )

    action = bot_runner.handle_event(event, bot, runner=fake_runner)
    # ADJUST_MAX=1 → 第一次重启，第二次 adjust 超额，降级 abort
    assert counter["n"] == 2  # 跑过两次（原 + 重启 1 次）
    assert action.aborted is True
    assert "超" in action.abort_reason  # "/adjust 超 1 次上限"
