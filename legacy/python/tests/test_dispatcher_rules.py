"""roostery.dispatcher.rules 单测。"""
from pathlib import Path

import pytest

yaml = pytest.importorskip("yaml")

from roostery.dispatcher import rules


# ---- compile_rule / load_rules_file -------------------------------------

def test_compile_rule_minimal():
    r = rules.compile_rule({
        "name": "x",
        "when": {"event_type": "agent.stop"},
        "action": {"runner": "noop", "prompt": "hi"},
    })
    assert r.name == "x"
    assert r.when["event_type"] == "agent.stop"
    assert r.action.runner == "noop"
    assert r.action.prompt == "hi"
    assert r.cont is False


def test_compile_rule_missing_name():
    with pytest.raises(ValueError, match="missing 'name'"):
        rules.compile_rule({"when": {}, "action": {"runner": "noop"}})


def test_compile_rule_missing_runner():
    with pytest.raises(ValueError, match="missing action.runner"):
        rules.compile_rule({"name": "n", "when": {}, "action": {}})


def test_load_rules_file_unknown_version(tmp_path):
    p = tmp_path / "r.yaml"
    p.write_text("version: 9\nrules: []\n", encoding="utf-8")
    with pytest.raises(ValueError, match="unsupported version"):
        rules.load_rules_file(p)


def test_load_rules_file_full(tmp_path):
    p = tmp_path / "r.yaml"
    p.write_text(
        "version: 1\n"
        "rules:\n"
        "  - name: a\n"
        "    when:\n"
        "      event_type: agent.session_end\n"
        "    action:\n"
        "      runner: noop\n"
        "      prompt: hi\n",
        encoding="utf-8",
    )
    rs = rules.load_rules_file(p)
    assert len(rs) == 1
    assert rs[0].name == "a"


# ---- _match_one ---------------------------------------------------------

def _r(when, action_runner="noop", **action_kw):
    action_kw.setdefault("prompt", "")
    return rules.Rule(
        name="r",
        when=when,
        action=rules.Action(runner=action_runner, **action_kw),
    )


def test_match_event_type_exact():
    r = _r({"event_type": "agent.stop"})
    assert rules._match_one(r, {"event_type": "agent.stop"})
    assert not rules._match_one(r, {"event_type": "agent.start"})


def test_match_actor_agent():
    r = _r({"actor.agent": "cc"})
    assert rules._match_one(r, {"actor": {"agent": "cc"}})
    assert not rules._match_one(r, {"actor": {"agent": "codex"}})


def test_match_cwd_glob():
    r = _r({"cwd_glob": "/Users/ben/Projects/*"})
    assert rules._match_one(r, {"cwd": "/Users/ben/Projects/Foo"})
    assert not rules._match_one(r, {"cwd": "/tmp/x"})
    assert not rules._match_one(r, {})  # cwd 缺


def test_match_tags_includes_all():
    r = _r({"tags_includes": ["task_done", "important"]})
    assert rules._match_one(r, {"tags": ["task_done", "important", "extra"]})
    assert not rules._match_one(r, {"tags": ["task_done"]})


def test_match_result_contains_in_summary():
    r = _r({"result_contains": "insufficient"})
    assert rules._match_one(r, {"summary": "result is insufficient yet"})
    assert not rules._match_one(r, {"summary": "all good"})


def test_match_result_contains_in_actor_result():
    r = _r({"result_contains": "needs more"})
    assert rules._match_one(r, {"actor": {"result": "needs more research"}})


def test_match_result_contains_in_stdout_head():
    r = _r({"result_contains": "TODO"})
    assert rules._match_one(r, {"io": {"stdout_head": "Hello\nTODO: fix\n"}})


def test_match_summary_regex():
    r = _r({"summary_regex": r"PR-\d+"})
    assert rules._match_one(r, {"summary": "merge PR-42 done"})
    assert not rules._match_one(r, {"summary": "no number"})


def test_match_multi_condition_and():
    r = _r({"event_type": "agent.stop", "actor.agent": "cc",
            "tags_includes": ["task_done"]})
    yes = {"event_type": "agent.stop",
           "actor": {"agent": "cc"},
           "tags": ["task_done"]}
    assert rules._match_one(r, yes)
    # 缺 tags 不匹配
    no = {"event_type": "agent.stop", "actor": {"agent": "cc"}}
    assert not rules._match_one(r, no)


# ---- self-event 防自激 --------------------------------------------------

def test_self_event_is_filtered():
    r = _r({"event_type": "dispatch.completed"})
    assert rules.matches([r], {"event_type": "dispatch.completed"}) == []


