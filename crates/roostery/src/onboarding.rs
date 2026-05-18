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
//! See `.codestable/features/2026-05-18-roostery-init/roostery-init-design.md`
//! §2.1.4.

use crate::agent_detect::{self, DetectResult};
use crate::hooks_merge::{self, AgentKind, HooksError, STOP_HOOK_AGENT_NOTIFY_SH};
use crate::identity::{self, Identity, IdentityError};
use crate::lark_cli::LarkRunner;
use crate::paths;
use crate::smoke::{self, SmokeError};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Marker comments wrapping the line added to the user's shell rc. Matches
/// the conda / pyenv pattern: idempotent re-patching grepped via these
/// sentinels.
const RC_MARKER_BEGIN: &str = "# >>> roostery >>>";
const RC_MARKER_END: &str = "# <<< roostery <<<";
const SHIM_TARGET_RELATIVE: &str = ".local/bin/lark-cli";
const SHIM_SOURCE_FILENAME: &str = "shim";
const SH_BRIDGE_FILENAME: &str = "agent_stop_notify.sh";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShellKind {
    Zsh,
    Bash,
}

impl ShellKind {
    pub fn rc_path(self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        Some(match self {
            ShellKind::Zsh => home.join(".zshrc"),
            ShellKind::Bash => home.join(".bashrc"),
        })
    }

    /// Detect from `$SHELL`. Returns `Err(UnsupportedShell { detected })` if
    /// not zsh/bash; the `detected` field is the raw `$SHELL` value (or
    /// `None` if unset) for diagnostic clarity.
    pub fn detect_from_env() -> Result<Self, OnboardingError> {
        let raw = std::env::var("SHELL").ok();
        match raw.as_deref() {
            Some(s) if s.ends_with("/zsh") || s == "zsh" => Ok(ShellKind::Zsh),
            Some(s) if s.ends_with("/bash") || s == "bash" => Ok(ShellKind::Bash),
            _ => Err(OnboardingError::UnsupportedShell { detected: raw }),
        }
    }
}

/// Per-agent skip reason for [`InitReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SkipReason {
    NotInstalled,
    UserSkipped,
    MergeFailed(String),
}

#[derive(Debug, Clone, Default)]
pub struct InitOptions {
    pub dry_run: bool,
    pub skip_agents: Vec<AgentKind>,
    /// 显式指定真 lark-cli 路径，跳过 PATH 搜索；优先级最高。
    /// `None` → 读 `ROOSTERY_LARK_CLI_BIN` env → PATH 搜索。
    pub real_lark_cli_override: Option<PathBuf>,
}

/// `InitReport.real_lark_cli` 的来源——让 `format_report` 显式输出走的哪条 path。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RealLarkCliSource {
    /// 来自 `InitOptions.real_lark_cli_override`（一般经 `--real-lark-cli` flag）
    Flag,
    /// 来自 `ROOSTERY_LARK_CLI_BIN` env
    Env,
    /// 来自 PATH 搜索（`which::which_all` 排 shim_target 后第一个候选）
    PathDetected,
}

impl std::fmt::Display for RealLarkCliSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RealLarkCliSource::Flag => "flag",
            RealLarkCliSource::Env => "env",
            RealLarkCliSource::PathDetected => "path",
        })
    }
}

#[derive(Debug)]
pub struct InitReport {
    pub identity: Option<Identity>,
    pub identity_error: Option<IdentityError>,
    pub agents_installed: Vec<AgentKind>,
    pub agents_skipped: Vec<(AgentKind, SkipReason)>,
    pub shim_path: PathBuf,
    pub shell_rc_patched: Option<PathBuf>,
    pub real_lark_cli: PathBuf,
    pub real_lark_cli_source: RealLarkCliSource,
    pub dry_run: bool,
}

