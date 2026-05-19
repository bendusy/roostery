//! Git log multi-repo aggregation for daily-recap engine.
//!
//! Spawns `git -C <repo> log --since=<ts> --until=<ts> --pretty=...` per repo
//! and parses the output into a structured [`GitLogAggregate`]. Field separator
//! is `\x1f` (US) and record separator is `\x1e` (RS) — chosen so commit
//! subjects/bodies containing newlines, tabs, or pipes survive intact.
//!
//! See feature `2026-05-19-report-recap-engine` design §2.1 + §2.4 step 4.
//! All types are pure data + `thiserror` errors; no async, no LLM SDK.
//!
//! Rust idiom-first per `2026-05-18-decision-rust-idiom-first`:
//! - [`CommitHash`] newtype (aligns with `business-identifier-newtype` decision)
//! - [`RepoSpec`] smart constructor (canonicalize + is_dir invariants)
//! - Errors are typed `thiserror::Error` enums with `#[from]` chains

use chrono::{DateTime, FixedOffset, NaiveDate};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

const FIELD_SEP: char = '\x1f';
const RECORD_SEP: char = '\x1e';
const PRETTY_FORMAT: &str = "%H\x1f%cI\x1f%an\x1f%s\x1f%b\x1e";

/// Git commit hash (full or short). Only invariant is trim + non-empty.
/// Git itself accepts shortened hashes, so we don't enforce length here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommitHash(String);

