"""nl_router LLM 版单测：mock _get_llm_caller 返回 canned JSON。"""
import pytest
from typing import Callable, Optional

from roostery.base_config import BaseConfig
from roostery.nl_router import parse


def _cfg_gzh() -> BaseConfig:
    return BaseConfig(
        role="公众号-2026",
        base_token="K6Y",
        table_id="tbl_gzh",
        stage_to_bot={"📋 选题": "selector_bot", "✏️ 草稿": "drafter_bot"},
        initial_stage="📋 选题",
        nl_keywords={"strong": ["公众号", "写一篇", "文章"], "weak": ["内容", "发布"]},
    )


def _cfg_coder() -> BaseConfig:
    return BaseConfig(
        role="Coder",
        base_token="TauG",
        table_id="tbl_coder",
        stage_to_bot={"🎯 待办": "planner_bot"},
        initial_stage="🎯 待办",
        nl_keywords={"strong": ["bug", "issue", "修"], "weak": []},
    )


def _mock_llm(monkeypatch, response: str) -> None:
    """Replace _get_llm_caller with a fake returning canned response."""
    def fake_caller(prompt: str) -> str:
        return response
    monkeypatch.setattr("roostery.nl_router._get_llm_caller", lambda: fake_caller)


def _mock_llm_unavailable(monkeypatch) -> None:
    monkeypatch.setattr("roostery.nl_router._get_llm_caller", lambda: None)


def _mock_llm_raises(monkeypatch, exc: Exception) -> None:
    def fake_caller(prompt: str) -> str:
        raise exc
    monkeypatch.setattr("roostery.nl_router._get_llm_caller", lambda: fake_caller)


def test_parse_returns_none_on_empty_text(monkeypatch) -> None:
    _mock_llm(monkeypatch, '{"role":null}')
    res, tried = parse("", [_cfg_gzh()])
    assert res is None and tried is False
    res, tried = parse("   ", [_cfg_gzh()])
    assert res is None and tried is False


def test_parse_returns_none_when_no_candidates_have_nl_keywords(monkeypatch) -> None:
    _mock_llm(monkeypatch, '{"role":"X","title":"X","confidence":0.9}')
    cfg_no_kw = BaseConfig(
        role="X", base_token="b", table_id="t",
        stage_to_bot={"s1": "bot_a"}, nl_keywords=None,
    )
    res, tried = parse("公众号写一篇 AI", [cfg_no_kw])
    assert res is None and tried is False


def test_parse_returns_none_when_llm_unavailable(monkeypatch) -> None:
    _mock_llm_unavailable(monkeypatch)
    res, tried = parse("公众号写一篇 AI", [_cfg_gzh()])
    assert res is None and tried is False


def test_parse_returns_none_when_llm_raises(monkeypatch) -> None:
    _mock_llm_raises(monkeypatch, RuntimeError("network down"))
    res, tried = parse("公众号写一篇 AI", [_cfg_gzh()])
    assert res is None and tried is True


def test_parse_returns_none_when_role_is_null(monkeypatch) -> None:
    _mock_llm(monkeypatch, '{"role":null,"title":"","confidence":0.0,"why":"否定句"}')
    res, tried = parse("我不想关注公众号", [_cfg_gzh()])
    assert res is None and tried is True


def test_parse_returns_none_when_llm_hallucinates_role(monkeypatch) -> None:
    _mock_llm(monkeypatch, '{"role":"小红书","title":"X","confidence":0.9}')
    res, tried = parse("写一篇极简生活", [_cfg_gzh()])
    assert res is None and tried is True


def test_parse_returns_none_when_json_malformed(monkeypatch) -> None:
    _mock_llm(monkeypatch, "Sure, here it is: not-a-json")
    res, tried = parse("公众号写一篇", [_cfg_gzh()])
    assert res is None and tried is True


def test_parse_high_confidence_routing(monkeypatch) -> None:
    _mock_llm(
        monkeypatch,
        '{"role":"公众号-2026","title":"AI 产品设计入门","confidence":0.9,"why":"明确指向公众号写作"}',
    )
    res, tried = parse("公众号写一篇 AI 产品设计入门", [_cfg_gzh(), _cfg_coder()])
    assert res is not None and tried is False
    assert res.role == "公众号-2026"
    assert res.title == "AI 产品设计入门"
    assert res.initial_stage == "📋 选题"
    assert res.confidence == pytest.approx(0.9)
    assert res.raw_text == "公众号写一篇 AI 产品设计入门"
    assert "公众号" in res.why


def test_parse_clamps_confidence_to_unit_range(monkeypatch) -> None:
    _mock_llm(monkeypatch, '{"role":"公众号-2026","title":"X","confidence":1.5}')
    res, _ = parse("公众号写一篇 X", [_cfg_gzh()])
    assert res is not None
    assert res.confidence == 1.0

    _mock_llm(monkeypatch, '{"role":"公众号-2026","title":"Y","confidence":-0.5}')
    res, _ = parse("公众号写一篇 Y", [_cfg_gzh()])
    assert res is not None
    assert res.confidence == 0.0


