//! `roostery init` orchestrator — install shim, merge hooks, patch shell rc.
//!
//! **Note on module naming.** The legacy Python module of the same name
//! (`onboarding.py`) creates welcome tasks via `task_writer`. In the Rust
//! port the welcome-task behavior is deferred to Phase 5 (`bot-stop-hook` /
//! `bot-task-writer` features); this Rust `onboarding` is a pure
//! **installer** for Phase 3. Name kept for searchability across the
//! Python→Rust port; doc-comment makes the scope explicit so the two
//! eras don't collide in `git blame`.
//!
//! 拆分自单文件 `onboarding.rs`（refactor `2026-05-19-onboarding-split`）。业务块按
//! audit `2026-05-18-post-release-rust-idiom` finding-04 的边界切分：
//!
//! - [`types`] — `ShellKind` / `SkipReason` / `InitOptions` / `RealLarkCliSource` / `InitReport`
//! - [`shim`] — `install_shim` + sha256 比对 + roostery-shim 识别
//! - [`hooks`] — sh bridge 写入 + 跨 agent hooks_merge 应用
//! - [`lark_cli_override`] — 真 lark-cli 三层链解析（flag > env > PATH）+ override 校验
//! - [`env_rc`] — 写 `~/.roostery/env` + patch 用户 shell rc
//!
//! 本文件保留：`OnboardingError`（模块中心错误类型）、`run()` 主编排、`format_report`、
//! 跨子模块共享的 file helpers（`create_dir_all` / `home_join` / `set_executable`）+ 公共
//! 常量。
//!
//! See `.codestable/features/2026-05-18-roostery-init/roostery-init-design.md` §2.1.4.

mod env_rc;
mod hooks;
mod lark_cli_override;
mod shim;
pub mod types;

pub use types::{InitOptions, InitReport, RealLarkCliSource, ShellKind, SkipReason};

use crate::agent_detect;
use crate::hooks_merge::{AgentKind, HooksError};
use crate::identity;
use crate::lark_cli::LarkRunner;
use crate::paths;
use crate::smoke::{self, SmokeError};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

// --- Module-wide constants --------------------------------------------------

/// Marker comments wrapping the line added to the user's shell rc. Matches
/// the conda / pyenv pattern: idempotent re-patching grepped via these
/// sentinels.
const RC_MARKER_BEGIN: &str = "# >>> roostery >>>";
const RC_MARKER_END: &str = "# <<< roostery <<<";
const SHIM_TARGET_RELATIVE: &str = ".local/bin/lark-cli";
const SHIM_SOURCE_FILENAME: &str = "shim";
const SH_BRIDGE_FILENAME: &str = "agent_stop_notify.sh";

// --- Module-wide error type ------------------------------------------------

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum OnboardingError {
    #[error("smoke gate failed: {source}")]
    SmokeNotReady {
        #[from]
        source: SmokeError,
    },
    #[error("failed to create directory {path}: {source}")]
    StateDirFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "shim source not found at {path}; install with \
         `cargo install --path crates/roostery --bins` or build via \
         `cargo build --release --bin shim` and place sibling to roostery"
    )]
    ShimSourceMissing { path: PathBuf },
    #[error(
        "shim target {path} exists and is not a roostery shim; \
         back it up and remove first, then re-run `roostery init`"
    )]
    ShimTargetConflict { path: PathBuf },
    #[error("failed to copy shim {from} -> {to}: {source}")]
    ShimCopyFailed {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to write {path}: {source}")]
    WriteFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("hook merge failed for agent {agent}: {source}")]
    HookMergeFailed {
        agent: AgentKind,
        #[source]
        source: HooksError,
    },
    #[error(
        "unsupported or undetected shell (only zsh/bash; detected: {detected:?}); \
         set $SHELL or run from zsh/bash"
    )]
    UnsupportedShell { detected: Option<String> },
    #[error(
        "no `lark-cli` found on PATH; install it (e.g. `npm install -g @larksuite/cli`) \
         or pass `--real-lark-cli <path>` / set `ROOSTERY_LARK_CLI_BIN` env"
    )]
    LarkCliNotInPath,
    #[error(
        "only `lark-cli` candidate on PATH is the shim install target ({shim_target}); \
         pass `--real-lark-cli <path>` or set `ROOSTERY_LARK_CLI_BIN` env to the \
         real binary path (note: `found_at` == `shim_target` = {found_at})"
    )]
    LarkCliCollidesShimTarget {
        found_at: PathBuf,
        shim_target: PathBuf,
    },
    #[error("real lark-cli override at {path} is invalid: {reason}")]
    OverrideInvalid { path: PathBuf, reason: &'static str },
    #[error("failed to resolve current_exe: {source}")]
    CurrentExeFailed {
        #[source]
        source: io::Error,
    },
}

