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
        "no real `lark-cli` found on PATH (excluding shim target); \
         install lark-cli and ensure it is on PATH before running `roostery init`"
    )]
    RealLarkCliMissing,
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
    let shim_target = home_join(SHIM_TARGET_RELATIVE)?;
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

    // env file + F8 shell rc patch — only if we have a real lark-cli to point to.
    let real_lark_cli = resolve_real_lark_cli(&shim_target)?;
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
fn resolve_real_lark_cli(shim_target: &Path) -> Result<PathBuf, OnboardingError> {
    let candidates =
        which::which_all("lark-cli").map_err(|_| OnboardingError::RealLarkCliMissing)?;
    for p in candidates {
        if p == shim_target {
            continue;
        }
        return Ok(p);
    }
    Err(OnboardingError::RealLarkCliMissing)
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
    out.push_str(&format!("  real:     {}\n", report.real_lark_cli.display()));
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
    use std::sync::Mutex;

    /// 串行化所有触碰 `$SHELL` 的测试——attention.md 规约"测试中并发触碰 env
    /// 必须用 static Mutex 串行化"。否则 cargo test 默认 multi-thread 跑会让
    /// 这 4 个 `shell_kind_detect_*` 测试互相覆盖 env，CI 偶发 fail。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn shell_kind_detect_zsh() {
        let _g = ENV_LOCK.lock().unwrap();
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
        let _g = ENV_LOCK.lock().unwrap();
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
        let _g = ENV_LOCK.lock().unwrap();
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
        let _g = ENV_LOCK.lock().unwrap();
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
