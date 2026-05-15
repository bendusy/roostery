"""M3.D — 反向 indexer：从飞书 Task 列表派生 Base 索引表。

协同模型角色：Task 是事实源（agent 在飞书 Task UI 看进度），Base 是
**索引层**——给 Base 的看板/网格/甘特视图提供跨工作项查询能力。

调用方向：**只 Task → Base，不 Base → Task**。用户在 Base 上手改不会
反向同步——索引层不被支持作为输入。

入口：``python -m roostery indexer run [--full]``
- 默认增量模式（用 ``state/indexer/cursor.json`` 的 ``last_updated_at_us``）
- ``--full`` 全量校准（每周日 03:00 launchd 任务跑一次）

依赖 task_writer 不变，纯 read：
1. ``lark-cli task +get-related-tasks --followed-by-me --include-complete``
2. 解析 task summary 拿 agent / cwd / host
3. ``lark-cli base +record-search`` 探 task_guid
4. ``base +record-upsert``（命中 update / 缺失 create）
"""
from __future__ import annotations

import datetime as _dt
import json
import os
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Mapping, Optional

from roostery.lark_cli import LarkCLIError, base_record_list, run_json

# --- summary 解析 ---------------------------------------------------------

# M3.B host suffix 形态：`[<agent>] @ <cwd_basename> · <host>`
# 兼容无 host suffix 的旧任务：`[<agent>] @ <cwd_basename>`
_SUMMARY_RE = re.compile(
    r"^\[(?P<agent>[^\]]+)\]\s*@\s*(?P<cwd>[^·]+?)(?:\s*·\s*(?P<host>\S+))?\s*$"
)


def parse_summary(summary: str) -> Dict[str, str]:
    """summary → ``{agent, cwd_basename, host}``（缺字段返回空串）。"""
    s = (summary or "").strip()
    m = _SUMMARY_RE.match(s)
    if not m:
        return {"agent": "", "cwd_basename": "", "host": ""}
    return {
        "agent": (m.group("agent") or "").strip(),
        "cwd_basename": (m.group("cwd") or "").strip(),
        "host": (m.group("host") or "").strip(),
    }


# --- task → bitable record fields ----------------------------------------

_STATUS_MAP = {
    "todo": "排队中",
    "doing": "进行中",
    "done": "已完成",
    "archived": "已完成",
}


def _ts_to_iso_ms(value: Any) -> Optional[int]:
    """飞书 task API 返回的时间可能是 ``"YYYY-MM-DD HH:MM:SS"`` / Unix sec / ms / None。
    Base time 字段要求 ms 时间戳。无法解析返回 None。"""
    if not value:
        return None
    if isinstance(value, (int, float)):
        v = int(value)
        if v < 10_000_000_000:    # 秒 → 毫秒
            v *= 1000
        return v
    if isinstance(value, str):
        try:
            return int(_dt.datetime.fromisoformat(value.replace(" ", "T")).timestamp() * 1000)
        except (ValueError, OverflowError):
            return None
    return None


def task_to_record(task: Mapping[str, Any]) -> Dict[str, Any]:
    """把 task JSON 转 Base record fields dict（按 schema 字段名）。"""
    summary = task.get("summary", "")
    parsed = parse_summary(summary)

    creator = task.get("creator") or {}
    creator_app_id = creator.get("id") if creator.get("type") == "app" else ""

    assignee_id = ""
    for member in (task.get("members") or []):
        if isinstance(member, dict) and member.get("role") == "assignee":
            assignee_id = member.get("id") or ""
            break

    fields: Dict[str, Any] = {
        "任务标题": summary,
        "状态": _STATUS_MAP.get(task.get("status"), "进行中"),
        "Agent": parsed["agent"] or "unknown",
        "task_guid": task.get("guid", ""),
        "host": parsed["host"],
        "creator_app_id": creator_app_id,
        "assignee": assignee_id,
        "task_url": task.get("url", ""),
        "last_synced": int(_dt.datetime.now().timestamp() * 1000),
    }
    created_at = _ts_to_iso_ms(task.get("created_at"))
    if created_at:
        fields["创建时间"] = created_at
    completed_at = _ts_to_iso_ms(task.get("completed_at"))
    if completed_at:
        fields["完成时间"] = completed_at
    return fields


