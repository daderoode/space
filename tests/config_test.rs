use space::core::config::SpaceConfig;
use std::path::PathBuf;

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
}

#[test]
fn config_path_is_under_config_dir() {
    let path = SpaceConfig::config_path();
    assert!(path.ends_with("space/config.toml"));
}
