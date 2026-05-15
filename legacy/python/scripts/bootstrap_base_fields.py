#!/usr/bin/env python3
"""一键给 role Base 加 M4.A 强制字段：运行状态 / 产物 / _last_writer_marker + 运行中视图。

幂等：已存在的字段/视图跳过；只新建缺失的。

用法：
  python scripts/bootstrap_base_fields.py --config config/bases/gongzhonghao_2026.yaml

退出码：0 成功；非 0 失败。
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any, Dict, List, Optional

import yaml


# 飞书 Base API 字段类型用字符串名（不是整型码），由 lark-cli 透传
# - text: 多行文本（写入时支持 \n）
# - select: 单/多选；multiple:false 表示单选
REQUIRED_FIELDS: List[Dict[str, Any]] = [
    {
        "name": "运行状态",
        "type": "select",
        "multiple": False,
        "options": [
            {"name": "idle"},
            {"name": "running"},
            {"name": "done"},
            {"name": "aborted"},
            {"name": "failed"},
        ],
    },
    {"name": "产物", "type": "text"},
    {"name": "_last_writer_marker", "type": "text"},
]

RUNNING_VIEW_NAME = "运行中"


def _lark(*args: str, timeout: int = 30) -> Dict[str, Any]:
    """调用 lark-cli 并解析 JSON 输出。失败直接 raise。"""
    proc = subprocess.run(
        ["lark-cli", *args],
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"lark-cli failed: argv={args!r}\nstderr={proc.stderr}"
        )
    out = proc.stdout.strip()
    if not out:
        return {}
    try:
        return json.loads(out)
    except json.JSONDecodeError as e:
        raise RuntimeError(
            f"lark-cli non-JSON output: {proc.stdout[:500]}"
        ) from e


def list_fields(base_token: str, table_id: str) -> List[Dict[str, Any]]:
    data = _lark(
        "base", "+field-list",
        "--as", "user",
        "--base-token", base_token,
        "--table-id", table_id,
    )
    return (data.get("data") or {}).get("fields") or []


def create_field(base_token: str, table_id: str, spec: Dict[str, Any]) -> None:
    _lark(
        "base", "+field-create",
        "--as", "user",
        "--base-token", base_token,
        "--table-id", table_id,
        "--json", json.dumps(spec, ensure_ascii=False),
    )


def find_view_id(base_token: str, table_id: str, name: str) -> Optional[str]:
    data = _lark(
        "base", "+view-list",
        "--as", "user",
        "--base-token", base_token,
        "--table-id", table_id,
    )
    items = (data.get("data") or {}).get("views") or []
    for v in items:
        if v.get("name") == name or v.get("view_name") == name:
            return v.get("view_id") or v.get("id")
    return None


def create_view(base_token: str, table_id: str, name: str) -> str:
    data = _lark(
        "base", "+view-create",
        "--as", "user",
        "--base-token", base_token,
        "--table-id", table_id,
        "--json", json.dumps({"name": name, "type": "grid"}, ensure_ascii=False),
    )
    body = data.get("data") or {}
    # 防御性提取 view_id：可能 .view.view_id / .views[0].view_id / .view_id
    if isinstance(body.get("view"), dict):
        vid = body["view"].get("view_id") or body["view"].get("id")
        if vid:
            return vid
    for key in ("views", "items"):
        arr = body.get(key)
        if isinstance(arr, list) and arr:
            first = arr[0]
            vid = first.get("view_id") or first.get("id")
            if vid:
                return vid
    vid = body.get("view_id")
    if vid:
        return vid
    raise RuntimeError(f"create_view: cannot find view_id in response: {body!r}")


def set_running_filter(base_token: str, table_id: str, view_id: str) -> None:
    payload = {
        "logic": "and",
        "conditions": [
            ["运行状态", "intersects", ["running"]],
        ],
    }
    _lark(
        "base", "+view-set-filter",
        "--as", "user",
        "--base-token", base_token,
        "--table-id", table_id,
        "--view-id", view_id,
        "--json", json.dumps(payload, ensure_ascii=False),
    )


def bootstrap(config_path: Path) -> int:
    cfg = yaml.safe_load(config_path.read_text(encoding="utf-8")) or {}
    bt = cfg.get("base_token")
    tid = cfg.get("table_id")
    role = cfg.get("role", "<unknown>")
    if not (bt and tid):
        print(
            f"error: {config_path} missing base_token / table_id",
            file=sys.stderr,
        )
        return 1

    print(f"target: {role} ({bt}/{tid})")

    existing = {f.get("name") for f in list_fields(bt, tid)}
    for spec in REQUIRED_FIELDS:
        if spec["name"] in existing:
            print(f"  - 字段「{spec['name']}」已存在，跳过")
        else:
            print(f"  + 添加字段「{spec['name']}」")
            create_field(bt, tid, spec)

    if find_view_id(bt, tid, RUNNING_VIEW_NAME):
        print(f"  - 视图「{RUNNING_VIEW_NAME}」已存在，跳过")
    else:
        print(f"  + 创建视图「{RUNNING_VIEW_NAME}」并设置 filter")
        vid = create_view(bt, tid, RUNNING_VIEW_NAME)
        set_running_filter(bt, tid, vid)

    print("done.")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--config", required=True,
        help="path to a config/bases/*.yaml file",
    )
    args = ap.parse_args()
    return bootstrap(Path(args.config))


if __name__ == "__main__":
    sys.exit(main())