def test_parse_handles_non_numeric_confidence(monkeypatch) -> None:
    _mock_llm(monkeypatch, '{"role":"公众号-2026","title":"X","confidence":"high"}')
    res, _ = parse("公众号 X", [_cfg_gzh()])
    assert res is not None
    assert res.confidence == 0.5  # fallback


def test_parse_empty_title_falls_back_to_raw_text(monkeypatch) -> None:
    _mock_llm(monkeypatch, '{"role":"公众号-2026","title":"","confidence":0.7}')
    res, _ = parse("公众号 写一篇", [_cfg_gzh()])
    assert res is not None
    assert res.title == "公众号 写一篇"


def test_parse_initial_stage_fallback_to_first_stage_to_bot(monkeypatch) -> None:
    _mock_llm(monkeypatch, '{"role":"Y","title":"foo","confidence":0.9}')
    cfg = BaseConfig(
        role="Y", base_token="b", table_id="t",
        stage_to_bot={"stage_one": "bot_a", "stage_two": "bot_b"},
        nl_keywords={"strong": ["alpha"], "weak": []},
        initial_stage=None,
    )
    res, _ = parse("alpha foo", [cfg])
    assert res is not None
    assert res.initial_stage == "stage_one"


def test_parse_returns_none_when_no_initial_stage_and_empty_stage_to_bot(monkeypatch) -> None:
    _mock_llm(monkeypatch, '{"role":"Z","title":"foo","confidence":0.9}')
    cfg = BaseConfig(
        role="Z", base_token="b", table_id="t",
        stage_to_bot={},
        nl_keywords={"strong": ["alpha"], "weak": []},
        initial_stage=None,
    )
    res, tried = parse("alpha foo", [cfg])
    assert res is None and tried is True


def test_parse_extracts_json_when_wrapped_in_markdown(monkeypatch) -> None:
    _mock_llm(
        monkeypatch,
        "```json\n{\"role\":\"公众号-2026\",\"title\":\"X\",\"confidence\":0.9}\n```",
    )
    res, _ = parse("公众号写一篇 X", [_cfg_gzh()])
    assert res is not None
    assert res.role == "公众号-2026"


# ---- Issue 7: LLM 输出格式 edge case 测试 ----

def test_parse_handles_json_with_extra_keys(monkeypatch) -> None:
    """LLM 多返回了字段 → 应该忽略不报错。"""
    _mock_llm(
        monkeypatch,
        '{"role":"公众号-2026","title":"X","confidence":0.9,"why":"foo","extra_key":"junk","another":42}',
    )
    res, tried_and_failed = parse("公众号写一篇 X", [_cfg_gzh()])
    assert res is not None
    assert res.role == "公众号-2026"
    assert tried_and_failed is False


def test_parse_handles_json_missing_optional_keys(monkeypatch) -> None:
    """LLM 漏了 why / confidence → 用默认值。"""
    _mock_llm(monkeypatch, '{"role":"公众号-2026","title":"X"}')
    res, tried_and_failed = parse("公众号 X", [_cfg_gzh()])
    assert res is not None
    assert res.confidence == 0.5  # fallback
    assert res.why == ""
    assert tried_and_failed is False


def test_parse_handles_prose_before_json(monkeypatch) -> None:
    """LLM 输出有前导话术 → 正则提取 JSON 仍 OK。"""
    _mock_llm(
        monkeypatch,
        'Sure, here is the JSON:\n\n{"role":"公众号-2026","title":"X","confidence":0.9}\n\nLet me know!',
    )
    res, tried_and_failed = parse("公众号 X", [_cfg_gzh()])
    assert res is not None
    assert res.role == "公众号-2026"
    assert tried_and_failed is False


def test_parse_returns_tried_and_failed_on_llm_exception(monkeypatch) -> None:
    """codex Q5: LLM 异常应返回 (None, True) 给 try_handle_nl 触发兜底回复。"""
    _mock_llm_raises(monkeypatch, RuntimeError("network down"))
    res, tried_and_failed = parse("公众号写一篇 AI", [_cfg_gzh()])
    assert res is None
    assert tried_and_failed is True


def test_parse_returns_tried_and_failed_on_null_role(monkeypatch) -> None:
    """LLM 显式 role=null 也算 tried_and_failed（spec §3 兜底）。"""
    _mock_llm(monkeypatch, '{"role":null,"title":"","confidence":0.0,"why":"否定"}')
    res, tried_and_failed = parse("我不想关注公众号", [_cfg_gzh()])
    assert res is None
    assert tried_and_failed is True


