"""把 Stop hook 片段合并进现有 JSON 配置（CC ``settings.json`` / Codex ``hooks.json``）。

幂等：以命令字符串作为去重标识；多次合并不重复写入。
原子：先写 ``.tmp`` 再 ``os.replace``。
失败容错：原文件可解析失败时不破坏，直接报错。
"""
from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any, Dict, List, Optional


# ---- 模板渲染 -----------------------------------------------------------

TEMPLATES_DIR = Path(__file__).parent / "templates"


def render_template(name: str, *, hook_script: str) -> Dict[str, Any]:
    src = (TEMPLATES_DIR / name).read_text(encoding="utf-8")
    src = src.replace("{{HOOK_SCRIPT}}", hook_script)
    return json.loads(src)


# ---- merge --------------------------------------------------------------

def _load_existing(path: Path) -> Dict[str, Any]:
    if not path.exists():
        return {}
    raw = path.read_text(encoding="utf-8").strip()
    if not raw:
        return {}
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as e:
        raise RuntimeError(f"existing {path} is not valid JSON: {e}") from e
    if not isinstance(data, dict):
        raise RuntimeError(f"{path} top-level must be an object, got {type(data).__name__}")
    return data


def _is_same_command(existing_cmd: Any, target_cmd: str) -> bool:
    """如果命令字符串包含我们脚本路径，视为同一条 hook。"""
    if not isinstance(existing_cmd, str):
        return False
    # 取 ``HOOK_SCRIPT`` 这段（去掉 env 前缀）作匹配键
    return existing_cmd.strip().endswith(target_cmd_tail(target_cmd))


def target_cmd_tail(target_cmd: str) -> str:
    """去掉 ``KEY=VAL`` env 前缀，留命令本体。"""
    parts = target_cmd.split()
    for i, p in enumerate(parts):
        if "=" not in p:
            return " ".join(parts[i:])
    return target_cmd


def _detect_event_key(fragment: Dict[str, Any]) -> str:
    """从 fragment["hooks"] 取唯一的 event key（Stop / SessionEnd / AfterAgent ...）。"""
    keys = list((fragment.get("hooks") or {}).keys())
    if len(keys) != 1:
        raise ValueError(f"fragment.hooks 必须只有 1 个 event key，实际：{keys}")
    return keys[0]


def merge_event_hook(
    target_path: Path,
    fragment: Dict[str, Any],
) -> Dict[str, Any]:
    """把 ``fragment["hooks"][<event>][0]`` 合并到 ``target_path`` 的同 event 数组。

    支持任意 hook event（CC Stop / SessionEnd / Gemini AfterAgent / 等）；自动
    从 fragment 探测 event key，**不**写死 Stop。M3.A 之前叫 ``merge_stop_hook``
    硬绑 Stop，本期 (M3.E.A) 重命名并扩展。

    Returns
    -------
    最终写入的完整对象。
    """
    event = _detect_event_key(fragment)
    new_matcher_entry = fragment["hooks"][event][0]
    new_hook = new_matcher_entry["hooks"][0]
    new_cmd = new_hook["command"]

    data = _load_existing(target_path)
    data.setdefault("hooks", {})
    arr = data["hooks"].setdefault(event, [])

    # 找同 matcher 的项；没有就追加；有就 hooks 数组内去重 append
    matcher = new_matcher_entry.get("matcher", "*")
    bucket: Optional[Dict[str, Any]] = None
    for item in arr:
        if isinstance(item, dict) and item.get("matcher") == matcher:
            bucket = item
            break
    if bucket is None:
        arr.append(new_matcher_entry)
        return data

    bucket_hooks = bucket.setdefault("hooks", [])
    for h in bucket_hooks:
        if isinstance(h, dict) and _is_same_command(h.get("command"), new_cmd):
            # 已存在，更新 timeout 但不重复 append
            if "timeout" in new_hook and new_hook["timeout"] != h.get("timeout"):
                h["timeout"] = new_hook["timeout"]
            return data
    bucket_hooks.append(new_hook)
    return data


# 旧名兼容（M3.A 老测试可能引用；本期 deprecated 但不删）
merge_stop_hook = merge_event_hook


def _frag_valid(fragment: Dict[str, Any]) -> bool:
    """fragment.hooks.<event>[0].hooks[0].command 非空即合法。"""
    try:
        events = list((fragment.get("hooks") or {}).keys())
        if not events:
            return False
        h = fragment["hooks"][events[0]][0]["hooks"][0]
        return bool(h.get("command"))
    except (KeyError, IndexError, TypeError):
        return False


def write_json(path: Path, data: Dict[str, Any]) -> Path:
    """原子写 JSON。"""
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n",
                   encoding="utf-8")
    os.replace(tmp, path)
    return path


def apply_template(
    *,
    template_name: str,
    target_path: Path,
    hook_script: str,
) -> Path:
    """一站式：渲染模板 → 合并到 target → 原子写回。"""
    fragment = render_template(template_name, hook_script=hook_script)
    merged = merge_stop_hook(target_path, fragment)
    return write_json(target_path, merged)
