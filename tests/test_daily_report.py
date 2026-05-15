"""roostery.daily_report 单测（mock lark_cli + 临时 state）。"""
import datetime as _dt
import json
import os
from pathlib import Path
from unittest.mock import patch

import pytest

yaml = pytest.importorskip("yaml")

from roostery import config as cfgmod  # noqa: E402
from roostery import daily_report, journal, lark_cli  # noqa: E402


@pytest.fixture
def fhub_home(monkeypatch, tmp_path):
    home = tmp_path / "fhub"
    monkeypatch.setenv(cfgmod.ENV_ROOT, str(home))
    cfgmod.save(
        {
            "notify_receive_id": "ou_test",
            "daily_report": {
                "root_folder_token": "fldcnROOT",
                "monthly_subfolder": True,
                "git_repos": [],
                "summarizer": "trivial",  # 单测不调真 LLM
            },
        },
        path=home / "config.yaml",
    )
    return home


def _fake_summary(prompt: str) -> str:
    return "### 主要完成\n- mocked"


# ---- render_markdown -----------------------------------------------------

def test_render_markdown_has_all_sections():
    date = _dt.date(2026, 5, 12)
    md = daily_report.render_markdown(date, [], [],
                                       summary="### 主要完成\n- s",
                                       manual="hand note")
    assert "## 一、今日小结" in md
    assert "## 二、完成事件" in md
    assert "## 三、代码提交" in md
    assert "## 四、原始 lark-cli 调用时间线" in md
    assert "## 五、自由记录" in md
    assert "hand note" in md
    assert md.startswith("_自动生成于")
    # 不应再出现一级标题（避免与 docx title 重复）
    assert "\n# " not in "\n" + md
    weekday = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"][date.weekday()]
    assert weekday in md


def test_render_completed_events_filters_by_tag():
    records = [
        {"ts": "2026-05-12T09:00:00+08:00", "tags": ["task_done"],
         "actor": {"agent": "cc"}, "summary": "task A"},
        {"ts": "2026-05-12T10:00:00+08:00", "tags": [],
         "actor": {"agent": "cc"}, "summary": "no tag"},
        {"ts": "2026-05-12T11:00:00+08:00", "tags": ["task_done"],
         "actor": {"agent": "codex"}, "summary": "task B"},
    ]
    md = daily_report._render_completed_events(records)
    assert "task A" in md
    assert "task B" in md
    assert "no tag" not in md
    # 倒序
    assert md.index("task B") < md.index("task A")


def test_render_completed_events_empty_returns_placeholder():
    assert "今日无任务完成" in daily_report._render_completed_events([])


# ---- collect_records / state -------------------------------------------

def test_collect_records_skips_skipped(fhub_home):
    today = _dt.date.today()
    journal.append({"event_type": "lark_cli.invoke", "actor": {"agent": "cc"},
                    "ts": today.isoformat() + "T10:00:00+00:00"})
    journal.append({"event_type": "lark_cli.skipped",
                    "ts": today.isoformat() + "T11:00:00+00:00"})
    out = daily_report.collect_records(today)
    assert len(out) == 1
    assert out[0]["event_type"] == "lark_cli.invoke"


def test_state_load_save_roundtrip(fhub_home):
    state = {"date": "2026-05-12", "title": "x", "folder_token": "f",
             "doc_token": "d", "doc_url": "u",
             "record_count": 1, "commit_count": 0, "updated_at": "z"}
    daily_report._save_state(state)
    loaded = daily_report._load_state(_dt.date(2026, 5, 12))
    assert loaded["doc_token"] == "d"


def test_state_returns_none_when_missing(fhub_home):
    assert daily_report._load_state(_dt.date(2030, 1, 1)) is None


# ---- generate end-to-end (mock lark_cli) -------------------------------

