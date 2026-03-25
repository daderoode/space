// Types introduced ahead of use — subsequent tasks wire these into screen handlers.
#![allow(dead_code)]

use crate::core::config::SpaceConfig;
use crate::core::workspace::{BranchStrategy, Workspace};
use std::path::PathBuf;

/// Read-only view of app state passed to screen handlers.
pub struct ScreenContext<'a> {
    pub config: &'a SpaceConfig,
    pub repos_cache: &'a [PathBuf],
    pub workspaces: &'a [Workspace],
    pub selected_ws: usize,
}

/// Parameters for creating/adding worktrees — shared between Create and Add flows.
pub struct WorktreeParams {
    pub workspace_name: String,
    pub workspace_dir: PathBuf,
    pub repos: Vec<PathBuf>,
    pub branch_strategy: BranchStrategy,
    pub is_new: bool,
}

/// Actions a screen handler can request from the app.
pub enum ScreenAction {
    /// Key handled internally, no app-level work needed.
    Continue,
    /// Return to Dashboard.
    Back,
    /// Return to Dashboard with a transient status message.
    BackWithStatus(String),
    /// Set cd target and quit (Go, Search).
    CdAndQuit(PathBuf),
    /// Quit without cd.
    Quit,
    /// Execute worktree creation/addition.
    ExecuteWorktreeFlow(WorktreeParams),
    /// Delete a workspace.
    DeleteWorkspace { name: String, force: bool },
    /// Save config and reload.
    SaveConfig(SpaceConfig),
    /// Navigate to a workspace by name (used by Search to select matching ws).
    NavigateToWorkspace(String),
}
