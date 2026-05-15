#!/usr/bin/env python3
"""R5 HITL POC fake runner: 输出 prompt → sleep N 秒 → 退 0。

SIGTERM 时 Python 默认抛 KeyboardInterrupt 风格异常退出（returncode=-15）。
不需要显式 signal handler。

用 CC headless argv 风格调用兼容（吃 -p / --output-format / --model 等 flag）：
    sleeper.py -p "<prompt>" --output-format json [--sleep 30]
"""
from __future__ import annotations

import argparse
import sys
import time


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--sleep", type=int, default=30,
                   help="sleep duration in seconds (default 30)")
    # 用 parse_known_args 吃 cc-style flag（-p / --output-format / --model 等）
    args, unknown = p.parse_known_args()

    # 拼 prompt：所有非 flag 的位置参数或 -p 后跟值
    prompt_parts = []
    i = 0
    while i < len(unknown):
        token = unknown[i]
        if token == "-p" and i + 1 < len(unknown):
            prompt_parts.append(unknown[i + 1])
            i += 2
            continue
        if token.startswith("-"):
            # 吃掉 flag 跟它的值（如 --output-format json）
            if i + 1 < len(unknown) and not unknown[i + 1].startswith("-"):
                i += 2
            else:
                i += 1
            continue
        prompt_parts.append(token)
        i += 1
    prompt = " ".join(prompt_parts)

    # 以 cc -p --output-format json 风格输出，让 bot_runner final_text 提取成功
    print(f'{{"result": "sleeper: got prompt={prompt!r}, sleeping {args.sleep}s",'
          f' "cost_usd": 0, "num_tokens": 0}}')
    sys.stdout.flush()
    time.sleep(args.sleep)
    print(f'sleeper: done after {args.sleep}s', file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
