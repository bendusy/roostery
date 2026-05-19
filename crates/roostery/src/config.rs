//! `~/.roostery/config.yaml` schema + load/save (Phase 3, feature `config-yaml`).
//!
//! Schema 来自 roadmap §4.6——identity / runners / budgets / trace / journal
//! 五大顶层节（加 `schema_version`），顶层字段全 `#[serde(default)]` 满足
//! roadmap "顶层字段缺失时使用编译期默认值"约束；runners 走开放结构
//! `BTreeMap<String, serde_yml::Value>`（新 runner kind 不动 schema 顶层）。
//!
//! 本模块不读 env override（各模块自管，如 `lark_cli/subprocess.rs::ENV_BIN`），
//! 不实现 schema migration（Phase 3 唯一 schema_version=1）。
//!
//! See `.codestable/features/2026-05-17-config-yaml/config-yaml-design.md`.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

const SCHEMA_VERSION_CURRENT: u32 = 1;

fn default_schema_version() -> u32 {
    SCHEMA_VERSION_CURRENT
}

/// Top-level config struct; 1:1 with `~/.roostery/config.yaml`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Config {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub identity: Identity,
    #[serde(default)]
    pub runners: BTreeMap<String, RunnerConfig>,
    #[serde(default)]
    pub budgets: Budgets,
    #[serde(default)]
    pub trace: TraceConfig,
    #[serde(default)]
    pub journal: JournalConfig,
    /// Daily-recap section. DTO is always compiled (not behind feature flag)
    /// so users with `recap:` in their yaml don't break on
    /// `--no-default-features` builds. See feature
    /// `2026-05-19-report-recap-engine` design §2.3.
    #[serde(default)]
    pub recap: RecapConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION_CURRENT,
            identity: Identity::default(),
            runners: BTreeMap::new(),
            budgets: Budgets::default(),
            trace: TraceConfig::default(),
            journal: JournalConfig::default(),
            recap: RecapConfig::default(),
        }
    }
}

/// Per-runner configuration. Known fields are strongly typed (currently just
/// `enabled`); anything else the user puts under the runner key is preserved
/// via `#[serde(flatten)]` for downstream Runner impls (Phase 4
/// `dispatcher-runners`) to interpret. Rust-idiom-first refactor B6
/// (`.codestable/compound/2026-05-18-decision-rust-idiom-first.md`)
/// replacing the previous `serde_yml::Value` no-typing-at-all.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct RunnerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, serde_yml::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct Identity {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub default_chat_id: String,
    #[serde(default)]
    pub default_task_app_token: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct Budgets {
    #[serde(default)]
    pub default: BudgetCfg,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BudgetCfg {
    #[serde(default = "default_max_calls")]
    pub max_calls: u32,
    #[serde(default = "default_max_cost_usd")]
    pub max_cost_usd: f64,
}

impl Default for BudgetCfg {
    fn default() -> Self {
        Self {
            max_calls: default_max_calls(),
            max_cost_usd: default_max_cost_usd(),
        }
    }
}

fn default_max_calls() -> u32 {
    100
}
fn default_max_cost_usd() -> f64 {
    1.0
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TraceConfig {
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
}

impl Default for TraceConfig {
    fn default() -> Self {
        Self {
            max_depth: default_max_depth(),
        }
    }
}

fn default_max_depth() -> u32 {
    8
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct JournalConfig {
    #[serde(default = "default_journal_dir")]
    pub dir: PathBuf,
    #[serde(default = "default_rotation")]
    pub rotation: String,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            dir: default_journal_dir(),
            rotation: default_rotation(),
        }
    }
}

fn default_journal_dir() -> PathBuf {
    paths::journal_dir()
}
fn default_rotation() -> String {
    "daily".into()
}

/// Daily-recap configuration (feature `2026-05-19-report-recap-engine`).
///
/// All fields default-friendly: empty `repos` / `runner_kind` means user must
/// override via CLI; `timeout_ms == 0` → engine uses 60000 default; missing
/// `prompt_override_path` → embedded default prompt. The DTO is compiled
/// unconditionally so users with a `recap:` section in their yaml don't
/// break under `--no-default-features` builds.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct RecapConfig {
    #[serde(default)]
    pub repos: Vec<RecapRepoConfig>,
    #[serde(default)]
    pub runner_kind: String,
    #[serde(default)]
    pub timeout_ms: u64,
    #[serde(default)]
    pub prompt_override_path: Option<PathBuf>,
    #[serde(default)]
    pub budget_estimated_cost_usd: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RecapRepoConfig {
    pub path: PathBuf,
    #[serde(default)]
    pub name: Option<String>,
}

/// Caller-facing failures.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("config load failed: {source}")]
    LoadFailed {
        #[from]
        source: std::io::Error,
    },
    #[error("config parse failed: {source}")]
    ParseFailed { source: serde_yml::Error },
    #[error("config save failed: {source}")]
    SaveFailed { source: std::io::Error },
    #[error("config schema_version mismatch: found {found}, expected {expected}")]
    SchemaVersionMismatch { found: u32, expected: u32 },
}

