"""Stop hook python 入口。从 shell 脚本调用，避免 shell 拼复杂 JSON。"""
from __future__ import annotations

import argparse
import sys
from typing import Optional

from roostery import task_writer
from roostery.lark_cli import LarkCLIError, run_json


def _send_im_fallback(receive_id: str, text: str, idempotency_key: str) -> None:
    """task_writer 失败时的兜底——纯文本 IM。任何异常都吞掉，确保 exit 0。"""
    try:
        run_json(
            [
                "im", "+messages-send",
                "--as", "bot",
                "--user-id", receive_id,
                "--text", text,
                "--idempotency-key", idempotency_key,
            ],
            timeout=15,
        )
    except Exception as e:
        sys.stderr.write(f"[roostery.stop_hook] IM fallback failed: {e}\n")


def run(
    *,
    agent: str,
    session: str,
    cwd: str,
    summary: str,
    assignee_open_id: Optional[str],
) -> int:
    """主入口。失败兜底不阻塞 agent。

    ``assignee_open_id`` 缺省时走 identity.resolve_user_open_id()：先 env
    ``FEISHU_NOTIFY_TO``，再 lark-cli active profile 的 user_open_id，再
    ``~/.feishu_hub/config.yaml`` 的 notify_receive_id。三个全空才真静默退出。
    """
    if not assignee_open_id:
        from roostery.identity import resolve_user_open_id
        assignee_open_id = resolve_user_open_id()
    if not assignee_open_id:
        return 0  # 没配通知对象 → 静默退出

    try:
        ref = task_writer.get_or_create_for_session(
            agent=agent,
            session=session,
            cwd=cwd,
            summary=f"[{agent}] @ {cwd.rsplit('/', 1)[-1]}",
            description=f"Agent {agent} working in {cwd}",
            assignee_open_id=assignee_open_id,
        )
        task_writer.append_steps(
            ref.guid,
            steps=[summary or "Agent stopped (no summary)"],
            idempotency_key=f"{agent}-{session}-step-{hash(summary) & 0xFFFFFFFF:x}",
        )
        return 0
    except LarkCLIError as e:
        sys.stderr.write(f"[roostery.stop_hook] task path failed: {e}\n")
        # 降级到 IM text 兜底
        _send_im_fallback(
            assignee_open_id,
            f"[{agent}] @ {cwd.rsplit('/', 1)[-1]}: {summary[:120]}",
            f"{agent}-stop-{session}-fallback",
        )
        return 0


def _main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--agent", required=True)
    p.add_argument("--session", required=True)
    p.add_argument("--cwd", required=True)
    p.add_argument("--summary", default="")
    # 接受两种 CLI flag：新 --assignee-open-id（推荐）+ 旧 --follower-open-id
    # （兼容已部署在 ~/.feishu_hub/bin/agent-stop-notify.sh 的旧 shell 脚本）。
    p.add_argument("--assignee-open-id", default="")
    p.add_argument("--follower-open-id", default="")
    args = p.parse_args()
    return run(
        agent=args.agent,
        session=args.session,
        cwd=args.cwd,
        summary=args.summary,
        assignee_open_id=args.assignee_open_id or args.follower_open_id,
    )


if __name__ == "__main__":
    sys.exit(_main())
