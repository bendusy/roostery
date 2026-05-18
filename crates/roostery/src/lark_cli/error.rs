//! `LarkError` rich enum + `retriable()` method. See module-level docs.

use std::path::PathBuf;
use thiserror::Error;

/// Maximum length for `stdout`/`stderr`/`program_args` fields embedded in a
/// `LarkError` — prevents a misbehaving binary from making the error blow up
/// in panic chains or journal entries.
pub const MAX_FIELD_LEN_IN_ERR: usize = 4096;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LarkError {
    /// subprocess 启动失败（binary not found / permission denied / fork 失败）。
    /// `program_args` 是 owned 拷贝（已截断 ≤ 4 KiB）帮 debug。
    #[error("failed to spawn lark-cli at {path:?}: {source}")]
    Spawn {
        path: PathBuf,
        program_args: Vec<String>,
        #[source]
        source: std::io::Error,
    },

    /// lark-cli 退出码非 0；`body_code` 是从 stdout JSON 解出的飞书业务码
    /// （如 99991663 token expire）。`message` 是 summary（给 Display 用）；
    /// `stdout`/`stderr` 是 raw 数据（已截断 ≤ 4 KiB）给 caller 自己解析。
    #[error("lark-cli exited {exit_code} (body code {body_code:?}): {message}")]
    NonZeroExit {
        exit_code: i32,
        body_code: Option<i64>,
        message: String,
        stdout: String,
        stderr: String,
    },

    /// stdout 不是合法 JSON。`stdout` 已截断 ≤ 4 KiB。
    #[error("lark-cli stdout is not valid JSON: {source}")]
    OutputParse {
        #[source]
        source: serde_json::Error,
        stdout: String,
    },

    /// 超 `RunOptions.timeout` 或 `LarkCli` 默认 30s。
    #[error("lark-cli timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    /// `RunOptions.stdin` 写入或 shutdown 失败 —— caller 提供的 stdin 数据
    /// 未真正交付给 lark-cli。**codex audit round-3** finding：原实现
    /// `let _ = write/shutdown` 静默吞错让 caller 误以为 stdin 已送达。
    #[error("failed to write {bytes_written} bytes to lark-cli stdin: {source}")]
    StdinWriteFailed {
        bytes_written: usize,
        #[source]
        source: std::io::Error,
    },
}

impl LarkError {
    /// 提示给 caller 的"是否值得重试"——本模块自身不重试。
    /// 判别规则：`Timeout` / OS exit 124 / 飞书 transient 业务码（99991663/99991664）。
    pub fn retriable(&self) -> bool {
        matches!(
            self,
            Self::Timeout { .. }
                | Self::NonZeroExit { exit_code: 124, .. }
                | Self::NonZeroExit {
                    body_code: Some(99991663 | 99991664),
                    ..
                }
        )
    }

    /// Variant 短名（`"Spawn"` / `"NonZeroExit"` / ...），给 journal 用。
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Spawn { .. } => "Spawn",
            Self::NonZeroExit { .. } => "NonZeroExit",
            Self::OutputParse { .. } => "OutputParse",
            Self::Timeout { .. } => "Timeout",
            Self::StdinWriteFailed { .. } => "StdinWriteFailed",
        }
    }
}