// --- Shared file helpers (visible to sibling submodules) ------------------

pub(super) fn create_dir_all(p: &Path) -> Result<(), OnboardingError> {
    fs::create_dir_all(p).map_err(|source| OnboardingError::StateDirFailed {
        path: p.to_path_buf(),
        source,
    })
}

fn home_join(relative: &str) -> Result<PathBuf, OnboardingError> {
    let home = dirs::home_dir().ok_or_else(|| OnboardingError::StateDirFailed {
        path: PathBuf::from("$HOME"),
        source: io::Error::new(io::ErrorKind::NotFound, "no home dir"),
    })?;
    Ok(home.join(relative))
}

#[cfg(unix)]
pub(super) fn set_executable(p: &Path) -> Result<(), OnboardingError> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(p)
        .map_err(|source| OnboardingError::WriteFailed {
            path: p.to_path_buf(),
            source,
        })?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(p, perms).map_err(|source| OnboardingError::WriteFailed {
        path: p.to_path_buf(),
        source,
    })
}

#[cfg(not(unix))]
pub(super) fn set_executable(_p: &Path) -> Result<(), OnboardingError> {
    Ok(())
}

// --- Main orchestrator ---------------------------------------------------

/// Main orchestrator entry. Linear pipeline; identity failures are
/// non-fatal (warned + continue). See module-level docs.
pub async fn run(
    runner: &dyn LarkRunner,
    opts: InitOptions,
) -> Result<InitReport, OnboardingError> {
    // F1: smoke gate. Failure → no filesystem mutation.
    smoke::ensure_ready()?;

    // **Early-gate resolve**（design §2.2）：F1 之后第一时间解析真 lark-cli。
    // 任一失败路径（无候选 / shim_target 碰撞 / override 无效）都在写文件之前返还
    // → 与 smoke gate 同样"失败零文件副作用"语义。修了原 L205 fail-late 留破损态 bug。
    let shim_target = home_join(SHIM_TARGET_RELATIVE)?;
    let (real_lark_cli, real_lark_cli_source) = lark_cli_override::resolve_real_lark_cli(
        &shim_target,
        opts.real_lark_cli_override.as_deref(),
    )?;

    // F3: identity (non-fatal).
    let (identity_snapshot, identity_error) = match identity::current(runner).await {
        Ok(i) => (Some(i), None),
        Err(e) => (None, Some(e)),
    };

    // F4: agent detection (no I/O cost; safe before mkdir).
    let detections = agent_detect::detect_all(&opts.skip_agents);

    // F2: bootstrap directories.
    if !opts.dry_run {
        for dir in [
            paths::journal_dir(),
            paths::state_dir(),
            paths::scripts_dir(),
        ] {
            create_dir_all(&dir)?;
        }
    }

    // F5: install shim.
    if !opts.dry_run {
        shim::install_shim(&shim_target)?;
    }

    // F6: write sh bridge.
    let sh_path = paths::scripts_dir().join(SH_BRIDGE_FILENAME);
    if !opts.dry_run {
        hooks::write_sh_bridge(&sh_path)?;
    }

    // F7: merge hooks per installed agent (single-agent failure isolated).
    let (installed, skipped) = hooks::merge_hooks_for(&detections, &sh_path, &opts);

    // F8 env file + shell rc patch — real_lark_cli 已在 early gate 解出。
    if !opts.dry_run {
        env_rc::write_env_file(&paths::env_file(), &real_lark_cli)?;
    }

    let shell_rc_patched = match ShellKind::detect_from_env() {
        Ok(shell) => {
            if let Some(rc) = shell.rc_path() {
                if !opts.dry_run {
                    env_rc::patch_shell_rc(&rc, &paths::env_file())?;
                }
                Some(rc)
            } else {
                None
            }
        }
        Err(e) => return Err(e),
    };

    Ok(InitReport {
        identity: identity_snapshot,
        identity_error,
        agents_installed: installed,
        agents_skipped: skipped,
        shim_path: shim_target,
        shell_rc_patched,
        real_lark_cli,
        real_lark_cli_source,
        dry_run: opts.dry_run,
    })
}

