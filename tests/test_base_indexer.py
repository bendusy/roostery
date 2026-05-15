"""base_indexer 测试：summary 解析 + task → record 字段映射 + cursor 持久化 + upsert 路由。"""
from __future__ import annotations

import json
from unittest.mock import patch

import pytest

from roostery.base_indexer import (
    IndexerRunSummary,
    _filter_incremental,
    _ts_to_iso_ms,
    load_cursor,
    parse_summary,
    run_indexer,
    save_cursor,
    task_to_record,
    upsert_record,
)


# --- parse_summary 解析 ---------------------------------------------------

class TestParseSummary:
    def test_full_with_host_suffix(self):
        r = parse_summary("[cc] @ GenericAgent · M4")
        assert r == {"agent": "cc", "cwd_basename": "GenericAgent", "host": "M4"}

    def test_no_host_suffix(self):
        r = parse_summary("[codex] @ MyRepo")
        assert r["agent"] == "codex"
        assert r["cwd_basename"] == "MyRepo"
        assert r["host"] == ""

    def test_malformed_returns_empty(self):
        r = parse_summary("just a sentence")
        assert r == {"agent": "", "cwd_basename": "", "host": ""}

    def test_chinese_basename(self):
        r = parse_summary("[cc] @ 我的项目 · axis")
        assert r["agent"] == "cc"
        assert r["cwd_basename"] == "我的项目"
        assert r["host"] == "axis"


# --- task_to_record ------------------------------------------------------

class TestTaskToRecord:
    def test_basic_fields(self):
        task = {
            "guid": "abc-123",
            "summary": "[cc] @ Foo · M4",
            "status": "done",
            "url": "https://applink.feishu.cn/client/todo/detail?guid=abc-123",
            "creator": {"id": "cli_xxx", "type": "app"},
            "members": [
                {"id": "ou_user1", "role": "assignee", "type": "user"},
                {"id": "ou_user2", "role": "follower", "type": "user"},
            ],
        }
        f = task_to_record(task)
        assert f["task_guid"] == "abc-123"
        assert f["状态"] == "已完成"
        assert f["Agent"] == "cc"
        assert f["host"] == "M4"
        assert f["creator_app_id"] == "cli_xxx"
        assert f["assignee"] == "ou_user1"  # 第一个 role=assignee
        assert "last_synced" in f  # 应填当前时间戳

    def test_status_mapping(self):
        for status, want in [("todo", "排队中"), ("doing", "进行中"), ("done", "已完成"), ("xxx", "进行中")]:
            f = task_to_record({"guid": "g", "summary": "[a] @ b", "status": status})
            assert f["状态"] == want, f"status={status}"

    def test_no_assignee_yields_empty_string(self):
        f = task_to_record({
            "guid": "g", "summary": "[a] @ b",
            "members": [{"id": "ou_x", "role": "follower"}],
        })
        assert f["assignee"] == ""

    def test_created_at_parsed_from_iso_string(self):
        f = task_to_record({
            "guid": "g", "summary": "[a] @ b",
            "created_at": "2026-05-13 10:00:00",
        })
        assert "创建时间" in f
        assert isinstance(f["创建时间"], int)

    def test_created_at_missing_field_skipped(self):
        f = task_to_record({"guid": "g", "summary": "[a] @ b"})
        # 没 created_at 就不写该字段（避免写 None 触发 Base 校验失败）
        assert "创建时间" not in f


# --- _ts_to_iso_ms 时间戳归一化 -----------------------------------------

class TestTsToIsoMs:
    def test_int_seconds(self):
        assert _ts_to_iso_ms(1700000000) == 1700000000000

    def test_int_ms(self):
        assert _ts_to_iso_ms(1700000000000) == 1700000000000

    def test_iso_string(self):
        v = _ts_to_iso_ms("2026-05-13 10:00:00")
        assert isinstance(v, int) and v > 0

    def test_none(self):
        assert _ts_to_iso_ms(None) is None
        assert _ts_to_iso_ms("") is None

    def test_bad_string(self):
        assert _ts_to_iso_ms("not a date") is None


