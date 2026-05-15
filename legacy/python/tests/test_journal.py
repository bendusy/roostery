"""roostery.journal 单测。"""
import json
import os
import datetime as _dt
from pathlib import Path

import pytest

from roostery import SCHEMA_VERSION, journal


@pytest.fixture
def jdir(tmp_path):
    return tmp_path / "journal"


def test_new_event_id_is_ulid_like():
    eid = journal.new_event_id()
    assert len(eid) == 26
    # Crockford base32 字符集
    allowed = set("0123456789ABCDEFGHJKMNPQRSTVWXYZ")
    assert set(eid) <= allowed


def test_new_event_id_monotonic_prefix():
    a = journal.new_event_id()
    b = journal.new_event_id()
    # ULID 时间前缀单调（同毫秒可能相等）
    assert a[:10] <= b[:10]


def test_now_iso_has_offset():
    s = journal.now_iso()
    # 形如 2026-05-12T14:23:05+08:00
    assert "T" in s
    assert (s.endswith("Z")
            or s[-6] in "+-")


def test_append_writes_jsonl_line(jdir):
    rec = {"event_type": "test", "summary": "hello"}
    p = journal.append(rec, dir_override=jdir)
    assert p.exists()
    lines = p.read_text(encoding="utf-8").strip().split("\n")
    assert len(lines) == 1
    parsed = json.loads(lines[0])
    assert parsed["event_type"] == "test"
    assert parsed["schema_version"] == SCHEMA_VERSION
    assert "event_id" in parsed
    assert "ts" in parsed


def test_append_fills_defaults_but_keeps_caller_values(jdir):
    rec = {"schema_version": 9, "event_id": "FIXED", "ts": "2020-01-01T00:00:00+00:00",
           "event_type": "x"}
    p = journal.append(rec, dir_override=jdir)
    parsed = json.loads(p.read_text().strip())
    assert parsed["schema_version"] == 9
    assert parsed["event_id"] == "FIXED"
    assert parsed["ts"].startswith("2020-01-01")


def test_append_multiple_lines_each_terminated(jdir):
    for i in range(5):
        journal.append({"event_type": "t", "i": i}, dir_override=jdir)
    files = list(jdir.iterdir())
    assert len(files) == 1
    lines = files[0].read_text(encoding="utf-8").splitlines()
    assert len(lines) == 5
    parsed = [json.loads(line) for line in lines]
    assert [r["i"] for r in parsed] == [0, 1, 2, 3, 4]


def test_read_day_filters_bad_lines(jdir):
    journal.append({"event_type": "ok"}, dir_override=jdir)
    # 手动追加一行坏数据
    today = _dt.date.today().strftime("%Y-%m-%d") + ".jsonl"
    (jdir / today).write_text(
        (jdir / today).read_text() + "garbage not json\n",
        encoding="utf-8",
    )
    records = journal.read_day(dir_override=jdir)
    assert len(records) == 1
    assert records[0]["event_type"] == "ok"


def test_read_day_empty_when_missing(jdir):
    assert journal.read_day(dir_override=jdir) == []


def test_actor_from_env(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_AGENT", "cc")
    monkeypatch.setenv("FEISHU_HUB_SESSION", "s1")
    monkeypatch.delenv("FEISHU_HUB_TURN", raising=False)
    a = journal.actor_from_env()
    assert a == {"agent": "cc", "session": "s1", "turn": None}


def test_actor_from_env_unknown_default(monkeypatch):
    monkeypatch.delenv("FEISHU_HUB_AGENT", raising=False)
    a = journal.actor_from_env()
    assert a["agent"] == "unknown"


def test_tags_from_env(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_TAGS", " task_done , reviewed ,")
    assert journal.tags_from_env() == ["task_done", "reviewed"]


def test_tags_empty_when_unset(monkeypatch):
    monkeypatch.delenv("FEISHU_HUB_TAGS", raising=False)
    assert journal.tags_from_env() == []


def test_append_skipped_records_no_journal_reason(jdir):
    p = journal.append_skipped(["auth", "login"], reason="interactive", dir_override=jdir)
    rec = json.loads(p.read_text().strip())
    assert rec["event_type"] == "lark_cli.skipped"
    assert rec["privacy"]["no_journal_reason"] == "interactive"
    assert rec["io"]["tty"] is True
    assert rec["command"]["argv"] == ["auth", "login"]


def test_journal_dir_respects_env(monkeypatch, tmp_path):
    monkeypatch.setenv(journal.ENV_ROOT, str(tmp_path))
    assert journal.journal_dir() == tmp_path / "journal"


def test_envelope_does_not_import_ga():
    """硬性约束：roostery.journal 不得拉起 GA 模块。"""
    import sys
    forbidden = {"ga", "agent_loop", "mykey", "bbs"}
    leaked = forbidden & set(sys.modules.keys())
    # 注：单测进程里可能因其它原因加载了，因此只要 journal 本身不直接导入即可；
    # 真正的运行时隔离测试见 tests/test_no_ga_runtime.py（M1a-T5 加）。
    src = Path(journal.__file__).read_text(encoding="utf-8")
    for name in forbidden:
        assert f"import {name}" not in src
        assert f"from {name}" not in src
