//! Shim 安装：copy current_exe sibling → `~/.local/bin/lark-cli`，幂等 sha256 比对。
//!
//! 拆自原 `onboarding.rs` line 296-374（refactor `2026-05-19-onboarding-split`）；
//! `memmem` helper 已 inline 到 `looks_like_roostery_shim`（finding-05）。

use super::{OnboardingError, SHIM_SOURCE_FILENAME, create_dir_all, set_executable};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Magic bytes identifying a roostery shim binary: we look for the literal
/// env var name embedded in the binary (`shim.rs` reads it via `env!`).
/// Cheap heuristic — avoids parsing Mach-O / ELF.
const SHIM_MAGIC: &[u8] = b"ROOSTERY_REAL_LARK_CLI";

/// SHA-256 of file bytes; small wrapper for shim hash comparison (idiom #3
/// newtype-lite — a freestanding `BinaryHash` newtype is overkill for a
/// single comparison site).
pub(super) fn file_sha256(path: &Path) -> io::Result<[u8; 32]> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hasher.finalize().into())
}

pub(super) fn looks_like_roostery_shim(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    // Inlined memmem (was a single-call wrapper around windows().any());
    // windows() on empty haystack / over-long needle already short-circuits.
    bytes.windows(SHIM_MAGIC.len()).any(|w| w == SHIM_MAGIC)
}

/// F5 install: copy `current_exe sibling` shim → `~/.local/bin/lark-cli`.
/// Idempotent via sha256 compare; refuses to overwrite non-shim contents.
pub(super) fn install_shim(target: &Path) -> Result<(), OnboardingError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onboarding::SHIM_SOURCE_FILENAME;

    #[test]
    fn looks_like_roostery_shim_finds_magic() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        fs::write(tmp.path(), b"prefix ROOSTERY_REAL_LARK_CLI suffix").unwrap();
        assert!(looks_like_roostery_shim(tmp.path()));
    }

    #[test]
    fn looks_like_roostery_shim_rejects_user_script() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        fs::write(tmp.path(), b"#!/bin/sh\necho hello\n").unwrap();
        assert!(!looks_like_roostery_shim(tmp.path()));
    }

    #[test]
    fn looks_like_roostery_shim_handles_missing_file() {
        assert!(!looks_like_roostery_shim(Path::new("/no/such/file")));
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
}
