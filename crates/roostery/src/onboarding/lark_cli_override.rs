//! `lark-cli` 真二进制路径三层链解析（flag > env > PATH）+ override 校验。
//!
//! 拆自原 `onboarding.rs` line 458-524（refactor `2026-05-19-onboarding-split`）。

use super::OnboardingError;
use super::types::RealLarkCliSource;
use std::path::{Path, PathBuf};

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
pub(super) fn resolve_real_lark_cli(
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
pub(super) fn validate_override(path: &Path) -> Result<(), OnboardingError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::TEST_ENV_LOCK as ENV_LOCK;

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
        let prev_path = std::env::var("PATH").unwrap_or_default();
        unsafe { std::env::set_var("PATH", dir.path()) };
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
}
