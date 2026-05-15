"""日报生成入口（S3）。

把 journal + git_log + llm_summary 串起来，调 lark_cli 写入飞书 docx；
幂等 state 文件存 ``doc_token``，重复运行优先 overwrite 已有文档。
"""
from __future__ import annotations

import datetime as _dt
import json
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence

from . import config as cfgmod
from . import git_log, journal, lark_cli, llm_summary


@dataclass
class DailyReport:
    date: _dt.date
    title: str
    doc_token: str
    doc_url: Optional[str]
    folder_token: str
    created: bool        # True = 本次新建，False = 复用已有
    record_count: int
    commit_count: int


# ---- state ----------------------------------------------------------------

def _state_dir() -> Path:
    return cfgmod.root_dir() / "state" / "reports"


def _state_path(date: _dt.date) -> Path:
    return _state_dir() / (date.strftime("%Y-%m-%d") + ".json")


def _load_state(date: _dt.date) -> Optional[Dict[str, Any]]:
    p = _state_path(date)
    if not p.exists():
        return None
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return None


def _save_state(state: Dict[str, Any]) -> Path:
    d = _state_dir()
    d.mkdir(parents=True, exist_ok=True)
    p = _state_path(_dt.date.fromisoformat(state["date"]))
    tmp = p.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(state, ensure_ascii=False, indent=2),
                   encoding="utf-8")
    os.replace(tmp, p)
    return p


# ---- 数据组装 -------------------------------------------------------------

def collect_records(date: _dt.date) -> List[Dict[str, Any]]:
    """读取 journal 当日所有 ``lark_cli.invoke`` 类事件（跳过 skipped）。"""
    rows = journal.read_day(date=date)
    return [r for r in rows if r.get("event_type") == "lark_cli.invoke"]


def collect_commits(cfg: Dict[str, Any], date: _dt.date) -> List[git_log.Commit]:
    repos = cfg.get("daily_report", {}).get("git_repos") or []
    return git_log.list_commits_today(repos, today=date)


# ---- 文档 markdown 渲染 ---------------------------------------------------

WEEKDAY_CN = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]


def _title(date: _dt.date) -> str:
    return f"日报 {date.strftime('%Y-%m-%d')}"


def _render_completed_events(records: Sequence[Dict[str, Any]]) -> str:
    """筛 ``tags`` 含 task_done 的事件，按时间倒序成表。"""
    rows: List[Dict[str, Any]] = []
    for r in records:
        if "task_done" in (r.get("tags") or []):
            rows.append(r)
    rows.sort(key=lambda r: r.get("ts") or "", reverse=True)
    if not rows:
        return "_（今日无任务完成事件）_"
    lines = ["| 时间 | Agent | 说明 |", "|---|---|---|"]
    for r in rows:
        ts = (r.get("ts") or "")[11:16]
        agent = ((r.get("actor") or {}).get("agent") or "?")
        summary = r.get("summary") or ""
        if not summary:
            cmd = r.get("command") or {}
            argv = cmd.get("argv") or []
            summary = " ".join(str(x) for x in argv[:3])
        lines.append(f"| {ts} | {agent} | {summary} |")
    return "\n".join(lines)


def _render_timeline(records: Sequence[Dict[str, Any]]) -> str:
    if not records:
        return "_（今日 journal 为空）_"
    lines = []
    for r in records:
        ts = (r.get("ts") or "")[11:16]
        agent = ((r.get("actor") or {}).get("agent") or "?")
        evt = r.get("event_type") or "?"
        cmd = r.get("command") or {}
        argv = cmd.get("argv") or []
        argv_str = " ".join(str(x) for x in argv[:6])
        rc = cmd.get("exit_code")
        lines.append(f"- `{ts}` **{agent}** {evt} `{argv_str}` → rc={rc}")
    return "\n".join(lines)


def render_markdown(
    date: _dt.date,
    records: Sequence[Dict[str, Any]],
    commits: Sequence[git_log.Commit],
    *,
    summary: str,
    manual: Optional[str] = None,
) -> str:
    weekday = WEEKDAY_CN[date.weekday()]
    parts = [
        f"_自动生成于 {_dt.datetime.now().astimezone().isoformat(timespec='seconds')} · {weekday}_",
        "",
        "## 一、今日小结",
        summary.strip() or "_(no summary)_",
        "",
        "## 二、完成事件",
        _render_completed_events(records),
        "",
        "## 三、代码提交",
        git_log.render_markdown(commits),
        "",
        "## 四、原始 lark-cli 调用时间线",
        _render_timeline(records),
    ]
    if manual:
        parts += ["", "## 五、自由记录", manual.strip()]
    return "\n".join(parts)


# ---- 主流程 ---------------------------------------------------------------

