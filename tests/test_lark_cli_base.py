"""Tests for roostery.lark_cli base_* helpers (Phase 0 of M4.C)."""
from __future__ import annotations

import json
from unittest.mock import patch


# --- base_record_get ------------------------------------------------------

class TestBaseRecordGet:
    @patch("roostery.lark_cli.run_json")
    def test_parses_columnar_response(self, run_json):
        """实测响应（lark-cli 1.0.29 + 公众号-2026, 2026-05-14）是列式：
        data.data[0] = 行值数组；data.fields = 平行列名。"""
        run_json.return_value = {
            "ok": True,
            "data": {
                "data": [["hi", ["📋 选题"], ["cc"], "x"]],
                "fields": ["任务标题", "阶段", "负责 AI", "备注"],
                "field_id_list": ["fldA", "fldB", "fldC", "fldD"],
                "record_id_list": ["rec1"],
                "has_more": False,
            },
        }
        from roostery.lark_cli import base_record_get

        rec = base_record_get(base_token="bt", table_id="tbl", record_id="rec1")
        assert rec == {"任务标题": "hi", "阶段": ["📋 选题"],
                       "负责 AI": ["cc"], "备注": "x"}
        argv = run_json.call_args.args[0]
        assert argv[:2] == ["base", "+record-get"]
        assert "--base-token" in argv and argv[argv.index("--base-token") + 1] == "bt"
        assert "--table-id" in argv and argv[argv.index("--table-id") + 1] == "tbl"
        assert "--record-id" in argv and argv[argv.index("--record-id") + 1] == "rec1"
        assert "--format" in argv and argv[argv.index("--format") + 1] == "json"

    @patch("roostery.lark_cli.run_json")
    def test_returns_empty_when_no_rows(self, run_json):
        run_json.return_value = {
            "ok": True,
            "data": {"data": [], "fields": [], "record_id_list": [], "has_more": False},
        }
        from roostery.lark_cli import base_record_get

        assert base_record_get(base_token="bt", table_id="tbl", record_id="rec1") == {}

    @patch("roostery.lark_cli.run_json")
    def test_raises_on_business_code_nonzero(self, run_json):
        import pytest
        run_json.return_value = {"code": 91234, "msg": "forbidden", "data": {}}
        from roostery.lark_cli import base_record_get, LarkCLIError

        with pytest.raises(LarkCLIError) as exc:
            base_record_get(base_token="bt", table_id="tbl", record_id="rec1")
        assert exc.value.code == 91234
        assert "forbidden" in exc.value.msg


# --- base_record_upsert ---------------------------------------------------

class TestBaseRecordUpsertCreate:
    @patch("roostery.lark_cli.run_json")
    def test_create_returns_extracted_id(self, run_json):
        run_json.return_value = {
            "code": 0,
            "data": {"record": {"record_id_list": ["recNEW"]}},
        }
        from roostery.lark_cli import base_record_upsert

        rid = base_record_upsert(
            base_token="bt", table_id="tbl",
            fields={"任务标题": "x", "阶段": ["📋 选题"]},
        )
        assert rid == "recNEW"
        argv = run_json.call_args.args[0]
        assert "--record-id" not in argv  # create path

    @patch("roostery.lark_cli.run_json")
    def test_create_payload_contains_field_map(self, run_json):
        run_json.return_value = {"data": {"record": {"record_id_list": ["recX"]}}}
        from roostery.lark_cli import base_record_upsert

        base_record_upsert(
            base_token="bt", table_id="tbl",
            fields={"任务标题": "hello", "阶段": ["📋 选题"]},
        )
        argv = run_json.call_args.args[0]
        assert "--json" in argv
        payload = json.loads(argv[argv.index("--json") + 1])
        assert payload == {"任务标题": "hello", "阶段": ["📋 选题"]}


class TestBaseRecordUpsertUpdate:
    @patch("roostery.lark_cli.run_json")
    def test_update_passes_through_record_id(self, run_json):
        run_json.return_value = {"code": 0, "data": {}}
        from roostery.lark_cli import base_record_upsert

        rid = base_record_upsert(
            base_token="bt", table_id="tbl",
            fields={"任务标题": "x"},
            record_id="recOLD",
        )
        assert rid == "recOLD"
        argv = run_json.call_args.args[0]
        assert "--record-id" in argv
        assert argv[argv.index("--record-id") + 1] == "recOLD"


class TestBaseRecordUpsertBusinessCode:
    @patch("roostery.lark_cli.run_json")
    def test_base_record_upsert_raises_on_business_code_nonzero(self, run_json):
        """Update path: run_json exits 0 but body has non-zero business code."""
        import pytest
        run_json.return_value = {"code": 91234, "msg": "forbidden", "data": {}}
        from roostery.lark_cli import base_record_upsert, LarkCLIError

        with pytest.raises(LarkCLIError) as exc:
            base_record_upsert(
                base_token="bt", table_id="tbl",
                fields={"任务标题": "x"},
                record_id="recOLD",
            )
        assert exc.value.code == 91234
        assert "forbidden" in exc.value.msg

    @patch("roostery.lark_cli.run_json")
    def test_base_record_upsert_create_raises_on_business_code_nonzero(self, run_json):
        """Create path: same business-code check applies."""
        import pytest
        run_json.return_value = {"code": 91234, "msg": "forbidden", "data": {}}
        from roostery.lark_cli import base_record_upsert, LarkCLIError

        with pytest.raises(LarkCLIError) as exc:
            base_record_upsert(
                base_token="bt", table_id="tbl",
                fields={"任务标题": "x"},
            )
        assert exc.value.code == 91234
        assert "forbidden" in exc.value.msg


