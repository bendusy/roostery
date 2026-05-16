//! `Journaled<R: LarkRunner>` decorator that writes a `JournalEntry` per
//! call (with `redact::scrub_argv` on params). See module-level docs.

use crate::journal::{Journal, JournalEntry, JournalResult};
use crate::lark_cli::error::LarkError;
use crate::lark_cli::runner::{LarkRunner, RunOptions};
use crate::redact;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::time::Instant;

/// Decorator that writes a `JournalEntry` per call (params pre-scrubbed
/// via `redact::scrub_argv`). The wrapped `R` does the actual work; this
/// layer only adds journaling.
pub struct Journaled<R: LarkRunner> {
    inner: R,
    journal: Journal,
    source: String,
}

impl<R: LarkRunner> Journaled<R> {
    pub fn new(inner: R, journal: Journal, source: impl Into<String>) -> Self {
        Self {
            inner,
            journal,
            source: source.into(),
        }
    }
}

#[async_trait]
impl<R: LarkRunner> LarkRunner for Journaled<R> {
    async fn run_with_options(&self, args: &[&str], opts: RunOptions) -> Result<Value, LarkError> {
        let started = Instant::now();
        let result = self.inner.run_with_options(args, opts.clone()).await;
        let duration_ms = started.elapsed().as_millis() as u64;

        let args_owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (scrubbed_argv, _paths) = redact::scrub_argv(&args_owned);
        let params = json!({
            "argv": scrubbed_argv,
            "options": run_options_to_value(&opts),
        });

        let action = format!("lark-cli:{}", args.first().copied().unwrap_or("<empty>"));

        let entry_result = match &result {
            Ok(value) => JournalResult::Ok {
                value: value.clone(),
            },
            Err(e) => JournalResult::Err {
                kind: e.variant_name().to_string(),
                message: e.to_string(),
            },
        };

        let mut entry = JournalEntry::new(self.source.clone(), action);
        entry.params = params;
        entry.result = entry_result;
        entry.duration_ms = duration_ms;

        if let Err(io_err) = self.journal.append(&entry) {
            tracing::warn!(
                error = %io_err,
                source = %self.source,
                "Journaled: failed to write journal entry; original result preserved"
            );
        }

        result
    }
}

fn run_options_to_value(opts: &RunOptions) -> Value {
    json!({
        "timeout_ms": opts.timeout.map(|d| d.as_millis() as u64),
        "has_stdin": opts.stdin.is_some(),
        "profile": opts.profile.as_deref(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lark_cli::mock::MockLarkRunner;
    use tempfile::tempdir;

    fn journal_in_tmp() -> (tempfile::TempDir, Journal) {
        let dir = tempdir().unwrap();
        let journal = Journal::open(dir.path());
        (dir, journal)
    }

    fn read_one_entry(dir: &std::path::Path) -> JournalEntry {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        let content = std::fs::read_to_string(&files[0]).unwrap();
        let line = content.lines().next().unwrap();
        serde_json::from_str(line).unwrap()
    }

    #[tokio::test]
    async fn s5_1_happy_path_writes_entry() {
        let (dir, journal) = journal_in_tmp();
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!({"data": {"message_id": "om_abc"}}));
        let runner = Journaled::new(mock, journal, "test_source");

        let v = runner
            .run(&["im", "+messages-send", "--text", "hi"])
            .await
            .unwrap();
        assert_eq!(v, json!({"data": {"message_id": "om_abc"}}));

        let entry = read_one_entry(dir.path());
        assert_eq!(entry.schema_version, 1);
        assert_eq!(entry.source, "test_source");
        assert_eq!(entry.action, "lark-cli:im");
        assert_eq!(
            entry.params["argv"],
            json!(["im", "+messages-send", "--text", "hi"])
        );
        match entry.result {
            JournalResult::Ok { value } => {
                assert_eq!(value, json!({"data": {"message_id": "om_abc"}}));
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn s5_2_err_path_writes_kind_and_message() {
        let (dir, journal) = journal_in_tmp();
        let mock = MockLarkRunner::new();
        mock.enqueue_err(LarkError::Timeout { timeout_ms: 5 });
        let runner = Journaled::new(mock, journal, "test_source");

        let err = runner.run(&["docs", "+create"]).await.unwrap_err();
        assert!(matches!(err, LarkError::Timeout { .. }));

        let entry = read_one_entry(dir.path());
        match entry.result {
            JournalResult::Err { kind, message } => {
                assert_eq!(kind, "Timeout");
                assert!(message.contains("5ms"));
            }
            other => panic!("expected Err, got {other:?}"),
        }
        assert_eq!(entry.action, "lark-cli:docs");
    }

    #[tokio::test]
    async fn s5_3_argv_scrubbed_via_redact() {
        let (dir, journal) = journal_in_tmp();
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!(null));
        let runner = Journaled::new(mock, journal, "test_source");

        // --access-token is in redact::SENSITIVE_KEYS; the value after it
        // must be masked.
        runner
            .run(&["im", "--access-token", "xyz-secret", "+messages-send"])
            .await
            .unwrap();

        let entry = read_one_entry(dir.path());
        let argv = entry.params["argv"].as_array().unwrap();
        assert_eq!(argv[0], "im");
        assert_eq!(argv[1], "--access-token");
        assert_eq!(argv[2], redact::MASK); // "xyz-secret" → "***"
        assert_eq!(argv[3], "+messages-send");
    }

    #[tokio::test]
    async fn s5_4_journal_write_failure_preserves_original_result() {
        // Construct a Journal pointing to a non-writable path (a path under
        // a file rather than a directory). Journal::append will fail
        // mkdir_p; Journaled must swallow that and return original result.
        let tempfile = tempfile::NamedTempFile::new().unwrap();
        // mkdir_p on tempfile.path().join("nested") fails because tempfile
        // is a regular file, not a directory.
        let bad_journal_dir = tempfile.path().join("nested");
        let journal = Journal::open(&bad_journal_dir);

        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!({"ok": true}));
        let runner = Journaled::new(mock, journal, "test_source");

        let v = runner.run(&["any"]).await.unwrap();
        assert_eq!(v, json!({"ok": true}));
        // No way to assert journal was written (it wasn't); the test
        // succeeds simply by Journaled not panicking and original result
        // passing through unchanged.
    }

    #[tokio::test]
    async fn dyn_compat_journaled_over_box_dyn_works() {
        let (_dir, journal) = journal_in_tmp();
        let mock = MockLarkRunner::new();
        mock.enqueue_ok(json!("v"));
        let runner = Journaled::new(mock, journal, "test_source");
        let r: Box<dyn LarkRunner> = Box::new(runner);
        let v = r.run(&["x"]).await.unwrap();
        assert_eq!(v, json!("v"));
    }
}
