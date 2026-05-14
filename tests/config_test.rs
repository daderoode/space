use space::core::config::SpaceConfig;
use std::path::PathBuf;
use std::sync::Mutex;
use tempfile::TempDir;

/// Serialise tests that read or write SPACE_CONFIG_DIR so they never run
/// concurrently.  Rust's test harness runs tests in parallel by default;
/// without this lock, config_path_is_under_config_dir can observe the temp
/// path set by config_dir_respects_space_config_dir_env and fail the
/// `ends_with("space/config.toml")` assertion.
static ENV_LOCK: Mutex<()> = Mutex::new(());

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
    let _guard = ENV_LOCK.lock().unwrap();
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
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    std::env::set_var("SPACE_CONFIG_DIR", tmp.path());
    let dir = SpaceConfig::config_dir();
    std::env::remove_var("SPACE_CONFIG_DIR");
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
