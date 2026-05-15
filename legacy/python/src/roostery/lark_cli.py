"""lark-cli 业务包裹：仅暴露已验证子命令，统一 JSON 解析与异常类型。

设计：``docs/FEISHU_OFFICE_HUB_DESIGN_V2.md`` §9.3。所有命令名与参数都在本机
``lark-cli --dry-run`` 验证过。

调用方式：始终通过 PATH 上的 ``lark-cli``（即部署后的 shim），让 journal 自动落盘；
单测可通过 ``binary=`` 参数或 ``FEISHU_HUB_LARK_CLI_BIN`` env 覆盖。
"""
from __future__ import annotations

import json
import os
import subprocess
import time
from dataclasses import dataclass
from typing import Any, Dict, Iterable, List, Mapping, Optional, Sequence

DEFAULT_TIMEOUT = 30
DEFAULT_BIN = "lark-cli"
ENV_BIN = "FEISHU_HUB_LARK_CLI_BIN"
TRANSIENT_CODES = {99991663, 99991664}  # token 过期类，触发一次重试


class LarkCLIError(RuntimeError):
    """lark-cli 执行失败。``code`` 是业务码（int）或 -1（无法解析）。"""

    def __init__(self, code: int, msg: str, argv: Sequence[str],
                 stdout: str = "", stderr: str = "", retriable: bool = False):
        super().__init__(f"lark-cli failed: code={code} msg={msg}")
        self.code = code
        self.msg = msg
        self.argv = list(argv)
        self.stdout = stdout
        self.stderr = stderr
        self.retriable = retriable


@dataclass(frozen=True)
class DocInfo:
    doc_token: str
    url: Optional[str] = None


@dataclass(frozen=True)
class FolderEntry:
    name: str
    token: str
    type: str  # "folder" / "docx" / ...


def _binary() -> str:
    return os.getenv(ENV_BIN) or DEFAULT_BIN


def run_json(
    argv: Sequence[str],
    *,
    stdin: Optional[str] = None,
    timeout: int = DEFAULT_TIMEOUT,
    jq: Optional[str] = None,
    binary: Optional[str] = None,
    retries: int = 1,
    profile: Optional[str] = None,
) -> Any:
    """运行 lark-cli，强制 ``--format json``，解析 stdout。

    ``profile`` 非空时插入 ``--profile X`` global flag（必须在子命令前）；
    多 bot 协作场景下用它指定调用哪个 lark-cli profile。

    Returns
    -------
    解析后的 JSON 对象（dict / list / ...）。``--jq`` 提供时直接返回字符串去除两端
    空白后的结果（飞书侧 ``--jq`` 输出未必是 JSON）。
    """
    bin_ = binary or _binary()
    # lark-cli 多数子命令默认就输出 JSON 且不接受 --format flag（如 im +messages-send /
    # docs +create）；仅当调用方显式在 argv 里给了 --format 才透传。
    full: List[str] = [bin_]
    if profile:
        full += ["--profile", profile]
    full += list(argv)
    if jq:
        full += ["--jq", jq]

    last_err: Optional[LarkCLIError] = None
    for attempt in range(retries + 1):
        try:
            proc = subprocess.run(
                full,
                input=stdin,
                capture_output=True,
                text=True,
                timeout=timeout,
            )
        except subprocess.TimeoutExpired as e:
            raise LarkCLIError(-1, f"timeout after {timeout}s", full,
                               retriable=True) from e
        except FileNotFoundError as e:
            raise LarkCLIError(-1, f"binary not found: {bin_}", full) from e

        if proc.returncode == 0:
            return _parse_output(proc.stdout, jq=jq, argv=full,
                                 stderr=proc.stderr)

        err = _parse_error(proc.stdout, proc.stderr, proc.returncode, full)
        if err.retriable and attempt < retries:
            last_err = err
            time.sleep(0.2 * (attempt + 1))
            continue
        raise err
    # 不会走到这里，但类型完整
    assert last_err is not None  # pragma: no cover
    raise last_err


def _parse_output(stdout: str, *, jq: Optional[str], argv: Sequence[str],
                  stderr: str) -> Any:
    if jq:
        return stdout.strip()
    text = stdout.strip()
    if not text:
        return None
    try:
        return json.loads(text)
    except json.JSONDecodeError as e:
        raise LarkCLIError(-1, f"non-JSON stdout: {e}", argv,
                           stdout=stdout, stderr=stderr) from e


