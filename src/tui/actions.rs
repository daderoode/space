use crate::core::config::SpaceConfig;
use crate::core::workspace::BranchStrategy;
use std::path::PathBuf;

/// Read-only view of app state passed to screen handlers.
///
/// Add fields here as screens need them (e.g. `workspaces`, `repos_cache`).
/// The split-borrow pattern in `App::handle_key` ensures disjoint access.
pub struct ScreenContext<'a> {
    pub config: &'a SpaceConfig,
}

/// Parameters for creating/adding worktrees — shared between Create and Add flows.
pub struct WorktreeParams {
    pub workspace_name: String,
    pub workspace_dir: PathBuf,
    pub repos: Vec<PathBuf>,
    pub branch_strategy: BranchStrategy,
    pub is_new: bool,
}

/// Severity of a transient status message shown in the status bar.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum StatusKind {
    Error,
    Success,
    Warning,
    #[default]
    Info,
}

/// Actions a screen handler can request from the app.
pub enum ScreenAction {
    /// Key handled internally, no app-level work needed.
    Continue,
    /// Return to Dashboard.
    Back,
    /// Return to Dashboard with a transient status message.
    BackWithStatus(String, StatusKind),
    /// Set cd target and quit (used by Go).
    CdAndQuit(PathBuf),
    /// Execute worktree creation/addition.
    ExecuteWorktreeFlow(WorktreeParams),
    /// Delete a workspace.
    DeleteWorkspace { name: String, force: bool },
    /// Save config and reload.
    SaveConfig(SpaceConfig),
    /// Navigate to the workspace containing a repo with the given name
    /// (repo name from Search, resolved to a workspace in `App::process_action`).
    NavigateToWorkspace(String),
    /// Stage or unstage a single file from the diff overlay.
    StageFile {
        repo_index: usize,
        repo_path: PathBuf,
        path: String,
        currently_staged: bool,
    },
}