impl CommitHash {
    pub fn new(raw: &str) -> Result<Self, GitLogError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(GitLogError::InvalidHash(raw.to_string()));
        }
        Ok(Self(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CommitHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Repo spec; constructed via `RepoSpec::new` / `RepoSpec::with_name` so the
/// path is canonicalized and `is_dir` enforced before any git work runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoSpec {
    path: PathBuf,
    name: String,
}

impl RepoSpec {
    /// Construct from a path; derives `name` from the canonical path's last
    /// component (falls back to the original input if no file_name).
    pub fn new(raw_path: impl AsRef<Path>) -> Result<Self, RepoSpecError> {
        let raw = raw_path.as_ref();
        let canonical = raw
            .canonicalize()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => RepoSpecError::PathNotFound(raw.to_path_buf()),
                _ => RepoSpecError::Canonicalize {
                    path: raw.to_path_buf(),
                    source: e,
                },
            })?;
        if !canonical.is_dir() {
            return Err(RepoSpecError::NotADirectory(canonical));
        }
        let name = canonical
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| canonical.to_string_lossy().into_owned());
        Ok(Self {
            path: canonical,
            name,
        })
    }

    /// Same as `new` but lets caller override the display name.
    pub fn with_name(
        raw_path: impl AsRef<Path>,
        name: impl Into<String>,
    ) -> Result<Self, RepoSpecError> {
        let mut spec = Self::new(raw_path)?;
        spec.name = name.into();
        Ok(spec)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RepoSpecError {
    #[error("repo path not found: {0:?}")]
    PathNotFound(PathBuf),
    #[error("repo path not a directory: {0:?}")]
    NotADirectory(PathBuf),
    #[error("canonicalize failed for {path:?}: {source}")]
    Canonicalize {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Commit {
    pub hash: CommitHash,
    pub timestamp: DateTime<FixedOffset>,
    pub author: String,
    pub subject: String,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoCommits {
    pub repo: RepoSpec,
    pub commits: Vec<Commit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitLogAggregate {
    pub date: NaiveDate,
    /// User's local timezone offset captured at aggregation time; recorded so
    /// downstream consumers can audit the `--since/--until` boundary choice.
    /// `chrono::FixedOffset` does not implement `Serialize`, so we expose it
    /// as `i32` seconds in serialized form via `serialize_with`.
    #[serde(serialize_with = "serialize_offset")]
    pub timezone: FixedOffset,
    pub repos: Vec<RepoCommits>,
}

fn serialize_offset<S>(tz: &FixedOffset, ser: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    ser.serialize_i32(tz.local_minus_utc())
}

impl GitLogAggregate {
    pub fn is_empty(&self) -> bool {
        self.repos.iter().all(|r| r.commits.is_empty())
    }

    pub fn total_commits(&self) -> usize {
        self.repos.iter().map(|r| r.commits.len()).sum()
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GitLogError {
    #[error("repo not a git repository: {0:?}")]
    NotAGitRepo(PathBuf),
    #[error("git command spawn failed for {repo:?}: {source}")]
    Spawn {
        repo: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("git exited {exit_code} for {repo:?}: {stderr_head}")]
    NonZeroExit {
        repo: PathBuf,
        exit_code: i32,
        stderr_head: String,
    },
    #[error("git output parse failed for {repo:?}: {detail}")]
    ParseFailed { repo: PathBuf, detail: String },
    #[error("commit hash invalid: {0:?}")]
    InvalidHash(String),
}

/// Aggregate `git log` output across `repos` for the local day `date` in
/// timezone `tz`. The lower bound is `date T00:00:00 tz` and the upper bound
/// is `(date+1) T00:00:00 tz` (half-open).
pub fn collect_aggregate(
    date: NaiveDate,
    tz: FixedOffset,
    repos: &[RepoSpec],
) -> Result<GitLogAggregate, GitLogError> {
    let since = format!("{date}T00:00:00{}", format_offset(tz));
    let next_day = date.succ_opt().ok_or_else(|| GitLogError::ParseFailed {
        repo: PathBuf::new(),
        detail: format!("date {date} has no successor"),
    })?;
    let until = format!("{next_day}T00:00:00{}", format_offset(tz));

    let mut collected = Vec::with_capacity(repos.len());
    for repo in repos {
        let commits = collect_repo(repo, &since, &until)?;
        collected.push(RepoCommits {
            repo: repo.clone(),
            commits,
        });
    }

    Ok(GitLogAggregate {
        date,
        timezone: tz,
        repos: collected,
    })
}

fn format_offset(tz: FixedOffset) -> String {
    // chrono's FixedOffset Display already gives "+08:00" form.
    tz.to_string()
}

fn collect_repo(
    repo: &RepoSpec,
    since: &str,
    until: &str,
) -> Result<Vec<Commit>, GitLogError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .arg("log")
        .arg(format!("--since={since}"))
        .arg(format!("--until={until}"))
        .arg(format!("--pretty=format:{PRETTY_FORMAT}"))
        .output()
        .map_err(|e| GitLogError::Spawn {
            repo: repo.path().to_path_buf(),
            source: e,
        })?;

    if !output.status.success() {
        let exit_code = output.status.code().unwrap_or(-1);
        let stderr_head = String::from_utf8_lossy(&output.stderr);
        let head: String = stderr_head.chars().take(200).collect();
        let head_lower = head.to_lowercase();
        // Git uses 128 for "not a git repo", so map that for clarity.
        if exit_code == 128 && head_lower.contains("not a git repository") {
            return Err(GitLogError::NotAGitRepo(repo.path().to_path_buf()));
        }
        // An initialized-but-empty repo (no commits yet) also exits 128.
        // Treat as zero-commit success — it's a valid repo, just quiet.
        if exit_code == 128 && head_lower.contains("does not have any commits yet") {
            return Ok(Vec::new());
        }
        return Err(GitLogError::NonZeroExit {
            repo: repo.path().to_path_buf(),
            exit_code,
            stderr_head: head,
        });
    }

    let stdout = std::str::from_utf8(&output.stdout).map_err(|e| GitLogError::ParseFailed {
        repo: repo.path().to_path_buf(),
        detail: format!("non-UTF-8 stdout: {e}"),
    })?;

    parse_pretty(stdout, repo.path())
}

fn parse_pretty(stdout: &str, repo_path: &Path) -> Result<Vec<Commit>, GitLogError> {
    if stdout.is_empty() {
        return Ok(Vec::new());
    }

    let mut commits = Vec::new();
    for raw in stdout.split(RECORD_SEP) {
        // The trailing record after the final RS is always empty; skip it
        // along with any whitespace-only fragments git may emit between
        // records (newlines `git log` injects implicitly).
        let trimmed = raw.trim_start_matches('\n');
        if trimmed.is_empty() {
            continue;
        }
        commits.push(parse_record(trimmed, repo_path)?);
    }
    Ok(commits)
}

fn parse_record(record: &str, repo_path: &Path) -> Result<Commit, GitLogError> {
    let mut parts = record.splitn(5, FIELD_SEP);
    let hash_raw = parts.next().ok_or_else(|| GitLogError::ParseFailed {
        repo: repo_path.to_path_buf(),
        detail: "missing hash field".into(),
    })?;
    let ts_raw = parts.next().ok_or_else(|| GitLogError::ParseFailed {
        repo: repo_path.to_path_buf(),
        detail: "missing timestamp field".into(),
    })?;
    let author = parts.next().ok_or_else(|| GitLogError::ParseFailed {
        repo: repo_path.to_path_buf(),
        detail: "missing author field".into(),
    })?;
    let subject = parts.next().ok_or_else(|| GitLogError::ParseFailed {
        repo: repo_path.to_path_buf(),
        detail: "missing subject field".into(),
    })?;
    let body_raw = parts.next().unwrap_or("");

    let hash = CommitHash::new(hash_raw)?;
    let timestamp =
        DateTime::parse_from_rfc3339(ts_raw.trim()).map_err(|e| GitLogError::ParseFailed {
            repo: repo_path.to_path_buf(),
            detail: format!("bad timestamp {ts_raw:?}: {e}"),
        })?;
    let body = {
        let trimmed = body_raw.trim_matches(|c: char| c == '\n' || c == '\r');
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    };

    Ok(Commit {
        hash,
        timestamp,
        author: author.to_string(),
        subject: subject.to_string(),
        body,
    })
}

/// Render aggregate to human-readable markdown:
///
/// ```text
/// ## <repo name>
/// - <short hash> <subject>   _<author>, <timestamp>_
///   <body indented if present>
/// ```
///
/// Repo with zero commits gets a `(no commits today)` line so the caller can
/// distinguish "missing" from "quiet" repos.
pub fn render_markdown(aggregate: &GitLogAggregate) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Daily git activity — {} ({})\n\n",
        aggregate.date,
        aggregate.timezone
    ));
    for rc in &aggregate.repos {
        out.push_str(&format!("## {}\n\n", rc.repo.name()));
        if rc.commits.is_empty() {
            out.push_str("_(no commits today)_\n\n");
            continue;
        }
        for c in &rc.commits {
            let short = c.hash.as_str().chars().take(8).collect::<String>();
            out.push_str(&format!(
                "- `{}` {}   _{}, {}_\n",
                short,
                c.subject,
                c.author,
                c.timestamp.format("%H:%M %z")
            ));
            if let Some(body) = &c.body {
                for line in body.lines() {
                    out.push_str(&format!("  > {line}\n"));
                }
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::fs;
    use tempfile::TempDir;

    fn tz_utc8() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    fn make_repo(commits: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        let path = dir.path();
        run_git(path, &["init", "--initial-branch=main"]);
        run_git(path, &["config", "user.email", "t@example.com"]);
        run_git(path, &["config", "user.name", "Tester"]);
        run_git(path, &["config", "commit.gpgsign", "false"]);
        for (subject, body) in commits {
            let file = path.join("f.txt");
            fs::write(&file, subject).unwrap();
            run_git(path, &["add", "f.txt"]);
            let mut args = vec!["commit", "-m", subject];
            if !body.is_empty() {
                args.push("-m");
                args.push(body);
            }
            run_git(path, &args);
        }
        dir
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let out = Command::new("git").arg("-C").arg(cwd).args(args).output().unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn commit_hash_rejects_empty() {
        assert!(CommitHash::new("").is_err());
        assert!(CommitHash::new("   ").is_err());
        assert_eq!(CommitHash::new(" abc1234 ").unwrap().as_str(), "abc1234");
    }

    #[test]
    fn repo_spec_canonicalizes_and_derives_name() {
        let dir = TempDir::new().unwrap();
        let spec = RepoSpec::new(dir.path()).unwrap();
        assert!(spec.path().is_absolute());
        assert!(!spec.name().is_empty());
    }

    #[test]
    fn repo_spec_with_name_overrides() {
        let dir = TempDir::new().unwrap();
        let spec = RepoSpec::with_name(dir.path(), "my-proj").unwrap();
        assert_eq!(spec.name(), "my-proj");
    }

    #[test]
    fn repo_spec_path_not_found() {
        match RepoSpec::new(Path::new("/path/that/does/not/exist/abc123")) {
            Err(RepoSpecError::PathNotFound(_)) => {}
            other => panic!("expected PathNotFound, got {other:?}"),
        }
    }

    #[test]
    fn empty_repo_returns_no_commits() {
        let dir = TempDir::new().unwrap();
        run_git(dir.path(), &["init", "--initial-branch=main"]);
        let spec = RepoSpec::new(dir.path()).unwrap();
        let date = chrono::Utc::now().with_timezone(&tz_utc8()).date_naive();
        let agg = collect_aggregate(date, tz_utc8(), &[spec]).unwrap();
        assert_eq!(agg.repos.len(), 1);
        assert!(agg.repos[0].commits.is_empty());
        assert!(agg.is_empty());
    }

    #[test]
    fn single_commit_today_collected() {
        let dir = make_repo(&[("feat: hello", "")]);
        let spec = RepoSpec::new(dir.path()).unwrap();
        let date = chrono::Utc::now().with_timezone(&tz_utc8()).date_naive();
        let agg = collect_aggregate(date, tz_utc8(), &[spec]).unwrap();
        assert_eq!(agg.total_commits(), 1);
        let c = &agg.repos[0].commits[0];
        assert_eq!(c.subject, "feat: hello");
        assert_eq!(c.body, None);
        assert_eq!(c.author, "Tester");
        assert!(!c.hash.as_str().is_empty());
    }

    #[test]
    fn multi_commit_body_with_newline_preserved() {
        let dir = make_repo(&[
            ("first", ""),
            ("second", "line1\nline2\nline3"),
            ("third", "with `backticks` and special chars: |\\n"),
        ]);
        let spec = RepoSpec::new(dir.path()).unwrap();
        let date = chrono::Utc::now().with_timezone(&tz_utc8()).date_naive();
        let agg = collect_aggregate(date, tz_utc8(), &[spec]).unwrap();
        assert_eq!(agg.total_commits(), 3);
        // Order: most-recent first per `git log` default.
        let third = &agg.repos[0].commits[0];
        assert_eq!(third.subject, "third");
        assert!(third.body.as_ref().unwrap().contains("backticks"));
        let second = &agg.repos[0].commits[1];
        assert_eq!(second.subject, "second");
        let body = second.body.as_ref().unwrap();
        assert!(body.contains("line1"));
        assert!(body.contains("line2"));
        assert!(body.contains("line3"));
        let first = &agg.repos[0].commits[2];
        assert_eq!(first.body, None);
    }

    #[test]
    fn not_a_git_repo_returns_typed_error() {
        let dir = TempDir::new().unwrap(); // tempdir is not a git repo
        let spec = RepoSpec::new(dir.path()).unwrap();
        let date = chrono::Utc::now().with_timezone(&tz_utc8()).date_naive();
        match collect_aggregate(date, tz_utc8(), &[spec]) {
            Err(GitLogError::NotAGitRepo(_)) => {}
            other => panic!("expected NotAGitRepo, got {other:?}"),
        }
    }

    #[test]
    fn future_date_returns_empty() {
        let dir = make_repo(&[("only commit", "")]);
        let spec = RepoSpec::new(dir.path()).unwrap();
        // Pick a date 10 years in the future — no commits should match.
        let future = chrono::Utc
            .with_ymd_and_hms(2999, 1, 1, 0, 0, 0)
            .unwrap()
            .date_naive();
        let agg = collect_aggregate(future, tz_utc8(), &[spec]).unwrap();
        assert!(agg.is_empty());
    }

    #[test]
    fn render_markdown_contains_repo_and_commits() {
        let dir = make_repo(&[("fix: bug", "longer body text"), ("feat: add", "")]);
        let spec = RepoSpec::with_name(dir.path(), "my-proj").unwrap();
        let date = chrono::Utc::now().with_timezone(&tz_utc8()).date_naive();
        let agg = collect_aggregate(date, tz_utc8(), &[spec]).unwrap();
        let md = render_markdown(&agg);
        assert!(md.contains("# Daily git activity"));
        assert!(md.contains("## my-proj"));
        assert!(md.contains("fix: bug"));
        assert!(md.contains("feat: add"));
        assert!(md.contains("longer body text"));
        assert!(md.contains("Tester"));
    }

    #[test]
    fn render_markdown_empty_repo_shows_no_commits_line() {
        let dir = TempDir::new().unwrap();
        run_git(dir.path(), &["init", "--initial-branch=main"]);
        let spec = RepoSpec::with_name(dir.path(), "quiet").unwrap();
        let date = chrono::Utc::now().with_timezone(&tz_utc8()).date_naive();
        let agg = collect_aggregate(date, tz_utc8(), &[spec]).unwrap();
        let md = render_markdown(&agg);
        assert!(md.contains("## quiet"));
        assert!(md.contains("no commits today"));
    }
}
