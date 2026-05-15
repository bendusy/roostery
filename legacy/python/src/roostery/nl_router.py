"""LLM 版 NL → (role, title, initial_stage) 解析。

设计：docs/superpowers/specs/2026-05-14-m5-user-first-pivot-design.md §3
M5.A 升级版：1 次 llmcore 调用替代硬规则（spec §3 原 M5.B 计划前移）。
LLM 失败 / 无 GA → parse() 返回 (None, False) 静默 fall-through。
LLM 试过但失败（call raise / 非 JSON / role=null / 幻觉）→ (None, True) tried_and_failed。
复用 roostery.llm_summary.make_ga_summarizer（GA llmcore wrapper）。
"""
from __future__ import annotations

import json
import logging
from dataclasses import dataclass
from typing import Callable, List, Optional, Tuple

from roostery.base_config import BaseConfig

_log = logging.getLogger(__name__)


@dataclass(frozen=True)
class NLParseResult:
    role: str
    title: str
    initial_stage: str
    confidence: float
    raw_text: str
    why: str = ""


_PROMPT_TEMPLATE = """你是飞书机器人意图分类器。根据用户消息选择匹配的 role。

可选 role：
{roles}

判断规则：
- 用户拒绝任务（"我不想..."/"别给我..."/"没想做..."）→ role=null
- 闲聊无关内容（"天气真好"/"在吗"）→ role=null
- 命中 role 时提取简洁标题：去掉口语化前缀（"帮我"/"请"/"想"等），保留核心动作
- 任务约束（"公众号不要写 AI"/"小红书别用 emoji"）= 仍命中对应 role，title 含约束
- 支持中文、英文、繁简、emoji；忽略大小写
- confidence：强匹配 0.85+，模糊 0.5-0.7，不确定 <0.5

示例：
- "我不想关注公众号" → role=null（拒绝）
- "公众号不要写 AI 主题" → role="公众号-2026", title="不写 AI 主题"（任务约束）
- "天气真好" → role=null（闲聊）
- "Write an article about React on 公众号" → role="公众号-2026", title="Write an article about React"

用户消息：「{text}」

只返回 JSON（无 markdown，无多余文字）：
{{"role": "<role 名或 null>", "title": "<标题>", "confidence": <0-1 数字>, "why": "<一句中文理由>"}}"""

# Module-level cache：惰性 resolve LLM caller (prompt → str)。
# 显式声明类型让 monkeypatch 测试可替换。
_llm_caller: Optional[Callable[[str], str]] = None


def _get_llm_caller() -> Optional[Callable[[str], str]]:
    """复用 GA llmcore 客户端（通过 llm_summary.make_ga_summarizer 保持 layering）。

    GA 不可用返回 None。Cache 命中即复用；调用方负责在异常后清 cache（见 parse() 异常路径）。
    """
    global _llm_caller
    if _llm_caller is not None:
        return _llm_caller
    try:
        from roostery.llm_summary import make_ga_summarizer
    except Exception:
        _log.debug("nl_router: llm_summary import failed", exc_info=True)
        return None
    caller = make_ga_summarizer()
    if caller is None:
        return None
    _llm_caller = caller
    return _llm_caller


def _build_prompt(text: str, candidates: List[BaseConfig]) -> str:
    role_lines: List[str] = []
    for cfg in candidates:
        kw = cfg.nl_keywords or {}
        strong = kw.get("strong", [])
        weak = kw.get("weak", [])
        parts = []
        if strong:
            parts.append(f"强提示：{', '.join(strong)}")
        if weak:
            parts.append(f"弱提示：{', '.join(weak)}")
        hint = f"（{' / '.join(parts)}）" if parts else ""
        role_lines.append(f"- {cfg.role}{hint}")
    return _PROMPT_TEMPLATE.format(roles="\n".join(role_lines), text=text.strip())


def _parse_llm_json(raw: str) -> Optional[dict]:
    """Extract first balanced JSON object from raw LLM output.

    处理 LLM 常见返回形态：
    - 纯 JSON：`{"role": ...}`
    - markdown 包装：```json\n{...}\n```
    - 前导话术：`Sure, here it is: {"role": ...}`
    - 多个对象：取**第一个**完整 balanced {...}

    用括号匹配遍历，避免贪婪正则 `{.*}` 在多对象时取错。
    """
    if not raw or not raw.strip():
        return None

    # 快路径：raw 本身就是 JSON
    stripped = raw.strip()
    if stripped.startswith("{"):
        try:
            obj = json.loads(stripped)
            return obj if isinstance(obj, dict) else None
        except json.JSONDecodeError:
            pass  # 继续走括号遍历

    # 慢路径：扫描第一个 balanced {...}
    start = raw.find("{")
    if start < 0:
        return None
    depth = 0
    in_string = False
    escape = False
    for i in range(start, len(raw)):
        ch = raw[i]
        if escape:
            escape = False
            continue
        if ch == "\\":
            escape = True
            continue
        if ch == '"':
            in_string = not in_string
            continue
        if in_string:
            continue
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                candidate = raw[start : i + 1]
                try:
                    obj = json.loads(candidate)
                    return obj if isinstance(obj, dict) else None
                except json.JSONDecodeError:
                    return None
    return None


def parse(text: str, configs: List[BaseConfig]) -> Tuple[Optional[NLParseResult], bool]:
    """LLM-based NL parser.

    Returns:
        (result, tried_and_failed) where:
        - (NLParseResult, False)  — 成功
        - (None, False)           — silent fall-through（空 text / 无 candidates / 无 LLM）
        - (None, True)            — LLM 试过但失败（call raise / 非 JSON / role=null / 幻觉）
                                    → 调用方应回复 spec §3 兜底文案
    """
    if not text or not text.strip():
        return None, False

    candidates = [c for c in configs if c.nl_keywords]
    if not candidates:
        return None, False

    caller = _get_llm_caller()
    if caller is None:
        return None, False

    try:
        raw = caller(_build_prompt(text, candidates))
    except Exception:
        _log.exception("nl_router LLM call failed: text=%r", text[:100])
        global _llm_caller
        _llm_caller = None  # invalidate cache so next call re-resolves
        return None, True

    data = _parse_llm_json(raw)
    if not data:
        return None, True

    role = data.get("role")
    if not isinstance(role, str) or not role.strip():
        # role=null（否定 / 闲聊）或解析失败 → tried but failed
        return None, True

    cfg = next((c for c in candidates if c.role == role), None)
    if cfg is None:
        _log.warning("nl_router: LLM hallucinated role %r not in candidates", role)
        return None, True

    try:
        confidence = float(data.get("confidence", 0.5))
    except (TypeError, ValueError):
        confidence = 0.5
    confidence = max(0.0, min(1.0, confidence))

    raw_title = data.get("title", "")
    title = raw_title.strip() if isinstance(raw_title, str) else ""
    if not title:
        title = text.strip()

    initial_stage = cfg.initial_stage
    if initial_stage is None:
        if cfg.stage_to_bot:
            initial_stage = next(iter(cfg.stage_to_bot.keys()))
        else:
            return None, True

    raw_why = data.get("why", "")
    why = raw_why if isinstance(raw_why, str) else ""

    return NLParseResult(
        role=cfg.role,
        title=title,
        initial_stage=initial_stage,
        confidence=confidence,
        raw_text=text,
        why=why,
    ), False
