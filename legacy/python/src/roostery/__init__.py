"""🪺 Roostery — vendor-neutral agent broker, Feishu-native.

A roost for your agent flock. Initial code import from feishu_hub baseline
(M3.C → M5.A LLM 版, 7339 LOC, 681 tests).

设计约束：本包除 ``llm_summary`` 子模块外，不得 import 外部 GA-style 客户端
（llmcore / mykey 等）；agent runtime / LLM provider 通过 adapter 接入。

See README.md and https://github.com/bendusy/roostery for project description.
"""

__all__ = ["journal", "redact", "remoterefs"]
__version__ = "0.0.0"
SCHEMA_VERSION = 1
