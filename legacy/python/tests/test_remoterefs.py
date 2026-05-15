"""roostery.remoterefs 单测。"""
import json

from roostery import remoterefs


def test_empty_when_no_stdout():
    refs = remoterefs.extract(["im", "+messages-send"], b"")
    assert refs == {"message_id": None, "doc_token": None,
                    "folder_token": None, "record_id": None}


def test_extract_message_id_from_im_send():
    body = {"code": 0, "data": {"message_id": "om_abcdef"}}
    refs = remoterefs.extract(["im", "+messages-send"],
                              json.dumps(body).encode())
    assert refs["message_id"] == "om_abcdef"
    assert refs["doc_token"] is None


def test_extract_doc_token_v2_create():
    body = {"code": 0, "data": {"document": {"document_id": "doxcnXXX",
                                              "url": "https://..."}}}
    refs = remoterefs.extract(["docs", "+create", "--api-version", "v2"],
                              json.dumps(body))
    assert refs["doc_token"] == "doxcnXXX"


def test_extract_folder_token_only_when_create_folder():
    body = {"data": {"token": "fldcnYYY", "name": "2026-05"}}
    refs_create = remoterefs.extract(
        ["drive", "+create-folder", "--name", "x"],
        json.dumps(body),
    )
    refs_other = remoterefs.extract(
        ["drive", "files", "list"],
        json.dumps(body),
    )
    assert refs_create["folder_token"] == "fldcnYYY"
    assert refs_other["folder_token"] is None


def test_extract_record_id_from_base():
    body = {"data": {"record": {"record_id": "rec_zzz", "fields": {}}}}
    refs = remoterefs.extract(["base", "+record-create"], json.dumps(body))
    assert refs["record_id"] == "rec_zzz"


def test_malformed_json_returns_empty():
    refs = remoterefs.extract(["x"], b"not json {{{")
    assert all(v is None for v in refs.values())


def test_text_starting_with_log_lines_skipped():
    refs = remoterefs.extract(["x"], "INFO loading...\n{\"message_id\":\"X\"}")
    # text 不以 '{' 开头，按设计跳过解析
    assert refs["message_id"] is None


def test_non_string_value_ignored():
    body = {"data": {"message_id": 12345}}
    refs = remoterefs.extract(["im", "+messages-send"], json.dumps(body))
    assert refs["message_id"] is None
