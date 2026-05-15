"""roostery.hooks_merge 单测：模板渲染 + 幂等合并 + 原子写。"""
import json
from pathlib import Path

import pytest

from roostery import hooks_merge


HOOK_SCRIPT = "/Users/ben/.roostery/bin/agent-stop-notify.sh"


# ---- render_template ----

def test_render_claude_template():
    data = hooks_merge.render_template("claude_code_settings.json.tmpl",
                                        hook_script=HOOK_SCRIPT)
    cmd = data["hooks"]["SessionEnd"][0]["hooks"][0]["command"]
    assert HOOK_SCRIPT in cmd
    assert "FEISHU_HUB_AGENT=cc" in cmd
    assert "{{HOOK_SCRIPT}}" not in cmd


def test_render_codex_template():
    data = hooks_merge.render_template("codex_hooks.json.tmpl",
                                        hook_script=HOOK_SCRIPT)
    cmd = data["hooks"]["SessionEnd"][0]["hooks"][0]["command"]
    assert HOOK_SCRIPT in cmd
    assert "FEISHU_HUB_AGENT=codex" in cmd


# ---- merge_stop_hook ----

def test_merge_into_empty_file(tmp_path):
    p = tmp_path / "settings.json"
    frag = hooks_merge.render_template("claude_code_settings.json.tmpl",
                                        hook_script=HOOK_SCRIPT)
    merged = hooks_merge.merge_stop_hook(p, frag)
    hooks_merge.write_json(p, merged)
    loaded = json.loads(p.read_text())
    assert loaded == merged
    assert merged["hooks"]["SessionEnd"][0]["matcher"] == "*"


def test_merge_preserves_existing_unrelated(tmp_path):
    p = tmp_path / "settings.json"
    p.write_text(json.dumps({
        "theme": "dark",
        "permissions": {"allow": ["bash"]},
        "hooks": {"UserPromptSubmit": [{"matcher": "*", "hooks": []}]},
    }), encoding="utf-8")
    frag = hooks_merge.render_template("claude_code_settings.json.tmpl",
                                        hook_script=HOOK_SCRIPT)
    merged = hooks_merge.merge_stop_hook(p, frag)
    assert merged["theme"] == "dark"
    assert merged["permissions"]["allow"] == ["bash"]
    assert "UserPromptSubmit" in merged["hooks"]
    assert "SessionEnd" in merged["hooks"]


def test_merge_is_idempotent(tmp_path):
    p = tmp_path / "settings.json"
    frag = hooks_merge.render_template("claude_code_settings.json.tmpl",
                                        hook_script=HOOK_SCRIPT)
    merged1 = hooks_merge.merge_stop_hook(p, frag)
    hooks_merge.write_json(p, merged1)
    merged2 = hooks_merge.merge_stop_hook(p, frag)
    hooks_merge.write_json(p, merged2)
    # Stop 数组里 hooks 只有一个，没有重复
    stop_hooks = merged2["hooks"]["SessionEnd"][0]["hooks"]
    assert len(stop_hooks) == 1


def test_merge_appends_when_existing_hook_different(tmp_path):
    p = tmp_path / "settings.json"
    p.write_text(json.dumps({
        "hooks": {"SessionEnd": [{"matcher": "*", "hooks": [
            {"type": "command", "command": "echo old", "timeout": 5}
        ]}]}
    }), encoding="utf-8")
    frag = hooks_merge.render_template("claude_code_settings.json.tmpl",
                                        hook_script=HOOK_SCRIPT)
    merged = hooks_merge.merge_stop_hook(p, frag)
    stop_hooks = merged["hooks"]["SessionEnd"][0]["hooks"]
    assert len(stop_hooks) == 2
    assert any("echo old" in h["command"] for h in stop_hooks)
    assert any(HOOK_SCRIPT in h["command"] for h in stop_hooks)


def test_merge_adds_new_matcher_bucket(tmp_path):
    p = tmp_path / "settings.json"
    p.write_text(json.dumps({
        "hooks": {"SessionEnd": [{"matcher": "Read", "hooks": []}]}
    }), encoding="utf-8")
    frag = hooks_merge.render_template("claude_code_settings.json.tmpl",
                                        hook_script=HOOK_SCRIPT)
    merged = hooks_merge.merge_stop_hook(p, frag)
    matchers = [b["matcher"] for b in merged["hooks"]["SessionEnd"]]
    assert "Read" in matchers
    assert "*" in matchers


def test_merge_updates_timeout_on_existing(tmp_path):
    """同 command 已存在，timeout 不同则刷新值，不重复加。"""
    p = tmp_path / "settings.json"
    cmd = f"FEISHU_HUB_AGENT=cc {HOOK_SCRIPT}"
    p.write_text(json.dumps({
        "hooks": {"SessionEnd": [{"matcher": "*", "hooks": [
            {"type": "command", "command": cmd, "timeout": 99}
        ]}]}
    }), encoding="utf-8")
    frag = hooks_merge.render_template("claude_code_settings.json.tmpl",
                                        hook_script=HOOK_SCRIPT)
    merged = hooks_merge.merge_stop_hook(p, frag)
    stop_hooks = merged["hooks"]["SessionEnd"][0]["hooks"]
    assert len(stop_hooks) == 1
    assert stop_hooks[0]["timeout"] == 10  # template 值


def test_merge_rejects_invalid_existing(tmp_path):
    p = tmp_path / "settings.json"
    p.write_text("[]")  # 顶层 list 而不是 dict
    frag = hooks_merge.render_template("claude_code_settings.json.tmpl",
                                        hook_script=HOOK_SCRIPT)
    with pytest.raises(RuntimeError, match="top-level must be an object"):
        hooks_merge.merge_stop_hook(p, frag)


def test_merge_rejects_invalid_json(tmp_path):
    p = tmp_path / "settings.json"
    p.write_text("{ not valid")
    frag = hooks_merge.render_template("claude_code_settings.json.tmpl",
                                        hook_script=HOOK_SCRIPT)
    with pytest.raises(RuntimeError, match="not valid JSON"):
        hooks_merge.merge_stop_hook(p, frag)


# ---- apply_template (one-stop) ----

def test_apply_template_end_to_end(tmp_path):
    target = tmp_path / "settings.json"
    hooks_merge.apply_template(
        template_name="claude_code_settings.json.tmpl",
        target_path=target, hook_script=HOOK_SCRIPT,
    )
    data = json.loads(target.read_text())
    assert data["hooks"]["SessionEnd"][0]["hooks"][0]["command"].endswith(HOOK_SCRIPT)


def test_apply_template_writes_atomically(tmp_path):
    target = tmp_path / "settings.json"
    hooks_merge.apply_template(
        template_name="codex_hooks.json.tmpl",
        target_path=target, hook_script=HOOK_SCRIPT,
    )
    assert target.exists()
    # 不留 .tmp
    assert not (tmp_path / "settings.json.tmp").exists()
