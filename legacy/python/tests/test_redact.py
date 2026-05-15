"""roostery.redact 单测。"""
from roostery import redact


def test_scrub_argv_flag_value():
    new, paths = redact.scrub_argv(
        ["docs", "+create", "--app-secret", "supersecret", "--title", "hi"]
    )
    assert new[3] == "***"
    assert new[1] == "+create"
    assert new[5] == "hi"
    assert paths == ["argv[3]"]


def test_scrub_argv_flag_equals_value():
    new, paths = redact.scrub_argv(
        ["call", "--access-token=abc.def.ghi", "--user", "x"]
    )
    assert new[1] == "--access-token=***"
    assert paths == ["argv[1]"]


def test_scrub_argv_header_authorization():
    new, paths = redact.scrub_argv(
        ["api", "--header", "Authorization: Bearer abcdef", "--header", "X-Trace: 1"]
    )
    assert new[2] == "Authorization: ***"
    assert new[4] == "X-Trace: 1"
    assert paths == ["argv[2]"]


def test_scrub_argv_underscore_dash_equivalence():
    new, paths = redact.scrub_argv(["--user_access_token", "tk", "--ok", "1"])
    assert new[1] == "***"
    assert paths == ["argv[1]"]


def test_scrub_argv_returns_new_list():
    src = ["--access-token", "v"]
    new, _ = redact.scrub_argv(src)
    assert src[1] == "v"
    assert new[1] == "***"


def test_scrub_argv_no_value_after_flag():
    new, paths = redact.scrub_argv(["--access-token"])
    assert new == ["--access-token"]
    assert paths == []


def test_scrub_text_json_form():
    text = '{"app_secret":"abc","name":"x","access_token": "longtoken"}'
    out = redact.scrub_text(text)
    assert "abc" not in out
    assert "longtoken" not in out
    assert "***" in out
    assert "\"name\":\"x\"" in out


def test_scrub_text_yaml_form():
    text = "user: ben\napp_secret: deadbeef\nport: 80\n"
    out = redact.scrub_text(text)
    assert "deadbeef" not in out
    assert "***" in out
    assert "user: ben" in out


def test_scrub_text_bytes_input():
    out = redact.scrub_text(b'{"access_token":"x"}')
    assert "x" not in out.split('"access_token"')[1]
    assert "***" in out


def test_scrub_text_case_insensitive():
    text = '{"Access_Token":"X"}'
    out = redact.scrub_text(text)
    assert "X" not in out.split(":")[1]
    assert "***" in out
