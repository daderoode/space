use crate::tui::actions::{GitOp, ScreenAction, ScreenContext};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::path::PathBuf;

/// Stage of the git-operations overlay. Phase 1 ships only the action menu;
/// later phases add Committing / Log / Running / ConfirmPush.
#[derive(Debug, Clone, PartialEq)]
pub enum GitOpsStage {
    Menu,
    /// A network op (fetch / pull / push) is running, showing live output lines.
    Running,
    /// Confirm publishing a branch that has no upstream yet (push -u origin).
    ConfirmPush,
}

pub struct GitOpsState {
    pub stage: GitOpsStage,
    pub repo_name: String,
    pub repo_path: PathBuf,
    pub branch: String,
    pub selected: usize,
    pub has_staged: bool,
    /// Whether the current branch already has a configured upstream. Drives the
    /// push routing: plain push when true, ConfirmPush (set upstream) when false.
    pub has_upstream: bool,
    pub status: Option<String>,
    /// Label of the network op currently running ("Fetch"/"Pull"/"Push"),
    /// used by the Running-stage header. Empty when no op has run.
    pub op_label: &'static str,
    /// Live output lines captured while a Running network op streams.
    pub output: Vec<String>,
    /// None while running; `Some(success)` once the op completes.
    pub finished: Option<bool>,
    /// When set (success only), the overlay auto-closes at this instant.
    pub close_at: Option<std::time::Instant>,
}

impl std::fmt::Debug for GitOpsState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitOpsState")
            .field("stage", &self.stage)
            .field("repo_name", &self.repo_name)
            .field("branch", &self.branch)
            .field("selected", &self.selected)
            .finish()
    }
}

impl GitOpsState {
    /// Build the menu state for a repo. Panic-safe on non-existent paths:
    /// `current_branch` and `file_diff` failures degrade to "?" / no-staged.
    pub fn new(repo_name: String, repo_path: PathBuf) -> Self {
        let branch =
            crate::core::git::current_branch(&repo_path).unwrap_or_else(|_| "?".to_string());
        let has_staged = crate::core::git::file_diff(&repo_path)
            .map(|v| v.iter().any(|e| e.staged))
            .unwrap_or(false);
        let has_upstream = crate::core::git::has_upstream(&repo_path);
        Self {
            stage: GitOpsStage::Menu,
            repo_name,
            repo_path,
            branch,
            selected: 0,
            has_staged,
            has_upstream,
            status: None,
            op_label: "",
            output: Vec::new(),
            finished: None,
            close_at: None,
        }
    }

    /// Highest selectable menu index (six items, 0..=5).
    const MAX_IDX: usize = 5;

    pub fn handle_key(&mut self, key: KeyEvent, _ctx: &ScreenContext) -> ScreenAction {
        match self.stage {
            GitOpsStage::Running => match key.code {
                // Close early; while running other keys are no-ops.
                KeyCode::Esc => ScreenAction::Back,
                _ => ScreenAction::Continue,
            },
            GitOpsStage::ConfirmPush => self.handle_confirm_push_key(key),
            GitOpsStage::Menu => self.handle_menu_key(key),
        }
    }

    /// Handle keys in the ConfirmPush stage (branch has no upstream).
    /// `y`/Enter confirms and pushes with `-u origin <branch>`.
    fn handle_confirm_push_key(&mut self, key: KeyEvent) -> ScreenAction {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                self.start_network_op(GitOp::Push { set_upstream: true })
            }
            // Decline: back to the menu without touching the remote.
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                self.stage = GitOpsStage::Menu;
                ScreenAction::Continue
            }
            _ => ScreenAction::Continue,
        }
    }

    fn handle_menu_key(&mut self, key: KeyEvent) -> ScreenAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => ScreenAction::Back,
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                ScreenAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected < Self::MAX_IDX {
                    self.selected += 1;
                }
                ScreenAction::Continue
            }
            KeyCode::Enter => self.fire(self.selected),
            KeyCode::Char('f') => self.fire(0),
            KeyCode::Char('p') => self.fire(1),
            KeyCode::Char('P') => self.fire(2),
            KeyCode::Char('c') => self.fire(3),
            KeyCode::Char('l') => self.fire(4),
            KeyCode::Char('r') => self.fire(5),
            _ => ScreenAction::Continue,
        }
    }

    /// Fire the menu item at `idx`, moving the highlight to it. Fetch and pull
    /// stand up the async Running stage; the other actions keep their Phase-1
    /// placeholder status until their own phases land.
    fn fire(&mut self, idx: usize) -> ScreenAction {
        self.selected = idx;
        match idx {
            0 => self.start_network_op(GitOp::Fetch),
            1 => self.start_network_op(GitOp::Pull),
            2 => {
                // With an upstream, push straight away; otherwise confirm before
                // publishing the branch (push -u origin <branch>).
                if self.has_upstream {
                    self.start_network_op(GitOp::Push { set_upstream: false })
                } else {
                    self.stage = GitOpsStage::ConfirmPush;
                    self.status = None;
                    ScreenAction::Continue
                }
            }
            3 if !self.has_staged => {
                self.status = Some("Stage files first with s/S".to_string());
                ScreenAction::Continue
            }
            3 => {
                self.status = Some("Commit: not yet implemented".to_string());
                ScreenAction::Continue
            }
            4 => {
                self.status = Some("Log: not yet implemented".to_string());
                ScreenAction::Continue
            }
            5 => {
                self.status = Some("Rebase coming soon (item 7)".to_string());
                ScreenAction::Continue
            }
            _ => ScreenAction::Continue,
        }
    }

    /// Reset the run buffers, enter the Running stage, and dispatch `op` to the
    /// background git-ops worker.
    fn start_network_op(&mut self, op: GitOp) -> ScreenAction {
        self.op_label = match op {
            GitOp::Fetch => "Fetch",
            GitOp::Pull => "Pull",
            GitOp::Push { .. } => "Push",
        };
        self.stage = GitOpsStage::Running;
        self.output.clear();
        self.finished = None;
        self.close_at = None;
        self.status = None;
        ScreenAction::ExecuteGitOp {
            repo_path: self.repo_path.clone(),
            op,
        }
    }
}
