#![allow(clippy::disallowed_methods)] // fixtures start git directly (ADR 0002)

// Every test binary declares `common`, whose `git_isolation` keeps it off the
// invoking user's git config before `main`; this one runs no git today.
mod common;

use space::core::config::SpaceConfig;
use std::path::PathBuf;
use std::sync::Mutex;
use tempfile::TempDir;

/// Serialise tests that read or write SPACE_CONFIG_DIR so they never run
/// concurrently.  Rust's test harness runs tests in parallel by default;
/// without this lock, config_path_is_under_config_dir can observe the temp
/// path set by config_dir_respects_space_config_dir_env and fail the
/// `ends_with("space/config.toml")` assertion. Taken through `env_lock`.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Take `ENV_LOCK`, recovering the guard if a previous holder panicked (the
/// shape of `core::spawn::enter`): a `std::sync::Mutex` poisons when a
/// holder panics, and a bare `unwrap()` would fail every later holder in
/// this binary for one failing assertion. The lock guards no data, only the
/// order of the tests. Every site takes the lock through here, so
/// `a_panic_under_the_env_lock_fails_only_its_own_test` covers them all.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Removes `SPACE_CONFIG_DIR` when dropped, so a panic between `set_var` and
/// the removal (an unwind under `ENV_LOCK`) cannot leak the temp path into
/// the next holder's `config_path_is_under_config_dir` assertion.
struct EnvGuard;
impl Drop for EnvGuard {
    fn drop(&mut self) {
        std::env::remove_var("SPACE_CONFIG_DIR");
    }
}

#[test]
fn default_config_has_reasonable_values() {
    let cfg = SpaceConfig::default();
    assert!(!cfg.repos.roots.is_empty(), "roots must not be empty");
    assert!(cfg.repos.max_depth > 0);
    assert!(cfg.repos.cache_age_secs > 0);
    assert!(!cfg.workspaces.dir.as_os_str().is_empty());
}

#[test]
fn loads_from_toml_string() {
    let toml = r#"
[repos]
roots = ["/tmp/test-repos"]
max_depth = 2
cache_age_secs = 1800

[workspaces]
dir = "/tmp/test-workspaces"
"#;
    let cfg: SpaceConfig = toml::from_str(toml).unwrap();
    assert_eq!(cfg.repos.roots, vec![PathBuf::from("/tmp/test-repos")]);
    assert_eq!(cfg.repos.max_depth, 2);
    assert_eq!(cfg.repos.cache_age_secs, 1800);
    assert_eq!(cfg.workspaces.dir, PathBuf::from("/tmp/test-workspaces"));
}

#[test]
fn config_path_is_under_config_dir() {
    let _guard = env_lock();
    let path = SpaceConfig::config_path();
    assert!(
        path.ends_with("space/config.toml"),
        "expected path ending in space/config.toml, got: {path:?}"
    );
}

#[test]
fn config_dir_respects_space_config_dir_env() {
    // ENV_LOCK serialises this test with config_path_is_under_config_dir so
    // the set_var/remove_var pair does not race with the assertion there.
    let _guard = env_lock();
    let tmp = TempDir::new().unwrap();
    std::env::set_var("SPACE_CONFIG_DIR", tmp.path());
    let _env_guard = EnvGuard;
    let dir = SpaceConfig::config_dir();
    assert_eq!(dir, tmp.path());
}

/// Exercises the full save → disk → load round-trip through the filesystem.
/// Uses the same logic as `save()` and `load()` but with a temp dir path,
/// avoiding the process-global env var race that `set_var` causes.
#[test]
fn config_save_load_round_trip() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");

    let original = SpaceConfig {
        repos: space::core::config::RepoConfig {
            roots: vec![PathBuf::from("/test/repos")],
            max_depth: 5,
            cache_age_secs: 1800,
        },
        workspaces: space::core::config::WorkspaceConfig {
            dir: PathBuf::from("/test/workspaces"),
        },
    };

    // Same logic as save(): serialize to TOML, write to disk
    std::fs::write(&config_path, toml::to_string_pretty(&original).unwrap()).unwrap();

    // Same logic as load(): read from disk, deserialize
    let content = std::fs::read_to_string(&config_path).unwrap();
    let loaded: SpaceConfig = toml::from_str(&content).unwrap();

    assert_eq!(loaded, original);
}

/// One test panicking under `ENV_LOCK` must not fail the tests that lock it
/// next. The guard drops while the thread is panicking, which is what poisons
/// a `std::sync::Mutex`; the second acquisition then only succeeds because
/// `env_lock` recovers the guard from the poison instead of unwrapping.
/// Every site takes the lock through `env_lock`, so this one call covers
/// them all whatever order the scheduler picks.
///
/// Recovering the guard does not clear the poison, so from this test on the
/// lock stays poisoned for the rest of the binary; a site that bypasses
/// `env_lock` with a bare `unwrap()` fails whenever it runs after this one.
#[test]
fn a_panic_under_the_env_lock_fails_only_its_own_test() {
    let outcome = std::panic::catch_unwind(|| {
        let _guard = env_lock();
        panic!("deliberate panic while holding ENV_LOCK");
    });
    assert!(outcome.is_err(), "the body must have panicked");
    let _guard = env_lock();
}
