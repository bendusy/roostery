//! 写 `~/.roostery/env` 文件 + patch 用户 shell rc（marker-wrapped 幂等块）。
//!
//! 拆自原 `onboarding.rs` line 526-578（refactor `2026-05-19-onboarding-split`）。

use super::{OnboardingError, RC_MARKER_BEGIN, RC_MARKER_END, create_dir_all};
use std::fs;
use std::io;
use std::path::Path;

pub(super) fn write_env_file(path: &Path, real_lark_cli: &Path) -> Result<(), OnboardingError> {
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

pub(super) fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// F8 shell rc patch. Marker-wrapped idempotent block (conda/pyenv pattern).
pub(super) fn patch_shell_rc(rc_path: &Path, env_path: &Path) -> Result<(), OnboardingError> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