# --- 状态 cursor ----------------------------------------------------------

def _indexer_state_dir() -> Path:
    root = os.getenv("FEISHU_HUB_HOME")
    base = Path(root) if root else Path.home() / ".roostery"
    d = base / "state" / "indexer"
    d.mkdir(parents=True, exist_ok=True)
    return d


def _cursor_path() -> Path:
    return _indexer_state_dir() / "cursor.json"


def load_cursor() -> int:
    """读 cursor.json 的 last_updated_at_us（缺则 0）。"""
    p = _cursor_path()
    if not p.exists():
        return 0
    try:
        d = json.loads(p.read_text(encoding="utf-8"))
        return int(d.get("last_updated_at_us") or 0)
    except (json.JSONDecodeError, ValueError):
        return 0


def save_cursor(updated_at_us: int) -> None:
    """原子写 cursor.json。"""
    p = _cursor_path()
    tmp = p.with_suffix(".tmp")
    tmp.write_text(
        json.dumps({"last_updated_at_us": int(updated_at_us)}, ensure_ascii=False),
        encoding="utf-8",
    )
    os.replace(tmp, p)


# --- 主流程 ---------------------------------------------------------------

@dataclass
class IndexerRunSummary:
    succeeded: int = 0
    failed: List[Dict[str, str]] = field(default_factory=list)
    skipped: int = 0
    started_at: str = ""
    finished_at: str = ""
    full: bool = False

    @property
    def total(self) -> int:
        return self.succeeded + len(self.failed) + self.skipped


def fetch_tasks_for_user(*, since_us: int = 0, limit_pages: int = 20) -> List[Dict[str, Any]]:
    """拉 user 视角所有相关 task（assignee + created + followed）。

    **不**加 ``--followed-by-me``——M3.B 默认用 ``--assignee`` 把 user 加进
    task.members（让 user 在飞书 "我的待办" inbox 可见），followed-by-me
    过滤反而**拉不到**。

    indexer 不依赖该 API 做增量游标（page_token 在 lark-cli 上语义不
    稳定）；增量在 client side 做（见 :func:`_filter_incremental`）。
    """
    argv = [
        "task", "+get-related-tasks",
        "--as", "user",
        "--include-complete",
        "--page-all",
        "--page-limit", str(limit_pages),
    ]
    try:
        resp = run_json(argv, timeout=60)
    except LarkCLIError:
        return []
    data = resp.get("data") if isinstance(resp, dict) else None
    items = (data or {}).get("items") or resp.get("items") if isinstance(resp, dict) else None
    return items or []


def _is_agent_task(task: Mapping[str, Any]) -> bool:
    """只刷 bot/agent 创建的 task；user 手动建的不进 Base（避免污染索引）。

    判据：``creator.type == "app"``。app 即飞书"应用"身份，对应 lark-cli
    某 profile 的 bot。
    """
    creator = task.get("creator") or {}
    return creator.get("type") == "app"


def _filter_incremental(tasks: List[Dict[str, Any]], since_us: int) -> List[Dict[str, Any]]:
    """client-side 增量过滤：只保留 updated_at > since_us（按 ms 比较，转 us）。"""
    if since_us <= 0:
        return tasks
    since_ms = since_us // 1000
    filtered: List[Dict[str, Any]] = []
    for t in tasks:
        upd = _ts_to_iso_ms(t.get("updated_at") or t.get("created_at"))
        if upd is None or upd > since_ms:
            filtered.append(t)
    return filtered


