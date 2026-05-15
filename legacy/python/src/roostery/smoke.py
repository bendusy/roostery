"""lark-cli 子命令可用性 smoke 探测。

每条命令跑 ``--help``（不接触账号、不发请求），仅判定子命令是否仍然存在。
结果写入 ``state/smoke.json``，供 daily_report 入口在启动期拒绝执行降级状态。
"""
from __future__ import annotations

import datetime as _dt
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Dict, List, Optional, Tuple

from . import config as cfgmod

# 已验证命令清单。
#
# 每条探测**实际执行**带 `--dry-run` 的命令，让 lark-cli 真正解析所有 flag。
# 这能在 lark-cli 升级时立即发现 flag 改名 / 哑 flag（比单纯 `--help` 文本扫描严格）。
# 设计：发现 lark-cli 的 `--folder-token` 哑 flag 与 `--format` 不普适都是
# 通过真链路联调发现的（见 memory/lark_cli_quirks.md），smoke 必须能 catch 这类问题。
PROBES: List[Tuple[str, List[str]]] = [
    ("im_messages_send", [
        "im", "+messages-send",
        "--user-id", "ou_smoke", "--text", "probe",
        "--dry-run",
    ]),
    ("docs_create_v2", [
        "docs", "+create",
        "--api-version", "v2",
        "--folder-token", "fld_smoke",
        "--content", "# probe",
        "--doc-format", "markdown",
        "--dry-run",
    ]),
    ("docs_update_overwrite", [
        "docs", "+update",
        "--doc", "doc_smoke",
        "--mode", "overwrite",
        "--markdown", "# probe",
        "--new-title", "smoke",
        "--dry-run",
    ]),
    ("drive_files_list", [
        "drive", "files", "list",
        "--params", '{"folder_token":"fld_smoke","page_size":5}',
        "--as", "user",
        "--dry-run",
    ]),
    ("drive_create_folder", [
        "drive", "+create-folder",
        "--folder-token", "fld_smoke",
        "--name", "smoke",
        "--as", "user",
        "--dry-run",
    ]),
    ("drive_move", [
        "drive", "+move",
        "--file-token", "doc_smoke",
        "--folder-token", "fld_smoke",
        "--type", "docx",
        "--as", "user",
        "--dry-run",
    ]),
]


def _binary() -> str:
    """优先用 PATH 上的 lark-cli（部署后是 shim），其次 config.real_lark_cli。"""
    env = os.getenv("FEISHU_HUB_LARK_CLI_BIN")
    if env:
        return env
    cfg = cfgmod.load(apply_env=False)
    return cfg.get("shim", {}).get("real_lark_cli") or "lark-cli"


def _probe_one(binary: str, argv: List[str], *, timeout: int = 10) -> Dict[str, object]:
    try:
        proc = subprocess.run(
            [binary, *argv],
            capture_output=True, text=True, timeout=timeout,
        )
    except FileNotFoundError:
        return {"ok": False, "reason": f"binary not found: {binary}"}
    except subprocess.TimeoutExpired:
        return {"ok": False, "reason": f"timeout after {timeout}s"}
    text = (proc.stdout or "") + (proc.stderr or "")
    head = text[:500]
    # --dry-run 必须输出 "=== Dry Run ===" 段（lark-cli+ 行为），
    # 且退出码 0 才视为 flag 全被接受
    if proc.returncode == 0 and "Dry Run" in text:
        return {"ok": True, "rc": 0, "head": head}
    # unknown flag 之类的硬错：rc != 0 且 stderr 含 'unknown flag' / 'unknown command'
    if "unknown flag" in text.lower() or "unknown command" in text.lower():
        return {"ok": False, "rc": proc.returncode, "head": head,
                "reason": "flag/command mismatch (lark-cli upgrade?)"}
    return {"ok": False, "rc": proc.returncode, "head": head,
            "reason": f"unexpected exit {proc.returncode} or missing Dry Run marker"}


def run() -> Dict[str, object]:
    """跑所有 probe，返回结果字典并写入 state。"""
    binary = _binary()
    started = _dt.datetime.now().astimezone().isoformat(timespec="seconds")
    probes: Dict[str, Dict[str, object]] = {}
    for name, argv in PROBES:
        probes[name] = _probe_one(binary, argv)
    all_ok = all(p.get("ok") for p in probes.values())
    result = {
        "schema_version": 1,
        "binary": binary,
        "started_at": started,
        "all_ok": all_ok,
        "probes": probes,
    }
    _save(result)
    return result


def _save(result: Dict[str, object]) -> Path:
    state_dir = cfgmod.root_dir() / "state"
    state_dir.mkdir(parents=True, exist_ok=True)
    path = state_dir / "smoke.json"
    tmp = path.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(result, indent=2, ensure_ascii=False),
                   encoding="utf-8")
    os.replace(tmp, path)
    return path


def load_last() -> Optional[Dict[str, object]]:
    path = cfgmod.root_dir() / "state" / "smoke.json"
    if not path.exists():
        return None
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return None


def ensure_ready_or_raise() -> None:
    """daily_report 入口可调：上次 smoke 失败 / 未跑 → 抛 RuntimeError。"""
    state = load_last()
    if state is None:
        raise RuntimeError(
            "smoke probe never run; execute `python -m roostery.smoke` first"
        )
    if not state.get("all_ok"):
        bad = [k for k, v in (state.get("probes") or {}).items()
               if not v.get("ok")]
        raise RuntimeError(
            f"smoke probe last run reported failures: {bad}; "
            f"re-run `python -m roostery.smoke` after fixing"
        )


def main(argv: Optional[List[str]] = None) -> int:
    result = run()
    print(json.dumps(result, indent=2, ensure_ascii=False))
    return 0 if result["all_ok"] else 1


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main(sys.argv[1:]))