impl InitReport {
    pub fn had_errors(&self) -> bool {
        self.agents_skipped
            .iter()
            .any(|(_, r)| matches!(r, SkipReason::MergeFailed(_)))
    }
}

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
    let (real_lark_cli, real_lark_cli_source) =
        resolve_real_lark_cli(&shim_target, opts.real_lark_cli_override.as_deref())?;

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
        install_shim(&shim_target)?;
    }

    // F6: write sh bridge.
    let sh_path = paths::scripts_dir().join(SH_BRIDGE_FILENAME);
    if !opts.dry_run {
        write_sh_bridge(&sh_path)?;
    }

    // F7: merge hooks per installed agent (single-agent failure isolated).
    let (installed, skipped) = merge_hooks_for(&detections, &sh_path, &opts);

    // F8 env file + shell rc patch — real_lark_cli 已在 early gate 解出。
    if !opts.dry_run {
        write_env_file(&paths::env_file(), &real_lark_cli)?;
    }

    let shell_rc_patched = match ShellKind::detect_from_env() {
        Ok(shell) => {
            if let Some(rc) = shell.rc_path() {
                if !opts.dry_run {
                    patch_shell_rc(&rc, &paths::env_file())?;
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

// --- Private helpers -------------------------------------------------------

fn create_dir_all(p: &Path) -> Result<(), OnboardingError> {
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

/// SHA-256 of file bytes; small wrapper for shim hash comparison (idiom #3
/// newtype-lite — a freestanding `BinaryHash` newtype is overkill for a
/// single comparison site).
fn file_sha256(path: &Path) -> io::Result<[u8; 32]> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hasher.finalize().into())
}

/// Magic bytes identifying a roostery shim binary: we look for the literal
/// env var name embedded in the binary (`shim.rs` reads it via `env!`).
/// Cheap heuristic — avoids parsing Mach-O / ELF.
const SHIM_MAGIC: &[u8] = b"ROOSTERY_REAL_LARK_CLI";

fn looks_like_roostery_shim(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    memmem(&bytes, SHIM_MAGIC)
}

fn memmem(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// F5 install: copy `current_exe sibling` shim → `~/.local/bin/lark-cli`.
/// Idempotent via sha256 compare; refuses to overwrite non-shim contents.
fn install_shim(target: &Path) -> Result<(), OnboardingError> {
    let exe =
        std::env::current_exe().map_err(|source| OnboardingError::CurrentExeFailed { source })?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| OnboardingError::ShimSourceMissing {
            path: PathBuf::from(SHIM_SOURCE_FILENAME),
        })?;
    let source = exe_dir.join(SHIM_SOURCE_FILENAME);
    if !source.exists() {
        return Err(OnboardingError::ShimSourceMissing { path: source });
    }

    if let Some(parent) = target.parent() {
        create_dir_all(parent)?;
    }

    if target.exists() {
        if !looks_like_roostery_shim(target) {
            return Err(OnboardingError::ShimTargetConflict {
                path: target.to_path_buf(),
            });
        }
        let src_hash =
            file_sha256(&source).map_err(|source_err| OnboardingError::ShimCopyFailed {
                from: source.clone(),
                to: target.to_path_buf(),
                source: source_err,
            })?;
        let tgt_hash =
            file_sha256(target).map_err(|source_err| OnboardingError::ShimCopyFailed {
                from: source.clone(),
                to: target.to_path_buf(),
                source: source_err,
            })?;
        if src_hash == tgt_hash {
            return Ok(()); // idempotent skip
        }
    }

    fs::copy(&source, target).map_err(|source_err| OnboardingError::ShimCopyFailed {
        from: source.clone(),
        to: target.to_path_buf(),
        source: source_err,
    })?;
    set_executable(target)?;
    Ok(())
}

#[cfg(unix)]
fn set_executable(p: &Path) -> Result<(), OnboardingError> {
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
fn set_executable(_p: &Path) -> Result<(), OnboardingError> {
    Ok(())
}

/// F6 write sh bridge. Always overwrites (small embedded const; idempotent
/// via content equality even without sha compare).
fn write_sh_bridge(target: &Path) -> Result<(), OnboardingError> {
    if let Some(parent) = target.parent() {
        create_dir_all(parent)?;
    }
    fs::write(target, STOP_HOOK_AGENT_NOTIFY_SH).map_err(|source| {
        OnboardingError::WriteFailed {
            path: target.to_path_buf(),
            source,
        }
    })?;
    set_executable(target)?;
    Ok(())
}

/// F7 hook merge across installed agents. Per-agent failure is recorded
/// in skipped list with `SkipReason::MergeFailed` — does not abort the loop.
fn merge_hooks_for(
    detections: &[DetectResult],
    sh_path: &Path,
    opts: &InitOptions,
) -> (Vec<AgentKind>, Vec<(AgentKind, SkipReason)>) {
    let mut installed = Vec::new();
    let mut skipped = Vec::new();
    for det in detections {
        let kind = det.spec.kind;
        if opts.skip_agents.contains(&kind) {
            skipped.push((kind, SkipReason::UserSkipped));
            continue;
        }
        if !det.installed() {
            skipped.push((kind, SkipReason::NotInstalled));
            continue;
        }
        if opts.dry_run {
            installed.push(kind);
            continue;
        }
        let sh_str = match sh_path.to_str() {
            Some(s) => s,
            None => {
                skipped.push((
                    kind,
                    SkipReason::MergeFailed("sh path is not valid UTF-8".to_string()),
                ));
                continue;
            }
        };
        match hooks_merge::apply_template(
            kind.template(),
            &det.spec.expanded_hooks_target(),
            sh_str,
        ) {
            Ok(_) => installed.push(kind),
            Err(e) => skipped.push((kind, SkipReason::MergeFailed(e.to_string()))),
        }
    }
    (installed, skipped)
}

/// Resolve the real `lark-cli` binary to point the shim at, excluding the
/// shim itself (target file).
/// 三层链解析真 lark-cli 路径：override (flag) > env `ROOSTERY_LARK_CLI_BIN` >
/// PATH 搜索（`which::which_all` 排 shim_target）。
///
/// 返还 `(path, source)` 让 caller 写 `InitReport.real_lark_cli_source`，
/// `format_report` 输出"from {source}"让用户知道走的哪条路径。
///
/// 失败分 3 子情形：
/// - PATH 上 0 候选且无 override → `LarkCliNotInPath`
/// - PATH 上唯一候选 = shim_target（npm prefix 撞 shim target 经典场景）→
///   `LarkCliCollidesShimTarget { found_at, shim_target }`
/// - override 路径不存在或是目录 → `OverrideInvalid { path, reason }`
fn resolve_real_lark_cli(
    shim_target: &Path,
    override_path: Option<&Path>,
) -> Result<(PathBuf, RealLarkCliSource), OnboardingError> {
    // 1. flag override
    if let Some(p) = override_path {
        validate_override(p)?;
        return Ok((p.to_path_buf(), RealLarkCliSource::Flag));
    }
    // 2. env override（ROOSTERY_LARK_CLI_BIN 与 runtime LarkCli subprocess 复用同一 env，
    //    design §1.2 D1。空字符串视为未设；走下一层。）
    if let Ok(s) = std::env::var("ROOSTERY_LARK_CLI_BIN")
        && !s.is_empty()
    {
        let p = PathBuf::from(s);
        validate_override(&p)?;
        return Ok((p, RealLarkCliSource::Env));
    }
    // 3. PATH search via which::which_all，排 shim_target
    let candidates: Vec<PathBuf> = match which::which_all("lark-cli") {
        Ok(iter) => iter.collect(),
        Err(_) => return Err(OnboardingError::LarkCliNotInPath),
    };
    if candidates.is_empty() {
        return Err(OnboardingError::LarkCliNotInPath);
    }
    for p in &candidates {
        if p != shim_target {
            return Ok((p.clone(), RealLarkCliSource::PathDetected));
        }
    }
    // 所有候选 == shim_target → npm prefix 撞 shim target 经典场景
    Err(OnboardingError::LarkCliCollidesShimTarget {
        found_at: candidates[0].clone(),
        shim_target: shim_target.to_path_buf(),
    })
}

/// 校验 override 路径：存在 + 不是目录。不查 unix execute bit（design §5 U1）。
fn validate_override(path: &Path) -> Result<(), OnboardingError> {
    if !path.exists() {
        return Err(OnboardingError::OverrideInvalid {
            path: path.to_path_buf(),
            reason: "missing",
        });
    }
    if path.is_dir() {
        return Err(OnboardingError::OverrideInvalid {
            path: path.to_path_buf(),
            reason: "is a directory",
        });
    }
    Ok(())
}

fn write_env_file(path: &Path, real_lark_cli: &Path) -> Result<(), OnboardingError> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    let body = format!(
        "# Written by `roostery init`; safe to re-source.\nexport ROOSTERY_REAL_LARK_CLI={}\n",
        shell_quote(&real_lark_cli.to_string_lossy()),
    );
    fs::write(path, body).map_err(|source| OnboardingError::WriteFailed {
        path: path.to_path_buf(),
        source,
    })
}

fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// F8 shell rc patch. Marker-wrapped idempotent block (conda/pyenv pattern).
fn patch_shell_rc(rc_path: &Path, env_path: &Path) -> Result<(), OnboardingError> {
    let existing = match fs::read_to_string(rc_path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(OnboardingError::WriteFailed {
                path: rc_path.to_path_buf(),
                source,
            });
        }
    };
    if existing.contains(RC_MARKER_BEGIN) {
        return Ok(()); // idempotent skip
    }
    let env_str = env_path.to_string_lossy();
    let block = format!(
        "\n{begin}\n[ -f {env} ] && source {env}\n{end}\n",
        begin = RC_MARKER_BEGIN,
        end = RC_MARKER_END,
        env = shell_quote(&env_str),
    );
    let mut next = existing;
    next.push_str(&block);
    fs::write(rc_path, next).map_err(|source| OnboardingError::WriteFailed {
        path: rc_path.to_path_buf(),
        source,
    })
}

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
    use crate::paths::TEST_ENV_LOCK as ENV_LOCK;

    // ----- S1 type-skeleton trivial tests --------------------------------

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
    fn real_lark_cli_source_display_is_lowercase() {
        assert_eq!(RealLarkCliSource::Flag.to_string(), "flag");
        assert_eq!(RealLarkCliSource::Env.to_string(), "env");
        assert_eq!(RealLarkCliSource::PathDetected.to_string(), "path");
    }

    // ----- S2 validate_override tests -----------------------------------

    #[test]
    fn validate_override_accepts_existing_regular_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        assert!(validate_override(tmp.path()).is_ok());
    }

    #[test]
    fn validate_override_missing_path_returns_missing() {
        let err =
            validate_override(Path::new("/definitely/not/here/lark-cli")).expect_err("missing");
        match err {
            OnboardingError::OverrideInvalid { reason, .. } => assert_eq!(reason, "missing"),
            other => panic!("expected OverrideInvalid::missing, got {other:?}"),
        }
    }

    #[test]
    fn validate_override_directory_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let err = validate_override(dir.path()).expect_err("dir");
        match err {
            OnboardingError::OverrideInvalid { reason, .. } => {
                assert_eq!(reason, "is a directory")
            }
            other => panic!("expected OverrideInvalid::is a directory, got {other:?}"),
        }
    }

    #[test]
    fn validate_override_relative_path_relative_to_cwd() {
        // 写一个 cwd 内的临时文件名，验 path.exists() 走相对 cwd
        let cwd = std::env::current_dir().unwrap();
        let tmp = tempfile::NamedTempFile::new_in(&cwd).unwrap();
        let rel = tmp.path().file_name().unwrap();
        let rel_path = Path::new(rel);
        assert!(
            validate_override(rel_path).is_ok(),
            "relative path should validate"
        );
    }

    // ----- S3 resolve_real_lark_cli three-tier chain tests ---------------

    /// 制造一个临时目录 + 内含可执行 "lark-cli" 脚本，返还 (tempdir, path)。
    /// caller 保留 tempdir 让其活到测试结束。
    fn make_fake_lark_cli(dir: &tempfile::TempDir, name: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mut perm = std::fs::metadata(&path).unwrap().permissions();
            perm.set_mode(0o755);
            std::fs::set_permissions(&path, perm).unwrap();
        }
        path
    }

    #[test]
    fn resolve_flag_only_short_circuits() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        unsafe { std::env::remove_var("ROOSTERY_LARK_CLI_BIN") };
        let dir = tempfile::tempdir().unwrap();
        let fake = make_fake_lark_cli(&dir, "lark-cli");
        let shim_target = Path::new("/tmp/nowhere/lark-cli");
        let (path, source) = resolve_real_lark_cli(shim_target, Some(&fake)).unwrap();
        assert_eq!(path, fake);
        assert_eq!(source, RealLarkCliSource::Flag);
    }

    #[test]
    fn resolve_env_only() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let fake = make_fake_lark_cli(&dir, "lark-cli");
        unsafe { std::env::set_var("ROOSTERY_LARK_CLI_BIN", &fake) };
        let shim_target = Path::new("/tmp/nowhere/lark-cli");
        let (path, source) = resolve_real_lark_cli(shim_target, None).unwrap();
        assert_eq!(path, fake);
        assert_eq!(source, RealLarkCliSource::Env);
        unsafe { std::env::remove_var("ROOSTERY_LARK_CLI_BIN") };
    }

    #[test]
    fn resolve_flag_wins_over_env() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let fake_env = make_fake_lark_cli(&dir, "lark-cli-env");
        let fake_flag = make_fake_lark_cli(&dir, "lark-cli-flag");
        unsafe { std::env::set_var("ROOSTERY_LARK_CLI_BIN", &fake_env) };
        let shim_target = Path::new("/tmp/nowhere/lark-cli");
        let (path, source) = resolve_real_lark_cli(shim_target, Some(&fake_flag)).unwrap();
        assert_eq!(path, fake_flag, "flag wins");
        assert_eq!(source, RealLarkCliSource::Flag);
        unsafe { std::env::remove_var("ROOSTERY_LARK_CLI_BIN") };
    }

    #[test]
    fn resolve_path_detected_non_shim_candidate() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        unsafe { std::env::remove_var("ROOSTERY_LARK_CLI_BIN") };
        let dir = tempfile::tempdir().unwrap();
        let fake = make_fake_lark_cli(&dir, "lark-cli");
        // **完全替换** PATH 为孤立 tempdir，避免被其他系统位置的 lark-cli 干扰
        let prev_path = std::env::var("PATH").unwrap_or_default();
        unsafe { std::env::set_var("PATH", dir.path()) };
        let shim_target = Path::new("/tmp/nowhere/lark-cli");
        let (path, source) = resolve_real_lark_cli(shim_target, None).unwrap();
        assert_eq!(path.file_name(), fake.file_name());
        assert_eq!(source, RealLarkCliSource::PathDetected);
        unsafe { std::env::set_var("PATH", prev_path) };
    }

    #[test]
    fn resolve_zero_candidates_returns_not_in_path() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        unsafe { std::env::remove_var("ROOSTERY_LARK_CLI_BIN") };
        let prev_path = std::env::var("PATH").unwrap_or_default();
        // PATH 设为只含空目录
        let empty = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("PATH", empty.path()) };
        let shim_target = Path::new("/tmp/nowhere/lark-cli");
        let err = resolve_real_lark_cli(shim_target, None).expect_err("0 candidates");
        assert!(matches!(err, OnboardingError::LarkCliNotInPath));
        unsafe { std::env::set_var("PATH", prev_path) };
    }

    #[test]
    fn resolve_collision_returns_shim_target_variant() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        unsafe { std::env::remove_var("ROOSTERY_LARK_CLI_BIN") };
        let dir = tempfile::tempdir().unwrap();
        let fake = make_fake_lark_cli(&dir, "lark-cli");
        // **完全替换** PATH 为孤立 tempdir，让 which::which_all 只返还 fake 一个候选
        let prev_path = std::env::var("PATH").unwrap_or_default();
        unsafe { std::env::set_var("PATH", dir.path()) };
        // shim_target 就是 fake 自己——所有候选都 == shim_target → collision
        let shim_target = &fake;
        let err = resolve_real_lark_cli(shim_target, None).expect_err("collision");
        match err {
            OnboardingError::LarkCliCollidesShimTarget {
                found_at,
                shim_target: st,
            } => {
                assert_eq!(found_at, *shim_target);
                assert_eq!(st, *shim_target);
            }
            other => panic!("expected CollidesShimTarget, got {other:?}"),
        }
        unsafe { std::env::set_var("PATH", prev_path) };
    }

    #[test]
    fn resolve_flag_invalid_path_propagates_override_invalid() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        unsafe { std::env::remove_var("ROOSTERY_LARK_CLI_BIN") };
        let bad = Path::new("/definitely/missing/path/lark-cli");
        let shim_target = Path::new("/tmp/nowhere/lark-cli");
        let err = resolve_real_lark_cli(shim_target, Some(bad)).expect_err("invalid");
        match err {
            OnboardingError::OverrideInvalid { path, reason } => {
                assert_eq!(path, bad);
                assert_eq!(reason, "missing");
            }
            other => panic!("expected OverrideInvalid, got {other:?}"),
        }
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

    #[test]
    fn init_options_default_real_lark_cli_override_is_none() {
        let opts = InitOptions::default();
        assert!(opts.real_lark_cli_override.is_none());
        assert!(!opts.dry_run);
        assert!(opts.skip_agents.is_empty());
    }

    #[test]
    fn shell_kind_detect_zsh() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var("SHELL").ok();
        unsafe { std::env::set_var("SHELL", "/bin/zsh") };
        assert_eq!(ShellKind::detect_from_env().unwrap(), ShellKind::Zsh);
        unsafe {
            match prev {
                Some(v) => std::env::set_var("SHELL", v),
                None => std::env::remove_var("SHELL"),
            }
        }
    }

    #[test]
    fn shell_kind_detect_bash() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var("SHELL").ok();
        unsafe { std::env::set_var("SHELL", "/usr/local/bin/bash") };
        assert_eq!(ShellKind::detect_from_env().unwrap(), ShellKind::Bash);
        unsafe {
            match prev {
                Some(v) => std::env::set_var("SHELL", v),
                None => std::env::remove_var("SHELL"),
            }
        }
    }

    #[test]
    fn shell_kind_detect_fish_errors() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var("SHELL").ok();
        unsafe { std::env::set_var("SHELL", "/usr/bin/fish") };
        match ShellKind::detect_from_env() {
            Err(OnboardingError::UnsupportedShell { detected }) => {
                assert_eq!(detected.as_deref(), Some("/usr/bin/fish"));
            }
            other => panic!("expected UnsupportedShell, got {other:?}"),
        }
        unsafe {
            match prev {
                Some(v) => std::env::set_var("SHELL", v),
                None => std::env::remove_var("SHELL"),
            }
        }
    }

    #[test]
    fn shell_kind_detect_unset_errors() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var("SHELL").ok();
        unsafe { std::env::remove_var("SHELL") };
        match ShellKind::detect_from_env() {
            Err(OnboardingError::UnsupportedShell { detected: None }) => {}
            other => panic!("expected UnsupportedShell(None), got {other:?}"),
        }
        unsafe {
            if let Some(v) = prev {
                std::env::set_var("SHELL", v);
            }
        }
    }

    #[test]
    fn memmem_finds_needle() {
        assert!(memmem(b"hello ROOSTERY_REAL_LARK_CLI world", SHIM_MAGIC));
        assert!(!memmem(b"hello world", SHIM_MAGIC));
        assert!(!memmem(b"", SHIM_MAGIC));
    }

    #[test]
    fn shell_quote_passthrough_for_safe_chars() {
        assert_eq!(
            shell_quote("/usr/local/bin/lark-cli"),
            "/usr/local/bin/lark-cli"
        );
        assert_eq!(shell_quote("abc_def-123.bin"), "abc_def-123.bin");
    }

    #[test]
    fn shell_quote_escapes_special_chars() {
        assert_eq!(shell_quote("/path with space"), "'/path with space'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn patch_shell_rc_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let rc = tmp.path().join(".zshrc");
        let env = tmp.path().join("env");
        fs::write(&rc, "# user content\n").unwrap();
        fs::write(&env, "export ROOSTERY_REAL_LARK_CLI=/usr/bin/lark-cli\n").unwrap();

        patch_shell_rc(&rc, &env).unwrap();
        let first = fs::read_to_string(&rc).unwrap();
        assert!(first.contains(RC_MARKER_BEGIN));
        assert!(first.contains(RC_MARKER_END));
        assert!(first.contains("source"));

        patch_shell_rc(&rc, &env).unwrap();
        let second = fs::read_to_string(&rc).unwrap();
        assert_eq!(
            first, second,
            "second patch must be byte-for-byte identical"
        );
    }

    #[test]
    fn patch_shell_rc_creates_file_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let rc = tmp.path().join(".zshrc");
        let env = tmp.path().join("env");
        patch_shell_rc(&rc, &env).unwrap();
        let content = fs::read_to_string(&rc).unwrap();
        assert!(content.contains(RC_MARKER_BEGIN));
    }

    #[test]
    fn patch_shell_rc_preserves_existing_content() {
        let tmp = tempfile::tempdir().unwrap();
        let rc = tmp.path().join(".zshrc");
        let env = tmp.path().join("env");
        let prelude = "# my custom prompt\nexport PS1='$ '\n";
        fs::write(&rc, prelude).unwrap();
        patch_shell_rc(&rc, &env).unwrap();
        let content = fs::read_to_string(&rc).unwrap();
        assert!(content.starts_with(prelude));
    }

    #[test]
    fn write_env_file_has_export_line() {
        let tmp = tempfile::tempdir().unwrap();
        let env = tmp.path().join("env");
        write_env_file(&env, Path::new("/usr/local/bin/lark-cli")).unwrap();
        let body = fs::read_to_string(&env).unwrap();
        assert!(body.contains("export ROOSTERY_REAL_LARK_CLI=/usr/local/bin/lark-cli"));
    }

    #[test]
    fn write_sh_bridge_chmods_0755() {
        let tmp = tempfile::tempdir().unwrap();
        let sh = tmp.path().join(SH_BRIDGE_FILENAME);
        write_sh_bridge(&sh).unwrap();
        assert!(sh.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&sh).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }

    #[test]
    fn install_shim_idempotent_on_same_content() {
        let tmp = tempfile::tempdir().unwrap();
        // Fake "current_exe sibling" by writing a shim binary with magic.
        let exe_dir = tmp.path().join("exedir");
        fs::create_dir(&exe_dir).unwrap();
        let shim_src = exe_dir.join(SHIM_SOURCE_FILENAME);
        let content = b"FAKE\0ROOSTERY_REAL_LARK_CLI\0body\n";
        fs::write(&shim_src, content).unwrap();
        let target = tmp.path().join("target_bin");
        // First copy.
        fs::copy(&shim_src, &target).unwrap();
        // looks_like_roostery_shim must recognize it.
        assert!(looks_like_roostery_shim(&target));
        // Hash equality:
        let h1 = file_sha256(&shim_src).unwrap();
        let h2 = file_sha256(&target).unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn install_shim_refuses_non_shim_target() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("user_script");
        fs::write(&target, b"#!/bin/sh\necho user wrote this\n").unwrap();
        assert!(!looks_like_roostery_shim(&target));
    }

    #[test]
    fn init_options_default() {
        let o = InitOptions::default();
        assert!(!o.dry_run);
        assert!(o.skip_agents.is_empty());
    }

    #[test]
    fn skip_reason_variants_distinguishable() {
        let a = SkipReason::NotInstalled;
        let b = SkipReason::UserSkipped;
        let c = SkipReason::MergeFailed("x".to_string());
        assert_ne!(a, b);
        assert_ne!(b, c);
    }
}