def test_parse_returns_silent_when_llm_unavailable(monkeypatch) -> None:
    """LLM 未配置应静默 fall-through，不触发兜底回复。"""
    _mock_llm_unavailable(monkeypatch)
    res, tried_and_failed = parse("公众号写一篇 AI", [_cfg_gzh()])
    assert res is None
    assert tried_and_failed is False


def test_parse_returns_silent_when_no_text(monkeypatch) -> None:
    """空 text → silent，不调 LLM。"""
    _mock_llm(monkeypatch, '{"role":"X"}')  # 不应被调用
    res, tried_and_failed = parse("", [_cfg_gzh()])
    assert res is None
    assert tried_and_failed is False


def test_parse_returns_silent_when_no_candidates(monkeypatch) -> None:
    """没有 nl_keywords 配置 → silent，不调 LLM。"""
    _mock_llm(monkeypatch, '{"role":"X"}')
    cfg = BaseConfig(role="X", base_token="b", table_id="t",
                     stage_to_bot={"s1": "bot_a"}, nl_keywords=None)
    res, tried_and_failed = parse("公众号写一篇 AI", [cfg])
    assert res is None
    assert tried_and_failed is False


def test_parse_invalidates_cache_after_llm_exception(monkeypatch) -> None:
    """codex Q5: 必须先 prime cache，再让 caller raise，验证 cache 被清。"""
    import roostery.nl_router as nr

    call_count = [0]

    def raising_caller(prompt: str) -> str:
        call_count[0] += 1
        raise RuntimeError("transient network error")

    # 预置 cached caller —— 关键：模拟"client 之前 resolve 成功，现在调用失败"
    nr._llm_caller = raising_caller

    # _get_llm_caller cache 命中路径：`if _llm_caller is not None: return _llm_caller`
    # 不需要 monkeypatch _get_llm_caller，cache 命中即走 raising_caller

    res, tried_and_failed = parse("公众号写一篇 X", [_cfg_gzh()])

    assert res is None
    assert tried_and_failed is True
    assert call_count[0] == 1  # caller 真被调过 1 次
    assert nr._llm_caller is None  # 关键断言：异常后 cache 被清


def test_parse_re_resolves_caller_after_cache_invalidation(monkeypatch) -> None:
    """补充：cache 清除后下次调用应重新 resolve（不卡在 stale state）。"""
    import roostery.nl_router as nr

    nr._llm_caller = None

    versions: list[str] = []

    def make_caller(version: str) -> Callable[[str], str]:
        def _c(prompt: str) -> str:
            return '{"role":"公众号-2026","title":"X","confidence":0.9}'
        return _c

    def fake_get_llm_caller() -> Optional[Callable[[str], str]]:
        if nr._llm_caller is not None:
            return nr._llm_caller
        v = f"v{len(versions) + 1}"
        versions.append(v)
        nr._llm_caller = make_caller(v)
        return nr._llm_caller

    monkeypatch.setattr("roostery.nl_router._get_llm_caller", fake_get_llm_caller)

    # 1st parse: resolve 一次，创建 v1
    parse("公众号写一篇 X", [_cfg_gzh()])
    assert versions == ["v1"]

    # 2nd parse: cache hit，不再 resolve
    parse("公众号写一篇 Y", [_cfg_gzh()])
    assert versions == ["v1"]

    # 模拟 LLM 故障后清 cache
    nr._llm_caller = None

    # 3rd parse: re-resolve，创建 v2
    parse("公众号写一篇 Z", [_cfg_gzh()])
    assert versions == ["v1", "v2"]


def test_parse_handles_multi_json_takes_first(monkeypatch) -> None:
    """codex 新发现风险：贪婪正则曾会吞两个对象，现改 balanced 扫描取第一个。"""
    _mock_llm(
        monkeypatch,
        '{"role":"公众号-2026","title":"A","confidence":0.9}\n{"role":"Coder","title":"B"}',
    )
    res, _ = parse("公众号写一篇 X", [_cfg_gzh(), _cfg_coder()])
    assert res is not None
    assert res.role == "公众号-2026"
    assert res.title == "A"


def test_parse_handles_nested_object_in_json(monkeypatch) -> None:
    """嵌套对象（如 why 字段含 dict）应正确括号匹配，不在第一个 } 处截断。"""
    _mock_llm(
        monkeypatch,
        '{"role":"公众号-2026","title":"X","confidence":0.9,"meta":{"nested":{"k":1}}}',
    )
    res, _ = parse("公众号写一篇 X", [_cfg_gzh()])
    assert res is not None
    assert res.role == "公众号-2026"


def test_parse_handles_json_with_brace_in_string(monkeypatch) -> None:
    """JSON 字符串值含 `{` `}` 时括号扫描不能误判 depth。"""
    _mock_llm(
        monkeypatch,
        '{"role":"公众号-2026","title":"题目{含括号}","confidence":0.9}',
    )
    res, _ = parse("公众号写一篇 X", [_cfg_gzh()])
    assert res is not None
    assert res.title == "题目{含括号}"
