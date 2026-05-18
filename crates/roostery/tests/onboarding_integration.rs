//! End-to-end integration tests for `roostery init` orchestrator
//! (`onboarding::run`).
//!
//! Tests serialize on `ENV_LOCK` because they manipulate `HOME` / `SHELL` /
//! `ROOSTERY_HOME` / `PATH`. Rust 2024 `set_var` is `unsafe`.

use roostery::hooks_merge::AgentKind;
use roostery::lark_cli::LarkError;
use roostery::lark_cli::mock::MockLarkRunner;
use roostery::onboarding::{self, InitOptions, SkipReason};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Pre-write a smoke state file declaring `all_ok = true` so `ensure_ready`
/// passes without spawning a real subprocess. `SmokeReport` is
/// `#[non_exhaustive]`, so we write JSON directly rather than via struct
/// literal.
fn seed_passing_smoke(home: &Path) {
    let state_dir = home.join("state");
    fs::create_dir_all(&state_dir).unwrap();
    let body = r#"{"schema_version":1,"binary":"/usr/bin/lark-cli","lark_cli_version":"1.0.28","started_at":"2026-05-18T00:00:00Z","all_ok":true,"probes":{}}"#;
    fs::write(state_dir.join("smoke.json"), body).unwrap();
}

fn seed_failing_smoke(home: &Path) {
    let state_dir = home.join("state");
    fs::create_dir_all(&state_dir).unwrap();
    let body = r#"{"schema_version":1,"binary":"/usr/bin/lark-cli","lark_cli_version":null,"started_at":"2026-05-18T00:00:00Z","all_ok":false,"probes":{"version":{"ok":false,"reason":"simulated"}}}"#;
    fs::write(state_dir.join("smoke.json"), body).unwrap();
}

/// Place a fake `shim` binary in the current_exe sibling directory so
/// `install_shim` can find it. Returns the source path (left in place for
/// later tests in the same binary; LBO is fine because tests serialize).
fn ensure_fake_shim_sibling() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap();
    let shim = dir.join("shim");
    // Magic bytes embedded so install_shim::looks_like_roostery_shim accepts it.
    let body = b"FAKE TEST SHIM ROOSTERY_REAL_LARK_CLI marker\n";
    fs::write(&shim, body).unwrap();
    shim
}

/// Seed a fake `lark-cli` binary in `bin_dir` so `which::which("lark-cli")`
/// can resolve to it. `bin_dir` is prepended to PATH by the caller.
fn ensure_fake_real_lark_cli(bin_dir: &Path) -> PathBuf {
    fs::create_dir_all(bin_dir).unwrap();
    let p = bin_dir.join("lark-cli");
    fs::write(&p, b"#!/bin/sh\necho fake\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&p, perms).unwrap();
    }
    p
}

/// Wire HOME, ROOSTERY_HOME, SHELL, PATH for an isolated init test.
struct TestEnv {
    _guard: std::sync::MutexGuard<'static, ()>,
    home: tempfile::TempDir,
    prev_home: Option<String>,
    prev_roostery: Option<String>,
    prev_shell: Option<String>,
    prev_path: Option<String>,
}

impl TestEnv {
    fn new() -> Self {
        let guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let prev_home = std::env::var("HOME").ok();
        let prev_roostery = std::env::var("ROOSTERY_HOME").ok();
        let prev_shell = std::env::var("SHELL").ok();
        let prev_path = std::env::var("PATH").ok();
        let fake_bin = home.path().join("realbin");
        ensure_fake_real_lark_cli(&fake_bin);
        let path_with_fake = format!(
            "{}:{}",
            fake_bin.display(),
            prev_path.as_deref().unwrap_or("")
        );
        unsafe {
            std::env::set_var("HOME", home.path());
            std::env::set_var("ROOSTERY_HOME", home.path().join(".roostery"));
            std::env::set_var("SHELL", "/bin/zsh");
            std::env::set_var("PATH", &path_with_fake);
        }
        Self {
            _guard: guard,
            home,
            prev_home,
            prev_roostery,
            prev_shell,
            prev_path,
        }
    }

