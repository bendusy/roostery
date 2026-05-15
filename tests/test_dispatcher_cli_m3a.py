"""M3.A 契约：cli 模块不再 import bitable_writer / 不再有 tail 子命令。"""
import importlib
import sys


def test_cli_does_not_import_bitable_writer():
    # 清掉 cache 强制重新加载，避免之前的测试污染
    for key in [k for k in sys.modules if k.startswith("roostery.dispatcher")]:
        del sys.modules[key]

    importlib.import_module("roostery.dispatcher.cli")

    # 检查 import 链上不应触及 bitable_writer 模块（覆盖：直接 import / 别名 import / from x import y）
    assert "roostery.dispatcher.bitable_writer" not in sys.modules, \
        "M3.A: cli.py import 链不应触及 bitable_writer 模块"


def test_cli_has_no_tail_subcommand():
    from roostery.dispatcher import cli
    parser = cli.build_parser()
    # 子命令列表里不应有 tail
    subparsers_action = next(
        a for a in parser._actions if a.__class__.__name__ == "_SubParsersAction"
    )
    assert "tail" not in subparsers_action.choices, \
        "M3.A: cli 不应保留 tail 子命令（违反协同模型）"
    # fire / replay 必须仍在
    assert "fire" in subparsers_action.choices
    assert "replay" in subparsers_action.choices