def test_generate_creates_when_no_state(fhub_home, monkeypatch):
    with patch.object(lark_cli, "find_or_create_folder",
                      return_value="fldcnMONTH") as mk_folder, \
         patch.object(lark_cli, "find_doc_in_folder",
                      return_value=None) as mk_find, \
         patch.object(lark_cli, "docs_create_v2",
                      return_value=lark_cli.DocInfo("doxcnNEW",
                                                    "https://u")) as mk_create, \
         patch.object(lark_cli, "docs_update_overwrite") as mk_update, \
         patch.object(lark_cli, "im_send_text", return_value="om_x") as mk_im:
        rep = daily_report.generate(date=_dt.date(2026, 5, 12),
                                    summarizer=_fake_summary)
    assert rep.created is True
    assert rep.doc_token == "doxcnNEW"
    assert mk_create.call_count == 1
    assert mk_update.call_count == 0
    mk_folder.assert_called_once_with(parent_token="fldcnROOT", name="2026-05")
    mk_im.assert_called_once()
    state = daily_report._load_state(_dt.date(2026, 5, 12))
    assert state["doc_token"] == "doxcnNEW"
    assert state["folder_token"] == "fldcnMONTH"


def test_generate_updates_when_state_exists(fhub_home):
    daily_report._save_state({
        "date": "2026-05-12", "title": "x", "folder_token": "fldcnMONTH",
        "doc_token": "doxcnEXIST", "doc_url": "u",
        "record_count": 0, "commit_count": 0, "updated_at": "z",
    })
    with patch.object(lark_cli, "find_or_create_folder",
                      return_value="fldcnMONTH"), \
         patch.object(lark_cli, "docs_update_overwrite") as mk_update, \
         patch.object(lark_cli, "docs_create_v2") as mk_create, \
         patch.object(lark_cli, "im_send_text"):
        rep = daily_report.generate(date=_dt.date(2026, 5, 12),
                                    summarizer=_fake_summary)
    assert rep.created is False
    assert rep.doc_token == "doxcnEXIST"
    mk_update.assert_called_once()
    assert mk_create.call_count == 0


def test_generate_force_new_bypasses_state(fhub_home):
    daily_report._save_state({
        "date": "2026-05-12", "title": "x", "folder_token": "fldcnMONTH",
        "doc_token": "doxcnOLD", "doc_url": "u",
        "record_count": 0, "commit_count": 0, "updated_at": "z",
    })
    with patch.object(lark_cli, "find_or_create_folder", return_value="fldcnMONTH"), \
         patch.object(lark_cli, "find_doc_in_folder", return_value=None), \
         patch.object(lark_cli, "docs_update_overwrite") as mk_update, \
         patch.object(lark_cli, "docs_create_v2",
                      return_value=lark_cli.DocInfo("doxcnFORCED", None)), \
         patch.object(lark_cli, "im_send_text"):
        rep = daily_report.generate(date=_dt.date(2026, 5, 12),
                                    summarizer=_fake_summary, force_new=True)
    assert rep.created is True
    assert rep.doc_token == "doxcnFORCED"
    assert mk_update.call_count == 0


def test_generate_falls_back_to_drive_list_when_state_missing(fhub_home):
    with patch.object(lark_cli, "find_or_create_folder",
                      return_value="fldcnMONTH"), \
         patch.object(lark_cli, "find_doc_in_folder",
                      return_value="doxcnFOUND") as mk_find, \
         patch.object(lark_cli, "docs_update_overwrite") as mk_update, \
         patch.object(lark_cli, "docs_create_v2") as mk_create, \
         patch.object(lark_cli, "im_send_text"):
        rep = daily_report.generate(date=_dt.date(2026, 5, 12),
                                    summarizer=_fake_summary)
    assert rep.created is False
    assert rep.doc_token == "doxcnFOUND"
    assert mk_create.call_count == 0
    mk_find.assert_called_once()
    mk_update.assert_called_once()


