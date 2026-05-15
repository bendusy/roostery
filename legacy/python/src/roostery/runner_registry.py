"""runner_registry — 飞书 task_guid ↔ runner 子进程 PID 映射（R5 HITL POC）。

state_dir：``~/.feishu_hub/state/runners/`` （受 FEISHU_HUB_HOME 覆盖）
  - ``<task_guid>.json`` — RunnerEntry 序列化
  - ``<task_guid>.abort`` — 文本 sentinel，内容为 abort reason（"/stop" 等）

register / unregister 必须配对；unregister 同时删 sentinel 防泄漏。
cleanup_orphans 在 daemon 启动时调用——按 PID 存活状态清孤儿。
"""
from __future__ import annotations

import json
import os
import re
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Optional


_SAFE = re.compile(r"[^A-Za-z0-9._-]")


@dataclass(frozen=True)
class RunnerEntry:
    task_guid: str
    task_url: str
    runner_pid: int
    bot_app_id: str
    chat_id: str  # 新增（route B：hitl_router 按 chat_id 反查）
    source_message_id: str
    started_at: str  # ISO8601
    record_id: Optional[str] = None  # M4.C：Base 记录 record_id
    base_token: Optional[str] = None  # M4.C：Base app_token
    table_id: Optional[str] = None  # M4.C：Base table_id


def _state_root() -> Path:
    home = os.getenv("FEISHU_HUB_HOME")
    base = Path(home) if home else Path.home() / ".roostery"
    d = base / "state" / "runners"
    d.mkdir(parents=True, exist_ok=True)
    return d


def _safe(guid: str) -> str:
    return _SAFE.sub("_", guid) or "unknown"


def _pid_alive(pid: int) -> bool:
    if pid <= 0:
        return False
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


class RunnerRegistry:
    def __init__(self, root: Optional[Path] = None) -> None:
        self._root = root or _state_root()

    def _entry_path(self, task_guid: str) -> Path:
        return self._root / f"{_safe(task_guid)}.json"

    def _sentinel_path(self, task_guid: str) -> Path:
        return self._root / f"{_safe(task_guid)}.abort"

    def register(self, entry: RunnerEntry) -> None:
        self._entry_path(entry.task_guid).write_text(
            json.dumps(asdict(entry), ensure_ascii=False),
            encoding="utf-8",
        )

    def unregister(self, task_guid: str) -> None:
        for p in (self._entry_path(task_guid),
                  self._sentinel_path(task_guid),
                  self._adjust_sentinel_path(task_guid)):
            try:
                p.unlink()
            except FileNotFoundError:
                pass

    def lookup(self, task_guid: str) -> Optional[RunnerEntry]:
        p = self._entry_path(task_guid)
        if not p.exists():
            return None
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
            return RunnerEntry(**data)
        except (json.JSONDecodeError, TypeError, OSError):
            return None

    def write_abort_sentinel(self, task_guid: str, reason: str) -> Path:
        p = self._sentinel_path(task_guid)
        p.write_text(reason, encoding="utf-8")
        return p

    def read_abort_sentinel(self, task_guid: str) -> Optional[str]:
        p = self._sentinel_path(task_guid)
        if not p.exists():
            return None
        try:
            return p.read_text(encoding="utf-8")
        except OSError:
            return None

    def _adjust_sentinel_path(self, task_guid: str) -> Path:
        return self._root / f"{_safe(task_guid)}.adjust"

    def write_adjust_sentinel(self, task_guid: str, supplement: str) -> Path:
        p = self._adjust_sentinel_path(task_guid)
        p.write_text(supplement, encoding="utf-8")
        return p

    def read_adjust_sentinel(self, task_guid: str) -> Optional[str]:
        p = self._adjust_sentinel_path(task_guid)
        if not p.exists():
            return None
        try:
            return p.read_text(encoding="utf-8")
        except OSError:
            return None

    def lookup_by_chat_id(self, chat_id: str) -> Optional[RunnerEntry]:
        """扫 state_dir 找 chat_id 匹配的活 entry（POC：1 chat = 1 runner）。
        多于 1 个时返回 started_at 最新的（防 stale entry 干扰）。
        """
        candidates = []
        for p in self._root.glob("*.json"):
            try:
                data = json.loads(p.read_text(encoding="utf-8"))
                entry = RunnerEntry(**data)
            except (json.JSONDecodeError, TypeError, OSError):
                continue
            if entry.chat_id == chat_id:
                candidates.append(entry)
        if not candidates:
            return None
        return max(candidates, key=lambda e: e.started_at)

    def lookup_by_record_id(self, record_id: str) -> Optional[RunnerEntry]:
        """扫 state_dir 找 record_id 匹配的活 entry（M4.C：Base 记录 → runner）。

        注意：``_pid_alive`` 是本机 ``os.kill(pid, 0)``。多机部署下，machine A
        register 的 runner pid 在 machine B 上不存在，B 调本方法会返回 ``None``，
        让二次 ``/run`` 通过。多机的并发控制依赖 ``record_writer.cas_acquire_running``
        的 ``_last_writer_marker`` 字段（飞书侧共享），不靠 registry。
        """
        for p in self._root.glob("*.json"):
            try:
                data = json.loads(p.read_text(encoding="utf-8"))
            except (json.JSONDecodeError, OSError):
                continue
            if data.get("record_id") != record_id:
                continue
            try:
                entry = RunnerEntry(**data)
            except TypeError:
                continue
            if _pid_alive(entry.runner_pid):
                return entry
        return None

    def cleanup_orphans(self) -> int:
        cleaned = 0
        for p in self._root.glob("*.json"):
            try:
                entry = RunnerEntry(**json.loads(p.read_text(encoding="utf-8")))
            except (json.JSONDecodeError, TypeError, OSError):
                p.unlink(missing_ok=True)
                cleaned += 1
                continue
            if not _pid_alive(entry.runner_pid):
                self.unregister(entry.task_guid)
                cleaned += 1
        return cleaned
