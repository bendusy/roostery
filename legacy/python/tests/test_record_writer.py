from unittest.mock import patch
import pytest
from roostery.record_writer import (
    append_product,
    cas_acquire_running,
    mirror_doc_urls,
    set_run_state,
)


def test_set_run_state_writes_correct_payload():
    captured = {}

    def fake_upsert(*, base_token, table_id, record_id, fields, **kw):
        captured.update(fields=fields, record_id=record_id,
                        base_token=base_token, table_id=table_id)
        return record_id

    with patch("roostery.record_writer.base_record_upsert", side_effect=fake_upsert):
        set_run_state(record_id="rec1", state="running",
                      base_token="bt", table_id="tbl")
    assert captured["fields"] == {"运行状态": "running"}
    assert captured["record_id"] == "rec1"


def test_set_run_state_rejects_unknown_state():
    with pytest.raises(ValueError, match="invalid state"):
        set_run_state(record_id="rec1", state="bogus",
                      base_token="bt", table_id="tbl")


def _patch_get_upsert(get_return, capture):
    def fake_get(*, base_token, table_id, record_id, **kw):
        return get_return
    def fake_upsert(*, base_token, table_id, record_id, fields, **kw):
        capture.update(fields=fields, record_id=record_id)
        return record_id
    return (
        patch("roostery.record_writer.base_record_get", side_effect=fake_get),
        patch("roostery.record_writer.base_record_upsert", side_effect=fake_upsert),
    )


def test_append_product_first_write():
    captured = {}
    g, u = _patch_get_upsert({}, captured)
    with g, u:
        append_product(record_id="rec1", text="hello",
                       base_token="bt", table_id="tbl")
    new = captured["fields"]["产物"]
    assert "hello" in new
    assert "---" in new


def test_append_product_preserves_old_content():
    captured = {}
    g, u = _patch_get_upsert({"产物": "old chunk"}, captured)
    with g, u:
        append_product(record_id="rec1", text="new chunk",
                       base_token="bt", table_id="tbl")
    new = captured["fields"]["产物"]
    assert "old chunk" in new
    assert "new chunk" in new
    assert new.index("old chunk") < new.index("new chunk")


def test_append_product_handles_old_as_list_form():
    captured = {}
    g, u = _patch_get_upsert({"产物": ["legacy"]}, captured)
    with g, u:
        append_product(record_id="rec1", text="fresh",
                       base_token="bt", table_id="tbl")
    new = captured["fields"]["产物"]
    assert "legacy" in new
    assert "fresh" in new


def _cas_patches(get_returns, upsert_capture):
    """get_returns: list of dicts (consumed in order)."""
    calls = {"n": 0}

    def fake_get(*, base_token, table_id, record_id, **kw):
        idx = min(calls["n"], len(get_returns) - 1)
        calls["n"] += 1
        return get_returns[idx]

    def fake_upsert(*, base_token, table_id, record_id, fields, **kw):
        upsert_capture.append(fields)
        return record_id

    return (
        patch("roostery.record_writer.base_record_get", side_effect=fake_get),
        patch("roostery.record_writer.base_record_upsert", side_effect=fake_upsert),
        patch("roostery.record_writer.time.sleep", return_value=None),
    )


def test_cas_acquire_running_success():
    upserts = []
    # First get: idle. Second get: marker = ours (filled in after upsert).
    state = {"records": [{"运行状态": "idle"}, {}]}

    def fake_get(*, base_token, table_id, record_id, **kw):
        return state["records"].pop(0) if state["records"] else {}

    def fake_upsert(*, base_token, table_id, record_id, fields, **kw):
        upserts.append(fields)
        # Simulate write-through: the second get should see our marker.
        state["records"] = [{"_last_writer_marker": fields["_last_writer_marker"]}]
        return record_id

    with patch("roostery.record_writer.base_record_get", side_effect=fake_get), \
         patch("roostery.record_writer.base_record_upsert", side_effect=fake_upsert), \
         patch("roostery.record_writer.time.sleep", return_value=None):
        marker, status = cas_acquire_running(
            record_id="rec1", base_token="bt", table_id="tbl"
        )
    assert status == "ok"
    assert marker is not None
    assert len(upserts) == 1
    assert upserts[0]["运行状态"] == "running"
    assert upserts[0]["_last_writer_marker"] == marker


