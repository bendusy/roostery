"""roostery.bot_role — bots.yaml 加载 + per-bot 事件匹配。

设计契约：

- ``BotRole`` 是不可变 dataclass，从 ``~/.roostery/bots.yaml`` 顶层 ``bots:`` 数组加载
- ``event_matches_bot(event, bot)`` 回答："这条 IM 事件是否应该被 *这个 bot* 处理？"
  - 必须 ``chat_type == "group"`` 且 ``message_type == "text"``（M3.C 只做群文本接力，不动 p2p / interactive）
  - ``chat_whitelist`` 非空时 ``chat_id`` 必须命中
  - ``content`` 必须以 ``@<mention_alias>`` 起头（容忍 lark-cli convertlib 渲染的双空格 / 单空格 / 中文空格）
- 多 bot 同时匹配同一 event 是合法的（plan C4=A），由调用方决定如何编排——本模块不裁决
"""
from __future__ import annotations

from pathlib import Path

import pytest

yaml = pytest.importorskip("yaml")

from roostery import bot_role as br  # noqa: E402


# ---------------------------------------------------------------------------
# load_bots
# ---------------------------------------------------------------------------

def test_load_bots_parses_minimal_yaml(tmp_path: Path):
    p = tmp_path / "bots.yaml"
    p.write_text(
        """
bots:
  - app_id: cli_aaaaaaaaaaaaaaaa
    role: reviewer
    mention_alias: 审核Bot
    runner: cc_headless
    default_cwd: /tmp/proj
    prompt_template: "审核：{message}"
""",
        encoding="utf-8",
    )
    bots = br.load_bots(p)
    assert len(bots) == 1
    b = bots[0]
    assert b.app_id == "cli_aaaaaaaaaaaaaaaa"
    assert b.role == "reviewer"
    assert b.mention_alias == "审核Bot"
    assert b.runner == "cc_headless"
    assert b.default_cwd == "/tmp/proj"
    assert b.prompt_template == "审核：{message}"
    # 默认字段
    assert b.reply_template == ""
    assert b.chat_whitelist == ()
    assert b.next_bot_mention == ""


def test_load_bots_returns_empty_when_file_missing(tmp_path: Path):
    bots = br.load_bots(tmp_path / "nope.yaml")
    assert bots == []


def test_load_bots_returns_empty_when_no_bots_key(tmp_path: Path):
    p = tmp_path / "bots.yaml"
    p.write_text("other: stuff\n", encoding="utf-8")
    assert br.load_bots(p) == []


def test_load_bots_rejects_missing_required_fields(tmp_path: Path):
    p = tmp_path / "bots.yaml"
    p.write_text(
        """
bots:
  - role: reviewer
""",
        encoding="utf-8",
    )
    with pytest.raises(br.BotRoleConfigError):
        br.load_bots(p)


def test_load_bots_reads_relay_writer_app_id(tmp_path: Path):
    """relay_writer_app_id 可选；让多机 daemon 把 relay_task 都写到同一身份下，
    跨 bot 收敛到同一飞书 task guid（per-bot idempotency 限制的 workaround）。"""
    p = tmp_path / "bots.yaml"
    p.write_text(
        """
bots:
  - app_id: cli_a
    role: reviewer
    mention_alias: A
    runner: noop
    default_cwd: /tmp
    prompt_template: x
    relay_writer_app_id: cli_writer_central
  - app_id: cli_b
    role: scribe
    mention_alias: B
    runner: noop
    default_cwd: /tmp
    prompt_template: x
""",
        encoding="utf-8",
    )
    bots = br.load_bots(p)
    assert bots[0].relay_writer_app_id == "cli_writer_central"
    # 没设的 bot 默认空
    assert bots[1].relay_writer_app_id == ""


def test_load_bots_preserves_chat_whitelist_as_tuple(tmp_path: Path):
    p = tmp_path / "bots.yaml"
    p.write_text(
        """
bots:
  - app_id: cli_x
    role: scribe
    mention_alias: 沉淀Bot
    runner: cc_headless
    default_cwd: /tmp/p
    prompt_template: "x"
    chat_whitelist:
      - oc_aaa
      - oc_bbb
""",
        encoding="utf-8",
    )
    bots = br.load_bots(p)
    assert bots[0].chat_whitelist == ("oc_aaa", "oc_bbb")


# ---------------------------------------------------------------------------
# event_matches_bot
# ---------------------------------------------------------------------------

def _bot(**over) -> "br.BotRole":
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


def _ev(**over) -> dict:
    base = {
        "chat_type": "group",
        "message_type": "text",
        "chat_id": "oc_test",
        "content": "@审核Bot  hello world",
        "sender_id": "ou_user",
        "message_id": "om_xxx",
    }
    base.update(over)
    return base


def test_event_matches_when_content_starts_with_mention():
    assert br.event_matches_bot(_ev(), _bot()) is True


def test_event_matches_tolerates_single_space_after_mention():
    # lark-cli convertlib 默认双空格，但某些客户端只发单空格——都要容忍
    assert br.event_matches_bot(_ev(content="@审核Bot hello"), _bot()) is True


def test_event_does_not_match_when_mention_in_middle():
    assert br.event_matches_bot(
        _ev(content="hello @审核Bot please review"), _bot()
    ) is False


def test_event_does_not_match_different_bot():
    assert br.event_matches_bot(_ev(content="@沉淀Bot foo"), _bot()) is False


def test_event_does_not_match_p2p_chat():
    assert br.event_matches_bot(_ev(chat_type="p2p"), _bot()) is False


def test_event_does_not_match_non_text_message():
    assert br.event_matches_bot(_ev(message_type="image"), _bot()) is False


def test_chat_whitelist_filters_unlisted_chat():
    bot = _bot(chat_whitelist=("oc_aaa",))
    assert br.event_matches_bot(_ev(chat_id="oc_bbb"), bot) is False
    assert br.event_matches_bot(_ev(chat_id="oc_aaa"), bot) is True


def test_empty_chat_whitelist_allows_any_chat():
    assert br.event_matches_bot(_ev(chat_id="oc_anything"), _bot()) is True


def test_extract_message_body_strips_leading_mention():
    body = br.extract_message_body(_ev(content="@审核Bot  hello world"), _bot())
    assert body == "hello world"


def test_extract_message_body_returns_full_content_when_no_mention():
    # 给的 event 可能根本不是 @ 这个 bot 的——调用方应先 event_matches_bot 过滤，
    # 但 extract_message_body 不应崩，保留原文
    body = br.extract_message_body(_ev(content="random text"), _bot())
    assert body == "random text"
