"""roostery.bot_relay_task — runner 前 record_start + runner 后 record_end (M3.E)。

设计：
- 每个 chat_id 在本机 cache 一个飞书 Task GUID（``~/.roostery/state/m3c_chats/<chat_id>.json``）
- record_start 在 runner 跑之前调一次：首次见到 chat 建 task + append 起始 step
- record_end 在 runner 跑之后调一次：append 完成 / 超时 / 失败 step（同一个 task）
- step 文案：emoji 状态 + [role] + 内容（人类可读，飞书 task 详情页一眼看懂）
"""
from __future__ import annotations

import json
from pathlib import Path
from types import SimpleNamespace

import pytest

from roostery import bot_relay_task as brt
from roostery import bot_role as br
from roostery.bot_runner import BotAction
from roostery.task_writer import TaskRef


def _bot(**over) -> br.BotRole:
    base = dict(
        app_id="cli_aaa",
        role="reviewer",
        mention_alias="审核Bot",
        runner="cc_headless",
        default_cwd="/tmp/x",
        prompt_template="x",
    )
    base.update(over)
    return br.BotRole(**base)


def _action(**over) -> BotAction:
    base = dict(
        bot_app_id="cli_aaa",
        chat_id="oc_e6e50b04fc21414d6364036b23438af9",
        source_message_id="om_test_001",
        reply_message_id="om_reply_001",
        runner_exit_code=0,
        timed_out=False,
    )
    base.update(over)
    return BotAction(**base)


def _event(**over) -> dict:
    base = {
        "chat_id": "oc_e6e50b04fc21414d6364036b23438af9",
        "message_id": "om_test_001",
        "sender_id": "ou_user_xxxxxx",
    }
    base.update(over)
    return base


