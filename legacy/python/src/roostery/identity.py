"""identity — roostery 唯一身份解析层（user / bot / profile / host）。

设计原则：**lark-cli profile 是身份事实源，roostery 不发明 identity 概念**。

- lark-cli 每个 profile = 一个 ``(brand, appId, optional user)`` 元组
- `lark-cli profile use <name>` 切换 active profile
- `lark-cli auth status` 返回 active profile 的 user open_id + app_id + token 状态

roostery 调用方应该用本模块取身份，**不要**从 ``~/.feishu_hub/config.yaml`` 的
``notify_receive_id`` 隐式拿 user_open_id（那是 M3.B 之前的硬编码做法，仅作
兜底保留；多 user 切 profile 时它不更新）。

# 多机多用户场景

| 场景 | 怎么切 |
|---|---|
| 一台机器、一个用户、一个 bot | 不用切；默认 profile 即可 |
| 一台机器、一个用户、多个 bot（如 reviewer/scribe 分工） | `lark-cli profile add` 加新 profile 绑新 appId；`profile use` 切换 |
| 一台机器、多个用户（家庭账号 + 工作账号） | 同上——每个 user 对应一个 profile |
| 多台机器（mac + axis）、同一个用户 | 各自建 profile（appId 不同，user 相同） |

# 接口

- ``current_identity()`` —— 主入口，返回 ``Identity`` 不可变 dataclass
- ``list_profiles()`` —— 同 ``lark-cli profile list``
- ``Identity.short_user`` / ``short_bot`` —— 用于 task summary / IM 简称
"""
from __future__ import annotations

import json
import os
import socket
import subprocess
from dataclasses import dataclass
from typing import List, Optional


@dataclass(frozen=True)
class Identity:
    """当前 active 飞书身份三元组 + host。

    所有字段都可能 ``None`` —— 当 lark-cli 未配置 / 未登录时。调用方需自行兜底。
    """
    profile_name: Optional[str]
    user_open_id: Optional[str]
    user_name: Optional[str]
    bot_app_id: Optional[str]
    brand: Optional[str]
    token_status: Optional[str]
    host: str

    @property
    def short_user(self) -> str:
        """user_name 优先，缺则 open_id 后 6 位，再缺则 ``anon``。"""
        if self.user_name:
            return self.user_name
        if self.user_open_id:
            return self.user_open_id[-6:]
        return "anon"

    @property
    def short_bot(self) -> str:
        """bot_app_id 末 8 位（cli_xxxxxxxx）。"""
        if self.bot_app_id and self.bot_app_id.startswith("cli_"):
            return self.bot_app_id[4:12]
        if self.bot_app_id:
            return self.bot_app_id[:8]
        return "no-bot"

    @property
    def is_ready(self) -> bool:
        """token 有效且 user/bot 双备。"""
        return bool(
            self.user_open_id
            and self.bot_app_id
            and self.token_status == "valid"
        )

    def describe(self) -> str:
        """单行人类可读描述。"""
        status = "✓" if self.is_ready else "✗"
        return (
            f"{status} profile={self.profile_name or '?'} "
            f"user={self.short_user} ({self.user_open_id or '-'}) "
            f"bot={self.short_bot} ({self.bot_app_id or '-'}) "
            f"host={self.host} "
            f"token={self.token_status or '-'}"
        )


def _default_host() -> str:
    """优先 ``FEISHU_HUB_HOST`` env，缺则 ``socket.gethostname()`` 首段。"""
    explicit = os.getenv("FEISHU_HUB_HOST")
    if explicit:
        return explicit
    return socket.gethostname().split(".", 1)[0] or "unknown"


def _run_lark_cli(argv: List[str], timeout: int = 10) -> Optional[dict]:
    """跑 ``lark-cli ...`` 拿 JSON；失败时返回 None 不抛（identity 解析在启动期不应破坏调用方）。"""
    try:
        p = subprocess.run(
            ["lark-cli", *argv],
            capture_output=True, text=True, timeout=timeout,
        )
    except (subprocess.TimeoutExpired, FileNotFoundError):
        return None
    if p.returncode != 0:
        return None
    try:
        return json.loads(p.stdout)
    except json.JSONDecodeError:
        return None


def current_identity() -> Identity:
    """返回当前 active lark-cli profile + auth 解出的身份。

    无 lark-cli / 未配置 / 未登录任何一种情况都返回 host-only Identity（其他字段 None）。
    """
    host = _default_host()

    # auth status 拿 user open_id + appId
    auth = _run_lark_cli(["auth", "status"]) or {}
    user_open_id = auth.get("userOpenId")
    user_name = auth.get("userName")
    bot_app_id = auth.get("appId")
    brand = auth.get("brand")
    token_status = auth.get("tokenStatus")

    # profile list 拿当前 active profile name
    profiles = _run_lark_cli(["profile", "list"]) or []
    profile_name = None
    if isinstance(profiles, list):
        for p in profiles:
            if isinstance(p, dict) and p.get("active"):
                profile_name = p.get("name")
                break

    return Identity(
        profile_name=profile_name,
        user_open_id=user_open_id,
        user_name=user_name,
        bot_app_id=bot_app_id,
        brand=brand,
        token_status=token_status,
        host=host,
    )


def list_profiles() -> List[dict]:
    """list all configured lark-cli profiles。返回 lark-cli 原始 JSON 数组。"""
    return _run_lark_cli(["profile", "list"]) or []


def resolve_user_open_id(explicit: Optional[str] = None) -> Optional[str]:
    """user_open_id 解析优先级：

    1. 显式参数（调用方明确传入）
    2. ``FEISHU_NOTIFY_TO`` env（兼容 M3.B 旧路径）
    3. ``current_identity().user_open_id``（lark-cli auth status，跟随 profile 切换）
    4. ``~/.feishu_hub/config.yaml`` 的 ``notify_receive_id`` 字段（兜底；不推荐依赖）
    """
    if explicit:
        return explicit

    env = os.getenv("FEISHU_NOTIFY_TO")
    if env:
        return env

    ident = current_identity()
    if ident.user_open_id:
        return ident.user_open_id

    # 最后兜底：读 config.yaml（避免循环 import，延迟引入）
    try:
        from roostery import config as cfgmod
        cfg = cfgmod.load(apply_env=False)
        receive_id = cfg.get("notify_receive_id")
        if receive_id:
            return receive_id
    except Exception:
        pass

    return None
