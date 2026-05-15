"""日报顶部"今日小结"的 LLM 摘要。

后端优先级（可被 config / 入参覆盖）：

1. 显式注入的 ``summarizer`` 参数（单测用）
2. **Gemini CLI**（`gemini -p ...` 子进程；用户钦点的文风）
3. GA llmcore（这是包内**唯一**允许 import GA 的位置）
4. ``trivial_summarizer``（兜底）

可在 ``~/.feishu_hub/config.yaml`` 的 ``daily_report.summarizer`` 字段强制选某后端。
"""
from __future__ import annotations

import os
import shutil
import subprocess
from typing import Any, Callable, List, Mapping, Optional, Sequence

# Callable[[prompt], summary_text]
Summarizer = Callable[[str], str]


SYSTEM_PROMPT = (
    "你是研发工程师的日报助手。基于 journal 事件与 git 提交，"
    "用简洁的中文给出 4 段：主要完成、进行中、阻塞、明日建议。"
    "每段不超过 3 条 bullet；总长度不超过 200 字；不要寒暄。"
)


def build_prompt(
    records: Sequence[Mapping[str, Any]],
    commits: Sequence[Any],
    manual: Optional[str] = None,
) -> str:
    """把 journal records + git commits + 手动备注组装为提示。

    纯数据函数；不调任何 LLM，便于单测覆盖。
    """
    lines: List[str] = [SYSTEM_PROMPT, "", "## journal 事件"]
    if not records:
        lines.append("（今日无 journal 事件）")
    else:
        for r in records:
            ts = (r.get("ts") or "")[11:16]
            actor = ((r.get("actor") or {}).get("agent") or "?")
            evt = r.get("event_type") or "?"
            summary = r.get("summary") or ""
            tags = ",".join(r.get("tags") or [])
            argv_first = ""
            cmd = r.get("command") or {}
            if isinstance(cmd, dict):
                argv = cmd.get("argv") or []
                if argv:
                    argv_first = " ".join(str(x) for x in argv[:3])
            extras = " | ".join(x for x in (tags, argv_first, summary) if x)
            lines.append(f"- [{ts}] {actor} {evt}" + (f" — {extras}" if extras else ""))

    lines += ["", "## git 提交"]
    if not commits:
        lines.append("（今日无新提交）")
    else:
        for c in commits:
            repo = getattr(c, "repo", "?")
            sha = getattr(c, "sha", "")[:8]
            subj = getattr(c, "subject", "")
            lines.append(f"- {repo} `{sha}` {subj}")

    if manual:
        lines += ["", "## 用户备注", manual.strip()]

    lines += [
        "",
        "请按要求输出 4 段 markdown，标题为：",
        "### 主要完成 / ### 进行中 / ### 阻塞 / ### 明日建议",
    ]
    return "\n".join(lines)


def make_gemini_summarizer(
    *,
    binary: Optional[str] = None,
    model: Optional[str] = None,
    timeout: int = 60,
) -> Optional[Summarizer]:
    """``gemini -p <prompt> --output-format text``；二进制不存在返回 ``None``。"""
    bin_ = binary or os.getenv("FEISHU_HUB_GEMINI_BIN") or shutil.which("gemini")
    if not bin_:
        return None

    def _call(prompt: str) -> str:
        cmd = [bin_, "-p", prompt, "--output-format", "text"]
        if model:
            cmd += ["-m", model]
        try:
            proc = subprocess.run(cmd, capture_output=True, text=True,
                                  timeout=timeout)
        except subprocess.TimeoutExpired:
            return trivial_summarizer(prompt) + "\n<!-- gemini timeout -->"
        if proc.returncode != 0:
            err = (proc.stderr or proc.stdout or "").strip()[:200]
            return trivial_summarizer(prompt) + f"\n<!-- gemini exit={proc.returncode}: {err} -->"
        text = proc.stdout.strip()
        return text or trivial_summarizer(prompt) + "\n<!-- gemini returned empty -->"

    return _call


