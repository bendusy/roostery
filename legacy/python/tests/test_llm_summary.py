"""roostery.llm_summary 单测。"""
from dataclasses import dataclass

from roostery import llm_summary as ls


@dataclass
class _C:
    repo: str
    sha: str
    subject: str
    when: str = "2026-05-12T14:00:00+08:00"
    author: str = "ben"


def test_build_prompt_no_data():
    out = ls.build_prompt([], [])
    assert "今日无 journal 事件" in out
    assert "今日无新提交" in out
    assert "主要完成" in out


def test_build_prompt_includes_journal_fields():
    records = [{
        "ts": "2026-05-12T14:23:05+08:00",
        "event_type": "lark_cli.invoke",
        "actor": {"agent": "cc"},
        "command": {"argv": ["im", "+messages-send", "--user-id", "x"]},
        "summary": "任务完成 @ ProjectX",
        "tags": ["task_done"],
    }]
    out = ls.build_prompt(records, [])
    assert "14:23" in out
    assert "cc" in out
    assert "task_done" in out
    assert "lark_cli.invoke" in out


def test_build_prompt_includes_commits():
    out = ls.build_prompt([], [_C("repo1", "abcdef0123", "feat: x"),
                                _C("repo2", "1234567890", "fix: y")])
    assert "repo1" in out
    assert "abcdef01" in out
    assert "feat: x" in out


def test_build_prompt_appends_manual():
    out = ls.build_prompt([], [], manual="  important note  ")
    assert "用户备注" in out
    assert "important note" in out


def test_summarize_uses_injected_summarizer():
    seen = {}

    def fake(prompt):
        seen["prompt"] = prompt
        return "### 主要完成\n- ok"

    out = ls.summarize([], [], summarizer=fake)
    assert out.startswith("### 主要完成")
    assert "journal 事件" in seen["prompt"]


def test_summarize_falls_back_when_summarizer_raises():
    def bad(prompt):
        raise RuntimeError("boom")

    out = ls.summarize([], [], summarizer=bad)
    assert "summarizer raised" in out


def test_summarize_uses_trivial_when_no_backend(monkeypatch):
    monkeypatch.setattr(ls, "make_gemini_summarizer", lambda **kw: None)
    monkeypatch.setattr(ls, "make_ga_summarizer", lambda **kw: None)
    out = ls.summarize([], [])
    assert "主要完成" in out
    assert "无可用 LLM" in out


def test_trivial_summarizer_returns_structured_md():
    out = ls.trivial_summarizer("anything")
    assert "### 主要完成" in out
    assert "### 进行中" in out
    assert "### 阻塞" in out
    assert "### 明日建议" in out


def test_make_ga_summarizer_returns_none_when_llmcore_absent(monkeypatch):
    """模拟 llmcore import 失败（典型：纯 CC/Codex 环境）。"""
    import sys
    import builtins
    real_import = builtins.__import__

    def _import(name, *a, **kw):
        if name == "llmcore":
            raise ModuleNotFoundError(name)
        return real_import(name, *a, **kw)

    monkeypatch.setattr(builtins, "__import__", _import)
    monkeypatch.delitem(sys.modules, "llmcore", raising=False)
    assert ls.make_ga_summarizer() is None


def test_wrap_client_handles_string_return():
    class C:
        def chat(self, messages): return "summary text"
    fn = ls._wrap_client(C())
    assert fn("x") == "summary text"


def test_wrap_client_handles_generator_return():
    class C:
        def chat(self, messages):
            yield "a"; yield "b"
    fn = ls._wrap_client(C())
    assert fn("x") == "ab"


def test_make_gemini_summarizer_returns_none_when_binary_missing(monkeypatch):
    monkeypatch.delenv("FEISHU_HUB_GEMINI_BIN", raising=False)
    monkeypatch.setattr(ls.shutil, "which", lambda _: None)
    assert ls.make_gemini_summarizer() is None