    fn roostery_home(&self) -> PathBuf {
        self.home.path().join(".roostery")
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        unsafe {
            restore("HOME", self.prev_home.take());
            restore("ROOSTERY_HOME", self.prev_roostery.take());
            restore("SHELL", self.prev_shell.take());
            restore("PATH", self.prev_path.take());
        }
    }
}

unsafe fn restore(key: &str, prev: Option<String>) {
    unsafe {
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
}

/// Identity stub: enqueue happy auth-status + profile-list responses.
fn happy_identity_mock() -> MockLarkRunner {
    let mock = MockLarkRunner::new();
    mock.enqueue_ok(json!({
        "userOpenId": "ou_test123456",
        "userName": "test-user",
        "appId": "cli_app00001111",
        "brand": "lark",
        "tokenStatus": "valid",
    }));
    mock.enqueue_ok(json!([
        {"name": "default", "active": true},
    ]));
    mock
}

#[tokio::test]
async fn dry_run_passes_with_passing_smoke_and_does_not_write() {
    let env = TestEnv::new();
    seed_passing_smoke(&env.roostery_home());
    ensure_fake_shim_sibling();
    let mock = happy_identity_mock();

    let report = onboarding::run(
        &mock,
        InitOptions {
            dry_run: true,
            skip_agents: vec![],
            real_lark_cli_override: None,
        },
    )
    .await
    .expect("dry-run should succeed with passing smoke");

    assert!(report.dry_run);
    // No journal/scripts/state dirs created in dry-run (scripts/state did
    // exist because seed_passing_smoke wrote state; we check scripts only).
    assert!(!env.roostery_home().join("scripts").exists());
    // No env file written.
    assert!(!env.roostery_home().join("env").exists());
    // No shell rc patched.
    assert!(!env.home.path().join(".zshrc").exists());
    // No shim copied.
    assert!(!env.home.path().join(".local/bin/lark-cli").exists());
    assert_eq!(
        report.shell_rc_patched.as_ref().unwrap(),
        &env.home.path().join(".zshrc")
    );
}

#[tokio::test]
async fn smoke_never_run_aborts_without_writing() {
    let env = TestEnv::new();
    // intentionally no seed_passing_smoke → NeverRun
    ensure_fake_shim_sibling();
    let mock = happy_identity_mock();

    let result = onboarding::run(
        &mock,
        InitOptions {
            dry_run: false,
            skip_agents: vec![],
            real_lark_cli_override: None,
        },
    )
    .await;

    assert!(result.is_err(), "must error when smoke never ran");
    // Filesystem must be untouched.
    assert!(!env.home.path().join(".local/bin/lark-cli").exists());
    assert!(!env.roostery_home().join("scripts").exists());
    assert!(!env.home.path().join(".zshrc").exists());
}

/// S4 new test: resolve fails (OverrideInvalid) AFTER smoke gate → zero
/// filesystem side effects (no shim copied / no scripts written / no hook
/// merged / no env file / no shell rc patched). Verifies the early-gate
/// position of resolve_real_lark_cli (design §2.2 + §3 E3).
#[tokio::test]
async fn resolve_fail_leaves_zero_side_effects() {
    let env = TestEnv::new();
    seed_passing_smoke(&env.roostery_home());
    ensure_fake_shim_sibling();
    let mock = happy_identity_mock();

    let result = onboarding::run(
        &mock,
        InitOptions {
            dry_run: false,
            skip_agents: vec![],
            real_lark_cli_override: Some(PathBuf::from("/definitely/nowhere/lark-cli")),
        },
    )
    .await;

    assert!(result.is_err(), "OverrideInvalid expected");
    // Filesystem totally untouched outside seed (smoke.json was seeded
    // before calling run; everything else must be absent).
    assert!(!env.home.path().join(".local/bin/lark-cli").exists());
    assert!(!env.roostery_home().join("scripts").exists());
    assert!(!env.roostery_home().join("journal").exists());
    assert!(!env.roostery_home().join("env").exists());
    assert!(!env.home.path().join(".zshrc").exists());
}

#[tokio::test]
async fn smoke_last_failed_aborts() {
    let env = TestEnv::new();
    seed_failing_smoke(&env.roostery_home());
    ensure_fake_shim_sibling();
    let mock = happy_identity_mock();

    let result = onboarding::run(
        &mock,
        InitOptions {
            dry_run: false,
            skip_agents: vec![],
            real_lark_cli_override: None,
        },
    )
    .await;

    assert!(result.is_err());
    assert!(!env.home.path().join(".zshrc").exists());
}

#[tokio::test]
async fn full_install_writes_expected_files_and_is_idempotent() {
    let env = TestEnv::new();
    seed_passing_smoke(&env.roostery_home());
    ensure_fake_shim_sibling();

    // First run.
    let mock = happy_identity_mock();
    let report1 = onboarding::run(
        &mock,
        InitOptions {
            dry_run: false,
            // Skip cc/codex/gemini so we don't accidentally hit real ~/.claude
            // installs on the host (and there is no `claude` binary on PATH
            // in CI anyway, so they'd be NotInstalled).
            skip_agents: vec![AgentKind::Cc, AgentKind::Codex, AgentKind::Gemini],
            real_lark_cli_override: None,
        },
    )
    .await
    .expect("happy path must succeed");

    assert!(!report1.dry_run);
    assert_eq!(
        report1.shim_path,
        env.home.path().join(".local/bin/lark-cli")
    );
    assert!(env.home.path().join(".local/bin/lark-cli").exists());
    assert!(
        env.roostery_home()
            .join("scripts/agent_stop_notify.sh")
            .exists()
    );
    assert!(env.roostery_home().join("env").exists());
    let env_body = fs::read_to_string(env.roostery_home().join("env")).unwrap();
    assert!(env_body.contains("export ROOSTERY_REAL_LARK_CLI="));
    assert!(env.home.path().join(".zshrc").exists());
    let rc_body = fs::read_to_string(env.home.path().join(".zshrc")).unwrap();
    assert!(rc_body.contains("# >>> roostery >>>"));
    assert!(rc_body.contains("# <<< roostery <<<"));
    assert!(report1.agents_installed.is_empty());
    assert_eq!(report1.agents_skipped.len(), 3);
    for (_, reason) in &report1.agents_skipped {
        assert!(matches!(
            reason,
            SkipReason::UserSkipped | SkipReason::NotInstalled
        ));
    }

    // Second run — full idempotency.
    let mock2 = happy_identity_mock();
    let report2 = onboarding::run(
        &mock2,
        InitOptions {
            dry_run: false,
            skip_agents: vec![AgentKind::Cc, AgentKind::Codex, AgentKind::Gemini],
            real_lark_cli_override: None,
        },
    )
    .await
    .expect("second run must also succeed");

    assert!(!report2.dry_run);
    let rc_body_2 = fs::read_to_string(env.home.path().join(".zshrc")).unwrap();
    assert_eq!(
        rc_body, rc_body_2,
        "rc must be byte-for-byte identical after re-run"
    );
    let env_body_2 = fs::read_to_string(env.roostery_home().join("env")).unwrap();
    assert_eq!(env_body, env_body_2);
}

#[tokio::test]
async fn identity_failure_does_not_abort_install() {
    let env = TestEnv::new();
    seed_passing_smoke(&env.roostery_home());
    ensure_fake_shim_sibling();

    // Mock returns LarkError for auth status → identity propagates as warning,
    // does not abort.
    let mock = MockLarkRunner::new();
    mock.enqueue_err(LarkError::Timeout { timeout_ms: 1 });

    let report = onboarding::run(
        &mock,
        InitOptions {
            dry_run: false,
            skip_agents: vec![AgentKind::Cc, AgentKind::Codex, AgentKind::Gemini],
            real_lark_cli_override: None,
        },
    )
    .await
    .expect("identity failure must NOT abort");

    assert!(report.identity.is_none());
    assert!(report.identity_error.is_some());
    assert!(env.home.path().join(".local/bin/lark-cli").exists());
}

// ============================================================================
// S6: feature 2026-05-18-init-real-lark-cli-override e2e CLI tests
// ============================================================================

/// E2E #1: --real-lark-cli flag → init 跑通；env 文件指向 override；source=Flag
#[tokio::test]
async fn override_flag_happy_writes_env_pointing_to_override_path() {
    let env = TestEnv::new();
    seed_passing_smoke(&env.roostery_home());
    ensure_fake_shim_sibling();
    // 制造一个不在 PATH 上的 fake real lark-cli（区分 TestEnv 默认那个）
    let extra_dir = tempfile::tempdir().unwrap();
    let override_path = ensure_fake_real_lark_cli(extra_dir.path());
    let mock = happy_identity_mock();

    let report = onboarding::run(
        &mock,
        InitOptions {
            dry_run: false,
            skip_agents: vec![AgentKind::Cc, AgentKind::Codex, AgentKind::Gemini],
            real_lark_cli_override: Some(override_path.clone()),
        },
    )
    .await
    .expect("flag override happy path");

    assert_eq!(report.real_lark_cli, override_path);
    assert_eq!(
        report.real_lark_cli_source,
        onboarding::RealLarkCliSource::Flag
    );
    let env_body = fs::read_to_string(env.roostery_home().join("env")).unwrap();
    assert!(
        env_body.contains(&format!(
            "export ROOSTERY_REAL_LARK_CLI={}",
            override_path.display()
        )),
        "env file should contain override path; got: {env_body}"
    );
}

/// E2E #2: ROOSTERY_LARK_CLI_BIN env → init 跑通；source=Env
#[tokio::test]
async fn override_env_happy_uses_env_value() {
    let env = TestEnv::new();
    seed_passing_smoke(&env.roostery_home());
    ensure_fake_shim_sibling();
    let extra_dir = tempfile::tempdir().unwrap();
    let env_target = ensure_fake_real_lark_cli(extra_dir.path());

    let prev_bin_env = std::env::var("ROOSTERY_LARK_CLI_BIN").ok();
    unsafe { std::env::set_var("ROOSTERY_LARK_CLI_BIN", &env_target) };

    let mock = happy_identity_mock();
    let report = onboarding::run(
        &mock,
        InitOptions {
            dry_run: false,
            skip_agents: vec![AgentKind::Cc, AgentKind::Codex, AgentKind::Gemini],
            real_lark_cli_override: None,
        },
    )
    .await
    .expect("env override happy path");

    assert_eq!(report.real_lark_cli, env_target);
    assert_eq!(
        report.real_lark_cli_source,
        onboarding::RealLarkCliSource::Env
    );

    // Restore env
    unsafe {
        match prev_bin_env {
            Some(v) => std::env::set_var("ROOSTERY_LARK_CLI_BIN", v),
            None => std::env::remove_var("ROOSTERY_LARK_CLI_BIN"),
        }
    }
}

/// E2E #3: PATH 唯一候选 = shim target → LarkCliCollidesShimTarget + 零副作用
#[tokio::test]
async fn collision_returns_error_and_leaves_zero_side_effects() {
    let env = TestEnv::new();
    seed_passing_smoke(&env.roostery_home());
    ensure_fake_shim_sibling();

    // 在 TestEnv 的 HOME/.local/bin/ 放一个 fake lark-cli（即 shim target 路径）。
    // 并把 PATH 完全替换成那一个目录，让 which 只返还这一个候选 = shim_target → collision。
    let shim_target_dir = env.home.path().join(".local/bin");
    let shim_target = ensure_fake_real_lark_cli(&shim_target_dir);
    unsafe { std::env::set_var("PATH", &shim_target_dir) };

    let mock = happy_identity_mock();
    let result = onboarding::run(
        &mock,
        InitOptions {
            dry_run: false,
            skip_agents: vec![AgentKind::Cc, AgentKind::Codex, AgentKind::Gemini],
            real_lark_cli_override: None,
        },
    )
    .await;

    assert!(result.is_err(), "expected LarkCliCollidesShimTarget");
    // 重要：fake lark-cli **预先存在**（我们刚写的）；零副作用断言意指 init 没有把 shim 装上去
    // 覆盖那个 fake。验证文件内容仍是 fake 的 `#!/bin/sh\necho fake\n`，不是 shim binary
    let body = fs::read_to_string(&shim_target).unwrap();
    assert!(
        body.contains("echo fake"),
        "fake lark-cli content must remain (shim must NOT have overwritten it); got: {body:?}"
    );
    // 其他 init 产物均未生成
    assert!(!env.roostery_home().join("scripts").exists());
    assert!(!env.roostery_home().join("env").exists());
    assert!(!env.home.path().join(".zshrc").exists());
}
