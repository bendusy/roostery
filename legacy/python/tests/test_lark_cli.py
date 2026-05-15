"""roostery.lark_cli 单测（mock subprocess）。"""
import json
import subprocess
from unittest.mock import patch, MagicMock

import pytest

from roostery import lark_cli as lc


def _completed(rc=0, stdout="", stderr=""):
    cp = MagicMock(spec=subprocess.CompletedProcess)
    cp.returncode = rc
    cp.stdout = stdout
    cp.stderr = stderr
    return cp


# ---- run_json --------------------------------------------------------------

@patch("roostery.lark_cli.subprocess.run")
def test_run_json_does_not_inject_format_flag(mock_run):
    """许多子命令（im +messages-send / docs +create）不接受 --format，
    且默认就输出 JSON，所以 run_json 不自动注入。"""
    mock_run.return_value = _completed(0, "{}", "")
    lc.run_json(["wiki", "+search"])
    argv = mock_run.call_args[0][0]
    assert argv[0] == "lark-cli"
    assert argv[1:3] == ["wiki", "+search"]
    assert "--format" not in argv


@patch("roostery.lark_cli.subprocess.run")
def test_run_json_passes_explicit_format_unchanged(mock_run):
    """调用方显式给 --format 时透传。"""
    mock_run.return_value = _completed(0, "{}", "")
    lc.run_json(["x", "--format", "ndjson"])
    argv = mock_run.call_args[0][0]
    assert argv.count("--format") == 1


@patch("roostery.lark_cli.subprocess.run")
def test_run_json_appends_jq(mock_run):
    mock_run.return_value = _completed(0, "hello\n", "")
    out = lc.run_json(["x"], jq=".y")
    assert out == "hello"
    argv = mock_run.call_args[0][0]
    assert argv[-2:] == ["--jq", ".y"]


@patch("roostery.lark_cli.subprocess.run")
def test_run_json_parses_json(mock_run):
    mock_run.return_value = _completed(0, '{"a":1}', "")
    assert lc.run_json(["x"]) == {"a": 1}


@patch("roostery.lark_cli.subprocess.run")
def test_run_json_passes_stdin(mock_run):
    mock_run.return_value = _completed(0, "{}", "")
    lc.run_json(["docs", "+create"], stdin="# md")
    kwargs = mock_run.call_args.kwargs
    assert kwargs["input"] == "# md"


@patch("roostery.lark_cli.subprocess.run")
def test_run_json_raises_on_nonzero(mock_run):
    mock_run.return_value = _completed(1, '{"code":1234,"msg":"bad"}', "")
    with pytest.raises(lc.LarkCLIError) as exc:
        lc.run_json(["x"])
    assert exc.value.code == 1234
    assert exc.value.msg == "bad"
    assert exc.value.retriable is False


@patch("roostery.lark_cli.subprocess.run")
def test_run_json_raises_on_non_json_stdout(mock_run):
    mock_run.return_value = _completed(0, "not json", "")
    with pytest.raises(lc.LarkCLIError, match="non-JSON"):
        lc.run_json(["x"])


@patch("roostery.lark_cli.subprocess.run")
def test_run_json_returns_none_on_empty(mock_run):
    mock_run.return_value = _completed(0, "  \n", "")
    assert lc.run_json(["x"]) is None


@patch("roostery.lark_cli.subprocess.run")
def test_run_json_timeout(mock_run):
    mock_run.side_effect = subprocess.TimeoutExpired("lark-cli", 5)
    with pytest.raises(lc.LarkCLIError) as exc:
        lc.run_json(["x"], timeout=5)
    assert "timeout" in exc.value.msg.lower()
    assert exc.value.retriable is True


@patch("roostery.lark_cli.subprocess.run")
def test_run_json_binary_not_found(mock_run):
    mock_run.side_effect = FileNotFoundError()
    with pytest.raises(lc.LarkCLIError, match="binary not found"):
        lc.run_json(["x"])