def test_agent_dispatched_is_filtered():
    r = _r({"event_type": "agent.dispatched"})
    assert rules.matches([r], {"event_type": "agent.dispatched"}) == []


# ---- 模板渲染 -----------------------------------------------------------

def test_render_pluck():
    out = rules.render("review cwd={{ cwd }} agent={{ trigger.actor.agent }}",
                        {"cwd": "/p", "trigger": {"actor": {"agent": "cc"}}})
    assert out == "review cwd=/p agent=cc"


def test_render_missing_path_becomes_empty():
    out = rules.render("x={{ a.b.c }}y", {"a": {"b": {}}})
    assert out == "x=y"


def test_render_default_filter_used_when_path_missing():
    out = rules.render('s={{ a.b | default("FB") }}', {"a": {}})
    assert out == "s=FB"


def test_render_default_filter_used_when_value_empty_string():
    out = rules.render('{{ x | default("FB") }}', {"x": ""})
    assert out == "FB"


def test_render_default_filter_bypassed_when_present():
    out = rules.render('{{ x | default("FB") }}', {"x": "real"})
    assert out == "real"


def test_render_default_filter_single_quote_ok():
    out = rules.render("{{ x | default('单引号') }}", {})
    assert out == "单引号"


def test_render_handles_empty_template():
    assert rules.render("", {}) == ""


# ---- to_spec / switch_by_field ------------------------------------------

def test_to_spec_renders_prompt():
    a = rules.Action(runner="noop",
                     prompt="hello {{trigger.actor.agent}} cwd={{cwd}}")
    spec = rules.to_spec(a, {"actor": {"agent": "cc"}, "cwd": "/proj"})
    assert spec.runner == "noop"
    assert spec.prompt == "hello cc cwd=/proj"


def test_to_spec_switch_by_field():
    a = rules.Action(
        runner="switch_by_field",
        prompt="task: {{fields.task}}",
        branch_field="fields.agent",
        branches={"cc": "cc_headless", "codex": "codex_exec"},
    )
    spec = rules.to_spec(a, {"fields": {"agent": "codex", "task": "build X"}})
    assert spec.runner == "codex_exec"
    assert spec.prompt == "task: build X"


def test_to_spec_switch_missing_branch_raises():
    a = rules.Action(
        runner="switch_by_field",
        branch_field="fields.agent",
        branches={"cc": "cc_headless"},
    )
    with pytest.raises(ValueError, match="no branch"):
        rules.to_spec(a, {"fields": {"agent": "gemini"}})


def test_to_spec_switch_missing_field_decl_raises():
    a = rules.Action(runner="switch_by_field", branches={"cc": "cc_headless"})
    with pytest.raises(ValueError, match="requires 'field'"):
        rules.to_spec(a, {"fields": {"agent": "cc"}})


def test_to_spec_uses_action_timeout():
    a = rules.Action(runner="noop", timeout_s=42)
    spec = rules.to_spec(a, {})
    assert spec.timeout_s == 42


def test_to_spec_default_timeout():
    from roostery.dispatcher.runners import DEFAULT_TIMEOUT_S
    a = rules.Action(runner="noop")
    spec = rules.to_spec(a, {})
    assert spec.timeout_s == DEFAULT_TIMEOUT_S


def test_to_spec_renders_cwd_template():
    a = rules.Action(runner="noop", cwd="{{cwd}}/sub")
    spec = rules.to_spec(a, {"cwd": "/p"})
    assert spec.cwd == "/p/sub"


# ---- matches / continue 行为 --------------------------------------------

def test_matches_stops_at_first_hit():
    r1 = _r({"event_type": "X"})
    r2 = _r({"event_type": "X"})
    hits = rules.matches([r1, r2], {"event_type": "X"})
    assert len(hits) == 1
    assert hits[0].rule is r1


def test_matches_continues_when_flag_set():
    r1 = rules.Rule(name="a", when={"event_type": "X"},
                    action=rules.Action(runner="noop"), cont=True)
    r2 = _r({"event_type": "X"})
    hits = rules.matches([r1, r2], {"event_type": "X"})
    assert len(hits) == 2
    assert [m.rule.name for m in hits] == ["a", "r"]


def test_matches_no_rule_hit_returns_empty():
    r = _r({"event_type": "X"})
    assert rules.matches([r], {"event_type": "Y"}) == []


def test_matches_invalid_switch_branch_skipped_silently():
    r = rules.Rule(
        name="bad",
        when={"event_type": "X"},
        action=rules.Action(runner="switch_by_field", branch_field="f",
                            branches={"a": "noop"}),
    )
    hits = rules.matches([r], {"event_type": "X", "f": "z"})
    assert hits == []