def test_generate_no_notify_flag(fhub_home):
    with patch.object(lark_cli, "find_or_create_folder", return_value="f"), \
         patch.object(lark_cli, "find_doc_in_folder", return_value=None), \
         patch.object(lark_cli, "docs_create_v2",
                      return_value=lark_cli.DocInfo("doxcnX", None)), \
         patch.object(lark_cli, "im_send_text") as mk_im:
        daily_report.generate(date=_dt.date(2026, 5, 12),
                              summarizer=_fake_summary, notify=False)
    assert mk_im.call_count == 0


def test_generate_im_failure_does_not_fail_report(fhub_home):
    with patch.object(lark_cli, "find_or_create_folder", return_value="f"), \
         patch.object(lark_cli, "find_doc_in_folder", return_value=None), \
         patch.object(lark_cli, "docs_create_v2",
                      return_value=lark_cli.DocInfo("doxcnX", None)), \
         patch.object(lark_cli, "im_send_text",
                      side_effect=lark_cli.LarkCLIError(-1, "x", ["im"])):
        rep = daily_report.generate(date=_dt.date(2026, 5, 12),
                                    summarizer=_fake_summary)
    assert rep.doc_token == "doxcnX"
    # 应在 journal 写一条 notify_failed
    today = _dt.date.today() if not _dt.date(2026, 5, 12) else _dt.date.today()
    notify_failed = [r for r in journal.read_day()
                     if r.get("event_type") == "daily_report.notify_failed"]
    assert notify_failed


def test_generate_requires_root_folder_token(monkeypatch, tmp_path):
    home = tmp_path / "fhub"
    monkeypatch.setenv(cfgmod.ENV_ROOT, str(home))
    cfgmod.save(
        {"daily_report": {"root_folder_token": ""}},
        path=home / "config.yaml",
    )
    with pytest.raises(RuntimeError, match="root_folder_token not configured"):
        daily_report.generate(summarizer=_fake_summary)


def test_generate_skips_monthly_when_disabled(fhub_home):
    cfg = cfgmod.load(apply_env=False)
    cfg["daily_report"]["monthly_subfolder"] = False
    cfgmod.save(cfg)
    with patch.object(lark_cli, "find_or_create_folder") as mk_folder, \
         patch.object(lark_cli, "find_doc_in_folder", return_value=None), \
         patch.object(lark_cli, "docs_create_v2",
                      return_value=lark_cli.DocInfo("doxcnX", None)), \
         patch.object(lark_cli, "im_send_text"):
        daily_report.generate(date=_dt.date(2026, 5, 12),
                              summarizer=_fake_summary)
    # 关闭月文件夹时应直接用 root_folder_token，不调 find_or_create_folder
    assert mk_folder.call_count == 0


# ---- CLI -----------------------------------------------------------------

def test_cli_run_returns_zero(fhub_home, capsys):
    with patch.object(lark_cli, "find_or_create_folder", return_value="f"), \
         patch.object(lark_cli, "find_doc_in_folder", return_value=None), \
         patch.object(lark_cli, "docs_create_v2",
                      return_value=lark_cli.DocInfo("doxcnX", "https://u")), \
         patch.object(lark_cli, "im_send_text"):
        rc = daily_report.main(["run", "--date", "2026-05-12", "--no-notify"])
    assert rc == 0
    out = capsys.readouterr().out
    assert "doxcnX" in out
    assert "created" in out


def test_cli_run_handles_error(monkeypatch, tmp_path, capsys):
    home = tmp_path / "fhub"
    monkeypatch.setenv(cfgmod.ENV_ROOT, str(home))
    cfgmod.save({"daily_report": {"root_folder_token": "",
                                   "summarizer": "trivial"}},
                path=home / "config.yaml")
    rc = daily_report.main(["run", "--date", "2026-05-12", "--no-notify"])
    assert rc == 2
    out = capsys.readouterr().out
    assert "failed" in out