def _parse_error(stdout: str, stderr: str, returncode: int,
                 argv: Sequence[str]) -> LarkCLIError:
    code: int = returncode
    msg = (stderr or stdout or "").strip()[:500]
    text = stdout.strip()
    if text.startswith("{"):
        try:
            body = json.loads(text)
            if isinstance(body, dict):
                if "code" in body and isinstance(body["code"], int):
                    code = body["code"]
                if "msg" in body and isinstance(body["msg"], str):
                    msg = body["msg"]
        except json.JSONDecodeError:
            pass
    retriable = code in TRANSIENT_CODES or returncode in (124,)  # 124: timeout
    return LarkCLIError(code, msg, argv, stdout=stdout, stderr=stderr,
                        retriable=retriable)


# ---- 已验证子命令 ---------------------------------------------------------

def im_send_text(
    *,
    user_id: str,
    text: str,
    idempotency_key: Optional[str] = None,
    timeout: int = DEFAULT_TIMEOUT,
    binary: Optional[str] = None,
) -> Optional[str]:
    """``im +messages-send``，根据 ``user_id`` 前缀自动选 ``--user-id`` 或 ``--chat-id``。

    - ``ou_*`` / ``on_*`` / ``ol_*``：open_id 类，走 ``--user-id``
    - ``oc_*``：chat_id（群/单聊），走 ``--chat-id``
    """
    target = user_id.strip()
    if target.startswith("oc_"):
        flag = "--chat-id"
    else:
        flag = "--user-id"
    argv: List[str] = ["im", "+messages-send", flag, target, "--text", text]
    if idempotency_key:
        argv += ["--idempotency-key", idempotency_key]
    body = run_json(argv, timeout=timeout, binary=binary)
    return _pluck(body, ("data", "message_id")) or _pluck(body, ("message_id",))


def im_messages_reply(
    *,
    message_id: str,
    text: str,
    reply_in_thread: bool = True,
    profile: Optional[str] = None,
    idempotency_key: Optional[str] = None,
    timeout: int = DEFAULT_TIMEOUT,
    binary: Optional[str] = None,
) -> Optional[str]:
    """``[--profile X] im +messages-reply --message-id <om_xxx> --text <...> [--reply-in-thread]``。

    ``--profile`` 是 lark-cli **global** flag，必须出现在子命令 ``im`` 之前；
    多 bot 场景下用它指定回复用哪个 lark-cli profile（= 哪个 bot）。
    """
    argv: List[str] = []
    if profile:
        argv += ["--profile", profile]
    argv += ["im", "+messages-reply",
             "--message-id", message_id,
             "--text", text,
             "--as", "bot"]
    if reply_in_thread:
        argv.append("--reply-in-thread")
    if idempotency_key:
        argv += ["--idempotency-key", idempotency_key]
    body = run_json(argv, timeout=timeout, binary=binary)
    return _pluck(body, ("data", "message_id")) or _pluck(body, ("message_id",))


def docs_create_v2(
    *,
    parent_token: str,
    markdown: str,
    title: Optional[str] = None,
    timeout: int = DEFAULT_TIMEOUT,
    binary: Optional[str] = None,
) -> DocInfo:
    """创建 docx 并移动到目标文件夹（如果 ``parent_token`` 给了）。

    lark-cli 实测：``docs +create --folder-token`` 仅是 hint，**不会**真的把
    文档放进目标文件夹（请求 body 不含 folder 字段，doc 默认落用户根目录）。
    解决：create 之后再调 ``drive +move`` 把 docx 移过去；title 用
    ``docs +update --new-title`` 设置。
    """
    # 1) 创建（lark-cli 的 --title / --folder-token 被静默忽略，所以这里
    #    只用 content 创建，下面再 move + rename）
    argv = [
        "docs", "+create",
        "--api-version", "v2",
        "--content", "-",
        "--doc-format", "markdown",
    ]
    body = run_json(argv, stdin=markdown, timeout=timeout, binary=binary)
    token = (
        _pluck(body, ("data", "document", "document_id"))
        or _pluck(body, ("data", "document_id"))
        or _pluck(body, ("document_id",))
    )
    if not token:
        raise LarkCLIError(-1, "docs +create response missing document_id",
                           argv, stdout=json.dumps(body)[:500])
    url = (
        _pluck(body, ("data", "document", "url"))
        or _pluck(body, ("data", "url"))
        or _pluck(body, ("url",))
    )

    # 2) 落进目标文件夹（drive +move 是写真实生效的 API）
    if parent_token:
        try:
            drive_move(file_token=token, target_folder_token=parent_token,
                       type_="docx", binary=binary, timeout=timeout)
        except LarkCLIError:
            pass  # 文件夹移动失败不致命，doc 已建好
    # 3) 设置标题（用 update overwrite + --new-title；标题写不上不致命）
    if title:
        try:
            docs_update_overwrite(doc_token=token, markdown=markdown,
                                  title=title, binary=binary, timeout=timeout)
        except LarkCLIError:
            pass
    return DocInfo(doc_token=token, url=url)