@pytest.fixture
def isolated_state(tmp_path, monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
    return tmp_path


@pytest.fixture
def fake_writer(monkeypatch):
    """劫持 task_writer.create_task / append_steps，记录调用。"""
    state = SimpleNamespace(creates=[], appends=[])

    def fake_create(agent, cwd, summary, *, description="", **kw):
        state.creates.append({"summary": summary, "agent": agent, "kw": kw})
        return TaskRef(guid=f"g_{len(state.creates)}", url=f"https://t.io/{len(state.creates)}")

    def fake_append(guid, steps, *, idempotency_key=None, **kw):
        state.appends.append({"guid": guid, "steps": list(steps),
                              "idempotency_key": idempotency_key, **kw})

    monkeypatch.setattr(brt.task_writer, "create_task", fake_create)
    monkeypatch.setattr(brt.task_writer, "append_steps", fake_append)
    return state


# ---------------------------------------------------------------------------
# record_start (M3.E)
# ---------------------------------------------------------------------------

def test_record_start_creates_task_and_appends_received_step(isolated_state, fake_writer):
    bot = _bot()
    ref = brt.record_start(
        bot=bot,
        event=_event(),
        message_brief="整理 GenericAgent 今天 commit",
    )
    assert ref is not None
    assert ref.guid == "g_1"
    # task summary 含 M3.C 接力链 + chat_id 短码
    assert len(fake_writer.creates) == 1
    summary = fake_writer.creates[0]["summary"]
    assert "M3.C" in summary
    assert "23438af9" in summary
    # 起始 step 应含 emoji 🚀 + role + brief
    assert len(fake_writer.appends) == 1
    step = fake_writer.appends[0]["steps"][0]
    assert step.startswith("🚀 ")
    assert "[reviewer]" in step
    assert "整理 GenericAgent 今天 commit" in step
    # idem key 跟 record_end 不同
    assert "m3c-step-start" in fake_writer.appends[0]["idempotency_key"]


def test_record_start_truncates_message_brief_at_80_chars(isolated_state, fake_writer):
    bot = _bot()
    long_msg = "A" * 200
    brt.record_start(bot=bot, event=_event(), message_brief=long_msg)
    step = fake_writer.appends[0]["steps"][0]
    assert "A" * 80 in step
    assert "A" * 100 not in step


def test_record_start_returns_none_when_no_chat_id(isolated_state, fake_writer):
    bot = _bot()
    ref = brt.record_start(bot=bot, event=_event(chat_id=""), message_brief="x")
    assert ref is None
    assert len(fake_writer.creates) == 0


def test_record_start_reuses_existing_task_for_same_chat(isolated_state, fake_writer):
    bot = _bot()
    brt.record_start(bot=bot, event=_event(message_id="om_1"), message_brief="m1")
    brt.record_start(bot=bot, event=_event(message_id="om_2"), message_brief="m2")
    # 同一 chat → 同一 task；create_task 只调一次
    assert len(fake_writer.creates) == 1
    # 但 append 两次（两条起始 step）
    assert len(fake_writer.appends) == 2


def test_record_start_separates_tasks_per_chat(isolated_state, fake_writer):
    bot = _bot()
    brt.record_start(bot=bot, event=_event(chat_id="oc_aaa"), message_brief="m1")
    brt.record_start(bot=bot, event=_event(chat_id="oc_bbb"), message_brief="m2")
    assert len(fake_writer.creates) == 2
    assert {a["guid"] for a in fake_writer.appends} == {"g_1", "g_2"}


def test_record_start_idempotent_step_key_per_event(isolated_state, fake_writer):
    """同一 message_id × bot 重复调 → start step idem key 一致。"""
    bot = _bot()
    brt.record_start(bot=bot, event=_event(message_id="om_777"), message_brief="x")
    brt.record_start(bot=bot, event=_event(message_id="om_777"), message_brief="x")
    k1 = fake_writer.appends[0]["idempotency_key"]
    k2 = fake_writer.appends[1]["idempotency_key"]
    assert k1 and k1 == k2


# ---------------------------------------------------------------------------
# record_end (M3.E)
# ---------------------------------------------------------------------------

def test_record_end_appends_completed_step_with_result(isolated_state, fake_writer):
    bot = _bot()
    # 先 record_start 让 cache 建好
    brt.record_start(bot=bot, event=_event(), message_brief="x")
    # 然后 record_end
    ref = brt.record_end(
        bot=bot,
        action=_action(),
        result_text="一切 OK，commit 已整理",
    )
    assert ref is not None
    # 应该是同一个 task（cache 命中）
    assert ref.guid == "g_1"
    # 现在 appends 两条：起始 + 完成
    assert len(fake_writer.appends) == 2
    end_step = fake_writer.appends[1]["steps"][0]
    assert end_step.startswith("✅ ")
    assert "[reviewer]" in end_step
    assert "一切 OK" in end_step
    assert "m3c-step-end" in fake_writer.appends[1]["idempotency_key"]


def test_record_end_marks_timeout_with_warning_emoji(isolated_state, fake_writer):
    bot = _bot()
    brt.record_start(bot=bot, event=_event(), message_brief="x")
    brt.record_end(
        bot=bot,
        action=_action(timed_out=True, runner_exit_code=-1),
        result_text="",
    )
    step = fake_writer.appends[1]["steps"][0]
    assert step.startswith("⚠️ ")
    assert "超时" in step or "timeout" in step.lower()


def test_record_end_marks_failure_with_x_emoji(isolated_state, fake_writer):
    bot = _bot()
    brt.record_start(bot=bot, event=_event(), message_brief="x")
    brt.record_end(
        bot=bot,
        action=_action(runner_exit_code=1),
        result_text="(no result)",
    )
    step = fake_writer.appends[1]["steps"][0]
    assert step.startswith("❌ ")
    assert "exit=1" in step


def test_record_end_truncates_result_at_200_chars(isolated_state, fake_writer):
    bot = _bot()
    brt.record_start(bot=bot, event=_event(), message_brief="x")
    long_result = "B" * 500
    brt.record_end(bot=bot, action=_action(), result_text=long_result)
    step = fake_writer.appends[1]["steps"][0]
    assert "B" * 200 in step
    assert "B" * 300 not in step


def test_record_end_returns_none_when_no_chat_id(isolated_state, fake_writer):
    bot = _bot()
    ref = brt.record_end(
        bot=bot,
        action=_action(chat_id=""),
        result_text="x",
    )
    assert ref is None
    assert len(fake_writer.creates) == 0


# ---------------------------------------------------------------------------
# relay_writer_app_id (跨机统一身份写手；保留)
# ---------------------------------------------------------------------------

def test_record_start_routes_to_relay_writer_profile_when_set(isolated_state, monkeypatch):
    """relay_writer_app_id 非空 → create_task / append_steps 都带 profile=。"""
    creates = []
    appends = []

    def fake_create(agent, cwd, summary, *, description="", **kw):
        creates.append(kw)
        return TaskRef(guid="g1", url="u1")

    def fake_append(guid, steps, *, idempotency_key=None, profile=None):
        appends.append({"profile": profile})

    monkeypatch.setattr(brt.task_writer, "create_task", fake_create)
    monkeypatch.setattr(brt.task_writer, "append_steps", fake_append)

    bot = _bot(relay_writer_app_id="cli_central_writer")
    brt.record_start(bot=bot, event=_event(), message_brief="x")
    assert creates[0]["profile"] == "cli_central_writer"
    assert appends[0]["profile"] == "cli_central_writer"


def test_record_end_routes_to_relay_writer_profile_when_set(isolated_state, monkeypatch):
    creates = []
    appends = []

    def fake_create(agent, cwd, summary, *, description="", **kw):
        creates.append(kw)
        return TaskRef(guid="g1", url="u1")

    def fake_append(guid, steps, *, idempotency_key=None, profile=None):
        appends.append({"profile": profile})

    monkeypatch.setattr(brt.task_writer, "create_task", fake_create)
    monkeypatch.setattr(brt.task_writer, "append_steps", fake_append)

    bot = _bot(relay_writer_app_id="cli_central_writer")
    brt.record_start(bot=bot, event=_event(), message_brief="x")
    brt.record_end(bot=bot, action=_action(), result_text="ok")
    # append 两次：start + end，两次都应带 profile
    assert appends[0]["profile"] == "cli_central_writer"
    assert appends[1]["profile"] == "cli_central_writer"


def test_record_default_profile_none_when_no_writer_configured(isolated_state, monkeypatch):
    """relay_writer_app_id 空 = 不传 --profile，沿用当前 active profile（向后兼容）。"""
    creates = []
    appends = []

    def fake_create(agent, cwd, summary, *, description="", **kw):
        creates.append(kw)
        return TaskRef(guid="g1", url="u1")

    def fake_append(guid, steps, *, idempotency_key=None, profile=None):
        appends.append({"profile": profile})

    monkeypatch.setattr(brt.task_writer, "create_task", fake_create)
    monkeypatch.setattr(brt.task_writer, "append_steps", fake_append)

    bot = _bot()
    brt.record_start(bot=bot, event=_event(), message_brief="x")
    assert creates[0].get("profile") in (None, "")
    assert appends[0]["profile"] in (None, "")


# ---------------------------------------------------------------------------
# _format_end_step aborted 分支（R5 POC T6）
# ---------------------------------------------------------------------------

def test_format_end_step_aborted_takes_priority():
    """aborted=True 时不管 exit_code / timed_out，都出'中止'文案。"""
    bot = _bot()
    action = _action(
        runner_exit_code=-15,
        timed_out=False,
        aborted=True,
        abort_reason="/stop",
    )
    step = brt._format_end_step(bot, action, "ignored output")
    assert "⚠️" in step
    assert "用户请求中止" in step
    assert "/stop" in step


def test_format_end_step_adjust_attempts_in_success():
    from roostery import bot_relay_task as brt
    from roostery.bot_runner import BotAction
    from roostery.bot_role import BotRole

    bot = BotRole(
        app_id="cli_x", role="r", mention_alias="r",
        runner="noop", prompt_template="{message}",
        reply_template="{result}", default_cwd=".",
    )
    action = BotAction(
        bot_app_id="cli_x", chat_id="oc_x",
        source_message_id="om_x", reply_message_id=None,
        runner_exit_code=0, timed_out=False,
        adjust_attempts=1,
    )
    step = brt._format_end_step(bot, action, "final text")
    assert "✅" in step
    assert "调整后完成" in step
    assert "#1" in step


def test_record_adjust_calls_append_steps(monkeypatch):
    from roostery import bot_relay_task as brt, task_writer
    from roostery.bot_role import BotRole
    from roostery.task_writer import TaskRef

    bot = BotRole(
        app_id="cli_x", role="r", mention_alias="r",
        runner="noop", prompt_template="{message}",
        reply_template="{result}", default_cwd=".",
    )
    ref = TaskRef(guid="t1", url="u")

    calls = []
    def fake_append_steps(guid, steps, *, idempotency_key, profile):
        calls.append((guid, steps, idempotency_key, profile))
    monkeypatch.setattr(task_writer, "append_steps", fake_append_steps)

    brt.record_adjust(bot=bot, task_ref=ref, adjust_text="加细节", attempt=1)
    assert len(calls) == 1
    guid, steps, idem, profile = calls[0]
    assert guid == "t1"
    assert "🔄" in steps[0]
    assert "调整" in steps[0]
    assert "加细节" in steps[0]
    assert "#1" in steps[0]
    assert "m3g-step-adjust" in idem