# --- base_record_search ---------------------------------------------------

class TestBaseRecordSearch:
    @patch("roostery.lark_cli.run_json")
    def test_returns_items_list(self, run_json):
        run_json.return_value = {
            "code": 0,
            "data": {"items": [{"record_id": "r1"}, {"record_id": "r2"}]},
        }
        from roostery.lark_cli import base_record_search

        items = base_record_search(
            base_token="bt", table_id="tbl", keyword="hello",
        )
        assert len(items) == 2
        assert items[0]["record_id"] == "r1"

    @patch("roostery.lark_cli.run_json")
    def test_filter_propagates_into_json(self, run_json):
        run_json.return_value = {"data": {"items": []}}
        from roostery.lark_cli import base_record_search

        base_record_search(
            base_token="bt", table_id="tbl",
            keyword="alpha", page_size=50,
        )
        argv = run_json.call_args.args[0]
        assert "+record-search" in argv
        assert "--json" in argv
        payload = json.loads(argv[argv.index("--json") + 1])
        assert payload.get("keyword") == "alpha"
        assert payload.get("limit") == 50


# --- base_record_list -----------------------------------------------------

class TestBaseRecordList:
    @patch("roostery.lark_cli.run_json")
    def test_parses_columnar_response(self, run_json):
        """实测响应是列式：data.data 行二维数组 + 平行 fields/record_id_list；
        helper 内部 zip 成上层期望的 {items: [{record_id, fields}], has_more}。"""
        run_json.return_value = {
            "ok": True,
            "data": {
                "data": [["a", ["running"]], ["b", ["idle"]]],
                "fields": ["任务标题", "运行状态"],
                "field_id_list": ["fldA", "fldB"],
                "record_id_list": ["rec1", "rec2"],
                "has_more": False,
            },
        }
        from roostery.lark_cli import base_record_list

        data = base_record_list(base_token="bt", table_id="tbl")
        assert data["has_more"] is False
        assert len(data["items"]) == 2
        assert data["items"][0] == {
            "record_id": "rec1",
            "fields": {"任务标题": "a", "运行状态": ["running"]},
        }
        assert data["items"][1] == {
            "record_id": "rec2",
            "fields": {"任务标题": "b", "运行状态": ["idle"]},
        }
        argv = run_json.call_args.args[0]
        assert argv[:2] == ["base", "+record-list"]
        assert "--base-token" in argv and argv[argv.index("--base-token") + 1] == "bt"
        assert "--table-id" in argv and argv[argv.index("--table-id") + 1] == "tbl"
        assert "--format" in argv and argv[argv.index("--format") + 1] == "json"
        assert "--limit" in argv and argv[argv.index("--limit") + 1] == "100"
        assert "--offset" in argv and argv[argv.index("--offset") + 1] == "0"

    @patch("roostery.lark_cli.run_json")
    def test_empty_rows_yields_empty_items(self, run_json):
        run_json.return_value = {
            "ok": True,
            "data": {
                "data": [], "fields": ["a", "b"],
                "record_id_list": [], "has_more": False,
            },
        }
        from roostery.lark_cli import base_record_list

        data = base_record_list(base_token="bt", table_id="tbl")
        assert data == {"items": [], "has_more": False}

    @patch("roostery.lark_cli.run_json")
    def test_base_record_list_passes_view_id_when_given(self, run_json):
        run_json.return_value = {
            "ok": True,
            "data": {"data": [], "fields": [], "record_id_list": [], "has_more": False},
        }
        from roostery.lark_cli import base_record_list

        base_record_list(base_token="bt", table_id="tbl", view_id="vwAbc")
        argv = run_json.call_args.args[0]
        assert "--view-id" in argv
        assert argv[argv.index("--view-id") + 1] == "vwAbc"

    @patch("roostery.lark_cli.run_json")
    def test_base_record_list_raises_on_business_code_nonzero(self, run_json):
        run_json.return_value = {"code": 99, "msg": "boom", "data": {}}
        from roostery.lark_cli import base_record_list, LarkCLIError
        import pytest

        with pytest.raises(LarkCLIError) as exc:
            base_record_list(base_token="bt", table_id="tbl")
        assert exc.value.code == 99
        assert "boom" in exc.value.msg


# --- base_record_delete ---------------------------------------------------

class TestBaseRecordDelete:
    @patch("roostery.lark_cli.run_json")
    def test_argv_contains_yes_and_record_id(self, run_json):
        run_json.return_value = {"code": 0, "data": {}}
        from roostery.lark_cli import base_record_delete

        result = base_record_delete(
            base_token="bt", table_id="tbl", record_id="recDEL",
        )
        assert result is None
        argv = run_json.call_args.args[0]
        assert "+record-delete" in argv
        assert "--yes" in argv
        assert "--record-id" in argv
        assert argv[argv.index("--record-id") + 1] == "recDEL"
