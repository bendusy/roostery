"""agent_detect — 检测本机装了哪些 AI agent CLI（CC / Codex / Gemini）。

产品理念：roostery init 不强制要求用户列清单，自动检测后只 hook 装了的。
没装的不强求；装了但用户不想 hook 的，提供 ``--skip-agent`` 覆盖。

依据 memory ``agent_hooks_facts.md``（必读，更新随 agent CLI 升级）：
- **CC**：``~/.claude/settings.json``，事件 ``SessionEnd``（per-session，非 Stop）
- **Codex**：本身**无**用户 hook；``~/.codex/hooks.json`` 实际是
  ``codex-plugin-cc`` 这个 CC 插件用的，跟 CC 一样的 hook 格式。我们仍 merge
  到这个文件——用户装了 codex-plugin-cc 才能用上，没装也不会破坏。
- **Gemini CLI**：``~/.gemini/settings.json``，事件 ``SessionEnd``；首选
  ``AfterAgent`` 因为 stdin 直接含 ``prompt_response``，但 per-turn 会刷屏，
  所以这里也用 ``SessionEnd``。
"""
from __future__ import annotations

import os
import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import List, Optional


@dataclass(frozen=True)
class AgentSpec:
    """一种 AI agent CLI 的检测 + hook 配置参数。"""
    name: str                 # 显示名："cc" / "codex" / "gemini"
    cli: str                  # 探测用的可执行命令名："claude" / "codex" / "gemini"
    template: str             # roostery/templates/ 下的 .json.tmpl 文件名
    hooks_target: Path        # 用户主目录下的目标 hook 配置文件路径


def _expand(p: str) -> Path:
    return Path(os.path.expanduser(p))


AGENTS: List[AgentSpec] = [
    AgentSpec(
        name="cc",
        cli="claude",
        template="claude_code_settings.json.tmpl",
        hooks_target=_expand("~/.claude/settings.json"),
    ),
    AgentSpec(
        name="codex",
        cli="codex",
        template="codex_hooks.json.tmpl",
        hooks_target=_expand("~/.codex/hooks.json"),
    ),
    AgentSpec(
        name="gemini",
        cli="gemini",
        template="gemini_settings.json.tmpl",
        hooks_target=_expand("~/.gemini/settings.json"),
    ),
]


@dataclass(frozen=True)
class DetectResult:
    """检测结果。``cli_path`` None 表示未装。"""
    spec: AgentSpec
    cli_path: Optional[str]

    @property
    def installed(self) -> bool:
        return self.cli_path is not None


def detect_all(*, skip: Optional[List[str]] = None) -> List[DetectResult]:
    """检测全部 ``AGENTS``。``skip=["codex"]`` 跳过 codex 强制 unknown。"""
    skip_set = set(skip or [])
    out: List[DetectResult] = []
    for spec in AGENTS:
        if spec.name in skip_set:
            out.append(DetectResult(spec=spec, cli_path=None))
            continue
        path = shutil.which(spec.cli)
        out.append(DetectResult(spec=spec, cli_path=path))
    return out


def installed_only(results: List[DetectResult]) -> List[DetectResult]:
    """过滤出真装了的。"""
    return [r for r in results if r.installed]


def describe(results: List[DetectResult]) -> str:
    """格式化检测结果给人看。"""
    lines = []
    for r in results:
        mark = "✓" if r.installed else "—"
        path = r.cli_path or "(not installed)"
        lines.append(f"  [{mark}] {r.spec.name:7} | cli={path}")
    return "\n".join(lines)
