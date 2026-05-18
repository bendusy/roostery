//! sh bridge 写入 + 跨 agent hooks_merge 应用。
//!
//! 拆自原 `onboarding.rs` line 397-456（refactor `2026-05-19-onboarding-split`）。

use super::types::{InitOptions, SkipReason};
use super::{OnboardingError, create_dir_all, set_executable};
use crate::agent_detect::DetectResult;
use crate::hooks_merge::{self, AgentKind, STOP_HOOK_AGENT_NOTIFY_SH};
use std::fs;
use std::path::Path;

/// F6 write sh bridge. Always overwrites (small embedded const; idempotent
/// via content equality even without sha compare).
pub(super) fn write_sh_bridge(target: &Path) -> Result<(), OnboardingError> {
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
pub(super) fn merge_hooks_for(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onboarding::SH_BRIDGE_FILENAME;

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
}