def drive_move(
    *,
    file_token: str,
    target_folder_token: str,
    type_: str,
    as_user: bool = True,
    timeout: int = DEFAULT_TIMEOUT,
    binary: Optional[str] = None,
) -> None:
    """``drive +move --file-token --folder-token --type``。

    ``type_`` 取值见 lark-cli ``+move --help``：file / docx / bitable / sheet / folder ...
    """
    argv = ["drive", "+move",
            "--file-token", file_token,
            "--folder-token", target_folder_token,
            "--type", type_]
    if as_user:
        argv += ["--as", "user"]
    run_json(argv, timeout=timeout, binary=binary)


def docs_update_overwrite(
    *,
    doc_token: str,
    markdown: str,
    title: Optional[str] = None,
    timeout: int = DEFAULT_TIMEOUT,
    binary: Optional[str] = None,
) -> None:
    """``docs +update --mode overwrite --markdown - [--new-title]``（v1 形式）。

    lark-cli 实测：v1 走 MCP update-doc，支持 ``--mode`` + ``--new-title``；
    v2 要 ``--command``，参数面有出入。这里固定 v1，更稳。
    """
    argv = [
        "docs", "+update",
        "--doc", doc_token,
        "--mode", "overwrite",
        "--markdown", "-",
    ]
    if title:
        argv += ["--new-title", title]
    run_json(argv, stdin=markdown, timeout=timeout, binary=binary)


def drive_list_folder(
    *,
    folder_token: str,
    page_size: int = 200,
    as_user: bool = True,
    timeout: int = DEFAULT_TIMEOUT,
    binary: Optional[str] = None,
) -> List[FolderEntry]:
    """``drive files list --as user --params '{folder_token,page_size}'``。"""
    params = json.dumps({"folder_token": folder_token, "page_size": page_size})
    argv = ["drive", "files", "list", "--params", params]
    if as_user:
        argv += ["--as", "user"]
    body = run_json(argv, timeout=timeout, binary=binary)
    files = _pluck(body, ("data", "files")) or _pluck(body, ("files",)) or []
    out: List[FolderEntry] = []
    if isinstance(files, list):
        for f in files:
            if not isinstance(f, dict):
                continue
            token = f.get("token") or f.get("file_token")
            name = f.get("name")
            ftype = f.get("type") or ""
            if token and name is not None:
                out.append(FolderEntry(name=name, token=token, type=ftype))
    return out


def drive_create_folder(
    *,
    parent_token: str,
    name: str,
    as_user: bool = True,
    timeout: int = DEFAULT_TIMEOUT,
    binary: Optional[str] = None,
) -> str:
    """``drive +create-folder --parent-token --name``，返回新文件夹 token。"""
    argv = ["drive", "+create-folder", "--folder-token", parent_token, "--name", name]
    if as_user:
        argv += ["--as", "user"]
    body = run_json(argv, timeout=timeout, binary=binary)
    token = (
        _pluck(body, ("data", "token"))
        or _pluck(body, ("data", "folder_token"))
        or _pluck(body, ("token",))
    )
    if not token:
        raise LarkCLIError(-1, "create-folder response missing token", argv,
                           stdout=json.dumps(body)[:500])
    return token


# ---- base 记录读写 -------------------------------------------------------

