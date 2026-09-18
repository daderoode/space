use crate::core::config::SpaceConfig;
use crate::core::workspace::{BranchStrategy, PreCreateFetch, UNATTENDED_FETCH_TIMEOUT};
use std::path::{Path, PathBuf};

/// Read-only view of app state passed to screen handlers.
///
/// Add fields here as screens need them (e.g. `workspaces`, `repos_cache`).
/// The split-borrow pattern in `App::handle_key` ensures disjoint access.
pub struct ScreenContext<'a> {
    pub config: &'a SpaceConfig,
    /// `App::create_job.is_some()`: the single source of truth for whether
    /// the Creating stage's background worker is running. The Creating log's
    /// key handling turns on it, so Esc stops a run in flight and leaves
    /// rather than being read as "leave a finished run".
    pub creating_in_flight: bool,
}

/// Parameters for creating/adding worktrees — shared between Create and Add flows.
#[derive(Clone)]
pub struct WorktreeParams {
    pub workspace_name: String,
    pub workspace_dir: PathBuf,
    pub repos: Vec<PathBuf>,
    pub branch_strategy: BranchStrategy,
    pub is_new: bool,
    /// Repos the sync report fetched successfully in this flow. Their refs
    /// are the remote's, so the worktree creation skips its own fetch and
    /// says nothing about it.
    pub fresh_repos: Vec<PathBuf>,
    /// Repos whose sync fetch ran the whole limit without the remote
    /// answering. The creation skips its fetch for these too, because
    /// another attempt would very likely spend that limit again to learn the
    /// same nothing, but it says so in the log: unlike `fresh_repos` these
    /// refs are of unknown age. See `SyncReport::timed_out_paths` for what
    /// this does and does not catch.
    ///
    /// A repo in neither list is fetched: two empty lists (no sync ran) are
    /// the safe default.
    pub unreachable_repos: Vec<PathBuf>,
}

impl WorktreeParams {
    /// The pre-create fetch for `repo`: skipped when the sync already
    /// fetched it (refs are the remote's) or when the sync's own fetch of
    /// that remote timed out (another attempt would very likely spend the
    /// whole limit again to learn the same nothing).
    ///
    /// A method rather than a rule the caller re-derives, so the Creating
    /// worker and the App cannot drift on the two skips that shipped in
    /// PR #28.
    pub fn pre_create_fetch(&self, repo: &Path) -> PreCreateFetch {
        if self.skipped_after_timeout(repo) || self.fresh_repos.iter().any(|p| p == repo) {
            PreCreateFetch::Skip
        } else {
            PreCreateFetch::Run(UNATTENDED_FETCH_TIMEOUT)
        }
    }

    /// True when this repo's skip is the kind the log must mention: unlike
    /// a skip for freshness, these refs are of unknown age.
    pub fn skipped_after_timeout(&self, repo: &Path) -> bool {
        self.unreachable_repos.iter().any(|p| p == repo)
    }
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
    /// Open the help overlay on top of the current screen, which is left
    /// exactly as it is. Not `Back`: that resets to the Dashboard.
    OpenHelp,
    /// Return to Dashboard with a transient status message.
    BackWithStatus(String, StatusKind),
    /// Set cd target and quit (used by Go).
    CdAndQuit(PathBuf),
    /// Execute worktree creation/addition.
    ExecuteWorktreeFlow(WorktreeParams),
    /// Stop the Creating stage's worker at its next boundary and return to
    /// the dashboard, leaving whatever worktrees it had already made.
    CancelCreating,
    /// Fetch + fast-forward the given repos, reporting into the sync report.
    ExecuteSyncFlow(Vec<std::path::PathBuf>),
    /// Rescan the repo list and rebuild the open repo picker from it
    /// (Ctrl-R in the PickRepos stage of the create and add flows).
    RescanRepoList,

    /// Enter on a finished sync report: load recent branches and move to the
    /// branch picker.
    ContinueFromSyncReport,
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
    /// Select the workspace at this index in place (space filter). Handled
    /// like the repo-search landing: reset and reload the repos pane, focus
    /// unchanged. A no-op when the index is already selected.
    SelectWorkspace(usize),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn params(fresh: Vec<PathBuf>, unreachable: Vec<PathBuf>) -> WorktreeParams {
        WorktreeParams {
            workspace_name: "ws".to_string(),
            workspace_dir: PathBuf::from("/ws"),
            repos: vec![],
            branch_strategy: BranchStrategy::DetachedHead,
            is_new: true,
            fresh_repos: fresh,
            unreachable_repos: unreachable,
        }
    }

    #[test]
    fn pre_create_fetch_skips_only_fetched_and_timed_out_repos() {
        let fresh = PathBuf::from("/r/fresh");
        let slow = PathBuf::from("/r/slow");
        let p = params(vec![fresh.clone()], vec![slow.clone()]);

        assert_eq!(
            p.pre_create_fetch(&fresh),
            PreCreateFetch::Skip,
            "the sync already fetched this repo, so its refs are the remote's"
        );
        assert_eq!(
            p.pre_create_fetch(&slow),
            PreCreateFetch::Skip,
            "a remote that took the whole limit is not asked again here"
        );
        assert_eq!(
            p.pre_create_fetch(Path::new("/r/other")),
            PreCreateFetch::Run(UNATTENDED_FETCH_TIMEOUT),
            "a repo in neither list is fetched under the unattended-run limit"
        );
        assert_eq!(
            params(vec![], vec![]).pre_create_fetch(&fresh),
            PreCreateFetch::Run(UNATTENDED_FETCH_TIMEOUT),
            "two empty lists (no sync ran) are the safe default"
        );
    }

    #[test]
    fn only_a_timed_out_skip_is_worth_a_log_line() {
        let fresh = PathBuf::from("/r/fresh");
        let slow = PathBuf::from("/r/slow");
        let p = params(vec![fresh.clone()], vec![slow.clone()]);

        assert!(
            p.skipped_after_timeout(&slow),
            "these refs are of unknown age, so the log must say so"
        );
        assert!(
            !p.skipped_after_timeout(&fresh),
            "a skip for freshness is silent: the refs are the remote's"
        );
        assert!(!p.skipped_after_timeout(Path::new("/r/other")));
    }
}