// --- Public format helper ------------------------------------------------

/// Format an [`InitReport`] for human-readable terminal output.
pub fn format_report(report: &InitReport) -> String {
    let mut out = String::new();
    out.push_str("\n🪺 roostery init — report\n\n");
    if report.dry_run {
        out.push_str("  ⚠ DRY RUN — no files were modified.\n\n");
    }
    match (&report.identity, &report.identity_error) {
        (Some(i), _) => {
            out.push_str(&format!("  identity: {}\n", i.describe()));
        }
        (None, Some(e)) => {
            out.push_str(&format!(
                "  identity: (unavailable — {e}; continuing without it)\n"
            ));
        }
        (None, None) => {}
    }
    out.push_str(&format!("  shim:     {}\n", report.shim_path.display()));
    out.push_str(&format!(
        "  real:     {} (from {})\n",
        report.real_lark_cli.display(),
        report.real_lark_cli_source,
    ));
    if let Some(rc) = &report.shell_rc_patched {
        out.push_str(&format!("  rc:       {}\n", rc.display()));
    }
    out.push_str("  agents installed:\n");
    if report.agents_installed.is_empty() {
        out.push_str("    (none)\n");
    } else {
        for k in &report.agents_installed {
            out.push_str(&format!("    ✓ {k}\n"));
        }
    }
    if !report.agents_skipped.is_empty() {
        out.push_str("  agents skipped:\n");
        for (k, r) in &report.agents_skipped {
            let reason = match r {
                SkipReason::NotInstalled => "not installed".to_string(),
                SkipReason::UserSkipped => "skipped by --skip-agent".to_string(),
                SkipReason::MergeFailed(msg) => format!("merge failed: {msg}"),
            };
            out.push_str(&format!("    — {k}: {reason}\n"));
        }
    }
    out.push_str("\n  next: open a new shell or `source ~/.roostery/env`,\n");
    out.push_str("        then run an agent (claude / codex / gemini) and watch\n");
    out.push_str("        the SessionEnd hook fire into the dispatcher path.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboarding_error_three_sub_variants_carry_fix_hint() {
        let e1 = OnboardingError::LarkCliNotInPath;
        let s1 = e1.to_string();
        assert!(
            s1.contains("--real-lark-cli") && s1.contains("ROOSTERY_LARK_CLI_BIN"),
            "LarkCliNotInPath should hint both flag + env: {s1}"
        );

        let e2 = OnboardingError::LarkCliCollidesShimTarget {
            found_at: PathBuf::from("/home/u/.local/bin/lark-cli"),
            shim_target: PathBuf::from("/home/u/.local/bin/lark-cli"),
        };
        let s2 = e2.to_string();
        assert!(
            s2.contains("--real-lark-cli") && s2.contains("ROOSTERY_LARK_CLI_BIN"),
            "LarkCliCollidesShimTarget should hint flag + env: {s2}"
        );
        assert!(s2.contains("/home/u/.local/bin/lark-cli"));

        let e3 = OnboardingError::OverrideInvalid {
            path: PathBuf::from("/not/exists"),
            reason: "missing",
        };
        let s3 = e3.to_string();
        assert!(s3.contains("/not/exists") && s3.contains("missing"));
    }

    #[test]
    fn format_report_shows_real_lark_cli_source() {
        let report = InitReport {
            identity: None,
            identity_error: None,
            agents_installed: vec![],
            agents_skipped: vec![],
            shim_path: PathBuf::from("/home/u/.local/bin/lark-cli"),
            shell_rc_patched: None,
            real_lark_cli: PathBuf::from("/opt/feishu/lark-cli"),
            real_lark_cli_source: RealLarkCliSource::Flag,
            dry_run: false,
        };
        let s = format_report(&report);
        assert!(
            s.contains("real:     /opt/feishu/lark-cli (from flag)"),
            "real line should include path + source: {s}"
        );
    }
}
