"""roostery.git_log 单测（用真 git 在 tmp_path 建仓）。"""
import datetime as _dt
import os
import shutil
import subprocess
from pathlib import Path

import pytest

from roostery import git_log

git_bin = shutil.which("git")
pytestmark = pytest.mark.skipif(git_bin is None, reason="git not installed")


def _git(repo: Path, *args, env_extra=None):
    env = dict(os.environ)
    env.setdefault("GIT_AUTHOR_NAME", "Tester")
    env.setdefault("GIT_AUTHOR_EMAIL", "t@example.com")
    env.setdefault("GIT_COMMITTER_NAME", "Tester")
    env.setdefault("GIT_COMMITTER_EMAIL", "t@example.com")
    if env_extra:
        env.update(env_extra)
    subprocess.run(["git", "-C", str(repo), *args], check=True, env=env,
                   capture_output=True)


@pytest.fixture
def repo(tmp_path):
    r = tmp_path / "demo"
    r.mkdir()
    _git(r, "init", "-q", "-b", "main")
    _git(r, "config", "user.name", "Tester")
    _git(r, "config", "user.email", "t@example.com")
    return r


def _commit(repo: Path, message: str, *, when: _dt.datetime, author: str = "Tester"):
    fp = repo / "F"
    fp.write_text((fp.read_text() if fp.exists() else "") + message + "\n")
    iso = when.strftime("%Y-%m-%dT%H:%M:%S")
    env = {
        "GIT_AUTHOR_NAME": author,
        "GIT_AUTHOR_EMAIL": f"{author.lower()}@example.com",
        "GIT_COMMITTER_NAME": author,
        "GIT_COMMITTER_EMAIL": f"{author.lower()}@example.com",
        "GIT_AUTHOR_DATE": iso,
        "GIT_COMMITTER_DATE": iso,
    }
    _git(repo, "add", "F")
    _git(repo, "commit", "-q", "-m", message, env_extra=env)


def test_list_commits_for_repo_filters_by_time(repo):
    now = _dt.datetime.now()
    _commit(repo, "yesterday", when=now - _dt.timedelta(days=1))
    _commit(repo, "now", when=now)
    since = _dt.datetime.combine(now.date(), _dt.time.min)
    out = git_log.list_commits_for_repo(repo, since=since)
    assert len(out) == 1
    assert out[0].subject == "now"
    assert out[0].sha


def test_list_commits_today_groups_multiple_repos(tmp_path):
    a = tmp_path / "a"; a.mkdir()
    b = tmp_path / "b"; b.mkdir()
    _git(a, "init", "-q", "-b", "main")
    _git(a, "config", "user.name", "T"); _git(a, "config", "user.email", "t@e")
    _git(b, "init", "-q", "-b", "main")
    _git(b, "config", "user.name", "T"); _git(b, "config", "user.email", "t@e")
    _commit(a, "in a", when=_dt.datetime.now())
    _commit(b, "in b", when=_dt.datetime.now())
    commits = git_log.list_commits_today([a, b])
    assert {c.repo for c in commits} == {"a", "b"}
    assert {c.subject for c in commits} == {"in a", "in b"}


def test_no_merges_flag_is_passed(monkeypatch, repo):
    """contract test：调 git 时必带 --no-merges。"""
    captured = {}
    real_run = subprocess.run

    def spy_run(args, **kw):
        captured["args"] = list(args)
        return real_run(args, **kw)

    monkeypatch.setattr("roostery.git_log.subprocess.run", spy_run)
    git_log.list_commits_for_repo(repo)
    assert "--no-merges" in captured["args"]


def test_nonexistent_repo_returns_empty(tmp_path):
    assert git_log.list_commits_for_repo(tmp_path / "nope") == []


def test_non_git_dir_returns_empty(tmp_path):
    d = tmp_path / "plain"
    d.mkdir()
    (d / "f.txt").write_text("hi")
    assert git_log.list_commits_for_repo(d) == []


def test_render_markdown_groups_by_repo(repo):
    now = _dt.datetime.now()
    _commit(repo, "alpha", when=now)
    _commit(repo, "beta", when=now)
    md = git_log.render_markdown(git_log.list_commits_today([repo]))
    assert "demo" in md
    assert "alpha" in md
    assert "beta" in md
    assert md.startswith("- **")


def test_render_markdown_empty():
    assert "（今日无新提交）" in git_log.render_markdown([])


def test_author_filter(repo):
    now = _dt.datetime.now()
    _commit(repo, "by alice", when=now, author="Alice")
    _commit(repo, "by bob", when=now, author="Bob")
    out = git_log.list_commits_today([repo], author="Alice")
    assert len(out) == 1
    assert out[0].author == "Alice"


def test_no_git_binary_graceful(monkeypatch, repo):
    """模拟 PATH 上没有 git：内部 _run_git 会捕获 FileNotFoundError。"""
    original = subprocess.run

    def fake_run(*args, **kwargs):
        raise FileNotFoundError("git not found")

    monkeypatch.setattr("roostery.git_log.subprocess.run", fake_run)
    assert git_log.list_commits_for_repo(repo) == []
