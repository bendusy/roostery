"""三层解耦静态检查（设计 v2 §10）。

规则：``roostery/*`` 除 ``llm_summary.py`` 外，不得引用 GA 内部模块。
覆盖两层：
1. AST：静态 import / from-import；
2. 文本 grep：``importlib.import_module(...)`` / 字符串拼接导入。
"""
from __future__ import annotations

import ast
import re
from pathlib import Path
from typing import Iterable, List, Set

import pytest

PKG_ROOT = Path(__file__).resolve().parent.parent / "src" / "roostery"

# 禁止 import 的 GA 内部模块名
FORBIDDEN_TOP = {"ga", "agent_loop", "mykey", "bbs", "frontends",
                 "agentmain", "reflect"}
# 唯一豁免文件
EXEMPT = {"llm_summary.py"}


def _python_files() -> List[Path]:
    return sorted(p for p in PKG_ROOT.rglob("*.py") if "__pycache__" not in p.parts)


def _imports_in(nodes: Iterable[ast.AST]) -> Set[str]:
    out: Set[str] = set()
    for node in nodes:
        if isinstance(node, ast.Import):
            for n in node.names:
                out.add(n.name.split(".")[0])
        elif isinstance(node, ast.ImportFrom):
            if node.level and node.level > 0:
                continue  # 相对导入，本包内部
            if node.module:
                out.add(node.module.split(".")[0])
    return out


def _all_imports(src: str) -> Set[str]:
    """所有 import（含函数体内的 lazy import）。"""
    try:
        tree = ast.parse(src)
    except SyntaxError:
        return set()
    return _imports_in(ast.walk(tree))


def _top_level_imports(src: str) -> Set[str]:
    """仅模块顶层 import，不进入函数 / 类体。"""
    try:
        tree = ast.parse(src)
    except SyntaxError:
        return set()
    return _imports_in(tree.body)


# 文本扫描：识别 importlib.import_module / __import__ / exec 等
_DYN_IMPORT_RES = [
    re.compile(r"\bimportlib\.import_module\(\s*[\"']([^\"']+)[\"']"),
    re.compile(r"\b__import__\(\s*[\"']([^\"']+)[\"']"),
]


def _dyn_imports(src: str) -> Set[str]:
    out: Set[str] = set()
    for pat in _DYN_IMPORT_RES:
        for m in pat.finditer(src):
            out.add(m.group(1).split(".")[0])
    return out


@pytest.mark.parametrize("path", _python_files(),
                         ids=lambda p: str(p.relative_to(PKG_ROOT)))
def test_no_forbidden_imports(path: Path):
    rel = path.relative_to(PKG_ROOT)
    src = path.read_text(encoding="utf-8")
    dyn = _dyn_imports(src) & FORBIDDEN_TOP
    # llm_summary 是唯一允许的入口；不允许顶层 import GA（必须 lazy）
    if rel.name in EXEMPT:
        top = _top_level_imports(src) & FORBIDDEN_TOP
        assert top == set(), (
            f"{rel}: GA modules must be lazy-imported inside functions, "
            f"not at module top: {top}"
        )
        return
    # 其它文件：任何位置（顶层或函数体）都不许引用 GA
    all_imports = _all_imports(src) & FORBIDDEN_TOP
    assert all_imports == set(), (
        f"{rel}: forbidden imports: {all_imports}. "
        f"Only roostery/llm_summary.py may reference GA, and only lazily."
    )
    assert dyn == set(), (
        f"{rel}: forbidden dynamic imports: {dyn}"
    )


def test_llm_summary_uses_lazy_import_pattern():
    """llm_summary.py 必须把 GA import 放进函数体，不在模块顶层；
    且至少存在一处函数体内 lazy import GA 才说明这条豁免被实际使用。"""
    src = (PKG_ROOT / "llm_summary.py").read_text(encoding="utf-8")
    top = _top_level_imports(src) & FORBIDDEN_TOP
    assert top == set(), f"llm_summary.py 顶层 GA import: {top}"
    all_imports = _all_imports(src) & FORBIDDEN_TOP
    assert all_imports, (
        "llm_summary.py 没有任何 lazy GA import；如果不再需要豁免，"
        "请把它从 EXEMPT 移除"
    )


def test_text_denylist_no_path_assembly():
    """禁止用字符串拼装路径再 sys.path / open / exec 加载 GA 文件。"""
    for path in _python_files():
        src = path.read_text(encoding="utf-8")
        for name in FORBIDDEN_TOP:
            for pat in (f"'{name}.py'", f'"{name}.py"'):
                assert pat not in src, (
                    f"{path.relative_to(PKG_ROOT)}: suspicious path literal {pat}"
                )
