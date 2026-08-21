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

/// A git operation dispatched to the background git-ops worker.
/// `Fetch` streams git's progress; `Pull` summarizes the classify/merge result;
/// `Push { set_upstream }` publishes the current branch (with `-u origin
/// <branch>` when it has no upstream yet); `Rebase { onto }` replays the current
/// branch onto `onto`, auto-aborting on conflict.
///
/// Not `Copy`: `Rebase` carries an owned target, so call sites clone where they
/// need to keep a copy (see `GitOpsState::start_network_op`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitOp {
    Fetch,
    Pull,
    Push { set_upstream: bool },
    Rebase { onto: String },
}

impl GitOp {
    /// Display label for this op, used e.g. by the Running-stage
    /// completion/failure headers ("Rebase complete").
    pub fn label(&self) -> &'static str {
        match self {
            GitOp::Fetch => "Fetch",
            GitOp::Pull => "Pull",
            GitOp::Push { .. } => "Push",
            GitOp::Rebase { .. } => "Rebase",
        }
    }

    /// Progressive ("-ing") form for the Running-stage in-progress header.
    /// A dedicated table rather than `format!("{}ing", label)`: "Rebase" + "ing"
    /// would render as "Rebaseing".
    pub fn progressive(&self) -> &'static str {
        match self {
            GitOp::Fetch => "Fetching",
            GitOp::Pull => "Pulling",
            GitOp::Push { .. } => "Pushing",
            GitOp::Rebase { .. } => "Rebasing",
        }
    }
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
    /// Fetch + fast-forward the given repos, then transition to branch picker.
    ExecuteSyncFlow(Vec<std::path::PathBuf>),
    /// Run a git operation on a single repo via the background git-ops worker.
    ExecuteGitOp { repo_path: PathBuf, op: GitOp },
    /// Commit the staged changes of a single repo (synchronous local op).
    CommitRepo { repo_path: PathBuf, message: String },
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
    /// Switch an existing worktree to a different branch.
    SwitchRepoBranch {
        repo_path: PathBuf,
        branch: String,
        /// true = create new branch from current HEAD; false = checkout existing (local or remote)
        new_branch: bool,
    },
}