# --- cursor 持久化 -------------------------------------------------------

class TestCursorPersistence:
    def test_load_when_missing_returns_zero(self, tmp_path, monkeypatch):
        monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
        assert load_cursor() == 0

    def test_save_then_load_roundtrip(self, tmp_path, monkeypatch):
        monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
        save_cursor(1778640000000000)
        assert load_cursor() == 1778640000000000

    def test_load_corrupt_file_returns_zero(self, tmp_path, monkeypatch):
        monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
        cursor_path = tmp_path / "state" / "indexer"
        cursor_path.mkdir(parents=True)
        (cursor_path / "cursor.json").write_text("not valid json")
        assert load_cursor() == 0


# --- 增量过滤 ------------------------------------------------------------

class TestFilterIncremental:
    def test_since_zero_returns_all(self):
        tasks = [{"guid": "a", "updated_at": 100}, {"guid": "b", "updated_at": 200}]
        assert _filter_incremental(tasks, since_us=0) == tasks

    def test_keeps_tasks_newer_than_cursor(self):
        # cursor 是 microseconds 单位；100_000_000_000 us = 100 sec ms
        # 我用 updated_at = 100 sec (int)，应被转 ms 100000 与 cursor 100000_us → 100 ms 比
        tasks = [
            {"guid": "old", "updated_at": 100},  # 100 sec → 100000 ms
            {"guid": "new", "updated_at": 200},  # 200 sec → 200000 ms
        ]
        # cursor 150 sec = 150_000 ms = 150_000_000 us
        kept = _filter_incremental(tasks, since_us=150_000_000)
        # since_ms = 150_000，只保留 updated_ms > 150_000，即 200000 (new)
        assert len(kept) == 1
        assert kept[0]["guid"] == "new"


# --- upsert 路由 ---------------------------------------------------------

class TestUpsertRouting:
    @patch("roostery.base_indexer.run_json")
    def test_existing_record_triggers_update(self, run_json):
        run_json.side_effect = [
            # 第一次 record-search 命中（响应形如 {data:{records:[...]}}）
            {"data": {"records": [{"record_id": "rec_x"}]}},
            # 第二次 record-upsert with --record-id
            {"data": {}},
        ]
        action = upsert_record(
            base_token="b", table_id="t",
            fields={"task_guid": "abc", "状态": "已完成"},
        )
        assert action == "updated"
        update_argv = run_json.call_args_list[1].args[0]
        assert "+record-upsert" in update_argv
        # 命中后必须传 --record-id
        assert "--record-id" in update_argv
        assert update_argv[update_argv.index("--record-id") + 1] == "rec_x"

    @patch("roostery.base_indexer.run_json")
    def test_missing_record_triggers_create(self, run_json):
        run_json.side_effect = [
            {"data": {"records": []}},  # search 空
            {"data": {}},  # upsert without record-id = create
        ]
        action = upsert_record(
            base_token="b", table_id="t",
            fields={"task_guid": "abc", "状态": "进行中"},
        )
        assert action == "created"
        create_argv = run_json.call_args_list[1].args[0]
        assert "+record-upsert" in create_argv
        # create 路径不应有 --record-id
        assert "--record-id" not in create_argv

    @patch("roostery.base_indexer.run_json")
    def test_missing_guid_skips(self, run_json):
        action = upsert_record(base_token="b", table_id="t", fields={"状态": "进行中"})
        assert action == "skipped"
        run_json.assert_not_called()


# --- run_indexer 主流程 --------------------------------------------------