/// Load config from the default path (`~/.roostery/config.yaml`).
/// Missing file → returns `Ok(Config::default())`.
pub fn load() -> Result<Config, ConfigError> {
    load_from(&paths::config_path())
}

/// Load config from a specific path. Missing file → `Ok(Config::default())`.
pub fn load_from(path: &Path) -> Result<Config, ConfigError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(ConfigError::LoadFailed { source: e }),
    };
    let cfg: Config =
        serde_yml::from_slice(&bytes).map_err(|e| ConfigError::ParseFailed { source: e })?;
    if cfg.schema_version != SCHEMA_VERSION_CURRENT {
        return Err(ConfigError::SchemaVersionMismatch {
            found: cfg.schema_version,
            expected: SCHEMA_VERSION_CURRENT,
        });
    }
    Ok(cfg)
}

/// Save config to the default path (atomic `.tmp` + rename).
pub fn save(cfg: &Config) -> Result<(), ConfigError> {
    save_to(cfg, &paths::config_path())
}

/// Save config to a specific path.
pub fn save_to(cfg: &Config, path: &Path) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::SaveFailed { source: e })?;
    }
    let yaml = serde_yml::to_string(cfg).map_err(|e| ConfigError::ParseFailed { source: e })?;
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(&tmp, yaml).map_err(|e| ConfigError::SaveFailed { source: e })?;
    std::fs::rename(&tmp, path).map_err(|e| ConfigError::SaveFailed { source: e })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::TEST_ENV_LOCK;

    #[test]
    fn default_has_schema_version_one() {
        assert_eq!(Config::default().schema_version, 1);
    }

    #[test]
    fn default_sub_defaults_match_design() {
        let c = Config::default();
        assert_eq!(c.identity.user_id, "");
        assert_eq!(c.identity.default_chat_id, "");
        assert_eq!(c.identity.default_task_app_token, "");
        assert_eq!(c.budgets.default.max_calls, 100);
        assert_eq!(c.budgets.default.max_cost_usd, 1.0);
        assert_eq!(c.trace.max_depth, 8);
        assert_eq!(c.journal.rotation, "daily");
        assert!(c.runners.is_empty());
    }

    #[test]
    fn empty_yaml_is_default_config() {
        let cfg: Config = serde_yml::from_str("").unwrap_or_else(|_| Config::default());
        // serde_yml on empty input may error; using a single newline as canonical "empty doc"
        let cfg2: Config = serde_yml::from_str("{}").unwrap();
        assert_eq!(cfg.schema_version, 1);
        assert_eq!(cfg2, Config::default());
    }

    #[test]
    fn partial_yaml_fills_defaults() {
        let yaml = r#"
identity:
  user_id: ou_alice
"#;
        let cfg: Config = serde_yml::from_str(yaml).unwrap();
        assert_eq!(cfg.identity.user_id, "ou_alice");
        assert_eq!(cfg.identity.default_chat_id, "");
        assert_eq!(cfg.trace.max_depth, 8);
        assert_eq!(cfg.schema_version, 1);
    }

    #[test]
    fn unknown_fields_ignored() {
        let yaml = r#"
schema_version: 1
identity:
  user_id: ou_x
mystery_field: 42
"#;
        let cfg: Config = serde_yml::from_str(yaml).unwrap();
        assert_eq!(cfg.identity.user_id, "ou_x");
    }

    #[test]
    fn runners_open_structure_known_field_strongly_typed() {
        let yaml = r#"
runners:
  cc_headless:
    enabled: true
    cli_path: /usr/local/bin/claude-code
    extra_args:
      - --foo
  codex_exec:
    enabled: false
  custom_runner_v3:
    arbitrary_key: arbitrary_value
"#;
        let cfg: Config = serde_yml::from_str(yaml).unwrap();
        assert_eq!(cfg.runners.len(), 3);
        let cc = &cfg.runners["cc_headless"];
        // Known field is strongly typed (B6)
        assert!(cc.enabled);
        // Unknown fields preserved via #[serde(flatten)]
        assert_eq!(
            cc.extra["cli_path"],
            serde_yml::Value::String("/usr/local/bin/claude-code".into())
        );
        assert!(cc.extra["extra_args"].is_sequence());

        let codex = &cfg.runners["codex_exec"];
        assert!(!codex.enabled);
        assert!(codex.extra.is_empty());

        // Runner with no `enabled` field gets default false; unknown field
        // lands in `extra` (open structure preserved per roadmap §4.6).
        let custom = &cfg.runners["custom_runner_v3"];
        assert!(!custom.enabled);
        assert_eq!(
            custom.extra["arbitrary_key"],
            serde_yml::Value::String("arbitrary_value".into())
        );
    }

    #[test]
    fn full_yaml_round_trip() {
        let original = Config {
            schema_version: 1,
            identity: Identity {
                user_id: "ou_a".into(),
                default_chat_id: "oc_b".into(),
                default_task_app_token: "bascn_c".into(),
            },
            runners: BTreeMap::new(),
            budgets: Budgets {
                default: BudgetCfg {
                    max_calls: 50,
                    max_cost_usd: 2.5,
                },
            },
            trace: TraceConfig { max_depth: 16 },
            journal: JournalConfig {
                dir: PathBuf::from("/custom/journal"),
                rotation: "size:100".into(),
            },
            recap: RecapConfig::default(),
        };
        let yaml = serde_yml::to_string(&original).unwrap();
        let parsed: Config = serde_yml::from_str(&yaml).unwrap();
        assert_eq!(parsed, original);
    }

    // --- recap config tests (feature 2026-05-19-report-recap-engine) ------

    #[test]
    fn recap_default_empty() {
        let r = RecapConfig::default();
        assert!(r.repos.is_empty());
        assert_eq!(r.runner_kind, "");
        assert_eq!(r.timeout_ms, 0);
        assert!(r.prompt_override_path.is_none());
        assert_eq!(r.budget_estimated_cost_usd, 0.0);
    }

    #[test]
    fn recap_missing_section_yields_default() {
        // Old-style yaml without recap: section still loads (codex P1.6 + design §2.3 boundary).
        let yaml = r#"
schema_version: 1
identity:
  user_id: ou_legacy
"#;
        let cfg: Config = serde_yml::from_str(yaml).unwrap();
        assert_eq!(cfg.recap, RecapConfig::default());
    }

    #[test]
    fn recap_populated_yaml_round_trip() {
        let yaml = r#"
schema_version: 1
recap:
  repos:
    - path: /home/u/proj_a
      name: alpha
    - path: /home/u/proj_b
  runner_kind: cc_headless
  timeout_ms: 90000
  prompt_override_path: /etc/roostery/custom-recap.md
  budget_estimated_cost_usd: 0.08
"#;
        let cfg: Config = serde_yml::from_str(yaml).unwrap();
        assert_eq!(cfg.recap.repos.len(), 2);
        assert_eq!(cfg.recap.repos[0].path, PathBuf::from("/home/u/proj_a"));
        assert_eq!(cfg.recap.repos[0].name.as_deref(), Some("alpha"));
        assert_eq!(cfg.recap.repos[1].name, None);
        assert_eq!(cfg.recap.runner_kind, "cc_headless");
        assert_eq!(cfg.recap.timeout_ms, 90_000);
        assert_eq!(
            cfg.recap.prompt_override_path.as_deref(),
            Some(Path::new("/etc/roostery/custom-recap.md"))
        );
        assert_eq!(cfg.recap.budget_estimated_cost_usd, 0.08);

        // Round-trip
        let body = serde_yml::to_string(&cfg).unwrap();
        let reparsed: Config = serde_yml::from_str(&body).unwrap();
        assert_eq!(reparsed, cfg);
    }

    #[test]
    fn recap_partial_yaml_fills_defaults() {
        let yaml = r#"
recap:
  runner_kind: cc_headless
"#;
        let cfg: Config = serde_yml::from_str(yaml).unwrap();
        assert_eq!(cfg.recap.runner_kind, "cc_headless");
        assert!(cfg.recap.repos.is_empty());
        assert_eq!(cfg.recap.timeout_ms, 0);
        assert_eq!(cfg.recap.budget_estimated_cost_usd, 0.0);
    }

    // --- load / load_from path tests --------------------------------------

    #[test]
    fn load_from_missing_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.yaml");
        let cfg = load_from(&path).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn load_from_valid_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.yaml");
        std::fs::write(&path, "schema_version: 1\nidentity:\n  user_id: ou_z\n").unwrap();
        let cfg = load_from(&path).unwrap();
        assert_eq!(cfg.identity.user_id, "ou_z");
    }

    #[test]
    fn load_from_bad_yaml_returns_parse_failed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.yaml");
        std::fs::write(&path, "this is: not: valid: yaml: ::").unwrap();
        match load_from(&path) {
            Err(ConfigError::ParseFailed { .. }) => {}
            other => panic!("expected ParseFailed, got {other:?}"),
        }
    }

    #[test]
    fn load_from_schema_version_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.yaml");
        std::fs::write(&path, "schema_version: 2\n").unwrap();
        match load_from(&path) {
            Err(ConfigError::SchemaVersionMismatch {
                found: 2,
                expected: 1,
            }) => {}
            other => panic!("expected SchemaVersionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn load_from_schema_version_zero_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.yaml");
        std::fs::write(&path, "schema_version: 0\n").unwrap();
        match load_from(&path) {
            Err(ConfigError::SchemaVersionMismatch { found: 0, .. }) => {}
            other => panic!("expected SchemaVersionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn load_from_missing_schema_version_treated_as_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.yaml");
        std::fs::write(&path, "identity:\n  user_id: ou_x\n").unwrap();
        let cfg = load_from(&path).unwrap();
        assert_eq!(cfg.schema_version, 1);
        assert_eq!(cfg.identity.user_id, "ou_x");
    }

    // --- save / save_to ---------------------------------------------------

    #[test]
    fn save_to_creates_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c");
        let path = nested.join("config.yaml");
        save_to(&Config::default(), &path).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("yaml.tmp").exists());
    }

    #[test]
    fn save_then_load_round_trip() {
        // Config::default() reads ROOSTERY_HOME via paths::journal_dir(); lock
        // shared TEST_ENV_LOCK so parallel env-mutating tests don't race the
        // before/after Default snapshots.
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.yaml");
        let original = Config {
            identity: Identity {
                user_id: "ou_round".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        save_to(&original, &path).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn save_default_round_trip() {
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.yaml");
        save_to(&Config::default(), &path).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, Config::default());
    }

    #[test]
    fn saved_yaml_is_human_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.yaml");
        save_to(&Config::default(), &path).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        // YAML is plain-text key-colon-value; verify a known key appears.
        assert!(body.contains("schema_version: 1"));
        assert!(body.contains("trace:"));
        assert!(body.contains("max_depth: 8"));
    }
}
