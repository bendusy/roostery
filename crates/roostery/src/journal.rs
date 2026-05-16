//! Local jsonl audit journal — `JournalEntry` schema is the public contract
//! of `portable-by-default` (see roadmap §4.2). schema_version=1 becomes a
//! commitment once this module lands; breaking changes require a version bump
//! plus backwards-compatible deserialization plus `cs-roadmap update`.

use crate::SCHEMA_VERSION;
use crate::paths;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct JournalEntry {
    pub schema_version: u32,
    pub event_id: String,
    pub trace_id: Option<String>,
    pub parent_event_id: Option<String>,
    pub depth: u32,
    pub ts: chrono::DateTime<chrono::Utc>,
    pub source: String,
    pub action: String,
    pub params: serde_json::Value,
    pub result: JournalResult,
    pub duration_ms: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "outcome", rename_all = "lowercase")]
pub enum JournalResult {
    Ok { value: serde_json::Value },
    Err { kind: String, message: String },
}

impl JournalEntry {
    pub fn new(source: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            event_id: new_event_id(),
            trace_id: None,
            parent_event_id: None,
            depth: 0,
            ts: chrono::Utc::now(),
            source: source.into(),
            action: action.into(),
            params: serde_json::Value::Null,
            result: JournalResult::Ok {
                value: serde_json::Value::Null,
            },
            duration_ms: 0,
        }
    }
}

// --- ULID (Crockford base32, no external dep) -------------------------------

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn encode_b32(mut num: u128, length: usize) -> String {
    let mut out = vec![0u8; length];
    for i in (0..length).rev() {
        out[i] = CROCKFORD[(num & 0x1F) as usize];
        num >>= 5;
    }
    String::from_utf8(out).expect("Crockford alphabet is ASCII")
}

pub fn new_event_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut rand_bytes = [0u8; 10];
    getrandom::getrandom(&mut rand_bytes).expect("OS RNG available");
    let mut rand: u128 = 0;
    for b in rand_bytes {
        rand = (rand << 8) | b as u128;
    }
    encode_b32(ms, 10) + &encode_b32(rand, 16)
}

// --- Journal handle ---------------------------------------------------------

pub struct Journal {
    dir: PathBuf,
}

