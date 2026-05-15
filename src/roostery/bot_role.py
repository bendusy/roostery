"""bot_role — 多 bot 协作的角色配置层（M3.C）。

从 ``~/.feishu_hub/bots.yaml`` 顶层 ``bots:`` 数组加载 :class:`BotRole`，并提供
:func:`event_matches_bot` 判定某条 IM 事件是否应被某个 bot 处理。

设计原则：

- BotRole 描述"这台机器上某个 lark-cli profile + 角色"，**不缓存** bot 的飞书显示名
  （随时可改，靠 ``mention_alias`` 作为匹配键）
- 多 bot 匹配同一事件是合法的；本模块只做 per-bot 判定，编排留给调用方
"""
from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Tuple


class BotRoleConfigError(ValueError):
    """bots.yaml 字段缺失或类型错误。"""


_REQUIRED_FIELDS = (
    "app_id",
    "role",
    "mention_alias",
    "runner",
    "default_cwd",
    "prompt_template",
)


@dataclass(frozen=True)
class BotRole:
    app_id: str
    role: str
    mention_alias: str
    runner: str
    default_cwd: str
    prompt_template: str
    reply_template: str = ""
    chat_whitelist: Tuple[str, ...] = ()
    next_bot_mention: str = ""
    # M3.C：所有 bot 把 relay_task 都用此 app_id 调 lark-cli，跨机收敛到同一 task。
    # 空 = 用 bot 自己身份（per-bot 各建一个 task；向后兼容默认）。
    # 要 work 还需要本机有该 app_id 的 lark-cli profile 且 token 有效。
    relay_writer_app_id: str = ""


def _yaml():
    try:
        import yaml  # type: ignore[import-not-found]
    except ImportError as e:  # pragma: no cover
        raise RuntimeError(
            "roostery.bot_role requires PyYAML. Install with: pip install pyyaml"
        ) from e
    return yaml


def load_bots(path: Path) -> List[BotRole]:
    """加载 bots.yaml；文件缺失或无 ``bots`` key 时返回空列表。

    缺必填字段抛 :class:`BotRoleConfigError`，由调用方决定是否致命。
    """
    if not path.exists():
        return []
    yaml = _yaml()
    with path.open("r", encoding="utf-8") as f:
        raw = yaml.safe_load(f) or {}
    items = raw.get("bots") or []
    if not isinstance(items, list):
        return []
    out: List[BotRole] = []
    for i, item in enumerate(items):
        if not isinstance(item, dict):
            raise BotRoleConfigError(f"bots[{i}] must be a mapping, got {type(item).__name__}")
        missing = [k for k in _REQUIRED_FIELDS if not item.get(k)]
        if missing:
            raise BotRoleConfigError(f"bots[{i}] missing required fields: {missing}")
        whitelist_raw = item.get("chat_whitelist") or ()
        if isinstance(whitelist_raw, (list, tuple)):
            whitelist = tuple(str(x) for x in whitelist_raw)
        else:
            raise BotRoleConfigError(
                f"bots[{i}].chat_whitelist must be a list, got {type(whitelist_raw).__name__}"
            )
        out.append(BotRole(
            app_id=str(item["app_id"]),
            role=str(item["role"]),
            mention_alias=str(item["mention_alias"]),
            runner=str(item["runner"]),
            default_cwd=str(item["default_cwd"]),
            prompt_template=str(item["prompt_template"]),
            reply_template=str(item.get("reply_template") or ""),
            chat_whitelist=whitelist,
            next_bot_mention=str(item.get("next_bot_mention") or ""),
            relay_writer_app_id=str(item.get("relay_writer_app_id") or ""),
        ))
    return out


# ---------------------------------------------------------------------------
# event matching
# ---------------------------------------------------------------------------

# lark-cli convertlib 把 @mention 渲染为 `@<name><space><space><body>`，
# 但某些客户端只用单空格 / 中文空格。这里都容忍。
_MENTION_SEP = (" ", " ", "　")


def _starts_with_mention(content: str, alias: str) -> bool:
    prefix = f"@{alias}"
    if not content.startswith(prefix):
        return False
    rest = content[len(prefix):]
    if not rest:
        return True  # @bot 单独一条
    return rest[0] in _MENTION_SEP


def event_matches_bot(event: Dict[str, Any], bot: BotRole) -> bool:
    """这条 IM 事件是否应被 ``bot`` 处理。"""
    if event.get("chat_type") != "group":
        return False
    if event.get("message_type") != "text":
        return False
    if bot.chat_whitelist:
        if event.get("chat_id") not in bot.chat_whitelist:
            return False
    content = event.get("content") or ""
    return _starts_with_mention(content, bot.mention_alias)


def extract_message_body(event: Dict[str, Any], bot: BotRole) -> str:
    """从 event.content 剥离开头 ``@<alias><sep>``，返回剩余正文。

    若 content 不以该 mention 起头，原样返回（调用方应先 :func:`event_matches_bot` 过滤）。
    """
    content = event.get("content") or ""
    prefix = f"@{bot.mention_alias}"
    if not content.startswith(prefix):
        return content
    rest = content[len(prefix):]
    # 吃掉所有连续的分隔符
    while rest and rest[0] in _MENTION_SEP:
        rest = rest[1:]
    return rest
