"""agent_detect 测试：mock shutil.which 验证 detection + skip + describe。"""
from __future__ import annotations

from unittest.mock import patch

import pytest

from roostery.agent_detect import (
    AGENTS,
    AgentSpec,
    DetectResult,
    describe,
    detect_all,
    installed_only,
)


def test_agents_list_has_three_known():
    names = {a.name for a in AGENTS}
    assert names == {"cc", "codex", "gemini"}


def test_detect_all_when_all_present():
    fake_paths = {"claude": "/usr/bin/claude", "codex": "/usr/bin/codex", "gemini": "/usr/bin/gemini"}
    with patch("roostery.agent_detect.shutil.which", side_effect=fake_paths.get):
        results = detect_all()
    assert len(results) == 3
    assert all(r.installed for r in results)


def test_detect_all_when_only_cc_present():
    def fake_which(cmd):
        return "/usr/bin/claude" if cmd == "claude" else None
    with patch("roostery.agent_detect.shutil.which", side_effect=fake_which):
        results = detect_all()
    inst = installed_only(results)
    assert len(inst) == 1
    assert inst[0].spec.name == "cc"


def test_skip_explicitly_excluded():
    """skip=['codex'] 即使 cli 装了也算未装。"""
    with patch("roostery.agent_detect.shutil.which", return_value="/usr/bin/anything"):
        results = detect_all(skip=["codex"])
    by_name = {r.spec.name: r for r in results}
    assert by_name["codex"].installed is False
    assert by_name["cc"].installed is True


def test_describe_renders():
    """describe 输出应该格式化清晰。"""
    fake_paths = {"claude": "/p/cc", "codex": None, "gemini": "/p/gemini"}
    with patch("roostery.agent_detect.shutil.which", side_effect=fake_paths.get):
        results = detect_all()
    out = describe(results)
    assert "[✓] cc" in out
    assert "[—] codex" in out or "[-]" in out or "[—]  codex" in out  # 容差
    assert "/p/cc" in out
