import os
from pathlib import Path
import pytest

from roostery.runner_registry import RunnerEntry, RunnerRegistry


@pytest.fixture
def registry(tmp_path, monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_HOME", str(tmp_path))
    return RunnerRegistry()


def _entry(guid="t1", pid=12345, chat_id="oc_test"):
    return RunnerEntry(
        task_guid=guid, task_url=f"https://feishu.cn/task/{guid}",
        runner_pid=pid, bot_app_id="cli_x", chat_id=chat_id,
        source_message_id="om_x", started_at="2026-05-13T22:30:00+08:00",
    )


def test_register_then_lookup_returns_entry(registry):
    e = _entry()
    registry.register(e)
    assert registry.lookup("t1") == e


def test_lookup_unknown_returns_none(registry):
    assert registry.lookup("missing") is None


def test_unregister_removes_entry(registry):
    registry.register(_entry())
    registry.unregister("t1")
    assert registry.lookup("t1") is None


def test_unregister_unknown_is_noop(registry):
    registry.unregister("never-existed")  # no exception


def test_write_and_read_abort_sentinel(registry):
    registry.register(_entry())
    registry.write_abort_sentinel("t1", "/stop")
    assert registry.read_abort_sentinel("t1") == "/stop"


def test_read_abort_sentinel_returns_none_when_absent(registry):
    registry.register(_entry())
    assert registry.read_abort_sentinel("t1") is None


def test_unregister_also_cleans_sentinel(registry):
    registry.register(_entry())
    registry.write_abort_sentinel("t1", "/stop")
    registry.unregister("t1")
    assert registry.read_abort_sentinel("t1") is None


def test_lookup_by_chat_id_returns_match(registry):
    registry.register(_entry(guid="t1", pid=1, chat_id="oc_alpha"))
    registry.register(_entry(guid="t2", pid=2, chat_id="oc_beta"))
    e = registry.lookup_by_chat_id("oc_alpha")
    assert e is not None and e.task_guid == "t1"


def test_lookup_by_chat_id_returns_none(registry):
    registry.register(_entry(chat_id="oc_alpha"))
    assert registry.lookup_by_chat_id("oc_other") is None


def test_lookup_by_chat_id_returns_most_recent_when_multiple(registry):
    import dataclasses
    from roostery.runner_registry import RunnerEntry
    old = _entry(guid="t_old", pid=1, chat_id="oc_x")
    registry.register(dataclasses.replace(old, started_at="2026-01-01T00:00:00+08:00"))
    new = _entry(guid="t_new", pid=2, chat_id="oc_x")
    registry.register(dataclasses.replace(new, started_at="2026-05-13T22:30:00+08:00"))
    e = registry.lookup_by_chat_id("oc_x")
    assert e.task_guid == "t_new"


def test_cleanup_orphans_removes_dead_pids(registry, monkeypatch):
    registry.register(_entry(guid="alive", pid=os.getpid()))
    registry.register(_entry(guid="dead", pid=999999))  # 假设 999999 不存在

    def fake_pid_alive(pid):
        return pid == os.getpid()

    monkeypatch.setattr("roostery.runner_registry._pid_alive", fake_pid_alive)
    n = registry.cleanup_orphans()
    assert n == 1
    assert registry.lookup("alive") is not None
    assert registry.lookup("dead") is None


def test_write_and_read_adjust_sentinel(registry):
    registry.register(_entry())
    registry.write_adjust_sentinel("t1", "加点细节")
    assert registry.read_adjust_sentinel("t1") == "加点细节"


def test_read_adjust_sentinel_returns_none_when_absent(registry):
    registry.register(_entry())
    assert registry.read_adjust_sentinel("t1") is None


def test_unregister_also_cleans_adjust_sentinel(registry):
    registry.register(_entry())
    registry.write_adjust_sentinel("t1", "x")
    registry.write_abort_sentinel("t1", "/stop")
    registry.unregister("t1")
    assert registry.read_adjust_sentinel("t1") is None
    assert registry.read_abort_sentinel("t1") is None


def test_runner_entry_optional_base_fields_default_none():
    e = _entry()
    assert e.record_id is None
    assert e.base_token is None
    assert e.table_id is None


def test_runner_entry_serializes_record_id(registry):
    import dataclasses
    e = dataclasses.replace(
        _entry(),
        record_id="recABC",
        base_token="bascnXYZ",
        table_id="tbl001",
    )
    registry.register(e)
    got = registry.lookup("t1")
    assert got is not None
    assert got.record_id == "recABC"
    assert got.base_token == "bascnXYZ"
    assert got.table_id == "tbl001"


def test_lookup_old_json_without_record_id_field(registry, tmp_path):
    import json
    # 模拟旧版 JSON 文件（缺 record_id/base_token/table_id 字段）
    old = {
        "task_guid": "t_old",
        "task_url": "https://feishu.cn/task/t_old",
        "runner_pid": 1234,
        "bot_app_id": "cli_x",
        "chat_id": "oc_old",
        "source_message_id": "om_old",
        "started_at": "2026-01-01T00:00:00+08:00",
    }
    p = Path(os.environ["FEISHU_HUB_HOME"]) / "state" / "runners" / "t_old.json"
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(old), encoding="utf-8")
    got = registry.lookup("t_old")
    assert got is not None
    assert got.task_guid == "t_old"
    assert got.record_id is None
    assert got.base_token is None
    assert got.table_id is None


def test_lookup_by_record_id_finds_alive_entry(registry, monkeypatch):
    import dataclasses
    e = dataclasses.replace(_entry(guid="t1", pid=os.getpid()), record_id="recAlive")
    registry.register(e)
    monkeypatch.setattr("roostery.runner_registry._pid_alive", lambda pid: True)
    got = registry.lookup_by_record_id("recAlive")
    assert got is not None and got.task_guid == "t1"


def test_lookup_by_record_id_skips_dead_pid(registry, monkeypatch):
    import dataclasses
    e = dataclasses.replace(_entry(guid="t1", pid=999999), record_id="recDead")
    registry.register(e)
    monkeypatch.setattr("roostery.runner_registry._pid_alive", lambda pid: False)
    assert registry.lookup_by_record_id("recDead") is None


def test_lookup_by_record_id_missing_returns_none(registry, monkeypatch):
    monkeypatch.setattr("roostery.runner_registry._pid_alive", lambda pid: True)
    assert registry.lookup_by_record_id("recNope") is None


def test_lookup_by_record_id_ignores_entries_without_record_id_field(registry, monkeypatch):
    # 旧风格 entry（无 record_id）
    registry.register(_entry(guid="t1"))
    monkeypatch.setattr("roostery.runner_registry._pid_alive", lambda pid: True)
    assert registry.lookup_by_record_id("recAny") is None
