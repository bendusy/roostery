"""聚合多个 git 仓库的"今日提交"。

零外部依赖；非 git 目录 / git 缺失 / 仓库无新提交时返回空列表，不抛异常。
"""
from __future__ import annotations

import datetime as _dt
import os
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import List, Optional, Sequence

DEFAULT_TIMEOUT = 10


@dataclass(frozen=True)
class Commit:
    repo: str
    sha: str
    when: str   # ISO，带时区
    author: str
    subject: str


def _run_git(repo: Path, args: Sequence[str], timeout: int) -> Optional[str]:
    """运行 git 子命令，返回 stdout；任何失败返回 None。"""
    try:
        proc = subprocess.run(
            ["git", "-C", str(repo), *list(args)],
            capture_output=True, text=True, timeout=timeout,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return None
    if proc.returncode != 0:
        return None
    return proc.stdout


def list_commits_for_repo(
    repo: Path,
    *,
    since: Optional[_dt.datetime] = None,
    until: Optional[_dt.datetime] = None,
    author: Optional[str] = None,
    timeout: int = DEFAULT_TIMEOUT,
) -> List[Commit]:
    """读取 ``repo`` 在 ``[since, until)`` 区间内的非 merge 提交。

    时间字符串用 ISO ``YYYY-MM-DDTHH:MM:SS`` 形式传给 git ``--since/--until``。
    """
    if not repo.exists():
        return []
    args = ["log", "--no-merges", "--pretty=format:%H%x09%aI%x09%an%x09%s"]
    if since is not None:
        args += ["--since", since.isoformat()]
    if until is not None:
        args += ["--until", until.isoformat()]
    if author:
        args += ["--author", author]

    out = _run_git(repo, args, timeout)
    if out is None:
        return []
    name = repo.name or str(repo)
    commits: List[Commit] = []
    for line in out.splitlines():
        parts = line.split("\t")
        if len(parts) < 4:
            continue
        sha, when, who, subject = parts[0], parts[1], parts[2], "\t".join(parts[3:])
        commits.append(Commit(repo=name, sha=sha, when=when,
                              author=who, subject=subject))
    return commits


def list_commits_today(
    repos: Sequence[os.PathLike],
    *,
    author: Optional[str] = None,
    today: Optional[_dt.date] = None,
    timeout: int = DEFAULT_TIMEOUT,
) -> List[Commit]:
    """所有仓库今日（本地午夜起，含未来）非 merge 提交，合并按时间倒序。"""
    today = today or _dt.date.today()
    # 本地时区
    tz = _dt.datetime.now().astimezone().tzinfo
    since = _dt.datetime.combine(today, _dt.time.min, tzinfo=tz)
    until = since + _dt.timedelta(days=1)
    out: List[Commit] = []
    for r in repos:
        out.extend(list_commits_for_repo(
            Path(r), since=since, until=until, author=author, timeout=timeout,
        ))
    out.sort(key=lambda c: c.when, reverse=True)
    return out


def render_markdown(commits: Sequence[Commit]) -> str:
    """按仓库分组渲染 markdown 列表。"""
    if not commits:
        return "_（今日无新提交）_"
    by_repo: dict = {}
    for c in commits:
        by_repo.setdefault(c.repo, []).append(c)
    lines: List[str] = []
    for repo in sorted(by_repo):
        lines.append(f"- **{repo}**")
        for c in by_repo[repo]:
            short = c.sha[:8]
            t = c.when[11:16] if "T" in c.when else c.when
            lines.append(f"  - `{short}` {t} — {c.subject}")
    return "\n".join(lines)
