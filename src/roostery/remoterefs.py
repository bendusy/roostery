"""从 lark-cli stdout 中提取远端对象 token（best-effort）。

写入 journal envelope 的 ``remote_refs`` 子对象，供 S6/S7/S9 复用。
任何解析失败都返回全 None，**不抛异常**。
"""
from __future__ import annotations

import json
from typing import Any, Dict, List, Optional

FIELDS = ("message_id", "doc_token", "folder_token", "record_id")


def _empty() -> Dict[str, Optional[str]]:
    return {k: None for k in FIELDS}


def _walk(obj: Any, path: str = "") -> List[tuple]:
    """递归 yield (path, key, value) 三元组，便于按 key 名搜索。"""
    out: List[tuple] = []
    if isinstance(obj, dict):
        for k, v in obj.items():
            p = f"{path}.{k}" if path else k
            out.append((p, k, v))
            out.extend(_walk(v, p))
    elif isinstance(obj, list):
        for i, item in enumerate(obj):
            p = f"{path}[{i}]"
            out.extend(_walk(item, p))
    return out


def _coerce_str(v: Any) -> Optional[str]:
    if isinstance(v, str) and v:
        return v
    return None


def extract(argv: List[str], stdout) -> Dict[str, Optional[str]]:
    """根据 argv 和 stdout 抽取已知字段。

    argv 用于推断"在跑什么子命令"，stdout 用于真正抓 token。
    """
    refs = _empty()
    if not stdout:
        return refs
    if isinstance(stdout, (bytes, bytearray)):
        try:
            text = bytes(stdout).decode("utf-8", errors="replace")
        except Exception:
            return refs
    else:
        text = str(stdout)
    text = text.strip()
    if not text or text[0] not in "{[":
        return refs
    try:
        body = json.loads(text)
    except (json.JSONDecodeError, ValueError):
        return refs

    walked = _walk(body)
    by_key: Dict[str, List[Any]] = {}
    for _path, key, value in walked:
        by_key.setdefault(key, []).append(value)

    # message_id：im +messages-send / +messages-reply
    for k in ("message_id",):
        if refs["message_id"] is None and by_key.get(k):
            refs["message_id"] = _coerce_str(by_key[k][0])

    # doc_token：docs +create / +update v2 的 document_id；老接口 doc_token
    for k in ("document_id", "doc_token", "obj_token"):
        if refs["doc_token"] is None and by_key.get(k):
            refs["doc_token"] = _coerce_str(by_key[k][0])

    # folder_token：drive +create-folder 返回 token；从 argv 判断
    if _argv_has(argv, "create-folder") or _argv_has(argv, "+create-folder"):
        for k in ("token", "folder_token"):
            if refs["folder_token"] is None and by_key.get(k):
                refs["folder_token"] = _coerce_str(by_key[k][0])

    # record_id：base record 类
    for k in ("record_id",):
        if refs["record_id"] is None and by_key.get(k):
            refs["record_id"] = _coerce_str(by_key[k][0])

    return refs


def _argv_has(argv: List[str], needle: str) -> bool:
    return any(needle in a for a in argv or [])