def test_cas_acquire_running_rejects_non_idle():
    upserts = []
    g, u, s = _cas_patches([{"运行状态": "running"}], upserts)
    with g, u, s:
        marker, status = cas_acquire_running(
            record_id="rec1", base_token="bt", table_id="tbl"
        )
    assert marker is None
    assert status == "non_idle"
    assert upserts == []  # no write performed


def test_cas_acquire_running_detects_conflict():
    upserts = []
    # idle → write ours → re-get returns someone else's marker
    g, u, s = _cas_patches(
        [
            {"运行状态": "idle"},
            {"_last_writer_marker": "someone-else|ts|abcd1234"},
        ],
        upserts,
    )
    with g, u, s:
        marker, status = cas_acquire_running(
            record_id="rec1", base_token="bt", table_id="tbl"
        )
    assert marker is None
    assert status == "concurrent_conflict"
    assert len(upserts) == 1


# ---- M4.D-3.1: mirror_doc_urls ----

def _patch_upsert(capture):
    def fake_upsert(*, base_token, table_id, record_id, fields, **kw):
        capture.append({"base_token": base_token, "table_id": table_id,
                        "record_id": record_id, "fields": fields})
        return record_id
    return patch("roostery.record_writer.base_record_upsert", side_effect=fake_upsert)


def test_mirror_doc_urls_extracts_docx_url():
    captured = []
    with _patch_upsert(captured):
        n = mirror_doc_urls(
            record_id="rec1", target_field="关联文档",
            stdout="see https://feishu.cn/docx/Abc123 done",
            base_token="bt", table_id="tbl",
        )
    assert n == 1
    assert len(captured) == 1
    assert captured[0]["fields"] == {"关联文档": "https://feishu.cn/docx/Abc123"}
    assert captured[0]["record_id"] == "rec1"


def test_mirror_doc_urls_extracts_multiple_types():
    captured = []
    stdout = (
        "doc: https://feishu.cn/docx/Abc123\n"
        "sheet: https://feishu.cn/sheets/Sht456\n"
        "base: https://feishu.cn/base/Bas789\n"
    )
    with _patch_upsert(captured):
        n = mirror_doc_urls(
            record_id="rec1", target_field="关联文档", stdout=stdout,
            base_token="bt", table_id="tbl",
        )
    assert n == 3
    val = captured[0]["fields"]["关联文档"]
    assert "https://feishu.cn/docx/Abc123" in val
    assert "https://feishu.cn/sheets/Sht456" in val
    assert "https://feishu.cn/base/Bas789" in val
    assert val.count("\n") == 2


def test_mirror_doc_urls_dedupes_preserving_order():
    captured = []
    stdout = ("https://feishu.cn/docx/Abc123 ... again "
              "https://feishu.cn/docx/Abc123")
    with _patch_upsert(captured):
        n = mirror_doc_urls(
            record_id="rec1", target_field="关联文档", stdout=stdout,
            base_token="bt", table_id="tbl",
        )
    assert n == 1
    assert captured[0]["fields"]["关联文档"] == "https://feishu.cn/docx/Abc123"


def test_mirror_doc_urls_no_urls_returns_zero_and_no_upsert():
    captured = []
    with _patch_upsert(captured):
        n = mirror_doc_urls(
            record_id="rec1", target_field="关联文档", stdout="no urls here",
            base_token="bt", table_id="tbl",
        )
    assert n == 0
    assert captured == []


def test_mirror_doc_urls_handles_empty_stdout():
    captured = []
    with _patch_upsert(captured):
        n = mirror_doc_urls(
            record_id="rec1", target_field="关联文档", stdout="",
            base_token="bt", table_id="tbl",
        )
    assert n == 0
    assert captured == []
