"""journal 写入器：按日切 jsonl，单行原子 append。

envelope schema 详见 ``docs/FEISHU_OFFICE_HUB_DESIGN_V2.md`` §5。
"""
from __future__ import annotations

import datetime as _dt
import json
import os
import secrets
import time
from pathlib import Path
from typing import Any, Dict, List, Optional

from . import SCHEMA_VERSION

DEFAULT_ROOT = Path(os.path.expanduser("~/.feishu_hub"))
DEFAULT_JOURNAL_DIR = DEFAULT_ROOT / "journal"
ENV_ROOT = "FEISHU_HUB_HOME"


# --- ULID（Crockford base32，无外部依赖） -----------------------------------

_CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"


def _encode_b32(num: int, length: int) -> str:
    out = []
    for _ in range(length):
        out.append(_CROCKFORD[num & 0x1F])
        num >>= 5
    return "".join(reversed(out))


def new_event_id() -> str:
    """生成一个 ULID（26 字符，时间排序）。"""
    ms = int(time.time() * 1000)
    rand = int.from_bytes(secrets.token_bytes(10), "big")
    return _encode_b32(ms, 10) + _encode_b32(rand, 16)


# --- 路径与时间 -------------------------------------------------------------

def journal_dir() -> Path:
    root = os.getenv(ENV_ROOT)
    base = Path(root) if root else DEFAULT_ROOT
    return base / "journal"


def now_iso(tz: Optional[_dt.tzinfo] = None) -> str:
    """带本地时区偏移的 ISO-8601 时间串。"""
    now = _dt.datetime.now(tz or _dt.datetime.now().astimezone().tzinfo)
    return now.isoformat(timespec="seconds")


def _today_filename(ts: Optional[_dt.datetime] = None) -> str:
    d = ts or _dt.datetime.now()
    return d.strftime("%Y-%m-%d") + ".jsonl"


# --- 环境快照 ---------------------------------------------------------------

def actor_from_env() -> Dict[str, Any]:
    """读 env 拼装 envelope 的 ``actor`` 字段。

    包含 trace 上下文（``trace_id`` / ``depth`` / ``parent_event_id``）；
    dispatcher 派子进程时塞 env，受派 agent 的 journal envelope 自然带上。
    """
    out: Dict[str, Any] = {
        "agent": os.getenv("FEISHU_HUB_AGENT") or "unknown",
        "session": os.getenv("FEISHU_HUB_SESSION") or None,
        "turn": os.getenv("FEISHU_HUB_TURN") or None,
    }
    trace_id = os.getenv("FEISHU_HUB_TRACE_ID")
    if trace_id:
        out["trace_id"] = trace_id
        try:
            out["depth"] = int(os.getenv("FEISHU_HUB_DEPTH", "0"))
        except ValueError:
            out["depth"] = 0
        parent = os.getenv("FEISHU_HUB_PARENT_EVENT_ID")
        if parent:
            out["parent_event_id"] = parent
    return out


def tags_from_env() -> List[str]:
    raw = os.getenv("FEISHU_HUB_TAGS", "")
    return [t for t in (s.strip() for s in raw.split(",")) if t]


# --- 写入 -------------------------------------------------------------------

def _line(payload: Dict[str, Any]) -> bytes:
    """单行 JSON，UTF-8，换行结尾。"""
    return (json.dumps(payload, ensure_ascii=False, separators=(",", ":")) + "\n").encode("utf-8")


def append(record: Dict[str, Any], *, dir_override: Optional[Path] = None) -> Path:
    """追加一条 envelope 记录到当日 jsonl，返回文件路径。

    缺省字段补全：``schema_version`` / ``event_id`` / ``ts``。其它字段调用方负责。
    POSIX 下小于 PIPE_BUF (4KB) 的 append 写入是原子的；超过时仍按 best-effort。
    """
    record = dict(record)
    record.setdefault("schema_version", SCHEMA_VERSION)
    record.setdefault("event_id", new_event_id())
    record.setdefault("ts", now_iso())

    d = dir_override if dir_override is not None else journal_dir()
    d.mkdir(parents=True, exist_ok=True)
    path = d / _today_filename()
    data = _line(record)
    # O_APPEND 保证多进程 append 不互相截断
    flags = os.O_WRONLY | os.O_CREAT | os.O_APPEND
    fd = os.open(path, flags, 0o644)
    try:
        os.write(fd, data)
    finally:
        os.close(fd)
    return path


def append_skipped(argv: List[str], *, reason: str,
                   dir_override: Optional[Path] = None) -> Path:
    """记录"被跳过未走 journal stdout/stderr"的调用（如交互/NOJOURNAL）。"""
    return append(
        {
            "event_type": "lark_cli.skipped",
            "source": "shim",
            "actor": actor_from_env(),
            "cwd": os.getcwd(),
            "command": {"argv": list(argv), "exit_code": None, "duration_ms": None},
            "io": {
                "stdout_head": "",
                "stderr_head": "",
                "stdin_present": None,
                "tty": reason == "interactive",
            },
            "remote_refs": {"message_id": None, "doc_token": None,
                            "folder_token": None, "record_id": None},
            "summary": None,
            "tags": tags_from_env(),
            "privacy": {"redacted_fields": [], "no_journal_reason": reason},
        },
        dir_override=dir_override,
    )


def read_day(date: Optional[_dt.date] = None,
             *, dir_override: Optional[Path] = None) -> List[Dict[str, Any]]:
    """读取某日的全部记录（解析失败的行被丢弃，不抛异常）。"""
    d = dir_override if dir_override is not None else journal_dir()
    when = date or _dt.date.today()
    path = d / (when.strftime("%Y-%m-%d") + ".jsonl")
    if not path.exists():
        return []
    out: List[Dict[str, Any]] = []
    with path.open("r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                out.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    return out