def base_record_get(
    *,
    base_token: str,
    table_id: str,
    record_id: str,
    timeout: int = DEFAULT_TIMEOUT,
    binary: Optional[str] = None,
) -> Dict[str, Any]:
    """``base +record-get --format json``，返回 ``{field_name: value}`` 字段映射。

    实测响应（lark-cli 1.0.29 + 公众号-2026 base, 2026-05-14）是**列式**结构：
    ``data.data[0]`` = 一行的值数组；``data.fields`` = 平行的列名数组；
    ``data.record_id_list[0]`` = 行的 record_id。select 字段返回 ``["opt"]``
    数组形态（与 M4.C 下游 ``rec[name][0]`` 访问一致），text 返回字符串，
    未填值返回 ``null``。这里 zip 列名 + 行值给上层一个 friendly dict。
    """
    argv = [
        "base", "+record-get",
        "--base-token", base_token,
        "--table-id", table_id,
        "--record-id", record_id,
        "--format", "json",
    ]
    body = run_json(argv, timeout=timeout, binary=binary)
    if isinstance(body, dict):
        biz_code = body.get("code", 0)
        if isinstance(biz_code, int) and biz_code != 0:
            biz_msg = body.get("msg") if isinstance(body.get("msg"), str) else ""
            raise LarkCLIError(biz_code, biz_msg or "record-get business error",
                               argv, stdout=json.dumps(body, ensure_ascii=False)[:500])
    data = (body or {}).get("data") if isinstance(body, dict) else None
    if not isinstance(data, dict):
        return {}
    rows = data.get("data") or []
    fields = data.get("fields") or []
    if not rows or not fields:
        return {}
    return dict(zip(fields, rows[0]))


def base_record_upsert(
    *,
    base_token: str,
    table_id: str,
    fields: Mapping[str, Any],
    record_id: Optional[str] = None,
    timeout: int = DEFAULT_TIMEOUT,
    binary: Optional[str] = None,
) -> str:
    """``base +record-upsert``：``record_id`` 空 → 创建；否则 → 更新。

    返回 record_id：create 路径从 ``data.record.record_id_list[0]`` 抽；
    update 路径直接 passthrough 入参。
    """
    argv: List[str] = [
        "base", "+record-upsert",
        "--base-token", base_token,
        "--table-id", table_id,
        "--json", json.dumps(dict(fields), ensure_ascii=False),
    ]
    if record_id:
        argv += ["--record-id", record_id]
    body = run_json(argv, timeout=timeout, binary=binary)
    # run_json only checks subprocess exit code; lark-cli can exit 0 with
    # ``{"code": <非零业务码>, "msg": "..."}``。这里显式核对业务码，避免静默失败。
    if isinstance(body, dict):
        biz_code = body.get("code", 0)
        if isinstance(biz_code, int) and biz_code != 0:
            biz_msg = body.get("msg") if isinstance(body.get("msg"), str) else ""
            raise LarkCLIError(biz_code, biz_msg or "record-upsert business error",
                               argv, stdout=json.dumps(body, ensure_ascii=False)[:500])
    if record_id:
        return record_id
    rid_list = _pluck(body, ("data", "record", "record_id_list"))
    if isinstance(rid_list, list) and rid_list:
        return str(rid_list[0])
    # 兜底：部分形态 data.record.record_id
    fallback = _pluck(body, ("data", "record", "record_id"))
    if isinstance(fallback, str) and fallback:
        return fallback
    raise LarkCLIError(-1, "record-upsert response missing record_id", argv,
                       stdout=json.dumps(body)[:500] if body else "")


# WARNING: base_record_search 走的是 lark-cli ``+record-search``，本质是
# **全文 keyword 检索**，不是结构化字段过滤。如果你想按字段值（如
# ``record_id == X`` / ``阶段 == "📋 选题"``）精确筛选记录，必须改用
# ``base +record-list`` 配合**预先在 Base 里建好的过滤视图**（``view_id``），
# 把过滤条件落在视图侧。本 helper 不支持结构化 filter，Phase 6 reconcile 这
# 类语义请勿误用。
def base_record_search(
    *,
    base_token: str,
    table_id: str,
    keyword: str,
    page_size: int = 100,
    search_fields: Optional[Sequence[str]] = None,
    timeout: int = DEFAULT_TIMEOUT,
    binary: Optional[str] = None,
) -> List[Dict[str, Any]]:
    """**Keyword (full-text) search, not structured filter.** lark-cli 1.0.29 限制。

    ``base +record-search`` 仅接受 ``--json`` 一种参数形式（没有 ``--filter`` /
    ``--query``），里头 schema 是
    ``{"keyword":"<text>","search_fields":[...],"limit":<int>}``。
    入参 ``keyword`` 直接透传给 lark-cli 的 ``keyword`` 字段，``page_size`` →
    ``limit``。返回 ``data.items`` 列表。

    若要按字段做精确/范围过滤，请改用 ``base +record-list`` + 预过滤视图。
    """
    payload: Dict[str, Any] = {"keyword": keyword, "limit": page_size}
    if search_fields:
        payload["search_fields"] = list(search_fields)
    argv = [
        "base", "+record-search",
        "--base-token", base_token,
        "--table-id", table_id,
        "--json", json.dumps(payload, ensure_ascii=False),
        "--format", "json",
    ]
    body = run_json(argv, timeout=timeout, binary=binary)
    items = _pluck(body, ("data", "items"))
    return list(items) if isinstance(items, list) else []


