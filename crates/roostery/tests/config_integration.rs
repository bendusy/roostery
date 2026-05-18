//! End-to-end integration tests for the `config` module.

use roostery::config::{self, Config};
use roostery::paths::TEST_ENV_LOCK as ENV_LOCK;

#[test]
fn roostery_home_override_drives_default_paths() {
    let home = tempfile::tempdir().unwrap();
    let _g = ENV_LOCK.lock().unwrap();
    // Safety: serialized via ENV_LOCK.
    unsafe { std::env::set_var("ROOSTERY_HOME", home.path()) };

    let mut cfg = Config::default();
    cfg.identity.user_id = "ou_e2e".into();
    cfg.identity.default_chat_id = "oc_e2e".into();
    cfg.budgets.default.max_calls = 42;
    cfg.budgets.default.max_cost_usd = 2.5;
    config::save(&cfg).unwrap();

    let expected_path = home.path().join("config.yaml");
    assert!(
        expected_path.exists(),
        "save() should respect ROOSTERY_HOME"
    );

    let loaded = config::load().unwrap();
    assert_eq!(loaded, cfg);

    unsafe { std::env::remove_var("ROOSTERY_HOME") };
}

#[test]
fn full_yaml_with_runners_round_trips() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("c.yaml");
    let yaml = r#"
schema_version: 1
identity:
  user_id: ou_full
  default_chat_id: oc_full
  default_task_app_token: bascn_full
runners:
  cc_headless:
    enabled: true
    cli_path: /opt/cc
    extra_args:
      - --foo
      - --bar
  codex_exec:
    enabled: false
budgets:
  default:
    max_calls: 200
    max_cost_usd: 5.0
trace:
  max_depth: 12
journal:
  dir: /custom/journal
  rotation: daily
"#;
    std::fs::write(&path, yaml).unwrap();

    let cfg = config::load_from(&path).unwrap();
    assert_eq!(cfg.identity.user_id, "ou_full");
    assert_eq!(cfg.runners.len(), 2);
    // B6: runners is now BTreeMap<String, RunnerConfig> — `enabled` strongly
    // typed, `cli_path` / `extra_args` preserved under `extra`.
    let cc = &cfg.runners["cc_headless"];
    assert!(cc.enabled);
    assert_eq!(
        cc.extra["cli_path"],
        serde_yml::Value::String("/opt/cc".into())
    );
    assert!(cc.extra["extra_args"].is_sequence());
    let codex = &cfg.runners["codex_exec"];
    assert!(!codex.enabled);
    assert_eq!(cfg.budgets.default.max_calls, 200);
    assert_eq!(cfg.trace.max_depth, 12);

    // Save back and confirm parse equivalence (YAML key ordering may differ
    // but parsed struct must be identical).
    let out_path = home.path().join("out.yaml");
    config::save_to(&cfg, &out_path).unwrap();
    let reloaded = config::load_from(&out_path).unwrap();
    assert_eq!(reloaded, cfg);
}
