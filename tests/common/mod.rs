#![allow(dead_code)]

use space::core::config::SpaceConfig;
use space::core::workspace::Workspace;
use space::tui::app::{App, Pane, Screen};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// Initialise a git repo at `dir` with branch `main`, user config, and an empty commit.
/// Extracted from duplicated helpers in git_test.rs and workspace_test.rs.
pub fn init_repo(dir: &Path) {
    let out = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = Command::new("git")
        .args(["config", "user.email", "space@local"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git config user.email failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git config user.name failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = Command::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Self-contained test environment with isolated config, repos, and workspaces directories.
///
/// Creates a TempDir with three subdirectories (`config/`, `repos/`, `workspaces/`)
/// and writes a `config.toml` pointing at the temp dirs.
///
/// Does NOT set env vars automatically -- callers decide.
pub struct TestEnv {
    pub dir: TempDir,
    pub config_dir: PathBuf,
    pub repos_dir: PathBuf,
    pub workspaces_dir: PathBuf,
}

impl TestEnv {
    pub fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("config");
        let repos_dir = dir.path().join("repos");
        let workspaces_dir = dir.path().join("workspaces");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&repos_dir).unwrap();
        std::fs::create_dir_all(&workspaces_dir).unwrap();

        // Write config.toml pointing at the temp dirs
        let config_toml = format!(
            r#"[repos]
roots = ["{}"]
max_depth = 3
cache_age_secs = 3600

[workspaces]
dir = "{}"
"#,
            repos_dir.display(),
            workspaces_dir.display(),
        );
        std::fs::write(config_dir.join("config.toml"), config_toml).unwrap();

        Self {
            dir,
            config_dir,
            repos_dir,
            workspaces_dir,
        }
    }

    /// Create a git repo under repos_dir/<name> and return its path.
    pub fn create_repo(&self, name: &str) -> PathBuf {
        let repo_path = self.repos_dir.join(name);
        std::fs::create_dir_all(&repo_path).unwrap();
        init_repo(&repo_path);
        repo_path
    }

    /// Write a repos.cache file with the given repo paths (newline-delimited,
    /// matching the format used by `repo::save_cache` / `repo::load_cache`).
    pub fn write_cache(&self, repos: &[PathBuf]) {
        let cache_path = self.config_dir.join("repos.cache");
        let content = repos
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(cache_path, content).unwrap();
    }
}

/// Construct an App with explicit fields, bypassing `App::new()`.
/// Same pattern as `app_with_status()` in `src/tui/app.rs`.
pub fn test_app(workspaces: Vec<Workspace>, repos_cache: Vec<PathBuf>) -> App {
    App {
        config: SpaceConfig::default(),
        workspaces,
        repos_cache,
        selected_ws: 0,
        selected_repo: 0,
        expanded_repos: std::collections::HashSet::new(),
        repo_file_cache: std::collections::HashMap::new(),
        cursor_row: 0,
        diff_target: space::core::git::DiffTarget::Base,
        focus: Pane::Left,
        screen: Screen::Dashboard,
        should_quit: false,
        space_cd_target: None,
        status_message: None,
        status_message_set_at: None,
    }
}

/// Construct an App with a custom config, bypassing `App::new()`.
/// Used for tests that need real filesystem paths (create/add flows).
pub fn test_app_with_config(
    config: SpaceConfig,
    workspaces: Vec<Workspace>,
    repos_cache: Vec<PathBuf>,
) -> App {
    App {
        config,
        workspaces,
        repos_cache,
        selected_ws: 0,
        selected_repo: 0,
        expanded_repos: std::collections::HashSet::new(),
        repo_file_cache: std::collections::HashMap::new(),
        cursor_row: 0,
        diff_target: space::core::git::DiffTarget::Base,
        focus: Pane::Left,
        screen: Screen::Dashboard,
        should_quit: false,
        space_cd_target: None,
        status_message: None,
        status_message_set_at: None,
    }
}

/// Build a Workspace with named repos, all pointing at `/tmp/test-ws/<name>`.
/// Used for testing flattened_rows, cursor navigation, and expand/collapse.
pub fn workspace_with_repos(names: &[&str]) -> space::core::workspace::Workspace {
    use space::core::git::RepoStatus;
    use space::core::workspace::{Workspace, WorkspaceRepo};
    Workspace {
        name: "test-ws".into(),
        path: std::path::PathBuf::from("/tmp/test-ws"),
        repos: names
            .iter()
            .map(|&name| WorkspaceRepo {
                name: name.into(),
                path: std::path::PathBuf::from(format!("/tmp/test-ws/{}", name)),
                branch: "main".into(),
                status: RepoStatus::default(),
                ahead: 0,
                behind: 0,
            })
            .collect(),
    }
}

/// Helper to construct a KeyEvent with NONE modifiers and Press kind.
pub fn key(code: ratatui::crossterm::event::KeyCode) -> ratatui::crossterm::event::KeyEvent {
    ratatui::crossterm::event::KeyEvent::new(code, ratatui::crossterm::event::KeyModifiers::NONE)
}

/// Construct a KeyEvent for Shift+<key> (e.g. Shift+T = Char('T')).
pub fn shift_key(code: ratatui::crossterm::event::KeyCode) -> ratatui::crossterm::event::KeyEvent {
    ratatui::crossterm::event::KeyEvent::new(code, ratatui::crossterm::event::KeyModifiers::SHIFT)
}
