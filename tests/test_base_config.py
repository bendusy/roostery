"""Tests for roostery.base_config."""
from __future__ import annotations
from pathlib import Path

import pytest

from roostery.base_config import BaseConfig, load_all, resolve_by_base_token, resolve_by_role


def _write_yaml(p: Path, name: str, content: str) -> Path:
    f = p / f"{name}.yaml"
    f.write_text(content, encoding="utf-8")
    return f


_FULL = """\
role: Coder
base_token: tokABC
table_id: tblXYZ
stage_to_bot:
  "🎯 待办": planner_bot
  "🛠️ 实现": dev_bot
output_mirror:
  dev_bot: 关联 PR/Commit
"""


def test_load_all_returns_indexed_configs(tmp_path: Path) -> None:
    _write_yaml(tmp_path, "coder", _FULL)
    configs = load_all(tmp_path)
    assert len(configs) == 1
    c = configs[0]
    assert isinstance(c, BaseConfig)
    assert c.role == "Coder"
    assert c.base_token == "tokABC"
    assert c.table_id == "tblXYZ"
    assert c.stage_to_bot == {"🎯 待办": "planner_bot", "🛠️ 实现": "dev_bot"}
    assert c.output_mirror == {"dev_bot": "关联 PR/Commit"}


def test_load_all_handles_missing_dir(tmp_path: Path) -> None:
    missing = tmp_path / "nope"
    assert load_all(missing) == []


def test_load_all_loads_multiple_sorted(tmp_path: Path) -> None:
    _write_yaml(tmp_path, "b", _FULL)
    _write_yaml(
        tmp_path,
        "a",
        """\
role: 公众号-2026
base_token: tok2
table_id: tbl2
stage_to_bot:
  "📋 选题": selector_bot
""",
    )
    configs = load_all(tmp_path)
    assert len(configs) == 2
    assert configs[0].role == "公众号-2026"  # a.yaml first
    assert configs[1].role == "Coder"


def test_load_rejects_missing_required_fields(tmp_path: Path) -> None:
    _write_yaml(
        tmp_path,
        "bad",
        """\
role: Broken
base_token: tokX
stage_to_bot:
  "stage": bot
""",
    )
    with pytest.raises(ValueError, match="missing"):
        load_all(tmp_path)


def test_resolve_by_role_returns_match(tmp_path: Path) -> None:
    _write_yaml(tmp_path, "coder", _FULL)
    configs = load_all(tmp_path)
    c = resolve_by_role(configs, "Coder")
    assert c is not None
    assert c.base_token == "tokABC"


def test_resolve_by_role_missing_returns_none(tmp_path: Path) -> None:
    _write_yaml(tmp_path, "coder", _FULL)
    configs = load_all(tmp_path)
    assert resolve_by_role(configs, "Nonexistent") is None


def test_resolve_by_base_token_returns_match(tmp_path: Path) -> None:
    _write_yaml(tmp_path, "coder", _FULL)
    configs = load_all(tmp_path)
    c = resolve_by_base_token(configs, "tokABC")
    assert c is not None
    assert c.role == "Coder"


def test_output_mirror_optional_defaults_empty(tmp_path: Path) -> None:
    _write_yaml(
        tmp_path,
        "minimal",
        """\
role: Minimal
base_token: tokM
table_id: tblM
stage_to_bot:
  "x": bot_x
""",
    )
    configs = load_all(tmp_path)
    assert configs[0].output_mirror == {}


def test_initial_stage_optional_defaults_none(tmp_path: Path) -> None:
    (tmp_path / "a.yaml").write_text(
        "role: A\nbase_token: bx\ntable_id: tbl_a\nstage_to_bot:\n  s1: bot_a\n",
        encoding="utf-8",
    )
    configs = load_all(tmp_path)
    assert configs[0].initial_stage is None


def test_initial_stage_read_when_present(tmp_path: Path) -> None:
    (tmp_path / "a.yaml").write_text(
        'role: A\nbase_token: bx\ntable_id: tbl_a\n'
        'stage_to_bot:\n  "📋 选题": bot_a\n'
        'initial_stage: "📋 选题"\n',
        encoding="utf-8",
    )
    configs = load_all(tmp_path)
    assert configs[0].initial_stage == "📋 选题"


def test_nl_keywords_optional_defaults_none(tmp_path: Path) -> None:
    (tmp_path / "a.yaml").write_text(
        "role: A\nbase_token: bx\ntable_id: tbl_a\nstage_to_bot:\n  s1: bot_a\n",
        encoding="utf-8",
    )
    configs = load_all(tmp_path)
    assert configs[0].nl_keywords is None


def test_nl_keywords_parses_strong_weak(tmp_path: Path) -> None:
    (tmp_path / "a.yaml").write_text(
        "role: A\nbase_token: bx\ntable_id: tbl_a\n"
        "stage_to_bot:\n  s1: bot_a\n"
        "nl_keywords:\n"
        "  strong: [公众号, 写一篇]\n"
        "  weak: [内容, 发布]\n",
        encoding="utf-8",
    )
    configs = load_all(tmp_path)
    kw = configs[0].nl_keywords
    assert kw is not None
    assert kw["strong"] == ["公众号", "写一篇"]
    assert kw["weak"] == ["内容", "发布"]


def test_digest_optional_defaults_none(tmp_path: Path) -> None:
    (tmp_path / "a.yaml").write_text(
        "role: A\nbase_token: bx\ntable_id: tbl_a\nstage_to_bot:\n  s1: bot_a\n",
        encoding="utf-8",
    )
    configs = load_all(tmp_path)
    assert configs[0].digest is None


def test_digest_parses_decision_stages(tmp_path: Path) -> None:
    (tmp_path / "a.yaml").write_text(
        "role: A\nbase_token: bx\ntable_id: tbl_a\n"
        "stage_to_bot:\n  s1: bot_a\n"
        "digest:\n"
        "  decision_stages: [\"📝 修订\", \"✅ 发布\"]\n"
        "  product_fields: [产物, 关联文档]\n",
        encoding="utf-8",
    )
    configs = load_all(tmp_path)
    d = configs[0].digest
    assert d is not None
    assert d["decision_stages"] == ["📝 修订", "✅ 发布"]
    assert d["product_fields"] == ["产物", "关联文档"]


@pytest.mark.parametrize("yaml_snippet", [
    "nl_keywords: null\n",
    "nl_keywords: {}\n",
    "nl_keywords: not-a-dict\n",
])
def test_nl_keywords_degrades_to_none(tmp_path: Path, yaml_snippet: str) -> None:
    (tmp_path / "a.yaml").write_text(
        "role: A\nbase_token: bx\ntable_id: tbl_a\n"
        "stage_to_bot:\n  s1: bot_a\n" + yaml_snippet,
        encoding="utf-8",
    )
    configs = load_all(tmp_path)
    assert configs[0].nl_keywords is None