def base_record_list(
    *,
    base_token: str,
    table_id: str,
    view_id: Optional[str] = None,
    limit: int = 100,
    offset: int = 0,
    fields: Optional[Sequence[str]] = None,
    timeout: int = DEFAULT_TIMEOUT,
    binary: Optional[str] = None,
) -> Dict[str, Any]:
    """``base +record-list``：分页列出记录。

    Use this (not ``+record-search``) for structured filtering — pass ``view_id``
    of a pre-filtered view, OR list all + filter in Python.

    实测响应（lark-cli 1.0.29 + 公众号-2026 base, 2026-05-14）是**列式**结构：
    ``data.data`` = 行值二维数组；``data.fields`` = 平行列名；
    ``data.record_id_list`` = 平行 record_id。这里 zip 成上层期望的形态：
    ``{"items": [{"record_id": ..., "fields": {name: val}}, ...], "has_more": bool}``。
    """
    argv: List[str] = [
        "base", "+record-list",
        "--base-token", base_token,
        "--table-id", table_id,
        "--format", "json",
        "--limit", str(limit),
        "--offset", str(offset),
    ]
    if view_id:
        argv += ["--view-id", view_id]
    if fields:
        for f in fields:
            argv += ["--field-id", f]
    body = run_json(argv, timeout=timeout, binary=binary)
    if isinstance(body, dict):
        biz_code = body.get("code", 0)
        if isinstance(biz_code, int) and biz_code != 0:
            biz_msg = body.get("msg") if isinstance(body.get("msg"), str) else ""
            raise LarkCLIError(biz_code, biz_msg or "record-list business error",
                               argv, stdout=json.dumps(body, ensure_ascii=False)[:500])
    data = body.get("data", {}) if isinstance(body, dict) else {}
    if not isinstance(data, dict):
        return {"items": [], "has_more": False}
    rows = data.get("data") or []
    field_names = data.get("fields") or []
    rids = data.get("record_id_list") or []
    items: List[Dict[str, Any]] = []
    for i, row in enumerate(rows):
        rec = dict(zip(field_names, row)) if field_names else {}
        rid = rids[i] if i < len(rids) else ""
        items.append({"record_id": rid, "fields": rec})
    return {"items": items, "has_more": bool(data.get("has_more"))}


def base_record_delete(
    *,
    base_token: str,
    table_id: str,
    record_id: str,
    timeout: int = DEFAULT_TIMEOUT,
    binary: Optional[str] = None,
) -> None:
    """``base +record-delete --yes``（high-risk-write，必须带 ``--yes`` 才执行）。"""
    argv = [
        "base", "+record-delete",
        "--base-token", base_token,
        "--table-id", table_id,
        "--record-id", record_id,
        "--yes",
    ]
    run_json(argv, timeout=timeout, binary=binary)


# ---- 组合便利函数 ---------------------------------------------------------

def find_or_create_folder(
    *,
    parent_token: str,
    name: str,
    binary: Optional[str] = None,
) -> str:
    """先 list 找同名 folder；找到则复用，否则创建。"""
    for entry in drive_list_folder(folder_token=parent_token, binary=binary):
        if entry.name == name and entry.type == "folder":
            return entry.token
    return drive_create_folder(parent_token=parent_token, name=name, binary=binary)


def find_doc_in_folder(
    *,
    folder_token: str,
    title: str,
    binary: Optional[str] = None,
) -> Optional[str]:
    """精确匹配 ``name == title AND type == "docx"``，命中返回 doc_token。"""
    for entry in drive_list_folder(folder_token=folder_token, binary=binary):
        if entry.name == title and entry.type == "docx":
            return entry.token
    return None


# ---- helpers ---------------------------------------------------------------

def _pluck(obj: Any, path: Sequence[str]) -> Any:
    cur = obj
    for key in path:
        if isinstance(cur, dict) and key in cur:
            cur = cur[key]
        else:
            return None
    return cur