@patch("roostery.lark_cli.subprocess.run")
def test_run_json_retries_on_token_expired(mock_run):
    mock_run.side_effect = [
        _completed(1, '{"code":99991663,"msg":"expired"}', ""),
        _completed(0, '{"ok":true}', ""),
    ]
    assert lc.run_json(["x"]) == {"ok": True}
    assert mock_run.call_count == 2


@patch("roostery.lark_cli.subprocess.run")
def test_run_json_no_retry_on_business_error(mock_run):
    mock_run.return_value = _completed(1, '{"code":1234,"msg":"x"}', "")
    with pytest.raises(lc.LarkCLIError):
        lc.run_json(["x"])
    assert mock_run.call_count == 1


def test_run_json_uses_env_binary(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_LARK_CLI_BIN", "/custom/lark-cli")
    with patch("roostery.lark_cli.subprocess.run") as mock_run:
        mock_run.return_value = _completed(0, "{}", "")
        lc.run_json(["x"])
        assert mock_run.call_args[0][0][0] == "/custom/lark-cli"


# ---- im_send_text ----------------------------------------------------------

@patch("roostery.lark_cli.subprocess.run")
def test_im_send_text_returns_message_id(mock_run):
    mock_run.return_value = _completed(0,
        json.dumps({"data": {"message_id": "om_abc"}}), "")
    mid = lc.im_send_text(user_id="ou_x", text="hi",
                          idempotency_key="k1")
    assert mid == "om_abc"
    argv = mock_run.call_args[0][0]
    assert "im" in argv and "+messages-send" in argv
    assert "--user-id" in argv and argv[argv.index("--user-id") + 1] == "ou_x"
    assert "--text" in argv and argv[argv.index("--text") + 1] == "hi"
    assert "--idempotency-key" in argv


@patch("roostery.lark_cli.subprocess.run")
def test_im_send_text_without_idempotency_key(mock_run):
    mock_run.return_value = _completed(0, '{"data":{"message_id":"x"}}', "")
    lc.im_send_text(user_id="ou_x", text="hi")
    argv = mock_run.call_args[0][0]
    assert "--idempotency-key" not in argv


# ---- im_messages_reply (M3.C T4) -------------------------------------------

@patch("roostery.lark_cli.subprocess.run")
def test_im_messages_reply_returns_message_id(mock_run):
    mock_run.return_value = _completed(0,
        json.dumps({"data": {"message_id": "om_reply"}}), "")
    mid = lc.im_messages_reply(message_id="om_src", text="ack",
                               reply_in_thread=True)
    assert mid == "om_reply"
    argv = mock_run.call_args[0][0]
    assert argv[1:3] == ["im", "+messages-reply"]
    assert "--message-id" in argv and argv[argv.index("--message-id") + 1] == "om_src"
    assert "--text" in argv and argv[argv.index("--text") + 1] == "ack"
    assert "--reply-in-thread" in argv


@patch("roostery.lark_cli.subprocess.run")
def test_im_messages_reply_omits_thread_flag_when_false(mock_run):
    mock_run.return_value = _completed(0, '{"data":{"message_id":"x"}}', "")
    lc.im_messages_reply(message_id="om_src", text="ack", reply_in_thread=False)
    argv = mock_run.call_args[0][0]
    assert "--reply-in-thread" not in argv


@patch("roostery.lark_cli.subprocess.run")
def test_im_messages_reply_routes_profile_global_flag(mock_run):
    mock_run.return_value = _completed(0, '{"data":{"message_id":"x"}}', "")
    lc.im_messages_reply(message_id="om_src", text="ack",
                         reply_in_thread=True, profile="cli_other")
    argv = mock_run.call_args[0][0]
    # --profile 是 global flag，必须在子命令前
    assert "--profile" in argv
    pi = argv.index("--profile")
    im_i = argv.index("im")
    assert pi < im_i, f"--profile must precede 'im' subcommand, got {argv}"
    assert argv[pi + 1] == "cli_other"


# ---- run_json + profile kwarg ----------------------------------------------

@patch("roostery.lark_cli.subprocess.run")
def test_run_json_inserts_profile_before_subcommand(mock_run):
    mock_run.return_value = _completed(0, "{}", "")
    lc.run_json(["im", "+messages-send", "--text", "x"], profile="cli_other")
    argv = mock_run.call_args[0][0]
    # binary first, --profile before subcommand
    pi = argv.index("--profile")
    im_i = argv.index("im")
    assert pi == 1 and im_i > pi, f"expected --profile right after binary, got {argv}"
    assert argv[pi + 1] == "cli_other"


@patch("roostery.lark_cli.subprocess.run")
def test_run_json_no_profile_when_none(mock_run):
    mock_run.return_value = _completed(0, "{}", "")
    lc.run_json(["auth", "status"])
    argv = mock_run.call_args[0][0]
    assert "--profile" not in argv


# ---- docs_create_v2 / docs_update_overwrite ---------------------------------

@patch("roostery.lark_cli.subprocess.run")
def test_docs_create_v2_returns_token_and_url(mock_run):
    """create 触发后会跟 drive +move（如果有 parent_token）；mock 全程。"""
    mock_run.side_effect = [
        # 1. docs +create
        _completed(0, json.dumps({
            "data": {"document": {"document_id": "doxcnXXX",
                                  "url": "https://docs.feishu.cn/docx/doxcnXXX"}}
        }), ""),
        # 2. drive +move
        _completed(0, "{}", ""),
    ]
    info = lc.docs_create_v2(parent_token="fldcnROOT", markdown="# hi")
    assert info.doc_token == "doxcnXXX"
    assert info.url == "https://docs.feishu.cn/docx/doxcnXXX"
    # 第一次调用是 docs +create（不再包含 --folder-token，因为它是哑 flag）
    first_argv = mock_run.call_args_list[0][0][0]
    assert "docs" in first_argv and "+create" in first_argv
    assert first_argv[first_argv.index("--api-version") + 1] == "v2"
    assert "--folder-token" not in first_argv
    assert mock_run.call_args_list[0].kwargs["input"] == "# hi"
    # 第二次是 drive +move
    second_argv = mock_run.call_args_list[1][0][0]
    assert "drive" in second_argv and "+move" in second_argv
    assert second_argv[second_argv.index("--file-token") + 1] == "doxcnXXX"
    assert second_argv[second_argv.index("--folder-token") + 1] == "fldcnROOT"


@patch("roostery.lark_cli.subprocess.run")
def test_docs_create_v2_without_parent_skips_move(mock_run):
    mock_run.return_value = _completed(0, json.dumps({
        "data": {"document": {"document_id": "doxcnX"}}}), "")
    lc.docs_create_v2(parent_token="", markdown="x")
    # 只调一次：create，没有 move
    assert mock_run.call_count == 1


@patch("roostery.lark_cli.subprocess.run")
def test_docs_create_v2_with_title_does_create_move_update(mock_run):
    """title 通过后续 docs +update --new-title 设置（--title 在 create 上是哑 flag）。"""
    mock_run.side_effect = [
        _completed(0, '{"data":{"document":{"document_id":"doxcnX"}}}', ""),
        _completed(0, "{}", ""),  # drive +move
        _completed(0, "{}", ""),  # docs +update --new-title
    ]
    lc.docs_create_v2(parent_token="fldcnROOT", markdown="x", title="日报 2026-05-12")
    assert mock_run.call_count == 3
    third_argv = mock_run.call_args_list[2][0][0]
    assert "+update" in third_argv
    assert third_argv[third_argv.index("--new-title") + 1] == "日报 2026-05-12"
    assert third_argv[third_argv.index("--mode") + 1] == "overwrite"


@patch("roostery.lark_cli.subprocess.run")
def test_docs_create_v2_missing_token_raises(mock_run):
    mock_run.return_value = _completed(0, '{"data":{}}', "")
    with pytest.raises(lc.LarkCLIError, match="missing document_id"):
        lc.docs_create_v2(parent_token="x", markdown="x")


@patch("roostery.lark_cli.subprocess.run")
def test_docs_update_overwrite(mock_run):
    mock_run.return_value = _completed(0, '{"data":{}}', "")
    lc.docs_update_overwrite(doc_token="doxcnX", markdown="# new",
                              title="新标题")
    argv = mock_run.call_args[0][0]
    assert "+update" in argv
    assert argv[argv.index("--mode") + 1] == "overwrite"
    assert argv[argv.index("--doc") + 1] == "doxcnX"
    assert argv[argv.index("--new-title") + 1] == "新标题"
    assert mock_run.call_args.kwargs["input"] == "# new"


# ---- drive ---------------------------------------------------------------

@patch("roostery.lark_cli.subprocess.run")
def test_drive_list_folder_parses_entries(mock_run):
    mock_run.return_value = _completed(0, json.dumps({
        "data": {"files": [
            {"name": "2026-05", "token": "fldcnM", "type": "folder"},
            {"name": "日报 2026-05-12", "token": "doxcnX", "type": "docx"},
            {"name": "garbage"},  # 缺 token，应被跳过
        ]}
    }), "")
    entries = lc.drive_list_folder(folder_token="fldcnROOT")
    assert len(entries) == 2
    assert entries[0].name == "2026-05" and entries[0].type == "folder"
    assert entries[1].token == "doxcnX"
    argv = mock_run.call_args[0][0]
    assert "drive" in argv and "files" in argv and "list" in argv
    params = json.loads(argv[argv.index("--params") + 1])
    assert params == {"folder_token": "fldcnROOT", "page_size": 200}
    assert argv[argv.index("--as") + 1] == "user"


@patch("roostery.lark_cli.subprocess.run")
def test_drive_create_folder_returns_token(mock_run):
    mock_run.return_value = _completed(0,
        '{"data":{"token":"fldcnNEW","name":"2026-05"}}', "")
    tk = lc.drive_create_folder(parent_token="fldcnROOT", name="2026-05")
    assert tk == "fldcnNEW"


@patch("roostery.lark_cli.subprocess.run")
def test_find_or_create_folder_reuses_existing(mock_run):
    mock_run.return_value = _completed(0, json.dumps({
        "data": {"files": [{"name": "2026-05", "token": "fldcnEXIST",
                            "type": "folder"}]}
    }), "")
    tk = lc.find_or_create_folder(parent_token="fldcnROOT", name="2026-05")
    assert tk == "fldcnEXIST"
    assert mock_run.call_count == 1  # 没创建


@patch("roostery.lark_cli.subprocess.run")
def test_find_or_create_folder_creates_when_absent(mock_run):
    mock_run.side_effect = [
        _completed(0, '{"data":{"files":[]}}', ""),
        _completed(0, '{"data":{"token":"fldcnNEW"}}', ""),
    ]
    tk = lc.find_or_create_folder(parent_token="fldcnROOT", name="x")
    assert tk == "fldcnNEW"
    assert mock_run.call_count == 2


@patch("roostery.lark_cli.subprocess.run")
def test_find_doc_in_folder_exact_match(mock_run):
    mock_run.return_value = _completed(0, json.dumps({
        "data": {"files": [
            {"name": "日报 2026-05-12", "token": "doxcnA", "type": "docx"},
            {"name": "日报 2026-05-12", "token": "fldcnB", "type": "folder"},
            {"name": "其它", "token": "doxcnC", "type": "docx"},
        ]}
    }), "")
    tk = lc.find_doc_in_folder(folder_token="fldcnM", title="日报 2026-05-12")
    assert tk == "doxcnA"


@patch("roostery.lark_cli.subprocess.run")
def test_find_doc_in_folder_miss_returns_none(mock_run):
    mock_run.return_value = _completed(0, '{"data":{"files":[]}}', "")
    assert lc.find_doc_in_folder(folder_token="x", title="y") is None