class TestRunIndexer:
    @patch("roostery.base_indexer.run_json")
    def test_full_mode_processes_all_tasks(self, run_json, tmp_path, monkeypatch):
        monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
        # 第 1 调：get-related-tasks
        # 第 2 调：search task1（missing）
        # 第 3 调：create task1
        # 第 4 调：search task2（missing）
        # 第 5 调：create task2
        # 所有 task 都需要 creator.type=app（_is_agent_task 过滤）
        run_json.side_effect = [
            {"data": {"items": [
                {"guid": "g1", "summary": "[a] @ x · M4", "status": "todo",
                 "creator": {"id": "cli_x", "type": "app"},
                 "updated_at": 1700000000},
                {"guid": "g2", "summary": "[b] @ y · axis", "status": "done",
                 "creator": {"id": "cli_y", "type": "app"},
                 "updated_at": 1700000100},
            ]}},
            {"data": {"items": []}},
            {"data": {}},
            {"data": {"items": []}},
            {"data": {}},
        ]
        s = run_indexer(base_token="b", table_id="t", full=True)
        assert s.succeeded == 2
        assert s.failed == []
        assert s.total == 2

    @patch("roostery.base_indexer.run_json")
    def test_user_manual_tasks_filtered_out(self, run_json, tmp_path, monkeypatch):
        """user 手动建的 task（creator.type=user）不该进 Base。"""
        monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
        run_json.side_effect = [
            {"data": {"items": [
                # 用户手建 task：creator.type=user
                {"guid": "manual1", "summary": "手写的工作任务", "status": "todo",
                 "creator": {"id": "ou_user", "type": "user"},
                 "updated_at": 1700000000},
                # 一条 agent 建的 task
                {"guid": "agent1", "summary": "[cc] @ proj · M4", "status": "done",
                 "creator": {"id": "cli_bot", "type": "app"},
                 "updated_at": 1700000100},
            ]}},
            # 只有 agent1 走 upsert
            {"data": {"items": []}},
            {"data": {}},
        ]
        s = run_indexer(base_token="b", table_id="t", full=True)
        assert s.succeeded == 1
        assert s.total == 1

    @patch("roostery.base_indexer.run_json")
    def test_single_failure_does_not_block_others(self, run_json, tmp_path, monkeypatch):
        from roostery.lark_cli import LarkCLIError
        monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
        run_json.side_effect = [
            {"data": {"items": [
                {"guid": "g1", "summary": "[a] @ x · M4", "status": "todo",
                 "creator": {"id": "cli_x", "type": "app"}, "updated_at": 1700000000},
                {"guid": "g2", "summary": "[b] @ y · M4", "status": "done",
                 "creator": {"id": "cli_y", "type": "app"}, "updated_at": 1700000100},
            ]}},
            # task1: search 空（不命中）
            {"data": {"items": []}},
            # task1: create 抛错 → 这条记入 failed
            LarkCLIError(code=10000, msg="create denied", argv=["base"]),
            # task2: search 空 + create OK
            {"data": {"items": []}},
            {"data": {}},
        ]
        s = run_indexer(base_token="b", table_id="t", full=True)
        assert s.succeeded == 1
        assert len(s.failed) == 1
        assert s.failed[0]["guid"] == "g1"

    @patch("roostery.base_indexer.run_json")
    def test_cursor_advances_after_run(self, run_json, tmp_path, monkeypatch):
        monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
        run_json.side_effect = [
            {"data": {"items": [
                {"guid": "g", "summary": "[a] @ x", "updated_at": 1700000000,
                 "creator": {"id": "cli_x", "type": "app"}},
            ]}},
            {"data": {"items": []}},
            {"data": {}},
        ]
        run_indexer(base_token="b", table_id="t", full=True)
        # cursor 应推进到该任务的 updated_at（×1000 转 us）
        assert load_cursor() > 0


# --- M4.C Phase 6: reconcile_stale_running -------------------------------