/// Truncate a String in-place to at most `MAX_FIELD_LEN_IN_ERR` bytes,
/// respecting char boundaries.
pub(crate) fn truncate_field(s: &mut String) {
    if s.len() > MAX_FIELD_LEN_IN_ERR {
        let mut cut = MAX_FIELD_LEN_IN_ERR;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
}

/// Truncate every element of a Vec<String> in place.
pub(crate) fn truncate_args(args: &mut Vec<String>) {
    for s in args {
        truncate_field(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io_err() -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::NotFound, "nope")
    }

    #[test]
    fn retriable_truth_table() {
        // Positive cases.
        assert!(LarkError::Timeout { timeout_ms: 100 }.retriable());
        assert!(
            LarkError::NonZeroExit {
                exit_code: 124,
                body_code: None,
                message: String::new(),
                stdout: String::new(),
                stderr: String::new(),
            }
            .retriable()
        );
        assert!(
            LarkError::NonZeroExit {
                exit_code: 1,
                body_code: Some(99991663),
                message: String::new(),
                stdout: String::new(),
                stderr: String::new(),
            }
            .retriable()
        );
        assert!(
            LarkError::NonZeroExit {
                exit_code: 1,
                body_code: Some(99991664),
                message: String::new(),
                stdout: String::new(),
                stderr: String::new(),
            }
            .retriable()
        );

        // Negative cases.
        assert!(
            !LarkError::NonZeroExit {
                exit_code: 1,
                body_code: None,
                message: String::new(),
                stdout: String::new(),
                stderr: String::new(),
            }
            .retriable()
        );
        assert!(
            !LarkError::NonZeroExit {
                exit_code: 1,
                body_code: Some(12345),
                message: String::new(),
                stdout: String::new(),
                stderr: String::new(),
            }
            .retriable()
        );
        assert!(
            !LarkError::Spawn {
                path: PathBuf::from("/nope"),
                program_args: vec![],
                source: io_err(),
            }
            .retriable()
        );
    }

    #[test]
    fn display_contains_variant_data() {
        let e = LarkError::Timeout { timeout_ms: 250 };
        assert!(format!("{e}").contains("250ms"));

        let e = LarkError::Spawn {
            path: PathBuf::from("/usr/local/bin/lark-cli"),
            program_args: vec!["im".into()],
            source: io_err(),
        };
        let msg = format!("{e}");
        assert!(msg.contains("/usr/local/bin/lark-cli"));
    }

    #[test]
    fn error_source_chain() {
        use std::error::Error as _;
        let e = LarkError::Spawn {
            path: PathBuf::from("/nope"),
            program_args: vec![],
            source: io_err(),
        };
        let src = e.source().expect("Spawn has source");
        assert!(src.to_string().contains("nope"));
    }

    #[test]
    fn non_exhaustive_match_in_external_view() {
        // For external crates, LarkError is #[non_exhaustive] and a `match`
        // without `_` arm fails to compile with E0004. Within the defining
        // crate, all variants are visible so the wildcard would be
        // unreachable; the defining-crate behavior is correct per Rust's
        // non_exhaustive semantics. This test simply asserts exhaustive
        // matching here works — an external compile_fail doctest in a
        // separate test crate would exercise the external view directly.
        let e = LarkError::Timeout { timeout_ms: 1 };
        let label = match &e {
            LarkError::Spawn { .. } => "spawn",
            LarkError::NonZeroExit { .. } => "exit",
            LarkError::OutputParse { .. } => "parse",
            LarkError::Timeout { .. } => "timeout",
            LarkError::StdinWriteFailed { .. } => "stdin_write",
        };
        assert_eq!(label, "timeout");
    }

    #[test]
    fn truncate_field_respects_char_boundary() {
        let mut s = "a".repeat(MAX_FIELD_LEN_IN_ERR + 100);
        truncate_field(&mut s);
        assert_eq!(s.len(), MAX_FIELD_LEN_IN_ERR);

        // multi-byte char straddling the cutoff
        let mut s = "a".repeat(MAX_FIELD_LEN_IN_ERR - 1) + "中文";
        truncate_field(&mut s);
        assert!(s.is_char_boundary(s.len()));
        assert!(s.len() <= MAX_FIELD_LEN_IN_ERR);
    }

    #[test]
    fn variant_name_round_trip() {
        assert_eq!(
            LarkError::Timeout { timeout_ms: 1 }.variant_name(),
            "Timeout"
        );
        assert_eq!(
            LarkError::Spawn {
                path: PathBuf::from("/x"),
                program_args: vec![],
                source: io_err(),
            }
            .variant_name(),
            "Spawn"
        );
    }
}