def _resolve_folder_token(cfg: Dict[str, Any], date: _dt.date) -> str:
    """根目录 + 月子文件夹（如启用）。"""
    daily = cfg["daily_report"]
    root = daily.get("root_folder_token") or ""
    if not root:
        raise RuntimeError(
            "daily_report.root_folder_token not configured; "
            "set it in ~/.feishu_hub/config.yaml"
        )
    if not daily.get("monthly_subfolder", True):
        return root
    month = date.strftime("%Y-%m")
    return lark_cli.find_or_create_folder(parent_token=root, name=month)


def _resolve_doc_token(
    folder_token: str,
    title: str,
    date: _dt.date,
    *,
    force_new: bool,
) -> Optional[str]:
    """state 主路径 → drive list 精确匹配回退。"""
    if force_new:
        return None
    state = _load_state(date)
    if state and state.get("doc_token") and state.get("folder_token") == folder_token:
        return state["doc_token"]
    return lark_cli.find_doc_in_folder(folder_token=folder_token, title=title)


def generate(
    *,
    date: Optional[_dt.date] = None,
    manual: Optional[str] = None,
    force_new: bool = False,
    notify: bool = True,
    summarizer: Optional[llm_summary.Summarizer] = None,
) -> DailyReport:
    """生成或更新当日日报，返回 :class:`DailyReport`。"""
    date = date or _dt.date.today()
    cfg = cfgmod.load()

    records = collect_records(date)
    commits = collect_commits(cfg, date)
    prefer = cfg.get("daily_report", {}).get("summarizer") or "auto"
    summary = llm_summary.summarize(records, commits,
                                    manual=manual, summarizer=summarizer,
                                    prefer=prefer)

    title = _title(date)
    markdown = render_markdown(date, records, commits,
                               summary=summary, manual=manual)

    folder_token = _resolve_folder_token(cfg, date)
    existing = _resolve_doc_token(folder_token, title, date, force_new=force_new)

    if existing:
        lark_cli.docs_update_overwrite(doc_token=existing, markdown=markdown,
                                       title=title)
        doc_token = existing
        doc_url = f"https://docs.feishu.cn/docx/{doc_token}"
        created = False
    else:
        info = lark_cli.docs_create_v2(parent_token=folder_token,
                                       markdown=markdown, title=title)
        doc_token = info.doc_token
        doc_url = info.url or f"https://docs.feishu.cn/docx/{doc_token}"
        created = True

    state = {
        "date": date.isoformat(),
        "title": title,
        "folder_token": folder_token,
        "doc_token": doc_token,
        "doc_url": doc_url,
        "record_count": len(records),
        "commit_count": len(commits),
        "updated_at": _dt.datetime.now().astimezone().isoformat(timespec="seconds"),
    }
    _save_state(state)

    if notify:
        target = cfg.get("notify_receive_id") or ""
        if target:
            try:
                lark_cli.im_send_text(
                    user_id=target,
                    text=f"📓 日报已{'生成' if created else '更新'} {doc_url}",
                    idempotency_key=f"daily-{date.isoformat()}",
                )
            except lark_cli.LarkCLIError as e:
                # 通知失败不影响日报本身的成功
                journal.append({
                    "event_type": "daily_report.notify_failed",
                    "source": "daily_report",
                    "summary": f"{e.code}: {e.msg}",
                    "tags": ["daily_report"],
                })

    return DailyReport(
        date=date, title=title,
        doc_token=doc_token, doc_url=doc_url,
        folder_token=folder_token, created=created,
        record_count=len(records), commit_count=len(commits),
    )


# ---- CLI ----------------------------------------------------------------

def main(argv: Optional[List[str]] = None) -> int:
    """``python -m roostery.daily_report run`` 入口。"""
    import argparse
    p = argparse.ArgumentParser(prog="roostery.daily_report")
    sub = p.add_subparsers(dest="cmd", required=True)
    p_run = sub.add_parser("run", help="生成或更新当日日报")
    p_run.add_argument("--date", help="YYYY-MM-DD（默认今日）")
    p_run.add_argument("--note", help="附加自由文本到日报第五段")
    p_run.add_argument("--force-new", action="store_true",
                       help="忽略 state / 已有同名文档，强制新建")
    p_run.add_argument("--no-notify", action="store_true",
                       help="跳过 IM 通知")
    args = p.parse_args(argv)

    if args.cmd == "run":
        date = _dt.date.fromisoformat(args.date) if args.date else None
        try:
            rep = generate(date=date, manual=args.note,
                           force_new=args.force_new, notify=not args.no_notify)
        except (RuntimeError, lark_cli.LarkCLIError) as e:
            print(f"[daily_report] failed: {e}")
            return 2
        action = "created" if rep.created else "updated"
        print(f"[daily_report] {action} {rep.title}")
        print(f"  url    = {rep.doc_url}")
        print(f"  token  = {rep.doc_token}")
        print(f"  records= {rep.record_count}, commits = {rep.commit_count}")
        return 0
    return 1


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
