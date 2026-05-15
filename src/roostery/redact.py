"""Token / secret 脱敏。

策略：
- argv：识别 ``--app-secret`` / ``--access-token`` 等 flag，下一项替换为 ``***``；
  也识别 ``--header "Authorization: Bearer xxx"`` 形式的 header 值。
- 文本（stdout/stderr 的 head 字节流）：用正则替换 ``"key":"value"`` 与
  ``"key": "value"`` 形式中的敏感字段。

所有函数返回新对象，**不修改入参**。返回值第二项是被脱敏的字段路径列表，
直接写入 journal 的 ``privacy.redacted_fields``。
"""
from __future__ import annotations

import re
from typing import Iterable, List, Tuple

MASK = "***"

# 敏感 key（不区分大小写、连字符/下划线等价）
_SENSITIVE_KEYS = (
    "app_secret",
    "access_token",
    "refresh_token",
    "user_access_token",
    "tenant_access_token",
    "authorization",
    "api_key",
)


def _norm(key: str) -> str:
    return key.lower().replace("-", "_").lstrip("_")


def _is_sensitive_flag(flag: str) -> bool:
    if not flag.startswith("--"):
        return False
    return _norm(flag[2:]) in _SENSITIVE_KEYS


# "Authorization: Bearer xxx" / "X-Token: abc"
_HEADER_RE = re.compile(
    r"^\s*([A-Za-z][A-Za-z0-9_-]*)\s*:\s*(.+)$",
)


def _scrub_header_value(value: str) -> Tuple[str, bool]:
    m = _HEADER_RE.match(value)
    if not m:
        return value, False
    name, _ = m.group(1), m.group(2)
    if _norm(name) in _SENSITIVE_KEYS:
        return f"{name}: {MASK}", True
    return value, False


def scrub_argv(argv: Iterable[str]) -> Tuple[List[str], List[str]]:
    """脱敏 argv。

    Returns
    -------
    (new_argv, redacted_paths)
        ``redacted_paths`` 类似 ``["argv[3]"]``，便于审计。
    """
    out: List[str] = list(argv)
    redacted: List[str] = []
    i = 0
    while i < len(out):
        token = out[i]
        # --flag value
        if _is_sensitive_flag(token) and i + 1 < len(out):
            out[i + 1] = MASK
            redacted.append(f"argv[{i + 1}]")
            i += 2
            continue
        # --flag=value
        if token.startswith("--") and "=" in token:
            flag, _, _value = token.partition("=")
            if _is_sensitive_flag(flag):
                out[i] = f"{flag}={MASK}"
                redacted.append(f"argv[{i}]")
                i += 1
                continue
        # --header "Authorization: ..." 的 value
        if token in ("--header", "-H") and i + 1 < len(out):
            new_val, changed = _scrub_header_value(out[i + 1])
            if changed:
                out[i + 1] = new_val
                redacted.append(f"argv[{i + 1}]")
            i += 2
            continue
        i += 1
    return out, redacted


# 文本中匹配 "key":"value" / "key": "value" / "key":  value
_TEXT_PATTERNS = [
    re.compile(
        rf'("{key}"\s*:\s*")[^"]*(")',
        re.IGNORECASE,
    )
    for key in _SENSITIVE_KEYS
] + [
    # YAML 风格：key: value
    re.compile(
        rf"(^|\n)([ \t]*{key}[ \t]*:[ \t]*)\S+",
        re.IGNORECASE,
    )
    for key in _SENSITIVE_KEYS
]


def scrub_text(buf) -> str:
    """脱敏文本。

    入参可以是 ``bytes`` / ``bytearray`` / ``str``；统一返回 ``str``
    （二进制按 utf-8 容错解码）。
    """
    if isinstance(buf, (bytes, bytearray)):
        text = bytes(buf).decode("utf-8", errors="replace")
    else:
        text = str(buf)
    for pat in _TEXT_PATTERNS:
        # 不同正则的捕获组数不同：JSON 风格 2 个，YAML 风格 2 个
        if pat.groups == 2 and pat.pattern.startswith('('):
            text = pat.sub(lambda m: f'{m.group(1)}{MASK}{m.group(2)}', text)
        else:
            text = pat.sub(lambda m: f"{m.group(1)}{m.group(2)}{MASK}", text)
    return text
