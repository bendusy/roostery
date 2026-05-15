"""config.yaml 读写。

依赖 PyYAML（延迟 import，缺失时给出明确错误）。env 覆盖在 :func:`load` 中合并。
"""
from __future__ import annotations

import os
from pathlib import Path
from typing import Any, Dict, Optional

ENV_ROOT = "FEISHU_HUB_HOME"
ENV_REAL_LARK_CLI = "FEISHU_HUB_REAL_LARK_CLI"
ENV_NOTIFY_TO = "FEISHU_NOTIFY_TO"

DEFAULT_ROOT = Path(os.path.expanduser("~/.feishu_hub"))

DEFAULTS: Dict[str, Any] = {
    "notify_receive_id": "",
    "notify_receive_id_type": "open_id",
    "daily_report": {
        "root_folder_token": "",
        "monthly_subfolder": True,
        "cron": "0 21 * * *",
        "llm_summary": True,
        "summarizer": "auto",  # auto | gemini | ga | trivial
        "git_repos": [],
    },
    "bitable": {
        "enabled": False,
        "base_token": "",
        "table_id": "",
    },
    "shim": {
        "real_lark_cli": "",
        "stdout_head_bytes": 2048,
        "stderr_head_bytes": 2048,
        "interactive_verbs": ["login", "logout", "auth", "config"],
    },
}


def root_dir() -> Path:
    override = os.getenv(ENV_ROOT)
    return Path(override) if override else DEFAULT_ROOT


def config_path() -> Path:
    return root_dir() / "config.yaml"


def _yaml():
    try:
        import yaml  # type: ignore[import-not-found]
    except ImportError as e:  # pragma: no cover
        raise RuntimeError(
            "roostery.config requires PyYAML. Install with: pip install pyyaml"
        ) from e
    return yaml


def _deep_merge(base: Dict[str, Any], over: Dict[str, Any]) -> Dict[str, Any]:
    out = dict(base)
    for k, v in (over or {}).items():
        if isinstance(v, dict) and isinstance(out.get(k), dict):
            out[k] = _deep_merge(out[k], v)
        else:
            out[k] = v
    return out


def load(path: Optional[Path] = None, *, apply_env: bool = True) -> Dict[str, Any]:
    """读取配置；缺失文件时返回默认值。"""
    p = path or config_path()
    cfg = dict(DEFAULTS)
    cfg["daily_report"] = dict(DEFAULTS["daily_report"])
    cfg["shim"] = dict(DEFAULTS["shim"])
    if p.exists():
        yaml = _yaml()
        with p.open("r", encoding="utf-8") as f:
            raw = yaml.safe_load(f) or {}
        cfg = _deep_merge(cfg, raw)
    if apply_env:
        cfg = _apply_env_overrides(cfg)
    return cfg


def _apply_env_overrides(cfg: Dict[str, Any]) -> Dict[str, Any]:
    real = os.getenv(ENV_REAL_LARK_CLI)
    if real:
        cfg["shim"]["real_lark_cli"] = real
    notify = os.getenv(ENV_NOTIFY_TO)
    if notify:
        cfg["notify_receive_id"] = notify
    return cfg


def save(cfg: Dict[str, Any], path: Optional[Path] = None) -> Path:
    """写配置文件（原子写）。"""
    yaml = _yaml()
    p = path or config_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    tmp = p.with_suffix(p.suffix + ".tmp")
    with tmp.open("w", encoding="utf-8") as f:
        yaml.safe_dump(cfg, f, sort_keys=False, allow_unicode=True)
    os.replace(tmp, p)
    return p