def upsert_record(*, base_token: str, table_id: str, fields: Mapping[str, Any]) -> str:
    """task_guid 主键 upsert。返回操作类型 ``"created"`` / ``"updated"`` / ``"skipped"``。

    实现细节（lark-cli 验证）：
    - 探测用 ``+record-search --json '{keyword, search_fields: ["task_guid"]}'``
      （keyword 模糊搜，但 task_guid 是 UUID 唯一，可视作精确）
    - 写用 ``+record-upsert --json '{field_map}'``，可选 ``--record-id`` 决定 update vs create
    - 不使用 ``+record-batch-*`` 系列：那些要求 ``{"fields": [...], "rows": [...]}`` 位置数组
      格式，跟字段顺序耦合（M3.A 已经因此删过一个 bitable_writer）。
    """
    guid = fields.get("task_guid")
    if not guid:
        return "skipped"

    # 1. 探测：keyword 搜 task_guid 字段
    record_id = None
    try:
        existing = run_json(
            [
                "base", "+record-search",
                "--base-token", base_token,
                "--table-id", table_id,
                "--as", "user",
                "--format", "json",
                "--json", json.dumps({
                    "keyword": str(guid),
                    "search_fields": ["task_guid"],
                    "limit": 1,
                }, ensure_ascii=False),
            ],
            timeout=20,
        )
        # search 返回结构可能是 {data:{records:[...]}}/{records:[...]}/{items:[...]}
        records = (
            (existing.get("data") or {}).get("records")
            or existing.get("records")
            or (existing.get("data") or {}).get("items")
            or existing.get("items")
            or []
        )
        if records:
            record_id = records[0].get("record_id") or records[0].get("id")
    except LarkCLIError as e:
        # task_guid 字段还没建时 search 会失败；让 caller 看到错（migrate-schema 应已建）
        if e.code in (1254013, 1254003):
            # field not found / table not found - 真的没 schema
            raise
        # 其他错误：可能就是查不到，按 missing 走

    # 2. 写：upsert（有 record_id → update，没有 → create）
    argv = [
        "base", "+record-upsert",
        "--base-token", base_token,
        "--table-id", table_id,
        "--as", "user",
        "--json", json.dumps(dict(fields), ensure_ascii=False),
    ]
    if record_id:
        argv += ["--record-id", record_id]
    run_json(argv, timeout=20)
    return "updated" if record_id else "created"


# --- schema migration --------------------------------------------------

_REQUIRED_FIELDS = [
    # M3.D 新增字段；类型严格遵守 lark-cli base field type names
    ("task_guid",      "text"),
    ("host",           "text"),
    ("creator_app_id", "text"),
    ("assignee",       "text"),
    ("task_url",       "url"),
    ("last_synced",    "datetime"),
]


def ensure_schema(*, base_token: str, table_id: str) -> Dict[str, str]:
    """幂等建 M3.D 需要的字段。返回 ``{field_name: action}``，action 是
    ``"created"`` / ``"exists"`` / ``"type_conflict"``（type conflict 时不强改，
    用户需自己决定）。
    """
    try:
        resp = run_json(
            [
                "base", "+field-list",
                "--as", "user",
                "--base-token", base_token,
                "--table-id", table_id,
            ],
            timeout=20,
        )
    except LarkCLIError:
        resp = {}
    fields = (resp.get("data") or {}).get("fields") or []
    have = {f.get("name"): f for f in fields}

    actions: Dict[str, str] = {}
    for name, want_type in _REQUIRED_FIELDS:
        if name in have:
            if have[name].get("type") != want_type:
                actions[name] = "type_conflict"
                continue
            actions[name] = "exists"
            continue
        # 缺，建（lark-cli: --json '{"name":..,"type":..}' 而非 --name/--type）
        try:
            run_json(
                [
                    "base", "+field-create",
                    "--as", "user",
                    "--base-token", base_token,
                    "--table-id", table_id,
                    "--json", json.dumps({"name": name, "type": want_type},
                                         ensure_ascii=False),
                ],
                timeout=20,
            )
            actions[name] = "created"
        except LarkCLIError as e:
            actions[name] = f"failed: {e.code} {e.msg[:80]}"
    return actions


