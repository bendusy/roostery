# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

🪺 **Roostery** — vendor-neutral, Feishu-native agent broker. Local daemon that bridges arbitrary agent runtimes (Claude Code / Codex / Gemini / OpenClaw / custom Python) into Feishu (Lark) as the collaboration surface. **Planning phase** (v0.0.0); code is an import from a prior `feishu_hub` baseline (M3.C → M5.A, ~7339 LOC, 681 tests upstream).

Python package layout, but `package.json` + `index.js` also reserve the npm namespace. The npm side is a placeholder — all real code is Python under `src/roostery/`.

## Commands

```bash
# Install (editable) + dev deps
pip install -e ".[dev]"

# Tests — pytest is configured via pyproject (testpaths=tests, pythonpath=src)
pytest                              # full suite
pytest tests/test_dispatcher_loop.py -v      # single file
pytest tests/test_dispatcher_loop.py::test_name   # single test
pytest -k "dispatcher and not cli"  # by expression

# Lint / type-check
ruff check src tests
mypy src

# Runtime entry points (after install + lark-cli setup)
python -m roostery init             # provisions ~/.feishu_hub, merges CC/Codex Stop hooks, deploys lark-cli shim
python -m roostery smoke            # regression probes against verified lark-cli command matrix
python -m roostery.dispatcher.cli   # dispatcher: fire / replay / test-rule subcommands
```

There is no Node build — `index.js` is a stub exporting `{version, status:"planning"}`.

## Architecture — the load-bearing mental model

> **Feishu is the shared state machine. `lark-cli` is the agent's syscall surface to Feishu. Roostery is only the execution bridge + local audit cache.**

This invariant is enforced in code review: any module that re-implements what `lark-cli` already does, or treats local state as the source of truth for collaboration data, will be deleted. See `src/roostery/README.md` for the full red-line.

### State ownership

| State                                    | Owner                                                                |
| ---------------------------------------- | -------------------------------------------------------------------- |
| Work-item lifecycle, agent step stream   | Feishu Task (`lark-cli task +create` / `append_task_steps`)          |
| Cross-agent live context                 | Feishu IM thread (`lark-cli im +messages-reply --thread`)            |
| Comments / collab traces                 | Feishu Docs comments, group chat                                     |
| Index / stats / dashboard views          | Feishu Base (index layer, **not** source of truth)                   |
| Cloud-side routing (@mention / cron)     | Feishu Base Workflow (`LarkMessageTrigger` / `TimerTrigger`)         |
| Local process / model calls / budget     | Local (`dispatcher.runners`, `dispatcher.budget`)                    |
| Audit / replay                           | Local journal jsonl (`journal.py`)                                   |

### Module map (`src/roostery/`)

- **`lark_cli.py`** — stable subprocess wrapper around `lark-cli` (JSON parsing, exception normalization). The only sanctioned way to talk to Feishu.
- **Shim & audit:** `shim.py` (PATH-prefix shim that transparently proxies real `lark-cli` and writes journal), `journal.py`, `redact.py`, `remoterefs.py` (extract `doc_token` / `record_id` etc. from stdout).
- **Local config / install:** `config.py` (`~/.feishu_hub/config.yaml`), `hooks_merge.py` (merges Stop hooks into `~/.claude/settings.json` / `~/.codex/hooks.json`), `onboarding.py`, `identity.py`, `templates/`.
- **Bot bridge (M3.B main path):** `task_writer.py` (creates Feishu task + appends step stream + session cache), `stop_hook.py` (shell→python bridge: task_writer first, IM fallback), `bot_runner.py`, `bot_bridge.py`, `bot_relay_task.py`, `bot_role.py`, `hitl_router.py`.
- **`dispatcher/`** — local execution bridge (thin since M3.A):
  - `cli.py` — `fire` (single hook), `replay` (debug), `test-rule`
  - `loop.py` — event → match rules → trace/budget gate → run runner → emit
  - `rules.py` — local hook → runner matching
  - `runners.py` — `cc_headless` / `codex_exec` / `gemini_headless` / `noop`
  - `trace.py` — `trace_id` / `depth` / `parent_event_id` chain (loop protection)
  - `budget.py` — call-count + cost ceilings
- **Reporting:** `git_log.py` (multi-repo aggregation), `llm_summary.py` (**only** module allowed to import a GA-style llmcore client), `daily_report.py`, `record_writer.py`.
- **Other:** `agent_detect.py`, `base_config.py`, `base_indexer.py`, `base_intent_router.py`, `event_bridge.py`, `runner_registry.py`.

### Hard rules

1. **No `lark-cli` reimplementation.** If Feishu has an API, call it through `lark_cli.py`. Do not reach out to `requests` for `open.feishu.cn` directly.
2. **Local state is cache, not truth.** Anything in `~/.feishu_hub/` (journal, state) is replayable audit — never the canonical record of collaboration. If you find code that reads local state to answer "what is the status of task X", that's a bug.
3. **`llm_summary.py` is the only file allowed to `import` external GA-style llmcore / mykey clients.** Other modules must stay vendor-neutral; agent runtime and LLM provider integrations go through adapters.
4. **lark-cli version is pinned at 1.0.28** for verified compatibility (especially `task agent_task_step_info append_task_steps` timestamp schema). Higher versions need a smoke re-run before adoption.
5. **`python -m roostery smoke` is the post-upgrade gate.** It exercises the verified command matrix (`im +messages-send`, `docs +create v2`, `docs +update overwrite`, `drive files list / +create-folder / move`). If any probe fails, `init` and `daily_report` refuse to run.

## Test conventions

- `tests/conftest.py` puts project root on `sys.path`; pytest config adds `src` to `pythonpath`.
- `tests/fake_runners/` holds stub agent binaries (e.g. `sleeper.py`) used by dispatcher tests — don't replace them with mocks unless you also adjust the subprocess paths.
- 40+ test files cover dispatcher rules / loop / budget / runners, bot bridge, base indexer, daily report, etc. New modules in `dispatcher/` or `bot_*` are expected to ship with matching `test_*.py`.

## When extending

- New agent runtime → add a runner in `dispatcher/runners.py` and register in `runner_registry.py`; do not bake provider-specific logic into `loop.py`.
- New Feishu surface → add a thin method to `lark_cli.py`; add a smoke probe; do **not** add a new top-level HTTP client.
- New hook source (Codex / Gemini / Cursor) → extend `hooks_merge.py` and `templates/`; the dispatcher is hook-agnostic by design.
