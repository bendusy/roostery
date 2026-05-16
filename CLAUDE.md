# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

🪺 **Roostery** — vendor-neutral, Feishu-native agent broker. Local daemon that bridges arbitrary agent runtimes (Claude Code / Codex / Gemini / OpenClaw / custom) into Feishu (Lark) as the cross-device collaboration surface for vibecoding.

**Phase**: Rust rewrite in progress (since 2026-05-15). Repository has **not** released any version — first 0.1.0 lands when Rust reaches "usable" (Phase 5 = bot bridge + CC headless can produce Feishu task end-to-end). See `.codestable/brainstorms/v0.x-direction/` for release strategy.

**Active code**: `crates/roostery/` (Rust workspace, single crate currently).

**Reference code** (read-only, not maintained): `legacy/python/` — the prior `feishu_hub` baseline (M3.C → M5.A, ~7339 LOC, 40+ tests). Per `.codestable/attention.md` "code-doc-authority": when Python code and current docs disagree, **docs win**. Phase 7 (`legacy-removal` feature) deletes this directory.

## Where things live

| Question | Look here |
|---|---|
| What hard constraints must I respect? | `.codestable/attention.md` (9 entries, every CodeStable skill loads it) |
| What capabilities does Roostery provide? | `.codestable/requirements/` (3 draft reqs) |
| How is the system structured? | `.codestable/architecture/ARCHITECTURE.md` |
| What's the rewrite plan? | `.codestable/roadmap/rust-rewrite/` (21 features / 8 modules / 7 interface contracts) |
| Why these design choices? | `.codestable/brainstorms/v0.x-direction/` |
| What was the Python doing before? | `legacy/python/src/roostery/` (reference only) |

## Commands

```bash
# Build / run
cargo build                 # debug build
cargo run -- --version      # print "roostery 0.0.0 (rust)"
cargo build --release       # release build → target/release/roostery

# Test
cargo test --all
cargo test --all -- --nocapture <test_name>

# Lint / format (must pass for CI green)
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
```

Rust toolchain pinned via `rust-toolchain.toml` (stable channel + clippy + rustfmt). CI configured in `.github/workflows/ci.yml` runs fmt / clippy / test on ubuntu-latest.

The legacy Python is **not** maintained. If you absolutely need to run it for archaeological purposes: `cd legacy/python && pip install -e .` (no longer guaranteed to work; no support).

## Architecture — the load-bearing mental model

> **Feishu is the shared state machine. `lark-cli` is the agent's syscall surface to Feishu. Roostery is only the execution bridge + local audit cache.**

This invariant is enforced in code review: any module that re-implements what `lark-cli` already does, or treats local state as the source of truth for collaboration data, will be deleted. See `.codestable/architecture/ARCHITECTURE.md` §1-§3 for the full red-line and module map.

### State ownership

| State                                    | Owner                                                                |
| ---------------------------------------- | -------------------------------------------------------------------- |
| Work-item lifecycle, agent step stream   | Feishu Task (`lark-cli task +create` / `append_task_steps`)          |
| Cross-agent live context                 | Feishu IM thread (`lark-cli im +messages-reply --thread`)            |
| Comments / collab traces                 | Feishu Docs comments, group chat                                     |
| Index / stats / dashboard views          | Feishu Base (index layer, **not** source of truth)                   |
| Cloud-side routing (@mention / cron)     | Feishu Base Workflow (`LarkMessageTrigger` / `TimerTrigger`)         |
| Local process / model calls / budget     | Local (planned: `dispatcher::runners`, `dispatcher::budget` — Phase 4) |
| Audit / replay                           | Local journal jsonl in `~/.roostery/journal/` (`journal` module — Phase 1, done) |

### Rust module map (target — Phase 1 onwards)

Active code currently has only `src/main.rs` + `src/lib.rs` (Phase 0 scaffold). Target structure per `.codestable/roadmap/rust-rewrite/`:

- **Module A — Foundations** (Phase 1): `schema` constants, `redact`, `remoterefs` — pure data utilities
- **Module B — Local Audit** (Phase 1): `journal` — jsonl audit / replay; `JournalEntry` schema is the **portable-by-default** req's public contract
- **Module C — Feishu Syscall** (Phase 2): `lark_cli` (LarkRunner trait + subprocess impl), `roostery smoke`, `bin/shim`
- **Module D — Local Config & Install** (Phase 3): `config`, `hooks_merge`, `identity`, `agent_detect`, `onboarding` (`roostery init`)
- **Module E — Dispatcher** (Phase 4): `trace`, `budget`, `rules`, `runners` (with Runner trait), `loop_`, `event_bridge`
- **Module F — Bot Bridge** (Phase 5): `task_writer`, `stop_hook`, `bot_*` cluster — **agent-work-in-feishu** req's direct delivery layer; `bot-stop-hook` feature = "Rust usable" milestone
- **Module G — Reporting** (Phase 6): `git_log`, `llm_summary` (only module allowed `reqwest` for external LLM), `daily_report`, `record_writer`
- **Module H — Base Index** (Phase 7): `base_*` cluster

Cross-module interface contracts (LarkRunner / JournalEntry / Runner / HookEvent / TraceContext / Config / templates) are defined in `.codestable/roadmap/rust-rewrite/rust-rewrite-roadmap.md` §4 and are **hard constraints** for feature-design.

### Hard rules

Loaded automatically into every CodeStable skill via `.codestable/attention.md`. Key ones (full list there):

1. **No `lark-cli` reimplementation.** Feishu API goes through the `lark_cli` wrapper. No direct `reqwest` / HTTP calls to `open.feishu.cn`.
2. **Local state is cache, not truth.** Anything in `~/.roostery/` (Rust era; Python era used `~/.feishu_hub/`) is replayable audit — to answer "what's the status of task X", query Feishu, not local state.
3. **`llm_summary` is the only module allowed to import an external LLM client.** Other modules stay vendor-neutral.
4. **lark-cli pinned at 1.0.28** (timestamp schema compatibility). Higher versions must pass smoke first.
5. **Smoke is the upgrade gate**. Any probe failure → `roostery init` and `daily_report` refuse to run.

## Test conventions (Rust era)

- All Feishu-touching code MUST take a `LarkRunner` trait (Phase 2 onward); production wires `LarkCli` subprocess impl, tests wire `MockLarkRunner`.
- `cargo test --all` runs both unit tests (`#[cfg(test)] mod tests` per module) and integration tests (`crates/roostery/tests/*.rs`).
- E2E tests touching real Feishu are marked `#[ignore]` and run manually only.

## When extending

- **New feature** (any of the 21 planned items): start with `cs-feat-design` reading the roadmap item from `.codestable/roadmap/rust-rewrite/rust-rewrite-items.yaml`. The roadmap §4 contracts are hard constraints — to change a contract, go back to `cs-roadmap update`.
- **Issue / bug** discovered in active code: `cs-issue` workflow. Issues found in `legacy/python/` are not fixed (per "code-doc-authority").
- **New agent runtime adapter** (CC / Codex / Gemini / custom): add a `Runner` trait impl + register in `runner_registry`. Do not put provider-specific logic in the `loop_` module.
- **New Feishu surface**: add a method via `LarkRunner` trait + add a smoke probe + bump no version in lark-cli pin (1.0.28).