def run_indexer(*, base_token: str, table_id: str, full: bool = False) -> IndexerRunSummary:
    """主入口。拉 task → 转 record → upsert → 推进 cursor。"""
    summary = IndexerRunSummary(
        full=full,
        started_at=_dt.datetime.now().astimezone().isoformat(timespec="seconds"),
    )

    since_us = 0 if full else load_cursor()
    tasks = fetch_tasks_for_user(since_us=since_us)
    # 只索引 agent 创建的 task；user 手动建的 task 不进 Base（避免污染索引层）
    tasks = [t for t in tasks if _is_agent_task(t)]
    tasks = _filter_incremental(tasks, since_us=since_us)

    max_updated_ms = since_us // 1000 if since_us else 0
    for t in tasks:
        try:
            fields = task_to_record(t)
            action = upsert_record(base_token=base_token, table_id=table_id, fields=fields)
            if action == "skipped":
                summary.skipped += 1
            else:
                summary.succeeded += 1
            upd_ms = _ts_to_iso_ms(t.get("updated_at") or t.get("created_at"))
            if upd_ms and upd_ms > max_updated_ms:
                max_updated_ms = upd_ms
        except LarkCLIError as e:
            summary.failed.append({"guid": t.get("guid", ""), "err": f"{e.code}: {e.msg[:200]}"})
        except Exception as e:    # noqa: BLE001
            summary.failed.append({"guid": t.get("guid", ""), "err": str(e)[:200]})

    if max_updated_ms:
        save_cursor(max_updated_ms * 1000)

    summary.finished_at = _dt.datetime.now().astimezone().isoformat(timespec="seconds")
    _save_last_run(summary)
    return summary


def _save_last_run(summary: IndexerRunSummary) -> None:
    """把最近一次跑结果写到 state/indexer/last_run.json 方便外部 inspect。"""
    p = _indexer_state_dir() / "last_run.json"
    payload = {
        "started_at": summary.started_at,
        "finished_at": summary.finished_at,
        "full": summary.full,
        "succeeded": summary.succeeded,
        "skipped": summary.skipped,
        "failed": summary.failed,
        "total": summary.total,
    }
    p.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")


# --- M4.C Phase 6: reconcile stale running --------------------------------

def reconcile_stale_running(
    *,
    configs: Optional[List[Any]] = None,
    registry: Optional[Any] = None,
) -> int:
    """扫所有 role Base：``运行状态==running`` 但本机无活 runner → 标 failed。

    兜底机制：runner 异常退出（OOM kill / 断电 / 进程崩溃后未走清理路径）会让
    Base 上的「运行状态」永远停在 ``running``。本 job 周期性扫描，把没人收尾
    的 row 标成 ``failed`` 并附 reconcile 备注，避免看板假阳性。

    返回修复的记录数；幂等可重复调用。
    """
    import logging
    from roostery import base_config, runner_registry
    from roostery.record_writer import STATE_FIELD, append_product, set_run_state

    log = logging.getLogger(__name__)
    cfgs = configs if configs is not None else base_config.load_all()
    reg = registry or runner_registry.RunnerRegistry()
    n_fixed = 0
    for cfg in cfgs:
        offset = 0
        while True:
            try:
                data = base_record_list(
                    base_token=cfg.base_token,
                    table_id=cfg.table_id,
                    limit=100,
                    offset=offset,
                )
            except LarkCLIError as e:
                log.warning("reconcile: record-list failed role=%s code=%s",
                            cfg.role, e.code)
                break
            items = data.get("items") or []
            for r in items:
                fields = r.get("fields") or {}
                state_val = fields.get(STATE_FIELD)
                if isinstance(state_val, list):
                    state = state_val[0] if state_val else ""
                else:
                    state = state_val or ""
                if state != "running":
                    continue
                rid = r.get("record_id") or ""
                if not rid:
                    continue
                if reg.lookup_by_record_id(rid):
                    continue
                # 本机没人收 → 兜底失败
                try:
                    set_run_state(record_id=rid, state="failed",
                                  base_token=cfg.base_token, table_id=cfg.table_id)
                    append_product(record_id=rid,
                                   text="--- reconcile：本机无 runner，兜底改 failed ---",
                                   base_token=cfg.base_token, table_id=cfg.table_id)
                    n_fixed += 1
                except Exception:  # noqa: BLE001
                    log.exception("reconcile fix failed: record_id=%s", rid)
            if not data.get("has_more"):
                break
            if not items:
                # defensive: empty page but has_more=true → 避免死循环
                break
            offset += len(items)
    return n_fixed


def load_last_run() -> Optional[Dict[str, Any]]:
    p = _indexer_state_dir() / "last_run.json"
    if not p.exists():
        return None
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return None
