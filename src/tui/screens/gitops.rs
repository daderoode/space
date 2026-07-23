use crate::core::git::CommitInfo;
use crate::tui::actions::{GitOp, ScreenAction, ScreenContext};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::path::PathBuf;
use tui_input::Input;

/// Stage of the git-operations overlay. Phase 1 ships only the action menu;
/// later phases add Committing / Log / Running / ConfirmPush.
#[derive(Debug, Clone, PartialEq)]
pub enum GitOpsStage {
    Menu,
    /// Single-line commit-message entry for the staged changes.
    Committing,
    /// Read-only scrollable list of recent commits.
    Log,
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
    /// Paths of the currently staged files, listed above the commit-message
    /// input in the Committing stage. Captured in `new()` so the renderer needs
    /// no git call.
    pub staged_files: Vec<String>,
    /// Single-line commit message entered in the Committing stage.
    pub message_input: Input,
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
    /// Recent commits loaded synchronously when the Log stage opens.
    pub commits: Vec<CommitInfo>,
    /// Scroll offset (top row index) for the Log stage. The renderer applies a
    /// viewport-aware upper clamp so it can never scroll past the last full
    /// screenful.
    pub log_scroll: u16,
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
        let staged_files: Vec<String> = crate::core::git::file_diff(&repo_path)
            .map(|v| {
                v.iter()
                    .filter(|e| e.staged)
                    .map(|e| e.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        let has_staged = !staged_files.is_empty();
        let has_upstream = crate::core::git::has_upstream(&repo_path);
        Self {
            stage: GitOpsStage::Menu,
            repo_name,
            repo_path,
            branch,
            selected: 0,
            has_staged,
            staged_files,
            message_input: Input::default(),
            has_upstream,
            status: None,
            op_label: "",
            output: Vec::new(),
            finished: None,
            close_at: None,
            commits: Vec::new(),
            log_scroll: 0,
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
            GitOpsStage::Committing => self.handle_committing_key(key),
            GitOpsStage::Log => self.handle_log_key(key),
            GitOpsStage::Menu => self.handle_menu_key(key),
        }
    }

    /// Handle keys in the read-only Log stage. `Esc`/`q` return to the menu;
    /// the arrow / `j`/`k` / PageUp/PageDown / Home / End keys scroll
    /// `log_scroll` with saturating add/sub. The down/end direction is clamped
    /// to the last commit index here; the renderer applies the viewport-aware
    /// clamp so no blank screenful is shown.
    fn handle_log_key(&mut self, key: KeyEvent) -> ScreenAction {
        let max = self.commits.len().saturating_sub(1) as u16;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.stage = GitOpsStage::Menu;
                ScreenAction::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.log_scroll = self.log_scroll.saturating_sub(1);
                ScreenAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.log_scroll = self.log_scroll.saturating_add(1).min(max);
                ScreenAction::Continue
            }
            KeyCode::PageUp => {
                self.log_scroll = self.log_scroll.saturating_sub(10);
                ScreenAction::Continue
            }
            KeyCode::PageDown => {
                self.log_scroll = self.log_scroll.saturating_add(10).min(max);
                ScreenAction::Continue
            }
            KeyCode::Home => {
                self.log_scroll = 0;
                ScreenAction::Continue
            }
            KeyCode::End => {
                self.log_scroll = max;
                ScreenAction::Continue
            }
            _ => ScreenAction::Continue,
        }
    }

    /// Handle keys in the Committing stage (single-line commit-message entry).
    /// `Esc` returns to the menu; `Enter` commits a non-empty message; every
    /// other key (including `q`) feeds the text input.
    fn handle_committing_key(&mut self, key: KeyEvent) -> ScreenAction {
        match key.code {
            KeyCode::Esc => {
                self.status = None;
                self.stage = GitOpsStage::Menu;
                ScreenAction::Continue
            }
            KeyCode::Enter => {
                let message = self.message_input.value().trim().to_string();
                if message.is_empty() {
                    self.status = Some("Commit message cannot be empty".to_string());
                    return ScreenAction::Continue;
                }
                self.status = None;
                ScreenAction::CommitRepo {
                    repo_path: self.repo_path.clone(),
                    message,
                }
            }
            _ => {
                if let Some(req) = crate::tui::app::key_to_input_request(&key) {
                    self.message_input.handle(req);
                }
                self.status = None;
                ScreenAction::Continue
            }
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

    /// Fire the menu item at `idx`, moving the highlight to it. Fetch/pull/push
    /// run through the async Running stage (push confirms first when the branch
    /// has no upstream); commit and log open their own stages synchronously;
    /// rebase is a disabled placeholder (Tier 3 item 7).
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
                // Enter the commit-message stage with a fresh input.
                self.stage = GitOpsStage::Committing;
                self.message_input = Input::default();
                self.status = None;
                ScreenAction::Continue
            }
            4 => {
                // Log is a local read-only op: load the commits synchronously.
                self.commits = crate::core::git::recent_commits(&self.repo_path, 50);
                self.log_scroll = 0;
                self.status = None;
                self.stage = GitOpsStage::Log;
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