class TestReconcileStaleRunning:
    @patch("roostery.base_indexer.base_record_list")
    def test_marks_orphan_as_failed(self, list_mock, tmp_path, monkeypatch):
        monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
        from roostery.base_config import BaseConfig
        from roostery.runner_registry import RunnerEntry, RunnerRegistry
        from roostery.base_indexer import reconcile_stale_running

        cfg = BaseConfig(role="R", base_token="bt", table_id="tbl",
                        stage_to_bot={"S": "B"})
        list_mock.return_value = {
            "items": [
                {"record_id": "recA", "fields": {"运行状态": ["running"]}},
                {"record_id": "recB", "fields": {"运行状态": ["running"]}},
                {"record_id": "recC", "fields": {"运行状态": ["idle"]}},
            ],
            "has_more": False,
        }

        registry = RunnerRegistry()
        import os
        # only recA has a local live runner
        registry.register(RunnerEntry(
            task_guid="base-recA", task_url="u",
            runner_pid=os.getpid(),
            bot_app_id="cli", chat_id="c", source_message_id="m",
            started_at="2026-01-01T00:00:00",
            record_id="recA", base_token="bt", table_id="tbl",
        ))

        with patch("roostery.record_writer.base_record_upsert") as upsert, \
             patch("roostery.record_writer.base_record_get") as getrec:
            getrec.return_value = {}
            upsert.return_value = "recB"
            n = reconcile_stale_running(configs=[cfg], registry=registry)
        assert n == 1
        # set_run_state + append_product both call upsert → ≥2 calls for the 1 fixed row
        # but only recB should be touched (state set to failed exactly once)
        state_calls = [c for c in upsert.call_args_list
                       if c.kwargs.get("fields", {}).get("运行状态") == "failed"]
        assert len(state_calls) == 1
        assert state_calls[0].kwargs["record_id"] == "recB"

    @patch("roostery.base_indexer.base_record_list")
    def test_paginates_through_has_more(self, list_mock, tmp_path, monkeypatch):
        monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
        from roostery.base_config import BaseConfig
        from roostery.runner_registry import RunnerRegistry
        from roostery.base_indexer import reconcile_stale_running

        cfg = BaseConfig(role="R", base_token="bt", table_id="tbl",
                        stage_to_bot={"S": "B"})
        list_mock.side_effect = [
            {"items": [{"record_id": "rec1", "fields": {"运行状态": ["idle"]}}],
             "has_more": True},
            {"items": [{"record_id": "rec2", "fields": {"运行状态": ["idle"]}}],
             "has_more": False},
        ]
        reconcile_stale_running(configs=[cfg], registry=RunnerRegistry())
        assert list_mock.call_count == 2

    @patch("roostery.base_indexer.base_record_list")
    def test_returns_zero_when_nothing_to_fix(self, list_mock, tmp_path, monkeypatch):
        monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
        from roostery.base_config import BaseConfig
        from roostery.runner_registry import RunnerRegistry
        from roostery.base_indexer import reconcile_stale_running

        cfg = BaseConfig(role="R", base_token="bt", table_id="tbl",
                        stage_to_bot={"S": "B"})
        list_mock.return_value = {"items": [], "has_more": False}
        assert reconcile_stale_running(configs=[cfg], registry=RunnerRegistry()) == 0

    @patch("roostery.base_indexer.base_record_list")
    def test_skips_when_record_list_raises(self, list_mock, tmp_path, monkeypatch):
        """LarkCLIError 时不应炸；该 role 跳过即可（避免阻塞其他 role）。"""
        monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
        from roostery.lark_cli import LarkCLIError
        from roostery.base_config import BaseConfig
        from roostery.runner_registry import RunnerRegistry
        from roostery.base_indexer import reconcile_stale_running

        cfg = BaseConfig(role="R", base_token="bt", table_id="tbl",
                        stage_to_bot={"S": "B"})
        list_mock.side_effect = LarkCLIError(99, "x", ["base"])
        assert reconcile_stale_running(configs=[cfg], registry=RunnerRegistry()) == 0
