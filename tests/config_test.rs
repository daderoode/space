use space::core::config::SpaceConfig;
use std::path::PathBuf;
use tempfile::TempDir;

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
fn round_trips_through_toml() {
    let original = SpaceConfig::default();
    let serialized = toml::to_string_pretty(&original).unwrap();
    let restored: SpaceConfig = toml::from_str(&serialized).unwrap();
    assert_eq!(original.repos.max_depth, restored.repos.max_depth);
    assert_eq!(original.repos.roots, restored.repos.roots);
    assert_eq!(original.repos.cache_age_secs, restored.repos.cache_age_secs);
    assert_eq!(original.workspaces.dir, restored.workspaces.dir);
}

#[test]
fn config_path_is_under_config_dir() {
    let path = SpaceConfig::config_path();
    assert!(path.ends_with("space/config.toml"));
}

// NOTE: set_var/remove_var are process-global. This test could flake if run
// concurrently with other tests that depend on config_dir() returning the
// real path. Currently safe because no other test does this.
#[test]
fn config_dir_respects_space_config_dir_env() {
    let tmp = TempDir::new().unwrap();
    std::env::set_var("SPACE_CONFIG_DIR", tmp.path());
    let dir = SpaceConfig::config_dir();
    std::env::remove_var("SPACE_CONFIG_DIR");
    assert_eq!(dir, tmp.path());
}