impl Journal {
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn append(&self, entry: &JournalEntry) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.dir)?;
        let filename = entry.ts.format("%Y-%m-%d").to_string() + ".jsonl";
        let path = self.dir.join(filename);
        let mut buf = serde_json::to_vec(entry).expect("JournalEntry serializes");
        buf.push(b'\n');
        let mut f = OpenOptions::new().append(true).create(true).open(&path)?;
        f.write_all(&buf)?;
        Ok(path)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Default for Journal {
    fn default() -> Self {
        Self::open(paths::journal_dir())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;

    // --- schema tests -------------------------------------------------------

    mod schema {
        use super::*;

        #[test]
        fn full_entry_roundtrip() {
            let original = JournalEntry {
                schema_version: 1,
                event_id: "01H000000000000000000XYZ12".into(),
                trace_id: Some("trace-abc".into()),
                parent_event_id: Some("01H000000000000000000PARENT".into()),
                depth: 2,
                ts: chrono::DateTime::parse_from_rfc3339("2026-05-16T10:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
                source: "shim".into(),
                action: "lark-cli:im_messages_send".into(),
                params: json!({"chat_id": "oc_x"}),
                result: JournalResult::Err {
                    kind: "Timeout".into(),
                    message: "5s".into(),
                },
                duration_ms: 1234,
            };
            let s = serde_json::to_string(&original).unwrap();
            let back: JournalEntry = serde_json::from_str(&s).unwrap();
            assert_eq!(back, original);
        }

        #[test]
        fn field_names_match_roadmap() {
            let e = JournalEntry::new("shim", "test");
            let v = serde_json::to_value(&e).unwrap();
            let obj = v.as_object().unwrap();
            for k in [
                "schema_version",
                "event_id",
                "trace_id",
                "parent_event_id",
                "depth",
                "ts",
                "source",
                "action",
                "params",
                "result",
                "duration_ms",
            ] {
                assert!(obj.contains_key(k), "missing field {k}");
            }
            assert_eq!(obj.len(), 11, "exactly 11 fields per roadmap §4.2");
        }

        #[test]
        fn result_ok_serializes_with_outcome_tag() {
            let r = JournalResult::Ok {
                value: json!({"x": 1}),
            };
            let s = serde_json::to_string(&r).unwrap();
            let v: serde_json::Value = serde_json::from_str(&s).unwrap();
            assert_eq!(v["outcome"], "ok");
            assert_eq!(v["value"], json!({"x": 1}));
        }

        #[test]
        fn result_err_serializes_with_outcome_tag() {
            let r = JournalResult::Err {
                kind: "Timeout".into(),
                message: "5s".into(),
            };
            let s = serde_json::to_string(&r).unwrap();
            let v: serde_json::Value = serde_json::from_str(&s).unwrap();
            assert_eq!(v["outcome"], "err");
            assert_eq!(v["kind"], "Timeout");
            assert_eq!(v["message"], "5s");
        }

        #[test]
        fn ts_serializes_as_rfc3339_with_z() {
            let e = JournalEntry::new("shim", "test");
            let v = serde_json::to_value(&e).unwrap();
            let ts = v["ts"].as_str().unwrap();
            assert!(
                ts.ends_with('Z') || ts.contains("+00:00"),
                "ts must be UTC, got {ts}"
            );
        }

        #[test]
        fn new_uses_schema_version_1() {
            let e = JournalEntry::new("shim", "test");
            assert_eq!(e.schema_version, 1);
            assert_eq!(e.depth, 0);
            assert!(e.trace_id.is_none());
            assert!(e.parent_event_id.is_none());
            assert_eq!(e.duration_ms, 0);
            assert_eq!(e.params, serde_json::Value::Null);
            match e.result {
                JournalResult::Ok { value } => assert_eq!(value, serde_json::Value::Null),
                _ => panic!("default result should be Ok(Null)"),
            }
        }
    }

    // --- ULID tests ---------------------------------------------------------

    mod ulid {
        use super::*;

        #[test]
        fn length_is_26() {
            for _ in 0..50 {
                assert_eq!(new_event_id().len(), 26);
            }
        }

        #[test]
        fn alphabet_is_crockford_base32() {
            let allowed: HashSet<u8> = CROCKFORD.iter().copied().collect();
            for _ in 0..50 {
                let id = new_event_id();
                for b in id.bytes() {
                    assert!(
                        allowed.contains(&b),
                        "char {} not in Crockford alphabet (full id: {id})",
                        b as char
                    );
                }
            }
        }

        #[test]
        fn many_calls_are_unique() {
            let mut seen = HashSet::new();
            for _ in 0..1000 {
                assert!(seen.insert(new_event_id()), "ULID collision");
            }
        }

        #[test]
        fn time_prefix_is_monotonic_across_ms() {
            let a = new_event_id();
            std::thread::sleep(std::time::Duration::from_millis(5));
            let b = new_event_id();
            assert!(
                a[..10] <= b[..10],
                "time prefix must be monotonic across ms boundaries: {a} vs {b}"
            );
        }
    }

    // --- append tests -------------------------------------------------------

    mod append {
        use super::*;
        use tempfile::tempdir;

        #[test]
        fn basic_write_and_readback() {
            let tmp = tempdir().unwrap();
            let j = Journal::open(tmp.path());
            let entry = JournalEntry::new("shim", "test_action");
            let path = j.append(&entry).unwrap();
            assert!(path.exists());
            let content = std::fs::read_to_string(&path).unwrap();
            assert_eq!(content.lines().count(), 1);
            let back: JournalEntry = serde_json::from_str(content.trim()).unwrap();
            assert_eq!(back, entry);
        }

        #[test]
        fn cross_day_backfill_lands_on_entry_ts_day() {
            let tmp = tempdir().unwrap();
            let j = Journal::open(tmp.path());
            let yesterday = chrono::Utc::now() - chrono::Duration::days(1);
            let mut entry = JournalEntry::new("shim", "backfill");
            entry.ts = yesterday;
            let path = j.append(&entry).unwrap();
            let expected_name = yesterday.format("%Y-%m-%d").to_string() + ".jsonl";
            assert_eq!(path.file_name().unwrap().to_str().unwrap(), expected_name);
        }

        #[test]
        fn mkdir_p_creates_nested_dir() {
            let tmp = tempdir().unwrap();
            let nested = tmp.path().join("nested/deep/journal");
            assert!(!nested.exists());
            let j = Journal::open(&nested);
            let entry = JournalEntry::new("shim", "test");
            j.append(&entry).unwrap();
            assert!(nested.is_dir());
        }

        #[test]
        fn multiple_appends_produce_multi_line_jsonl() {
            let tmp = tempdir().unwrap();
            let j = Journal::open(tmp.path());
            let mut paths = Vec::new();
            for i in 0..5 {
                let entry = JournalEntry::new("shim", format!("act_{i}"));
                paths.push(j.append(&entry).unwrap());
            }
            // All entries share the same UTC day → same file.
            let path = &paths[0];
            assert!(paths.iter().all(|p| p == path));
            let content = std::fs::read_to_string(path).unwrap();
            assert_eq!(content.lines().count(), 5);
            for line in content.lines() {
                let _: JournalEntry =
                    serde_json::from_str(line).expect("each line must be valid JSON");
            }
        }

        #[test]
        fn returned_path_matches_actual_write() {
            let tmp = tempdir().unwrap();
            let j = Journal::open(tmp.path());
            let entry = JournalEntry::new("shim", "test");
            let returned = j.append(&entry).unwrap();
            let listed: Vec<_> = std::fs::read_dir(tmp.path())
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0], returned);
        }
    }

    // --- redact integration -------------------------------------------------

    mod redact_integration {
        use super::*;
        use crate::redact;
        use tempfile::tempdir;

        #[test]
        fn scrubbed_params_persist_through_journal() {
            let tmp = tempdir().unwrap();
            let j = Journal::open(tmp.path());
            let raw = json!({
                "access_token": "xyz-secret",
                "user": "alice",
            });
            let (scrubbed, paths) = redact::scrub_value(&raw);
            assert!(!paths.is_empty(), "redact must report scrubbed paths");

            let mut entry = JournalEntry::new("shim", "lark-cli:im_messages_send");
            entry.params = scrubbed;
            let file = j.append(&entry).unwrap();

            let content = std::fs::read_to_string(&file).unwrap();
            let back: JournalEntry = serde_json::from_str(content.trim()).unwrap();
            assert_eq!(back.params["access_token"], redact::MASK);
            assert_eq!(back.params["user"], "alice");
        }
    }
}