def trivial_summarizer(prompt: str) -> str:
    """无 LLM 兜底：返回固定占位段，便于离线/无 GA 环境跑通。"""
    return (
        "### 主要完成\n- （无可用 LLM；请在 GA 环境运行或注入 summarizer）\n"
        "### 进行中\n- —\n"
        "### 阻塞\n- —\n"
        "### 明日建议\n- 配置 LLM 后重跑 `/daily` 可获得自动摘要。\n"
    )


def make_ga_summarizer(
    *,
    session_key: Optional[str] = None,
) -> Optional[Summarizer]:
    """惰性构造一个 GA llmcore 客户端；GA 不可用返回 ``None``。

    ``session_key`` 指定 mykey 里的某个 LLM 配置；未给则用第一个 chat-capable 的。
    """
    try:
        import llmcore  # type: ignore[import-not-found]
    except Exception:
        return None
    try:
        llmcore.reload_mykeys()
    except Exception:
        return None
    keys = _candidate_session_keys(session_key)
    for key in keys:
        try:
            client = llmcore.resolve_client(key)
        except Exception:
            continue
        if client is None:
            continue
        return _wrap_client(client)
    return None


def _candidate_session_keys(prefer: Optional[str]) -> List[str]:
    """决定尝试哪些 mykey session 名。"""
    if prefer:
        return [prefer]
    try:
        import mykey  # type: ignore[import-not-found]
    except Exception:
        return []
    candidates: List[str] = []
    for name in dir(mykey):
        if name.startswith("_"):
            continue
        val = getattr(mykey, name)
        if isinstance(val, dict) and "apikey" in val and "model" in val and "mixin" not in name:
            candidates.append(name)
    return candidates


def _wrap_client(client: Any) -> Summarizer:
    """把 GA client 包装成 ``Summarizer`` 接口。

    llmcore Client 使用 client.chat([messages]) 而非旧的 client.ask(prompt)；
    chat() 可能返回 str 或 generator，两者都处理。
    """
    def _call(prompt: str) -> str:
        try:
            ret = client.chat([{"role": "user", "content": prompt}])
        except Exception as e:
            return trivial_summarizer(prompt) + f"\n<!-- llm error: {e} -->"
        if isinstance(ret, str):
            return ret
        try:
            return "".join(ret)
        except Exception:
            return str(ret)
    return _call


def resolve_summarizer(
    *,
    prefer: Optional[str] = None,
    session_key: Optional[str] = None,
) -> Summarizer:
    """按优先级返回一个可用的 Summarizer。

    ``prefer`` 可选 ``gemini`` / ``ga`` / ``trivial``，强制使用某后端；缺省时
    按 Gemini > GA > trivial 试，第一个返回 ``None`` 之外的就用。
    """
    prefer = (prefer or "").strip().lower()
    if prefer == "trivial":
        return trivial_summarizer
    if prefer == "gemini":
        return make_gemini_summarizer() or trivial_summarizer
    if prefer == "ga":
        return make_ga_summarizer(session_key=session_key) or trivial_summarizer
    return (
        make_gemini_summarizer()
        or make_ga_summarizer(session_key=session_key)
        or trivial_summarizer
    )


def summarize(
    records: Sequence[Mapping[str, Any]],
    commits: Sequence[Any],
    *,
    manual: Optional[str] = None,
    summarizer: Optional[Summarizer] = None,
    session_key: Optional[str] = None,
    prefer: Optional[str] = None,
) -> str:
    """给定 journal records + git commits，返回 markdown 摘要段。

    优先级：``summarizer`` 参数 > ``prefer`` 指定后端 > Gemini > GA > ``trivial_summarizer``。
    """
    prompt = build_prompt(records, commits, manual=manual)
    fn: Summarizer = summarizer or resolve_summarizer(prefer=prefer,
                                                       session_key=session_key)
    try:
        text = fn(prompt)
    except Exception as e:
        text = trivial_summarizer(prompt) + f"\n<!-- summarizer raised: {e} -->"
    return text.strip()