def test_make_gemini_summarizer_calls_subprocess(monkeypatch):
    import subprocess as _sp
    monkeypatch.setenv("FEISHU_HUB_GEMINI_BIN", "/fake/gemini")
    captured = {}

    class _CP:
        returncode = 0
        stdout = "### 主要完成\n- 写代码\n"
        stderr = ""

    def fake_run(cmd, **kw):
        captured["cmd"] = cmd
        captured["kw"] = kw
        return _CP()

    monkeypatch.setattr(ls.subprocess, "run", fake_run)
    fn = ls.make_gemini_summarizer(model="gemini-2.5-pro")
    out = fn("compose me a daily")
    assert "主要完成" in out
    assert captured["cmd"][0] == "/fake/gemini"
    assert "-p" in captured["cmd"]
    assert captured["cmd"][captured["cmd"].index("-p") + 1] == "compose me a daily"
    assert "--output-format" in captured["cmd"]
    assert captured["cmd"][captured["cmd"].index("-m") + 1] == "gemini-2.5-pro"


def test_make_gemini_summarizer_falls_back_on_nonzero(monkeypatch):
    monkeypatch.setenv("FEISHU_HUB_GEMINI_BIN", "/fake/gemini")

    class _CP:
        returncode = 1
        stdout = ""
        stderr = "quota exceeded"

    monkeypatch.setattr(ls.subprocess, "run", lambda *a, **kw: _CP())
    fn = ls.make_gemini_summarizer()
    out = fn("x")
    assert "主要完成" in out  # trivial fallback header
    assert "gemini exit=1" in out


def test_make_gemini_summarizer_falls_back_on_timeout(monkeypatch):
    import subprocess as _sp
    monkeypatch.setenv("FEISHU_HUB_GEMINI_BIN", "/fake/gemini")

    def raise_timeout(*a, **kw):
        raise _sp.TimeoutExpired("gemini", 1)

    monkeypatch.setattr(ls.subprocess, "run", raise_timeout)
    fn = ls.make_gemini_summarizer()
    out = fn("x")
    assert "gemini timeout" in out


def test_resolve_summarizer_prefer_gemini(monkeypatch):
    sentinel_gemini = lambda p: "GEMINI"
    monkeypatch.setattr(ls, "make_gemini_summarizer", lambda **kw: sentinel_gemini)
    monkeypatch.setattr(ls, "make_ga_summarizer", lambda **kw: lambda p: "GA")
    fn = ls.resolve_summarizer(prefer="gemini")
    assert fn("x") == "GEMINI"


def test_resolve_summarizer_prefer_ga(monkeypatch):
    monkeypatch.setattr(ls, "make_gemini_summarizer", lambda **kw: lambda p: "GEMINI")
    monkeypatch.setattr(ls, "make_ga_summarizer", lambda **kw: lambda p: "GA")
    fn = ls.resolve_summarizer(prefer="ga")
    assert fn("x") == "GA"


def test_resolve_summarizer_prefer_trivial(monkeypatch):
    """prefer=trivial 不调任何 LLM。"""
    monkeypatch.setattr(ls, "make_gemini_summarizer",
                        lambda **kw: (_ for _ in ()).throw(AssertionError("should not be called")))
    fn = ls.resolve_summarizer(prefer="trivial")
    assert "主要完成" in fn("x")


def test_resolve_summarizer_auto_prefers_gemini_over_ga(monkeypatch):
    monkeypatch.setattr(ls, "make_gemini_summarizer", lambda **kw: lambda p: "GEMINI")
    monkeypatch.setattr(ls, "make_ga_summarizer",
                        lambda **kw: (_ for _ in ()).throw(AssertionError("ga should not be tried first")))
    fn = ls.resolve_summarizer(prefer="auto")
    assert fn("x") == "GEMINI"


def test_resolve_summarizer_auto_falls_back_to_ga(monkeypatch):
    monkeypatch.setattr(ls, "make_gemini_summarizer", lambda **kw: None)
    monkeypatch.setattr(ls, "make_ga_summarizer", lambda **kw: lambda p: "GA")
    fn = ls.resolve_summarizer(prefer="auto")
    assert fn("x") == "GA"


def test_wrap_client_handles_exception():
    class C:
        def chat(self, messages): raise RuntimeError("nope")
    fn = ls._wrap_client(C())
    out = fn("x")
    assert "llm error" in out
    assert "主要完成" in out  # trivial fallback header
